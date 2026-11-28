//! The endpoints Studio adds for an operator, against a running server:
//! the change stream (`GET /admin/changes/stream`) and the health report
//! (`GET /admin/health`). Who may call them, and what they show.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pylon_kernel::{AppManifest, ManifestEntity, ManifestField};
use pylon_runtime::Runtime;
use serde_json::json;

const ADMIN_TOKEN: &str = "studio-changes-e2e-admin-token";
const SECRET: &str = "cus_never_shown_9f3";

fn field(name: &str, ty: &str, server_only: bool) -> ManifestField {
    ManifestField {
        name: name.into(),
        field_type: ty.into(),
        optional: true,
        server_only,
        ..Default::default()
    }
}

fn test_runtime() -> Arc<Runtime> {
    let mut manifest = AppManifest {
        manifest_version: 1,
        name: "studio-changes-e2e".into(),
        version: "0.1.0".into(),
        ..Default::default()
    };
    manifest.entities = vec![
        ManifestEntity {
            name: "User".into(),
            fields: vec![field("email", "string", false)],
            ..Default::default()
        },
        ManifestEntity {
            name: "Org".into(),
            fields: vec![
                field("name", "string", false),
                field("stripeCustomerId", "string", true),
            ],
            ..Default::default()
        },
    ];
    Arc::new(Runtime::in_memory(manifest).unwrap())
}

