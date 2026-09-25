//! OAuth sign-in on a platform (tenant) domain, end to end.
//!
//! An app serving a customer on `tenant.test` (a ready platform domain) has
//! one OAuth callback, on its own host. This drives the whole loop on one
//! server that is both the app and its OIDC provider (same harness as
//! oidc_self_federation.rs), with a stub control plane listing `tenant.test`
//! as trusted:
//!
//!   1. The sign-in starts ON `tenant.test` and sets a host-only binding
//!      cookie there. Starting it on the app's own host is refused.
//!   2. The provider returns to the app's host. The callback sets NO cookie
//!      there; it redirects to `https://tenant.test/api/auth/handoff?code=…`.
//!   3. The handoff on `tenant.test`, in the browser holding the binding
//!      cookie, sets a host-only session cookie and redirects to the page the
//!      sign-in started from. `/api/auth/me` on `tenant.test` is that user.
//!   4. A replayed code, a code redeemed on another host, and a code redeemed
//!      without the binding cookie are all refused.
//!
//! `PYLON_COOKIE_DOMAIN` is set to a domain `tenant.test` is outside of, so
//! the test also pins that every cookie on the tenant host is host-only.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use pylon_kernel::{AppManifest, ManifestEntity, ManifestField};
use pylon_runtime::Runtime;

const TENANT: &str = "tenant.test";

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
        required_env: Vec::new(),
        build: Default::default(),
        shards: Vec::new(),
        manifest_version: 1,
        name: "oauth-tenant-handoff".into(),
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

fn free_port(lane: std::ops::Range<u16>) -> u16 {
    for p in lane.step_by(4) {
        if TcpListener::bind(format!("127.0.0.1:{p}")).is_ok() {
            return p;
        }
    }
    panic!("no free port");
}

/// The control plane's `listProjectTrustedHosts`, answering `tenant.test`
/// to the right bearer token and 401 to anything else.
fn start_control_plane(port: u16) {
    let listener = TcpListener::bind(format!("127.0.0.1:{port}")).unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase();
            let body = if req.starts_with("post /api/fn/listprojecttrustedhosts")
                && req.contains("authorization: bearer domains-token")
            {
                format!(r#"{{"hosts":["{TENANT}"]}}"#)
            } else {
                String::new()
            };
            let status = if body.is_empty() {
                "401 Unauthorized"
            } else {
                "200 OK"
            };
            let _ = stream.write_all(
                format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            );
        }
    });
}

struct Resp {
    status: u16,
    set_cookies: Vec<String>,
    headers: Vec<(String, String)>,
    body: String,
}

impl Resp {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// Raw HTTP to the server on `port`, presenting `host` as the Host header.
/// Redirects are never followed.
fn request(
    port: u16,
    host: &str,
    method: &str,
    path: &str,
    body: Option<&str>,
    extra: &[(&str, &str)],
) -> Resp {
    let body_str = body.unwrap_or("");
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body_str.len()
    );
    for (k, v) in extra {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(body_str);
    let mut stream = TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
    stream.write_all(req.as_bytes()).expect("write");
    let mut raw = Vec::new();
    let _ = stream.read_to_end(&mut raw);
    let text = String::from_utf8_lossy(&raw).to_string();
    let status = text
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
    let body = text
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_string())
        .unwrap_or_default();
    Resp {
        status,
        set_cookies,
        headers,
        body,
    }
}

fn query_param(url: &str, name: &str) -> Option<String> {
    let q = url.split_once('?')?.1;
    q.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == name).then(|| v.to_string())
    })
}

/// The path (and query) of an absolute URL.
fn path_of(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    rest.find('/')
        .map(|i| rest[i..].to_string())
        .unwrap_or_else(|| "/".into())
}

fn cookie_named<'a>(set_cookies: &'a [String], name: &str) -> Option<&'a String> {
    set_cookies.iter().find(|c| {
        c.split(';')
            .next()
            .is_some_and(|kv| kv.starts_with(&format!("{name}=")))
    })
}

fn pair(set_cookie: &str) -> String {
    set_cookie.split(';').next().unwrap_or("").to_string()
}

struct Env {
    port: u16,
    main: String,
    idp_token: String,
    session_cookie: String,
}

