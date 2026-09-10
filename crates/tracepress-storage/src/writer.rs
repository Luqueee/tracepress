#![allow(
    clippy::redundant_pub_crate,
    reason = "crate-private test modules consume deterministic writer synchronization seams"
)]

use std::{path::PathBuf, thread::JoinHandle};

use tokio::sync::{Semaphore, mpsc, oneshot};
use tracepress_core::{ContentId, ContextSnapshotId, MaxIpcQueueItems, RequestId};

use crate::{
    ConnectionSettings, ContextInspection, ContextSnapshotStatus, ContextSnapshotStatusLookup,
    Durability, RecoveryReceipt, StorageError, WriteBatch, WriteCommand, WriteReceipt,
    blob_store::{
        BlobError,
        database::{
            self, BlobDbOperation, BlobDbReply, BlobRecord, ExternalRegistration, GcProtection,
            InlineRegistration, PinChange,
        },
    },
    inspection::query_context_inspection,
    query_context_snapshot_status,
    records::{execute_batch, execute_single, recover_stale_sessions},
};

#[cfg(not(test))]
use crate::schema::open_database;
#[cfg(test)]
use crate::schema::open_database_with_busy_timeout;

#[derive(Debug)]
enum WriterOperation {
    Single(Box<WriteCommand>),
    Batch(WriteBatch),
    #[cfg(test)]
    InterruptedBatch {
        batch: WriteBatch,
        fail_after: usize,
    },
    #[cfg(test)]
    Pause {
        entered: oneshot::Sender<()>,
        release: oneshot::Receiver<()>,
    },
}

#[derive(Debug)]
struct Envelope {
    operation: WriterOperation,
    reply: oneshot::Sender<Result<WriteReceipt, StorageError>>,
}

#[derive(Debug)]
struct RecoveryEnvelope {
    recovered_at: String,
    reply: oneshot::Sender<Result<RecoveryReceipt, StorageError>>,
}

#[derive(Debug)]
enum WriterMessage {
    Write(Envelope),
    Recovery(RecoveryEnvelope),
    Blob(BlobEnvelope),
    ContextInspection(ContextInspectionEnvelope),
    ContextSnapshotStatus(ContextSnapshotStatusEnvelope),
}

#[derive(Debug)]
struct BlobEnvelope {
    operation: BlobDbOperation,
    reply: oneshot::Sender<Result<BlobDbReply, BlobError>>,
}

#[derive(Debug)]
struct ContextInspectionEnvelope {
    request_id: RequestId,
    reply: oneshot::Sender<Result<ContextInspection, StorageError>>,
}

#[derive(Debug)]
struct ContextSnapshotStatusEnvelope {
    lookup: ContextSnapshotStatusLookup,
    reply: oneshot::Sender<Result<Option<ContextSnapshotStatus>, StorageError>>,
}

/// Startup configuration for the single `SQLite` writer.
#[derive(Clone, Debug)]
pub struct StorageConfig {
    path: PathBuf,
    durability: Durability,
    queue_items: MaxIpcQueueItems,
    #[cfg(test)]
    busy_timeout: std::time::Duration,
}

impl StorageConfig {
    /// Creates writer configuration for one database file.
    #[must_use]
    pub const fn new(path: PathBuf, durability: Durability, queue_items: MaxIpcQueueItems) -> Self {
        Self {
            path,
            durability,
            queue_items,
            #[cfg(test)]
            busy_timeout: std::time::Duration::from_secs(5),
        }
    }

    pub(crate) const fn durability(&self) -> Durability {
        self.durability
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) const fn with_test_busy_timeout(
        mut self,
        busy_timeout: std::time::Duration,
    ) -> Self {
        self.busy_timeout = busy_timeout;
        self
    }
}

/// The sole daemon-owned handle capable of submitting `SQLite` writes.
#[derive(Debug)]
pub struct StorageWriter {
    sender: mpsc::Sender<WriterMessage>,
    worker: JoinHandle<()>,
    settings: ConnectionSettings,
}

