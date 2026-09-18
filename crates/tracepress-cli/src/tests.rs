use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use super::{
    AnalysisSequence, BackgroundTaskSpawner, CONTEXT_INGESTION_QUEUE_HARD_CAP, CommandFamily,
    CompressionCandidateId, Config, ContextAnalysisDropReason, ContextAnalysisInput,
    ContextAnalysisStatus, ContextCounters, ContextReceipt, ContextSnapshotId,
    ContextSnapshotStatus, ControlRequest, CorrelationCounters, CorrelationStatus,
    DeferredAnalysisMetrics, DurableEventIngress, ObservationRecord, OperationId,
    PendingAnalysisEvidence, RECORDER_QUEUE_ITEMS, RequestId, RunRecorder, SessionId,
    ShadowCacheRisk, ShadowCandidateRecord, ShadowCandidateStatus, SourceExecutionId,
    SourceRecoveryMetadata, SourceRecoveryWrite, TERMINAL_ANALYSIS_IDENTITIES, UuidV7Generator,
    active_source_reducer, bounded_record_provider_observation_request,
    cleanup_expired_source_recoveries, configure_codex_subscription,
    context_ingestion_queue_capacity, context_metrics, measurement_metadata_line,
    reconcile_pending_context_with, safe_source_recovery_executable, scheduler_metrics_line,
    shadow_candidate_batch_fits, source_policy_fail_open_reason, source_recovery_token,
    source_reducer_id, store_source_recovery, valid_source_recovery_token,
};

use tracepress_provider::{ObservationInput, ObservationLimits, parse_request, parse_response};
use tracepress_proxy::{ContextAnalysisOutcome, ForwardId};

#[test]
fn complete_context_without_tool_definitions_records_known_zero_schema_tokens() {
    let (metrics, _) = context_metrics(&[], ContextAnalysisStatus::Complete, None);

    assert_eq!(metrics.tool_count, Some(0));
    assert_eq!(metrics.schema_bytes, Some(0));
    assert_eq!(metrics.estimated_schema_tokens, Some(0));
    assert_eq!(metrics.repeated_schema_tokens, Some(0));
}

#[test]
fn rejected_source_policies_cannot_select_an_active_reducer() {
    assert!(
        active_source_reducer(Some("cargo_test_v1_active"), CommandFamily::CargoTest).is_some()
    );
    assert!(
        active_source_reducer(Some("cargo_check_v2_active"), CommandFamily::CargoCheck).is_some()
    );
    assert!(active_source_reducer(Some("rg_v1_active"), CommandFamily::Ripgrep).is_some());
    assert!(
        active_source_reducer(Some("cargo_check_v1_active"), CommandFamily::CargoCheck).is_none()
    );
    assert!(
        active_source_reducer(Some("git_status_v1_active"), CommandFamily::GitStatus).is_none()
    );
}

#[test]
fn source_policy_fail_open_reasons_are_explicit() {
    assert_eq!(
        source_policy_fail_open_reason(Some("git_status_v1_active"), CommandFamily::GitStatus),
        Some("policy_rejected")
    );
    assert_eq!(
        source_policy_fail_open_reason(Some("rg_v1_active"), CommandFamily::CargoTest),
        Some("policy_family_mismatch")
    );
    assert_eq!(
        source_policy_fail_open_reason(Some("unknown"), CommandFamily::CargoTest),
        Some("unknown_reducer")
    );
    assert_eq!(
        source_policy_fail_open_reason(Some("cargo_test_v1_active"), CommandFamily::CargoTest),
        None
    );
    assert_eq!(
        source_policy_fail_open_reason(None, CommandFamily::CargoTest),
        None
    );
    assert_eq!(
        source_reducer_id(Some("git_status_v1_active")),
        "git_status_v1_active"
    );
    assert_eq!(source_reducer_id(Some("unknown")), "passthrough");
}

#[test]
#[allow(
    clippy::expect_used,
    reason = "test setup needs immediate diagnostics for operating-system randomness"
)]
fn source_recovery_tokens_are_random_opaque_and_path_safe() {
    let mut tokens = std::collections::BTreeSet::new();
    for _ in 0..32 {
        let token = source_recovery_token().expect("random source recovery token");
        assert!(valid_source_recovery_token(&token));
        assert!(tokens.insert(token));
    }
    assert!(safe_source_recovery_executable(
        "/opt/tracepress/bin/tracepress-v1"
    ));
    assert!(!safe_source_recovery_executable("tracepress; touch pwned"));
    assert!(!safe_source_recovery_executable("$(tracepress)"));
}

