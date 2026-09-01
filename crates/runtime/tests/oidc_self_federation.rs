//! One pylon server federating against ITSELF: the OIDC provider
//! (`PYLON_OIDC_ISSUER`) and a discovery-configured OIDC client
//! (`PYLON_OAUTH_SELFIDP_OIDC_ISSUER`) in the same process.
//!
//! This is the loop Stack0 runs in production — Cloud as the IdP,
//! Analytics/Mast as Pylon-app clients — collapsed to one server so
//! it can run in CI. It exists because the two halves were shipped
//! and tested separately, and separately they were incompatible:
//! the IdP rejects every authorize request without PKCE S256, while
//! the discovery-client path never minted PKCE (`requires_pkce()`
//! returned false for `ResolvedSpec::Oidc`). Either half's own tests
//! pass; the handshake between them is what breaks. So this test
//! drives the handshake:
//!
//!   1. `/api/auth/login/selfidp` → 302 to `/oidc/authorize` — and the
//!      URL MUST carry `code_challenge` + `code_challenge_method=S256`
//!      (the regression assertion).
//!   2. `/oidc/authorize` with an authenticated IdP session → 302 back
//!      to the registered redirect_uri with `code` + `state`.
//!   3. `/api/auth/callback/selfidp?code=...&state=...` → the client
//!      exchanges the code at `/oidc/token` (client_secret_post +
//!      code_verifier), pulls `/oidc/userinfo`, links the user by
//!      email, and establishes an app session.
//!   4. `/api/auth/me` with that session resolves to the same user.
//!
//! Self-calls are safe: the HTTP server is thread-per-connection, so
//! the token exchange the callback performs against its own origin
//! runs on a second thread while the callback request is in flight.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use pylon_kernel::{AppManifest, ManifestEntity, ManifestField};
use pylon_runtime::Runtime;

fn string_field(name: &str, optional: bool, unique: bool) -> ManifestField {
    ManifestField {
        name: name.into(),
        field_type: "string".into(),
        optional,
        unique,
        crdt: None,
        server_only: false,
        readonly: false,
        default: None,
        enum_values: None,
        encrypted: false,
        sync_omit: false,
    }
}

fn test_manifest() -> AppManifest {
    AppManifest {
        manifest_version: 1,
        name: "oidc-self-federation".into(),
        version: "0.1.0".into(),
        entities: vec![ManifestEntity {
            name: "User".into(),
            fields: vec![
                string_field("email", false, true),
                string_field("displayName", true, false),
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
    // Own lane (25_000+) so parallel test binaries can't collide — see
    // oidc_provider.rs for the rationale.
    for base in (25_000..26_000).step_by(4) {
        if std::net::TcpListener::bind(format!("127.0.0.1:{base}")).is_ok() {
            return base;
        }
    }
    panic!("no free port in the 25000-26000 lane");
}

/// Minimal raw-TCP request helper. Returns (status, set_cookies, headers-as-
/// lowercased-pairs, body). Raw TCP rather than a client lib so redirects are
/// NEVER followed — every 302 in this flow is an assertion target.
fn http_request(
    method: &str,
    url: &str,
    body: Option<&str>,
    extra_headers: &[(&str, &str)],
) -> (u16, Vec<String>, Vec<(String, String)>, String) {
    let host = url.strip_prefix("http://").unwrap_or(url);
    let (host_port, path) = match host.find('/') {
        Some(i) => (&host[..i], &host[i..]),
        None => (host, "/"),
    };
    let body_str = body.unwrap_or("");
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host_port}\r\nOrigin: http://{host_port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body_str.len()
    );
    for (k, v) in extra_headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(body_str);

    let mut stream = TcpStream::connect(host_port).expect("connect");
    // Generous: the callback request performs a discovery fetch + token
    // exchange + userinfo against this same server before answering.
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
    stream.write_all(req.as_bytes()).expect("write");
    let mut response = Vec::new();
    let _ = stream.read_to_end(&mut response);
    let text = String::from_utf8_lossy(&response).to_string();

    let status: u16 = text
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let mut set_cookies = Vec::new();
    let mut headers = Vec::new();
    for line in text.lines().skip(1) {
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().to_ascii_lowercase();
            let val = v.trim().to_string();
            if key == "set-cookie" {
                set_cookies.push(val.clone());
            }
            headers.push((key, val));
        }
    }
    let body_out = match text.find("\r\n\r\n") {
        Some(i) => text[i + 4..].to_string(),
        None => String::new(),
    };
    (status, set_cookies, headers, body_out)
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

/// `name=value` pairs from Set-Cookie lines, joined for a Cookie header.
fn cookie_jar(set_cookies: &[String]) -> String {
    set_cookies
        .iter()
        .filter_map(|c| c.split(';').next())
        .collect::<Vec<_>>()
        .join("; ")
}

fn query_param(url: &str, name: &str) -> Option<String> {
    let q = url.split_once('?')?.1;
    q.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == name).then(|| v.to_string())
    })
}

