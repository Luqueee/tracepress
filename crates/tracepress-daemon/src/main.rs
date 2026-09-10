#![allow(
    clippy::arithmetic_side_effects,
    clippy::manual_let_else,
    clippy::too_many_lines,
    missing_docs,
    reason = "the process boundary keeps exhaustive wire handling and startup ordering explicit"
)]

use std::{path::PathBuf, sync::Arc};

use tokio_util::sync::CancellationToken;
use tracepress_core::{
    MaxIpcFrameBytes, MaxIpcQueueItems, MaxRequestBodyBytes, MaxResponseBodyBytes, SessionState,
};
use tracepress_daemon::{
    ContextAnalysisBegin, ContextBlockBatch, ControlRequest, ControlResponse, DaemonService,
    RecordContextAnalysisDropped, RecordCorrelationDegradation, RecordProviderObservation,
};
use tracepress_ipc::{
    Credential, IpcLimits, IpcResponse, IpcTransport, ResponseOutcome, SocketOwner, UnixBinding,
    UnixEndpoint, UnixTransport, UnixTransportConfig,
};
use tracepress_storage::{Durability, StorageConfig, StorageWriter};

const DEFAULT_WRITER_QUEUE_ITEMS: u64 = 128;
const IPC_FRAME_BYTES: u64 = 65_536;
const IPC_BODY_BYTES: u64 = 32_768;

fn read_hex(
    path: &std::path::Path,
    expected: usize,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(path)?;
    let text = text.trim();
    if text.len() != expected * 2 {
        return Err(format!("{} must contain {expected} bytes as hex", path.display()).into());
    }
    let mut bytes = Vec::with_capacity(expected);
    for pair in text.as_bytes().chunks_exact(2) {
        bytes.push(u8::from_str_radix(std::str::from_utf8(pair)?, 16)?);
    }
    Ok(bytes)
}

