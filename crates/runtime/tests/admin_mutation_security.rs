//! Production-mode checks for operator authorization, CSRF, and request limits.

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
        max_length: None,
    }
}

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
    static NEXT: AtomicU16 = AtomicU16::new(17_000);
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
    ENV.call_once(|| unsafe {
        std::env::set_var("PYLON_DEV_MODE", "0");
        std::env::set_var("PYLON_JOBS_IN_MEMORY", "1");
        std::env::set_var("PYLON_SESSION_IN_MEMORY", "1");
        let functions = std::env::temp_dir().join(format!(
            "pylon-admin-security-functions-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&functions).unwrap();
        std::env::set_var("PYLON_FUNCTIONS_DIR", functions);
        std::env::set_var("PYLON_ADMIN_TOKEN", ADMIN_TOKEN);
        std::env::set_var("PYLON_METRICS_TOKEN", "review-metrics-token");
        std::env::set_var("PYLON_CORS_ORIGIN", "https://app.example.test");
        std::env::remove_var("PYLON_ADMIN_EMAILS");
        std::env::set_var("PYLON_AUTH_LOGIN_IP_PER_MIN", "1000");
    });
    let rt2 = Arc::clone(&rt);
    let boot_err: std::sync::Arc<std::sync::Mutex<Option<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let boot_err_thread = std::sync::Arc::clone(&boot_err);
    std::thread::spawn(move || {
        let r = pylon_runtime::server::start(rt2, port);
        if let Err(e) = r {
            *boot_err_thread.lock().unwrap() = Some(e.to_string());
        }
    });
    {
        let mut ready = false;
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

fn http(
    port: u16,
    method: &str,
    path: &str,
    auth: Option<&str>,
    body: Option<&str>,
) -> (u16, String) {
    http_with(
        port,
        method,
        path,
        auth,
        body,
        "Origin: https://app.example.test\r\nContent-Type: application/json\r\n",
    )
}

fn http_with(
    port: u16,
    method: &str,
    path: &str,
    auth: Option<&str>,
    body: Option<&str>,
    extra_headers: &str,
) -> (u16, String) {
    let body_str = body.unwrap_or("");
    let mut hdrs = format!(
        "Host: 127.0.0.1:{port}\r\nContent-Length: {}\r\nConnection: close\r\n{extra_headers}",
        body_str.len()
    );
    if let Some(t) = auth {
        hdrs.push_str(&format!("Authorization: Bearer {t}\r\n"));
    }
    let req = format!("{method} {path} HTTP/1.1\r\n{hdrs}\r\n{body_str}");
    let mut s = TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(30))).ok();
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

fn studio_cookie(port: u16) -> String {
    let (status, body) = http(
        port,
        "POST",
        "/admin/studio-ticket",
        Some(ADMIN_TOKEN),
        None,
    );
    assert_eq!(status, 200, "{body}");
    let value: Value = serde_json::from_str(&body).unwrap();
    let path = format!("/studio/enter?ticket={}", value["ticket"].as_str().unwrap());
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut raw = String::new();
    stream.read_to_string(&mut raw).unwrap();
    raw.lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case("set-cookie")
                .then(|| value.trim().split(';').next().unwrap().to_string())
        })
        .expect("studio cookie")
}

