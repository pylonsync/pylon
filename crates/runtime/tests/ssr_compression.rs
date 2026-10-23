//! Streamed SSR HTML is compressed when the browser accepts it, and still
//! streams: the shell arrives (and decodes) before a slow Suspense boundary
//! resolves. Same stub-FnOps harness as page_host.rs.
//!
//! BREACH: a request with credentials is compressed only when its URL has
//! no attacker-chosen input (no query string, no dynamic segment).

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
        if let Some(mut start) = on_response_start {
            let mut headers = HashMap::new();
            headers.insert(
                "content-type".to_string(),
                "text/html; charset=utf-8".to_string(),
            );
            start(200, headers);
        }
        // The shell, then a Suspense boundary that resolves later.
        on_chunk(SHELL.as_bytes());
        std::thread::sleep(Duration::from_millis(BOUNDARY_DELAY_MS));
        on_chunk(BOUNDARY.as_bytes());
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

const SHELL: &str = "<!DOCTYPE html><html><body><main>shell of the page</main>";
const BOUNDARY: &str = "<template>the resolved boundary</template></body></html>";
const BOUNDARY_DELAY_MS: u64 = 600;
const GZIP: &str = "Accept-Encoding: gzip\r\n";
const AUTH: &str = "Authorization: Bearer some-token\r\n";

/// The response head (lowercased), and the de-chunked body bytes that had
/// arrived when the head arrived plus `wait`; then the rest.
struct Streamed {
    head: String,
    early: Vec<u8>,
    early_at: Duration,
    all: Vec<u8>,
}

fn dechunk(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut rest = raw;
    loop {
        let Some(nl) = rest.windows(2).position(|w| w == b"\r\n") else {
            break;
        };
        let size = usize::from_str_radix(std::str::from_utf8(&rest[..nl]).unwrap().trim(), 16)
            .unwrap_or(0);
        if size == 0 || rest.len() < nl + 2 + size {
            break;
        }
        out.extend_from_slice(&rest[nl + 2..nl + 2 + size]);
        rest = &rest[(nl + 4 + size).min(rest.len())..];
    }
    out
}

fn get_streamed(port: u16, path: &str, extra: &str) -> Streamed {
    let mut stream = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nAccept: text/html\r\n{extra}Connection: close\r\n\r\n"
    );
    let started = std::time::Instant::now();
    stream.write_all(request.as_bytes()).unwrap();
    // Read what arrives before the boundary is due.
    stream
        .set_read_timeout(Some(Duration::from_millis(50)))
        .unwrap();
    let mut raw = Vec::new();
    let mut buf = [0u8; 16 * 1024];
    while started.elapsed() < Duration::from_millis(BOUNDARY_DELAY_MS / 2) {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => raw.extend_from_slice(&buf[..n]),
            Err(_) => {}
        }
    }
    let early_at = started.elapsed();
    let early_raw = raw.clone();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let _ = stream.read_to_end(&mut raw);
    let head_end = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("a response head");
    let head = String::from_utf8_lossy(&raw[..head_end]).to_ascii_lowercase();
    let early = if early_raw.len() > head_end + 4 {
        dechunk(&early_raw[head_end + 4..])
    } else {
        Vec::new()
    };
    Streamed {
        head,
        early,
        early_at,
        all: dechunk(&raw[head_end + 4..]),
    }
}

fn gunzip_partial(bytes: &[u8]) -> String {
    let mut d = flate2::write::GzDecoder::new(Vec::new());
    d.write_all(bytes).unwrap();
    d.flush().unwrap();
    String::from_utf8_lossy(d.get_ref()).to_string()
}

#[test]
fn streamed_html_is_compressed_and_still_streams() {
    unsafe {
        std::env::set_var("PYLON_DEV_MODE", "1");
    }
    let port = start_stub_server();

    // No Accept-Encoding: plain, and the shell arrives before the boundary.
    let plain = get_streamed(port, "/", "");
    assert!(!plain.head.contains("content-encoding"), "{}", plain.head);
    let plain_early = String::from_utf8_lossy(&plain.early).to_string();
    assert!(
        plain_early.contains("shell of the page"),
        "early: {plain_early:?}"
    );
    assert!(
        !plain_early.contains("resolved boundary"),
        "early: {plain_early:?}"
    );
    // Anonymous: gzip, chunked, and the shell decodes before the boundary.
    let r = get_streamed(port, "/", GZIP);
    assert!(r.head.contains("content-encoding: gzip"), "{}", r.head);
    assert!(r.head.contains("transfer-encoding: chunked"), "{}", r.head);
    assert!(r.head.contains("accept-encoding"), "Vary: {}", r.head);
    assert!(r.early_at < Duration::from_millis(BOUNDARY_DELAY_MS));
    let early = gunzip_partial(&r.early);
    assert!(early.contains("shell of the page"), "early: {early:?}");
    assert!(!early.contains("resolved boundary"), "early: {early:?}");
    let mut whole = String::new();
    flate2::read::GzDecoder::new(&r.all[..])
        .read_to_string(&mut whole)
        .unwrap();
    assert!(
        whole.contains("shell of the page") && whole.contains("the resolved boundary"),
        "{whole}"
    );

    // Credentials and a static URL: compressed.
    let r = get_streamed(port, "/", &format!("{GZIP}{AUTH}"));
    assert!(r.head.contains("content-encoding: gzip"), "{}", r.head);

    // Credentials plus attacker-chosen input in the URL: not compressed.
    let r = get_streamed(port, "/?q=guess", &format!("{GZIP}{AUTH}"));
    assert!(!r.head.contains("content-encoding"), "{}", r.head);
    assert!(String::from_utf8_lossy(&r.all).contains("the resolved boundary"));
    let r = get_streamed(port, "/item/abc", &format!("{GZIP}{AUTH}"));
    assert!(!r.head.contains("content-encoding"), "{}", r.head);

    // The same URL without credentials has no secret to leak: compressed.
    let r = get_streamed(port, "/item/abc", GZIP);
    assert!(r.head.contains("content-encoding: gzip"), "{}", r.head);
}
