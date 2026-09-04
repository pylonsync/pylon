//! `POST /api/fn/:name` with `Accept: text/event-stream` must answer
//! with SSE only when the handler actually streams.
//!
//! The bug this pins: MCP streamable-HTTP clients send
//! `Accept: application/json, text/event-stream` on every POST but only
//! parse SSE events of the default `message` type. The old fast path
//! upgraded unconditionally and delivered the result as
//! `event: result`, which those clients ignore — every MCP client
//! talking to a Pylon action timed out. The MCP spec explicitly allows
//! a plain-JSON answer to a POST that advertises SSE support, so a
//! handler that never calls `ctx.stream` must get the router path's
//! plain-JSON response; a handler that streams keeps the SSE shape.
//!
//! We drive the REAL request loop with a stub `FnOps` (no Bun).

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;

use pylon_functions::protocol::FnType;
use pylon_functions::registry::{FnAuthMode, FnDef};
use pylon_kernel::AppManifest;
use pylon_runtime::Runtime;

/// Three public functions with different runtime behavior:
/// - `plainFn` returns a value without ever streaming.
/// - `streamingFn` emits two chunks, then returns a value.
/// - `errFn` fails without ever streaming.
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
            "plainFn" | "streamingFn" | "errFn" => Some(def(name)),
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
        match fn_name {
            "plainFn" => Ok((
                serde_json::json!({"jsonrpc": "2.0", "id": 1, "result": {"ok": true}}),
                stub_trace(fn_name),
            )),
            "streamingFn" => {
                if let Some(cb) = on_stream.as_mut() {
                    cb(pylon_functions::runner::StreamChunk {
                        data: "chunk-one",
                        event: None,
                    });
                    cb(pylon_functions::runner::StreamChunk {
                        data: "chunk-two",
                        event: None,
                    });
                }
                Ok((serde_json::json!({"done": true}), stub_trace(fn_name)))
            }
            "errFn" => Err(pylon_functions::runner::FnCallError {
                code: "BOOM".into(),
                message: "handler failed before streaming".into(),
            }),
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
        manifest_version: 1,
        name: "fn-sse-negotiation".into(),
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
    // Below the ephemeral range (Linux 32768-60999): a port up there can be
    // handed to one of these tests' OWN client sockets between the probe
    // below and the server's bind, which surfaces as EADDRINUSE on CI.
    // One 1000-port lane per test binary so parallel binaries can't overlap.
    static NEXT: AtomicU16 = AtomicU16::new(26_000);
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
    let boot_err: std::sync::Arc<std::sync::Mutex<Option<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let boot_err_thread = std::sync::Arc::clone(&boot_err);
    std::thread::spawn(move || {
        let r = pylon_runtime::server::start_server_for_test_with_fn_ops(rt, port, fn_ops);
        if let Err(e) = r {
            *boot_err_thread.lock().unwrap() = Some(e.to_string());
        }
    });
    let mut ready = false;
    // Bound the wall clock, not the attempt count: a failed connect is
    // not instant on every platform (see csrf_form_route.rs).
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        if TcpStream::connect(format!("127.0.0.1:{port}")).is_ok() {
            ready = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        ready,
        "test server never bound 127.0.0.1:{port} within 15s (server error: {:?})",
        boot_err.lock().unwrap()
    );
    port
}

/// POST with the MCP client's exact Accept header. Returns the full
/// raw HTTP response text (status line + headers + body).
fn mcp_style_post(port: u16, fn_name: &str) -> String {
    let host_port = format!("127.0.0.1:{port}");
    let body = "{}";
    let request = format!(
        "POST /api/fn/{fn_name} HTTP/1.1\r\nHost: {host_port}\r\n\
         Accept: application/json, text/event-stream\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream = TcpStream::connect(&host_port).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
    stream.write_all(request.as_bytes()).expect("write");
    let mut response = Vec::new();
    let _ = stream.read_to_end(&mut response);
    String::from_utf8_lossy(&response).into_owned()
}

fn status_of(response: &str) -> u16 {
    response
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0)
}

fn body_of(response: &str) -> &str {
    response.split("\r\n\r\n").nth(1).unwrap_or("")
}

#[test]
fn non_streaming_fn_answers_plain_json_despite_sse_accept() {
    let port = start_stub_server();
    let resp = mcp_style_post(port, "plainFn");

    assert_eq!(status_of(&resp), 200, "response: {resp}");
    assert!(
        resp.to_ascii_lowercase()
            .contains("content-type: application/json"),
        "a handler that never streams must answer application/json, got: {resp}"
    );
    assert!(
        !resp.contains("event: result"),
        "no SSE framing for a non-streaming handler: {resp}"
    );
    // The body is the raw return value — an MCP client can parse it as
    // the JSON-RPC response directly. tiny_http may chunk-encode the
    // body, so parse the JSON out of it rather than string-comparing.
    let raw = body_of(&resp);
    let json_start = raw.find('{').expect("JSON object in body");
    let json_end = raw.rfind('}').expect("JSON object in body");
    let value: serde_json::Value =
        serde_json::from_str(&raw[json_start..=json_end]).expect("body parses as JSON");
    assert_eq!(value["jsonrpc"], "2.0");
    assert_eq!(value["result"]["ok"], true);
}

#[test]
fn streaming_fn_keeps_sse_shape() {
    let port = start_stub_server();
    let resp = mcp_style_post(port, "streamingFn");

    assert_eq!(status_of(&resp), 200, "response: {resp}");
    assert!(
        resp.to_ascii_lowercase()
            .contains("content-type: text/event-stream"),
        "a handler that streams must answer SSE, got: {resp}"
    );
    assert!(resp.contains("data: chunk-one"), "first chunk: {resp}");
    assert!(resp.contains("data: chunk-two"), "second chunk: {resp}");
    assert!(
        resp.contains("event: result"),
        "final result event must close the stream: {resp}"
    );
}

#[test]
fn non_streaming_error_answers_router_shaped_400() {
    let port = start_stub_server();
    let resp = mcp_style_post(port, "errFn");

    assert_eq!(status_of(&resp), 400, "response: {resp}");
    assert!(
        resp.to_ascii_lowercase()
            .contains("content-type: application/json"),
        "an error before any stream chunk must be a plain-JSON 400: {resp}"
    );
    assert!(resp.contains("BOOM"), "error code in body: {resp}");
    assert!(
        !resp.contains("event: error"),
        "no SSE error framing for a never-streamed call: {resp}"
    );
}