#[test]
fn admin_mutations_enforce_credentials_origin_and_body_limits() {
    let port = start_server(test_runtime());
    let payload = json!({"username":"operator", "password":"review-password-12345"}).to_string();
    for token in [None, Some("review-metrics-token")] {
        for (method, path) in [
            ("POST", "/admin/operators"),
            ("POST", "/admin/operators/operator/password"),
            ("DELETE", "/admin/operators/operator"),
            ("POST", "/admin/studio-ticket"),
        ] {
            assert_eq!(
                http(port, method, path, token, Some(&payload)).0,
                401,
                "{method} {path}"
            );
        }
    }
    assert_eq!(
        http(port, "GET", "/metrics", Some("review-metrics-token"), None).0,
        200
    );
    assert_eq!(
        http(
            port,
            "POST",
            "/admin/operators",
            Some(ADMIN_TOKEN),
            Some(&payload)
        )
        .0,
        201
    );
    let cookie = studio_cookie(port);
    let app_cookie = cookie.replacen("_studio=", "=", 1);
    let legacy_headers = format!("Cookie: {app_cookie}\r\nOrigin: https://evil.example.test\r\nContent-Type: application/json\r\n");
    assert_eq!(
        http_with(
            port,
            "POST",
            "/admin/studio-ticket",
            None,
            None,
            &legacy_headers
        )
        .0,
        403
    );
    let forged = json!({"username":"forged", "password":"forged-password-12345"}).to_string();
    for origin in ["Origin: null\r\n", "Origin: https://evil.example.test\r\n"] {
        let headers = format!("Cookie: {cookie}\r\n{origin}Content-Type: text/plain\r\n");
        for (method, path, body) in [
            ("POST", "/admin/operators", forged.as_str()),
            (
                "POST",
                "/admin/operators/operator/password",
                r#"{"password":"changed-password-12345"}"#,
            ),
            ("DELETE", "/admin/operators/operator", ""),
            ("POST", "/admin/studio-ticket", ""),
        ] {
            for bearer in [None, Some("invalid"), Some("review-metrics-token")] {
                assert_eq!(
                    http_with(port, method, path, bearer, Some(body), &headers).0,
                    403,
                    "{method} {path}"
                );
            }
        }
    }
    let (_, operators) = http(port, "GET", "/admin/operators", Some(ADMIN_TOKEN), None);
    let operators: Value = serde_json::from_str(&operators).unwrap();
    assert!(operators["operators"]
        .as_array()
        .unwrap()
        .iter()
        .any(|o| o["username"] == "operator"));
    assert!(!operators["operators"]
        .as_array()
        .unwrap()
        .iter()
        .any(|o| o["username"] == "forged"));
    let (login, _) = http_with(
        port,
        "POST",
        "/studio/login",
        None,
        Some("username=operator&password=review-password-12345"),
        "Origin: https://app.example.test\r\nContent-Type: application/x-www-form-urlencoded\r\n",
    );
    assert_eq!(login, 303);
    let trusted = format!("Cookie: {cookie}\r\nOrigin: https://app.example.test\r\nContent-Type: application/json\r\n");
    assert_eq!(
        http_with(
            port,
            "POST",
            "/admin/operators",
            None,
            Some(&forged),
            &trusted
        )
        .0,
        201
    );
    let session = cookie.split_once('=').unwrap().1;
    assert_eq!(
        http_with(
            port,
            "POST",
            "/admin/studio-ticket",
            Some(session),
            None,
            "Origin: https://evil.example.test\r\n"
        )
        .0,
        200
    );
    assert_eq!(
        http_with(
            port,
            "POST",
            "/admin/operators",
            Some(ADMIN_TOKEN),
            Some(&payload),
            "Content-Type: text/plain\r\n"
        )
        .0,
        415
    );
    assert_eq!(
        http(
            port,
            "POST",
            "/admin/operators",
            Some(ADMIN_TOKEN),
            Some("{")
        )
        .0,
        400
    );
    let oversized = json!({"username":"oversized", "password":"review-password-12345", "padding":"x".repeat(65536)}).to_string();
    assert_eq!(
        http(
            port,
            "POST",
            "/admin/operators",
            Some(ADMIN_TOKEN),
            Some(&oversized)
        )
        .0,
        413
    );
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    write!(stream, "POST /admin/operators HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {ADMIN_TOKEN}\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{oversized}\r\n0\r\n\r\n", oversized.len()).unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).unwrap();
    assert!(reply.starts_with("HTTP/1.1 413"), "{reply}");
    assert_eq!(
        http_with(
            port,
            "DELETE",
            "/admin/operators/forged",
            None,
            None,
            &trusted
        )
        .0,
        200
    );
    assert_eq!(
        http(
            port,
            "POST",
            "/admin/operators/operator/password",
            Some(ADMIN_TOKEN),
            Some(r#"{"password":"changed-password-12345"}"#)
        )
        .0,
        200
    );
    assert_eq!(
        http(
            port,
            "DELETE",
            "/admin/operators/operator",
            Some(ADMIN_TOKEN),
            None
        )
        .0,
        200
    );
}
