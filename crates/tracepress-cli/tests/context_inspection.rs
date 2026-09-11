#![allow(
    clippy::indexing_slicing,
    clippy::too_many_arguments,
    reason = "the subprocess fixture validates bounded HTTP framing before slicing"
)]

//! Real daemon/CLI subprocess coverage for metadata-only context inspection.

use std::{
    io::{Read as _, Write as _},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Output},
    thread,
};

use rusqlite::Connection;
use tempfile::TempDir;
use tracepress_core::{RequestId, UuidV7Generator};

type TestResult = Result<(), Box<dyn std::error::Error>>;
type SetupResult<T> = Result<T, Box<dyn std::error::Error>>;

const PROMPT_CANARY: &str = "context-prompt-canary-72e1";
const INSTRUCTIONS_CANARY: &str = "context-instructions-canary-5f03";
const OUTPUT_CANARY: &str = "context-output-canary-a8c4";
const METADATA_CANARY: &str = "context-metadata-canary-1b9d";
const TOOL_CANARY: &str = "context-tool-canary-6d10";

const AGENT: &str = "import os,urllib.request; request=urllib.request.Request(os.environ['TRACEPRESS_RESPONSES_URL'],data=os.environ['TRACEPRESS_E2E_REQUEST'].encode(),headers={'Content-Type':'application/json'}); response=urllib.request.urlopen(request); received=response.read(); assert received==os.environ['TRACEPRESS_E2E_STREAM'].encode(), received";

struct Fixture {
    cli: &'static str,
    home: PathBuf,
    daemon: PathBuf,
}

impl Fixture {
    fn new(home: &Path) -> SetupResult<Self> {
        let executable = std::env::current_exe()?;
        let target = executable
            .parent()
            .and_then(Path::parent)
            .ok_or("test executable has no target/debug parent")?;
        Ok(Self {
            cli: env!("CARGO_BIN_EXE_tracepress"),
            home: home.to_owned(),
            daemon: target.join("tracepressd"),
        })
    }

    fn command(&self) -> Command {
        let mut command = Command::new(self.cli);
        let _configured = command
            .env("TRACEPRESS_HOME", &self.home)
            .env("TRACEPRESSD_BIN", &self.daemon);
        command
    }

    fn run<const N: usize>(&self, args: [&str; N]) -> TestResult {
        let output = self.command().args(args).output()?;
        if !output.status.success() {
            return Err(format!(
                "tracepress {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }
        Ok(())
    }
}

fn request_body() -> String {
    format!(
        r#"{{"model":"gpt-test","stream":true,"instructions":"{INSTRUCTIONS_CANARY}","input":[{{"type":"input_text","text":"{PROMPT_CANARY}"}}],"metadata":{{"ticket":"{METADATA_CANARY}","tool":"{TOOL_CANARY}"}}}}"#
    )
}

fn stream_body(input_tokens: Option<u64>) -> String {
    let mut response = serde_json::json!({
        "response": {
            "id": "resp_context",
            "model": "gpt-test",
            "status": "completed",
            "output": [{
                "type": "message",
                "content": [{"type": "output_text", "text": OUTPUT_CANARY}]
            }]
        }
    });
    if let Some(tokens) = input_tokens {
        response["response"]["usage"] = serde_json::json!({
            "input_tokens": tokens,
            "output_tokens": 2,
            "total_tokens": tokens.saturating_add(2)
        });
    }
    format!("event: response.completed\ndata: {response}\n\n")
}

fn compaction_v2_request_body() -> String {
    r#"{"model":"gpt-test","stream":true,"input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"compact now"}]},{"type":"compaction_trigger"}]}"#.to_owned()
}

fn compaction_v2_stream_body() -> String {
    concat!(
        "event: response.output_item.done\n",
        "data: {\"item\":{\"type\":\"compaction\",\"encrypted_content\":\"private-compaction-output\"}}\n\n",
        "event: response.completed\n",
        "data: {\"response\":{\"id\":\"resp_compaction_v2\",\"model\":\"gpt-test\",\"status\":\"completed\",\"usage\":{\"input_tokens\":320,\"output_tokens\":12,\"total_tokens\":332}}}\n\n"
    )
    .to_owned()
}

fn serve_response(listener: &TcpListener, body: &str) -> std::io::Result<()> {
    let (mut stream, _) = listener.accept()?;
    read_http_request(&mut stream)?;
    write_http_response(&mut stream, body)
}

fn read_http_request(stream: &mut TcpStream) -> std::io::Result<()> {
    let mut received = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            return Err(std::io::Error::other("incomplete upstream request"));
        }
        received.extend_from_slice(&buffer[..read]);
        let Some(header_end) = received.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let headers =
            std::str::from_utf8(&received[..header_end]).map_err(std::io::Error::other)?;
        let body_length = headers
            .lines()
            .find_map(|line| {
                line.strip_prefix("Content-Length:")
                    .or_else(|| line.strip_prefix("content-length:"))
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })
            .unwrap_or_default();
        if received.len() >= header_end.saturating_add(4).saturating_add(body_length) {
            return Ok(());
        }
    }
}

fn write_http_response(stream: &mut TcpStream, body: &str) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
}

