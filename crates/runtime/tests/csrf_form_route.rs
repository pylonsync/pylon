//! Regression: the CSRF Origin/Referer gate must clear BEFORE a `route.ts`
//! form/method handler (#276) runs.
//!
//! The bug this pins: `try_handle` dispatches a non-GET request matching a
//! `kind:"route"` SSR route to `serve_via_form_rpc` (which invokes the
//! handler and can WRITE to the DB). That dispatch used to run in the server
//! request loop BEFORE the CSRF check, so a cross-origin POST to a form route
//! bypassed the Origin gate entirely — classic CSRF on any state-changing
//! `route.ts`. The fix moves the CSRF check above `try_handle`.
//!
//! We drive the REAL request loop with a stub `FnOps` (no Bun) so a reordering
//! regression fails here: a disallowed Origin must yield 403 and never reach
//! `handle_form`; an allowed Origin must reach it.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::Arc;
use std::time::Duration;

use pylon_kernel::{AppManifest, ManifestRoute};
use pylon_runtime::Runtime;

/// A minimal `FnOps` whose `handle_form` just records that it was reached and
/// replies 200. If the CSRF gate is correctly ordered, a forged-origin POST
/// never gets here.
struct StubFnOps {
    form_called: Arc<AtomicBool>,
}

impl pylon_router::FnOps for StubFnOps {
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
        _on_stream: Option<Box<dyn FnMut(&str) + Send>>,
        _request: Option<pylon_functions::protocol::RequestInfo>,
    ) -> Result<
        (serde_json::Value, pylon_functions::trace::FnTrace),
        pylon_functions::runner::FnCallError,
    > {
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
        on_response_start: Option<pylon_functions::runner::ResponseStartCallback>,
        _on_chunk: pylon_functions::runner::ByteStreamCallback,
    ) -> Result<(), pylon_functions::runner::FnCallError> {
        // Reaching here means the handler ran — the exact thing CSRF must
        // prevent for a forged origin.
        self.form_called.store(true, Ordering::SeqCst);
        if let Some(mut cb) = on_response_start {
            cb(200, HashMap::new());
        }
        Ok(())
    }

    fn recent_traces(&self, _limit: usize) -> Vec<pylon_functions::trace::FnTrace> {
        vec![]
    }
}

fn form_route_manifest() -> AppManifest {
    AppManifest {
        manifest_version: 1,
        name: "csrf-form".into(),
        version: "0.1.0".into(),
        entities: vec![],
        // A `route.ts` POST/PUT/PATCH/DELETE handler at /contact. `mode:"ssr"`
        // + `kind:"route"` is what `match_form_route` dispatches on.
        routes: vec![ManifestRoute {
            path: "/contact".into(),
            mode: "ssr".into(),
            query: None,
            auth: None,
            component: Some("app/contact/route".into()),
            layouts: vec![],
            kind: Some("route".into()),
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
    static NEXT: AtomicU16 = AtomicU16::new(43_200);
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

fn start_stub_server(form_called: Arc<AtomicBool>) -> u16 {
    let port = available_port();
    // Dev mode so the in-memory server boots without prod-only config
    // (PYLON_CORS_ORIGIN, persistent session store, ...). PYLON_CSRF_ORIGINS
    // is honored REGARDLESS of dev mode, so set an explicit allowlist that
    // excludes the "attacker" origin — dev's default `*` would trust everyone.
    // SAFETY: this test binary is a dedicated process; env is isolated.
    // Once per binary — a per-call set_var races the other test threads
    // reading the environment. See the note in e2e_smoke.rs.
    static CSRF_ENV: std::sync::Once = std::sync::Once::new();
    CSRF_ENV.call_once(|| unsafe {
        std::env::set_var("PYLON_DEV_MODE", "1");
        std::env::set_var("PYLON_CSRF_ORIGINS", "http://good.example");
    });
    let rt = Arc::new(Runtime::in_memory(form_route_manifest()).unwrap());
    let fn_ops: Arc<dyn pylon_router::FnOps> = Arc::new(StubFnOps { form_called });
    // The server's own error is the only explanation for a bind
    // failure; dropping it leaves "never bound" with no cause.
    let boot_err: std::sync::Arc<std::sync::Mutex<Option<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let boot_err_thread = std::sync::Arc::clone(&boot_err);
    std::thread::spawn(move || {
        let r = pylon_runtime::server::start_server_for_test_with_fn_ops(rt, port, fn_ops);
        if let Err(e) = r {
            *boot_err_thread.lock().unwrap() = Some(e.to_string());
        }
    });
    // 300 x 50ms = 15s. The old budget was 5s AND fell through
    // silently when it ran out, so a slow CI runner walked into a
    // bare `.expect("connect")` panic further down that looked like a
    // product bug. Fail here instead, naming the port.
    {
        let mut ready = false;
        for _ in 0..300 {
            if TcpStream::connect(format!("127.0.0.1:{port}")).is_ok() {
                ready = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            ready,
            "test server never bound {} within 15s (server error: {:?})",
            format!("127.0.0.1:{port}"),
            boot_err.lock().unwrap()
        );
    }
    port
}

/// POST an urlencoded form to `path` with an explicit `Origin`, return the
/// HTTP status.
fn post_form_with_origin(port: u16, path: &str, origin: &str) -> u16 {
    let host_port = format!("127.0.0.1:{port}");
    let body = "name=alice";
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {host_port}\r\nOrigin: {origin}\r\n\
         Content-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
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
fn csrf_gate_precedes_route_form_dispatch() {
    let form_called = Arc::new(AtomicBool::new(false));
    let port = start_stub_server(Arc::clone(&form_called));

    // 1. Forged cross-origin POST → rejected by the CSRF gate with 403, and
    //    the form handler is NEVER reached. Before the fix this dispatched to
    //    `serve_via_form_rpc` and ran the handler (a state-changing write).
    let status = post_form_with_origin(port, "/contact", "http://evil.example");
    assert_eq!(
        status, 403,
        "forged-origin POST to a route.ts form handler must be CSRF-rejected (403), got {status}"
    );
    assert!(
        !form_called.load(Ordering::SeqCst),
        "the form handler must NOT run for a forged-origin request — CSRF bypass regression"
    );

    // 2. Allowed origin → clears CSRF, dispatches to the handler (200).
    let status = post_form_with_origin(port, "/contact", "http://good.example");
    assert_eq!(
        status, 200,
        "allowed-origin POST must clear CSRF and reach the handler (200), got {status}"
    );
    assert!(
        form_called.load(Ordering::SeqCst),
        "the form handler must run for an allowed-origin request"
    );
}