/// Run a sign-in from `tenant.test` up to the handoff redirect. Returns the
/// handoff URL and the binding cookie pair the start set.
fn sign_in_until_handoff(env: &Env) -> (String, String) {
    let callback = format!("https://{TENANT}/board?x=1");
    let error_callback = format!("https://{TENANT}/login");
    let kickoff = request(
        env.port,
        TENANT,
        "GET",
        &format!(
            "/api/auth/login/selfidp?callback={}&error_callback={}",
            url_encode(&callback),
            url_encode(&error_callback)
        ),
        None,
        &[],
    );
    assert_eq!(
        kickoff.status, 200,
        "kickoff on the tenant host: {}",
        kickoff.body
    );
    let binding_name = format!("{}_handoff", env.session_cookie);
    let binding = cookie_named(&kickoff.set_cookies, &binding_name)
        .unwrap_or_else(|| panic!("kickoff must set {binding_name}: {:?}", kickoff.set_cookies));
    assert!(
        !binding.contains("Domain="),
        "the binding cookie on the tenant host must be host-only: {binding}"
    );
    assert!(binding.contains("HttpOnly"), "{binding}");
    let authorize_url = serde_json::from_str::<serde_json::Value>(&kickoff.body).unwrap()
        ["redirect"]
        .as_str()
        .unwrap()
        .to_string();

    // The IdP leg, on the app's own host, with the IdP session.
    let auth_header = format!("Bearer {}", env.idp_token);
    let authorize = request(
        env.port,
        &env.main,
        "GET",
        &path_of(&authorize_url),
        None,
        &[("Authorization", auth_header.as_str())],
    );
    assert_eq!(authorize.status, 302, "authorize: {}", authorize.body);
    let callback_url = authorize.header("location").unwrap().to_string();

    // The provider returns to the app's host.
    let cb = request(
        env.port,
        &env.main,
        "GET",
        &path_of(&callback_url),
        None,
        &[],
    );
    assert_eq!(cb.status, 302, "callback: {}", cb.body);
    let location = cb.header("location").unwrap().to_string();
    assert!(
        location.starts_with(&format!("https://{TENANT}/api/auth/handoff?code=hoff_")),
        "the callback must hand off to the tenant host, got {location}"
    );
    assert!(
        cookie_named(&cb.set_cookies, &env.session_cookie).is_none(),
        "no session cookie on the app's own host: {:?}",
        cb.set_cookies
    );
    assert_eq!(cb.header("referrer-policy"), Some("no-referrer"));
    (location, pair(binding))
}

