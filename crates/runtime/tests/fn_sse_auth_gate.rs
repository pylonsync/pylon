//! Regression: the streaming fast path (`POST /api/fn/:name` with
//! `Accept: text/event-stream`) must enforce the declarative function
//! auth mode, exactly like the non-streaming router path does.
//!
//! The bug this pins: the SSE fast path in `server.rs` mirrored the
//! router's gates for existence, `internal`, and rate limiting — but
//! never called `check_fn_auth`. Since `auth: "user"` is the default
//! for every function def, an anonymous caller could invoke any
//! non-internal function just by adding `Accept: text/event-stream`.
//!
//! We drive the REAL request loop with a stub `FnOps` (no Bun). The
//! stub records whether `call` was reached — for a denied request it
//! must never be.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::Arc;
use std::time::Duration;

use pylon_functions::protocol::FnType;
use pylon_functions::registry::{FnAuthMode, FnDef};
use pylon_kernel::AppManifest;
use pylon_runtime::Runtime;

/// Serves three functions with different declared auth modes; `call`
/// flips `call_reached` so the tests can assert the gate fired BEFORE
/// invocation, not after.
struct StubFnOps {
    call_reached: Arc<AtomicBool>,
}

fn def(name: &str, auth: FnAuthMode) -> FnDef {
    FnDef {
        name: name.into(),
        fn_type: FnType::Action,
        args_schema: None,
        internal: false,
        auth,
        timeout_secs: None,
    }
}

impl pylon_router::FnOps for StubFnOps {
    fn get_fn(&self, name: &str) -> Option<FnDef> {
        match name {
            "userFn" => Some(def("userFn", FnAuthMode::User)),
            "adminFn" => Some(def("adminFn", FnAuthMode::Admin)),
            "openFn" => Some(def("openFn", FnAuthMode::Public)),
            _ => None,
        }
    }

    fn list_fns(&self) -> Vec<FnDef> {
        vec![]
    }

    fn call(
        &self,
        _fn_name: &str,
        _args: serde_json::Value,
        _auth: pylon_functions::protocol::AuthInfo,
        _on_stream: Option<Box<dyn FnMut(&str) + Send>>,
        _request: Option<pylon_functions::protocol::RequestInfo>,
    ) -> Result<
        (serde_json::Value, pylon_functions::trace::FnTrace),
        pylon_functions::runner::FnCallError,
    > {
        self.call_reached.store(true, Ordering::SeqCst);
        Err(pylon_functions::runner::FnCallError {
            code: "STUB".into(),
            message: "stub FnOps does not run functions".into(),
        })
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
        name: "fn-sse-auth".into(),
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
    static NEXT: AtomicU16 = AtomicU16::new(44_400);
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

fn start_stub_server(call_reached: Arc<AtomicBool>) -> u16 {
    let port = available_port();
    // Once per binary — a per-call set_var races other test threads
    // reading the environment. See the note in e2e_smoke.rs.
    static ENV: std::sync::Once = std::sync::Once::new();
    ENV.call_once(|| unsafe {
        std::env::set_var("PYLON_DEV_MODE", "1");
    });
    let rt = Arc::new(Runtime::in_memory(empty_manifest()).unwrap());
    let fn_ops: Arc<dyn pylon_router::FnOps> = Arc::new(StubFnOps { call_reached });
    std::thread::spawn(move || {
        let _ = pylon_runtime::server::start_server_for_test_with_fn_ops(rt, port, fn_ops);
    });
    for _ in 0..100 {
        if TcpStream::connect(format!("127.0.0.1:{port}")).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    port
}

/// Anonymous streaming invoke: POST with `Accept: text/event-stream`
/// and no session cookie. Returns the HTTP status line's code.
fn sse_post_anonymous(port: u16, fn_name: &str) -> u16 {
    let host_port = format!("127.0.0.1:{port}");
    let body = "{}";
    let request = format!(
        "POST /api/fn/{fn_name} HTTP/1.1\r\nHost: {host_port}\r\n\
         Accept: text/event-stream\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream = TcpStream::connect(&host_port).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    stream.write_all(request.as_bytes()).expect("write");
    let mut response = Vec::new();
    let _ = stream.read_to_end(&mut response);
    let text = String::from_utf8_lossy(&response);
    text.lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0)
}

#[test]
fn sse_fast_path_enforces_fn_auth_mode() {
    let call_reached = Arc::new(AtomicBool::new(false));
    let port = start_stub_server(Arc::clone(&call_reached));

    // 1. `auth: "user"` (the default for every def) — anonymous SSE
    //    invoke must be 401 and the function must never run. Before
    //    the fix this returned 200 and invoked the handler.
    let status = sse_post_anonymous(port, "userFn");
    assert_eq!(
        status, 401,
        "anonymous SSE invoke of an auth:\"user\" function must be 401, got {status}"
    );
    assert!(
        !call_reached.load(Ordering::SeqCst),
        "the function must NOT be invoked for a denied streaming request"
    );

    // 2. `auth: "admin"` — anonymous caller gets 403, still no invoke.
    let status = sse_post_anonymous(port, "adminFn");
    assert_eq!(
        status, 403,
        "anonymous SSE invoke of an auth:\"admin\" function must be 403, got {status}"
    );
    assert!(
        !call_reached.load(Ordering::SeqCst),
        "the function must NOT be invoked for an admin-gated streaming request"
    );

    // 3. `auth: "public"` — the gate must still let explicit public
    //    functions stream anonymously; the stub records the invoke.
    let status = sse_post_anonymous(port, "openFn");
    assert!(
        call_reached.load(Ordering::SeqCst),
        "an auth:\"public\" function must still be invocable over SSE (status {status})"
    );
    assert_ne!(status, 401, "public fn must not be auth-rejected");
    assert_ne!(status, 403, "public fn must not be auth-rejected");
}