fn wait_for_snapshot(database_path: &Path) -> SetupResult<()> {
    let database = Connection::open(database_path)?;
    for _attempt in 0..200 {
        let snapshots: i64 =
            database.query_row("SELECT COUNT(*) FROM context_snapshots", [], |row| {
                row.get(0)
            })?;
        if snapshots > 0 {
            return Ok(());
        }
        thread::sleep(std::time::Duration::from_millis(5));
    }
    Err("context analysis did not persist before the recording shutdown".into())
}

fn run_recording(input_tokens: Option<u64>) -> SetupResult<(TempDir, String)> {
    let request = request_body();
    let stream = stream_body(input_tokens);
    run_recording_with(&request, &stream)
}

fn run_recording_with(request: &str, stream: &str) -> SetupResult<(TempDir, String)> {
    let directory = TempDir::new()?;
    let upstream = TcpListener::bind("127.0.0.1:0")?;
    let upstream_address = upstream.local_addr()?;
    let server_body = stream.to_owned();
    let server = thread::spawn(move || serve_response(&upstream, &server_body));
    let fixture = Fixture::new(directory.path())?;
    fixture.run(["init"])?;
    fixture.run(["daemon", "start"])?;
    let output = fixture
        .command()
        .env(
            "TRACEPRESS_UPSTREAM",
            format!("http://{upstream_address}/v1/responses"),
        )
        .env("TRACEPRESS_E2E_REQUEST", request)
        .env("TRACEPRESS_E2E_STREAM", stream)
        .args(["run", "python3", "--", "-c", AGENT])
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "tracepress run failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    server.join().map_err(|_| "upstream thread panicked")??;
    wait_for_snapshot(&directory.path().join("tracepress.sqlite3"))?;
    fixture.run(["daemon", "stop"])?;
    let database = Connection::open(directory.path().join("tracepress.sqlite3"))?;
    let request_id: String = database
        .query_row(
            "SELECT request_id FROM provider_requests ORDER BY rowid DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .map_err(|error| {
            let provider_requests: i64 = database
                .query_row("SELECT COUNT(*) FROM provider_requests", [], |row| {
                    row.get(0)
                })
                .unwrap_or(-1);
            let attempts: i64 = database
                .query_row("SELECT COUNT(*) FROM provider_attempts", [], |row| {
                    row.get(0)
                })
                .unwrap_or(-1);
            format!("{error}; provider_requests={provider_requests}; attempts={attempts}")
        })?;
    Ok((directory, request_id))
}

