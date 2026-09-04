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
        sync_omit: false,
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
    // Below the ephemeral range (Linux 32768-60999): a port up there can be
    // handed to one of these tests' OWN client sockets between the probe
    // below and the server's bind, which surfaces as EADDRINUSE on CI.
    // One 1000-port lane per test binary so parallel binaries can't overlap.
    static NEXT: AtomicU16 = AtomicU16::new(27_000);
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
        // Bound the wall clock, not the attempt count: a failed connect is
        // not instant on every platform (see csrf_form_route.rs).
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        while std::time::Instant::now() < deadline {
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
        status, 303,
        "anonymous should be sent to sign in, got {status}"
    );
    assert!(
        !is_studio_bundle(&body),
        "the bundle inlines the whole manifest — it must not reach an anonymous caller"
    );
    assert!(
        !body.contains("isAdmin"),
        "the redirect leaked an entity field name: {body}"
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
fn deep_links_render_the_shell_and_stay_gated() {
    // Studio writes real URLs now, so a refresh or a pasted link on an inner
    // page has to return the shell rather than 404. The gate has to hold on
    // every one of those paths too — an admin-only surface that is only
    // checked at its index is not gated.
    let rt = test_runtime();
    let port = start_server(Arc::clone(&rt));
    let token = signed_in_user(
        port,
        &rt,
        json!({"isAdmin": true, "email": "admin@x.test", "emailVerified": true}),
    );

    for path in ["/studio/health", "/studio/e/User", "/studio/e/User?page=2"] {
        let (status, body) = http(port, "GET", path, Some(&token), None);
        assert_eq!(status, 200, "{path} should render the shell");
        assert!(is_studio_bundle(&body), "{path} did not return the bundle");

        let (anon_status, anon_body) = http(port, "GET", path, None, None);
        assert_ne!(anon_status, 200, "{path} must stay gated for anonymous");
        assert!(!is_studio_bundle(&anon_body), "{path} leaked the bundle");
    }
}

#[test]
fn app_routes_that_merely_start_with_studio_are_untouched() {
    // `starts_with("/studio")` once 404'd legitimate app pages. Studio owning
    // a URL scheme makes that trap easier to fall into, not harder.
    let rt = test_runtime();
    let port = start_server(Arc::clone(&rt));
    let token = signed_in_user(
        port,
        &rt,
        json!({"isAdmin": true, "email": "admin2@x.test", "emailVerified": true}),
    );

    for path in ["/studios", "/studio-tour"] {
        let (_, body) = http(port, "GET", path, Some(&token), None);
        assert!(
            !is_studio_bundle(&body),
            "{path} belongs to the app, but Studio answered it"
        );
    }
}

/// POST a urlencoded form, returning `(status, Location, Set-Cookie)`.
fn post_form(port: u16, path: &str, body: &str) -> (u16, String, String) {
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://127.0.0.1:{port}\r\n\
         Content-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
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
    let header = |name: &str| {
        text.lines()
            .find(|l| l.to_ascii_lowercase().starts_with(&format!("{name}:")))
            .map(|l| l[name.len() + 1..].trim().to_string())
            .unwrap_or_default()
    };
    (status, header("location"), header("set-cookie"))
}

#[test]
fn an_operator_can_be_bootstrapped_with_the_admin_token_and_then_sign_in() {
    // The whole point: a Pylon with no users at all still has a way in, and the
    // bootstrap credential is the admin token the operator already holds rather
    // than a new global secret.
    let rt = test_runtime();
    let port = start_server(Arc::clone(&rt));

    // Anonymous cannot mint an operator.
    let (anon, _) = http(
        port,
        "POST",
        "/admin/operators",
        None,
        Some(&json!({"username": "eric", "password": "correct-horse-battery"}).to_string()),
    );
    assert_ne!(
        anon, 201,
        "anonymous must not be able to create an operator"
    );

    let (created, body) = http(
        port,
        "POST",
        "/admin/operators",
        Some(ADMIN_TOKEN),
        Some(&json!({"username": "eric", "password": "correct-horse-battery"}).to_string()),
    );
    assert_eq!(created, 201, "operator create failed: {body}");

    // Sign in through the form and get a session cookie.
    let (status, location, cookie) = post_form(
        port,
        "/studio/login",
        "username=eric&password=correct-horse-battery",
    );
    assert_eq!(status, 303, "operator sign-in should redirect");
    assert_eq!(location, "/studio");
    assert!(!cookie.is_empty(), "sign-in set no session cookie");

    // A wrong password does not.
    let (bad, _, bad_cookie) = post_form(port, "/studio/login", "username=eric&password=wrong-one");
    assert_eq!(bad, 401);
    assert!(bad_cookie.is_empty(), "a failed sign-in set a cookie");
}

#[test]
fn an_operator_session_opens_studio_and_can_read_data() {
    // A Studio that loads but whose every panel 403s is not access. The
    // operator has no row in the app's User entity, so `is_admin` has to be
    // lifted for the policy layer too.
    let rt = test_runtime();
    let port = start_server(Arc::clone(&rt));
    let (created, body) = http(
        port,
        "POST",
        "/admin/operators",
        Some(ADMIN_TOKEN),
        Some(&json!({"username": "ops", "password": "correct-horse-battery"}).to_string()),
    );
    assert_eq!(created, 201, "{body}");

    let (_, _, cookie) = post_form(
        port,
        "/studio/login",
        "username=ops&password=correct-horse-battery",
    );
    let token = cookie
        .split(';')
        .next()
        .and_then(|kv| kv.split_once('='))
        .map(|(_, v)| v.to_string())
        .expect("session token in Set-Cookie");

    let (studio, page) = http(port, "GET", "/studio", Some(&token), None);
    assert_eq!(studio, 200, "operator was refused Studio");
    assert!(is_studio_bundle(&page));

    let (me, me_body) = http(port, "GET", "/api/auth/session", Some(&token), None);
    assert_eq!(me, 200);
    let v: Value = serde_json::from_str(&me_body).unwrap();
    assert_eq!(
        v["session"]["is_admin"], true,
        "operator session must resolve as admin for the API layer: {me_body}"
    );

    let (entities, _) = http(port, "GET", "/admin/entities", Some(&token), None);
    assert_eq!(entities, 200, "operator could not read /admin/entities");
}

#[test]
fn deleting_an_operator_kills_its_live_sessions() {
    // Removing the credential while a cookie keeps working is not removing
    // access. This is the difference between `pylon admin rm` meaning something
    // and meaning something in 30 days.
    let rt = test_runtime();
    let port = start_server(Arc::clone(&rt));
    http(
        port,
        "POST",
        "/admin/operators",
        Some(ADMIN_TOKEN),
        Some(&json!({"username": "temp", "password": "correct-horse-battery"}).to_string()),
    );
    let (_, _, cookie) = post_form(
        port,
        "/studio/login",
        "username=temp&password=correct-horse-battery",
    );
    let token = cookie
        .split(';')
        .next()
        .and_then(|kv| kv.split_once('='))
        .map(|(_, v)| v.to_string())
        .expect("session token");
    assert_eq!(http(port, "GET", "/studio", Some(&token), None).0, 200);

    let (deleted, _) = http(
        port,
        "DELETE",
        "/admin/operators/temp",
        Some(ADMIN_TOKEN),
        None,
    );
    assert_eq!(deleted, 200);

    let (after, after_body) = http(port, "GET", "/studio", Some(&token), None);
    assert_ne!(
        after, 200,
        "a deleted operator's session still opens Studio"
    );
    assert!(!is_studio_bundle(&after_body));
}

#[test]
fn the_login_page_says_how_to_bootstrap_when_there_are_no_operators() {
    // A bare form on a Pylon with no operators is a dead end. The page has to
    // tell you the one command that gets you out of it.
    let rt = test_runtime();
    let port = start_server(Arc::clone(&rt));

    let (status, body) = http(port, "GET", "/studio/login", None, None);
    assert_eq!(status, 200, "the login page must be reachable anonymously");
    assert!(
        body.contains("pylon admin create"),
        "empty-store login page should name the bootstrap command"
    );
    assert!(
        !body.contains("PYLON_ADMIN_TOKEN=") && !body.contains("name=\"password\""),
        "should not render a sign-in form when no operator exists"
    );
}

#[test]
fn the_admin_token_login_form_is_gone() {
    // /studio/login still exists, but it now takes an operator's own password.
    // What must not come back is the box that asked for PYLON_ADMIN_TOKEN —
    // it put the deployment's superuser secret in localStorage.
    let rt = test_runtime();
    let port = start_server(Arc::clone(&rt));
    http(
        port,
        "POST",
        "/admin/operators",
        Some(ADMIN_TOKEN),
        Some(&json!({"username": "eric", "password": "correct-horse-battery"}).to_string()),
    );

    let (status, body) = http(port, "GET", "/studio/login", None, None);
    assert_eq!(status, 200, "the operator sign-in page should be reachable");
    assert!(
        !body.contains("Admin token") && !body.contains("PYLON_ADMIN_TOKEN"),
        "the admin-token form is back: {body}"
    );
    assert!(
        body.contains(r#"name="username""#) && body.contains(r#"name="password""#),
        "expected a username/password form"
    );

    // Submitting the admin token as a password must not work.
    let (denied, _, cookie) = post_form(
        port,
        "/studio/login",
        &format!("username=eric&password={ADMIN_TOKEN}"),
    );
    assert_eq!(denied, 401, "the admin token was accepted as a password");
    assert!(cookie.is_empty());
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
