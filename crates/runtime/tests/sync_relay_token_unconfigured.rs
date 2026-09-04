//! `GET /api/sync/relay-token` is an OPT-IN surface: with no relay env
//! configured it must 404 (a plain install must not expose a signing
//! oracle). Own binary because the configured-path test sets the env
//! vars process-globally.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::Duration;

use pylon_kernel::AppManifest;
use pylon_runtime::Runtime;

/// Minimal FnOps stub — this test never invokes functions.
struct NoopFnOps;

impl pylon_router::FnOps for NoopFnOps {
    fn get_fn(&self, _name: &str) -> Option<pylon_functions::registry::FnDef> {
        None
    }
    fn list_fns(&self) -> Vec<pylon_functions::registry::FnDef> {
        vec![]
    }
    fn call(
        &self,
        _fn_name: &str,
        _args: serde_json::Value,
        _auth: pylon_functions::protocol::AuthInfo,
        _on_stream: Option<pylon_functions::runner::StreamCallback>,
        _request: Option<pylon_functions::protocol::RequestInfo>,
        _stream_id: Option<String>,
    ) -> Result<
        (serde_json::Value, pylon_functions::trace::FnTrace),
        pylon_functions::runner::FnCallError,
    > {
        Err(pylon_functions::runner::FnCallError {
            code: "STUB".into(),
            message: "stub".into(),
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
        _body: String,
        _headers: std::collections::HashMap<String, String>,
        _cookies: std::collections::HashMap<String, String>,
        _auth: pylon_functions::protocol::AuthInfo,
        _on_response_start: Option<pylon_functions::runner::ResponseStartCallback>,
        _on_chunk: pylon_functions::runner::ByteStreamCallback,
    ) -> Result<(), pylon_functions::runner::FnCallError> {
        Err(pylon_functions::runner::FnCallError {
            code: "STUB".into(),
            message: "stub".into(),
        })
    }
    fn recent_traces(&self, _limit: usize) -> Vec<pylon_functions::trace::FnTrace> {
        vec![]
    }
}

fn available_port() -> u16 {
    static NEXT: AtomicU16 = AtomicU16::new(27_500);
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

#[test]
fn relay_token_404s_when_no_relay_is_configured() {
    static ENV: std::sync::Once = std::sync::Once::new();
    ENV.call_once(|| unsafe {
        std::env::set_var("PYLON_DEV_MODE", "1");
        std::env::remove_var("PYLON_SYNC_RELAY_URL");
        std::env::remove_var("PYLON_SYNC_RELAY_SECRET");
    });
    let manifest = AppManifest {
        manifest_version: 1,
        name: "norelay".into(),
        version: "0.1.0".into(),
        ..Default::default()
    };
    let port = available_port();
    let rt = Arc::new(Runtime::in_memory(manifest).unwrap());
    std::thread::spawn(move || {
        let fn_ops: std::sync::Arc<dyn pylon_router::FnOps> = std::sync::Arc::new(NoopFnOps);
        let _ = pylon_runtime::server::start_server_for_test_with_fn_ops(rt, port, fn_ops);
    });
    let host_port = format!("127.0.0.1:{port}");
    let mut ready = false;
    // Bound the wall clock, not the attempt count: a failed connect is
    // not instant on every platform (see csrf_form_route.rs).
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        if TcpStream::connect(&host_port).is_ok() {
            ready = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(ready, "test server never bound {host_port}");

    let request = format!(
        "GET /api/sync/relay-token HTTP/1.1\r\nHost: {host_port}\r\nConnection: close\r\n\r\n"
    );
    let mut stream = TcpStream::connect(&host_port).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    stream.write_all(request.as_bytes()).expect("write");
    let mut response = Vec::new();
    let _ = stream.read_to_end(&mut response);
    let text = String::from_utf8_lossy(&response);
    let status: u16 = text
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    assert_eq!(status, 404, "response: {text}");
}
