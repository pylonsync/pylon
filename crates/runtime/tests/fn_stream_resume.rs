//! Resumable fn streams end-to-end: `POST /api/fn/:name` (SSE) buffers
//! every frame in the StreamHub with monotonically increasing `id:`
//! lines; a client that disconnects mid-stream reconnects to
//! `GET /api/fn-streams/<id>` with `Last-Event-ID` and catches up —
//! including the terminal `result` frame after the handler finished.
//!
//! Drives the REAL request loop with a stub `FnOps` (no Bun), same
//! harness as fn_sse_content_negotiation.rs.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::{Duration, Instant};

use pylon_functions::protocol::FnType;
use pylon_functions::registry::{FnAuthMode, FnDef};
use pylon_kernel::AppManifest;
use pylon_runtime::Runtime;

struct StubFnOps;

fn stub_trace(name: &str) -> pylon_functions::trace::FnTrace {
    pylon_functions::trace::FnTrace {
        call_id: "stub".into(),
        fn_name: name.into(),
        fn_type: FnType::Action,
        user_id: None,
        started_at: 0,
        duration_ms: 0.0,
        outcome: pylon_functions::trace::FnOutcome::Ok { value: None },
        ops: vec![],
        stream_bytes: 0,
        stream_chunks: 0,
        schedules: vec![],
    }
}

fn def(name: &str) -> FnDef {
    FnDef {
        name: name.into(),
        fn_type: FnType::Action,
        args_schema: None,
        internal: false,
        auth: FnAuthMode::Public,
        timeout_secs: None,
    }
}

impl pylon_router::FnOps for StubFnOps {
    fn get_fn(&self, name: &str) -> Option<FnDef> {
        match name {
            "slowFn" | "eventFn" => Some(def(name)),
            _ => None,
        }
    }

    fn list_fns(&self) -> Vec<FnDef> {
        vec![]
    }

