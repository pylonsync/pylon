//! `POST /api/auth/native/steam` against a fake Steam Web API
//! (tests/support/fake_steam.rs):
//!
//!   - a valid ticket mints a session whose bearer resolves on /api/auth/me,
//!     and a repeat sign-in lands on the same user
//!   - the new User row has a placeholder `.invalid` address, not verified
//!   - a ticket for another identity or another app id is refused
//!   - a banned account is refused (PYLON_STEAM_REFUSE_BANNED=1)
//!   - when Steam does not answer, the route fails closed with a 502
//!   - a malformed or missing ticket is a 400

#[path = "support/fake_steam.rs"]
mod fake_steam;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::Duration;

use pylon_kernel::{AppManifest, ManifestEntity, ManifestField, ManifestPolicy};
use pylon_runtime::Runtime;

const APP_ID: &str = "480";
const IDENTITY: &str = "pylon-test";

fn field(name: &str, optional: bool, unique: bool) -> ManifestField {
    ManifestField {
        name: name.into(),
        field_type: "string".into(),
        optional,
        unique,
        ..Default::default()
    }
}

fn manifest() -> AppManifest {
    AppManifest {
        manifest_version: 1,
        name: "steam-signin".into(),
        version: "0.1.0".into(),
        entities: vec![ManifestEntity {
            name: "User".into(),
            fields: vec![
                field("email", false, true),
                field("displayName", true, false),
                field("emailVerified", true, false),
                field("createdAt", true, false),
            ],
            ..Default::default()
        }],
        policies: vec![ManifestPolicy {
            name: "user_self".into(),
            entity: Some("User".into()),
            allow_read: Some("auth.userId == data.id".into()),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn available_port() -> u16 {
    static NEXT: AtomicU16 = AtomicU16::new(24_800);
    for _ in 0..200 {
        let base = NEXT.fetch_add(4, Ordering::Relaxed);
        if (0..4).all(|off| pylon_runtime::listen::port_is_free(base + off)) {
            return base;
        }
    }
    panic!("no free 4-port block");
}

fn start_server() -> u16 {
    static ENV: std::sync::Once = std::sync::Once::new();
    ENV.call_once(|| {
        let steam = fake_steam::start();
        // SAFETY: set once, before any server thread in this binary reads it.
        unsafe {
            std::env::set_var("PYLON_DEV_MODE", "1");
            std::env::set_var("PYLON_STEAM_WEB_API_KEY", "test-steam-key");
            std::env::set_var("PYLON_STEAM_APP_ID", APP_ID);
            std::env::set_var("PYLON_STEAM_IDENTITY", IDENTITY);
            std::env::set_var("PYLON_STEAM_REFUSE_BANNED", "1");
            std::env::set_var("PYLON_STEAM_API_BASE", &steam);
            std::env::set_var("PYLON_AUTH_VERIFY_IP_PER_MIN", "10000");
        }
    });
    let port = available_port();
    let rt = Arc::new(Runtime::in_memory(manifest()).unwrap());
    std::thread::spawn(move || {
        let _ = pylon_runtime::server::start(rt, port);
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        if TcpStream::connect(format!("127.0.0.1:{port}")).is_ok() {
            return port;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("server never bound port {port}");
}

fn http(port: u16, method: &str, path: &str, bearer: Option<&str>, body: &str) -> (u16, String) {
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if let Some(t) = bearer {
        req.push_str(&format!("Authorization: Bearer {t}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(body);
    let mut stream = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    stream.write_all(req.as_bytes()).unwrap();
    let mut raw = String::new();
    stream.read_to_string(&mut raw).unwrap();
    let status = raw
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body = raw
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_string())
        .unwrap_or_default();
    (status, body)
}

fn sign_in(port: u16, ticket: &str) -> (u16, serde_json::Value) {
    let (status, body) = http(
        port,
        "POST",
        "/api/auth/native/steam",
        None,
        &serde_json::json!({ "ticket": ticket }).to_string(),
    );
    (
        status,
        serde_json::from_str(&body).unwrap_or(serde_json::Value::Null),
    )
}

#[test]
fn a_valid_ticket_signs_in_and_a_repeat_lands_on_the_same_user() {
    let port = start_server();
    let player = "76561197960287930";
    let (status, v) = sign_in(port, &fake_steam::ticket(player, APP_ID, IDENTITY, false));
    assert_eq!(status, 200, "{v}");
    assert_eq!(v["provider"], "steam");
    assert_eq!(v["steam_id"], player);
    let session = v["token"].as_str().unwrap().to_string();
    let user_id = v["user_id"].as_str().unwrap().to_string();
    assert!(v["expires_at"].is_number());

    let (status, me) = http(port, "GET", "/api/auth/me", Some(&session), "");
    assert_eq!(status, 200, "{me}");
    let me: serde_json::Value = serde_json::from_str(&me).unwrap();
    assert_eq!(me["user_id"], user_id);

    let (status, row) = http(
        port,
        "GET",
        &format!("/api/entities/User/{user_id}"),
        Some(&session),
        "",
    );
    assert_eq!(status, 200, "{row}");
    let row: serde_json::Value = serde_json::from_str(&row).unwrap();
    let email = row["email"].as_str().unwrap();
    assert!(email.starts_with(&format!("steam-{player}-")), "{email}");
    assert!(email.ends_with("@steam.invalid"), "{email}");
    assert!(
        row["emailVerified"].is_null(),
        "a placeholder address is not verified: {row}"
    );

    let (status, again) = sign_in(port, &fake_steam::ticket(player, APP_ID, IDENTITY, false));
    assert_eq!(status, 200, "{again}");
    assert_eq!(
        again["user_id"], user_id,
        "the account link finds the same user"
    );

    // Another player is another user.
    let (status, other) = sign_in(
        port,
        &fake_steam::ticket("76561197960265729", APP_ID, IDENTITY, false),
    );
    assert_eq!(status, 200, "{other}");
    assert_ne!(other["user_id"], user_id);
}

#[test]
fn a_ticket_for_another_identity_or_app_is_refused() {
    let port = start_server();
    let player = "76561197960287931";
    let (status, v) = sign_in(
        port,
        &fake_steam::ticket(player, APP_ID, "other-service", false),
    );
    assert_eq!(status, 401, "{v}");
    assert_eq!(v["error"]["code"], "INVALID_TICKET");

    let (status, v) = sign_in(port, &fake_steam::ticket(player, "999", IDENTITY, false));
    assert_eq!(status, 401, "{v}");
    assert_eq!(v["error"]["code"], "INVALID_TICKET");
}

#[test]
fn a_banned_account_is_refused() {
    let port = start_server();
    let (status, v) = sign_in(
        port,
        &fake_steam::ticket("76561197960287932", APP_ID, IDENTITY, true),
    );
    assert_eq!(status, 403, "{v}");
    assert_eq!(v["error"]["code"], "ACCOUNT_BANNED");
}

#[test]
fn steam_not_answering_fails_closed() {
    let port = start_server();
    let (status, v) = sign_in(port, fake_steam::DOWN_TICKET);
    assert_eq!(status, 502, "{v}");
    assert_eq!(v["error"]["code"], "PROVIDER_UNAVAILABLE");
    assert!(v.get("token").is_none());
}

#[test]
fn malformed_and_missing_tickets_are_400() {
    let port = start_server();
    let (status, v) = sign_in(port, "not hex!");
    assert_eq!(status, 400, "{v}");
    let (status, v) = sign_in(port, &"ab".repeat(5000));
    assert_eq!(status, 400, "{v}");
    let (status, _) = http(port, "POST", "/api/auth/native/steam", None, "{}");
    assert_eq!(status, 400);
    let (status, _) = http(port, "GET", "/api/auth/native/steam", None, "");
    assert_eq!(status, 405);
}