async fn handle_request(
    daemon: &Arc<DaemonService>,
    request: ControlRequest,
) -> (ControlResponse, bool) {
    match request {
        ControlRequest::Status => (ControlResponse::ok("running"), false),
        ControlRequest::Shutdown => (ControlResponse::ok("shutting_down"), true),
        ControlRequest::StartSession { started_at } => {
            match daemon.start_session(&started_at).await {
                Ok(session) => {
                    match daemon
                        .create_operation(
                            session.session_id,
                            tracepress_core::OperationKind::Agent,
                            &started_at,
                            None,
                        )
                        .await
                    {
                        Ok(operation_id) => match daemon.session(session.session_id).await {
                            Ok(session) => (
                                ControlResponse::Ok {
                                    state: "running".to_owned(),
                                    session: Some(session),
                                    operation_id: Some(operation_id),
                                    provider_request_id: None,
                                    attempt_id: None,
                                    inference_operation_id: None,
                                    context_snapshot_id: None,
                                },
                                false,
                            ),
                            Err(error) => (
                                ControlResponse::Error {
                                    message: error.to_string(),
                                },
                                false,
                            ),
                        },
                        Err(error) => (
                            ControlResponse::Error {
                                message: error.to_string(),
                            },
                            false,
                        ),
                    }
                }
                Err(error) => (
                    ControlResponse::Error {
                        message: error.to_string(),
                    },
                    false,
                ),
            }
        }
        ControlRequest::RecordForward {
            session_id,
            parent_operation_id,
            observed_at,
        } => {
            match daemon
                .create_operation(
                    session_id,
                    tracepress_core::OperationKind::LlmInference,
                    &observed_at,
                    Some((
                        parent_operation_id,
                        tracepress_core::CausalRelationship::Spawned,
                    )),
                )
                .await
            {
                Ok(operation_id) => (
                    ControlResponse::Ok {
                        state: "running".to_owned(),
                        session: None,
                        operation_id: Some(operation_id),
                        provider_request_id: None,
                        attempt_id: None,
                        inference_operation_id: None,
                        context_snapshot_id: None,
                    },
                    false,
                ),
                Err(error) => (
                    ControlResponse::Error {
                        message: error.to_string(),
                    },
                    false,
                ),
            }
        }
        ControlRequest::RecordProviderObservation {
            session_id,
            parent_operation_id,
            observation,
        } => {
            match daemon
                .record_provider_observation(RecordProviderObservation::new(
                    session_id,
                    parent_operation_id,
                    *observation,
                ))
                .await
            {
                Ok(recorded) => (
                    ControlResponse::Ok {
                        state: "running".to_owned(),
                        session: None,
                        operation_id: Some(recorded.operation_id),
                        provider_request_id: Some(recorded.receipt.request_id),
                        attempt_id: Some(recorded.receipt.attempt_id),
                        inference_operation_id: Some(recorded.operation_id),
                        context_snapshot_id: None,
                    },
                    false,
                ),
                Err(error) => (
                    ControlResponse::Error {
                        message: error.to_string(),
                    },
                    false,
                ),
            }
        }
        ControlRequest::BeginContextAnalysis {
            session_id,
            provider_request_id,
            inference_operation_id,
            analysis_version,
            started_at_us,
        } => match daemon
            .begin_context_analysis(ContextAnalysisBegin::new(
                session_id,
                provider_request_id,
                inference_operation_id,
                analysis_version,
                started_at_us,
            ))
            .await
        {
            Ok(snapshot_id) => (
                ControlResponse::Ok {
                    state: "running".to_owned(),
                    session: None,
                    operation_id: None,
                    provider_request_id: None,
                    attempt_id: None,
                    inference_operation_id: None,
                    context_snapshot_id: Some(snapshot_id),
                },
                false,
            ),
            Err(error) => (
                ControlResponse::Error {
                    message: error.to_string(),
                },
                false,
            ),
        },
        ControlRequest::AppendContextBlocks {
            snapshot_id,
            sequence,
            blocks,
        } => match daemon
            .append_context_blocks(ContextBlockBatch::new(snapshot_id, sequence, blocks))
            .await
        {
            Ok(()) => (ControlResponse::ok("running"), false),
            Err(error) => (
                ControlResponse::Error {
                    message: error.to_string(),
                },
                false,
            ),
        },
        ControlRequest::FinalizeContextAnalysis { summary } => {
            match daemon.finalize_context_analysis(*summary).await {
                Ok(()) => (ControlResponse::ok("running"), false),
                Err(error) => (
                    ControlResponse::Error {
                        message: error.to_string(),
                    },
                    false,
                ),
            }
        }
        ControlRequest::RecordCorrelationDegradation {
            session_id,
            reason,
            observed_at,
        } => {
            match daemon
                .record_correlation_degradation(RecordCorrelationDegradation::new(
                    session_id,
                    reason,
                    observed_at,
                ))
                .await
            {
                Ok(()) => (ControlResponse::ok("running"), false),
                Err(error) => (
                    ControlResponse::Error {
                        message: error.to_string(),
                    },
                    false,
                ),
            }
        }
        ControlRequest::RecordContextAnalysisDropped {
            session_id,
            reason,
            dropped,
            observed_at_us,
        } => {
            let input = match RecordContextAnalysisDropped::builder()
                .session_id(session_id)
                .reason(reason)
                .dropped(dropped)
                .observed_at_us(observed_at_us)
                .build()
            {
                Ok(input) => input,
                Err(error) => {
                    return (
                        ControlResponse::Error {
                            message: error.to_string(),
                        },
                        false,
                    );
                }
            };
            match daemon.record_context_analysis_dropped(input).await {
                Ok(()) => (ControlResponse::ok("running"), false),
                Err(error) => (
                    ControlResponse::Error {
                        message: error.to_string(),
                    },
                    false,
                ),
            }
        }
        ControlRequest::Context { request_id } => match daemon.inspect_context(request_id).await {
            Ok(inspection) => (
                ControlResponse::Context {
                    inspection: Box::new(inspection),
                },
                false,
            ),
            Err(error) => (
                ControlResponse::Error {
                    message: error.to_string(),
                },
                false,
            ),
        },
        ControlRequest::FinishSession {
            session_id,
            ended_at,
        } => {
            match daemon
                .finish_session(session_id, SessionState::Closed, &ended_at)
                .await
            {
                Ok(session) => (
                    ControlResponse::Ok {
                        state: "running".to_owned(),
                        session: Some(session),
                        operation_id: None,
                        provider_request_id: None,
                        attempt_id: None,
                        inference_operation_id: None,
                        context_snapshot_id: None,
                    },
                    false,
                ),
                Err(error) => (
                    ControlResponse::Error {
                        message: error.to_string(),
                    },
                    false,
                ),
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let database = std::env::var_os("TRACEPRESS_DATABASE")
        .map(PathBuf::from)
        .ok_or("TRACEPRESS_DATABASE must name the daemon-owned SQLite file")?;
    let socket = std::env::var_os("TRACEPRESS_CONTROL_SOCKET")
        .map(PathBuf::from)
        .ok_or("TRACEPRESS_CONTROL_SOCKET must name the private Unix socket")?;
    let credential_path = std::env::var_os("TRACEPRESS_CONTROL_CREDENTIAL")
        .map(PathBuf::from)
        .ok_or("TRACEPRESS_CONTROL_CREDENTIAL must name the credential file")?;
    if let Some(parent) = database.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let credential_bytes: [u8; 32] = read_hex(&credential_path, 32)?
        .try_into()
        .map_err(|_| "invalid credential")?;
    let credential = Credential::new(credential_bytes);
    let owner_bytes = credential_bytes
        .get(..16)
        .and_then(|bytes| <[u8; 16]>::try_from(bytes).ok())
        .ok_or("credential cannot derive socket owner")?;
    let owner = SocketOwner::new(owner_bytes);
    let limits = IpcLimits::new(
        MaxIpcFrameBytes::new(IPC_FRAME_BYTES)?,
        MaxRequestBodyBytes::new(IPC_BODY_BYTES)?,
        MaxResponseBodyBytes::new(IPC_BODY_BYTES)?,
    );
    let transport = UnixTransport::bind(UnixTransportConfig::authenticated(
        UnixBinding::new(UnixEndpoint::new(socket)?, owner),
        credential,
        limits,
    ))?;
    let queue_items = MaxIpcQueueItems::new(DEFAULT_WRITER_QUEUE_ITEMS)?;
    let writer = StorageWriter::open(StorageConfig::new(
        database,
        Durability::Strict,
        queue_items,
    ))
    .await?;
    let daemon = Arc::new(DaemonService::open(writer, "daemon-startup").await?);
    if let Some(readiness) = std::env::var_os("TRACEPRESS_DAEMON_READY") {
        std::fs::write(readiness, b"ready\n")?;
    }
    let cancellation = CancellationToken::new();
    let (shutdown_sender, mut shutdown_receiver) = tokio::sync::mpsc::channel::<()>(1);
    let mut connections = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            result = tokio::signal::ctrl_c() => {
                result?;
                break;
            }
            signal = shutdown_receiver.recv() => {
                if signal.is_some() {
                    break;
                }
            }
            accepted = transport.accept(&cancellation) => {
                let mut connection = accepted?;
                let connection_cancellation = cancellation.clone();
                let connection_daemon = Arc::clone(&daemon);
                let connection_shutdown = shutdown_sender.clone();
                let _connection_task = connections.spawn(async move {
                    let request = match tokio::time::timeout(
                        std::time::Duration::from_secs(2),
                        connection.receive_request(&connection_cancellation),
                    )
                    .await
                    {
                        Ok(Ok(request)) => request,
                        Ok(Err(_)) | Err(_) => return,
                    };
                    let Ok(command) = serde_json::from_slice::<ControlRequest>(request.body()) else {
                        return;
                    };
                    let (body, stop) = handle_request(&connection_daemon, command).await;
                    let Ok(bytes) = serde_json::to_vec(&body) else {
                        return;
                    };
                    let Ok(outcome) =
                        ResponseOutcome::complete(bytes, limits.maximum_response_body())
                    else {
                        return;
                    };
                    if connection
                        .send_response(
                            &IpcResponse::new(request.request_id(), outcome),
                            &connection_cancellation,
                        )
                        .await
                        .is_ok()
                        && stop
                    {
                        let _sent = connection_shutdown.send(()).await;
                    }
                });
            }
        }
    }
    cancellation.cancel();
    connections.abort_all();
    while connections.join_next().await.is_some() {}
    transport.close()?;
    drop(shutdown_sender);
    match Arc::try_unwrap(daemon) {
        Ok(daemon) => daemon.shutdown().await?,
        Err(_) => return Err("daemon control connection still active".into()),
    }
    Ok(())
}
