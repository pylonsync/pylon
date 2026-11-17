//! A cached page past its `revalidate` window is served at once and rendered
//! again in the background: the visitor who finds it expired does not wait
//! for the render, and the next visitor gets the new page. Same stub-FnOps
//! harness as page_host.rs.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU16, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use pylon_functions::protocol::AuthInfo;
use pylon_functions::registry::FnDef;
use pylon_kernel::{AppManifest, ManifestRoute};
use pylon_runtime::Runtime;

/// Seconds a render stays fresh.
const REVALIDATE_SECS: u64 = 2;
/// How long one render takes.
const RENDER_MS: u64 = 400;

static RENDERS: AtomicUsize = AtomicUsize::new(0);
/// Set when the page stops existing: renders answer 404 from then on.
static GONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

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
        _params: serde_json::Value,
        _search_params: serde_json::Value,
        _headers: HashMap<String, String>,
        cookies: HashMap<String, String>,
        _auth: AuthInfo,
        _session_present: bool,
        _initial_status: Option<u16>,
        _design: bool,
        on_response_start: Option<pylon_functions::runner::ResponseStartCallback>,
        mut on_chunk: pylon_functions::runner::ByteStreamCallback,
    ) -> Result<(), pylon_functions::runner::FnCallError> {
        assert!(
            cookies.is_empty(),
            "a render reached the cache path with cookies"
        );
        let n = RENDERS.fetch_add(1, Ordering::SeqCst) + 1;
        std::thread::sleep(Duration::from_millis(RENDER_MS));
        let gone = GONE.load(Ordering::SeqCst);
        if let Some(mut start) = on_response_start {
            let mut headers = HashMap::new();
            headers.insert(
                "content-type".to_string(),
                "text/html; charset=utf-8".to_string(),
            );
            if gone {
                start(404, headers);
            } else {
                headers.insert("x-pylon-cacheable".to_string(), REVALIDATE_SECS.to_string());
                start(200, headers);
            }
        }
        if gone {
            on_chunk(b"<!DOCTYPE html><html><body>no such page</body></html>");
        } else {
            on_chunk(format!("<!DOCTYPE html><html><body>render {n}</body></html>").as_bytes());
        }
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
        offline: None,
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
    static NEXT: AtomicU16 = AtomicU16::new(24_000);
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
            eprintln!("server did not start: {e}");
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

/// GET `/`; the response head (lowercased), the body, and how long it took.
fn get(port: u16, extra: &str) -> (String, String, Duration) {
    let started = std::time::Instant::now();
    let mut stream = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
    let request = format!(
        "GET / HTTP/1.1\r\nHost: localhost\r\nAccept: text/html\r\n{extra}Connection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).unwrap();
    let mut raw = Vec::new();
    let _ = stream.read_to_end(&mut raw);
    let took = started.elapsed();
    let text = String::from_utf8_lossy(&raw).to_string();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    (head.to_ascii_lowercase(), body.to_string(), took)
}

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

fn renders() -> usize {
    RENDERS.load(Ordering::SeqCst)
}

#[test]
fn an_expired_page_is_served_at_once_and_rendered_again_in_the_background() {
    let cache = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "pylon-ssr-swr-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    // SAFETY: set before the server thread starts; this binary has one test.
    unsafe {
        // The page cache is off in dev mode, so this server runs as in
        // production, which needs a CORS origin and refuses in-memory
        // sessions unless told.
        std::env::remove_var("PYLON_DEV_MODE");
        std::env::set_var("PYLON_SESSION_IN_MEMORY", "1");
        std::env::set_var("PYLON_CORS_ORIGIN", "http://localhost");
        std::env::set_var("PYLON_SSR_CACHE_ROOT", &cache);
    }
    let port = start_stub_server();
    let render = Duration::from_millis(RENDER_MS);
    // The boot warm-up renders the page once and stores nothing. Renders are
    // counted from after it.
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while get_status(port, "/health/ready", "localhost") != Some(200) {
        assert!(std::time::Instant::now() < deadline, "never ready");
        std::thread::sleep(Duration::from_millis(50));
    }
    // The stub renderer builds no client bundle, and a render is stored only
    // when one exists.
    pylon_runtime::frontend::set_client_bundle_outdir_for_test(cache.join("bundle"));
    let warm = renders();
    let first = format!("render {}", warm + 1);
    let second = format!("render {}", warm + 2);

    // The first visitor waits for the render.
    let (head, body, took) = get(port, "");
    assert!(head.starts_with("http/1.1 200"), "{head}");
    assert!(body.contains(&first), "{body}");
    assert!(took >= render, "{took:?}");
    // The cache write follows the response.
    std::thread::sleep(Duration::from_millis(300));

    // Inside the window the stored page answers, with no render.
    let (head, body, took) = get(port, "");
    assert!(body.contains(&first), "{body}");
    assert!(took < render, "a fresh hit took {took:?}");
    assert!(!head.contains("x-pylon-cache:"), "{head}");
    assert_eq!(renders(), warm + 1);

    // Past the window: several visitors at once all get the stored page
    // without the wait, and one render starts.
    std::thread::sleep(Duration::from_secs(REVALIDATE_SECS + 1));
    let visitors: Vec<_> = (0..4)
        .map(|_| std::thread::spawn(move || get(port, "")))
        .collect();
    for visitor in visitors {
        let (head, body, took) = visitor.join().unwrap();
        assert!(head.starts_with("http/1.1 200"), "{head}");
        assert!(body.contains(&first), "{body}");
        assert!(took < render, "an expired hit waited {took:?}");
        assert!(
            head.contains("x-pylon-cache: stale-while-revalidate"),
            "{head}"
        );
        assert!(
            head.contains("cache-control: public, max-age=10, stale-while-revalidate=30"),
            "{head}"
        );
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while renders() < warm + 2 {
        assert!(std::time::Instant::now() < deadline, "no background render");
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(render + Duration::from_millis(400));
    assert_eq!(renders(), warm + 2, "one render for four visitors");

    // The next visitor gets the new page from the cache.
    let (head, body, took) = get(port, "");
    assert!(body.contains(&second), "{body}");
    assert!(took < render, "{took:?}");
    assert!(!head.contains("x-pylon-cache:"), "{head}");
    assert_eq!(renders(), warm + 2);

    // The page stops existing. The visitor who finds the stored page expired
    // still gets it, and the background render, a 404, removes it: the next
    // visitor gets the 404, not the old page for the rest of the day.
    std::thread::sleep(Duration::from_secs(REVALIDATE_SECS + 1));
    GONE.store(true, Ordering::SeqCst);
    let (head, body, _) = get(port, "");
    assert!(head.starts_with("http/1.1 200"), "{head}");
    assert!(body.contains(&second), "{body}");
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while renders() < warm + 3 {
        assert!(std::time::Instant::now() < deadline, "no background render");
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(render + Duration::from_millis(400));
    let (head, body, _) = get(port, "");
    assert!(head.starts_with("http/1.1 404"), "{head}");
    assert!(body.contains("no such page"), "{body}");
}
