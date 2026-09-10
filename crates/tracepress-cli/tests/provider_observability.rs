#![allow(
    clippy::indexing_slicing,
    reason = "the bounded HTTP fixture validates offsets before slicing"
)]

//! Production `tracepress run` wiring: a streamed Responses completion recorded durably.

use std::{
    io::{Read as _, Write as _},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::Command,
    sync::LazyLock,
    thread,
    time::Duration,
};

use rusqlite::Connection;
use tempfile::TempDir;

type TestResult = Result<(), Box<dyn std::error::Error>>;
type SetupResult<T> = Result<T, Box<dyn std::error::Error>>;

const AUTH_CANARY: &str = "auth-canary-e2e-4d19";
const PROMPT_CANARY: &str = "prompt-canary-e2e-8a53";
const INSTRUCTIONS_CANARY: &str = "instructions-canary-e2e-2c77";
const METADATA_CANARY: &str = "metadata-canary-e2e-6b04";
const TOOL_ARGUMENT_CANARY: &str = "tool-argument-canary-e2e-1f38";
const OUTPUT_CANARY: &str = "output-canary-e2e-9e26";
const CANARIES: &[&str] = &[
    AUTH_CANARY,
    PROMPT_CANARY,
    INSTRUCTIONS_CANARY,
    METADATA_CANARY,
    TOOL_ARGUMENT_CANARY,
    OUTPUT_CANARY,
];

const USAGE_OBJECT: &str = r#"{"input_tokens":12,"input_tokens_details":{"cached_tokens":4},"output_tokens":8,"output_tokens_details":{"reasoning_tokens":5},"total_tokens":20}"#;

/// Drives the proxy from a real child process over HTTP and checks byte-exact forwarding.
const AGENT_SCRIPT: &str = "import os,urllib.request; body=os.environ['TRACEPRESS_E2E_REQUEST'].encode(); request=urllib.request.Request(os.environ['TRACEPRESS_RESPONSES_URL'],data=body,headers={'Content-Type':'application/json','Authorization':'Bearer '+os.environ['TRACEPRESS_E2E_AUTH']}); response=urllib.request.urlopen(request); received=response.read(); assert received==os.environ['TRACEPRESS_E2E_STREAM'].encode(), received; assert response.headers.get('content-type')=='text/event-stream', response.headers.items()";

/// Drives two overlapping Responses forwards from one child process.
const CONCURRENT_AGENT_SCRIPT: &str = r"
import os, threading, urllib.request

failures = []

def call(name):
    try:
        body = os.environ['TRACEPRESS_E2E_REQUEST_' + name].encode()
        request = urllib.request.Request(
            os.environ['TRACEPRESS_RESPONSES_URL'],
            data=body,
            headers={'Content-Type': 'application/json'},
        )
        response = urllib.request.urlopen(request)
        received = response.read()
        assert received == os.environ['TRACEPRESS_E2E_STREAM_' + name].encode(), (name, received)
    except BaseException as error:
        failures.append((name, error))

threads = [threading.Thread(target=call, args=(name,)) for name in ('A', 'B')]
for thread in threads:
    thread.start()
for thread in threads:
    thread.join()
assert not failures, failures
";

fn request_body() -> String {
    format!(
        r#"{{"model":"gpt-test","stream":true,"store":false,"instructions":"{INSTRUCTIONS_CANARY}","reasoning":{{"effort":"medium"}},"text":{{"verbosity":"low"}},"truncation":"auto","previous_response_id":"resp_previous_e2e","input":[{{"role":"user","content":[{{"type":"input_text","text":"{PROMPT_CANARY}"}}]}},{{"type":"function_call","name":"lookup","arguments":"{TOOL_ARGUMENT_CANARY}"}}],"tools":[{{"type":"function","name":"lookup"}}],"metadata":{{"ticket":"{METADATA_CANARY}"}}}}"#
    )
}

fn stream_events() -> Vec<String> {
    vec![
        r#"event: response.created
data: {"response":{"id":"resp_e2e_1","model":"gpt-test","status":"in_progress"}}

"#
        .to_owned(),
        format!(
            r#"event: response.output_text.delta
data: {{"delta":"{OUTPUT_CANARY}"}}

"#
        ),
        format!(
            r#"event: response.completed
data: {{"response":{{"id":"resp_e2e_1","model":"gpt-test","status":"completed","output":[{{"type":"message","content":[{{"type":"output_text","text":"{OUTPUT_CANARY}"}}]}}],"usage":{USAGE_OBJECT}}}}}

"#
        ),
    ]
}

/// A completed stream whose usage object cannot fit in one bounded control frame.
fn oversized_usage_events() -> Vec<String> {
    let filler = "f".repeat(16 * 1024);
    vec![format!(
        r#"event: response.completed
data: {{"response":{{"id":"resp_e2e_2","model":"gpt-test","status":"completed","usage":{{"input_tokens":1,"output_tokens":1,"total_tokens":2,"filler":"{filler}"}}}}}}

"#
    )]
}

/// Evidence collected from one real `tracepress run` against a streaming upstream.
struct Recorded {
    directory: TempDir,
    received: Vec<u8>,
    request: String,
    stream: String,
    /// Everything the run itself reported, including its correlation counters.
    stdout: String,
}

