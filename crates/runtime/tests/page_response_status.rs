//! The request log and `/metrics` record the status a page answered with.
//! A page that calls notFound() is a 404 in the log and a client error in the
//! metrics, and a page that fails is a 500 and a server error. Same stub-FnOps
//! harness as ssr_compression.rs.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::Duration;

use pylon_functions::protocol::AuthInfo;
use pylon_functions::registry::FnDef;
use pylon_kernel::{AppManifest, ManifestRoute};
use pylon_runtime::Runtime;

struct StubFnOps;

impl pylon_router::FnOps for StubFnOps {
    fn get_fn(&self, _name: &str) -> Option<FnDef> {
        None
    }

    fn list_fns(&self) -> Vec<FnDef> {
        vec![]
    }

    fn call(
        &self,
        fn_name: &str,
        _args: serde_json::Value,
        _auth: AuthInfo,
        _on_stream: Option<pylon_functions::runner::StreamCallback>,
        _request: Option<pylon_functions::protocol::RequestInfo>,
        _stream_id: Option<String>,
    ) -> Result<
        (serde_json::Value, pylon_functions::trace::FnTrace),
        pylon_functions::runner::FnCallError,
    > {
        panic!("unexpected fn call {fn_name}");
    }

    #[allow(clippy::too_many_arguments)]
    fn render_route(
        &self,
        _component: &str,
        _layouts: Vec<String>,
        _route_path: &str,
        _url: &str,
        _host: &str,
        params: serde_json::Value,
        _search_params: serde_json::Value,
        _headers: HashMap<String, String>,
        _cookies: HashMap<String, String>,
        _auth: AuthInfo,
        _session_present: bool,
        _initial_status: Option<u16>,
        _design: bool,
        on_response_start: Option<pylon_functions::runner::ResponseStartCallback>,
        mut on_chunk: pylon_functions::runner::ByteStreamCallback,
    ) -> Result<(), pylon_functions::runner::FnCallError> {
        if let Some(mut start) = on_response_start {
            let mut headers = HashMap::new();
            headers.insert(
                "content-type".to_string(),
                "text/html; charset=utf-8".to_string(),
            );
            // The page answers with the status in its URL: /item/404 is a
            // page that calls notFound().
            let status = params["id"]
                .as_str()
                .and_then(|s| s.parse().ok())
                .unwrap_or(200);
            start(status, headers);
        }
        on_chunk(b"<!DOCTYPE html><html><body>page</body></html>");
        Ok(())
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
        _auth: AuthInfo,
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

fn manifest() -> AppManifest {
    AppManifest {
        required_env: Vec::new(),
        build: Default::default(),
        shards: Vec::new(),
        services: Vec::new(),
        offline: None,
        manifest_version: 1,
        name: "response-status".into(),
        version: "0.1.0".into(),
        entities: vec![],
        routes: vec![
            ManifestRoute {
                path: "/".into(),
                mode: "ssr".into(),
                query: None,
                auth: None,
                component: Some("app/page".into()),
                layouts: vec![],
                kind: None,
            },
            ManifestRoute {
                path: "/item/:id".into(),
                mode: "ssr".into(),
                query: None,
                auth: None,
                component: Some("app/item/[id]/page".into()),
                layouts: vec![],
                kind: None,
            },
        ],
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
    static NEXT: AtomicU16 = AtomicU16::new(25_000);
    for _ in 0..200 {
        let base = NEXT.fetch_add(4, Ordering::Relaxed);
        let ok = (0..4).all(|off| pylon_runtime::listen::port_is_free(base + off));
        if ok {
            return base;
        }
    }
    panic!("no free 4-port block");
}

fn start_stub_server() -> u16 {
    let port = available_port();
    let rt = Arc::new(Runtime::in_memory(manifest()).unwrap());
    let fn_ops: Arc<dyn pylon_router::FnOps> = Arc::new(StubFnOps);
    std::thread::spawn(move || {
        if let Err(e) = pylon_runtime::server::start_server_for_test_with_fn_ops(rt, port, fn_ops) {
            eprintln!("START ERROR: {e}");
        }
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while std::time::Instant::now() < deadline {
        if get_status(port, "/health", "localhost") == Some(200) {
            return port;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("test server on 127.0.0.1:{port} never answered GET /health");
}

/// GET `path` with the given `Host` header; the status, or None when the
/// server did not answer.
fn get_status(port: u16, path: &str, host: &str) -> Option<u16> {
    let mut stream = TcpStream::connect(format!("127.0.0.1:{port}")).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nAccept: text/html\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).ok()?;
    let mut raw = Vec::new();
    let _ = stream.read_to_end(&mut raw);
    let text = String::from_utf8_lossy(&raw);
    text.split_whitespace().nth(1)?.parse().ok()
}

#[test]
fn the_log_has_the_status_each_page_answered_with() {
    // SAFETY: set before the server thread starts; this binary has one test.
    unsafe {
        // Production mode, which needs a CORS origin and refuses in-memory
        // sessions unless told.
        std::env::remove_var("PYLON_DEV_MODE");
        std::env::set_var("PYLON_SESSION_IN_MEMORY", "1");
        std::env::set_var("PYLON_CORS_ORIGIN", "http://localhost");
    }
    let port = start_stub_server();
    assert_eq!(get_status(port, "/item/200", "localhost"), Some(200));
    assert_eq!(get_status(port, "/item/404", "localhost"), Some(404));
    assert_eq!(get_status(port, "/item/500", "localhost"), Some(500));

    let ring = pylon_runtime::log_ring::log_ring().expect("the server starts the request log");
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let rows = ring.tail_since(None);
        let status_of = |path: &str| rows.iter().find(|r| r.path == path).map(|r| r.status);
        let found = (
            status_of("/item/200"),
            status_of("/item/404"),
            status_of("/item/500"),
        );
        if found == (Some(200), Some(404), Some(500)) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the request log has {found:?} for the three pages, expected 200, 404 and 500"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}
