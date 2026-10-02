//! A bearer token that resolves to no one must not hide a valid session
//! cookie, and must not open a way past the CSRF check.
//!
//! The case: a browser holds a valid `<app>_session` cookie (SSR renders the
//! page signed in) and the client also has a stale token in localStorage,
//! which it sends as `Authorization: Bearer`. The server resolved the bearer
//! to anonymous and never read the cookie, so `callFn` got 401.
//!
//! The request loop skips the CSRF Origin check for any request with a bearer
//! header. These tests pin that a stale bearer cannot use that to ride the
//! cookie from another site: the cookie only applies when the request passes
//! the check a cookie-only request must pass.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::Duration;

use pylon_kernel::{AppManifest, ManifestEntity, ManifestField};
use pylon_runtime::Runtime;
use serde_json::{json, Value};

const ADMIN_TOKEN: &str = "bearer-fallback-admin-token";
const COOKIE: &str = "bearerfallback_session";
const EVIL_ORIGIN: &str = "http://evil.test";

fn field(name: &str, ty: &str) -> ManifestField {
    ManifestField {
        name: name.into(),
        field_type: ty.into(),
        optional: true,
        unique: false,
        crdt: None,
        server_only: false,
        readonly: false,
        default: None,
        enum_values: None,
        encrypted: false,
        sync_omit: false,
        max_length: None,
    }
}

fn test_runtime() -> Arc<Runtime> {
    let mut manifest = AppManifest {
        manifest_version: 1,
        // The session cookie is named after the app: `bearerfallback_session`.
        name: "bearerfallback".into(),
        version: "0.1.0".into(),
        ..Default::default()
    };
    manifest.entities = vec![ManifestEntity {
        name: "User".into(),
        fields: vec![field("email", "string"), field("emailVerified", "bool")],
        indexes: vec![],
        relations: vec![],
        search: None,
        crdt: false,
        sync: false,
        ..Default::default()
    }];
    manifest.auth.user.entity = "User".into();
    Arc::new(Runtime::in_memory(manifest).unwrap())
}

fn available_port() -> u16 {
    // One 1000-port lane per test binary, below the ephemeral range, so
    // parallel test binaries can't hand each other a port.
    static NEXT: AtomicU16 = AtomicU16::new(16_000);
    for _ in 0..200 {
        let base = NEXT.fetch_add(4, Ordering::Relaxed);
        let ok = (0..4).all(|off| pylon_runtime::listen::port_is_free(base + off));
        if ok {
            return base;
        }
    }
    panic!("no free 4-port block");
}

fn start_server(rt: Arc<Runtime>) -> u16 {
    let port = available_port();
    static ENV: std::sync::Once = std::sync::Once::new();
    ENV.call_once(|| {
        // SAFETY: exactly once, before any server thread is spawned.
        unsafe {
            std::env::set_var("PYLON_DEV_MODE", "1");
            std::env::set_var("PYLON_ADMIN_TOKEN", ADMIN_TOKEN);
            std::env::remove_var("PYLON_COOKIE_NAME");
            std::env::remove_var("PYLON_CSRF_ORIGINS");
            std::env::remove_var("PYLON_ADMIN_EMAILS");
            std::env::set_var("PYLON_AUTH_LOGIN_IP_PER_MIN", "1000");
        }
    });
    let boot_err: Arc<std::sync::Mutex<Option<String>>> = Arc::new(std::sync::Mutex::new(None));
    let boot_err_thread = Arc::clone(&boot_err);
    std::thread::spawn(move || {
        if let Err(e) = pylon_runtime::server::start(rt, port) {
            *boot_err_thread.lock().unwrap() = Some(e.to_string());
        }
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        if TcpStream::connect(format!("127.0.0.1:{port}")).is_ok() {
            return port;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!(
        "test server never bound 127.0.0.1:{port} within 15s (server error: {:?})",
        boot_err.lock().unwrap()
    );
}

struct Reply {
    status: u16,
    headers: String,
    body: String,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }

    fn bearer_rejected(&self) -> bool {
        self.headers
            .lines()
            .any(|l| l.to_ascii_lowercase().trim() == "x-pylon-bearer-rejected: 1")
    }
}

#[derive(Default)]
struct Req<'a> {
    bearer: Option<&'a str>,
    cookie: Option<&'a str>,
    origin: Option<&'a str>,
    extra: &'a str,
    body: Option<&'a str>,
}

fn send(port: u16, method: &str, path: &str, req: Req<'_>) -> Reply {
    let body = req.body.unwrap_or("");
    let origin = req
        .origin
        .map(str::to_string)
        .unwrap_or_else(|| format!("http://127.0.0.1:{port}"));
    let mut hdrs = format!(
        "Host: 127.0.0.1:{port}\r\nOrigin: {origin}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n{}",
        body.len(),
        req.extra
    );
    if let Some(t) = req.bearer {
        hdrs.push_str(&format!("Authorization: Bearer {t}\r\n"));
    }
    if let Some(c) = req.cookie {
        hdrs.push_str(&format!("Cookie: {COOKIE}={c}\r\n"));
    }
    let raw = format!("{method} {path} HTTP/1.1\r\n{hdrs}\r\n{body}");
    let mut s = TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(5))).ok();
    s.write_all(raw.as_bytes()).unwrap();
    let mut buf = Vec::new();
    let _ = s.read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf).to_string();
    let status = text
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);
    let (headers, body) = match text.find("\r\n\r\n") {
        Some(i) => (text[..i].to_string(), text[i + 4..].to_string()),
        None => (text, String::new()),
    };
    Reply {
        status,
        headers,
        body,
    }
}

