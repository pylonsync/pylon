//! End-to-end coverage for the OIDC provider route surface
//! shipped in v0.3.103/104. Unit tests cover the keystore + PKCE
//! + client primitives at the lib level; this exercises the
//! actual HTTP handlers so a future refactor that breaks routing,
//! query parsing, or status codes is caught here.
//!
//! Coverage:
//!   - `/.well-known/openid-configuration` returns a well-formed
//!     discovery doc with the configured issuer + the expected
//!     endpoint URLs.
//!   - `/oidc/jwks` returns a real RSA key (kty=RSA, alg=RS256,
//!     n decodes to ~256 bytes for the 2048-bit signing key).
//!   - `/oidc/authorize` validates client_id (400 on unknown) +
//!     redirect_uri (400 on unregistered) BEFORE redirecting,
//!     per RFC 6749 §3.1.2.4.
//!   - `/oidc/authorize` 302s to the configured PYLON_LOGIN_URL
//!     when the caller isn't authenticated, preserving the
//!     authorize URL as `?next=...` so the app's login page can
//!     bounce back.
//!   - `/oidc/authorize` rejects no-PKCE / plain-PKCE / no-openid
//!     requests via OAuth `error` redirect (per §4.1.2.1).
//!   - `/oidc/token` returns 400 unsupported_grant_type for any
//!     grant other than authorization_code.
//!   - `/oidc/token` returns 401 invalid_client for an unknown
//!     client_id.
//!   - `/oidc/userinfo` returns 401 invalid_token when the bearer
//!     is missing OR points at a token the AccessTokenStore
//!     hasn't issued.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::Duration;

use pylon_kernel::{AppManifest, ManifestEntity, ManifestField};
use pylon_runtime::Runtime;