impl StorageWriter {
    /// Opens, configures, and migrates a real `SQLite` file on its dedicated writer thread.
    ///
    /// # Errors
    /// Returns a typed storage error when the queue capacity is unsupported or the thread,
    /// connection, settings, or migration cannot be initialized.
    pub async fn open(config: StorageConfig) -> Result<Self, StorageError> {
        let capacity = writer_queue_capacity(config.queue_items)?;
        let (sender, mut receiver) = mpsc::channel::<WriterMessage>(capacity);
        let (startup_sender, startup_receiver) = oneshot::channel();
        let worker = std::thread::Builder::new()
            .name("tracepress-sqlite-writer".to_owned())
            .spawn(move || match open_writer_database(&config) {
                Ok((mut connection, settings)) => {
                    let _sent = startup_sender.send(Ok(settings));
                    while let Some(message) = receiver.blocking_recv() {
                        match message {
                            WriterMessage::Write(envelope) => {
                                let result =
                                    execute_writer_operation(&mut connection, envelope.operation);
                                let _sent = envelope.reply.send(result);
                            }
                            WriterMessage::Recovery(envelope) => {
                                let result =
                                    recover_stale_sessions(&mut connection, &envelope.recovered_at);
                                let _sent = envelope.reply.send(result);
                            }
                            WriterMessage::ContextInspection(envelope) => {
                                let result =
                                    query_context_inspection(&connection, envelope.request_id);
                                let _sent = envelope.reply.send(result);
                            }
                            WriterMessage::ContextSnapshotStatus(envelope) => {
                                let result =
                                    query_context_snapshot_status(&connection, envelope.lookup);
                                let _sent = envelope.reply.send(result);
                            }
                            WriterMessage::Blob(envelope) => {
                                let result = database::execute(&mut connection, envelope.operation);
                                let _sent = envelope.reply.send(result);
                            }
                        }
                    }
                    drop(connection);
                }
                Err(error) => {
                    let _sent = startup_sender.send(Err(error));
                }
            })
            .map_err(StorageError::ThreadStart)?;
        let settings = startup_receiver
            .await
            .map_err(|_error| StorageError::StartupStopped)??;
        Ok(Self {
            sender,
            worker,
            settings,
        })
    }

    /// Marks sessions left active by a previous daemon process as stale.
    ///
    /// # Errors
    /// Returns a typed storage error when the recovery transaction cannot commit.
    pub async fn recover_stale_sessions(
        &self,
        recovered_at: &str,
    ) -> Result<RecoveryReceipt, StorageError> {
        let (reply, result) = oneshot::channel();
        self.sender
            .send(WriterMessage::Recovery(RecoveryEnvelope {
                recovered_at: recovered_at.to_owned(),
                reply,
            }))
            .await
            .map_err(|_error| StorageError::QueueClosed)?;
        result.await.map_err(|_error| StorageError::ReplyStopped)?
    }

    /// Reads one bounded context inspection through the daemon-owned `SQLite` connection.
    ///
    /// # Errors
    /// Returns a typed not-found, queue, query, or reply error. No client connection is opened.
    pub async fn inspect_context(
        &self,
        request_id: RequestId,
    ) -> Result<ContextInspection, StorageError> {
        let (reply, result) = oneshot::channel();
        self.sender
            .send(WriterMessage::ContextInspection(
                ContextInspectionEnvelope { request_id, reply },
            ))
            .await
            .map_err(|_error| StorageError::QueueClosed)?;
        result.await.map_err(|_error| StorageError::ReplyStopped)?
    }

    /// Reads only one context snapshot's lifecycle status through the daemon-owned connection.
    ///
    /// The result is empty when the snapshot has not committed its begin record yet. No block
    /// occurrence or aggregate projection is loaded.
    ///
    /// # Errors
    /// Returns a typed queue or reply error when the writer worker cannot process the request.
    pub async fn context_snapshot_status_by_request(
        &self,
        request_id: RequestId,
    ) -> Result<Option<ContextSnapshotStatus>, StorageError> {
        self.context_snapshot_status(ContextSnapshotStatusLookup::Request(request_id))
            .await
    }

    /// Reads one context snapshot's lifecycle status by durable snapshot identity.
    ///
    /// # Errors
    /// Returns a typed queue or reply error when the writer worker cannot process the request.
    pub async fn context_snapshot_status_by_snapshot(
        &self,
        snapshot_id: ContextSnapshotId,
    ) -> Result<Option<ContextSnapshotStatus>, StorageError> {
        self.context_snapshot_status(ContextSnapshotStatusLookup::Snapshot(snapshot_id))
            .await
    }

    async fn context_snapshot_status(
        &self,
        lookup: ContextSnapshotStatusLookup,
    ) -> Result<Option<ContextSnapshotStatus>, StorageError> {
        let (reply, result) = oneshot::channel();
        self.sender
            .send(WriterMessage::ContextSnapshotStatus(
                ContextSnapshotStatusEnvelope { lookup, reply },
            ))
            .await
            .map_err(|_error| StorageError::QueueClosed)?;
        result.await.map_err(|_error| StorageError::ReplyStopped)?
    }