/// Insert a User row and mint a real session for it. `POST /api/auth/session`
/// mints for an arbitrary user id in dev mode.
fn session_for(port: u16, rt: &Arc<Runtime>, email: &str) -> (String, String) {
    let uid = rt
        .insert("User", &json!({"email": email, "emailVerified": true}))
        .unwrap();
    let r = send(
        port,
        "POST",
        "/api/auth/session",
        Req {
            body: Some(&json!({ "user_id": uid }).to_string()),
            ..Default::default()
        },
    );
    assert_eq!(r.status, 201, "session mint failed: {}", r.body);
    let token = r.json()["token"].as_str().expect("token").to_string();
    (uid, token)
}

fn me(port: u16, req: Req<'_>) -> (Option<String>, bool) {
    let r = send(port, "GET", "/api/auth/me", req);
    assert_eq!(r.status, 200, "/api/auth/me: {}", r.body);
    (
        r.json()["user_id"].as_str().map(str::to_string),
        r.bearer_rejected(),
    )
}

/// `select-org` with `orgId: null` is a POST that answers 200 for a signed-in
/// caller and 401 for an anonymous one, and acts on the session's token.
fn select_no_org(port: u16, req: Req<'_>) -> Reply {
    send(
        port,
        "POST",
        "/api/auth/select-org",
        Req {
            body: Some(r#"{"orgId":null}"#),
            ..req
        },
    )
}

#[test]
fn a_valid_bearer_wins_over_the_cookie() {
    let rt = test_runtime();
    let port = start_server(Arc::clone(&rt));
    let (bearer_uid, bearer) = session_for(port, &rt, "bearer@x.test");
    let (_, cookie) = session_for(port, &rt, "cookie@x.test");

    let (uid, rejected) = me(
        port,
        Req {
            bearer: Some(&bearer),
            cookie: Some(&cookie),
            ..Default::default()
        },
    );
    assert_eq!(uid.as_deref(), Some(bearer_uid.as_str()));
    assert!(!rejected, "a valid bearer must not be marked rejected");

    // Also from another site: a valid bearer is not ambient, so the CSRF
    // check does not apply to it, as before.
    let r = select_no_org(
        port,
        Req {
            bearer: Some(&bearer),
            cookie: Some(&cookie),
            origin: Some(EVIL_ORIGIN),
            ..Default::default()
        },
    );
    assert_eq!(r.status, 200, "{}", r.body);
}

#[test]
fn an_unknown_bearer_gives_way_to_a_valid_cookie() {
    let rt = test_runtime();
    let port = start_server(Arc::clone(&rt));
    let (cookie_uid, cookie) = session_for(port, &rt, "cookie@x.test");

    let (uid, rejected) = me(
        port,
        Req {
            bearer: Some("not-a-session-token"),
            cookie: Some(&cookie),
            ..Default::default()
        },
    );
    assert_eq!(uid.as_deref(), Some(cookie_uid.as_str()));
    assert!(
        rejected,
        "the response must tell the client to drop the token"
    );

    // A same-origin POST, which is what `callFn` sends.
    let r = select_no_org(
        port,
        Req {
            bearer: Some("not-a-session-token"),
            cookie: Some(&cookie),
            ..Default::default()
        },
    );
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(r.bearer_rejected());
}

#[test]
fn a_revoked_bearer_gives_way_to_a_valid_cookie() {
    let rt = test_runtime();
    let port = start_server(Arc::clone(&rt));
    let (_, revoked) = session_for(port, &rt, "old@x.test");
    let (cookie_uid, cookie) = session_for(port, &rt, "cookie@x.test");
    // Sign out with the first token.
    let out = send(
        port,
        "DELETE",
        "/api/auth/session",
        Req {
            bearer: Some(&revoked),
            ..Default::default()
        },
    );
    assert!(
        out.status < 300,
        "sign-out failed: {} {}",
        out.status,
        out.body
    );
    let (uid, _) = me(
        port,
        Req {
            bearer: Some(&revoked),
            ..Default::default()
        },
    );
    assert_eq!(uid, None, "the revoked token must resolve to no one");

    let (uid, rejected) = me(
        port,
        Req {
            bearer: Some(&revoked),
            cookie: Some(&cookie),
            ..Default::default()
        },
    );
    assert_eq!(uid.as_deref(), Some(cookie_uid.as_str()));
    assert!(rejected);
}

#[test]
fn a_cross_site_post_with_a_stale_bearer_does_not_ride_the_cookie() {
    let rt = test_runtime();
    let port = start_server(Arc::clone(&rt));
    let (_, cookie) = session_for(port, &rt, "victim@x.test");

    // The cookie alone from another site is stopped by the CSRF check.
    let r = select_no_org(
        port,
        Req {
            cookie: Some(&cookie),
            origin: Some(EVIL_ORIGIN),
            ..Default::default()
        },
    );
    assert_eq!(r.status, 403, "cookie-only cross-site POST: {}", r.body);

    // Adding a junk bearer skips that check, so the cookie must not apply:
    // the request stays anonymous.
    let r = select_no_org(
        port,
        Req {
            bearer: Some("junk"),
            cookie: Some(&cookie),
            origin: Some(EVIL_ORIGIN),
            ..Default::default()
        },
    );
    assert_eq!(
        r.status, 401,
        "a cross-site POST with a junk bearer must stay anonymous: {}",
        r.body
    );

    // No Origin and no Referer fails the check too.
    let r = send(
        port,
        "POST",
        "/api/auth/select-org",
        Req {
            bearer: Some("junk"),
            cookie: Some(&cookie),
            origin: Some(""),
            body: Some(r#"{"orgId":null}"#),
            ..Default::default()
        },
    );
    assert_eq!(r.status, 401, "{}", r.body);
}

#[test]
fn a_stale_bearer_without_a_cookie_is_anonymous() {
    let rt = test_runtime();
    let port = start_server(Arc::clone(&rt));

    let (uid, rejected) = me(
        port,
        Req {
            bearer: Some("not-a-session-token"),
            ..Default::default()
        },
    );
    assert_eq!(uid, None);
    assert!(rejected);

    let r = select_no_org(
        port,
        Req {
            bearer: Some("not-a-session-token"),
            ..Default::default()
        },
    );
    assert_eq!(r.status, 401, "{}", r.body);

    // No bearer at all: nothing to reject.
    let (uid, rejected) = me(port, Req::default());
    assert_eq!(uid, None);
    assert!(!rejected);
}

#[test]
fn a_bad_api_key_still_fails_even_with_a_valid_cookie() {
    // An API key is an explicit credential. A typo in one must surface, not
    // be covered by whatever session the browser has.
    let rt = test_runtime();
    let port = start_server(Arc::clone(&rt));
    let (_, cookie) = session_for(port, &rt, "cookie@x.test");
    let r = send(
        port,
        "GET",
        "/api/auth/me",
        Req {
            bearer: Some("pk.nope.nope"),
            cookie: Some(&cookie),
            ..Default::default()
        },
    );
    assert_eq!(r.status, 401, "{}", r.body);
}

/// Create an operator and sign it in to Studio; returns its session token.
fn operator_session(port: u16) -> String {
    let r = send(
        port,
        "POST",
        "/admin/operators",
        Req {
            bearer: Some(ADMIN_TOKEN),
            body: Some(
                &json!({"username": "ops", "password": "correct-horse-battery"}).to_string(),
            ),
            ..Default::default()
        },
    );
    assert_eq!(r.status, 201, "operator create failed: {}", r.body);
    let form = "username=ops&password=correct-horse-battery";
    let raw = format!(
        "POST /studio/login HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://127.0.0.1:{port}\r\n\
         Content-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{form}",
        form.len()
    );
    let mut s = TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(5))).ok();
    s.write_all(raw.as_bytes()).unwrap();
    let mut buf = Vec::new();
    let _ = s.read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf).to_string();
    text.lines()
        .find(|l| l.to_ascii_lowercase().starts_with("set-cookie:"))
        .and_then(|l| l.split_once(':'))
        .and_then(|(_, v)| v.trim().split(';').next())
        .and_then(|kv| kv.split_once('='))
        .map(|(_, v)| v.to_string())
        .expect("operator session cookie")
}

