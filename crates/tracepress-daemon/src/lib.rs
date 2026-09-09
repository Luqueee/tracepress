#![allow(
    clippy::multiple_crate_versions,
    reason = "daemon dependencies currently select distinct platform support versions"
)]

//! Daemon-owned session and causal-operation lifecycle service.

use std::collections::{HashMap, HashSet};

use thiserror::Error;
use tokio::sync::Mutex;
use tracepress_core::{
    CausalEdge, CausalEdgeError, CausalRelationship, OperationId, OperationKind, OperationStatus,
    SessionId, SessionState, UuidV7Generator,
};
use tracepress_storage::{StorageError, StorageWriter, WriteBatch, WriteCommand, WriteReceipt};

/// Failure returned by the daemon lifecycle boundary.
#[allow(
    missing_docs,
    reason = "variant fields repeat the typed identities and states named by each error"
)]
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DaemonError {
    /// Durable state could not be committed by the sole writer.
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// The requested session is unknown to this daemon instance.
    #[error("session {session_id} is not registered")]
    UnknownSession { session_id: SessionId },
    /// The requested operation is unknown or belongs to another session.
    #[error("operation {operation_id} is not registered in session {session_id}")]
    UnknownOperation {
        session_id: SessionId,
        operation_id: OperationId,
    },
    /// A non-active session cannot accept another operation.
    #[error("session {session_id} in state {state:?} cannot accept operations")]
    SessionNotActive {
        session_id: SessionId,
        state: SessionState,
    },
    /// The requested causal edge violates the core DAG contract.
    #[error(transparent)]
    InvalidCausalEdge(#[from] CausalEdgeError),
}

/// Read-only lifecycle view returned to daemon consumers.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub struct SessionSnapshot {
    /// Session identity.
    pub session_id: SessionId,
    /// Current lifecycle state.
    pub state: SessionState,
    /// Session-specific ingress key.
    pub ingress_key: String,
    /// Operations currently registered in this session.
    pub operations: usize,
}

#[derive(Debug)]
struct SessionRecord {
    state: SessionState,
    ingress_key: String,
    operations: HashSet<OperationId>,
}

/// Sole daemon owner of lifecycle memory and durable writes.
#[derive(Debug)]
pub struct DaemonService {
    writer: StorageWriter,
    ids: Mutex<UuidV7Generator>,
    sessions: Mutex<HashMap<SessionId, SessionRecord>>,
    recovered_stale_sessions: u64,
}

impl DaemonService {
    /// Opens a daemon service and marks interrupted sessions from a prior process stale.
    ///
    /// # Errors
    /// Returns a storage error when startup recovery cannot commit.
    pub async fn open(writer: StorageWriter, recovered_at: &str) -> Result<Self, DaemonError> {
        let recovered_stale_sessions = match writer.recover_stale_sessions(recovered_at).await? {
            WriteReceipt::Committed { rows_changed }
            | WriteReceipt::BatchCommitted { rows_changed } => rows_changed,
            WriteReceipt::EventAppended { sequence: _ } => 0,
        };
        let service = Self {
            writer,
            ids: Mutex::new(UuidV7Generator::new()),
            sessions: Mutex::new(HashMap::new()),
            recovered_stale_sessions,
        };
        tracing::info!(
            recovered_stale_sessions,
            "daemon startup recovery completed"
        );
        Ok(service)
    }

    /// Returns how many interrupted sessions startup recovery marked stale.
    #[must_use]
    pub const fn recovered_stale_sessions(&self) -> u64 {
        self.recovered_stale_sessions
    }

    /// Creates and durably registers an isolated active session.
    ///
    /// # Errors
    /// Returns a storage error when the session cannot be committed.
    pub async fn start_session(&self, started_at: &str) -> Result<SessionSnapshot, DaemonError> {
        let ids = self.ids.lock().await;
        let session_id = SessionId::generate(&ids);
        drop(ids);
        let ingress_key = format!("session-{session_id}");
        let _receipt = self
            .writer
            .submit(WriteCommand::Session {
                session_id,
                started_at: started_at.to_owned(),
                ended_at: None,
                state: SessionState::Active,
                ingress_key: ingress_key.clone(),
            })
            .await?;
        tracing::info!(%session_id, %ingress_key, "session started");
        let record = SessionRecord {
            state: SessionState::Active,
            ingress_key: ingress_key.clone(),
            operations: HashSet::new(),
        };
        let previous = self.sessions.lock().await.insert(session_id, record);
        debug_assert!(previous.is_none());
        Ok(SessionSnapshot {
            session_id,
            state: SessionState::Active,
            ingress_key,
            operations: 0,
        })
    }

