#![allow(
    clippy::redundant_pub_crate,
    reason = "the writer sibling module owns execution of these typed operations"
)]

use rusqlite::{Connection, OptionalExtension as _, Transaction, TransactionBehavior, params};
use tracepress_core::{ContentId, ContentKind};

use super::{BlobError, BlobPutReceipt, BlobStorage};
use crate::encode::{content_kind, sqlite};

#[derive(Debug)]
pub(crate) struct InlineRegistration {
    pub(crate) content_id: ContentId,
    pub(crate) raw_bytes: Box<[u8]>,
    pub(crate) kind: ContentKind,
    pub(crate) created_at: String,
}

#[derive(Debug)]
pub(crate) struct ExternalRegistration {
    pub(crate) content_id: ContentId,
    pub(crate) external_ref: String,
    pub(crate) byte_length: u64,
    pub(crate) kind: ContentKind,
    pub(crate) created_at: String,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum PinChange {
    Increment,
    Decrement,
}

#[derive(Debug)]
pub(crate) enum BlobDbOperation {
    RegisterInline(InlineRegistration),
    RegisterExternal(ExternalRegistration),
    Lookup(ContentId),
    Pin {
        content_id: ContentId,
        change: PinChange,
    },
    Protection(ContentId),
}

#[derive(Debug)]
pub(crate) enum BlobDbReply {
    Record(Option<BlobRecord>),
    PinCount(u64),
    Protection(GcProtection),
}

#[derive(Debug)]
pub(crate) enum BlobRecord {
    Inline {
        raw_bytes: Box<[u8]>,
    },
    External {
        external_ref: String,
        byte_length: u64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GcProtection {
    Orphan,
    Pinned,
    Recoverable,
    Referenced,
}

pub(crate) fn execute(
    connection: &mut Connection,
    operation: BlobDbOperation,
) -> Result<BlobDbReply, BlobError> {
    match operation {
        BlobDbOperation::RegisterInline(registration) => register_inline(connection, &registration)
            .map(|record| BlobDbReply::Record(Some(record))),
        BlobDbOperation::RegisterExternal(registration) => {
            register_external(connection, &registration)
                .map(|record| BlobDbReply::Record(Some(record)))
        }
        BlobDbOperation::Lookup(content_id) => {
            lookup(connection, content_id).map(BlobDbReply::Record)
        }
        BlobDbOperation::Pin { content_id, change } => {
            update_pin(connection, content_id, change).map(BlobDbReply::PinCount)
        }
        BlobDbOperation::Protection(content_id) => {
            protection(connection, content_id).map(BlobDbReply::Protection)
        }
    }
}

pub(crate) fn receipt_for_record(
    content_id: ContentId,
    record: BlobRecord,
) -> Result<BlobPutReceipt, BlobError> {
    let storage = match record {
        BlobRecord::Inline { raw_bytes } => {
            if ContentId::from_bytes(&raw_bytes) != content_id {
                return Err(BlobError::Corrupt {
                    content_id,
                    detail: "inline bytes do not match their database identity",
                });
            }
            BlobStorage::Inline
        }
        BlobRecord::External { .. } => BlobStorage::External,
    };
    Ok(BlobPutReceipt::new(content_id, storage))
}

fn register_inline(
    connection: &mut Connection,
    registration: &InlineRegistration,
) -> Result<BlobRecord, BlobError> {
    let byte_length = i64::try_from(registration.raw_bytes.len()).map_err(|_error| {
        BlobError::LimitUnrepresentable {
            field: "inline_blob_length",
            value: u64::MAX,
        }
    })?;
    let transaction = sqlite(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
    let _rows = sqlite(transaction.execute(
        "INSERT OR IGNORE INTO content_objects(content_id, raw_bytes, external_ref, byte_length, content_kind, pin_count, created_at) VALUES (?1, ?2, NULL, ?3, ?4, 0, ?5)",
        params![registration.content_id.to_string(), registration.raw_bytes.as_ref(), byte_length, content_kind(registration.kind), registration.created_at],
    ))?;
    let record =
        lookup_transaction(&transaction, registration.content_id)?.ok_or(BlobError::Corrupt {
            content_id: registration.content_id,
            detail: "inline registration committed without a readable row",
        })?;
    sqlite(transaction.commit())?;
    Ok(record)
}

fn register_external(
    connection: &mut Connection,
    registration: &ExternalRegistration,
) -> Result<BlobRecord, BlobError> {
    let byte_length = i64::try_from(registration.byte_length).map_err(|_error| {
        BlobError::LimitUnrepresentable {
            field: "external_blob_length",
            value: registration.byte_length,
        }
    })?;
    let transaction = sqlite(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
    let _rows = sqlite(transaction.execute(
        "INSERT OR IGNORE INTO content_objects(content_id, raw_bytes, external_ref, byte_length, content_kind, pin_count, created_at) VALUES (?1, NULL, ?2, ?3, ?4, 0, ?5)",
        params![registration.content_id.to_string(), registration.external_ref, byte_length, content_kind(registration.kind), registration.created_at],
    ))?;
    let record =
        lookup_transaction(&transaction, registration.content_id)?.ok_or(BlobError::Corrupt {
            content_id: registration.content_id,
            detail: "external registration committed without a readable row",
        })?;
    sqlite(transaction.commit())?;
    Ok(record)
}

fn lookup(connection: &Connection, content_id: ContentId) -> Result<Option<BlobRecord>, BlobError> {
    query_record(connection, content_id)
}

fn lookup_transaction(
    transaction: &Transaction<'_>,
    content_id: ContentId,
) -> Result<Option<BlobRecord>, BlobError> {
    query_record(transaction, content_id)
}

fn query_record(
    connection: &Connection,
    content_id: ContentId,
) -> Result<Option<BlobRecord>, BlobError> {
    let row = sqlite(connection.query_row(
        "SELECT raw_bytes, external_ref, byte_length FROM content_objects WHERE content_id = ?1",
        [content_id.to_string()],
        |row| {
            let raw_bytes = row.get::<_, Option<Vec<u8>>>(0)?;
            let external_ref = row.get::<_, Option<String>>(1)?;
            let byte_length = row.get::<_, i64>(2)?;
            Ok((raw_bytes, external_ref, byte_length))
        },
    ).optional())?;
    row.map(|(raw_bytes, external_ref, byte_length)| {
        let byte_length = u64::try_from(byte_length).map_err(|_error| BlobError::Corrupt {
            content_id,
            detail: "database byte length is negative",
        })?;
        match (raw_bytes, external_ref) {
            (Some(raw_bytes), None) => {
                if u64::try_from(raw_bytes.len()).ok() != Some(byte_length) {
                    return Err(BlobError::Corrupt {
                        content_id,
                        detail: "inline byte length disagrees with database metadata",
                    });
                }
                Ok(BlobRecord::Inline {
                    raw_bytes: raw_bytes.into_boxed_slice(),
                })
            }
            (None, Some(external_ref)) => Ok(BlobRecord::External {
                external_ref,
                byte_length,
            }),
            (Some(_), Some(_)) | (None, None) => Err(BlobError::Corrupt {
                content_id,
                detail: "database location violates inline/external exclusivity",
            }),
        }
    })
    .transpose()
}

fn update_pin(
    connection: &mut Connection,
    content_id: ContentId,
    change: PinChange,
) -> Result<u64, BlobError> {
    let transaction = sqlite(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
    let current = sqlite(
        transaction
            .query_row(
                "SELECT pin_count FROM content_objects WHERE content_id = ?1",
                [content_id.to_string()],
                |row| row.get::<_, i64>(0),
            )
            .optional(),
    )?
    .ok_or(BlobError::Missing { content_id })?;
    let next = match change {
        PinChange::Increment => current
            .checked_add(1)
            .ok_or(BlobError::PinOverflow { content_id })?,
        PinChange::Decrement => current
            .checked_sub(1)
            .filter(|value| *value >= 0)
            .ok_or(BlobError::PinUnderflow { content_id })?,
    };
    let _rows = sqlite(transaction.execute(
        "UPDATE content_objects SET pin_count = ?2 WHERE content_id = ?1",
        params![content_id.to_string(), next],
    ))?;
    sqlite(transaction.commit())?;
    u64::try_from(next).map_err(|_error| BlobError::Corrupt {
        content_id,
        detail: "database pin count is negative",
    })
}

fn protection(connection: &Connection, content_id: ContentId) -> Result<GcProtection, BlobError> {
    let row = sqlite(connection.query_row(
        "SELECT pin_count, EXISTS(SELECT 1 FROM recoveries WHERE raw_content_id = ?1 OR provenance_content_id = ?1) FROM content_objects WHERE content_id = ?1",
        [content_id.to_string()],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, bool>(1)?)),
    ).optional())?;
    Ok(match row {
        None => GcProtection::Orphan,
        Some((pin_count, _)) if pin_count > 0 => GcProtection::Pinned,
        Some((_, true)) => GcProtection::Recoverable,
        Some((_, false)) => GcProtection::Referenced,
    })
}