#[test]
fn an_operator_bearer_on_an_app_route_gives_way_to_the_app_cookie() {
    // An operator session is no one on the app's routes (the Studio cookie
    // split). A client that still holds one from before the split must not
    // lose the app user's cookie session to it.
    let rt = test_runtime();
    let port = start_server(Arc::clone(&rt));
    let operator = operator_session(port);
    let (cookie_uid, cookie) = session_for(port, &rt, "cookie@x.test");

    let (uid, rejected) = me(
        port,
        Req {
            bearer: Some(&operator),
            cookie: Some(&cookie),
            ..Default::default()
        },
    );
    assert_eq!(uid.as_deref(), Some(cookie_uid.as_str()));
    assert!(rejected);

    // Alone, it is still no one on the app's routes.
    let (uid, _) = me(
        port,
        Req {
            bearer: Some(&operator),
            ..Default::default()
        },
    );
    assert_eq!(uid, None);

    // On Studio's own requests it is still the operator, and it wins.
    let r = send(
        port,
        "GET",
        "/api/auth/me",
        Req {
            bearer: Some(&operator),
            cookie: Some(&cookie),
            extra: "X-Pylon-Studio: 1\r\n",
            ..Default::default()
        },
    );
    assert_eq!(r.status, 200);
    let v = r.json();
    assert_eq!(v["is_admin"], true, "{}", r.body);
    assert_ne!(v["user_id"].as_str(), Some(cookie_uid.as_str()));
    assert!(!r.bearer_rejected());
}