    /// Returns settings verified on the actual writer connection.
    #[must_use]
    pub const fn settings(&self) -> &ConnectionSettings {
        &self.settings
    }

    /// Submits one typed mutation and waits until its transaction commits or fails.
    ///
    /// # Errors
    /// Returns a typed queue, conversion, constraint, busy, or `SQLite` error without reporting
    /// success for an uncommitted command.
    pub async fn submit(&self, command: WriteCommand) -> Result<WriteReceipt, StorageError> {
        self.submit_operation(WriterOperation::Single(Box::new(command)))
            .await
    }

    /// Submits a non-empty command batch as one all-or-nothing transaction.
    ///
    /// # Errors
    /// Returns a typed storage error and rolls back every command if any command cannot commit.
    pub async fn submit_batch(&self, batch: WriteBatch) -> Result<WriteReceipt, StorageError> {
        self.submit_operation(WriterOperation::Batch(batch)).await
    }

    async fn submit_operation(
        &self,
        operation: WriterOperation,
    ) -> Result<WriteReceipt, StorageError> {
        let (reply, result) = oneshot::channel();
        self.sender
            .send(WriterMessage::Write(Envelope { operation, reply }))
            .await
            .map_err(|_error| StorageError::QueueClosed)?;
        result.await.map_err(|_error| StorageError::ReplyStopped)?
    }

    /// Attempts to submit without waiting for bounded queue capacity.
    ///
    /// # Errors
    /// Returns [`StorageError::QueueFull`] when backpressure is active, or another typed storage
    /// failure if the accepted transaction does not commit.
    pub async fn try_submit(&self, command: WriteCommand) -> Result<WriteReceipt, StorageError> {
        let (reply, result) = oneshot::channel();
        self.sender
            .try_send(
                Envelope {
                    operation: WriterOperation::Single(Box::new(command)),
                    reply,
                }
                .into(),
            )
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_envelope) => StorageError::QueueFull,
                mpsc::error::TrySendError::Closed(_envelope) => StorageError::QueueClosed,
            })?;
        result.await.map_err(|_error| StorageError::ReplyStopped)?
    }

    #[cfg(test)]
    pub(crate) async fn submit_interrupted_batch(
        &self,
        batch: WriteBatch,
        fail_after: usize,
    ) -> Result<WriteReceipt, StorageError> {
        self.submit_operation(WriterOperation::InterruptedBatch { batch, fail_after })
            .await
    }

    #[cfg(test)]
    pub(crate) async fn pause_for_test(
        &self,
    ) -> Result<(oneshot::Receiver<()>, oneshot::Sender<()>), StorageError> {
        let (entered_sender, entered_receiver) = oneshot::channel();
        let (release_sender, release_receiver) = oneshot::channel();
        let (reply, _result) = oneshot::channel();
        self.sender
            .send(WriterMessage::Write(Envelope {
                operation: WriterOperation::Pause {
                    entered: entered_sender,
                    release: release_receiver,
                },
                reply,
            }))
            .await
            .map_err(|_error| StorageError::QueueClosed)?;
        Ok((entered_receiver, release_sender))
    }

    #[cfg(test)]
    pub(crate) async fn enqueue_for_test(
        &self,
        command: WriteCommand,
    ) -> Result<oneshot::Receiver<Result<WriteReceipt, StorageError>>, StorageError> {
        let (reply, result) = oneshot::channel();
        self.sender
            .send(WriterMessage::Write(Envelope {
                operation: WriterOperation::Single(Box::new(command)),
                reply,
            }))
            .await
            .map_err(|_error| StorageError::QueueClosed)?;
        Ok(result)
    }

    /// Closes the queue, drains accepted work, and joins the dedicated writer thread.
    ///
    /// # Errors
    /// Returns a typed error if the blocking waiter or writer thread terminates unexpectedly.
    pub async fn shutdown(self) -> Result<(), StorageError> {
        let Self {
            sender,
            worker,
            settings: _,
        } = self;
        drop(sender);
        let joined = tokio::task::spawn_blocking(move || worker.join().is_ok())
            .await
            .map_err(StorageError::ShutdownJoin)?;
        if joined {
            Ok(())
        } else {
            Err(StorageError::WorkerPanicked)
        }
    }

    pub(crate) async fn blob_register_inline(
        &self,
        registration: InlineRegistration,
    ) -> Result<BlobRecord, BlobError> {
        match self
            .submit_blob(BlobDbOperation::RegisterInline(registration))
            .await?
        {
            BlobDbReply::Record(Some(record)) => Ok(record),
            BlobDbReply::Record(None) | BlobDbReply::PinCount(_) | BlobDbReply::Protection(_) => {
                Err(BlobError::Corrupt {
                    content_id: ContentId::from_bytes(&[]),
                    detail: "writer returned an invalid inline registration reply",
                })
            }
        }
    }

    pub(crate) async fn blob_register_external(
        &self,
        registration: ExternalRegistration,
    ) -> Result<BlobRecord, BlobError> {
        let content_id = registration.content_id;
        match self
            .submit_blob(BlobDbOperation::RegisterExternal(registration))
            .await?
        {
            BlobDbReply::Record(Some(record)) => Ok(record),
            BlobDbReply::Record(None) | BlobDbReply::PinCount(_) | BlobDbReply::Protection(_) => {
                Err(BlobError::Corrupt {
                    content_id,
                    detail: "writer returned an invalid external registration reply",
                })
            }
        }
    }

    pub(crate) async fn blob_lookup(
        &self,
        content_id: ContentId,
    ) -> Result<Option<BlobRecord>, BlobError> {
        match self
            .submit_blob(BlobDbOperation::Lookup(content_id))
            .await?
        {
            BlobDbReply::Record(record) => Ok(record),
            BlobDbReply::PinCount(_) | BlobDbReply::Protection(_) => Err(BlobError::Corrupt {
                content_id,
                detail: "writer returned an invalid lookup reply",
            }),
        }
    }

    pub(crate) async fn blob_pin(
        &self,
        content_id: ContentId,
        change: PinChange,
    ) -> Result<u64, BlobError> {
        match self
            .submit_blob(BlobDbOperation::Pin { content_id, change })
            .await?
        {
            BlobDbReply::PinCount(count) => Ok(count),
            BlobDbReply::Record(_) | BlobDbReply::Protection(_) => Err(BlobError::Corrupt {
                content_id,
                detail: "writer returned an invalid pin reply",
            }),
        }
    }

    pub(crate) async fn blob_protection(
        &self,
        content_id: ContentId,
    ) -> Result<GcProtection, BlobError> {
        match self
            .submit_blob(BlobDbOperation::Protection(content_id))
            .await?
        {
            BlobDbReply::Protection(protection) => Ok(protection),
            BlobDbReply::Record(_) | BlobDbReply::PinCount(_) => Err(BlobError::Corrupt {
                content_id,
                detail: "writer returned an invalid GC protection reply",
            }),
        }
    }

    async fn submit_blob(&self, operation: BlobDbOperation) -> Result<BlobDbReply, BlobError> {
        let (reply, result) = oneshot::channel();
        self.sender
            .send(WriterMessage::Blob(BlobEnvelope { operation, reply }))
            .await
            .map_err(|_error| BlobError::Database(StorageError::QueueClosed))?;
        result
            .await
            .map_err(|_error| BlobError::Database(StorageError::ReplyStopped))?
    }
}

