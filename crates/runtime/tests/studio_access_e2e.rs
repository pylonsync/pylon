//! Who can actually load `/studio`, against a running server.
//!
//! The unit tests in `server::studio_access_tests` pin the decision function.
//! These drive the real HTTP path, because the interesting failures were never
//! in the decision — they were in what the request handler did before reaching
//! it. Two in particular:
//!
//!   - The gate opened completely whenever `PYLON_DEV_MODE` was set. The whole
//!     data model (every entity, field, function and policy name is baked into
//!     the bundle at build time) went to any unauthenticated caller who could
//!     reach the port, which includes a LAN peer, an ngrok tunnel, and a
//!     forwarded Codespace port. This harness sets dev mode on purpose, so
//!     every assertion below also asserts that the exemption is gone.
//!   - `PYLON_ADMIN_TOKEN` was accepted as a Studio credential. A shared secret
//!     names nobody, so a destructive edit had no one to attribute it to.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::Duration;

use pylon_kernel::{AppManifest, ManifestEntity, ManifestField};
use pylon_runtime::Runtime;
use serde_json::{json, Value};

const ADMIN_TOKEN: &str = "studio-e2e-admin-token";

fn user_field(name: &str, ty: &str) -> ManifestField {
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
    }
}

/// A `User` entity whose `isAdmin` column is the configured `adminField` —
/// the ordinary way an app designates a platform admin.
fn test_runtime() -> Arc<Runtime> {
    let mut manifest = AppManifest {
        manifest_version: 1,
        name: "studio-access-e2e".into(),
        version: "0.1.0".into(),
        ..Default::default()
    };
    manifest.entities = vec![ManifestEntity {
        name: "User".into(),
        fields: vec![
            user_field("isAdmin", "bool"),
            user_field("email", "string"),
            user_field("emailVerified", "bool"),
        ],
        indexes: vec![],
        relations: vec![],
        search: None,
        crdt: false,
        sync: false,
        ..Default::default()
    }];
    manifest.auth.user.entity = "User".into();
    manifest.auth.user.admin_field = Some("isAdmin".into());
    Arc::new(Runtime::in_memory(manifest).unwrap())
}

fn available_port() -> u16 {
    static NEXT: AtomicU16 = AtomicU16::new(46_500);
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

fn start_server(rt: Arc<Runtime>) -> u16 {
    let port = available_port();
    // Set once per binary, before any server thread exists — `set_var` is a
    // data race against other test threads otherwise.
    //
    // Dev mode is deliberately ON for every test in this file: it used to be a
    // blanket bypass of the Studio gate, so leaving it on turns each assertion
    // into a regression test for that bypass.
    static ENV: std::sync::Once = std::sync::Once::new();
    ENV.call_once(|| {
        // SAFETY: exactly once, before any server thread is spawned.
        unsafe {
            std::env::set_var("PYLON_DEV_MODE", "1");
            std::env::set_var("PYLON_ADMIN_TOKEN", ADMIN_TOKEN);
            // Keep the email allowlist out of it — these tests exercise the
            // adminField path, and an inherited value would silently promote.
            std::env::remove_var("PYLON_ADMIN_EMAILS");
        }
    });
    let rt2 = Arc::clone(&rt);
    std::thread::spawn(move || {
        let _ = pylon_runtime::server::start(rt2, port);
    });
    for _ in 0..100 {
        if TcpStream::connect(format!("127.0.0.1:{port}")).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    port
}

/// Minimal HTTP client. `auth` becomes an `Authorization: Bearer` header —
/// which is how both a session token and the admin token are presented.
fn http(
    port: u16,
    method: &str,
    path: &str,
    auth: Option<&str>,
    body: Option<&str>,
) -> (u16, String) {
    let body_str = body.unwrap_or("");
    let mut hdrs = format!(
        "Host: 127.0.0.1:{port}\r\nOrigin: http://127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body_str.len()
    );
    if let Some(t) = auth {
        hdrs.push_str(&format!("Authorization: Bearer {t}\r\n"));
    }
    let req = format!("{method} {path} HTTP/1.1\r\n{hdrs}\r\n{body_str}");
    let mut s = TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(5))).ok();
    s.write_all(req.as_bytes()).unwrap();
    let mut buf = Vec::new();
    let _ = s.read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf).to_string();
    let status = text
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);
    let body = match text.find("\r\n\r\n") {
        Some(i) => text[i + 4..].to_string(),
        None => String::new(),
    };
    (status, body)
}

