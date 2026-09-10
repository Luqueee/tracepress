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

fn run_recording(input_tokens: Option<u64>) -> SetupResult<(TempDir, String)> {
    let directory = TempDir::new()?;
    let upstream = TcpListener::bind("127.0.0.1:0")?;
    let upstream_address = upstream.local_addr()?;
    let request = request_body();
    let stream = stream_body(input_tokens);
    let server_body = stream.clone();
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
        .env("TRACEPRESS_E2E_REQUEST", &request)
        .env("TRACEPRESS_E2E_STREAM", &stream)
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
    fixture.run(["daemon", "stop"])?;
    let database = Connection::open(directory.path().join("tracepress.sqlite3"))?;
    let request_id: String = database.query_row(
        "SELECT request_id FROM provider_requests ORDER BY rowid DESC LIMIT 1",
        [],
        |row| row.get(0),
    )?;
    Ok((directory, request_id))
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
