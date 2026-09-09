#![allow(
    clippy::redundant_pub_crate,
    reason = "the writer and schema-test sibling modules consume these open functions"
)]

use std::{path::Path, time::Duration};

use rusqlite::{Connection, OpenFlags, TransactionBehavior, params};

use crate::StorageError;

const LATEST_SCHEMA_VERSION: u32 = 2;
pub(crate) const MIGRATION_V1: &str = include_str!("../migrations/0001_initial.sql");
pub(crate) const MIGRATION_V2: &str = include_str!("../migrations/0002_provider_observability.sql");
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// `SQLite` durability policy used by the daemon-owned writer.
#[allow(
    clippy::exhaustive_enums,
    reason = "configuration must select one of the two documented durability modes"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Durability {
    /// WAL with `synchronous=NORMAL`.
    Balanced,
    /// WAL with `synchronous=FULL`.
    Strict,
}

impl Durability {
    const fn synchronous(self) -> i64 {
        match self {
            Self::Balanced => 1,
            Self::Strict => 2,
        }
    }
}

/// Settings observed from the actual writer connection after configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectionSettings {
    journal_mode: String,
    foreign_keys: bool,
    busy_timeout_ms: u64,
    synchronous: i64,
}

impl ConnectionSettings {
    /// Returns the configured journal mode.
    #[must_use]
    pub fn journal_mode(&self) -> &str {
        &self.journal_mode
    }

    /// Reports whether foreign-key enforcement is active.
    #[must_use]
    pub const fn foreign_keys(&self) -> bool {
        self.foreign_keys
    }

    /// Returns the configured busy timeout in milliseconds.
    #[must_use]
    pub const fn busy_timeout_ms(&self) -> u64 {
        self.busy_timeout_ms
    }

    /// Returns `SQLite`'s numeric synchronous mode.
    #[must_use]
    pub const fn synchronous(&self) -> i64 {
        self.synchronous
    }
}

pub(crate) fn open_database(
    path: &Path,
    durability: Durability,
) -> Result<(Connection, ConnectionSettings), StorageError> {
    open_database_with_busy_timeout(path, durability, BUSY_TIMEOUT)
}

pub(crate) fn open_database_with_busy_timeout(
    path: &Path,
    durability: Durability,
    busy_timeout: Duration,
) -> Result<(Connection, ConnectionSettings), StorageError> {
    let mut connection = open_configured(path, durability, busy_timeout)?;
    #[cfg(test)]
    migrate(&mut connection, false, false)?;
    #[cfg(not(test))]
    migrate(&mut connection)?;
    let settings = verify_settings(&connection, busy_timeout)?;
    Ok((connection, settings))
}

#[cfg(test)]
pub(crate) fn open_database_with_interrupted_migration(
    path: &Path,
    durability: Durability,
) -> Result<(Connection, ConnectionSettings), StorageError> {
    let mut connection = open_configured(path, durability, BUSY_TIMEOUT)?;
    migrate(&mut connection, true, false)?;
    let settings = verify_settings(&connection, BUSY_TIMEOUT)?;
    Ok((connection, settings))
}

#[cfg(test)]
pub(crate) fn open_database_with_interrupted_v2_migration(
    path: &Path,
    durability: Durability,
) -> Result<(Connection, ConnectionSettings), StorageError> {
    let mut connection = open_configured(path, durability, BUSY_TIMEOUT)?;
    migrate(&mut connection, false, true)?;
    let settings = verify_settings(&connection, BUSY_TIMEOUT)?;
    Ok((connection, settings))
}

fn open_configured(
    path: &Path,
    durability: Durability,
    busy_timeout: Duration,
) -> Result<Connection, StorageError> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(busy_timeout)?;
    connection.pragma_update(None, "foreign_keys", true)?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", durability.synchronous())?;
    Ok(connection)
}

fn migrate(
    connection: &mut Connection,
    #[cfg(test)] interrupt_v1: bool,
    #[cfg(test)] interrupt_v2: bool,
) -> Result<(), StorageError> {
    let current = current_schema_version(connection)?;
    if current > LATEST_SCHEMA_VERSION {
        return Err(StorageError::UnsupportedSchemaVersion {
            found: current,
            supported: LATEST_SCHEMA_VERSION,
        });
    }
    if current < 1 {
        apply_migration(
            connection,
            MIGRATION_V1,
            #[cfg(test)]
            interrupt_v1,
        )?;
    }
    if current < 2 {
        apply_migration(
            connection,
            MIGRATION_V2,
            #[cfg(test)]
            interrupt_v2,
        )?;
    }
    Ok(())
}

fn apply_migration(
    connection: &mut Connection,
    migration: &str,
    #[cfg(test)] interrupt: bool,
) -> Result<(), StorageError> {
    let version = current_schema_version(connection)?;
    let next_version = version
        .checked_add(1)
        .ok_or(StorageError::IntegerOverflow {
            field: "schema_version",
            value: u64::MAX,
        })?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(migration)?;
    #[cfg(test)]
    if interrupt {
        return Err(StorageError::InjectedFailure);
    }
    let _rows = transaction.execute(
        "INSERT INTO schema_metadata(schema_version, applied_at) VALUES (?1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        params![next_version],
    )?;
    transaction.commit()?;
    Ok(())
}

fn current_schema_version(connection: &Connection) -> Result<u32, StorageError> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'schema_metadata')",
        [],
        |row| row.get(0),
    )?;
    if !exists {
        return Ok(0);
    }
    let version = connection
        .query_row(
            "SELECT MAX(schema_version) FROM schema_metadata",
            [],
            |row| row.get::<_, Option<u32>>(0),
        )?
        .unwrap_or(0);
    Ok(version)
}

fn verify_settings(
    connection: &Connection,
    expected_busy_timeout: Duration,
) -> Result<ConnectionSettings, StorageError> {
    let journal_mode = connection.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
    let foreign_keys = connection.pragma_query_value(None, "foreign_keys", |row| row.get(0))?;
    let busy_timeout_ms = connection.pragma_query_value(None, "busy_timeout", |row| row.get(0))?;
    let synchronous = connection.pragma_query_value(None, "synchronous", |row| row.get(0))?;
    let settings = ConnectionSettings {
        journal_mode,
        foreign_keys,
        busy_timeout_ms,
        synchronous,
    };
    require_setting(&settings.journal_mode, "journal_mode", "wal")?;
    require_setting(
        if settings.foreign_keys { "on" } else { "off" },
        "foreign_keys",
        "on",
    )?;
    let expected_busy_timeout_ms =
        u64::try_from(expected_busy_timeout.as_millis()).map_err(|_error| {
            StorageError::IntegerOverflow {
                field: "busy_timeout_ms",
                value: u64::MAX,
            }
        })?;
    if settings.busy_timeout_ms != expected_busy_timeout_ms {
        return Err(StorageError::ConfigurationMismatch {
            setting: "busy_timeout",
            expected: "configured milliseconds",
            actual: settings.busy_timeout_ms.to_string(),
        });
    }
    Ok(settings)
}

fn require_setting(
    actual: &str,
    setting: &'static str,
    expected: &'static str,
) -> Result<(), StorageError> {
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(StorageError::ConfigurationMismatch {
            setting,
            expected,
            actual: actual.to_owned(),
        })
    }
}
