use thiserror::Error;

/// A failure at the durable storage boundary.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum StorageError {
    /// `SQLite` rejected an operation.
    #[error("SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// `SQLite` remained locked through the configured busy bound.
    #[error("SQLite remained busy: {0}")]
    Busy(#[source] rusqlite::Error),
    /// A persisted schema is newer than this binary understands.
    #[error("database schema version {found} is newer than supported version {supported}")]
    UnsupportedSchemaVersion {
        /// Version recorded by the database.
        found: u32,
        /// Latest version understood by this crate.
        supported: u32,
    },
    /// `SQLite` did not retain one mandatory connection setting.
    #[error("SQLite setting {setting} expected {expected}, found {actual}")]
    ConfigurationMismatch {
        /// Name of the mismatched pragma.
        setting: &'static str,
        /// Required value.
        expected: &'static str,
        /// Observed value.
        actual: String,
    },
    /// A positive Rust integer cannot be represented by this platform or `SQLite`.
    #[error("value {value} for {field} exceeds the supported integer range")]
    IntegerOverflow {
        /// Logical field being converted.
        field: &'static str,
        /// Rejected unsigned value.
        value: u64,
    },
    /// The configured writer queue exceeds Tokio's bounded channel capacity.
    #[error("writer queue capacity {requested} exceeds Tokio maximum {maximum}")]
    WriterQueueCapacityExceeded {
        /// Configured queue capacity.
        requested: u64,
        /// Maximum capacity supported by the linked Tokio version.
        maximum: u64,
    },
    /// The writer thread could not be created.
    #[error("failed to start SQLite writer thread: {0}")]
    ThreadStart(#[source] std::io::Error),
    /// The writer stopped before startup completed.
    #[error("SQLite writer stopped during startup")]
    StartupStopped,
    /// The bounded writer queue is closed.
    #[error("SQLite writer queue is closed")]
    QueueClosed,
    /// The bounded writer queue has no available capacity.
    #[error("SQLite writer queue is full")]
    QueueFull,
    /// The writer stopped before replying to an accepted command.
    #[error("SQLite writer stopped before returning a command result")]
    ReplyStopped,
    /// Tokio could not join the blocking shutdown waiter.
    #[error("failed to await SQLite writer shutdown: {0}")]
    ShutdownJoin(#[source] tokio::task::JoinError),
    /// The dedicated writer thread panicked.
    #[error("SQLite writer thread terminated unexpectedly")]
    WorkerPanicked,
    /// A test failpoint interrupted a transaction before commit.
    #[cfg(test)]
    #[error("injected storage transaction failure")]
    InjectedFailure,
}