fn start_server(rt: Arc<Runtime>) -> u16 {
    static ENV: std::sync::Once = std::sync::Once::new();
    ENV.call_once(|| {
        // SAFETY: exactly once, before any server thread is spawned.
        unsafe {
            std::env::set_var("PYLON_DEV_MODE", "1");
            std::env::set_var("PYLON_ADMIN_TOKEN", ADMIN_TOKEN);
            std::env::remove_var("PYLON_ADMIN_EMAILS");
            std::env::set_var("PYLON_AUTH_LOGIN_IP_PER_MIN", "1000");
        }
    });
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    std::thread::spawn(move || {
        let _ = pylon_runtime::server::start(rt, port);
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if TcpStream::connect(format!("127.0.0.1:{port}")).is_ok() {
            return port;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("the test server never bound 127.0.0.1:{port}");
}

/// Send one request and return the open connection.
fn open(port: u16, method: &str, path: &str, headers: &str, body: &str) -> TcpStream {
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{headers}\r\n{body}",
        body.len()
    );
    let mut s = TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect");
    s.set_read_timeout(Some(Duration::from_millis(200))).ok();
    s.write_all(req.as_bytes()).unwrap();
    s
}

/// Read from the connection until `done` accepts the text or `wait` passes.
fn read_until(s: &mut TcpStream, wait: Duration, done: impl Fn(&str) -> bool) -> String {
    let deadline = Instant::now() + wait;
    let mut text = String::new();
    let mut buf = [0u8; 4096];
    while Instant::now() < deadline && !done(&text) {
        match s.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => text.push_str(&String::from_utf8_lossy(&buf[..n])),
            Err(_) => {}
        }
    }
    text
}

fn status_of(text: &str) -> u16 {
    text.lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// A request that ends: the status and the whole response.
fn request(port: u16, method: &str, path: &str, headers: &str, body: &str) -> (u16, String) {
    let mut s = open(
        port,
        method,
        path,
        &format!("Connection: close\r\n{headers}"),
        body,
    );
    let text = read_until(&mut s, Duration::from_secs(5), |t| {
        t.contains("\r\n\r\n") && !t.starts_with("HTTP/1.1 200")
    });
    (status_of(&text), text)
}

fn bearer() -> String {
    format!("Authorization: Bearer {ADMIN_TOKEN}\r\n")
}

/// Create an operator, sign in, and return the `name=value` of its cookie.
fn operator_cookie(port: u16) -> String {
    let (created, text) = request(
        port,
        "POST",
        "/admin/operators",
        &bearer(),
        &json!({"username": "ops", "password": "correct-horse-battery"}).to_string(),
    );
    assert_eq!(created, 201, "{text}");
    let form = "username=ops&password=correct-horse-battery";
    let req = format!(
        "POST /studio/login HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://127.0.0.1:{port}\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{form}",
        form.len()
    );
    let mut s = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).ok();
    s.write_all(req.as_bytes()).unwrap();
    let mut text = String::new();
    let _ = s.read_to_string(&mut text);
    text.lines()
        .find_map(|l| {
            let (name, value) = l.split_once(':')?;
            name.eq_ignore_ascii_case("set-cookie")
                .then(|| value.trim().split(';').next().unwrap_or("").to_string())
        })
        .expect("the sign-in sets a cookie")
}

#[test]
fn a_request_with_no_credential_is_refused() {
    let port = start_server(test_runtime());
    let (status, text) = request(port, "GET", "/admin/changes/stream", "", "");
    assert_eq!(status, 401, "{text}");
}

#[test]
fn an_operator_cookie_from_another_origin_is_refused() {
    let port = start_server(test_runtime());
    let cookie = operator_cookie(port);
    let (status, text) = request(
        port,
        "GET",
        "/admin/changes/stream",
        &format!("Cookie: {cookie}\r\nOrigin: https://evil.example\r\n"),
        "",
    );
    assert_eq!(status, 403, "{text}");
    assert!(text.contains("ORIGIN_REJECTED"), "{text}");
    assert!(!text.contains("event: hello"), "{text}");
}

#[test]
fn an_operator_sees_a_patch_and_never_a_server_only_value() {
    let rt = test_runtime();
    let port = start_server(Arc::clone(&rt));
    let cookie = operator_cookie(port);
    let org = rt
        .insert("Org", &json!({"name": "Acme", "stripeCustomerId": SECRET}))
        .unwrap();

    let mut stream = open(
        port,
        "GET",
        "/admin/changes/stream",
        &format!("Cookie: {cookie}\r\nOrigin: http://127.0.0.1:{port}\r\n"),
        "",
    );
    let head = read_until(&mut stream, Duration::from_secs(5), |t| {
        t.contains("event: hello")
    });
    assert_eq!(status_of(&head), 200, "{head}");
    assert!(
        head.to_ascii_lowercase()
            .contains("content-type: text/event-stream"),
        "{head}"
    );
    assert!(
        !head
            .to_ascii_lowercase()
            .contains("access-control-allow-origin"),
        "the stream must not be readable from another origin: {head}"
    );

    let (patched, text) = request(
        port,
        "PATCH",
        &format!("/api/entities/Org/{org}"),
        &bearer(),
        &json!({"name": "Acme Ltd", "stripeCustomerId": SECRET}).to_string(),
    );
    assert_eq!(patched, 200, "{text}");

    let events = read_until(&mut stream, Duration::from_secs(5), |t| {
        t.contains("event: change")
    });
    let all = format!("{head}{events}");
    assert!(all.contains("event: change"), "no change arrived: {all}");
    assert!(all.contains("\"entity\":\"Org\""), "{all}");
    assert!(all.contains(&format!("\"row_id\":\"{org}\"")), "{all}");
    assert!(all.contains("\"kind\":\"update\""), "{all}");
    assert!(all.contains("Acme Ltd"), "{all}");
    assert!(all.contains("\"stripeCustomerId\":\"[redacted]\""), "{all}");
    assert!(
        !all.contains(SECRET),
        "a serverOnly value is in the stream: {all}"
    );
}

#[test]
fn the_admin_token_opens_the_stream() {
    let port = start_server(test_runtime());
    let mut stream = open(port, "GET", "/admin/changes/stream", &bearer(), "");
    let head = read_until(&mut stream, Duration::from_secs(5), |t| {
        t.contains("event: hello")
    });
    assert_eq!(status_of(&head), 200, "{head}");
    assert!(head.contains("\"head_seq\""), "{head}");
}

#[test]
fn the_health_report_needs_a_credential() {
    let port = start_server(test_runtime());
    let (status, text) = request(port, "GET", "/admin/health", "", "");
    assert_eq!(status, 401, "{text}");
    assert!(!text.contains("pylon_version"), "{text}");
}

#[test]
fn the_health_report_probes_the_database_and_reports_the_runtime_state() {
    let port = start_server(test_runtime());
    let cookie = operator_cookie(port);
    let mut s = open(
        port,
        "GET",
        "/admin/health",
        &format!("Connection: close\r\nCookie: {cookie}\r\n"),
        "",
    );
    let text = read_until(&mut s, Duration::from_secs(5), |t| t.contains("\"memory\""));
    assert_eq!(status_of(&text), 200, "{text}");
    let body = &text[text.find("\r\n\r\n").expect("a body") + 4..];
    let json_start = body.find('{').expect("json");
    let json_end = body.rfind('}').expect("json");
    let v: serde_json::Value = serde_json::from_str(&body[json_start..=json_end]).expect(body);

    assert_eq!(v["database"]["backend"], "sqlite");
    assert_eq!(v["database"]["probe"]["ok"], true, "{v}");
    assert!(v["database"]["probe"]["latency_ms"].is_number());
    assert_eq!(v["scheduler"]["leader"], true);
    assert!(v["jobs"]["stats"]["pending"].is_number());
    assert!(v["realtime"]["change_seq"].is_number());
    assert!(v["status"].is_string());
    assert_eq!(v["app"]["name"], "studio-changes-e2e");
    assert!(v["pylon_version"].as_str().is_some_and(|s| !s.is_empty()));
}

#[test]
fn the_list_endpoint_filters_by_a_substring_and_keeps_server_only_fields_out() {
    let rt = test_runtime();
    let port = start_server(Arc::clone(&rt));
    for name in ["Acme Tools", "Northwind", "Acme Travel"] {
        rt.insert("Org", &json!({"name": name, "stripeCustomerId": SECRET}))
            .unwrap();
    }
    let list = |query: &str| -> serde_json::Value {
        let mut s = open(
            port,
            "GET",
            &format!("/api/entities/Org?page=1&per_page=10&{query}"),
            &format!("Connection: close\r\n{}", bearer()),
            "",
        );
        let text = read_until(&mut s, Duration::from_secs(5), |t| t.contains("\"total\""));
        let body = &text[text.find("\r\n\r\n").expect("a body") + 4..];
        serde_json::from_str(&body[body.find('{').unwrap()..=body.rfind('}').unwrap()])
            .unwrap_or_else(|e| panic!("{e}: {text}"))
    };

    let acme = list("contains[name]=Acme");
    assert_eq!(acme["total"], 2, "{acme}");
    let names: Vec<&str> = acme["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert!(names.iter().all(|n| n.contains("Acme")), "{names:?}");

    assert_eq!(list("contains[name]=wind")["total"], 1);
    assert_eq!(list("contains[name]=zzz")["total"], 0);
    assert_eq!(list("filter[name]=Northwind")["total"], 1);

    // A serverOnly field is not a field a caller can filter on.
    let (status, text) = request(
        port,
        "GET",
        &format!("/api/entities/Org?page=1&per_page=10&contains[stripeCustomerId]={SECRET}"),
        &bearer(),
        "",
    );
    assert_ne!(status, 200, "{text}");
}

#[test]
fn deleting_an_operator_closes_an_existing_change_stream() {
    let rt = test_runtime();
    let port = start_server(Arc::clone(&rt));
    let cookie = operator_cookie(port);
    let org = rt
        .insert("Org", &json!({"name":"before-revocation"}))
        .unwrap();
    let mut stream = open(
        port,
        "GET",
        "/admin/changes/stream",
        &format!("Cookie: {cookie}\r\nOrigin: http://127.0.0.1:{port}\r\n"),
        "",
    );
    let head = read_until(&mut stream, Duration::from_secs(5), |t| {
        t.contains("event: hello")
    });
    assert_eq!(status_of(&head), 200, "{head}");
    assert_eq!(
        request(port, "DELETE", "/admin/operators/ops", &bearer(), "").0,
        200
    );
    assert_eq!(
        request(
            port,
            "PATCH",
            &format!("/api/entities/Org/{org}"),
            &bearer(),
            &json!({"name":"after-revocation"}).to_string()
        )
        .0,
        200
    );
    let events = read_until(&mut stream, Duration::from_secs(3), |t| {
        t.contains("after-revocation") || t.ends_with("0\r\n\r\n")
    });
    assert!(!events.contains("after-revocation"), "{events}");
    assert!(
        events.ends_with("0\r\n\r\n"),
        "stream did not close: {events}"
    );
}
