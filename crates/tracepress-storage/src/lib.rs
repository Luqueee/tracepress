//! Durable `SQLite` storage boundary for Tracepress.
use rusqlite::{Connection, OptionalExtension, params};
use tracepress_core::{ContextSnapshotId, RequestId};

mod blob_store;
mod encode;
mod error;
pub(crate) mod inspection;
mod models;
mod records;
mod schema;
mod writer;

#[cfg(test)]
mod blob_store_tests;

pub use blob_store::{
    BlobByteLimit, BlobError, BlobPut, BlobPutReceipt, BlobStorage, BlobStore, BlobStoreConfig,
    DaemonGc, FilesystemCasBlobStore, GcDecision, GcReport, HybridBlobStore, InlineBlobMaxBytes,
    InlineSqliteBlobStore, PersistenceCompression, ZstdLevel,
};
pub use error::StorageError;
pub use models::{
    CONTEXT_INSPECTION_MAX_BLOCKS, ContextAnalysisStatus, ContextBlockKind,
    ContextCorrelationStatus, ContextInspection, ContextInspectionBlock,
    ContextInspectionComposition, ContextInspectionCoverage, ContextInspectionNamedEstimate,
    ContextInspectionRepetition, ContextInspectionVisibility, ContextOrigin, ContextRole,
    ContextSnapshotStatus, DetectedContentKind, EstimateConfidence, EstimatedTokenComposition,
    EstimatedTokensByKind, EstimatedTokensByOrigin, EstimatedTokensByRole, FidelityClass,
    LogicalContextStatus, ObservationStatus, OpportunitySignal, ProviderKind, ProviderProtocol,
    ProviderResponseState, ReconciliationStatus, RecoveryReceipt, WriteBatch, WriteCommand,
    WriteReceipt,
};

/// Bounded key used by the daemon's lifecycle reconciliation read.
#[derive(Clone, Copy, Debug)]
pub(crate) enum ContextSnapshotStatusLookup {
    /// Select the newest snapshot for one provider request.
    Request(RequestId),
    /// Select exactly one durable snapshot.
    Snapshot(ContextSnapshotId),
}

/// Reads only the durable lifecycle fields for one context snapshot.
///
/// Unlike [`inspection::query_context_inspection`], this path never loads block occurrences or
/// aggregate metrics. It returns an empty result when the requested snapshot has not been
/// inserted yet.
///
/// # Errors
/// Returns a storage error when `SQLite` returns an invalid row or query failure.
pub(crate) fn query_context_snapshot_status(
    connection: &Connection,
    lookup: ContextSnapshotStatusLookup,
) -> Result<Option<ContextSnapshotStatus>, StorageError> {
    let (sql, value) = match lookup {
        ContextSnapshotStatusLookup::Request(request_id) => (
            "SELECT snapshot_id, provider_request_id, status, completed_at_us
             FROM context_snapshots
             WHERE provider_request_id = ?1
             ORDER BY analysis_version DESC, snapshot_id DESC
             LIMIT 1",
            request_id.to_string(),
        ),
        ContextSnapshotStatusLookup::Snapshot(snapshot_id) => (
            "SELECT snapshot_id, provider_request_id, status, completed_at_us
             FROM context_snapshots
             WHERE snapshot_id = ?1
             LIMIT 1",
            snapshot_id.to_string(),
        ),
    };
    let raw = crate::encode::sqlite(
        connection
            .query_row(sql, params![value], |row| {
                Ok((
                    row.get::<_, String>("snapshot_id")?,
                    row.get::<_, String>("provider_request_id")?,
                    row.get::<_, String>("status")?,
                    row.get::<_, Option<i64>>("completed_at_us")?,
                ))
            })
            .optional(),
    )?;
    let Some((snapshot_id, provider_request_id, status, completed_at_us)) = raw else {
        return Ok(None);
    };
    let snapshot_id =
        snapshot_id
            .parse()
            .map_err(|_error| StorageError::InvalidContextInspection {
                field: "snapshot_id",
            })?;
    let provider_request_id =
        provider_request_id
            .parse()
            .map_err(|_error| StorageError::InvalidContextInspection {
                field: "provider_request_id",
            })?;
    let completed_at_us = completed_at_us
        .map(|value| {
            u64::try_from(value).map_err(|_| StorageError::InvalidContextInspection {
                field: "completed_at_us",
            })
        })
        .transpose()?;
    Ok(Some(
        ContextSnapshotStatus::new(snapshot_id, provider_request_id, status)
            .with_completed_at(completed_at_us),
    ))
}
pub use schema::{ConnectionSettings, Durability};
pub use writer::{StorageConfig, StorageWriter};

#[cfg(test)]
mod capacity_tests;
#[cfg(test)]
mod concurrency_tests;
#[cfg(test)]
mod context_tests;
#[cfg(test)]
mod overflow_tests;
#[cfg(test)]
mod record_tests;
#[cfg(test)]
mod schema_tests;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod writer_tests;