/// Insert a User row and mint a real session for it. `POST /api/auth/session`
/// mints for an arbitrary user id in dev mode, which is what this harness is.
fn signed_in_user(port: u16, rt: &Arc<Runtime>, user: Value) -> String {
    let uid = rt.insert("User", &user).unwrap();
    let (status, body) = http(
        port,
        "POST",
        "/api/auth/session",
        None,
        Some(&json!({ "user_id": uid }).to_string()),
    );
    assert_eq!(status, 201, "session mint failed: {body}");
    let v: Value = serde_json::from_str(&body).unwrap();
    v["token"].as_str().expect("session token").to_string()
}

/// The Studio bundle is recognisable by the manifest it inlines.
fn is_studio_bundle(body: &str) -> bool {
    body.contains("window.__PYLON_MANIFEST__")
}

#[test]
fn anonymous_gets_no_studio_and_no_schema() {
    let rt = test_runtime();
    let port = start_server(Arc::clone(&rt));

    let (status, body) = http(port, "GET", "/studio", None, None);
    assert_ne!(status, 200, "anonymous must not receive Studio");
    assert_eq!(
        status, 401,
        "expected the sign-in-required page, got {status}"
    );
    assert!(
        !is_studio_bundle(&body),
        "the bundle inlines the whole manifest — it must not reach an anonymous caller"
    );
    // The replacement page must not leak the schema either.
    assert!(
        !body.contains("isAdmin"),
        "sign-in page leaked an entity field name: {body}"
    );
}

#[test]
fn admin_token_alone_does_not_open_studio() {
    // PYLON_ADMIN_TOKEN is a machine credential for /admin/*. It identifies no
    // one, so it cannot be the thing that authorizes a human-facing data
    // browser with destructive controls.
    let rt = test_runtime();
    let port = start_server(Arc::clone(&rt));

    let (status, body) = http(port, "GET", "/studio", Some(ADMIN_TOKEN), None);
    assert_ne!(status, 200, "the admin token must not open Studio");
    assert!(!is_studio_bundle(&body));
}

#[test]
fn admin_token_still_authorizes_the_admin_api() {
    // The other half of the split: closing Studio to the token must not break
    // the deploy scripts and probes that legitimately use it.
    let rt = test_runtime();
    let port = start_server(Arc::clone(&rt));

    let (status, _) = http(port, "GET", "/admin/entities", Some(ADMIN_TOKEN), None);
    assert_eq!(
        status, 200,
        "PYLON_ADMIN_TOKEN must still authorize /admin/* for machine callers"
    );
}

#[test]
fn signed_in_non_admin_is_refused() {
    let rt = test_runtime();
    let port = start_server(Arc::clone(&rt));
    let token = signed_in_user(
        port,
        &rt,
        json!({"isAdmin": false, "email": "nobody@x.test", "emailVerified": true}),
    );

    let (status, body) = http(port, "GET", "/studio", Some(&token), None);
    assert_eq!(status, 403, "a signed-in non-admin must be refused");
    assert!(!is_studio_bundle(&body));
}

#[test]
fn signed_in_admin_user_gets_studio() {
    // The positive case. Without it the tests above would pass on a Studio
    // that is simply broken for everyone.
    let rt = test_runtime();
    let port = start_server(Arc::clone(&rt));
    let token = signed_in_user(
        port,
        &rt,
        json!({"isAdmin": true, "email": "admin@x.test", "emailVerified": true}),
    );

    let (status, body) = http(port, "GET", "/studio", Some(&token), None);
    assert_eq!(status, 200, "a signed-in admin must get Studio: {body}");
    assert!(
        is_studio_bundle(&body),
        "expected the Studio bundle, got {} bytes",
        body.len()
    );
}

#[test]
fn the_admin_token_login_form_is_gone() {
    // It prompted operators to paste the production superuser secret into a
    // browser form, which then parked it in localStorage. Both endpoints that
    // served it are removed; nothing should answer there.
    let rt = test_runtime();
    let port = start_server(Arc::clone(&rt));

    for (method, path) in [("GET", "/studio/login"), ("POST", "/studio/login")] {
        let (status, body) = http(port, method, path, None, None);
        assert_ne!(
            status, 200,
            "{method} {path} still answers — the token form must be gone"
        );
        assert!(
            !body.contains("Admin token"),
            "{method} {path} still renders the token form"
        );
    }
}

#[test]
fn extensions_bundle_follows_the_same_gate() {
    // `/studio/extensions.js` carries app-authored React that introspects the
    // live API surface. It used to be exempt in dev mode alongside /studio.
    let rt = test_runtime();
    let port = start_server(Arc::clone(&rt));

    let (anon, _) = http(port, "GET", "/studio/extensions.js", None, None);
    assert_ne!(anon, 200, "anonymous must not fetch the extensions bundle");

    let (token_auth, _) = http(
        port,
        "GET",
        "/studio/extensions.js",
        Some(ADMIN_TOKEN),
        None,
    );
    assert_ne!(
        token_auth, 200,
        "the admin token must not fetch the extensions bundle either"
    );
}
