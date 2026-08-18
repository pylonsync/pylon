//! E2E: `GET /api/sync/relay-token` — the machine side of the Durable
//! Object sync relay handshake (docs/SYNC_DURABLE_OBJECTS_DESIGN.md).
//!
//! The route mints an HMAC-signed auth blob the relay verifies with
//! the shared secret. This drives the REAL request loop and then
//! verifies the minted blob with `pylon_auth::relay_blob::verify` —
//! the exact function the DO runs — so the machine→relay contract is
//! pinned end to end.
//!
//! The unconfigured-404 behavior lives in its own test binary
//! (`sync_relay_token_unconfigured.rs`): env vars are process-global.

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

const SECRET: &str = "relay-e2e-secret";

fn manifest() -> AppManifest {
    AppManifest {
        manifest_version: 1,
        name: "relaytoken".into(),
        version: "0.1.0".into(),
        ..Default::default()
    }
}

fn available_port() -> u16 {
    static NEXT: AtomicU16 = AtomicU16::new(26_500);
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

fn start_server() -> u16 {
    let port = available_port();
    static ENV: std::sync::Once = std::sync::Once::new();
    ENV.call_once(|| unsafe {
        std::env::set_var("PYLON_DEV_MODE", "1");
        // NOTE: the URL is never dialed by this test — the sink thread
        // will fail its manifest push against it, which is exactly the
        // degrade-to-catch-up behavior the design specifies.
        std::env::set_var("PYLON_SYNC_RELAY_URL", "https://relay.test.invalid");
        std::env::set_var("PYLON_SYNC_RELAY_SECRET", SECRET);
        std::env::set_var("PYLON_SYNC_RELAY_APP", "relaytoken");
    });
    let rt = Arc::new(Runtime::in_memory(manifest()).unwrap());
    std::thread::spawn(move || {
        let fn_ops: std::sync::Arc<dyn pylon_router::FnOps> = std::sync::Arc::new(NoopFnOps);
        let _ = pylon_runtime::server::start_server_for_test_with_fn_ops(rt, port, fn_ops);
    });
    for _ in 0..300 {
        if TcpStream::connect(format!("127.0.0.1:{port}")).is_ok() {
            return port;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("test server never bound 127.0.0.1:{port}");
}

fn get(port: u16, path: &str) -> (u16, String) {
    let host_port = format!("127.0.0.1:{port}");
    let request = format!("GET {path} HTTP/1.1\r\nHost: {host_port}\r\nConnection: close\r\n\r\n");
    let mut stream = TcpStream::connect(&host_port).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    stream.write_all(request.as_bytes()).expect("write");
    let mut response = Vec::new();
    let _ = stream.read_to_end(&mut response);
    let text = String::from_utf8_lossy(&response).to_string();
    let status: u16 = text
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body = text
        .split("\r\n\r\n")
        .nth(1)
        .unwrap_or_default()
        .to_string();
    (status, body)
}

#[test]
fn anonymous_token_mints_and_verifies_with_the_shared_secret() {
    let port = start_server();
    let (status, body) = get(port, "/api/sync/relay-token");
    assert_eq!(status, 200, "body: {body}");
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("json body");
    let token = parsed["token"].as_str().expect("token");
    let url = parsed["url"].as_str().expect("url");
    let exp = parsed["exp"].as_u64().expect("exp");

    // The advertised socket URL derives from the relay base:
    // https → wss, with the app id in the query. The blob is NOT in
    // the URL — the client sends it as a subprotocol.
    assert_eq!(url, "wss://relay.test.invalid/sync/ws?app=relaytoken");
    assert!(
        !url.contains("token="),
        "blob must not be baked into the URL"
    );

    // Verify with the DO's own verification path + shared secret + the
    // routed app.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let claims =
        pylon_auth::relay_blob::verify(SECRET, "relaytoken", token, now).expect("blob verifies");
    assert_eq!(claims.exp, exp);
    assert_eq!(claims.app, "relaytoken");
    assert!(exp > now, "token must not be born expired");
    // Anonymous caller → anonymous claims; no privilege appears from
    // nowhere.
    assert_eq!(claims.user_id, None);
    assert!(!claims.is_admin);
    assert!(claims.roles.is_empty());

    // The wrong secret must reject the same blob (what a relay with a
    // mismatched PYLON_RELAY_SECRET would do).
    assert!(pylon_auth::relay_blob::verify("other-secret", "relaytoken", token, now).is_err());
    // A DIFFERENT app's DO must reject this blob — the cross-app leak
    // class the review flagged.
    assert_eq!(
        pylon_auth::relay_blob::verify(SECRET, "other-app", token, now),
        Err(pylon_auth::relay_blob::RelayBlobError::WrongApp)
    );
}
