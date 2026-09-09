#![allow(
    clippy::indexing_slicing,
    clippy::too_many_arguments,
    reason = "the bounded HTTP fixture validates offsets before slicing and keeps subprocess inputs explicit"
)]

//! Full CLI, daemon, proxy, upstream, and persisted DAG smoke test.

use std::{
    io::{Read as _, Write as _},
    net::TcpListener,
    path::Path,
    process::Command,
    thread,
};

use rusqlite::Connection;
use tempfile::TempDir;

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn cli_proxy_daemon_storage_round_trip_is_byte_exact() -> TestResult {
    let directory = TempDir::new()?;
    let upstream = TcpListener::bind("127.0.0.1:0")?;
    let upstream_address = upstream.local_addr()?;
    let server = thread::spawn(move || -> std::io::Result<()> {
        let (mut stream, _) = upstream.accept()?;
        let mut received = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let count = stream.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            received.extend_from_slice(&buffer[..count]);
            if let Some(header_end) = received.windows(4).position(|window| window == b"\r\n\r\n") {
                let header_end = header_end + 4;
                let headers =
                    std::str::from_utf8(&received[..header_end]).map_err(std::io::Error::other)?;
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(str::trim)
                            .and_then(|v| v.parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if received.len() >= header_end + length {
                    let body = &received[header_end..header_end + length];
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )?;
                    stream.write_all(body)?;
                    return Ok(());
                }
            }
        }
        Err(std::io::Error::other("incomplete upstream request"))
    });

    let cli = env!("CARGO_BIN_EXE_tracepress");
    let daemon = daemon_binary()?;
    run(cli, directory.path(), &daemon, ["init"])?;
    run(cli, directory.path(), &daemon, ["daemon", "start"])?;
    let script = "import os,urllib.request; b=bytes([0,255,1,65]); r=urllib.request.urlopen(urllib.request.Request(os.environ['TRACEPRESS_PROXY_URL'],data=b)); assert r.read()==b";
    let status = Command::new(cli)
        .env("TRACEPRESS_HOME", directory.path())
        .env(
            "TRACEPRESS_UPSTREAM",
            format!("http://{upstream_address}/v1/chat/completions"),
        )
        .args(["run", "python3", "--", "-c", script])
        .status()?;
    assert!(status.success());
    server.join().map_err(|_| "upstream thread panicked")??;
    run(cli, directory.path(), &daemon, ["daemon", "stop"])?;

    let database = Connection::open(directory.path().join("tracepress.sqlite3"))?;
    let state: String = database.query_row("SELECT state FROM sessions", [], |row| row.get(0))?;
    let operations: i64 =
        database.query_row("SELECT COUNT(*) FROM operations", [], |row| row.get(0))?;
    let edges: i64 =
        database.query_row("SELECT COUNT(*) FROM causal_edges", [], |row| row.get(0))?;
    let kinds: String = database.query_row(
        "SELECT group_concat(kind, ',') FROM (SELECT kind FROM operations ORDER BY started_at)",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(state, "closed");
    assert_eq!(operations, 2);
    assert_eq!(edges, 1);
    assert_eq!(kinds, "agent,llm_inference");
    Ok(())
}

fn daemon_binary() -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    let executable = std::env::current_exe()?;
    let debug = executable
        .parent()
        .and_then(Path::parent)
        .ok_or("test executable has no target/debug parent")?;
    Ok(debug.join("tracepressd"))
}

fn run<const N: usize>(cli: &str, home: &Path, daemon: &Path, args: [&str; N]) -> TestResult {
    let output = Command::new(cli)
        .env("TRACEPRESS_HOME", home)
        .env("TRACEPRESSD_BIN", daemon)
        .args(args)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "tracepress failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(())
}

#[derive(Debug)]
struct RejectedPersistence;

impl tracepress_proxy::MetadataSink for RejectedPersistence {
    fn try_record(
        &self,
        _metadata: tracepress_proxy::ForwardMetadata,
    ) -> Result<(), tracepress_proxy::MetadataSinkError> {
        Err(tracepress_proxy::MetadataSinkError::rejected())
    }
}

#[tokio::test]
async fn storage_failure_forwards_original() -> TestResult {
    use std::sync::Arc;

    use axum::{Router, body::Bytes, routing::post};
    use tokio::net::TcpListener as TokioTcpListener;
    use tracepress_core::{MaxRequestBodyBytes, MaxResponseBodyBytes};
    use tracepress_provider::ProviderEndpoint;
    use tracepress_proxy::{ProxyConfig, TransparentProxy};

    let upstream = TokioTcpListener::bind("127.0.0.1:0").await?;
    let upstream_address = upstream.local_addr()?;
    let upstream_task = tokio::spawn(async move {
        axum::serve(
            upstream,
            Router::new().route(
                "/v1/chat/completions",
                post(|| async { Bytes::from_static(b"\0\xfforiginal") }),
            ),
        )
        .await
    });
    let proxy = TransparentProxy::new(ProxyConfig::new(
        ProviderEndpoint::new(&format!("http://{upstream_address}/v1/chat/completions"))?,
        MaxRequestBodyBytes::new(1024)?,
        MaxResponseBodyBytes::new(1024)?,
    ))?
    .with_metadata_sink(Arc::new(RejectedPersistence));
    let listener = TokioTcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let proxy_task = tokio::spawn(async move { axum::serve(listener, proxy.router()).await });
    let response = reqwest::Client::new()
        .post(format!("http://{address}/v1/chat/completions"))
        .body(Bytes::from_static(b"\0request"))
        .send()
        .await?;
    assert_eq!(response.bytes().await?.as_ref(), b"\0\xfforiginal");
    proxy_task.abort();
    upstream_task.abort();
    Ok(())
}
