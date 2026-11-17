//! Backup-code rotation requires the existing second factor.
use pylon_kernel::{AppManifest, ManifestEntity, ManifestField};
use pylon_runtime::Runtime;
use serde_json::json;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

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

fn call(
    port: u16,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: &str,
) -> (u16, serde_json::Value) {
    let auth = token
        .map(|t| format!("Authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost:{port}\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    stream.write_all(req.as_bytes()).unwrap();
    let mut raw = String::new();
    stream.read_to_string(&mut raw).ok();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw.as_str(), ""));
    let status = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    (
        status,
        serde_json::from_str(body).unwrap_or(serde_json::Value::Null),
    )
}

#[test]
fn backup_code_rotation_requires_current_totp() {
    // SAFETY: this binary has one test; no server thread exists yet.
    unsafe {
        std::env::set_var("PYLON_DEV_MODE", "0");
        std::env::set_var("PYLON_ADMIN_TOKEN", "totp-test-admin");
        std::env::set_var("PYLON_CORS_ORIGIN", "https://app.example.test");
        std::env::set_var("PYLON_JOBS_IN_MEMORY", "1");
        std::env::set_var("PYLON_SESSION_IN_MEMORY", "1");
        std::env::set_var("PYLON_AUTH_LOGIN_IP_PER_MIN", "1000");
        std::env::set_var("PYLON_AUTH_LOGIN_ACCOUNT_PER_HOUR", "5");
        let functions =
            std::env::temp_dir().join(format!("pylon-totp-test-{}", std::process::id()));
        std::fs::create_dir_all(&functions).unwrap();
        std::env::set_var("PYLON_FUNCTIONS_DIR", functions);
    }
    let rt = Arc::new(
        Runtime::in_memory(AppManifest {
            manifest_version: 1,
            name: "totp-security".into(),
            version: "0.1.0".into(),
            entities: vec![ManifestEntity {
                name: "User".into(),
                fields: vec![
                    user_field("email", "string"),
                    user_field("totpSecret", "string"),
                    user_field("totpVerified", "bool"),
                    user_field("totpBackupCodes", "json"),
                ],
                crdt: false,
                ..Default::default()
            }],
            ..Default::default()
        })
        .unwrap(),
    );
    let port = (22000..26000)
        .step_by(4)
        .find(|p| (0..4).all(|n| pylon_runtime::listen::port_is_free(p + n)))
        .unwrap();
    let server = rt.clone();
    std::thread::spawn(move || pylon_runtime::server::start(server, port).unwrap());
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(std::time::Instant::now() < deadline, "server did not start");
        std::thread::sleep(Duration::from_millis(50));
    }
    let secret = pylon_auth::totp::generate_secret();
    let uid = rt.insert("User", &json!({"email":"test@example.test", "totpSecret":pylon_auth::totp::seal_secret(&pylon_auth::totp::base32_encode(&secret)), "totpVerified":true, "totpBackupCodes":["old-hash"]})).unwrap();
    let (status, session) = call(
        port,
        "POST",
        "/api/auth/session",
        Some("totp-test-admin"),
        &json!({"user_id":uid}).to_string(),
    );
    assert_eq!(status, 201);
    let token = session["token"].as_str().unwrap();
    let route = "/api/auth/totp/backup-codes/regenerate";
    assert_eq!(call(port, "POST", route, None, "{}").0, 401);
    for body in ["{}", r#"{"code":"not-a-code"}"#] {
        assert_eq!(call(port, "POST", route, Some(token), body).0, 401);
        assert_eq!(
            rt.get_by_id("User", &uid).unwrap().unwrap()["totpBackupCodes"],
            json!(["old-hash"])
        );
    }
    let current = pylon_auth::totp::compute_now(&secret);
    let (status, rotated) = call(
        port,
        "POST",
        route,
        Some(token),
        &json!({"code":current}).to_string(),
    );
    assert_eq!(status, 200);
    assert_eq!(rotated["codes"].as_array().unwrap().len(), 10);
    assert_ne!(
        rt.get_by_id("User", &uid).unwrap().unwrap()["totpBackupCodes"],
        json!(["old-hash"])
    );
    let backup = rotated["codes"][0].as_str().unwrap();
    assert_eq!(
        call(
            port,
            "POST",
            "/api/auth/totp/verify",
            Some(token),
            &json!({"code":backup}).to_string()
        )
        .0,
        200
    );
    assert_eq!(call(port, "POST", route, Some(token), "{}").0, 401);
    assert_eq!(
        call(
            port,
            "POST",
            route,
            Some(token),
            &json!({"code":pylon_auth::totp::compute_now(&secret)}).to_string()
        )
        .0,
        429
    );
    // Pending enrollment and unreadable secrets must not replace existing codes.
    for (email, enrolled, seed, expected) in [
        (
            "pending@example.test",
            false,
            pylon_auth::totp::base32_encode(&secret),
            400,
        ),
        ("broken@example.test", true, "!invalid!".to_string(), 500),
        ("empty@example.test", true, String::new(), 500),
    ] {
        let id = rt
            .insert(
                "User",
                &json!({"email":email, "totpVerified":enrolled,
            "totpSecret":seed, "totpBackupCodes":["keep-hash"]}),
            )
            .unwrap();
        let (status, session) = call(
            port,
            "POST",
            "/api/auth/session",
            Some("totp-test-admin"),
            &json!({"user_id":id}).to_string(),
        );
        assert_eq!(status, 201);
        assert_eq!(
            call(
                port,
                "POST",
                route,
                session["token"].as_str(),
                &json!({"code":pylon_auth::totp::compute_now(&secret)}).to_string()
            )
            .0,
            expected
        );
        assert_eq!(
            rt.get_by_id("User", &id).unwrap().unwrap()["totpBackupCodes"],
            json!(["keep-hash"])
        );
    }
}
