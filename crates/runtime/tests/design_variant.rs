//! Design render (`X-Pylon-Design: 1`) and render-by-component
//! (`/_pylon/dev/render`) end-to-end against the REAL request loop with a stub
//! `FnOps` (no Bun), same harness as fn_stream_resume.rs.
//!
//! The stub records every `render_route` call (component, layouts, url, auth,
//! design flag) and emits a fixed document, so the assertions here cover the
//! HOST's side of the contract: which requests become design renders, the
//! response headers, the dev-mode and token gates, the path checks, and the
//! viewer impersonation. What the Bun runtime does with `design: true`
//! (hydration, CSS, `<base>`) is covered by
//! packages/functions/src/ssr-design-render.test.ts.
//!
//! `is_dev_mode()` and the token gate read process env, so the phases that
//! flip them run in ONE test function, in order.

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

#[derive(Debug, Clone)]
struct RenderCall {
    component: String,
    layouts: Vec<String>,
    url: String,
    auth: AuthInfo,
    design: bool,
}

struct StubFnOps {
    calls: Arc<Mutex<Vec<RenderCall>>>,
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
        component: &str,
        layouts: Vec<String>,
        _route_path: &str,
        url: &str,
        _params: serde_json::Value,
        _search_params: serde_json::Value,
        _headers: HashMap<String, String>,
        _cookies: HashMap<String, String>,
        auth: AuthInfo,
        _session_present: bool,
        _initial_status: Option<u16>,
        design: bool,
        on_response_start: Option<pylon_functions::runner::ResponseStartCallback>,
        mut on_chunk: pylon_functions::runner::ByteStreamCallback,
    ) -> Result<(), pylon_functions::runner::FnCallError> {
        self.calls.lock().unwrap().push(RenderCall {
            component: component.to_string(),
            layouts,
            url: url.to_string(),
            auth,
            design,
        });
        if let Some(mut start) = on_response_start {
            let mut headers = HashMap::new();
            headers.insert(
                "content-type".to_string(),
                "text/html; charset=utf-8".to_string(),
            );
            start(200, headers);
        }
        // What the Bun runtime emits for `design: true` vs a normal render, in
        // miniature: the design document is static (stamped DOM, inline CSS);
        // the normal one carries the hydration tail.
        let body = if design {
            "<!DOCTYPE html><html><head><style data-pylon-css=\"app.css\">.a{}</style></head>\
             <body><div data-pylon-src=\"app/page.tsx:3:5\">hi</div></body></html>"
        } else {
            "<!DOCTYPE html><html><head><link rel=\"stylesheet\" href=\"/_pylon/build/app.css\"></head>\
             <body><div>hi</div><script id=\"__PYLON_DATA__\" type=\"application/json\">{}</script>\
             <script type=\"module\" src=\"/_pylon/build/entry.js\"></script></body></html>"
        };
        on_chunk(body.as_bytes());
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
        manifest_version: 1,
        name: "design-variant".into(),
        version: "0.1.0".into(),
        entities: vec![],
        routes: vec![ManifestRoute {
            path: "/".into(),
            mode: "ssr".into(),
            query: None,
            auth: None,
            component: Some("app/page".into()),
            layouts: vec!["app/layout".into()],
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
    static NEXT: AtomicU16 = AtomicU16::new(28_000);
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

/// A project dir with a routable page, a layout, and a non-routable variant
/// under `.design/`. `/_pylon/dev/render` resolves modules against it.
fn project_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pylon-design-variant-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(dir.join("app")).unwrap();
    std::fs::create_dir_all(dir.join(".design/variants")).unwrap();
    std::fs::write(dir.join("app/page.tsx"), "export default () => <div/>;\n").unwrap();
    std::fs::write(
        dir.join("app/layout.tsx"),
        "export default (p) => p.children;\n",
    )
    .unwrap();
    std::fs::write(
        dir.join(".design/variants/v1.tsx"),
        "export default () => <div/>;\n",
    )
    .unwrap();
    // Something OUTSIDE the project that a traversal would reach.
    std::fs::write(
        dir.join("../pylon-design-outside.tsx"),
        "export default 1;\n",
    )
    .unwrap();
    dir
}

fn start_stub_server(calls: Arc<Mutex<Vec<RenderCall>>>) -> u16 {
    let port = available_port();
    let rt = Arc::new(Runtime::in_memory(manifest()).unwrap());
    let fn_ops: Arc<dyn pylon_router::FnOps> = Arc::new(StubFnOps { calls });
    std::thread::spawn(move || {
        let _ = pylon_runtime::server::start_server_for_test_with_fn_ops(rt, port, fn_ops);
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        if TcpStream::connect(format!("127.0.0.1:{port}")).is_ok() {
            return port;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("test server never bound 127.0.0.1:{port}");
}

struct Resp {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Resp {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

fn get(port: u16, path: &str, extra_headers: &[(&str, &str)]) -> Resp {
    let host_port = format!("127.0.0.1:{port}");
    let mut request = format!("GET {path} HTTP/1.1\r\nHost: {host_port}\r\nAccept: text/html\r\n");
    for (k, v) in extra_headers {
        request.push_str(&format!("{k}: {v}\r\n"));
    }
    request.push_str("Connection: close\r\n\r\n");
    let mut stream = TcpStream::connect(&host_port).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
    stream.write_all(request.as_bytes()).expect("write");
    let mut raw = Vec::new();
    let _ = stream.read_to_end(&mut raw);
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = text
        .split_once("\r\n\r\n")
        .unwrap_or_else(|| panic!("no http head for {path}: {text:?}"));
    let mut lines = head.lines();
    let status_line = lines.next().unwrap();
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(": "))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    // Chunked transfer: strip the chunk framing by keeping only the markup.
    let body = if headers.iter().any(|(k, v): &(String, String)| {
        k.eq_ignore_ascii_case("transfer-encoding") && v == "chunked"
    }) {
        dechunk(body)
    } else {
        body.to_string()
    };
    Resp {
        status,
        headers,
        body,
    }
}

fn dechunk(body: &str) -> String {
    let mut out = String::new();
    let mut rest = body;
    loop {
        let Some((size_line, after)) = rest.split_once("\r\n") else {
            break;
        };
        let size = usize::from_str_radix(size_line.trim(), 16).unwrap_or(0);
        if size == 0 {
            break;
        }
        out.push_str(&after[..size.min(after.len())]);
        rest = after.get(size + 2..).unwrap_or("");
    }
    out
}

fn last_call(calls: &Arc<Mutex<Vec<RenderCall>>>) -> RenderCall {
    calls
        .lock()
        .unwrap()
        .last()
        .cloned()
        .expect("a render_route call")
}

#[test]
fn design_render_variant_and_render_by_component() {
    let project = project_dir();
    let calls: Arc<Mutex<Vec<RenderCall>>> = Arc::new(Mutex::new(Vec::new()));
    // The dev-render endpoint resolves modules against this dir.
    // Boot in dev mode like the other stub-server tests; `is_dev_mode()` is
    // read per request, so the first phase flips it off after boot.
    unsafe {
        std::env::set_var("PYLON_DEV_WATCH_DIR", &project);
        std::env::remove_var("PYLON_DEV_FILE_API_TOKEN");
        std::env::set_var("PYLON_DEV_MODE", "1");
    }
    let port = start_stub_server(calls.clone());
    unsafe {
        std::env::set_var("PYLON_DEV_MODE", "0");
    }

    // --- Outside dev mode: the design header is a 404; the endpoint is a 404;
    // a normal request is unaffected.
    let r = get(port, "/", &[("X-Pylon-Design", "1")]);
    assert_eq!(r.status, 404, "design header outside dev mode: {}", r.body);
    let r = get(port, "/_pylon/dev/render?component=app/page", &[]);
    assert_eq!(r.status, 404, "dev render outside dev mode");
    assert!(
        calls.lock().unwrap().is_empty(),
        "no render ran outside dev mode"
    );
    let r = get(port, "/", &[]);
    assert_eq!(r.status, 200);
    assert!(!last_call(&calls).design);
    assert!(r.body.contains("__PYLON_DATA__"));

    // --- Dev mode, no token configured.
    unsafe {
        std::env::set_var("PYLON_DEV_MODE", "1");
    }
    let r = get(port, "/", &[("X-Pylon-Design", "1")]);
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.header("Cache-Control"), Some("no-store"));
    assert!(r.header("Content-Type").unwrap().starts_with("text/html"));
    let call = last_call(&calls);
    assert!(call.design, "design flag reaches render_route");
    assert_eq!(call.component, "app/page");
    assert_eq!(call.url, "/");
    assert_eq!(call.auth.user_id, None);
    assert!(r.body.contains("data-pylon-src=\"app/page.tsx:3:5\""));
    assert!(r.body.contains("<style data-pylon-css="));
    assert!(!r.body.contains("__PYLON_DATA__"));
    assert!(!r.body.contains("<script type=\"module\""));
    assert!(!r.body.contains("/_pylon/dev/live"));

    // A plain dev request is still a normal render (dev-mode Cache-Control,
    // hydration tail).
    let r = get(port, "/", &[]);
    assert_eq!(r.status, 200);
    assert!(!last_call(&calls).design);
    assert!(r.body.contains("__PYLON_DATA__"));

    // Viewer impersonation is never open: no token configured → 401.
    let before = calls.lock().unwrap().len();
    let r = get(
        port,
        "/",
        &[("X-Pylon-Design", "1"), ("X-Pylon-Design-Viewer", "u_1")],
    );
    assert_eq!(r.status, 401, "viewer header without a configured token");
    assert_eq!(
        calls.lock().unwrap().len(),
        before,
        "no render on a rejected viewer"
    );

    // Render by component: a variant outside app/ renders as a design render.
    let r = get(
        port,
        "/_pylon/dev/render?component=.design/variants/v1&layouts=app/layout&path=/pricing",
        &[],
    );
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.header("Cache-Control"), Some("no-store"));
    let call = last_call(&calls);
    assert!(call.design);
    assert_eq!(call.component, ".design/variants/v1");
    assert_eq!(call.layouts, vec!["app/layout".to_string()]);
    assert_eq!(call.url, "/pricing");
    assert!(r.body.contains("data-pylon-src="));

    // `path` defaults to "/"; `layouts` may be empty.
    let r = get(port, "/_pylon/dev/render?component=app/page", &[]);
    assert_eq!(r.status, 200);
    let call = last_call(&calls);
    assert_eq!(call.url, "/");
    assert!(call.layouts.is_empty());

    // Path checks: traversal, absolute paths, and missing modules are 400 and
    // never reach the renderer.
    let before = calls.lock().unwrap().len();
    for bad in [
        "/_pylon/dev/render?component=../pylon-design-outside",
        "/_pylon/dev/render?component=app/../../pylon-design-outside",
        "/_pylon/dev/render?component=/etc/passwd",
        "/_pylon/dev/render?component=app/missing",
        "/_pylon/dev/render?component=app/page&layouts=../pylon-design-outside",
        "/_pylon/dev/render",
    ] {
        let r = get(port, bad, &[]);
        assert_eq!(r.status, 400, "{bad}: {}", r.body);
    }
    assert_eq!(calls.lock().unwrap().len(), before);

    // --- Token configured: the endpoint requires it; the viewer header works
    // with it and is rejected without it.
    unsafe {
        std::env::set_var("PYLON_DEV_FILE_API_TOKEN", "sekret");
    }
    let r = get(port, "/_pylon/dev/render?component=app/page", &[]);
    assert_eq!(r.status, 401, "endpoint without bearer");
    let r = get(
        port,
        "/_pylon/dev/render?component=app/page",
        &[("Authorization", "Bearer wrong")],
    );
    assert_eq!(r.status, 401, "endpoint with a wrong bearer");
    let r = get(
        port,
        "/_pylon/dev/render?component=app/page",
        &[("Authorization", "Bearer sekret")],
    );
    assert_eq!(r.status, 200, "endpoint with the bearer: {}", r.body);
    assert!(last_call(&calls).design);

    // The design header itself does not need the token…
    let r = get(port, "/", &[("X-Pylon-Design", "1")]);
    assert_eq!(r.status, 200);
    // …but the viewer header does.
    let r = get(
        port,
        "/",
        &[("X-Pylon-Design", "1"), ("X-Pylon-Design-Viewer", "u_1")],
    );
    assert_eq!(r.status, 401, "viewer header without bearer");
    let r = get(
        port,
        "/",
        &[
            ("X-Pylon-Design", "1"),
            ("X-Pylon-Design-Viewer", "u_1"),
            ("Authorization", "Bearer sekret"),
        ],
    );
    assert_eq!(r.status, 200, "{}", r.body);
    let call = last_call(&calls);
    assert!(call.design);
    assert_eq!(call.auth.user_id.as_deref(), Some("u_1"));
    assert!(!call.auth.is_admin);

    // `anon` renders anonymous even with the token.
    let r = get(
        port,
        "/_pylon/dev/render?component=app/page",
        &[
            ("X-Pylon-Design-Viewer", "anon"),
            ("Authorization", "Bearer sekret"),
        ],
    );
    assert_eq!(r.status, 200);
    assert_eq!(last_call(&calls).auth.user_id, None);

    // The viewer header is ignored on a normal (non-design) render.
    let r = get(
        port,
        "/",
        &[
            ("X-Pylon-Design-Viewer", "u_1"),
            ("Authorization", "Bearer sekret"),
        ],
    );
    assert_eq!(r.status, 200);
    let call = last_call(&calls);
    assert!(!call.design);
    assert_eq!(call.auth.user_id, None);

    unsafe {
        std::env::remove_var("PYLON_DEV_FILE_API_TOKEN");
    }
    let _ = std::fs::remove_file(project.join("../pylon-design-outside.tsx"));
    let _ = std::fs::remove_dir_all(&project);
}