impl From<Envelope> for WriterMessage {
    fn from(envelope: Envelope) -> Self {
        Self::Write(envelope)
    }
}

fn execute_writer_operation(
    connection: &mut rusqlite::Connection,
    operation: WriterOperation,
) -> Result<WriteReceipt, StorageError> {
    match operation {
        WriterOperation::Single(command) => execute_single(connection, &command),
        WriterOperation::Batch(batch) => execute_batch(connection, &batch),
        #[cfg(test)]
        WriterOperation::InterruptedBatch { batch, fail_after } => {
            crate::records::execute_interrupted_batch(connection, &batch, fail_after)
        }
        #[cfg(test)]
        WriterOperation::Pause { entered, release } => {
            let _entered = entered.send(());
            match release.blocking_recv() {
                Ok(()) => Ok(WriteReceipt::Committed { rows_changed: 0 }),
                Err(_error) => Err(StorageError::ReplyStopped),
            }
        }
    }
}

pub(crate) fn writer_queue_capacity(queue_items: MaxIpcQueueItems) -> Result<usize, StorageError> {
    let requested = queue_items.get();
    let maximum = u64::try_from(Semaphore::MAX_PERMITS).unwrap_or(u64::MAX);
    if requested > maximum {
        return Err(StorageError::WriterQueueCapacityExceeded { requested, maximum });
    }
    usize::try_from(requested)
        .map_err(|_error| StorageError::WriterQueueCapacityExceeded { requested, maximum })
}

fn open_writer_database(
    config: &StorageConfig,
) -> Result<(rusqlite::Connection, ConnectionSettings), StorageError> {
    #[cfg(test)]
    return open_database_with_busy_timeout(&config.path, config.durability, config.busy_timeout);
    #[cfg(not(test))]
    open_database(&config.path, config.durability)
}
