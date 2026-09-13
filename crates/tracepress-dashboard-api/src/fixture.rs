use rusqlite::{Connection, params};

use crate::FixtureDatabase;

const MIGRATIONS: [&str; 10] = [
    include_str!("../../tracepress-storage/migrations/0001_initial.sql"),
    include_str!("../../tracepress-storage/migrations/0002_provider_observability.sql"),
    include_str!("../../tracepress-storage/migrations/0003_context_analysis.sql"),
    include_str!("../../tracepress-storage/migrations/0004_codex_subscription_transport.sql"),
    include_str!("../../tracepress-storage/migrations/0005_compaction_request_kind.sql"),
    include_str!("../../tracepress-storage/migrations/0006_analysis_content_hash.sql"),
    include_str!("../../tracepress-storage/migrations/0007_compaction_v2_observability.sql"),
    include_str!("../../tracepress-storage/migrations/0008_context_semantic_coverage.sql"),
    include_str!("../../tracepress-storage/migrations/0009_shadow_compression.sql"),
    include_str!("../../tracepress-storage/migrations/0010_shadow_drop_reasons.sql"),
];

pub(crate) fn create(large: bool) -> Result<FixtureDatabase, rusqlite::Error> {
    let directory = tempfile::tempdir().map_err(|_error| {
        rusqlite::Error::InvalidPath(std::path::PathBuf::from("temporary fixture directory"))
    })?;
    let path = directory.path().join("dashboard.sqlite");
    let mut connection = Connection::open(&path)?;
    for (index, migration) in MIGRATIONS.iter().enumerate() {
        connection.execute_batch(migration)?;
        let _rows = connection.execute(
            "INSERT INTO schema_metadata(schema_version, applied_at) VALUES (?1, '2026-09-13T00:00:00Z')",
            params![i64::try_from(index).unwrap_or(0) + 1],
        )?;
    }
    let (session_count, requests_per_session, blocks_per_request) = if large {
        (1_000_u64, 10_u64, 10_u64)
    } else {
        (4, 4, 5)
    };
    let transaction = connection.transaction()?;
    for session_index in 0..session_count {
        let session_id = format!("00000000-0000-7000-8000-{session_index:012}");
        let degraded = session_index == 2;
        let running = session_index == 3 && !large;
        let _rows = transaction.execute(
            "INSERT INTO sessions VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                session_id,
                format!("2026-09-13T12:{:02}:00Z", session_index % 60),
                (!running).then_some("2026-09-13T12:59:00Z"),
                if running { "running" } else { "complete" },
                format!("fixture-{session_index}")
            ],
        )?;
        for request_index in 0..requests_per_session {
            let operation_id = format!("operation-{session_index:04}-{request_index:04}");
            let request_id = format!("request-{session_index:04}-{request_index:04}");
            let attempt_id = format!("attempt-{session_index:04}-{request_index:04}");
            let snapshot_id = format!("snapshot-{session_index:04}-{request_index:04}");
            let _rows = transaction.execute(
                "INSERT INTO operations VALUES (?1, ?2, 'inference', '2026-09-13T12:00:00Z', '2026-09-13T12:00:01Z', 'complete')",
                params![operation_id, session_id],
            )?;
            let _rows = transaction.execute(
                "INSERT INTO provider_requests(operation_id,request_id,route,method,request_bytes,provider,protocol,observation_status,model,reasoning_effort,transport,legacy_request_kind,request_kind) VALUES (?1,?2,'/responses','POST',1024,'openai','responses','complete',?3,'high','chatgpt_codex_subscription','turn',?4)",
                params![operation_id, request_id, if session_index % 2 == 0 { "gpt-5" } else { "gpt-5-mini" }, if request_index == 2 && session_index == 1 { "compaction_v2" } else { "turn" }],
            )?;
            let _rows = transaction.execute(
                "INSERT INTO provider_attempts(attempt_id,request_id,ordinal,status_code,started_at,ended_at,status,duration_us) VALUES (?1,?2,0,200,'2026-09-13T12:00:00Z','2026-09-13T12:00:01Z','complete',125000)",
                params![attempt_id, request_id],
            )?;
            let input = 1_000_i64 + i64::try_from(request_index).unwrap_or(0) * 350;
            let cached = input * 3 / 5;
            let _rows = transaction.execute(
                "INSERT INTO provider_usage(attempt_id,input_total,input_uncached,input_cached,output_total,output_reasoning,usage_status) VALUES (?1,?2,?3,?4,180,80,'final')",
                params![attempt_id, input, input - cached, cached],
            )?;
            let _rows = transaction.execute(
                "INSERT INTO context_snapshots(snapshot_id,session_id,provider_request_id,inference_operation_id,analysis_version,status,started_at_us,completed_at_us,explicit_block_count,analyzed_bytes,skipped_bytes,explicit_request_complete,logical_context_status,correlation_status) VALUES (?1,?2,?3,?4,1,?5,100,200,?6,4096,0,1,?7,?8)",
                params![snapshot_id, session_id, request_id, operation_id, if degraded && request_index == 1 { "partial" } else { "complete" }, i64::try_from(blocks_per_request).unwrap_or(0), if degraded && request_index == 1 { "malformed" } else { "complete" }, if degraded && request_index == 1 { "uncorrelated" } else { "correlated" }],
            )?;
            let _rows = transaction.execute(
                "INSERT INTO context_analysis_metrics(snapshot_id,estimated_tokens_kind_text,estimated_tokens_kind_tool_result,estimated_tokens_kind_unknown,semantic_coverage_basis_points) VALUES (?1,400,350,150,?2)",
                params![snapshot_id, if degraded { 7200 } else { 9600 }],
            )?;
            if request_index > 0 {
                let _rows = transaction.execute(
                    "INSERT INTO context_deltas(current_snapshot_id,previous_snapshot_id,repeated_blocks,new_blocks,repeated_estimated_tokens,new_estimated_tokens) VALUES (?1,?2,3,2,520,380)",
                    params![snapshot_id, format!("snapshot-{session_index:04}-{:04}", request_index - 1)],
                )?;
            }
            for block_index in 0..blocks_per_request {
                let detected = match block_index % 4 {
                    0 => "json",
                    1 | 2 => "plain_text",
                    _ => "unknown",
                };
                let origin = match block_index % 4 {
                    0 => "tool_generated",
                    1 => "human_authored",
                    2 => "agent_generated",
                    _ => "unknown",
                };
                let kind = if detected == "json" {
                    "tool_result"
                } else {
                    "text"
                };
                let token_value = (!degraded || block_index != blocks_per_request - 1)
                    .then_some(100_i64 + i64::try_from(block_index).unwrap_or(0));
                let exact = vec![u8::try_from(block_index % 8).unwrap_or(0); 32];
                let semantic = vec![u8::try_from(block_index % 5).unwrap_or(0); 32];
                let _rows = transaction.execute(
                    "INSERT INTO context_block_occurrences(block_occurrence_id,snapshot_id,ordinal,kind,role,origin,raw_value_start,raw_value_end,locator_occurrence,raw_bytes,exact_fingerprint,semantic_fingerprint,fingerprint_version,estimated_tokens,estimator,estimator_version,estimate_confidence,detected_kind,detector_confidence,detector_version) VALUES (?1,?2,?3,?4,'user',?5,0,512,0,512,?6,?7,1,?8,'fixture-estimator',1,'high',?9,0.98,1)",
                    params![format!("block-{session_index:04}-{request_index:04}-{block_index:04}"), snapshot_id, i64::try_from(block_index).unwrap_or(0), kind, origin, exact, semantic, token_value, detected],
                )?;
            }
        }
    }
    if !large {
        let _experiment = transaction.execute(
            "INSERT INTO compression_experiments(experiment_id,compressor_set_json,runtime_sha,limits_json,status,started_at,completed_at,forwarding_mutations,shadow_drops,recovery_failures,determinism_failures) VALUES ('shadow-pilot-001','[[\"json.noop\",1],[\"json.minify\",1],[\"json.tabular\",1],[\"json.repeated_subtree\",1]]','93ffe0c9c32a0f9a','{\"max_candidate_input_bytes\":1048576}','completed','2026-09-13T13:00:00Z','2026-09-13T13:05:00Z',0,1,0,0)",
            [],
        )?;
        for session_index in 0..session_count {
            for request_index in 0..requests_per_session {
                let snapshot_id = format!("snapshot-{session_index:04}-{request_index:04}");
                let block_id = format!("block-{session_index:04}-{request_index:04}-0000");
                for (compressor_index, compressor) in [
                    "json.noop",
                    "json.minify",
                    "json.tabular",
                    "json.repeated_subtree",
                ]
                .iter()
                .enumerate()
                {
                    let applicable = *compressor == "json.minify"
                        || (*compressor == "json.tabular" && request_index % 2 == 0)
                        || (*compressor == "json.repeated_subtree" && request_index == 3);
                    let status = if *compressor == "json.noop" {
                        "no_improvement"
                    } else if applicable {
                        "applicable"
                    } else {
                        "not_applicable"
                    };
                    let input_bytes = 400_i64;
                    let output_bytes = if applicable {
                        Some(220_i64 + i64::try_from(compressor_index).unwrap_or(0) * 12)
                    } else if *compressor == "json.noop" {
                        Some(input_bytes)
                    } else {
                        None
                    };
                    let delta = output_bytes.map(|output| input_bytes.saturating_sub(output));
                    let candidate_id = format!(
                        "00000000-0000-7001-8000-{session_index:03}{request_index:03}{compressor_index:03}000"
                    );
                    let original = vec![u8::try_from(request_index + 1).unwrap_or(1); 32];
                    let candidate = output_bytes
                        .map(|_| vec![u8::try_from(compressor_index + 11).unwrap_or(11); 32]);
                    let _candidate = transaction.execute(
                        "INSERT INTO compression_candidates(candidate_id,experiment_id,snapshot_id,block_occurrence_id,compressor_id,compressor_version,status,original_fingerprint,candidate_fingerprint,first_modified_offset,preserved_prefix_bytes,cache_risk) VALUES (?1,'shadow-pilot-001',?2,?3,?4,'1',?5,?6,?7,?8,?8,?9)",
                        params![candidate_id, snapshot_id, block_id, compressor, status, original, candidate, if applicable { Some(32_i64) } else { None }, if request_index == 0 { "high" } else { "medium" }],
                    )?;
                    let _metrics = transaction.execute(
                        "INSERT INTO compression_candidate_metrics(candidate_id,input_bytes,output_bytes,bytes_delta,input_estimated_tokens,output_estimated_tokens,estimated_token_delta,processing_us,reversible,recovery_verified,deterministic,preserved_prefix_ratio_basis_points) VALUES (?1,?2,?3,?4,100,?5,?6,?7,1,1,1,?8)",
                        params![candidate_id, input_bytes, output_bytes, delta, output_bytes.map(|value| value / 4), delta.map(|value| value / 4), 35_i64 + i64::try_from(compressor_index).unwrap_or(0) * 20, if applicable { Some(2500_i64) } else { None }],
                    )?;
                    let _recovery = transaction.execute(
                        "INSERT INTO compression_recoveries(candidate_id,verified,recovered_fingerprint,verified_at_us) VALUES (?1,1,?2,1757721600000000)",
                        params![candidate_id, original],
                    )?;
                }
            }
        }
    }
    transaction.commit()?;
    drop(connection);
    Ok(FixtureDatabase::new(directory, path))
}