    /// Creates one operation and optionally commits its parent edge atomically.
    ///
    /// # Errors
    /// Returns a typed lifecycle, causal-edge, or storage error.
    #[allow(
        clippy::too_many_arguments,
        clippy::significant_drop_tightening,
        reason = "the lifecycle transaction keeps its typed request fields and session lock atomic"
    )]
    pub async fn create_operation(
        &self,
        session_id: SessionId,
        kind: OperationKind,
        started_at: &str,
        parent: Option<(OperationId, CausalRelationship)>,
    ) -> Result<OperationId, DaemonError> {
        let mut sessions = self.sessions.lock().await;
        let record = sessions
            .get_mut(&session_id)
            .ok_or(DaemonError::UnknownSession { session_id })?;
        if record.state != SessionState::Active {
            return Err(DaemonError::SessionNotActive {
                session_id,
                state: record.state,
            });
        }
        if let Some((parent_id, _relationship)) = parent {
            if !record.operations.contains(&parent_id) {
                return Err(DaemonError::UnknownOperation {
                    session_id,
                    operation_id: parent_id,
                });
            }
        }
        let ids = self.ids.lock().await;
        let operation_id = OperationId::generate(&ids);
        drop(ids);
        let operation = WriteCommand::Operation {
            operation_id,
            session_id,
            kind,
            started_at: started_at.to_owned(),
            ended_at: None,
            status: OperationStatus::Started,
        };
        let batch = match parent {
            Some((parent_id, relationship)) => {
                WriteBatch::new(operation).and(WriteCommand::CausalEdge {
                    edge: CausalEdge::new(parent_id, operation_id, relationship)?,
                })
            }
            None => WriteBatch::new(operation),
        };
        let _receipt = self.writer.submit_batch(batch).await?;
        tracing::info!(%session_id, %operation_id, ?kind, "operation started");
        let inserted = record.operations.insert(operation_id);
        debug_assert!(inserted);
        Ok(operation_id)
    }

    /// Transitions an active session to a terminal state after durable commit.
    ///
    /// # Errors
    /// Returns a typed error for an unknown/non-active session or failed durable update.
    #[allow(
        clippy::too_many_arguments,
        clippy::significant_drop_tightening,
        reason = "the terminal transition keeps identity, state, timestamp, and lock atomic"
    )]
    pub async fn finish_session(
        &self,
        session_id: SessionId,
        state: SessionState,
        ended_at: &str,
    ) -> Result<SessionSnapshot, DaemonError> {
        let mut sessions = self.sessions.lock().await;
        let record = sessions
            .get_mut(&session_id)
            .ok_or(DaemonError::UnknownSession { session_id })?;
        if record.state != SessionState::Active && record.state != SessionState::Closing {
            return Err(DaemonError::SessionNotActive {
                session_id,
                state: record.state,
            });
        }
        let _receipt = self
            .writer
            .submit(WriteCommand::SessionState {
                session_id,
                ended_at: Some(ended_at.to_owned()),
                state,
            })
            .await?;
        record.state = state;
        tracing::info!(%session_id, ?state, "session finished");
        Ok(snapshot(session_id, record))
    }

    /// Returns the current in-memory state for one session.
    ///
    /// # Errors
    /// Returns an error when the session is unknown.
    pub async fn session(&self, session_id: SessionId) -> Result<SessionSnapshot, DaemonError> {
        self.sessions
            .lock()
            .await
            .get(&session_id)
            .map(|record| snapshot(session_id, record))
            .ok_or(DaemonError::UnknownSession { session_id })
    }

    /// Drains durable writes and stops the service.
    ///
    /// # Errors
    /// Returns a typed storage error if the writer cannot drain or join.
    pub async fn shutdown(self) -> Result<(), DaemonError> {
        self.writer.shutdown().await.map_err(Into::into)
    }
}

fn snapshot(session_id: SessionId, record: &SessionRecord) -> SessionSnapshot {
    SessionSnapshot {
        session_id,
        state: record.state,
        ingress_key: record.ingress_key.clone(),
        operations: record.operations.len(),
    }
}

/// Minimal authenticated control protocol used by the CLI.
#[allow(
    clippy::exhaustive_enums,
    missing_docs,
    reason = "the private CLI/daemon wire protocol has an explicit versioned boundary"
)]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub enum ControlRequest {
    /// Returns daemon liveness and recovery information.
    Status,
    /// Requests an orderly daemon shutdown.
    Shutdown,
    /// Starts a session owned by the caller.
    StartSession { started_at: String },
    /// Records one provider forwarding operation beneath the agent root.
    RecordForward {
        session_id: SessionId,
        parent_operation_id: OperationId,
        observed_at: String,
    },
    /// Finishes a previously started session.
    FinishSession {
        session_id: SessionId,
        ended_at: String,
    },
}

/// Response returned by the daemon control protocol.
#[allow(
    clippy::exhaustive_enums,
    missing_docs,
    reason = "the private CLI/daemon wire protocol has an explicit versioned boundary"
)]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ControlResponse {
    /// Successful response with optional lifecycle data.
    Ok {
        /// Human-readable daemon state.
        state: String,
        /// Session snapshot when a session was started.
        session: Option<SessionSnapshot>,
        /// Operation created by the request, when applicable.
        operation_id: Option<OperationId>,
    },
    /// Request failed inside the daemon.
    Error { message: String },
}

impl ControlResponse {
    /// Creates a successful status response.
    #[must_use]
    pub fn ok(state: impl Into<String>) -> Self {
        Self::Ok {
            state: state.into(),
            session: None,
            operation_id: None,
        }
    }
}