#[test]
fn oauth_sign_in_is_handed_to_the_tenant_host() {
    let port = free_port(16_000..16_400);
    let cp_port = free_port(16_400..16_800);
    let main = format!("127.0.0.1:{port}");
    let origin = format!("http://{main}");
    let redirect_uri = format!("{origin}/api/auth/callback/selfidp");
    let key_path = std::env::temp_dir().join(format!("pylon-tenant-handoff-{port}.pem"));
    start_control_plane(cp_port);

    // One test in this binary, so no set_var races.
    unsafe {
        std::env::set_var("PYLON_DEV_MODE", "1");
        std::env::set_var("PYLON_ADMIN_TOKEN", "handoff-admin-token");
        std::env::set_var("PYLON_CLOUD_URL", format!("http://127.0.0.1:{cp_port}"));
        std::env::set_var("PYLON_DOMAINS_TOKEN", "domains-token");
        // A cookie domain the tenant host is outside of.
        std::env::set_var("PYLON_COOKIE_DOMAIN", ".app.example");
        std::env::set_var("PYLON_OIDC_ISSUER", &origin);
        std::env::set_var(
            "PYLON_OIDC_CLIENTS",
            format!(
                r#"[{{"client_id":"self-app","client_secret":"s3cr3t-handoff","redirect_uris":["{redirect_uri}"]}}]"#
            ),
        );
        std::env::set_var("PYLON_OIDC_KEY_PATH", &key_path);
        std::env::set_var("PYLON_LOGIN_URL", "/login");
        std::env::set_var("PYLON_OAUTH_SELFIDP_OIDC_ISSUER", &origin);
        std::env::set_var("PYLON_OAUTH_SELFIDP_CLIENT_ID", "self-app");
        std::env::set_var("PYLON_OAUTH_SELFIDP_CLIENT_SECRET", "s3cr3t-handoff");
        std::env::set_var("PYLON_OAUTH_SELFIDP_REDIRECT", &redirect_uri);
    }
    pylon_auth::oidc_provider::OidcKeyStore::load_or_generate(&key_path).unwrap();

    let rt = Arc::new(Runtime::in_memory(test_manifest()).unwrap());
    std::thread::spawn(move || {
        let _ = pylon_runtime::server::start(rt, port);
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while TcpStream::connect(&main).is_err() {
        assert!(
            std::time::Instant::now() < deadline,
            "server never bound {main}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    // The user, and their IdP-side session.
    let admin = [("Authorization", "Bearer handoff-admin-token")];
    let created = request(
        port,
        &main,
        "POST",
        "/api/entities/User",
        Some(r#"{"email":"tenant-user@example.com","displayName":"Tenant User"}"#),
        &admin,
    );
    assert_eq!(created.status, 201, "create user: {}", created.body);
    let user: serde_json::Value = serde_json::from_str(&created.body).unwrap();
    let user_id = user
        .get("id")
        .or_else(|| user.get("data").and_then(|d| d.get("id")))
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();
    let minted = request(
        port,
        &main,
        "POST",
        "/api/auth/session",
        Some(&format!(r#"{{"user_id":"{user_id}"}}"#)),
        &[],
    );
    assert_eq!(minted.status, 201, "dev session: {}", minted.body);
    let idp_token = serde_json::from_str::<serde_json::Value>(&minted.body).unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();
    let env = Env {
        port,
        main: main.clone(),
        idp_token,
        session_cookie: "oauth-tenant-handoff_session".into(),
    };

    // The trusted-host set refreshes from the control plane in the
    // background; wait until the tenant host is accepted as a callback.
    let callback = url_encode(&format!("https://{TENANT}/"));
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        let r = request(
            port,
            TENANT,
            "GET",
            &format!("/api/auth/login/selfidp?callback={callback}"),
            None,
            &[],
        );
        if r.status == 200 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "tenant host never became trusted: {} {}",
            r.status,
            r.body
        );
        std::thread::sleep(Duration::from_millis(100));
    }

    // A sign-in that returns to the tenant host must start there.
    let r = request(
        port,
        &main,
        "GET",
        &format!("/api/auth/login/selfidp?callback={callback}"),
        None,
        &[],
    );
    assert_eq!(r.status, 400, "{}", r.body);
    assert!(r.body.contains("HANDOFF_WRONG_START_HOST"), "{}", r.body);

    // An untrusted https host is still refused as a callback.
    let r = request(
        port,
        "evil.test",
        "GET",
        &format!(
            "/api/auth/login/selfidp?callback={}",
            url_encode("https://evil.test/")
        ),
        None,
        &[],
    );
    assert_eq!(r.status, 403, "{}", r.body);

    // ── The happy path. ─────────────────────────────────────────────────
    let (handoff, binding) = sign_in_until_handoff(&env);
    let done = request(
        port,
        TENANT,
        "GET",
        &path_of(&handoff),
        None,
        &[("Cookie", binding.as_str())],
    );
    assert_eq!(done.status, 302, "handoff: {}", done.body);
    assert_eq!(
        done.header("location"),
        Some(format!("https://{TENANT}/board?x=1").as_str()),
        "back to the page the sign-in started from"
    );
    assert_eq!(done.header("cache-control"), Some("no-store"));
    let session = cookie_named(&done.set_cookies, &env.session_cookie)
        .unwrap_or_else(|| panic!("handoff must set the session: {:?}", done.set_cookies));
    assert!(
        !session.contains("Domain="),
        "host-only on the tenant host: {session}"
    );
    let cleared = cookie_named(
        &done.set_cookies,
        &format!("{}_handoff", env.session_cookie),
    )
    .expect("the binding cookie is cleared");
    assert!(cleared.contains("Max-Age=0"), "{cleared}");

    let me = request(
        port,
        TENANT,
        "GET",
        "/api/auth/me",
        None,
        &[("Cookie", pair(session).as_str())],
    );
    assert_eq!(me.status, 200, "{}", me.body);
    let me: serde_json::Value = serde_json::from_str(&me.body).unwrap();
    assert_eq!(me["user_id"].as_str(), Some(user_id.as_str()));

    // Single use.
    let again = request(
        port,
        TENANT,
        "GET",
        &path_of(&handoff),
        None,
        &[("Cookie", binding.as_str())],
    );
    assert_eq!(again.status, 400, "{}", again.body);
    assert!(again.body.contains("HANDOFF_INVALID"), "{}", again.body);

    // ── Another host cannot redeem it. ──────────────────────────────────
    let (handoff, binding) = sign_in_until_handoff(&env);
    let r = request(
        port,
        "other.test",
        "GET",
        &path_of(&handoff),
        None,
        &[("Cookie", binding.as_str())],
    );
    assert_eq!(r.status, 400, "{}", r.body);
    assert!(r.body.contains("HANDOFF_WRONG_HOST"), "{}", r.body);
    assert!(
        cookie_named(&r.set_cookies, &env.session_cookie).is_none(),
        "no session on a refused handoff"
    );

    // ── Another browser (no binding cookie) cannot redeem it. ───────────
    let (handoff, _binding) = sign_in_until_handoff(&env);
    let r = request(port, TENANT, "GET", &path_of(&handoff), None, &[]);
    assert_eq!(r.status, 403, "{}", r.body);
    assert!(r.body.contains("HANDOFF_OTHER_BROWSER"), "{}", r.body);
    assert!(cookie_named(&r.set_cookies, &env.session_cookie).is_none());
}

fn url_encode(s: &str) -> String {
    let mut out = String::new();
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