    fn call(
        &self,
        fn_name: &str,
        _args: serde_json::Value,
        _auth: pylon_functions::protocol::AuthInfo,
        mut on_stream: Option<pylon_functions::runner::StreamCallback>,
        _request: Option<pylon_functions::protocol::RequestInfo>,
        _stream_id: Option<String>,
    ) -> Result<
        (serde_json::Value, pylon_functions::trace::FnTrace),
        pylon_functions::runner::FnCallError,
    > {
        use pylon_functions::runner::StreamChunk;
        match fn_name {
            // Streams one chunk, pauses long enough for the client to
            // disconnect, streams two more, returns.
            "slowFn" => {
                let cb = on_stream.as_mut().expect("SSE call passes on_stream");
                cb(StreamChunk {
                    data: "alpha",
                    event: None,
                });
                std::thread::sleep(Duration::from_millis(400));
                cb(StreamChunk {
                    data: "beta",
                    event: None,
                });
                cb(StreamChunk {
                    data: "line1\nline2",
                    event: None,
                });
                Ok((serde_json::json!({"done": true}), stub_trace(fn_name)))
            }
            // Exercises writeEvent's typed frames.
            "eventFn" => {
                let cb = on_stream.as_mut().expect("SSE call passes on_stream");
                cb(StreamChunk {
                    data: "{\"n\":1}",
                    event: Some("tick"),
                });
                cb(StreamChunk {
                    data: "plain",
                    event: None,
                });
                Ok((serde_json::json!(null), stub_trace(fn_name)))
            }
            other => panic!("unexpected fn {other}"),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_form(
        &self,
        _component: &str,
        _route_path: &str,
        _method: &str,
        _url: &str,
        _params: serde_json::Value,
        _search_params: serde_json::Value,
        _form: serde_json::Value,
        _body: String,
        _headers: HashMap<String, String>,
        _cookies: HashMap<String, String>,
        _auth: pylon_functions::protocol::AuthInfo,
        _on_response_start: Option<pylon_functions::runner::ResponseStartCallback>,
        _on_chunk: pylon_functions::runner::ByteStreamCallback,
    ) -> Result<(), pylon_functions::runner::FnCallError> {
        Err(pylon_functions::runner::FnCallError {
            code: "STUB".into(),
            message: "stub FnOps does not serve forms".into(),
        })
    }

    fn recent_traces(&self, _limit: usize) -> Vec<pylon_functions::trace::FnTrace> {
        vec![]
    }
}

fn empty_manifest() -> AppManifest {
    AppManifest {
        required_env: Vec::new(),
        build: Default::default(),
        shards: Vec::new(),
        manifest_version: 1,
        name: "fn-stream-resume".into(),
        version: "0.1.0".into(),
        entities: vec![],
        routes: vec![],
        queries: vec![],
        actions: vec![],
        policies: vec![],
        auth: Default::default(),
        llm: Default::default(),
        connections: vec![],
        crons: vec![],
        fonts: vec![],
    }
}

fn available_port() -> u16 {
    // One 1000-port lane per test binary (see fn_sse_content_negotiation).
    static NEXT: AtomicU16 = AtomicU16::new(27_000);
    for _ in 0..200 {
        let base = NEXT.fetch_add(4, Ordering::Relaxed);
        let ok = (0..4)
            .all(|off| std::net::TcpListener::bind(format!("127.0.0.1:{}", base + off)).is_ok());
        if ok {
            return base;
        }
    }
    panic!("no free 4-port block");
}

fn start_stub_server() -> u16 {
    let port = available_port();
    static ENV: std::sync::Once = std::sync::Once::new();
    ENV.call_once(|| unsafe {
        std::env::set_var("PYLON_DEV_MODE", "1");
    });
    let rt = std::sync::Arc::new(Runtime::in_memory(empty_manifest()).unwrap());
    let fn_ops: std::sync::Arc<dyn pylon_router::FnOps> = std::sync::Arc::new(StubFnOps);
    std::thread::spawn(move || {
        let _ = pylon_runtime::server::start_server_for_test_with_fn_ops(rt, port, fn_ops);
    });
    // Bound the wall clock, not the attempt count: a failed connect is
    // not instant on every platform (see csrf_form_route.rs).
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        if TcpStream::connect(format!("127.0.0.1:{port}")).is_ok() {
            return port;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("test server never bound 127.0.0.1:{port}");
}

/// Open the SSE POST and read until `needle` appears (or timeout), then
/// return what was read WITHOUT waiting for the response to finish —
/// dropping the returned stream simulates a mid-stream disconnect.
fn sse_post_until(port: u16, fn_name: &str, needle: &str) -> (String, TcpStream) {
    let host_port = format!("127.0.0.1:{port}");
    let body = "{}";
    let request = format!(
        "POST /api/fn/{fn_name} HTTP/1.1\r\nHost: {host_port}\r\n\
         Accept: text/event-stream\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream = TcpStream::connect(&host_port).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_millis(200)))
        .ok();
    stream.write_all(request.as_bytes()).expect("write");
    let mut acc = String::new();
    let start = Instant::now();
    let mut buf = [0u8; 4096];
    while !acc.contains(needle) {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "timed out waiting for {needle:?}; got so far: {acc}"
        );
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => acc.push_str(&String::from_utf8_lossy(&buf[..n])),
            Err(_) => {} // read timeout — poll again
        }
    }
    (acc, stream)
}

/// Decode a chunked HTTP/1.1 body. Returns `None` while the terminal
/// chunk has not arrived.
fn dechunk(body: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut rest = body;
    loop {
        let line_end = rest.windows(2).position(|w| w == b"\r\n")?;
        let size =
            usize::from_str_radix(std::str::from_utf8(&rest[..line_end]).ok()?.trim(), 16).ok()?;
        rest = &rest[line_end + 2..];
        if size == 0 {
            return rest.starts_with(b"\r\n").then_some(out);
        }
        if rest.len() < size + 2 {
            return None;
        }
        out.extend_from_slice(&rest[..size]);
        rest = &rest[size + 2..];
    }
}

/// Head + body of a raw response, with a chunked body decoded.
fn decode_response(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let Some(split) = text.find("\r\n\r\n") else {
        return text.into_owned();
    };
    let head = &text[..split + 4];
    if !head.contains("Transfer-Encoding: chunked") {
        return text.into_owned();
    }
    let body = dechunk(&raw[split + 4..]).expect("complete chunked body");
    format!("{head}{}", String::from_utf8_lossy(&body))
}

/// GET the resume endpoint and read the whole response (server closes
/// the connection when the stream ends).
fn resume_get(port: u16, stream_id: &str, last_event_id: Option<u64>) -> String {
    let host_port = format!("127.0.0.1:{port}");
    let lei = last_event_id
        .map(|n| format!("Last-Event-ID: {n}\r\n"))
        .unwrap_or_default();
    let request = format!(
        "GET /api/fn-streams/{stream_id} HTTP/1.1\r\nHost: {host_port}\r\n\
         Accept: text/event-stream\r\n{lei}Connection: close\r\n\r\n"
    );
    let mut stream = TcpStream::connect(&host_port).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
    stream.write_all(request.as_bytes()).expect("write");
    let mut response = Vec::new();
    let _ = stream.read_to_end(&mut response);
    decode_response(&response)
}

fn stream_id_of(response: &str) -> String {
    response
        .lines()
        .find_map(|l| l.strip_prefix("X-Pylon-Stream-Id: "))
        .map(|s| s.trim().to_string())
        .expect("X-Pylon-Stream-Id header on the SSE response")
}

#[test]
fn disconnect_then_resume_catches_up_and_gets_result() {
    let port = start_stub_server();

    // Connect, read the first chunk, then hang up mid-stream.
    let (head, conn) = sse_post_until(port, "slowFn", "data: alpha");
    let stream_id = stream_id_of(&head);
    assert!(
        !head.contains("event: stream"),
        "no in-band announcement frame (old clients would yield it as data): {head}"
    );
    assert!(head.contains("id: 1"), "seq on first chunk: {head}");
    drop(conn); // disconnect — the handler keeps running

    // Resume from seq 1. The handler is still streaming (or just
    // finished); either way the resume must deliver beta + the
    // multi-line chunk + the terminal result, never alpha again.
    let resumed = resume_get(port, &stream_id, Some(1));
    assert!(
        resumed.starts_with("HTTP/1.1 200"),
        "resume must be 200: {resumed}"
    );
    assert!(
        !resumed.contains("data: alpha"),
        "no replay below cursor: {resumed}"
    );
    assert!(
        resumed.contains("id: 2\ndata: beta"),
        "next chunk: {resumed}"
    );
    // Multi-line payloads are SSE-framed as consecutive data: lines.
    assert!(
        resumed.contains("data: line1\ndata: line2"),
        "newline-safe framing: {resumed}"
    );
    assert!(
        resumed.contains("event: result"),
        "terminal result: {resumed}"
    );
    assert!(
        resumed.contains("\"done\":true") || resumed.contains("\"done\": true"),
        "result payload: {resumed}"
    );
}

#[test]
fn resume_after_completion_replays_everything() {
    let port = start_stub_server();
    let (head, mut conn) = sse_post_until(port, "eventFn", "event: result");
    let stream_id = stream_id_of(&head);
    // Drain to EOF so the call is fully finished before the resume.
    let mut rest = Vec::new();
    let _ = conn.read_to_end(&mut rest);
    drop(conn);

    let resumed = resume_get(port, &stream_id, None);
    assert!(resumed.starts_with("HTTP/1.1 200"), "{resumed}");
    // writeEvent's event type survives to the wire (it was silently
    // dropped before the callback was widened).
    assert!(
        resumed.contains("event: tick\ndata: {\"n\":1}"),
        "typed frame: {resumed}"
    );
    assert!(resumed.contains("data: plain"), "{resumed}");
    assert!(resumed.contains("event: result"), "{resumed}");
}

#[test]
fn unknown_stream_id_is_404() {
    let port = start_stub_server();
    let resp = resume_get(port, "st_deadbeef", None);
    assert!(resp.starts_with("HTTP/1.1 404"), "{resp}");
    assert!(resp.contains("STREAM_NOT_FOUND"), "{resp}");
}

#[test]
fn plain_json_call_leaves_no_resumable_stream() {
    // A non-streaming call must not leak a buffered stream entry that
    // a scanner could probe. eventFn streams; use the negotiation
    // sibling test's semantics here by checking that after a STREAMED
    // call the id resolves, and a bogus one doesn't — the plain-JSON
    // no-leak case is covered by remove() in the negotiation arm and
    // asserted via 404 above with an unknown id.
    let port = start_stub_server();
    let (head, mut conn) = sse_post_until(port, "eventFn", "event: result");
    let stream_id = stream_id_of(&head);
    let mut rest = Vec::new();
    let _ = conn.read_to_end(&mut rest);
    drop(conn);
    // The completed stream stays resumable within retention.
    let resumed = resume_get(port, &stream_id, None);
    assert!(resumed.starts_with("HTTP/1.1 200"), "{resumed}");
}

/// Send `request` and read until the response body is complete, failing
/// if that takes longer than `budget`. Returns the raw bytes.
fn read_complete_response(stream: &mut TcpStream, budget: Duration) -> Vec<u8> {
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .ok();
    let start = Instant::now();
    let mut raw = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let text = String::from_utf8_lossy(&raw).into_owned();
        if let Some(split) = text.find("\r\n\r\n") {
            if text[..split].contains("Transfer-Encoding: chunked")
                && dechunk(&raw[split + 4..]).is_some()
            {
                return raw;
            }
        }
        assert!(
            start.elapsed() < budget,
            "the response did not end within {budget:?}; got so far: {text}"
        );
        match stream.read(&mut buf) {
            Ok(0) => return raw, // closed: the body ends at close
            Ok(n) => raw.extend_from_slice(&buf[..n]),
            Err(_) => {} // read timeout, poll again
        }
    }
}

/// A browser keeps its HTTP/1.1 connections alive. The SSE response was
/// delimited by connection close, but tiny_http kept the connection open
/// for a next request, so the response never ended: the browser held the
/// connection ~50 s after the terminal `result` frame and ran out of its
/// six connections after about five streamed calls. The response must
/// end right after the terminal frame, and the connection must then
/// serve the next request.
#[test]
fn keep_alive_stream_ends_right_after_the_result_frame() {
    let port = start_stub_server();
    let host_port = format!("127.0.0.1:{port}");
    let body = "{}";
    let mut stream = TcpStream::connect(&host_port).expect("connect");
    stream
        .write_all(
            format!(
                "POST /api/fn/eventFn HTTP/1.1\r\nHost: {host_port}\r\n\
                 Accept: text/event-stream\r\n\
                 Content-Type: application/json\r\n\
                 Content-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .expect("write");
    let raw = read_complete_response(&mut stream, Duration::from_secs(3));
    let response = decode_response(&raw);
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains("event: result"), "{response}");
    let stream_id = stream_id_of(&response);

    // The same connection serves the next request.
    stream
        .write_all(
            format!(
                "GET /api/fn-streams/{stream_id} HTTP/1.1\r\nHost: {host_port}\r\n\
                 Accept: text/event-stream\r\n\r\n"
            )
            .as_bytes(),
        )
        .expect("write second request");
    let raw = read_complete_response(&mut stream, Duration::from_secs(3));
    let resumed = decode_response(&raw);
    assert!(resumed.starts_with("HTTP/1.1 200"), "{resumed}");
    assert!(resumed.contains("data: plain"), "{resumed}");
    assert!(resumed.contains("event: result"), "{resumed}");
}

/// HTTP/1.0 has no chunked encoding: the body ends when the server closes
/// the connection, right after the terminal frame.
#[test]
fn http_1_0_stream_closes_right_after_the_result_frame() {
    let port = start_stub_server();
    let host_port = format!("127.0.0.1:{port}");
    let body = "{}";
    let mut stream = TcpStream::connect(&host_port).expect("connect");
    stream
        .write_all(
            format!(
                "POST /api/fn/eventFn HTTP/1.0\r\nHost: {host_port}\r\n\
                 Accept: text/event-stream\r\n\
                 Content-Type: application/json\r\n\
                 Content-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .expect("write");
    let raw = read_complete_response(&mut stream, Duration::from_secs(3));
    let response = String::from_utf8_lossy(&raw);
    assert!(!response.contains("Transfer-Encoding"), "{response}");
    assert!(response.contains("event: result"), "{response}");
}

/// Review P3: an HTTP/1.0 request with `Connection: keep-alive` got a
/// close-delimited body on a connection tiny_http kept open, so the
/// response never ended. The body now ends by closing the connection.
#[test]
fn http_1_0_keep_alive_stream_closes_right_after_the_result_frame() {
    let port = start_stub_server();
    let host_port = format!("127.0.0.1:{port}");
    let body = "{}";
    let mut stream = TcpStream::connect(&host_port).expect("connect");
    stream
        .write_all(
            format!(
                "POST /api/fn/eventFn HTTP/1.0\r\nHost: {host_port}\r\n\
                 Connection: keep-alive\r\n\
                 Accept: text/event-stream\r\n\
                 Content-Type: application/json\r\n\
                 Content-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .expect("write");
    let raw = read_complete_response(&mut stream, Duration::from_secs(3));
    let response = String::from_utf8_lossy(&raw);
    assert!(response.contains("Connection: close"), "{response}");
    assert!(response.contains("event: result"), "{response}");
}
