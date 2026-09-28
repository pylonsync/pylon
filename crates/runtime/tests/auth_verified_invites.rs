//! Invites attach to a verified email.
//!
//! - Signing in with a magic code proves the address, so pending invites
//!   for it are accepted at that moment.
//! - `GET /api/auth/invites/mine` and `POST /api/auth/invites/by-id/:id/accept`
//!   work only once the User row records `emailVerified`.
//! - The org invite list shows pending invites only.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use pylon_kernel::*;
use pylon_runtime::Runtime;

fn string_field(name: &str, optional: bool) -> ManifestField {
    ManifestField {
        name: name.into(),
        field_type: "string".into(),
        optional,
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

fn entity(name: &str, fields: Vec<ManifestField>) -> ManifestEntity {
    ManifestEntity {
        name: name.into(),
        fields,
        crdt: false,
        ..Default::default()
    }
}

fn manifest() -> AppManifest {
    AppManifest {
        manifest_version: MANIFEST_VERSION,
        name: "auth-verified-invites".into(),
        version: "0.1.0".into(),
        entities: vec![
            entity(
                "User",
                vec![
                    string_field("email", false),
                    string_field("displayName", true),
                    string_field("emailVerified", true),
                ],
            ),
            entity(
                "Org",
                vec![
                    string_field("name", false),
                    string_field("createdBy", false),
                    string_field("createdAt", false),
                ],
            ),
            entity(
                "OrgMember",
                vec![
                    string_field("orgId", false),
                    string_field("userId", false),
                    string_field("role", false),
                    string_field("joinedAt", false),
                ],
            ),
            entity(
                "OrgInvite",
                vec![
                    string_field("orgId", false),
                    string_field("email", false),
                    string_field("role", false),
                    string_field("invitedBy", false),
                    string_field("tokenHash", false),
                    string_field("tokenPrefix", false),
                    string_field("createdAt", false),
                    string_field("expiresAt", false),
                    string_field("acceptedAt", true),
                ],
            ),
        ],
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

/// A signed-in session for a new user with `email`.
fn session_for(port: u16, rt: &Runtime, email: &str, verified: bool) -> (String, String) {
    let mut row = serde_json::json!({ "email": email });
    if verified {
        row["emailVerified"] = "2026-09-28T00:00:00Z".into();
    }
    let id = rt.insert("User", &row).unwrap();
    let (status, s) = call(
        port,
        "POST",
        "/api/auth/session",
        None,
        &format!(r#"{{"user_id":"{id}"}}"#),
    );
    assert_eq!(status, 201, "{s}");
    (id, s["token"].as_str().unwrap().to_string())
}

fn members(rt: &Runtime, org: &str) -> Vec<String> {
    rt.query_filtered("OrgMember", &serde_json::json!({ "orgId": org }))
        .unwrap()
        .iter()
        .map(|r| r["userId"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn invites_attach_to_a_verified_email() {
    let (port, rt) = start();
    let (_alice, alice_tok) = session_for(port, &rt, "alice@dealer.test", true);
    let (status, org) = call(
        port,
        "POST",
        "/api/auth/orgs",
        Some(&alice_tok),
        r#"{"name":"Plaza Motors"}"#,
    );
    assert!(status == 200 || status == 201, "{org}");
    let org_id = org["id"].as_str().unwrap().to_string();
    for email in ["bob@dealer.test", "carol@dealer.test", "dave@dealer.test"] {
        let (status, inv) = call(
            port,
            "POST",
            &format!("/api/auth/orgs/{org_id}/invites"),
            Some(&alice_tok),
            &format!(r#"{{"email":"{email}","role":"member"}}"#),
        );
        assert!(status == 200 || status == 201, "{inv}");
    }

    // Bob signs up with a magic code, without the invite link: joined.
    let (_, sent) = call(
        port,
        "POST",
        "/api/auth/magic/send",
        None,
        r#"{"email":"bob@dealer.test"}"#,
    );
    let code = sent["dev_code"].as_str().unwrap().to_string();
    let (status, v) = call(
        port,
        "POST",
        "/api/auth/magic/verify",
        None,
        &format!(r#"{{"email":"bob@dealer.test","code":"{code}"}}"#),
    );
    assert_eq!(status, 200, "{v}");
    let bob = rt
        .lookup("User", "email", "bob@dealer.test")
        .unwrap()
        .unwrap();
    assert!(members(&rt, &org_id).contains(&bob["id"].as_str().unwrap().to_string()));

    // The org's invite list no longer shows Bob.
    let (_, list) = call(
        port,
        "GET",
        &format!("/api/auth/orgs/{org_id}/invites"),
        Some(&alice_tok),
        "",
    );
    let pending: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["email"].as_str().unwrap())
        .collect();
    assert_eq!(pending.len(), 2, "{list}");
    assert!(!pending.contains(&"bob@dealer.test"));

    // Carol is unverified: she cannot list or accept by id.
    let (_, carol_tok) = session_for(port, &rt, "carol@dealer.test", false);
    let (status, err) = call(port, "GET", "/api/auth/invites/mine", Some(&carol_tok), "");
    assert_eq!(status, 403, "{err}");
    assert_eq!(err["error"]["code"], "EMAIL_NOT_VERIFIED");

    // Dave is verified: he sees his invite and accepts it by id.
    let (dave, dave_tok) = session_for(port, &rt, "dave@dealer.test", true);
    let (status, mine) = call(port, "GET", "/api/auth/invites/mine", Some(&dave_tok), "");
    assert_eq!(status, 200, "{mine}");
    assert_eq!(mine.as_array().unwrap().len(), 1);
    let invite_id = mine[0]["id"].as_str().unwrap().to_string();
    // Carol cannot take Dave's invite.
    let (status, _) = call(
        port,
        "POST",
        &format!("/api/auth/invites/by-id/{invite_id}/accept"),
        Some(&carol_tok),
        "",
    );
    assert_eq!(status, 403);
    let (status, accepted) = call(
        port,
        "POST",
        &format!("/api/auth/invites/by-id/{invite_id}/accept"),
        Some(&dave_tok),
        "",
    );
    assert_eq!(status, 200, "{accepted}");
    assert!(members(&rt, &org_id).contains(&dave));
}
