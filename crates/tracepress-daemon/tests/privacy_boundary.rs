//! Provider observations persist only bounded metadata, never prompt or credential content.

use rusqlite::{Connection, types::Value};
use tempfile::TempDir;
use tracepress_core::{MaxIpcQueueItems, OperationKind};
use tracepress_daemon::{
    DaemonService, PersistProviderObservation, ProviderObservation, ProviderObservationOutcome,
};
use tracepress_provider::{
    ObservationInput, ObservationLimits, ObservationStatus, parse_request, parse_response,
};
use tracepress_storage::{Durability, StorageConfig, StorageWriter};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const PROMPT_CANARY: &str = "daemon-prompt-canary-4e1a";
const TOOL_CANARY: &str = "daemon-tool-canary-8b32";
const AUTH_CANARY: &str = "daemon-auth-canary-7c03";
const METADATA_CANARY: &str = "daemon-metadata-canary-2d94";
const OUTPUT_CANARY: &str = "daemon-output-canary-5f76";

const WIRE_CANARIES: [&str; 5] = [
    PROMPT_CANARY,
    TOOL_CANARY,
    AUTH_CANARY,
    METADATA_CANARY,
    OUTPUT_CANARY,
];

#[tokio::test]
async fn persistence_keeps_wire_canaries_out_of_debug_rows_and_events() -> TestResult {
    let directory = TempDir::new()?;
    let daemon = daemon(&directory).await?;
    let session = daemon.start_session("t0").await?;
    let operation = daemon
        .create_operation(session.session_id, OperationKind::LlmInference, "t1", None)
        .await?;

    let request_bytes = format!(
        r#"{{"model":"safe-model","input":[{{"role":"user","content":[{{"type":"input_text","text":"{PROMPT_CANARY}"}}]}},{{"type":"function_call","arguments":"{TOOL_CANARY}"}}],"metadata":{{"tag":"{METADATA_CANARY}"}},"user":"{AUTH_CANARY}","stream":true}}"#
    );
    let request = parse_request(ObservationInput::new(
        request_bytes.as_bytes(),
        ObservationLimits::default(),
    ));
    assert_eq!(request.status, ObservationStatus::Complete);
    let response_bytes = format!(
        r#"{{"output":[{{"type":"message","content":[{{"type":"output_text","text":"{OUTPUT_CANARY}"}}]}}]}}"#
    );
    let response = parse_response(ObservationInput::new(
        response_bytes.as_bytes(),
        ObservationLimits::default(),
    ));
    assert_eq!(
        response.response_state,
        tracepress_provider::ProviderResponseState::Unknown
    );
    let observation = ProviderObservation::new(321, request.clone(), "t2")
        .with_response(response.clone())
        .with_ended_at("t3")
        .with_outcome(ProviderObservationOutcome::Incomplete)
        .with_streaming(true);
    let debug = format!("{observation:?} {request:?} {response:?}");
    for canary in WIRE_CANARIES {
        assert!(!debug.contains(canary), "observation debug leaked {canary}");
    }
    let receipt = daemon
        .persist_provider_observation(PersistProviderObservation::new(
            session.session_id,
            operation,
            observation,
        ))
        .await?;
    daemon.shutdown().await?;

    let database = Connection::open(directory.path().join("tracepress.sqlite3"))?;
    assert_no_wire_canaries(&database)?;

    let attempt_status: String = database.query_row(
        "SELECT status FROM provider_attempts WHERE attempt_id = ?1",
        rusqlite::params![receipt.attempt_id.to_string()],
        |row| row.get(0),
    )?;
    assert_eq!(attempt_status, "incomplete");
    let usage: (Option<i64>, Option<i64>, String) = database.query_row(
        "SELECT input_total, total, usage_status FROM provider_usage WHERE attempt_id = ?1",
        rusqlite::params![receipt.attempt_id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(usage, (None, None, "unavailable".to_owned()));
    Ok(())
}

fn assert_no_wire_canaries(database: &Connection) -> TestResult {
    for table in [
        "provider_requests",
        "provider_attempts",
        "provider_usage",
        "events",
    ] {
        assert_no_canary_in_table(database, table, &WIRE_CANARIES)?;
    }
    Ok(())
}

async fn daemon(directory: &TempDir) -> Result<DaemonService, Box<dyn std::error::Error>> {
    let writer = StorageWriter::open(StorageConfig::new(
        directory.path().join("tracepress.sqlite3"),
        Durability::Strict,
        MaxIpcQueueItems::new(16)?,
    ))
    .await?;
    Ok(DaemonService::open(writer, "recovery").await?)
}

fn assert_no_canary_in_table(database: &Connection, table: &str, canaries: &[&str]) -> TestResult {
    let mut statement = database.prepare(&format!("SELECT * FROM {table}"))?;
    let column_count = statement.column_count();
    let rows = statement.query_map([], |row| {
        let mut values = Vec::with_capacity(column_count);
        for index in 0..column_count {
            let value: Value = row.get(index)?;
            values.push(match value {
                Value::Null => String::new(),
                Value::Integer(value) => value.to_string(),
                Value::Real(value) => value.to_string(),
                Value::Text(value) => value,
                Value::Blob(value) => String::from_utf8_lossy(&value).into_owned(),
            });
        }
        Ok(values.join("|"))
    })?;
    for row in rows {
        let text = row?;
        for canary in canaries {
            assert!(!text.contains(canary), "{table} leaked {canary}");
        }
    }
    Ok(())
}