#[test]
#[allow(
    clippy::expect_used,
    reason = "test fixture setup and byte-for-byte file assertions need local diagnostics"
)]
fn source_recovery_store_is_byte_faithful_and_private() {
    let temporary = tempfile::tempdir().expect("temporary recovery root");
    let root = temporary.path().join("tracepress");
    let config = Config {
        database: root.join("tracepress.sqlite3"),
        socket: root.join("tracepress.sock"),
        credential: root.join("control.cred"),
        ready: root.join("daemon.ready"),
        root,
    };
    config.ensure_root().expect("secure Tracepress root");
    let ids = UuidV7Generator::new();
    let session_id = SessionId::generate(&ids).to_string();
    let token = source_recovery_token().expect("random recovery token");
    let stdout = b"stdout\0bytes\n";
    let stderr = b"stderr\xffbytes\n";
    store_source_recovery(
        &config,
        SourceRecoveryWrite {
            session_id: &session_id,
            token: &token,
            source_execution_id: SourceExecutionId::generate(&ids),
            stdout,
            stderr,
        },
    )
    .expect("store source recovery");
    let recovery = config
        .root
        .join("source-recovery")
        .join(session_id)
        .join(token);
    assert_eq!(
        std::fs::read(recovery.join("stdout")).expect("stdout"),
        stdout
    );
    assert_eq!(
        std::fs::read(recovery.join("stderr")).expect("stderr"),
        stderr
    );
    let mut metadata: SourceRecoveryMetadata =
        serde_json::from_slice(&std::fs::read(recovery.join("metadata.json")).expect("metadata"))
            .expect("valid metadata");
    assert_eq!(
        metadata.stdout_bytes,
        u64::try_from(stdout.len()).unwrap_or(u64::MAX)
    );
    assert_eq!(
        metadata.stderr_bytes,
        u64::try_from(stderr.len()).unwrap_or(u64::MAX)
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            std::fs::metadata(recovery.join("stdout"))
                .expect("stdout metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    metadata.expires_at_unix = 0;
    std::fs::write(
        recovery.join("metadata.json"),
        serde_json::to_vec(&metadata).expect("encode expired metadata"),
    )
    .expect("expire recovery fixture");
    let session_root = recovery.parent().expect("session recovery root");
    cleanup_expired_source_recoveries(session_root, 1);
    assert!(!recovery.exists());
}

fn synthetic_shadow_candidate(
    generator: &UuidV7Generator,
    provider_readability: String,
) -> ShadowCandidateRecord {
    ShadowCandidateRecord {
        candidate_id: CompressionCandidateId::generate(generator),
        experiment_id: "test-shadow-batching".to_owned(),
        snapshot_id: ContextSnapshotId::generate(generator),
        block_ordinal: 0,
        compressor_id: "json.noop".to_owned(),
        compressor_version: "1".to_owned(),
        status: ShadowCandidateStatus::NoImprovement,
        input_bytes: 128,
        output_bytes: Some(128),
        bytes_delta: Some(0),
        input_estimated_tokens: Some(32),
        output_estimated_tokens: Some(32),
        estimated_token_delta: Some(0),
        processing_us: 1,
        reversible: true,
        recovery_verified: true,
        deterministic: true,
        original_fingerprint: vec![0_u8; 32].into_boxed_slice(),
        candidate_fingerprint: Some(vec![0_u8; 32].into_boxed_slice()),
        recovered_fingerprint: Some(vec![0_u8; 32].into_boxed_slice()),
        first_modified_offset: None,
        preserved_prefix_bytes: Some(128),
        preserved_prefix_ratio_basis_points: Some(10_000),
        cache_risk: ShadowCacheRisk::Unknown,
        provider_readability,
        json_root_kind: None,
        json_array_length_bucket: None,
        json_object_key_count_bucket: None,
        json_homogeneity_basis_points: None,
        json_primitive_cell_ratio_basis_points: None,
        json_nested_cell_ratio_basis_points: None,
        text_shape: None,
        verified_at_us: Some(1),
    }
}

#[test]
fn shadow_candidate_batches_are_bounded_by_ipc_body_size() {
    let generator = UuidV7Generator::new();
    let candidate = synthetic_shadow_candidate(&generator, "r".repeat(1_000));
    assert!(shadow_candidate_batch_fits(std::slice::from_ref(
        &candidate
    )));
    assert!(!shadow_candidate_batch_fits(&vec![candidate; 16]));
}

#[test]
fn codex_subscription_overrides_follow_the_exec_subcommand() {
    let mut args = vec!["exec".to_owned(), "--ephemeral".to_owned()];
    configure_codex_subscription(
        &mut args,
        std::net::SocketAddr::from(([127, 0, 0, 1], 43_191)),
    );

    assert_eq!(args[0], "exec");
    assert_eq!(args[1], "-c");
    assert_eq!(args[2], "model_provider=tracepress_subscription");
    assert!(args.iter().any(|argument| {
        argument == "model_providers.tracepress_subscription.base_url=\"http://127.0.0.1:43191/v1\""
    }));
    assert_eq!(args[13], "--ephemeral");
}

#[test]
fn measurement_lines_bind_scheduler_metrics_to_run_identity()
-> Result<(), Box<dyn std::error::Error>> {
    let session_id = SessionId::generate(&UuidV7Generator::new());
    let metadata = measurement_metadata_line(Some("run-123"), session_id, "unix-ms:1")
        .ok_or_else(|| std::io::Error::other("bounded run id should produce metadata"))?;
    let metadata_payload = metadata
        .strip_prefix("TRACEPRESS_MEASUREMENT_METADATA=")
        .ok_or_else(|| std::io::Error::other("metadata prefix"))?;
    let metadata_json: serde_json::Value = serde_json::from_str(metadata_payload)?;
    assert_eq!(metadata_json["measurement_run_id"], "run-123");
    assert_eq!(metadata_json["session_id"], session_id.to_string());
    assert_eq!(metadata_json["started_at"], "unix-ms:1");
    assert!(metadata_json["tracepress_pid"].as_u64().is_some());

    let metrics = {
        let mut value = DeferredAnalysisMetrics::default();
        value.queue_items = 0;
        value.queue_bytes = 0;
        value.high_water_items = 2;
        value.high_water_bytes = 128;
        value.deferred_total = 7;
        value.deferred_due_to_active_forwards_total = 3;
        value.processed_deferred_total = 7;
        value.backlog_capacity_drops = 0;
        value.analysis_wait_us = 42;
        scheduler_metrics_line(&value)
    };
    let metrics_payload = metrics
        .strip_prefix("TRACEPRESS_SCHEDULER_METRICS=")
        .ok_or_else(|| std::io::Error::other("metrics prefix"))?;
    let metrics_json: serde_json::Value = serde_json::from_str(metrics_payload)?;
    assert_eq!(metrics_json["analysis_admitted_total"], 7);
    assert_eq!(metrics_json["analysis_deferred_total"], 3);
    assert_eq!(metrics_json["processed_deferred_total"], 7);
    assert_eq!(metrics_json["deferred_queue_items"], 0);
    Ok(())
}

#[test]
fn context_ingestion_queue_is_hard_capped_without_zero_capacity() {
    assert_eq!(context_ingestion_queue_capacity(0), 1);
    assert_eq!(context_ingestion_queue_capacity(1), 1);
    assert_eq!(
        context_ingestion_queue_capacity(CONTEXT_INGESTION_QUEUE_HARD_CAP),
        CONTEXT_INGESTION_QUEUE_HARD_CAP
    );
    assert_eq!(
        context_ingestion_queue_capacity(CONTEXT_INGESTION_QUEUE_HARD_CAP + 1),
        CONTEXT_INGESTION_QUEUE_HARD_CAP
    );
    assert_eq!(context_ingestion_queue_capacity(128), 4);
}

#[tokio::test]
async fn durable_event_ingress_defers_when_recorder_channel_is_full() {
    let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
    let ingress = DurableEventIngress::new(sender.clone(), BackgroundTaskSpawner::new(), |_| {});

    assert!(sender.send(1).await.is_ok());
    assert!(ingress.try_send(2).is_ok());

    let first = receiver.recv().await.unwrap_or(u64::MAX);
    let second = match tokio::time::timeout(Duration::from_secs(1), receiver.recv()).await {
        Ok(Some(event)) => event,
        _ => u64::MAX,
    };
    assert_eq!(first, 1);
    assert_eq!(second, 2);
}

#[tokio::test]
async fn durable_event_ingress_accounts_events_when_recorder_closes() {
    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    let dropped = Arc::new(AtomicU64::new(0));
    let dropped_for_handler = Arc::clone(&dropped);
    let ingress = DurableEventIngress::new(
        sender.clone(),
        BackgroundTaskSpawner::new(),
        move |_event: u64| {
            let _ = dropped_for_handler.fetch_add(1, Ordering::Relaxed);
        },
    );

    assert!(sender.send(1).await.is_ok());
    assert!(ingress.try_send(2).is_ok());
    assert!(ingress.try_send(3).is_ok());
    drop(receiver);

    let result = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if dropped.load(Ordering::Relaxed) == 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(result.is_ok());
    assert!(ingress.try_send(4).is_err());
}

#[tokio::test]
async fn request_admission_waits_for_bounded_ingress_space() {
    let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
    let ingress = DurableEventIngress::new(sender.clone(), BackgroundTaskSpawner::new(), |_| {});

    assert!(sender.send(0).await.is_ok());
    {
        let mut state = ingress
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for event in 1..=RECORDER_QUEUE_ITEMS {
            state.events.push_back(event as u64);
        }
    }
    assert_eq!(ingress.queued_len(), RECORDER_QUEUE_ITEMS);

    let waiting_ingress = Arc::clone(&ingress);
    let waiting = tokio::task::spawn_blocking(move || {
        waiting_ingress.send_request((RECORDER_QUEUE_ITEMS + 1) as u64)
    });
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(!waiting.is_finished());

    {
        let mut state = ingress
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _removed = state.events.pop_front();
    }
    ingress.space.notify_one();

    for _ in 0..=RECORDER_QUEUE_ITEMS {
        assert!(receiver.recv().await.is_some());
    }
    assert!(
        tokio::time::timeout(Duration::from_secs(1), waiting)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn pending_context_receipt_wins_over_retired_identity() {
    let ids = UuidV7Generator::new();
    let forward = ForwardId::default();
    let counters = Arc::new(ContextCounters::default());
    let (context_sender, _context_receiver) = tokio::sync::mpsc::channel(1);
    let mut recorder = RunRecorder {
        config: Config {
            root: std::path::PathBuf::new(),
            database: std::path::PathBuf::new(),
            socket: std::path::PathBuf::new(),
            credential: std::path::PathBuf::new(),
            ready: std::path::PathBuf::new(),
        },
        session_id: SessionId::generate(&ids),
        parent_operation_id: OperationId::generate(&ids),
        context_sender,
        context_counters: Arc::clone(&counters),
        forwards: std::collections::BTreeMap::new(),
        analysis_enabled: true,
        retired_orphans: std::collections::BTreeMap::new(),
        pending_context: std::collections::BTreeMap::new(),
        retired: std::collections::BTreeSet::from([forward]),
        retired_through: None,
        counters: Arc::new(CorrelationCounters::default()),
    };
    let _previous = recorder.pending_context.insert(
        forward,
        ContextReceipt {
            forward,
            provider_request_id: RequestId::generate(&ids),
            attempt_id: tracepress_core::AttemptId::generate(&ids),
            inference_operation_id: OperationId::generate(&ids),
            provider_input_tokens: None,
            provider_usage_comparable: false,
            correlation: CorrelationStatus::Correlated,
        },
    );

    recorder
        .context_analysis(ContextAnalysisInput {
            forward,
            outcome: ContextAnalysisOutcome::Dropped(ContextAnalysisDropReason::Malformed),
            shadow_body: None,
            analysis_permit: None,
        })
        .await;

    assert!(recorder.pending_context.is_empty());
    assert_eq!(counters.analysis_requests_seen.load(Ordering::Relaxed), 1);
    assert_eq!(
        counters.analysis_requests_dropped.load(Ordering::Relaxed),
        1
    );
    assert_eq!(counters.correlation_degraded.load(Ordering::Relaxed), 0);
}

#[test]
fn oversized_provider_raw_usage_is_omitted_at_the_ipc_boundary() {
    let padding = "x".repeat(12_000);
    let response_body = format!(
        "{{\"id\":\"resp_1\",\"model\":\"gpt-5.6-luna\",\"status\":\"completed\",\"usage\":{{\"input_tokens\":7,\"output_tokens\":3,\"total_tokens\":10,\"padding\":\"{padding}\"}}}}"
    );
    let limits = ObservationLimits::default();
    let request = parse_request(ObservationInput::new(
        br#"{"model":"gpt-5.6-luna","stream":true}"#,
        limits,
    ));
    let response = parse_response(ObservationInput::new(response_body.as_bytes(), limits));
    assert!(response.raw_usage.is_some());

    let ids = UuidV7Generator::new();
    let observation = ObservationRecord::new(42, request, "unix-ms:1")
        .with_response(response)
        .with_ended_at("unix-ms:2");
    let control_request = bounded_record_provider_observation_request(
        SessionId::generate(&ids),
        OperationId::generate(&ids),
        observation,
    );

    let frame_fits = serde_json::to_vec(&control_request)
        .ok()
        .and_then(|body| {
            super::MaxRequestBodyBytes::new(super::BODY_BYTES)
                .ok()
                .and_then(|maximum| {
                    super::IpcRequest::new(RequestId::generate(&ids), body, maximum).ok()
                })
        })
        .and_then(|request| serde_json::to_vec(&request).ok())
        .map(|frame| u64::try_from(frame.len()).is_ok_and(|length| length <= super::FRAME_BYTES));
    assert_eq!(frame_fits, Some(true));
    let retained = match &control_request {
        ControlRequest::RecordProviderObservation { observation, .. } => {
            observation.response.as_ref().map(|response| {
                (
                    response.raw_usage.is_none(),
                    response
                        .normalized_usage
                        .as_ref()
                        .and_then(|usage| usage.input_total),
                )
            })
        }
        _ => None,
    };
    assert_eq!(retained, Some((true, Some(7))));
}

#[test]
fn context_lifecycle_bounds_reordered_terminal_holes() {
    let counters = ContextCounters::default();
    let total = u64::try_from(TERMINAL_ANALYSIS_IDENTITIES).unwrap_or(u64::MAX) + 32;
    for ordinal in 0..total {
        counters.ensure_seen_sequence(AnalysisSequence(ordinal));
    }
    for ordinal in (1..total).rev() {
        counters.complete_sequence(AnalysisSequence(ordinal));
    }

    let (terminal_through, terminal_holes_len) = {
        let lifecycle = counters
            .lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (lifecycle.terminal_through, lifecycle.terminal_holes.len())
    };
    assert_eq!(terminal_through, None);
    assert_eq!(terminal_holes_len, TERMINAL_ANALYSIS_IDENTITIES);

    counters.complete_sequence(AnalysisSequence(0));
    counters.complete_sequence(AnalysisSequence(0));
    let (terminal_through, terminal_holes_len) = {
        let lifecycle = counters
            .lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (lifecycle.terminal_through, lifecycle.terminal_holes.len())
    };
    assert_eq!(terminal_through, Some(AnalysisSequence(0)));
    assert!(terminal_holes_len <= TERMINAL_ANALYSIS_IDENTITIES);

    let seen = counters.analysis_requests_seen.load(Ordering::Relaxed);
    let complete = counters.analysis_requests_complete.load(Ordering::Relaxed);
    let partial = counters.analysis_requests_partial.load(Ordering::Relaxed);
    let dropped = counters.analysis_requests_dropped.load(Ordering::Relaxed);
    assert_eq!(seen, total);
    assert_eq!(seen, complete + partial + dropped);
}

#[tokio::test]
async fn timeout_reconciliation_committed_statuses_preserve_terminal_partition() {
    let counters = ContextCounters::default();
    let ids = UuidV7Generator::new();

    for ordinal in 0..2 {
        let sequence = AnalysisSequence(ordinal);
        counters.ensure_seen_sequence(sequence);
        let evidence = PendingAnalysisEvidence {
            sequence,
            provider_request_id: RequestId::generate(&ids),
            snapshot_id: Some(ContextSnapshotId::generate(&ids)),
        };
        let mut lifecycle = counters
            .lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _previous = lifecycle.evidence.insert(sequence, evidence);
    }

    reconcile_pending_context_with(&counters, |evidence| async move {
        let status = if evidence.sequence == AnalysisSequence(0) {
            "complete"
        } else {
            "partial"
        };
        evidence.snapshot_id.map(|snapshot_id| {
            ContextSnapshotStatus::new(snapshot_id, evidence.provider_request_id, status.to_owned())
                .with_completed_at(Some(1))
        })
    })
    .await;

    assert_eq!(
        counters.analysis_requests_complete.load(Ordering::Relaxed),
        1
    );
    assert_eq!(
        counters.analysis_requests_partial.load(Ordering::Relaxed),
        1
    );
    assert_eq!(
        counters.analysis_requests_dropped.load(Ordering::Relaxed),
        0
    );
    assert_eq!(
        counters.analysis_requests_seen.load(Ordering::Relaxed),
        counters.analysis_requests_complete.load(Ordering::Relaxed)
            + counters.analysis_requests_partial.load(Ordering::Relaxed)
            + counters.analysis_requests_dropped.load(Ordering::Relaxed)
    );
    assert!(counters.pending_evidence().is_empty());
}

#[tokio::test]
async fn timeout_reconciliation_active_or_unavailable_statuses_drop_exact_partition() {
    let counters = ContextCounters::default();
    let ids = UuidV7Generator::new();

    for ordinal in 0..3 {
        let sequence = AnalysisSequence(ordinal);
        counters.ensure_seen_sequence(sequence);
        let evidence = PendingAnalysisEvidence {
            sequence,
            provider_request_id: RequestId::generate(&ids),
            snapshot_id: Some(ContextSnapshotId::generate(&ids)),
        };
        let mut lifecycle = counters
            .lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _previous = lifecycle.evidence.insert(sequence, evidence);
    }

    reconcile_pending_context_with(&counters, |evidence| async move {
        if evidence.sequence == AnalysisSequence(0) {
            evidence.snapshot_id.map(|snapshot_id| {
                ContextSnapshotStatus::new(
                    snapshot_id,
                    evidence.provider_request_id,
                    "active".to_owned(),
                )
            })
        } else {
            // `ContextStatus` not-found and transport-unreachable both produce no status.
            None
        }
    })
    .await;

    assert_eq!(
        counters.analysis_requests_complete.load(Ordering::Relaxed),
        0
    );
    assert_eq!(
        counters.analysis_requests_partial.load(Ordering::Relaxed),
        0
    );
    assert_eq!(
        counters.analysis_requests_dropped.load(Ordering::Relaxed),
        0
    );
    assert_eq!(counters.pending_evidence().len(), 3);

    counters.drop_pending(ContextAnalysisDropReason::Cancelled);

    assert_eq!(
        counters.analysis_requests_dropped.load(Ordering::Relaxed),
        3
    );
    assert_eq!(counters.pending_drop_cancelled.load(Ordering::Relaxed), 3);
    assert_eq!(
        counters
            .pre_persistence_drop_cancelled
            .load(Ordering::Relaxed),
        3
    );
    assert_eq!(
        counters.analysis_requests_seen.load(Ordering::Relaxed),
        counters.analysis_requests_complete.load(Ordering::Relaxed)
            + counters.analysis_requests_partial.load(Ordering::Relaxed)
            + counters.analysis_requests_dropped.load(Ordering::Relaxed)
    );
    assert!(counters.pending_evidence().is_empty());
}
#[test]
fn transport_admission_is_bounded_when_dispatcher_stalls() {
    use super::{TRANSPORT_DISPATCH_QUEUE_ITEMS, TransportOrdering, TransportSequence};

    let (sender, mut jobs) = tokio::sync::mpsc::channel(TRANSPORT_DISPATCH_QUEUE_ITEMS);
    let ordering = TransportOrdering::new(sender);
    for ordinal in 0..TRANSPORT_DISPATCH_QUEUE_ITEMS {
        assert!(ordering.try_enqueue_placeholder(TransportSequence(ordinal as u64)));
    }
    assert_eq!(ordering.pending_len(), TRANSPORT_DISPATCH_QUEUE_ITEMS);
    assert!(
        !ordering.try_enqueue_placeholder(TransportSequence(TRANSPORT_DISPATCH_QUEUE_ITEMS as u64))
    );
    assert_eq!(ordering.pending_len(), TRANSPORT_DISPATCH_QUEUE_ITEMS);

    let mut queued = 0;
    while jobs.try_recv().is_ok() {
        queued += 1;
    }
    assert_eq!(queued, TRANSPORT_DISPATCH_QUEUE_ITEMS);
}

#[test]
fn transport_dispatcher_preserves_fifo_admission_order() {
    use super::{TransportDispatchJob, TransportOrdering, TransportSequence};

    let (sender, mut jobs) = tokio::sync::mpsc::channel(2);
    let ordering = TransportOrdering::new(sender);
    assert!(ordering.try_enqueue_placeholder(TransportSequence(11)));
    assert!(ordering.try_enqueue_placeholder(TransportSequence(12)));
    drop(ordering);

    assert!(matches!(
        jobs.try_recv(),
        Ok(TransportDispatchJob::Placeholder {
            sequence: TransportSequence(11),
            ..
        })
    ));
    assert!(matches!(
        jobs.try_recv(),
        Ok(TransportDispatchJob::Placeholder {
            sequence: TransportSequence(12),
            ..
        })
    ));
    assert!(jobs.try_recv().is_err());
}