#[test]
fn pylon_client_signs_in_against_pylon_idp() {
    let port = available_port();
    let origin = format!("http://127.0.0.1:{port}");
    let redirect_uri = format!("{origin}/api/auth/callback/selfidp");
    let key_path = std::env::temp_dir().join(format!("pylon-oidc-selffed-{port}.pem"));

    // Env BEFORE boot: the OIDC keystore, the client registry, and the OAuth
    // provider registry are all process-wide lazy singletons that read env on
    // first use. This test binary holds exactly one test, so no set_var race.
    unsafe {
        std::env::set_var("PYLON_DEV_MODE", "1");
        std::env::set_var("PYLON_ADMIN_TOKEN", "selffed-admin-token");
        // IdP half.
        std::env::set_var("PYLON_OIDC_ISSUER", &origin);
        std::env::set_var(
            "PYLON_OIDC_CLIENTS",
            format!(
                r#"[{{"client_id":"self-app","client_secret":"s3cr3t-selffed","redirect_uris":["{redirect_uri}"]}}]"#
            ),
        );
        std::env::set_var("PYLON_OIDC_KEY_PATH", &key_path);
        std::env::set_var("PYLON_LOGIN_URL", "/login");
        // Client half — generic OIDC via discovery, pointing at ourselves.
        std::env::set_var("PYLON_OAUTH_SELFIDP_OIDC_ISSUER", &origin);
        std::env::set_var("PYLON_OAUTH_SELFIDP_CLIENT_ID", "self-app");
        std::env::set_var("PYLON_OAUTH_SELFIDP_CLIENT_SECRET", "s3cr3t-selffed");
        std::env::set_var("PYLON_OAUTH_SELFIDP_REDIRECT", &redirect_uri);
    }
    // Pre-generate the signing key — keygen inside a request is long-tailed.
    pylon_auth::oidc_provider::OidcKeyStore::load_or_generate(&key_path)
        .expect("pre-generate OIDC signing key");

    let rt = Arc::new(Runtime::in_memory(test_manifest()).unwrap());
    let rt2 = Arc::clone(&rt);
    std::thread::spawn(move || {
        let _ = pylon_runtime::server::start(rt2, port);
    });
    {
        let mut ready = false;
        for _ in 0..300 {
            if TcpStream::connect(format!("127.0.0.1:{port}")).is_ok() {
                ready = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(ready, "server never bound 127.0.0.1:{port}");
    }

    // ── The user who will sign in, and their IdP-side session. ──────────
    let (status, _, _, body) = http_request(
        "POST",
        &format!("{origin}/api/entities/User"),
        Some(r#"{"email":"fed@example.com","displayName":"Fed E. Rated"}"#),
        &[("Authorization", "Bearer selffed-admin-token")],
    );
    assert_eq!(status, 201, "create User failed: {body}");
    let user: serde_json::Value = serde_json::from_str(&body).expect("user json");
    let user_id = user
        .get("id")
        .or_else(|| user.get("data").and_then(|d| d.get("id")))
        .and_then(|v| v.as_str())
        .expect("user id in create response")
        .to_string();

    let (status, _, _, body) = http_request(
        "POST",
        &format!("{origin}/api/auth/session"),
        Some(&format!(r#"{{"user_id":"{user_id}"}}"#)),
        &[],
    );
    assert_eq!(status, 201, "dev session mint failed: {body}");
    let session: serde_json::Value = serde_json::from_str(&body).expect("session json");
    let idp_token = session["token"].as_str().expect("session token").to_string();
    let idp_auth = [
        ("Authorization", format!("Bearer {idp_token}")),
        ("Cookie", format!("pylon_session={idp_token}")),
    ];
    let idp_auth_ref: Vec<(&str, &str)> =
        idp_auth.iter().map(|(k, v)| (*k, v.as_str())).collect();

    // ── 1. Client login kickoff — the PKCE regression assertion. ────────
    // callback/error_callback are where the app sends the browser AFTER the
    // whole dance; loopback origins are always trusted so no env needed.
    let (status, login_cookies, headers, body) = http_request(
        "GET",
        &format!("{origin}/api/auth/login/selfidp?callback={origin}/&error_callback={origin}/login"),
        None,
        &[],
    );
    // The kickoff answers 200 {"redirect","state"} — the app's own login page
    // performs the hop, so the URL comes back as JSON rather than a Location.
    assert_eq!(status, 200, "login kickoff failed: {body}");
    let _ = &headers;
    let kickoff: serde_json::Value = serde_json::from_str(&body).expect("kickoff json");
    let authorize_url = kickoff["redirect"]
        .as_str()
        .expect("kickoff redirect URL")
        .to_string();
    assert!(
        authorize_url.contains("/oidc/authorize"),
        "login should aim at the IdP authorize endpoint, got {authorize_url}"
    );
    assert!(
        authorize_url.contains("code_challenge=")
            && authorize_url.contains("code_challenge_method=S256"),
        "discovery-configured client MUST volunteer PKCE S256 — pylon's own \
         IdP rejects authorize without it. URL: {authorize_url}"
    );
    let state = query_param(&authorize_url, "state").expect("state in authorize URL");

    // ── 2. Authorize with the IdP session → code lands on redirect_uri. ─
    let (status, _, headers, body) =
        http_request("GET", &authorize_url, None, &idp_auth_ref);
    assert_eq!(status, 302, "authorize should 302 back to the client: {body}");
    let callback_url = header(&headers, "location")
        .expect("authorize 302 without Location")
        .to_string();
    assert!(
        callback_url.starts_with(&redirect_uri),
        "authorize must redirect to the registered redirect_uri, got {callback_url}"
    );
    let code = query_param(&callback_url, "code").expect("code in callback URL");
    assert!(!code.is_empty());
    assert_eq!(
        query_param(&callback_url, "state").as_deref(),
        Some(state.as_str()),
        "state must round-trip"
    );

    // ── 3. Callback: token exchange + userinfo + app session. ───────────
    // Replay the login kickoff's cookies (OAuth state may be cookie-bound);
    // NOT the IdP session — the client leg must stand on its own.
    let login_jar = cookie_jar(&login_cookies);
    let cb_headers: Vec<(&str, &str)> = if login_jar.is_empty() {
        vec![]
    } else {
        vec![("Cookie", login_jar.as_str())]
    };
    let (status, cb_cookies, headers, body) =
        http_request("GET", &callback_url, None, &cb_headers);
    assert!(
        status == 302 || status == 200,
        "callback should complete the login, got {status}: {body} \
         (Location: {:?})",
        header(&headers, "location")
    );
    let app_jar = cookie_jar(&cb_cookies);
    assert!(
        !app_jar.is_empty(),
        "callback must establish an app session cookie"
    );

    // ── 4. The app session resolves to the same identity. ───────────────
    let (status, _, _, body) = http_request(
        "GET",
        &format!("{origin}/api/auth/me"),
        None,
        &[("Cookie", app_jar.as_str())],
    );
    assert_eq!(status, 200, "/api/auth/me with the new session: {body}");
    let me: serde_json::Value = serde_json::from_str(&body).expect("me json");
    let me_user = me["user_id"].as_str().unwrap_or_default();
    assert!(
        !me_user.is_empty(),
        "session must resolve to a user: {body}"
    );
    assert_eq!(
        me_user, user_id,
        "OIDC round-trip must land on the SAME user (linked by email), \
         not mint a duplicate"
    );
}