fn run_streamed_case(events: Vec<String>) -> SetupResult<Recorded> {
    let directory = TempDir::new()?;
    let upstream = TcpListener::bind("127.0.0.1:0")?;
    let upstream_address = upstream.local_addr()?;
    let stream = events.concat();
    let server = thread::spawn(move || serve_streamed_sse(&upstream, &events));

    let request = request_body();
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
        .env("TRACEPRESS_E2E_AUTH", AUTH_CANARY)
        .args(["run", "python3", "--", "-c", AGENT_SCRIPT])
        .output()?;
    assert!(
        output.status.success(),
        "tracepress run failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let received = server.join().map_err(|_| "upstream thread panicked")??;
    fixture.run(["daemon", "stop"])?;
    Ok(Recorded {
        directory,
        received,
        request,
        stream,
        stdout: String::from_utf8(output.stdout)?,
    })
}

/// Two events completing the first forward, whose upstream status is 200.
const CONCURRENT_STREAM_A: &str = r#"event: response.created
data: {"response":{"id":"resp_concurrent_a","model":"gpt-a","status":"in_progress"}}

event: response.completed
data: {"response":{"id":"resp_concurrent_a","model":"gpt-a","status":"completed","usage":{"input_tokens":7,"output_tokens":3,"total_tokens":10}}}

"#;

/// One event completing the second forward, whose upstream status is 202.
const CONCURRENT_STREAM_B: &str = r#"event: response.completed
data: {"response":{"id":"resp_concurrent_b","model":"gpt-b","status":"completed","usage":{"input_tokens":5,"output_tokens":2,"total_tokens":7}}}

"#;

/// One of the two Responses forwards the overlapping upstream fixture serves.
#[derive(Clone, Debug)]
struct ConcurrentForward {
    model: &'static str,
    status_line: &'static str,
    request: String,
    stream: &'static str,
}

/// Forwards that differ in request body, upstream status, provider identity, and stream length.
fn concurrent_forwards() -> [ConcurrentForward; 2] {
    [
        ConcurrentForward {
            model: "gpt-a",
            status_line: "HTTP/1.1 200 OK",
            request: concurrent_request_body("gpt-a"),
            stream: CONCURRENT_STREAM_A,
        },
        ConcurrentForward {
            model: "gpt-b",
            status_line: "HTTP/1.1 202 Accepted",
            request: concurrent_request_body("gpt-b"),
            stream: CONCURRENT_STREAM_B,
        },
    ]
}

fn concurrent_request_body(model: &str) -> String {
    format!(
        r#"{{"model":"{model}","stream":true,"input":[{{"role":"user","content":[{{"type":"input_text","text":"{PROMPT_CANARY}"}}]}}]}}"#
    )
}

/// Evidence collected from one real `tracepress run` with two overlapping forwards.
struct RecordedConcurrent {
    directory: TempDir,
    forwards: [ConcurrentForward; 2],
}

fn run_concurrent_case() -> SetupResult<RecordedConcurrent> {
    let directory = TempDir::new()?;
    let upstream = TcpListener::bind("127.0.0.1:0")?;
    let upstream_address = upstream.local_addr()?;
    let forwards = concurrent_forwards();
    let answered = forwards.clone();
    let server = thread::spawn(move || serve_overlapping_sse(&upstream, &answered));

    let fixture = Fixture::new(directory.path())?;
    fixture.run(["init"])?;
    fixture.run(["daemon", "start"])?;
    let output = fixture
        .command()
        .env(
            "TRACEPRESS_UPSTREAM",
            format!("http://{upstream_address}/v1/responses"),
        )
        .env("TRACEPRESS_E2E_REQUEST_A", &forwards[0].request)
        .env("TRACEPRESS_E2E_REQUEST_B", &forwards[1].request)
        .env("TRACEPRESS_E2E_STREAM_A", forwards[0].stream)
        .env("TRACEPRESS_E2E_STREAM_B", forwards[1].stream)
        .args(["run", "python3", "--", "-c", CONCURRENT_AGENT_SCRIPT])
        .output()?;
    assert!(
        output.status.success(),
        "tracepress run failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    server.join().map_err(|_| "upstream thread panicked")??;
    fixture.run(["daemon", "stop"])?;
    Ok(RecordedConcurrent {
        directory,
        forwards,
    })
}

/// Answers both forwards' headers before either body, then completes them in reverse order.
fn serve_overlapping_sse(
    listener: &TcpListener,
    forwards: &[ConcurrentForward; 2],
) -> std::io::Result<()> {
    let mut accepted = Vec::new();
    for _connection in 0..forwards.len() {
        let (mut stream, _peer) = listener.accept()?;
        let received = read_request(&mut stream)?;
        let body = String::from_utf8_lossy(&received).into_owned();
        let forward = forwards
            .iter()
            .find(|forward| body.contains(forward.model))
            .ok_or_else(|| std::io::Error::other("upstream saw an unidentified forward"))?;
        write!(
            stream,
            "{}\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n",
            forward.status_line
        )?;
        stream.flush()?;
        accepted.push((stream, forward));
    }
    // Both forwards are now in flight with their upstream status observed. Completing them in
    // reverse order makes any arrival-order pairing attribute a response to the wrong request.
    accepted.sort_by(|(_left, left), (_right, right)| right.model.cmp(left.model));
    for (mut stream, forward) in accepted {
        write!(stream, "{:x}\r\n", forward.stream.len())?;
        stream.write_all(forward.stream.as_bytes())?;
        stream.write_all(b"\r\n0\r\n\r\n")?;
        stream.flush()?;
    }
    Ok(())
}

/// More forwards than the recorder keeps correlation state for, so eviction is exercised.
const SEQUENTIAL_FORWARDS: usize = 70;

/// Drives one forward after another so each is settled before the next is accepted.
const SEQUENTIAL_AGENT_SCRIPT: &str = r"
import os, urllib.request

url = os.environ['TRACEPRESS_RESPONSES_URL']
body = os.environ['TRACEPRESS_E2E_REQUEST'].encode()
expected = os.environ['TRACEPRESS_E2E_STREAM'].encode()
for index in range(int(os.environ['TRACEPRESS_E2E_COUNT'])):
    request = urllib.request.Request(url, data=body, headers={'Content-Type': 'application/json'})
    received = urllib.request.urlopen(request).read()
    assert received == expected, (index, received)
";

fn run_sequential_case() -> SetupResult<TempDir> {
    let directory = TempDir::new()?;
    let upstream = TcpListener::bind("127.0.0.1:0")?;
    let upstream_address = upstream.local_addr()?;
    let server = thread::spawn(move || serve_sequential_sse(&upstream, SEQUENTIAL_FORWARDS));

    let fixture = Fixture::new(directory.path())?;
    fixture.run(["init"])?;
    fixture.run(["daemon", "start"])?;
    let output = fixture
        .command()
        .env(
            "TRACEPRESS_UPSTREAM",
            format!("http://{upstream_address}/v1/responses"),
        )
        .env("TRACEPRESS_E2E_REQUEST", concurrent_request_body("gpt-a"))
        .env("TRACEPRESS_E2E_STREAM", CONCURRENT_STREAM_B)
        .env("TRACEPRESS_E2E_COUNT", SEQUENTIAL_FORWARDS.to_string())
        .args(["run", "python3", "--", "-c", SEQUENTIAL_AGENT_SCRIPT])
        .output()?;
    assert!(
        output.status.success(),
        "tracepress run failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    server.join().map_err(|_| "upstream thread panicked")??;
    fixture.run(["daemon", "stop"])?;
    Ok(directory)
}

/// Completes one forward per connection, closing each so the next one is a new forward.
fn serve_sequential_sse(listener: &TcpListener, forwards: usize) -> std::io::Result<()> {
    for _forward in 0..forwards {
        let (mut stream, _peer) = listener.accept()?;
        let _received = read_request(&mut stream)?;
        stream.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nTransfer-Encoding: chunked\r\n\r\n",
        )?;
        write!(stream, "{:x}\r\n", CONCURRENT_STREAM_B.len())?;
        stream.write_all(CONCURRENT_STREAM_B.as_bytes())?;
        stream.write_all(b"\r\n0\r\n\r\n")?;
        stream.flush()?;
    }
    Ok(())
}

const CONTEXT_SATURATION_WAVES: usize = 20;
const CONTEXT_SATURATION_WIDTH: usize = 8;

const CONTEXT_SATURATION_AGENT_SCRIPT: &str = r"
import os, threading, time, urllib.request

url = os.environ['TRACEPRESS_RESPONSES_URL']
body = os.environ['TRACEPRESS_E2E_REQUEST'].encode()
expected = os.environ['TRACEPRESS_E2E_STREAM'].encode()
width = int(os.environ['TRACEPRESS_E2E_WIDTH'])

def call(index):
    request = urllib.request.Request(url, data=body, headers={'Content-Type': 'application/json'})
    received = urllib.request.urlopen(request).read()
    assert received == expected, (index, received)

for wave in range(int(os.environ['TRACEPRESS_E2E_WAVES'])):
    threads = [threading.Thread(target=call, args=(wave * width + index,)) for index in range(width)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    time.sleep(0.05)
";

fn context_saturation_request_body() -> String {
    let items = (0..1024)
        .map(|index| {
            format!(
                r#"{{"role":"user","content":[{{"type":"input_text","text":"context-saturation-{index}"}}]}}"#
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(r#"{{"model":"gpt-saturation","stream":true,"input":[{items}]}}"#)
}

fn run_context_saturation_case() -> SetupResult<(TempDir, String)> {
    let directory = TempDir::new()?;
    let upstream = TcpListener::bind("127.0.0.1:0")?;
    let upstream_address = upstream.local_addr()?;
    let forwards = CONTEXT_SATURATION_WAVES * CONTEXT_SATURATION_WIDTH;
    let server = thread::spawn(move || serve_sequential_sse(&upstream, forwards));

    let fixture = Fixture::new(directory.path())?;
    fixture.run(["init"])?;
    fixture.run(["daemon", "start"])?;
    let request = context_saturation_request_body();
    let output = fixture
        .command()
        .env(
            "TRACEPRESS_UPSTREAM",
            format!("http://{upstream_address}/v1/responses"),
        )
        .env("TRACEPRESS_E2E_REQUEST", &request)
        .env("TRACEPRESS_E2E_STREAM", CONCURRENT_STREAM_B)
        .env("TRACEPRESS_E2E_WAVES", CONTEXT_SATURATION_WAVES.to_string())
        .env("TRACEPRESS_E2E_WIDTH", CONTEXT_SATURATION_WIDTH.to_string())
        .args([
            "run",
            "python3",
            "--",
            "-c",
            CONTEXT_SATURATION_AGENT_SCRIPT,
        ])
        .output()?;
    assert!(
        output.status.success(),
        "tracepress run failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    server.join().map_err(|_| "upstream thread panicked")??;
    fixture.run(["daemon", "stop"])?;
    Ok((directory, String::from_utf8(output.stdout)?))
}

/// Forwards admitted after the interleaved one, enough to evict its correlation state.
///
/// [`IN_FLIGHT_FORWARDS`](../src/main.rs) is 64, so one identity plus this many newer ones is
/// past the bound the recorder keeps.
const INTERLEAVED_FORWARDS: usize = 70;

/// Forwards flooding the bound while one identity's request half is still being parsed.
const BURST_FORWARDS: usize = 66;

/// Cheap forwards racing one older identity that has not reached the recorder at all.
///
/// One heavy forward plus this many cheap ones is 66 concurrent forwards: the cheap ones fill
/// [`IN_FLIGHT_FORWARDS`](../src/main.rs) and the last of them evicts correlation state while
/// the heavy forward, which owns the oldest identity, still holds none.
const HEAVY_FIRST_BURST: usize = 65;

/// Writes a request body whose bounded semantic parse costs tens of milliseconds.
///
/// The item budget of `tracepress run` is `100_000`, so this body is interpreted right up to it:
/// the forward's transport half reaches the recorder long before its own request half does. It
/// is handed over as a file because it is far larger than one environment variable holds.
fn write_parse_heavy_request(directory: &Path) -> SetupResult<PathBuf> {
    let nested = std::iter::repeat_n(r#"{"a":{"b":{"c":{"d":1}}}}"#, 25_000)
        .collect::<Vec<_>>()
        .join(",");
    let path = directory.join("parse-heavy-request.json");
    std::fs::write(
        &path,
        format!(r#"{{"model":"gpt-heavy","stream":true,"input":[{nested}]}}"#),
    )?;
    Ok(path)
}

/// One forward per iteration, each identifying itself with its own model name.
const INTERLEAVED_MODEL_MARK: &str = "MODEL_MARK";

fn interleaved_request_template() -> String {
    format!(
        r#"{{"model":"{INTERLEAVED_MODEL_MARK}","stream":true,"input":[{{"role":"user","content":[{{"type":"input_text","text":"interleaved"}}]}}]}}"#
    )
}

/// Holds one forward's upstream answer back, then drives newer forwards to completion.
///
/// The held forward's request half is interpreted immediately, so eviction records it before any
/// transport evidence exists; its status and response only arrive afterwards.
const LATE_TRANSPORT_AGENT_SCRIPT: &str = r"
import os, socket, time, urllib.parse, urllib.request

target = urllib.parse.urlparse(os.environ['TRACEPRESS_RESPONSES_URL'])
expected = os.environ['TRACEPRESS_E2E_STREAM'].encode()
template = os.environ['TRACEPRESS_E2E_REQUEST']


def frame(body):
    head = (
        'POST %s HTTP/1.1\r\nHost: %s:%d\r\nContent-Type: application/json\r\n'
        'Content-Length: %d\r\nConnection: close\r\n\r\n'
        % (target.path, target.hostname, target.port, len(body))
    )
    return head.encode() + body


def drain(connection):
    chunks = []
    while True:
        chunk = connection.recv(65536)
        if not chunk:
            break
        chunks.append(chunk)
    return b''.join(chunks)


held = socket.create_connection((target.hostname, target.port))
held.sendall(frame(os.environ['TRACEPRESS_E2E_HELD_REQUEST'].encode()))
# The held forward is accepted first, so it owns the oldest correlation identity.
time.sleep(0.05)
for index in range(int(os.environ['TRACEPRESS_E2E_COUNT'])):
    body = template.replace('MODEL_MARK', 'gpt-newer-%d' % index).encode()
    request = urllib.request.Request(
        os.environ['TRACEPRESS_RESPONSES_URL'],
        data=body,
        headers={'Content-Type': 'application/json'},
    )
    received = urllib.request.urlopen(request).read()
    assert received == expected, (index, received)
answer = drain(held)
held.close()
assert answer.split(b'\r\n\r\n', 1)[1] == expected, answer[:120]
";

/// Releases a burst of forwards while one identity's request half is still being parsed.
///
/// The heavy forward's answer is read before the burst is released. The proxy hands its
/// transport metadata to the recorder before it builds the response the client then reads, so
/// that read proves the heavy identity was admitted before any burst identity — no sleep can
/// establish that ordering.
const LATE_REQUEST_AGENT_SCRIPT: &str = r"
import os, socket, threading, urllib.parse

target = urllib.parse.urlparse(os.environ['TRACEPRESS_RESPONSES_URL'])
expected = os.environ['TRACEPRESS_E2E_STREAM'].encode()
template = os.environ['TRACEPRESS_E2E_REQUEST']
count = int(os.environ['TRACEPRESS_E2E_COUNT'])


def frame(body):
    head = (
        'POST %s HTTP/1.1\r\nHost: %s:%d\r\nContent-Type: application/json\r\n'
        'Content-Length: %d\r\nConnection: close\r\n\r\n'
        % (target.path, target.hostname, target.port, len(body))
    )
    return head.encode() + body


def drain(connection):
    chunks = []
    while True:
        chunk = connection.recv(65536)
        if not chunk:
            break
        chunks.append(chunk)
    return b''.join(chunks)


failures = []
release = threading.Barrier(count + 1)


def call(connection, index):
    try:
        body = template.replace('MODEL_MARK', 'gpt-newer-%d' % index).encode()
        release.wait()
        connection.sendall(frame(body))
        answer = drain(connection)
        assert answer.split(b'\r\n\r\n', 1)[1] == expected, (index, answer[:120])
    except BaseException as error:
        failures.append((index, error))
    finally:
        connection.close()


# Every burst connection is established before anything is sent, so releasing them costs no
# connect latency inside the window the heavy request half is still being parsed in.
connections = [
    socket.create_connection((target.hostname, target.port)) for _ in range(count)
]
threads = [
    threading.Thread(target=call, args=(connection, index))
    for index, connection in enumerate(connections)
]
for thread in threads:
    thread.start()

heavy = socket.create_connection((target.hostname, target.port))
with open(os.environ['TRACEPRESS_E2E_HEAVY_REQUEST'], 'rb') as source:
    heavy.sendall(frame(source.read()))
answer = drain(heavy)
heavy.close()
assert answer.split(b'\r\n\r\n', 1)[1] == expected, answer[:120]

# The heavy request half is still being interpreted, so the burst evicts its identity first.
release.wait()
for thread in threads:
    thread.join()
assert not failures, failures
";

/// Hands over the heavy forward's body only after a whole burst has been recorded.
///
/// The proxy allocates a forward's identity from the request head, before it reads the body, so
/// sending the head alone claims the oldest identity while nothing about that forward can reach
/// the recorder: its bounded parse only starts once the body follows, and its transport and
/// response halves only once the upstream answers it, which is last.
const HEAVY_FIRST_AGENT_SCRIPT: &str = r"
import os, socket, threading, time, urllib.parse

target = urllib.parse.urlparse(os.environ['TRACEPRESS_RESPONSES_URL'])
expected = os.environ['TRACEPRESS_E2E_STREAM'].encode()
template = os.environ['TRACEPRESS_E2E_REQUEST']
count = int(os.environ['TRACEPRESS_E2E_COUNT'])


def head(length):
    return (
        'POST %s HTTP/1.1\r\nHost: %s:%d\r\nContent-Type: application/json\r\n'
        'Content-Length: %d\r\nConnection: close\r\n\r\n'
        % (target.path, target.hostname, target.port, length)
    ).encode()


def drain(connection):
    chunks = []
    while True:
        chunk = connection.recv(65536)
        if not chunk:
            break
        chunks.append(chunk)
    return b''.join(chunks)


with open(os.environ['TRACEPRESS_E2E_HEAVY_REQUEST'], 'rb') as source:
    heavy_body = source.read()

heavy = socket.create_connection((target.hostname, target.port))
heavy.sendall(head(len(heavy_body)))
# The head alone claims the oldest identity, so the burst that follows is newer than a forward
# the recorder has never heard of.
time.sleep(0.05)

failures = []
release = threading.Barrier(count + 1)


def call(connection, index):
    try:
        body = template.replace('MODEL_MARK', 'gpt-newer-%d' % index).encode()
        release.wait()
        connection.sendall(head(len(body)) + body)
        answer = drain(connection)
        assert answer.split(b'\r\n\r\n', 1)[1] == expected, (index, answer[:120])
    except BaseException as error:
        failures.append((index, error))
    finally:
        connection.close()


# Every burst connection is established before anything is sent, so releasing them costs no
# connect latency.
connections = [
    socket.create_connection((target.hostname, target.port)) for _ in range(count)
]
threads = [
    threading.Thread(target=call, args=(connection, index))
    for index, connection in enumerate(connections)
]
for thread in threads:
    thread.start()
release.wait()
for thread in threads:
    thread.join()
assert not failures, failures

# Every burst forward is recorded and correlation state has already been evicted, so the heavy
# forward's own halves all arrive behind the eviction.
heavy.sendall(heavy_body)
answer = drain(heavy)
heavy.close()
assert answer.split(b'\r\n\r\n', 1)[1] == expected, answer[:120]
";

/// Drives one forward whose transport and response halves only arrive after its eviction.
fn run_late_transport_case() -> SetupResult<TempDir> {
    let directory = TempDir::new()?;
    let upstream = TcpListener::bind("127.0.0.1:0")?;
    let upstream_address = upstream.local_addr()?;
    let server = thread::spawn(move || serve_held_forward(&upstream, INTERLEAVED_FORWARDS));

    let fixture = Fixture::new(directory.path())?;
    fixture.run(["init"])?;
    fixture.run(["daemon", "start"])?;
    let output = fixture
        .command()
        .env(
            "TRACEPRESS_UPSTREAM",
            format!("http://{upstream_address}/v1/responses"),
        )
        .env("TRACEPRESS_E2E_REQUEST", interleaved_request_template())
        .env(
            "TRACEPRESS_E2E_HELD_REQUEST",
            interleaved_request_template().replace(INTERLEAVED_MODEL_MARK, "gpt-held"),
        )
        .env("TRACEPRESS_E2E_STREAM", CONCURRENT_STREAM_B)
        .env("TRACEPRESS_E2E_COUNT", INTERLEAVED_FORWARDS.to_string())
        .args(["run", "python3", "--", "-c", LATE_TRANSPORT_AGENT_SCRIPT])
        .output()?;
    assert!(
        output.status.success(),
        "tracepress run failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    server.join().map_err(|_| "upstream thread panicked")??;
    fixture.run(["daemon", "stop"])?;
    Ok(directory)
}

/// Drives one forward whose request half is only interpreted after its eviction.
fn run_late_request_case() -> SetupResult<RecordedDegradation> {
    let directory = TempDir::new()?;
    let payload = TempDir::new()?;
    let heavy = write_parse_heavy_request(payload.path())?;
    let upstream = TcpListener::bind("127.0.0.1:0")?;
    let upstream_address = upstream.local_addr()?;
    let server = thread::spawn(move || serve_answered_then_burst(&upstream, BURST_FORWARDS));

    let fixture = Fixture::new(directory.path())?;
    fixture.run(["init"])?;
    fixture.run(["daemon", "start"])?;
    let output = fixture
        .command()
        .env(
            "TRACEPRESS_UPSTREAM",
            format!("http://{upstream_address}/v1/responses"),
        )
        .env("TRACEPRESS_E2E_REQUEST", interleaved_request_template())
        .env("TRACEPRESS_E2E_HEAVY_REQUEST", &heavy)
        .env("TRACEPRESS_E2E_STREAM", CONCURRENT_STREAM_B)
        .env("TRACEPRESS_E2E_COUNT", BURST_FORWARDS.to_string())
        .args(["run", "python3", "--", "-c", LATE_REQUEST_AGENT_SCRIPT])
        .output()?;
    assert!(
        output.status.success(),
        "tracepress run failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    server.join().map_err(|_| "upstream thread panicked")??;
    fixture.run(["daemon", "stop"])?;
    Ok(RecordedDegradation {
        directory,
        report: correlation_report(&String::from_utf8(output.stdout)?)?,
    })
}

/// The one late-request run both of its tests read.
///
/// A burst racing one bounded parse is timing sensitive, so driving that race twice inside one
/// test binary would have both runs competing for the CPU the race is decided on. Both tests
/// observe the same run instead, and the second waits for the first to finish it.
static LATE_REQUEST_CASE: LazyLock<Result<RecordedDegradation, String>> =
    LazyLock::new(|| run_late_request_case().map_err(|error| error.to_string()));

/// Borrows that shared run, repeating its setup failure to every test that needs it.
fn late_request_case() -> Result<&'static RecordedDegradation, String> {
    LATE_REQUEST_CASE.as_ref().map_err(Clone::clone)
}

/// Drives one forward that owns the oldest identity and reaches the recorder last.
fn run_heavy_first_case() -> SetupResult<TempDir> {
    let directory = TempDir::new()?;
    let payload = TempDir::new()?;
    let heavy = write_parse_heavy_request(payload.path())?;
    let upstream = TcpListener::bind("127.0.0.1:0")?;
    let upstream_address = upstream.local_addr()?;
    let server = thread::spawn(move || serve_burst_then_heavy(&upstream, HEAVY_FIRST_BURST));

    let fixture = Fixture::new(directory.path())?;
    fixture.run(["init"])?;
    fixture.run(["daemon", "start"])?;
    let output = fixture
        .command()
        .env(
            "TRACEPRESS_UPSTREAM",
            format!("http://{upstream_address}/v1/responses"),
        )
        .env("TRACEPRESS_E2E_REQUEST", interleaved_request_template())
        .env("TRACEPRESS_E2E_HEAVY_REQUEST", &heavy)
        .env("TRACEPRESS_E2E_STREAM", CONCURRENT_STREAM_B)
        .env("TRACEPRESS_E2E_COUNT", HEAVY_FIRST_BURST.to_string())
        .args(["run", "python3", "--", "-c", HEAVY_FIRST_AGENT_SCRIPT])
        .output()?;
    assert!(
        output.status.success(),
        "tracepress run failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    server.join().map_err(|_| "upstream thread panicked")??;
    fixture.run(["daemon", "stop"])?;
    Ok(directory)
}

/// Reads the first forward without answering it until every newer forward has completed.
fn serve_held_forward(listener: &TcpListener, newer: usize) -> std::io::Result<()> {
    let (mut held, _peer) = listener.accept()?;
    let _received = read_request(&mut held)?;
    for _forward in 0..newer {
        let (mut stream, _peer) = listener.accept()?;
        let _received = read_request(&mut stream)?;
        answer_stream(&mut stream)?;
    }
    // Only now does the held forward obtain an upstream status and a response.
    answer_stream(&mut held)
}

/// Answers the first forward at once, then holds every burst forward's answer back.
fn serve_answered_then_burst(listener: &TcpListener, burst: usize) -> std::io::Result<()> {
    let (mut heavy, _peer) = listener.accept()?;
    let _received = read_request(&mut heavy)?;
    answer_stream(&mut heavy)?;
    drop(heavy);
    let mut accepted = Vec::with_capacity(burst);
    for _forward in 0..burst {
        let (mut stream, _peer) = listener.accept()?;
        let _received = read_request(&mut stream)?;
        accepted.push(stream);
    }
    for mut stream in accepted {
        answer_stream(&mut stream)?;
        drop(stream);
        // One settled forward costs a control round trip, so pacing the answers keeps the
        // bounded recorder queue from dropping the evidence this test counts.
        thread::sleep(Duration::from_millis(2));
    }
    Ok(())
}

/// Answers every burst forward, then the heavy forward whose body only arrives afterwards.
fn serve_burst_then_heavy(listener: &TcpListener, burst: usize) -> std::io::Result<()> {
    for _forward in 0..burst {
        let (mut stream, _peer) = listener.accept()?;
        let _received = read_request(&mut stream)?;
        answer_stream(&mut stream)?;
        drop(stream);
        // One settled forward costs a control round trip, so pacing the answers keeps the
        // bounded recorder queue from dropping the evidence this test counts.
        thread::sleep(Duration::from_millis(2));
    }
    // The heavy forward reaches the upstream only once the agent hands over its body.
    let (mut heavy, _peer) = listener.accept()?;
    let _received = read_request(&mut heavy)?;
    answer_stream(&mut heavy)
}

/// Answers one forward with the completed stream fixture and an exact length.
fn answer_stream(stream: &mut TcpStream) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{CONCURRENT_STREAM_B}",
        CONCURRENT_STREAM_B.len()
    )?;
    stream.flush()
}

/// A response body the Responses observer cannot interpret, so no response half is emitted.
const PLAIN_RESPONSE_BODY: &str = "opaque-provider-bytes";

/// Sends one non-streaming forward and checks the forwarded bytes.
const PLAIN_AGENT_SCRIPT: &str = "import os,urllib.request; request=urllib.request.Request(os.environ['TRACEPRESS_RESPONSES_URL'],data=os.environ['TRACEPRESS_E2E_REQUEST'].encode(),headers={'Content-Type':'application/json'}); received=urllib.request.urlopen(request).read(); assert received==os.environ['TRACEPRESS_E2E_STREAM'].encode(), received";

fn run_unobserved_response_case() -> SetupResult<TempDir> {
    let directory = TempDir::new()?;
    let upstream = TcpListener::bind("127.0.0.1:0")?;
    let upstream_address = upstream.local_addr()?;
    let server = thread::spawn(move || serve_plain_response(&upstream));

    let fixture = Fixture::new(directory.path())?;
    fixture.run(["init"])?;
    fixture.run(["daemon", "start"])?;
    let output = fixture
        .command()
        .env(
            "TRACEPRESS_UPSTREAM",
            format!("http://{upstream_address}/v1/responses"),
        )
        .env("TRACEPRESS_E2E_REQUEST", unobserved_request_body())
        .env("TRACEPRESS_E2E_STREAM", PLAIN_RESPONSE_BODY)
        .args(["run", "python3", "--", "-c", PLAIN_AGENT_SCRIPT])
        .output()?;
    assert!(
        output.status.success(),
        "tracepress run failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    server.join().map_err(|_| "upstream thread panicked")??;
    fixture.run(["daemon", "stop"])?;
    Ok(directory)
}

fn unobserved_request_body() -> String {
    format!(
        r#"{{"model":"gpt-plain","stream":false,"input":[{{"role":"user","content":[{{"type":"input_text","text":"{PROMPT_CANARY}"}}]}}]}}"#
    )
}

/// Answers with a content type the observer does not interpret.
fn serve_plain_response(listener: &TcpListener) -> std::io::Result<()> {
    let (mut stream, _peer) = listener.accept()?;
    let _received = read_request(&mut stream)?;
    answer_plain(&mut stream)
}

/// Answers one forward with a media type the observer does not interpret.
///
/// The forward then leaves a request half and a transport half behind and never a response
/// half, so its correlation state stays unsettled until something else settles it.
fn answer_plain(stream: &mut TcpStream) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{PLAIN_RESPONSE_BODY}",
        PLAIN_RESPONSE_BODY.len()
    )?;
    stream.flush()
}

/// Settled forwards driven after a degraded one, enough to evict its correlation state.
///
/// [`IN_FLIGHT_FORWARDS`](../src/main.rs) is 64, so one identity plus this many newer ones
/// crosses the bound, and every newer forward is settled long before it is itself evicted.
const DEGRADING_FORWARDS: usize = 70;

/// Drives one forward whose response no observer reads, then settled forwards behind it.
///
/// The first forward leaves a request half and a transport half and never a response half, so
/// its correlation state is still unsettled when the newer forwards cross the in-flight bound.
/// Every half it will ever have has already arrived, so its eviction is its only degradation.
const UNSETTLED_FIRST_AGENT_SCRIPT: &str = r"
import os, urllib.request

url = os.environ['TRACEPRESS_RESPONSES_URL']
unreadable = urllib.request.Request(
    url,
    data=os.environ['TRACEPRESS_E2E_PLAIN_REQUEST'].encode(),
    headers={'Content-Type': 'application/json'},
)
received = urllib.request.urlopen(unreadable).read()
assert received == os.environ['TRACEPRESS_E2E_PLAIN_RESPONSE'].encode(), received

body = os.environ['TRACEPRESS_E2E_REQUEST'].encode()
expected = os.environ['TRACEPRESS_E2E_STREAM'].encode()
for index in range(int(os.environ['TRACEPRESS_E2E_COUNT'])):
    request = urllib.request.Request(url, data=body, headers={'Content-Type': 'application/json'})
    received = urllib.request.urlopen(request).read()
    assert received == expected, (index, received)
";

/// Holds one forward, whose answer no observer reads, until every newer forward is recorded.
///
/// The held forward is admitted from its request half and evicted while it is the oldest
/// identity. Its status then arrives for an identity the recorder has already retired, which
/// is the one degradation the bounded retirement memory produces; its unreadable body means no
/// response half follows that status.
const HELD_UNREADABLE_AGENT_SCRIPT: &str = r"
import os, socket, time, urllib.parse, urllib.request

target = urllib.parse.urlparse(os.environ['TRACEPRESS_RESPONSES_URL'])


def frame(body):
    head = (
        'POST %s HTTP/1.1\r\nHost: %s:%d\r\nContent-Type: application/json\r\n'
        'Content-Length: %d\r\nConnection: close\r\n\r\n'
        % (target.path, target.hostname, target.port, len(body))
    )
    return head.encode() + body


def drain(connection):
    chunks = []
    while True:
        chunk = connection.recv(65536)
        if not chunk:
            break
        chunks.append(chunk)
    return b''.join(chunks)


held = socket.create_connection((target.hostname, target.port))
held.sendall(frame(os.environ['TRACEPRESS_E2E_PLAIN_REQUEST'].encode()))
# The held forward is accepted first, so it owns the oldest correlation identity.
time.sleep(0.05)

body = os.environ['TRACEPRESS_E2E_REQUEST'].encode()
expected = os.environ['TRACEPRESS_E2E_STREAM'].encode()
for index in range(int(os.environ['TRACEPRESS_E2E_COUNT'])):
    request = urllib.request.Request(
        os.environ['TRACEPRESS_RESPONSES_URL'],
        data=body,
        headers={'Content-Type': 'application/json'},
    )
    received = urllib.request.urlopen(request).read()
    assert received == expected, (index, received)

answer = drain(held)
held.close()
assert answer.split(b'\r\n\r\n', 1)[1] == os.environ['TRACEPRESS_E2E_PLAIN_RESPONSE'].encode(), (
    answer[:120]
)
";

/// Evidence collected from one real `tracepress run` whose correlation degraded.
struct RecordedDegradation {
    directory: TempDir,
    /// The correlation counters the run reported for itself.
    report: String,
}

/// Drives one forward evicted from correlation state while it was still unsettled.
fn run_inflight_degraded_case() -> SetupResult<RecordedDegradation> {
    let directory = TempDir::new()?;
    let upstream = TcpListener::bind("127.0.0.1:0")?;
    let upstream_address = upstream.local_addr()?;
    let server =
        thread::spawn(move || serve_unreadable_then_streams(&upstream, DEGRADING_FORWARDS));

    let fixture = Fixture::new(directory.path())?;
    fixture.run(["init"])?;
    fixture.run(["daemon", "start"])?;
    let output = fixture
        .command()
        .env(
            "TRACEPRESS_UPSTREAM",
            format!("http://{upstream_address}/v1/responses"),
        )
        .env("TRACEPRESS_E2E_PLAIN_REQUEST", unobserved_request_body())
        .env("TRACEPRESS_E2E_PLAIN_RESPONSE", PLAIN_RESPONSE_BODY)
        .env("TRACEPRESS_E2E_REQUEST", concurrent_request_body("gpt-a"))
        .env("TRACEPRESS_E2E_STREAM", CONCURRENT_STREAM_B)
        .env("TRACEPRESS_E2E_COUNT", DEGRADING_FORWARDS.to_string())
        .args(["run", "python3", "--", "-c", UNSETTLED_FIRST_AGENT_SCRIPT])
        .output()?;
    assert!(
        output.status.success(),
        "tracepress run failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    server.join().map_err(|_| "upstream thread panicked")??;
    fixture.run(["daemon", "stop"])?;
    Ok(RecordedDegradation {
        directory,
        report: correlation_report(&String::from_utf8(output.stdout)?)?,
    })
}

/// Drives one forward whose only remaining half arrives after its identity was retired.
fn run_retired_degraded_case() -> SetupResult<RecordedDegradation> {
    let directory = TempDir::new()?;
    let upstream = TcpListener::bind("127.0.0.1:0")?;
    let upstream_address = upstream.local_addr()?;
    let server =
        thread::spawn(move || serve_held_unreadable_forward(&upstream, DEGRADING_FORWARDS));

    let fixture = Fixture::new(directory.path())?;
    fixture.run(["init"])?;
    fixture.run(["daemon", "start"])?;
    let output = fixture
        .command()
        .env(
            "TRACEPRESS_UPSTREAM",
            format!("http://{upstream_address}/v1/responses"),
        )
        .env("TRACEPRESS_E2E_PLAIN_REQUEST", unobserved_request_body())
        .env("TRACEPRESS_E2E_PLAIN_RESPONSE", PLAIN_RESPONSE_BODY)
        .env("TRACEPRESS_E2E_REQUEST", concurrent_request_body("gpt-a"))
        .env("TRACEPRESS_E2E_STREAM", CONCURRENT_STREAM_B)
        .env("TRACEPRESS_E2E_COUNT", DEGRADING_FORWARDS.to_string())
        .args(["run", "python3", "--", "-c", HELD_UNREADABLE_AGENT_SCRIPT])
        .output()?;
    assert!(
        output.status.success(),
        "tracepress run failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    server.join().map_err(|_| "upstream thread panicked")??;
    fixture.run(["daemon", "stop"])?;
    Ok(RecordedDegradation {
        directory,
        report: correlation_report(&String::from_utf8(output.stdout)?)?,
    })
}

/// Answers the first forward with an unreadable body, then completes every newer forward.
fn serve_unreadable_then_streams(listener: &TcpListener, newer: usize) -> std::io::Result<()> {
    let (mut unreadable, _peer) = listener.accept()?;
    let _received = read_request(&mut unreadable)?;
    answer_plain(&mut unreadable)?;
    drop(unreadable);
    serve_paced_streams(listener, newer)
}

/// Holds the first forward's answer back until every newer forward has been recorded.
fn serve_held_unreadable_forward(listener: &TcpListener, newer: usize) -> std::io::Result<()> {
    let (mut held, _peer) = listener.accept()?;
    let _received = read_request(&mut held)?;
    serve_paced_streams(listener, newer)?;
    // Only now does the held forward obtain an upstream status, and nothing reads its body.
    answer_plain(&mut held)
}

/// Completes one forward per connection, pacing the answers the recorder must keep up with.
fn serve_paced_streams(listener: &TcpListener, forwards: usize) -> std::io::Result<()> {
    for _forward in 0..forwards {
        let (mut stream, _peer) = listener.accept()?;
        let _received = read_request(&mut stream)?;
        answer_stream(&mut stream)?;
        drop(stream);
        // One settled forward costs a control round trip, so pacing the answers keeps the
        // bounded recorder queue from dropping the evidence this test counts.
        thread::sleep(Duration::from_millis(2));
    }
    Ok(())
}

/// Extracts the one line on which a run reports its correlation counters.
fn correlation_report(stdout: &str) -> SetupResult<String> {
    stdout
        .lines()
        .find(|line| line.starts_with("correlation_degraded_total="))
        .map(str::to_owned)
        .ok_or_else(|| "the run reported no correlation counters".into())
}

/// The counters a run reports when nothing degraded its correlation.
const UNDEGRADED_REPORT: &str = "correlation_degraded_total=0 correlation_degraded_inflight_limit=0 correlation_degraded_retired_limit=0 correlation_missing_total=0";

/// The canonical event every correlation degradation commits.
const DEGRADED_EVENT: &str = "context.correlation.degraded";

/// A rejection body whose media type the observer does not interpret, so no response half exists.
const REJECTED_RESPONSE_BODY: &str = "upstream-rejected-opaque-bytes";

/// Accepts the forwarded upstream rejection and checks its bytes.
const REJECTED_AGENT_SCRIPT: &str = r"
import os, urllib.error, urllib.request

request = urllib.request.Request(
    os.environ['TRACEPRESS_RESPONSES_URL'],
    data=os.environ['TRACEPRESS_E2E_REQUEST'].encode(),
    headers={'Content-Type': 'application/json'},
)
try:
    urllib.request.urlopen(request)
except urllib.error.HTTPError as error:
    assert error.code == 429, error.code
    received = error.read()
    assert received == os.environ['TRACEPRESS_E2E_STREAM'].encode(), received
else:
    raise AssertionError('the proxy must forward the upstream rejection')
";

/// Drives one Responses forward whose only terminal evidence is the upstream status.
fn run_rejected_status_case() -> SetupResult<TempDir> {
    let directory = TempDir::new()?;
    let upstream = TcpListener::bind("127.0.0.1:0")?;
    let upstream_address = upstream.local_addr()?;
    let server = thread::spawn(move || serve_rejected_response(&upstream));

    let fixture = Fixture::new(directory.path())?;
    fixture.run(["init"])?;
    fixture.run(["daemon", "start"])?;
    let output = fixture
        .command()
        .env(
            "TRACEPRESS_UPSTREAM",
            format!("http://{upstream_address}/v1/responses"),
        )
        .env("TRACEPRESS_E2E_REQUEST", unobserved_request_body())
        .env("TRACEPRESS_E2E_STREAM", REJECTED_RESPONSE_BODY)
        .args(["run", "python3", "--", "-c", REJECTED_AGENT_SCRIPT])
        .output()?;
    assert!(
        output.status.success(),
        "tracepress run failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    server.join().map_err(|_| "upstream thread panicked")??;
    fixture.run(["daemon", "stop"])?;
    Ok(directory)
}

/// Rejects the forward with a status the observer can read and a body it cannot.
fn serve_rejected_response(listener: &TcpListener) -> std::io::Result<()> {
    let (mut stream, _peer) = listener.accept()?;
    let _received = read_request(&mut stream)?;
    write!(
        stream,
        "HTTP/1.1 429 Too Many Requests\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{REJECTED_RESPONSE_BODY}",
        REJECTED_RESPONSE_BODY.len()
    )?;
    stream.flush()
}

/// Accepts the proxy's 502 for a forward whose upstream could not be reached.
const UNREACHABLE_AGENT_SCRIPT: &str = r"
import os, urllib.error, urllib.request

request = urllib.request.Request(
    os.environ['TRACEPRESS_RESPONSES_URL'],
    data=os.environ['TRACEPRESS_E2E_REQUEST'].encode(),
    headers={'Content-Type': 'application/json'},
)
try:
    urllib.request.urlopen(request)
except urllib.error.HTTPError as error:
    assert error.code == 502, error.code
else:
    raise AssertionError('an unreachable upstream must not answer 200')
";

/// Drives one Responses forward at an address no listener owns, so transport fails.
fn run_unreachable_upstream_case() -> SetupResult<TempDir> {
    let directory = TempDir::new()?;
    let probe = TcpListener::bind("127.0.0.1:0")?;
    let upstream_address = probe.local_addr()?;
    // Releasing the port before the run makes every connection attempt fail to connect.
    drop(probe);

    let fixture = Fixture::new(directory.path())?;
    fixture.run(["init"])?;
    fixture.run(["daemon", "start"])?;
    let output = fixture
        .command()
        .env(
            "TRACEPRESS_UPSTREAM",
            format!("http://{upstream_address}/v1/responses"),
        )
        .env("TRACEPRESS_E2E_REQUEST", unobserved_request_body())
        .args(["run", "python3", "--", "-c", UNREACHABLE_AGENT_SCRIPT])
        .output()?;
    assert!(
        output.status.success(),
        "tracepress run failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    fixture.run(["daemon", "stop"])?;
    Ok(directory)
}

/// A completed non-streamed Responses document, served with an exact `Content-Length`.
fn json_completion_body() -> String {
    format!(
        r#"{{"id":"resp_json_e2e","model":"gpt-plain","status":"completed","output":[{{"type":"message","content":[{{"type":"output_text","text":"{OUTPUT_CANARY}"}}]}}],"usage":{USAGE_OBJECT}}}"#
    )
}

/// Drives one non-streamed Responses forward answered with a JSON document.
fn run_json_completion_case() -> SetupResult<TempDir> {
    let directory = TempDir::new()?;
    let upstream = TcpListener::bind("127.0.0.1:0")?;
    let upstream_address = upstream.local_addr()?;
    let body = json_completion_body();
    let answered = body.clone();
    let server = thread::spawn(move || serve_json_response(&upstream, &answered));

    let fixture = Fixture::new(directory.path())?;
    fixture.run(["init"])?;
    fixture.run(["daemon", "start"])?;
    let output = fixture
        .command()
        .env(
            "TRACEPRESS_UPSTREAM",
            format!("http://{upstream_address}/v1/responses"),
        )
        .env("TRACEPRESS_E2E_REQUEST", unobserved_request_body())
        .env("TRACEPRESS_E2E_STREAM", &body)
        .args(["run", "python3", "--", "-c", PLAIN_AGENT_SCRIPT])
        .output()?;
    assert!(
        output.status.success(),
        "tracepress run failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    server.join().map_err(|_| "upstream thread panicked")??;
    fixture.run(["daemon", "stop"])?;
    Ok(directory)
}

/// Answers one request with a JSON document body of advertised length.
fn serve_json_response(listener: &TcpListener, body: &str) -> std::io::Result<()> {
    let (mut stream, _peer) = listener.accept()?;
    let _received = read_request(&mut stream)?;
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()
}

#[test]
fn streamed_responses_run_records_provider_observability_durably() -> TestResult {
    let recorded = run_streamed_case(stream_events())?;
    assert_upstream_saw_exact_request(&recorded.received, &recorded.request)?;

    let root = recorded.directory.path();
    let database = Connection::open(root.join("tracepress.sqlite3"))?;
    assert_provider_request(&database, i64::try_from(recorded.request.len())?)?;
    assert_provider_attempt(&database, i64::try_from(recorded.stream.len())?)?;
    assert_provider_usage(&database)?;
    assert_measured_timings(&database)?;
    assert_canonical_events(&database)?;
    assert_context_events(&database)?;
    assert_context_storage(&database, 1, false)?;
    assert_causal_dag(&database)?;
    drop(database);

    assert_no_canaries_in_storage(root)
}

#[test]
fn observation_rejected_by_the_control_frame_bound_never_disturbs_the_agent() -> TestResult {
    let recorded = run_streamed_case(oversized_usage_events())?;
    // The child asserted byte-exact response bytes, so forwarding survived the rejected record.
    assert_upstream_saw_exact_request(&recorded.received, &recorded.request)?;

    let root = recorded.directory.path();
    let database = Connection::open(root.join("tracepress.sqlite3"))?;
    assert_eq!(
        integer(&database, "SELECT COUNT(*) FROM provider_requests")?,
        0,
        "an untransportable observation must be dropped, not partially recorded"
    );
    assert_eq!(
        integer(&database, "SELECT COUNT(*) FROM provider_attempts")?,
        0
    );
    assert_eq!(
        integer(&database, "SELECT COUNT(*) FROM provider_usage")?,
        0
    );
    // The undeliverable semantic record falls back to the transport record, so the forward
    // still leaves exactly one inference operation behind instead of none or two.
    assert_eq!(
        integer(
            &database,
            "SELECT COUNT(*) FROM operations WHERE kind = 'llm_inference'",
        )?,
        1
    );
    assert_eq!(text(&database, "SELECT state FROM sessions")?, "closed");
    Ok(())
}

#[test]
fn concurrent_responses_forwards_record_their_own_attempts() -> TestResult {
    let recorded = run_concurrent_case()?;
    let root = recorded.directory.path();
    let database = Connection::open(root.join("tracepress.sqlite3"))?;
    let [first, second] = &recorded.forwards;

    assert_eq!(
        integer(&database, "SELECT COUNT(*) FROM provider_requests")?,
        2
    );
    assert_eq!(
        integer(&database, "SELECT COUNT(*) FROM provider_attempts")?,
        2
    );
    // Two forwards are two inference operations, one owning each logical request.
    assert_eq!(
        integer(
            &database,
            "SELECT COUNT(*) FROM operations WHERE kind = 'llm_inference'",
        )?,
        2
    );
    assert_eq!(
        integer(
            &database,
            "SELECT COUNT(DISTINCT operation_id) FROM provider_requests",
        )?,
        2
    );
    assert_eq!(
        text(
            &database,
            "SELECT group_concat(evidence, '|') FROM (SELECT r.model || '=' || r.request_bytes || '=' || a.status_code || '=' || a.provider_response_id || '=' || a.byte_count || '=' || a.response_state AS evidence FROM provider_attempts a JOIN provider_requests r ON r.request_id = a.request_id ORDER BY r.model)",
        )?,
        format!(
            "gpt-a={}=200=resp_concurrent_a={}=completed|gpt-b={}=202=resp_concurrent_b={}=completed",
            first.request.len(),
            first.stream.len(),
            second.request.len(),
            second.stream.len()
        ),
        "each forward's transport status and response evidence must join its own request"
    );
    assert_context_storage(&database, 2, true)?;
    assert_eq!(text(&database, "SELECT state FROM sessions")?, "closed");
    drop(database);

    assert_no_canaries_in_storage(root)
}

#[test]
fn forwards_beyond_the_correlation_bound_keep_one_attempt_each() -> TestResult {
    let directory = run_sequential_case()?;
    let root = directory.path();
    let database = Connection::open(root.join("tracepress.sqlite3"))?;
    let forwards = i64::try_from(SEQUENTIAL_FORWARDS)?;

    // Evicting correlation state neither loses a forward nor records one twice.
    assert_eq!(
        integer(&database, "SELECT COUNT(*) FROM provider_requests")?,
        forwards
    );
    assert_eq!(
        integer(
            &database,
            "SELECT COUNT(*) FROM provider_attempts WHERE ordinal = 0 AND status_code = 200",
        )?,
        forwards
    );
    assert_eq!(
        integer(
            &database,
            "SELECT COUNT(*) FROM operations WHERE kind = 'llm_inference'",
        )?,
        forwards
    );
    assert_eq!(
        integer(
            &database,
            "SELECT COUNT(DISTINCT operation_id) FROM provider_requests",
        )?,
        forwards
    );
    assert_eq!(text(&database, "SELECT state FROM sessions")?, "closed");
    Ok(())
}

#[test]
fn context_queue_backpressure_drops_only_analysis_after_all_provider_receipts_commit() -> TestResult
{
    let (directory, stdout) = run_context_saturation_case()?;
    let root = directory.path();
    let database = Connection::open(root.join("tracepress.sqlite3"))?;
    let forwards = i64::try_from(CONTEXT_SATURATION_WAVES * CONTEXT_SATURATION_WIDTH)?;
    assert_eq!(
        integer(&database, "SELECT COUNT(*) FROM provider_requests")?,
        forwards,
        "every provider request must survive context queue saturation"
    );
    assert_eq!(
        integer(
            &database,
            "SELECT COUNT(*) FROM provider_attempts WHERE ordinal = 0 AND status = 'completed'",
        )?,
        forwards,
        "every provider attempt must survive context queue saturation"
    );
    let counter = stdout
        .lines()
        .find_map(|line| line.strip_prefix("context_observer_backpressure_total="))
        .ok_or("run did not report the context backpressure counter")?
        .parse::<u64>()?;
    let snapshots = integer(&database, "SELECT COUNT(*) FROM context_snapshots")?;
    assert!(
        counter > 0,
        "context queue must saturate deterministically; snapshots={snapshots} stdout={stdout}"
    );
    let payload: Vec<u8> = database.query_row(
        "SELECT payload FROM events WHERE event_type = 'context.analysis.dropped'",
        [],
        |row| row.get(0),
    )?;
    let payload = String::from_utf8(payload)?;
    assert!(payload.contains(r#""status":"observer_backpressure""#));
    assert!(payload.contains(r#""dropped_count":"#));
    assert_no_canaries_in_storage(root)?;
    Ok(())
}

#[test]
fn a_transport_half_arriving_after_eviction_keeps_one_inference() -> TestResult {
    let directory = run_late_transport_case()?;
    let root = directory.path();
    let database = Connection::open(root.join("tracepress.sqlite3"))?;
    let forwards = i64::try_from(INTERLEAVED_FORWARDS)? + 1;

    // The held forward is recorded from its request half when eviction reaches it, so its
    // status and response arrive for an identity the recorder has already accounted for.
    // Re-admitting it would double count that one inference.
    assert_eq!(
        integer(
            &database,
            "SELECT COUNT(*) FROM operations WHERE kind = 'llm_inference'",
        )?,
        forwards,
        "every forward is entitled to exactly one inference operation"
    );
    assert_eq!(
        integer(&database, "SELECT COUNT(*) FROM provider_requests")?,
        forwards
    );
    assert_eq!(
        integer(
            &database,
            "SELECT COUNT(DISTINCT model) FROM provider_requests",
        )?,
        forwards,
        "each forward names its own model, so a repeated model is a forward recorded twice"
    );
    assert_eq!(
        integer(
            &database,
            "SELECT COUNT(*) FROM provider_attempts WHERE status_code IS NULL",
        )?,
        1,
        "the held forward must be the one recorded before any transport evidence existed"
    );
    assert_eq!(text(&database, "SELECT state FROM sessions")?, "closed");
    Ok(())
}

#[test]
fn a_request_half_arriving_after_eviction_keeps_one_inference() -> TestResult {
    let recorded = late_request_case()?;
    let root = recorded.directory.path();
    let database = Connection::open(root.join("tracepress.sqlite3"))?;
    let burst = i64::try_from(BURST_FORWARDS)?;

    // The heavy forward is admitted by its transport half and evicted while its own request half
    // is still being parsed, so eviction records it as a transport-only inference. The request
    // and response that arrive afterwards must not open a second operation for it.
    assert_eq!(
        integer(
            &database,
            "SELECT COUNT(*) FROM operations WHERE kind = 'llm_inference'",
        )?,
        burst + 1,
        "every forward is entitled to exactly one inference operation"
    );
    assert_eq!(
        integer(&database, "SELECT COUNT(*) FROM provider_requests")?,
        burst,
        "the evicted forward left transport evidence only, so it owns no logical request"
    );
    assert_eq!(
        integer(
            &database,
            "SELECT COUNT(DISTINCT model) FROM provider_requests",
        )?,
        burst,
        "each forward names its own model, so a repeated model is a forward recorded twice"
    );
    assert_eq!(
        integer(
            &database,
            "SELECT COUNT(*) FROM provider_requests WHERE model = 'gpt-heavy'",
        )?,
        0
    );
    assert_eq!(text(&database, "SELECT state FROM sessions")?, "closed");
    Ok(())
}

#[test]
fn a_forward_evicted_without_its_request_half_reports_the_missing_correlation() -> TestResult {
    let recorded = late_request_case()?;
    let database = Connection::open(recorded.directory.path().join("tracepress.sqlite3"))?;
    let counts = correlation_counts(&recorded.report)?;

    // The heavy forward is evicted holding response and transport evidence but no request half,
    // and that forward owns no logical request afterwards, so the absent half is a loss of its
    // own. The burst races the bound, so how many forwards it degrades is not fixed; what is
    // fixed is that the missing half is counted and that every reason adds up to the total.
    assert!(
        counts.missing >= 1,
        "a forward recorded without its request half must be counted as missing: {}",
        recorded.report
    );
    assert_eq!(
        counts.total,
        counts.inflight_limit + counts.retired_limit + counts.missing,
        "the total must account for exactly the reasons it is composed of: {}",
        recorded.report
    );
    // Counters without durable events would report a loss nothing can be queried about.
    assert_eq!(
        integer(
            &database,
            &format!("SELECT COUNT(*) FROM events WHERE event_type = '{DEGRADED_EVENT}'"),
        )?,
        counts.total,
        "every counted degradation must have committed exactly one event"
    );
    assert!(
        blob(
            &database,
            &format!(
                "SELECT payload FROM events WHERE event_type = '{DEGRADED_EVENT}' AND CAST(payload AS TEXT) LIKE '%missing_request_half%'",
            ),
        )
        .is_ok(),
        "the missing request half must name its own reason"
    );
    Ok(())
}

/// The four correlation counters one run reported.
struct CorrelationCounts {
    total: i64,
    inflight_limit: i64,
    retired_limit: i64,
    missing: i64,
}

/// Reads the counters out of the line a run reports them on.
fn correlation_counts(report: &str) -> SetupResult<CorrelationCounts> {
    let counter = |name: &str| -> SetupResult<i64> {
        let field = format!("{name}=");
        report
            .split_whitespace()
            .find_map(|reported| reported.strip_prefix(field.as_str()))
            .ok_or_else(|| -> Box<dyn std::error::Error> {
                format!("the report has no {name}: {report}").into()
            })?
            .parse()
            .map_err(Into::into)
    };
    Ok(CorrelationCounts {
        total: counter("correlation_degraded_total")?,
        inflight_limit: counter("correlation_degraded_inflight_limit")?,
        retired_limit: counter("correlation_degraded_retired_limit")?,
        missing: counter("correlation_missing_total")?,
    })
}

#[test]
fn a_forward_whose_first_half_arrives_after_eviction_is_still_recorded() -> TestResult {
    let directory = run_heavy_first_case()?;
    let root = directory.path();
    let database = Connection::open(root.join("tracepress.sqlite3"))?;
    let forwards = i64::try_from(HEAVY_FIRST_BURST)? + 1;

    // The heavy forward owns the oldest identity but held no correlation state when the burst
    // evicted the oldest identity that did, so retiring every identity below that eviction
    // would leave this forward with no record at all.
    assert_eq!(
        integer(
            &database,
            "SELECT COUNT(*) FROM operations WHERE kind = 'llm_inference'",
        )?,
        forwards,
        "every forward is entitled to exactly one inference operation"
    );
    assert_eq!(
        integer(&database, "SELECT COUNT(*) FROM provider_requests")?,
        forwards,
        "every forward's logical request must survive the eviction of a newer identity"
    );
    assert_eq!(
        integer(&database, "SELECT COUNT(*) FROM provider_attempts")?,
        forwards
    );
    assert_eq!(
        integer(
            &database,
            "SELECT COUNT(*) FROM provider_requests WHERE request_bytes > 100000",
        )?,
        1,
        "the heavy forward owns exactly one logical request, neither lost nor duplicated"
    );
    assert_eq!(
        integer(
            &database,
            "SELECT COUNT(DISTINCT model) FROM provider_requests WHERE request_bytes <= 100000",
        )?,
        forwards - 1,
        "each burst forward names its own model, so a repeated model is a forward recorded twice"
    );
    assert_eq!(text(&database, "SELECT state FROM sessions")?, "closed");
    Ok(())
}

#[test]
fn a_forward_evicted_by_the_in_flight_bound_reports_one_degradation() -> TestResult {
    let recorded = run_inflight_degraded_case()?;
    let root = recorded.directory.path();
    let database = Connection::open(root.join("tracepress.sqlite3"))?;

    assert_eq!(
        recorded.report,
        "correlation_degraded_total=1 correlation_degraded_inflight_limit=1 correlation_degraded_retired_limit=0 correlation_missing_total=0",
        "the in-flight bound degraded exactly one forward, under its own reason"
    );
    assert_degraded_events(&database, &["in_flight_limit"])?;
    // The degradation names the forward's own inference, and that forward keeps every row its
    // evidence supports: a degraded correlation is missing causality, not missing evidence.
    assert_eq!(
        integer(
            &database,
            &format!(
                "SELECT COUNT(*) FROM events e JOIN provider_requests r ON r.operation_id = e.operation_id WHERE e.event_type = '{DEGRADED_EVENT}' AND r.model = 'gpt-plain'",
            ),
        )?,
        1,
        "the degraded forward must keep the logical request it had evidence for"
    );
    assert_eq!(
        integer(&database, "SELECT COUNT(*) FROM provider_requests")?,
        i64::try_from(DEGRADING_FORWARDS)? + 1,
        "degrading one forward may not lose another"
    );
    assert_eq!(text(&database, "SELECT state FROM sessions")?, "closed");
    drop(database);

    assert_no_canaries_in_storage(root)
}

#[test]
fn a_half_arriving_for_a_retired_identity_reports_its_own_degradation() -> TestResult {
    let recorded = run_retired_degraded_case()?;
    let root = recorded.directory.path();
    let database = Connection::open(root.join("tracepress.sqlite3"))?;

    // Eviction degraded the held forward once; its status then arrived for an identity the
    // bounded retirement memory still accounts for, which is a second, differently caused loss.
    assert_eq!(
        recorded.report,
        "correlation_degraded_total=2 correlation_degraded_inflight_limit=1 correlation_degraded_retired_limit=1 correlation_missing_total=0",
        "the retirement refusal must be counted under its own reason"
    );
    assert_degraded_events(&database, &["in_flight_limit", "retired_limit"])?;
    // A refused half belongs to no record, so its degradation names the session only.
    assert_eq!(
        integer(
            &database,
            &format!(
                "SELECT COUNT(*) FROM events WHERE event_type = '{DEGRADED_EVENT}' AND operation_id IS NULL",
            ),
        )?,
        1
    );
    assert_eq!(
        integer(
            &database,
            "SELECT COUNT(*) FROM provider_attempts WHERE status_code IS NULL",
        )?,
        1,
        "the held forward must be the one recorded before its status was ever observed"
    );
    assert_eq!(
        integer(&database, "SELECT COUNT(*) FROM provider_requests")?,
        i64::try_from(DEGRADING_FORWARDS)? + 1,
        "a refused half may not cost the forward its record"
    );
    assert_eq!(text(&database, "SELECT state FROM sessions")?, "closed");
    drop(database);

    assert_no_canaries_in_storage(root)
}

#[test]
fn a_fully_correlated_run_reports_no_degradation_at_all() -> TestResult {
    let recorded = run_streamed_case(stream_events())?;
    let database = Connection::open(recorded.directory.path().join("tracepress.sqlite3"))?;

    assert_eq!(
        correlation_report(&recorded.stdout)?,
        UNDEGRADED_REPORT,
        "a run whose every forward correlated must report zeroes, not silence"
    );
    assert_context_events(&database)?;
    Ok(())
}

/// Asserts the degradations one run committed, in order, with their whole payloads.
///
/// A degradation payload is checked in full rather than by lookup: the contract allows it
/// reason, session, and nothing else, and only an exact comparison can prove the absence of a
/// field nobody thought to query.
fn assert_degraded_events(database: &Connection, reasons: &[&str]) -> TestResult {
    let session = text(database, "SELECT session_id FROM sessions")?;
    let mut statement = database.prepare(&format!(
        "SELECT payload, schema_version FROM events WHERE event_type = '{DEGRADED_EVENT}' ORDER BY seq",
    ))?;
    let committed = statement
        .query_map([], |row| {
            Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(
        committed.len(),
        reasons.len(),
        "one degradation is exactly one event"
    );
    for ((payload, schema_version), reason) in committed.iter().zip(reasons) {
        assert_eq!(
            String::from_utf8_lossy(payload),
            format!(r#"{{"reason":"{reason}","session_id":"{session}"}}"#)
        );
        assert_eq!(schema_version, "1");
    }
    Ok(())
}

/// The eligible streamed fixture emits one complete context lifecycle with explicit partial
/// visibility; context events are asserted separately from the Phase 2 provider event contract.
fn assert_context_events(database: &Connection) -> TestResult {
    let mut statement = database
        .prepare("SELECT event_type FROM events WHERE event_type LIKE 'context.%' ORDER BY seq")?;
    let committed = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(
        committed,
        vec![
            "context.analysis.started",
            "context.visibility.partial",
            "context.reconciliation.unavailable",
            "context.analysis.completed",
        ],
        "eligible context analysis must emit its exact bounded event lifecycle",
    );
    Ok(())
}

/// Durable context rows must join the provider attempt and retain the bounded analysis evidence.
fn assert_context_storage(
    database: &Connection,
    expected_snapshots: i64,
    expect_delta: bool,
) -> TestResult {
    assert_eq!(
        integer(
            database,
            "SELECT COUNT(*) FROM context_snapshots s JOIN provider_requests r ON r.request_id = s.provider_request_id JOIN operations o ON o.operation_id = s.inference_operation_id WHERE o.kind = 'llm_inference'",
        )?,
        expected_snapshots,
        "every context snapshot must join one provider request and inference operation",
    );
    assert_eq!(
        integer(
            database,
            "SELECT COUNT(*) FROM context_snapshots WHERE status = 'complete'",
        )?,
        expected_snapshots,
    );
    assert!(
        integer(database, "SELECT COUNT(*) FROM context_block_occurrences")? > 0,
        "completed context snapshots must persist block occurrences",
    );
    assert_eq!(
        integer(database, "SELECT COUNT(*) FROM context_analysis_metrics")?,
        expected_snapshots,
    );
    assert_eq!(
        integer(
            database,
            "SELECT COUNT(*) FROM context_analysis_metrics WHERE explicit_bytes IS NOT NULL",
        )?,
        expected_snapshots,
        "context metrics must retain the explicit-byte aggregate",
    );
    assert_eq!(
        integer(
            database,
            "SELECT COUNT(*) FROM token_reconciliations tr JOIN provider_usage pu ON pu.attempt_id = tr.attempt_id WHERE tr.provider_input_tokens IS NOT NULL",
        )?,
        expected_snapshots,
        "each context snapshot must reconcile against its provider usage row",
    );
    if expect_delta {
        let delta: (i64, i64, i64) = database.query_row(
            "SELECT COUNT(*), COALESCE(MAX(common_prefix_blocks), -1), COUNT(DISTINCT previous_snapshot_id) FROM context_deltas",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        assert_eq!(delta.0, expected_snapshots.saturating_sub(1));
        assert!(
            delta.1 > 0,
            "the second request must retain a non-empty stable context prefix",
        );
        assert_eq!(delta.2, expected_snapshots.saturating_sub(1));
    } else {
        assert_eq!(
            integer(database, "SELECT COUNT(*) FROM context_deltas")?,
            0,
            "the first context snapshot has no predecessor",
        );
    }
    Ok(())
}

#[test]
fn a_forward_without_an_observable_response_records_its_request_half() -> TestResult {
    let directory = run_unobserved_response_case()?;
    let root = directory.path();
    let database = Connection::open(root.join("tracepress.sqlite3"))?;

    // The logical request is kept with the real transport status, and response evidence that
    // was never observed stays NULL instead of being invented.
    assert_eq!(
        text(&database, "SELECT model FROM provider_requests")?,
        "gpt-plain"
    );
    assert_eq!(
        integer(&database, "SELECT COUNT(*) FROM provider_attempts")?,
        1
    );
    assert_eq!(
        optional_integer(&database, "SELECT status_code FROM provider_attempts")?,
        Some(200)
    );
    assert_eq!(
        integer(
            &database,
            "SELECT COUNT(*) FROM provider_attempts WHERE provider_response_id IS NULL AND response_state IS NULL AND byte_count IS NULL AND ended_at IS NULL",
        )?,
        1
    );
    assert_eq!(
        integer(&database, "SELECT COUNT(*) FROM provider_usage")?,
        0
    );
    assert_eq!(
        integer(
            &database,
            "SELECT COUNT(*) FROM operations WHERE kind = 'llm_inference'",
        )?,
        1
    );
    assert_eq!(text(&database, "SELECT state FROM sessions")?, "closed");
    drop(database);

    assert_no_canaries_in_storage(root)
}

#[test]
fn an_errored_upstream_status_alone_closes_the_attempt_and_its_operation() -> TestResult {
    let directory = run_rejected_status_case()?;
    let root = directory.path();
    let database = Connection::open(root.join("tracepress.sqlite3"))?;

    // Nothing interpreted the rejection body, so the upstream status is the forward's only
    // terminal evidence. An errored row that keeps `ended_at IS NULL` claims to be closed while
    // every in-flight query still counts it, so the status must close the attempt itself.
    assert_eq!(
        integer(&database, "SELECT COUNT(*) FROM provider_requests")?,
        1
    );
    assert_eq!(
        text(
            &database,
            "SELECT status || '|' || status_code FROM provider_attempts",
        )?,
        "errored|429"
    );
    assert_eq!(
        integer(
            &database,
            "SELECT COUNT(*) FROM provider_attempts WHERE ended_at IS NOT NULL AND transport_error IS NULL AND response_state IS NULL",
        )?,
        1,
        "an errored attempt must not be indistinguishable from one still in flight"
    );
    assert_eq!(
        text(
            &database,
            "SELECT status FROM operations WHERE kind = 'llm_inference'",
        )?,
        "errored"
    );
    assert_eq!(
        integer(
            &database,
            "SELECT COUNT(*) FROM operations WHERE kind = 'llm_inference' AND ended_at IS NOT NULL",
        )?,
        1,
        "an errored operation must carry the timestamp its attempt was closed at"
    );
    assert_eq!(
        integer(&database, "SELECT COUNT(*) FROM provider_usage")?,
        0
    );
    assert_eq!(text(&database, "SELECT state FROM sessions")?, "closed");
    drop(database);

    assert_no_canaries_in_storage(root)
}

#[test]
fn an_upstream_transport_failure_records_its_failure_class_and_closes_the_attempt() -> TestResult {
    let directory = run_unreachable_upstream_case()?;
    let root = directory.path();
    let database = Connection::open(root.join("tracepress.sqlite3"))?;

    // The forward that never reached an upstream keeps its logical request and is settled with
    // the non-sensitive failure class instead of staying indistinguishable from one in flight.
    assert_eq!(
        integer(&database, "SELECT COUNT(*) FROM provider_requests")?,
        1
    );
    assert_eq!(
        text(
            &database,
            "SELECT status || '|' || transport_error FROM provider_attempts",
        )?,
        "errored|connect"
    );
    assert_eq!(
        optional_integer(&database, "SELECT status_code FROM provider_attempts")?,
        None,
        "no upstream status was ever observed"
    );
    assert_eq!(
        integer(
            &database,
            "SELECT COUNT(*) FROM provider_attempts WHERE ended_at IS NOT NULL",
        )?,
        1,
        "a failed forward must not be left pending"
    );
    assert_eq!(
        integer(&database, "SELECT COUNT(*) FROM provider_usage")?,
        0
    );
    assert_eq!(
        text(
            &database,
            "SELECT status FROM operations WHERE kind = 'llm_inference'",
        )?,
        "errored"
    );
    assert_eq!(
        integer(
            &database,
            "SELECT COUNT(*) FROM events WHERE event_type = 'provider.response.failed'",
        )?,
        1
    );
    drop(database);

    assert_no_canaries_in_storage(root)
}

#[test]
fn a_non_streamed_completion_records_its_measured_duration() -> TestResult {
    let directory = run_json_completion_case()?;
    let root = directory.path();
    let database = Connection::open(root.join("tracepress.sqlite3"))?;

    // A JSON document response is observed off the forwarding task, so §48 latency metrics are
    // not stream-only: the duration of the exchange is measured and persisted.
    assert_eq!(
        text(
            &database,
            "SELECT status || '|' || response_state || '|' || observation_status || '|' || streaming FROM provider_attempts",
        )?,
        "completed|completed|complete|0"
    );
    assert!(
        optional_integer(&database, "SELECT duration_us FROM provider_attempts")?.is_some(),
        "a non-streamed completion must record its measured duration"
    );
    assert!(
        optional_integer(&database, "SELECT ttfb_us FROM provider_attempts")?.is_some(),
        "the first observed upstream byte of a document response is measurable"
    );
    assert_eq!(
        optional_integer(&database, "SELECT ttft_us FROM provider_attempts")?,
        None,
        "a document response has no semantic output event, so TTFT stays unknown"
    );
    assert_eq!(
        text(&database, "SELECT usage_status FROM provider_usage")?,
        "final"
    );
    assert_eq!(
        text(
            &database,
            "SELECT status FROM operations WHERE kind = 'llm_inference'",
        )?,
        "completed"
    );
    assert_eq!(
        integer(
            &database,
            "SELECT COUNT(*) FROM operations WHERE kind = 'llm_inference' AND ended_at IS NOT NULL",
        )?,
        1
    );
    drop(database);

    assert_no_canaries_in_storage(root)
}

fn assert_upstream_saw_exact_request(received: &[u8], request: &str) -> TestResult {
    let header_end = window_position(received, b"\r\n\r\n").ok_or("upstream saw no headers")?;
    let body_start = header_end.saturating_add(4);
    let headers = String::from_utf8_lossy(&received[..body_start]).to_ascii_lowercase();
    assert!(
        headers.contains(&format!("authorization: bearer {AUTH_CANARY}").to_ascii_lowercase()),
        "upstream did not receive the forwarded credential"
    );
    assert_eq!(
        &received[body_start..],
        request.as_bytes(),
        "forwarded request body was not byte exact"
    );
    Ok(())
}

fn assert_provider_request(database: &Connection, request_bytes: i64) -> TestResult {
    assert_eq!(
        integer(database, "SELECT COUNT(*) FROM provider_requests")?,
        1
    );
    assert_eq!(
        text(
            database,
            "SELECT route || '|' || method || '|' || provider || '|' || protocol || '|' || parser_version || '|' || observation_status FROM provider_requests",
        )?,
        "responses|post|openai|openai-responses-v1|1|complete"
    );
    assert_eq!(
        text(
            database,
            "SELECT model || '|' || stream || '|' || store || '|' || reasoning_effort || '|' || text_verbosity || '|' || truncation || '|' || previous_response_id_present || '|' || input_item_count || '|' || tool_count || '|' || text_input_block_count FROM provider_requests",
        )?,
        "gpt-test|1|0|medium|low|auto|1|2|1|1"
    );
    assert_eq!(
        integer(database, "SELECT request_bytes FROM provider_requests")?,
        request_bytes
    );
    Ok(())
}

fn assert_provider_attempt(database: &Connection, response_bytes: i64) -> TestResult {
    assert_eq!(
        integer(database, "SELECT COUNT(*) FROM provider_attempts")?,
        1
    );
    assert_eq!(
        text(
            database,
            "SELECT ordinal || '|' || status || '|' || response_state || '|' || observation_status || '|' || streaming || '|' || provider_response_id || '|' || response_model FROM provider_attempts",
        )?,
        "0|completed|completed|complete|1|resp_e2e_1|gpt-test"
    );
    assert_eq!(
        integer(database, "SELECT byte_count FROM provider_attempts")?,
        response_bytes
    );
    assert!(
        integer(database, "SELECT chunk_count FROM provider_attempts")? >= 1,
        "a streamed attempt must observe upstream chunks"
    );
    assert_eq!(
        optional_integer(database, "SELECT status_code FROM provider_attempts")?,
        Some(200),
        "the upstream status observed by the transport half must reach the attempt"
    );
    Ok(())
}

fn assert_provider_usage(database: &Connection) -> TestResult {
    assert_eq!(integer(database, "SELECT COUNT(*) FROM provider_usage")?, 1);
    assert_eq!(
        text(
            database,
            "SELECT input_total || '|' || input_cached || '|' || cache_read || '|' || input_uncached || '|' || output_total || '|' || output_reasoning || '|' || reasoning || '|' || total || '|' || usage_status || '|' || normalizer_version FROM provider_usage",
        )?,
        "12|4|4|8|8|5|5|20|final|1"
    );
    assert_eq!(
        optional_integer(database, "SELECT cache_write FROM provider_usage")?,
        None,
        "an unreported usage value must stay NULL"
    );
    assert_eq!(
        blob(database, "SELECT raw_usage_json FROM provider_usage")?,
        USAGE_OBJECT.as_bytes(),
        "the provider usage object must be preserved byte for byte"
    );
    Ok(())
}

/// Every latency of a completed streamed run is a real measurement, never a sentinel.
fn assert_measured_timings(database: &Connection) -> TestResult {
    let timings = ["ttfb_us", "ttft_us", "duration_us"].map(|column| {
        optional_integer(database, &format!("SELECT {column} FROM provider_attempts"))
    });
    let [first_byte, first_token, duration] = timings;
    let first_byte = first_byte?.ok_or("time to first byte was not measured")?;
    let first_token = first_token?.ok_or("time to first token was not measured")?;
    let duration = duration?.ok_or("duration was not measured")?;
    assert!(
        first_byte <= first_token && first_token <= duration,
        "measured latencies must be ordered: {first_byte} <= {first_token} <= {duration}"
    );
    // The former placeholder marker must appear in no attempt text column.
    assert_eq!(
        integer(
            database,
            "SELECT COUNT(*) FROM provider_attempts WHERE '0' IN (started_at, ended_at, provider_created_at, incomplete_reason, error_code, transport_error)",
        )?,
        0,
        "unknown must be NULL, never the sentinel '0'"
    );
    Ok(())
}

/// The canonical events one completed observation supports exist once each, without content.
fn assert_canonical_events(database: &Connection) -> TestResult {
    for event_type in [
        "provider.request.observed",
        "provider.response.started",
        "provider.response.completed",
        "provider.usage.observed",
        "provider.usage.normalized",
    ] {
        assert_eq!(
            integer(
                database,
                &format!("SELECT COUNT(*) FROM events WHERE event_type = '{event_type}'"),
            )?,
            1,
            "{event_type} must be committed exactly once"
        );
    }
    for event_type in [
        "provider.response.incomplete",
        "provider.response.failed",
        "provider.observation.partial",
    ] {
        assert_eq!(
            integer(
                database,
                &format!("SELECT COUNT(*) FROM events WHERE event_type = '{event_type}'"),
            )?,
            0,
            "{event_type} is not supported by this evidence"
        );
    }
    // Every Phase 2 provider event hangs off the inference operation of the recorded session;
    // context lifecycle events are asserted independently above.
    assert_eq!(
        integer(
            database,
            "SELECT COUNT(*) FROM events e JOIN operations o ON o.operation_id = e.operation_id JOIN sessions s ON s.session_id = e.session_id WHERE e.event_type LIKE 'provider.%' AND o.kind = 'llm_inference'",
        )?,
        5
    );
    // No payload carries raw usage bytes or any provider content.
    let mut statement = database.prepare("SELECT event_type, payload FROM events")?;
    let payloads = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (event_type, payload) in payloads {
        assert!(
            window_position(&payload, b"input_tokens").is_none(),
            "{event_type} payload leaked raw usage bytes"
        );
        for canary in CANARIES {
            assert!(
                window_position(&payload, canary.as_bytes()).is_none(),
                "{event_type} payload leaked {canary}"
            );
        }
    }
    Ok(())
}

fn assert_causal_dag(database: &Connection) -> TestResult {
    assert_eq!(text(database, "SELECT state FROM sessions")?, "closed");
    // One forward is one inference operation: the semantic record owns the Responses forward,
    // so no transport-only operation is created beside it.
    assert_eq!(
        integer(
            database,
            "SELECT COUNT(*) FROM operations WHERE kind = 'llm_inference'",
        )?,
        1
    );
    assert_eq!(
        integer(
            database,
            "SELECT COUNT(*) FROM causal_edges e JOIN operations parent ON parent.operation_id = e.parent_operation_id JOIN operations child ON child.operation_id = e.child_operation_id WHERE parent.kind = 'agent' AND child.kind = 'llm_inference'",
        )?,
        1,
        "the inference operation must hang off the agent root"
    );
    assert_eq!(
        integer(
            database,
            "SELECT COUNT(*) FROM provider_requests r JOIN operations o ON o.operation_id = r.operation_id JOIN sessions s ON s.session_id = o.session_id WHERE o.kind = 'llm_inference'",
        )?,
        1,
        "the logical request must join one inference of the recorded session"
    );
    Ok(())
}

fn assert_no_canaries_in_storage(root: &Path) -> TestResult {
    let mut scanned = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let path = entry?.path();
        let Some(name) = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned)
        else {
            continue;
        };
        if !name.starts_with("tracepress.sqlite3") {
            continue;
        }
        let bytes = std::fs::read(&path)?;
        for canary in CANARIES {
            assert!(
                window_position(&bytes, canary.as_bytes()).is_none(),
                "{name} leaked {canary}"
            );
        }
        scanned.push(name);
    }
    assert!(!scanned.is_empty(), "no database file was scanned");
    Ok(())
}

fn integer(database: &Connection, query: &str) -> rusqlite::Result<i64> {
    database.query_row(query, [], |row| row.get(0))
}

fn optional_integer(database: &Connection, query: &str) -> rusqlite::Result<Option<i64>> {
    database.query_row(query, [], |row| row.get(0))
}

fn text(database: &Connection, query: &str) -> rusqlite::Result<String> {
    database.query_row(query, [], |row| row.get(0))
}

fn blob(database: &Connection, query: &str) -> rusqlite::Result<Vec<u8>> {
    database.query_row(query, [], |row| row.get(0))
}

fn window_position(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn serve_streamed_sse(listener: &TcpListener, events: &[String]) -> std::io::Result<Vec<u8>> {
    let (mut stream, _peer) = listener.accept()?;
    let received = read_request(&mut stream)?;
    stream.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nTransfer-Encoding: chunked\r\n\r\n",
    )?;
    for event in events {
        write!(stream, "{:x}\r\n", event.len())?;
        stream.write_all(event.as_bytes())?;
        stream.write_all(b"\r\n")?;
        stream.flush()?;
    }
    stream.write_all(b"0\r\n\r\n")?;
    stream.flush()?;
    Ok(received)
}

fn read_request(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut received = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        received.extend_from_slice(&buffer[..count]);
        let Some(body_start) =
            window_position(&received, b"\r\n\r\n").map(|index| index.saturating_add(4))
        else {
            continue;
        };
        let headers =
            std::str::from_utf8(&received[..body_start]).map_err(std::io::Error::other)?;
        let length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(str::trim)
                    .and_then(|value| value.parse::<usize>().ok())
            })
            .unwrap_or_default();
        if received.len() >= body_start.saturating_add(length) {
            return Ok(received);
        }
    }
    Err(std::io::Error::other("incomplete upstream request"))
}

/// A private state root plus the real CLI and daemon binaries under test.
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
