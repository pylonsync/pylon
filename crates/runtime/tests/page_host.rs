//! `props.host` end-to-end against the REAL request loop with a stub `FnOps`
//! (no Bun), same harness as design_variant.rs.
//!
//! The host passes the page the request's host ONLY when it trusts it, and the
//! value is the SSR cache key's host dimension. These assertions cover that
//! contract: loopback and configured hosts pass through lowercased, anything
//! else reads as "". What the Bun runtime does with the value (props,
//! hydration, the bucket tail) is covered by
//! packages/functions/src/ssr-page-host.test.ts.
//!
//! `PYLON_TRUSTED_HOSTS` is read per request, so the phases run in ONE test
//! function, in order.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pylon_functions::protocol::AuthInfo;
use pylon_functions::registry::FnDef;
use pylon_kernel::{AppManifest, ManifestRoute};
use pylon_runtime::Runtime;

struct StubFnOps {
    hosts: Arc<Mutex<Vec<String>>>,
}

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
        host: &str,
        _params: serde_json::Value,
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
        self.hosts.lock().unwrap().push(host.to_string());
        if let Some(mut start) = on_response_start {
            let mut headers = HashMap::new();
            headers.insert(
                "content-type".to_string(),
                "text/html; charset=utf-8".to_string(),
            );
            start(200, headers);
        }
        on_chunk(b"<!DOCTYPE html><html><body>hi</body></html>");
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
        manifest_version: 1,
        name: "page-host".into(),
        version: "0.1.0".into(),
        entities: vec![],
        routes: vec![ManifestRoute {
            path: "/".into(),
            mode: "ssr".into(),
            query: None,
            auth: None,
            component: Some("app/page".into()),
            layouts: vec![],
            kind: None,
        }],
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
    static NEXT: AtomicU16 = AtomicU16::new(17_000);
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

fn start_stub_server(hosts: Arc<Mutex<Vec<String>>>) -> u16 {
    let port = available_port();
    let rt = Arc::new(Runtime::in_memory(manifest()).unwrap());
    let fn_ops: Arc<dyn pylon_router::FnOps> = Arc::new(StubFnOps { hosts });
    std::thread::spawn(move || {
        let _ = pylon_runtime::server::start_server_for_test_with_fn_ops(rt, port, fn_ops);
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

/// The host the page render received for one request with `Host: host`.
fn rendered_host(port: u16, hosts: &Arc<Mutex<Vec<String>>>, host: &str) -> String {
    let before = hosts.lock().unwrap().len();
    assert_eq!(
        get_status(port, "/", host),
        Some(200),
        "GET / with Host: {host}"
    );
    let seen = hosts.lock().unwrap();
    assert_eq!(seen.len(), before + 1, "one render for Host: {host}");
    seen.last().unwrap().clone()
}

#[test]
fn page_host_is_the_trusted_request_host_or_empty() {
    // Dev mode, like the other stub-server tests: outside it the server
    // refuses to boot with in-memory sessions. The host rule is the same in
    // both modes.
    unsafe {
        std::env::set_var("PYLON_DEV_MODE", "1");
        std::env::remove_var("PYLON_PUBLIC_URL");
        std::env::remove_var("PYLON_CANONICAL_HOST");
        std::env::set_var("PYLON_TRUSTED_HOSTS", "www.example.com, Feedback.Acme.test");
    }
    let hosts: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let port = start_stub_server(hosts.clone());

    // Loopback passes through with its port: that is the dev server.
    let loopback = format!("127.0.0.1:{port}");
    assert_eq!(rendered_host(port, &hosts, &loopback), loopback);

    // A configured host passes through, lowercased on both sides.
    assert_eq!(
        rendered_host(port, &hosts, "feedback.acme.test"),
        "feedback.acme.test"
    );
    assert_eq!(
        rendered_host(port, &hosts, "FEEDBACK.ACME.TEST"),
        "feedback.acme.test"
    );
    assert_eq!(
        rendered_host(port, &hosts, "www.example.com"),
        "www.example.com"
    );

    // Anything the server does not trust reads as "": a forged Host cannot
    // choose what a page renders.
    assert_eq!(rendered_host(port, &hosts, "evil.example"), "");
    assert_eq!(
        rendered_host(port, &hosts, "feedback.acme.test.evil.example"),
        ""
    );

    unsafe {
        std::env::remove_var("PYLON_TRUSTED_HOSTS");
    }
}
