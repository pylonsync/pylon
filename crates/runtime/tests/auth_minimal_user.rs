//! Sign-up works on a User entity that declares only what the app needs.
//!
//! The auth routes stamp optional fields (`displayName`, `avatarColor`,
//! `emailVerified`, `createdAt`). A User entity that leaves them out must
//! still get accounts from magic-code and password sign-up.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use pylon_kernel::*;
use pylon_runtime::Runtime;

fn field(name: &str, optional: bool, server_only: bool) -> ManifestField {
    ManifestField {
        name: name.into(),
        field_type: "string".into(),
        optional,
        unique: false,
        crdt: None,
        server_only,
        readonly: false,
        default: None,
        enum_values: None,
        encrypted: false,
        sync_omit: false,
    }
}

fn manifest() -> AppManifest {
    AppManifest {
        manifest_version: MANIFEST_VERSION,
        name: "auth-minimal-user".into(),
        version: "0.1.0".into(),
        entities: vec![ManifestEntity {
            name: "User".into(),
            fields: vec![
                field("email", false, false),
                field("passwordHash", true, true),
            ],
            indexes: vec![ManifestIndex {
                name: "by_email".into(),
                fields: vec!["email".into()],
                unique: true,
                where_clause: None,
            }],
            crdt: false,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn start() -> (u16, Arc<Runtime>) {
    // SAFETY: set before any server thread in this binary starts.
    unsafe {
        std::env::set_var("PYLON_DEV_MODE", "1");
    }
    let port = loop {
        let base = 20_000 + rand::random::<u16>() % 8_000;
        if (0..4).all(|o| std::net::TcpListener::bind(("127.0.0.1", base + o)).is_ok()) {
            break base;
        }
    };
    let rt = Arc::new(Runtime::in_memory(manifest()).unwrap());
    let server_rt = Arc::clone(&rt);
    std::thread::spawn(move || {
        let _ = pylon_runtime::server::start(server_rt, port);
    });
    for _ in 0..100 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return (port, rt);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("server did not start");
}

fn post(port: u16, path: &str, body: &str) -> (u16, serde_json::Value) {
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: localhost:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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
fn magic_code_and_password_sign_up_work_on_a_minimal_user_entity() {
    let (port, rt) = start();

    let (status, sent) = post(
        port,
        "/api/auth/magic/send",
        r#"{"email":"new@seller.test"}"#,
    );
    assert_eq!(status, 200, "{sent}");
    let code = sent["dev_code"]
        .as_str()
        .expect("dev_code in dev mode")
        .to_string();
    let (status, verified) = post(
        port,
        "/api/auth/magic/verify",
        &format!(r#"{{"email":"new@seller.test","code":"{code}"}}"#),
    );
    assert_eq!(status, 200, "{verified}");
    let row = rt
        .lookup("User", "email", "new@seller.test")
        .unwrap()
        .expect("user row");
    assert!(row.get("emailVerified").is_none());

    let password = format!("Mls-{}-{}", rand::random::<u64>(), rand::random::<u64>());
    let (status, registered) = post(
        port,
        "/api/auth/password/register",
        &format!(r#"{{"email":"pw@seller.test","password":"{password}","displayName":"PW"}}"#),
    );
    assert!(status == 200 || status == 201, "{status}: {registered}");
    assert!(rt
        .lookup("User", "email", "pw@seller.test")
        .unwrap()
        .is_some());
}