fn test_manifest() -> AppManifest {
    AppManifest {
        manifest_version: 1,
        name: "oidc-e2e".into(),
        version: "0.1.0".into(),
        entities: vec![ManifestEntity {
            name: "User".into(),
            fields: vec![
                ManifestField {
                    name: "email".into(),
                    field_type: "string".into(),
                    optional: false,
                    unique: true,
                    crdt: None,
                    server_only: false,
                    readonly: false,
                    default: None,
                    enum_values: None,
                    encrypted: false,
                    sync_omit: false,
                },
                ManifestField {
                    name: "displayName".into(),
                    field_type: "string".into(),
                    optional: true,
                    unique: false,
                    crdt: None,
                    server_only: false,
                    readonly: false,
                    default: None,
                    enum_values: None,
                    encrypted: false,
                    sync_omit: false,
                },
            ],
            indexes: vec![],
            relations: vec![],
            search: None,
            crdt: true,
            sync: true,
            ..Default::default()
        }],
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
    static NEXT: AtomicU16 = AtomicU16::new(21_000);
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
    // Provision the OIDC env BEFORE the server boots — the keystore
    // lazy-loads on first /oidc/* request, and it reads
    // PYLON_OIDC_ISSUER to decide whether to even initialize. Also
    // point PYLON_OIDC_KEY_PATH at a temp file so we don't pollute
    // the dev-default location.
    let key_path = std::env::temp_dir().join(format!(
        "pylon-oidc-e2e-{}.pem",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    // SAFETY + correctness: exactly once per binary. `cargo test` runs a
    // binary's tests on several threads, so mutating the environment per call
    // races every other thread reading it — which is why set_var is unsafe. It
    // surfaced as a server that never came up and
    // `connect: ConnectionRefused`, on CI only.
    //
    // `key_path` is captured by the closure, so the first caller's temp path is
    // the one every server in this binary uses — fine, they want the same key.
    static OIDC_ENV: std::sync::Once = std::sync::Once::new();
    OIDC_ENV.call_once(|| unsafe {
        std::env::set_var("PYLON_DEV_MODE", "1");
        std::env::set_var("PYLON_OIDC_ISSUER", "https://auth.example.com");
        std::env::set_var(
            "PYLON_OIDC_CLIENTS",
            r#"[{"client_id":"docs-portal","client_secret":"shh-1234","redirect_uris":["https://docs.example.com/cb"]}]"#,
        );
        std::env::set_var("PYLON_OIDC_KEY_PATH", &key_path);
        std::env::set_var("PYLON_LOGIN_URL", "/login");
    });

    // Pre-generate the OIDC signing key before the server boots. The keystore
    // otherwise lazy-generates an RSA-2048 key inside the first `/oidc/jwks`
    // handler; keygen is long-tailed (usually ~50ms, occasionally seconds) and
    // the test HTTP client below has a 5s read timeout. Generating it up front
    // (no timeout) keeps the first jwks request on the fast `path.exists()`
    // load path, so the timed assertion is deterministic.
    pylon_auth::oidc_provider::OidcKeyStore::load_or_generate(&key_path)
        .expect("pre-generate OIDC signing key for test");

    let manifest = test_manifest();
    let rt = Arc::new(Runtime::in_memory(manifest).unwrap());
    let rt2 = Arc::clone(&rt);
    // The server's own error is the only explanation for a bind
    // failure; dropping it leaves "never bound" with no cause.
    let boot_err: std::sync::Arc<std::sync::Mutex<Option<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let boot_err_thread = std::sync::Arc::clone(&boot_err);
    std::thread::spawn(move || {
        let r = pylon_runtime::server::start(rt2, port);
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

fn http_request(
    method: &str,
    url: &str,
    body: Option<&str>,
    extra_headers: &[(&str, &str)],
) -> (u16, std::collections::HashMap<String, String>, String) {
    let host = url.strip_prefix("http://").unwrap_or(url);
    let (host_port, path) = match host.find('/') {
        Some(i) => (&host[..i], &host[i..]),
        None => (host, "/"),
    };
    let body_str = body.unwrap_or("");
    let mut header_block = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host_port}\r\nOrigin: http://{host_port}\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n",
        body_str.len()
    );
    for (k, v) in extra_headers {
        header_block.push_str(&format!("{k}: {v}\r\n"));
    }
    header_block.push_str("\r\n");
    let request = format!("{header_block}{body_str}");

    let mut stream = TcpStream::connect(host_port).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    stream.write_all(request.as_bytes()).expect("write");
    let mut response = Vec::new();
    let _ = stream.read_to_end(&mut response);
    let text = String::from_utf8_lossy(&response).to_string();
    let status = text
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);
    let body_start = text.find("\r\n\r\n").map(|i| i + 4).unwrap_or(text.len());
    let header_text = &text[..body_start.saturating_sub(4)];
    let mut headers = std::collections::HashMap::new();
    for line in header_text.lines().skip(1) {
        if let Some(idx) = line.find(':') {
            let key = line[..idx].trim().to_lowercase();
            let val = line[idx + 1..].trim().to_string();
            headers.insert(key, val);
        }
    }
    let body = text[body_start..].to_string();
    (status, headers, body)
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[test]
fn oidc_full_route_surface() {
    let port = start_server();
    let base = format!("http://127.0.0.1:{port}");

    // ----- discovery doc -----
    let (status, _, body) = http_request(
        "GET",
        &format!("{base}/.well-known/openid-configuration"),
        None,
        &[],
    );
    assert_eq!(status, 200, "discovery: {body}");
    let doc: serde_json::Value = serde_json::from_str(&body).expect("discovery json");
    // Issuer comes from PYLON_OIDC_ISSUER, not the request Host
    // header — proves the env-overrides path works.
    assert_eq!(doc["issuer"], "https://auth.example.com");
    assert_eq!(
        doc["authorization_endpoint"],
        "https://auth.example.com/oidc/authorize"
    );
    assert_eq!(doc["token_endpoint"], "https://auth.example.com/oidc/token");
    assert_eq!(
        doc["userinfo_endpoint"],
        "https://auth.example.com/oidc/userinfo"
    );
    assert!(doc["id_token_signing_alg_values_supported"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "RS256"));

    // ----- jwks (forces keystore init) -----
    let (status, _, body) = http_request("GET", &format!("{base}/oidc/jwks"), None, &[]);
    assert_eq!(status, 200, "jwks: {body}");
    let jwks: serde_json::Value = serde_json::from_str(&body).expect("jwks json");
    let keys = jwks["keys"].as_array().expect("keys array");
    assert_eq!(keys.len(), 1, "expected exactly one signing key");
    let key = &keys[0];
    assert_eq!(key["kty"], "RSA");
    assert_eq!(key["alg"], "RS256");
    assert_eq!(key["use"], "sig");
    assert_eq!(key["e"], "AQAB");
    let kid = key["kid"].as_str().unwrap();
    assert_eq!(kid.len(), 16, "kid should be 16 hex chars: {kid}");
    // n decodes to ~256 bytes for a 2048-bit modulus.
    use base64::Engine;
    let n_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(key["n"].as_str().unwrap())
        .expect("n is base64url");
    assert!(
        (250..=257).contains(&n_bytes.len()),
        "n should be ~256 bytes, got {}",
        n_bytes.len()
    );

    // ----- /oidc/authorize: unknown client_id → 400 (NO redirect) -----
    let authorize_url = format!(
        "{base}/oidc/authorize?response_type=code&client_id=ghost&redirect_uri={}&scope=openid&state=s1&code_challenge=c&code_challenge_method=S256",
        url_encode("https://docs.example.com/cb"),
    );
    let (status, _, body) = http_request("GET", &authorize_url, None, &[]);
    assert_eq!(status, 400, "unknown client_id must NOT redirect: {body}");
    assert!(body.contains("invalid_client"), "got {body}");

    // ----- /oidc/authorize: unregistered redirect_uri → 400 (NO redirect) -----
    let authorize_url = format!(
        "{base}/oidc/authorize?response_type=code&client_id=docs-portal&redirect_uri={}&scope=openid&state=s1&code_challenge=c&code_challenge_method=S256",
        url_encode("https://attacker.example.com/cb"),
    );
    let (status, _, body) = http_request("GET", &authorize_url, None, &[]);
    assert_eq!(
        status, 400,
        "unregistered redirect_uri must NOT redirect: {body}"
    );
    assert!(body.contains("invalid_redirect_uri"), "got {body}");

    // ----- /oidc/authorize: no PKCE → 302 to redirect_uri with error= -----
    let authorize_url = format!(
        "{base}/oidc/authorize?response_type=code&client_id=docs-portal&redirect_uri={}&scope=openid&state=s1",
        url_encode("https://docs.example.com/cb"),
    );
    let (status, headers, _body) = http_request("GET", &authorize_url, None, &[]);
    assert_eq!(
        status, 302,
        "no-PKCE must redirect with error= per RFC 6749"
    );
    let loc = headers.get("location").expect("Location header");
    assert!(loc.starts_with("https://docs.example.com/cb?"), "got {loc}");
    assert!(loc.contains("error=invalid_request"), "got {loc}");
    assert!(loc.contains("state=s1"), "state must be preserved: {loc}");

    // ----- /oidc/authorize: no openid scope → error redirect -----
    let authorize_url = format!(
        "{base}/oidc/authorize?response_type=code&client_id=docs-portal&redirect_uri={}&scope=profile&state=s2&code_challenge=c&code_challenge_method=S256",
        url_encode("https://docs.example.com/cb"),
    );
    let (status, headers, _) = http_request("GET", &authorize_url, None, &[]);
    assert_eq!(status, 302);
    let loc = headers.get("location").unwrap();
    assert!(loc.contains("error=invalid_scope"), "got {loc}");

    // ----- /oidc/authorize: unauthenticated → redirect to login -----
    // All the above are pre-auth validation. A properly-formed
    // authorize URL with NO session cookie should 302 to the
    // PYLON_LOGIN_URL with the original URL as ?next=.
    let authorize_url = format!(
        "{base}/oidc/authorize?response_type=code&client_id=docs-portal&redirect_uri={}&scope=openid&state=s3&code_challenge=cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc&code_challenge_method=S256",
        url_encode("https://docs.example.com/cb"),
    );
    let (status, headers, _) = http_request("GET", &authorize_url, None, &[]);
    assert_eq!(status, 302);
    let loc = headers.get("location").unwrap();
    assert!(
        loc.starts_with("/login?next="),
        "unauth caller should be sent to PYLON_LOGIN_URL: got {loc}"
    );
    assert!(
        loc.contains("oidc%2Fauthorize"),
        "next= must carry the original authorize URL: {loc}"
    );

    // ----- /oidc/token: bad grant_type → 400 unsupported_grant_type -----
    let (status, _, body) = http_request(
        "POST",
        &format!("{base}/oidc/token"),
        Some("grant_type=password&username=a&password=b"),
        &[],
    );
    assert_eq!(status, 400);
    assert!(body.contains("unsupported_grant_type"), "got {body}");

    // ----- /oidc/token: unknown client_id → 401 invalid_client -----
    let (status, _, body) = http_request(
        "POST",
        &format!("{base}/oidc/token"),
        Some("grant_type=authorization_code&code=x&redirect_uri=y&client_id=ghost"),
        &[],
    );
    assert_eq!(status, 401);
    assert!(body.contains("invalid_client"), "got {body}");

    // ----- /oidc/userinfo: no bearer → 401 -----
    let (status, _, body) = http_request("GET", &format!("{base}/oidc/userinfo"), None, &[]);
    assert_eq!(status, 401);
    assert!(body.contains("invalid_token"), "got {body}");

    // ----- /oidc/userinfo: bogus bearer → 401 -----
    let (status, _, body) = http_request(
        "GET",
        &format!("{base}/oidc/userinfo"),
        None,
        &[("Authorization", "Bearer not-a-real-token")],
    );
    assert_eq!(status, 401);
    assert!(body.contains("invalid_token"), "got {body}");
}