#[test]
fn compaction_v2_subprocess_persists_lifecycle_and_analysis_metadata() -> TestResult {
    let request = compaction_v2_request_body();
    let stream = compaction_v2_stream_body();
    let (directory, request_id) = run_recording_with(&request, &stream)?;
    let database = Connection::open(directory.path().join("tracepress.sqlite3"))?;
    let request_row: (String, Option<String>, i64, String, String, i64, Option<i64>) =
        database.query_row(
        "SELECT request_kind, compaction_trigger, request_bytes, transport, analysis_decode_status, decoded_bytes, parser_version FROM provider_requests WHERE request_id = ?1",
        [&request_id],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            ))
        },
    )?;
    assert_eq!(
        request_row,
        (
            "compaction_v2".to_owned(),
            Some("unknown".to_owned()),
            i64::try_from(request.len())?,
            "openai_public_api".to_owned(),
            "identity".to_owned(),
            i64::try_from(request.len())?,
            Some(1),
        )
    );
    let attempt: (String, i64) = database.query_row(
        "SELECT provider_response_id, compaction_output_seen FROM provider_attempts WHERE request_id = ?1",
        [&request_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(attempt, ("resp_compaction_v2".to_owned(), 1));
    let snapshot: (String, i64, i64) = database.query_row(
        "SELECT s.status, m.unknown_block_count, m.semantic_coverage_basis_points FROM context_snapshots AS s JOIN context_analysis_metrics AS m ON m.snapshot_id = s.snapshot_id WHERE s.provider_request_id = ?1",
        [&request_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(snapshot, ("complete".to_owned(), 0, 10_000));
    let event_payloads: Vec<Vec<u8>> = database
        .prepare("SELECT payload FROM events ORDER BY rowid")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let event_text = event_payloads
        .into_iter()
        .filter_map(|payload| String::from_utf8(payload).ok())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!event_text.contains("private-compaction-output"));
    Ok(())
}

fn inspect(input_tokens: Option<u64>) -> SetupResult<(Output, TempDir)> {
    let (directory, request_id) = run_recording(input_tokens)?;
    let fixture = Fixture::new(directory.path())?;
    fixture.run(["daemon", "start"])?;
    let output = fixture.command().args(["context", &request_id]).output()?;
    fixture.run(["daemon", "stop"])?;
    Ok((output, directory))
}

#[test]
fn context_subprocess_renders_signed_negative_residual_without_canaries() -> TestResult {
    let (output, _directory) = inspect(Some(1))?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout)?;
    for section in [
        "Context visibility",
        "Provider input observed",
        "Visible estimate + estimator + reconciliation/residual",
        "Composition (estimated)",
        "Repetition",
        "Stable explicit prefix",
        "Largest blocks",
        "Analysis",
        "Correlation",
    ] {
        assert!(stdout.contains(section), "missing section {section}");
    }
    let residual = stdout
        .lines()
        .find(|line| line.contains("residual (provider minus visible estimate"))
        .ok_or("missing residual")?;
    assert!(
        residual.contains('-'),
        "residual must be signed negative: {residual}\nstdout:\n{stdout}"
    );
    for canary in [
        PROMPT_CANARY,
        INSTRUCTIONS_CANARY,
        OUTPUT_CANARY,
        METADATA_CANARY,
        TOOL_CANARY,
    ] {
        assert!(!stdout.contains(canary), "leaked canary {canary}");
    }
    Ok(())
}

#[test]
fn context_subprocess_renders_positive_residual_and_unknown_missing_usage() -> TestResult {
    let (positive, _positive_directory) = inspect(Some(1_000))?;
    assert!(positive.status.success());
    let positive_stdout = String::from_utf8(positive.stdout)?;
    let positive_residual = positive_stdout
        .lines()
        .find(|line| line.contains("residual (provider minus visible estimate"))
        .ok_or("missing positive residual")?;
    assert!(
        positive_residual.contains('+'),
        "residual must be visibly signed: {positive_residual}\nstdout:\n{positive_stdout}"
    );
    let (missing, _missing_directory) = inspect(None)?;
    assert!(missing.status.success());
    let missing_stdout = String::from_utf8(missing.stdout)?;
    assert!(missing_stdout.contains("input tokens observed: unknown"));
    assert!(missing_stdout.contains("residual (provider minus visible estimate, signed): unknown"));
    assert!(missing_stdout.contains("estimated"));
    Ok(())
}

#[test]
fn context_subprocess_rejects_malformed_not_found_and_daemon_down() -> TestResult {
    let directory = TempDir::new()?;
    let fixture = Fixture::new(directory.path())?;
    fixture.run(["init"])?;

    let malformed = fixture
        .command()
        .args(["context", "not-a-request-id"])
        .output()?;
    assert!(!malformed.status.success());
    assert!(String::from_utf8_lossy(&malformed.stderr).contains("invalid value"));

    let missing_id = RequestId::generate(&UuidV7Generator::new()).to_string();
    let down = fixture.command().args(["context", &missing_id]).output()?;
    assert!(!down.status.success());
    assert!(String::from_utf8_lossy(&down.stderr).contains("daemon"));

    fixture.run(["daemon", "start"])?;
    let not_found = fixture.command().args(["context", &missing_id]).output()?;
    assert!(!not_found.status.success());
    assert!(
        String::from_utf8_lossy(&not_found.stderr).contains("was not found"),
        "not-found stderr: {}",
        String::from_utf8_lossy(&not_found.stderr)
    );
    fixture.run(["daemon", "stop"])?;
    Ok(())
}
