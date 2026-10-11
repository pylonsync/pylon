//! The hosting platform's connection management API (`/_pylon/connections`)
//! and the column upgrade for `_Connection`.
//!
//! The platform hands an app its connections: OAuth tokens it obtained
//! with its own client credentials, or keys the business typed in. The
//! route exists only with `PYLON_CONNECTIONS_TOKEN`, takes only app
//! connections, and never answers with a token or a key.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::Duration;

use pylon_kernel::{AppManifest, ManifestConnection};
use pylon_runtime::Runtime;
use serde_json::{json, Value};

const TOKEN: &str = "manage-token-0123456789abcdef0123456789";
const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn conn(
    name: &str,
    provider: &str,
    owner: &str,
    kind: &str,
    fields: &[&str],
) -> ManifestConnection {
    ManifestConnection {
        name: name.into(),
        provider: provider.into(),
        owner: owner.into(),
        kind: kind.into(),
        fields: fields.iter().map(|f| f.to_string()).collect(),
        ..Default::default()
    }
}

fn manifest() -> AppManifest {
    AppManifest {
        manifest_version: 1,
        name: "connmanage".into(),
        version: "0.1.0".into(),
        connections: vec![
            ManifestConnection {
                scopes: "ITEMS_READ".into(),
                ..conn("square", "square", "app", "", &[])
            },
            // An unknown provider makes no check call, so the test stays offline.
            conn("acme", "acme", "app", "api_key", &["apiKey", "siteId"]),
            conn("google", "google", "", "", &[]),
        ],
        ..Default::default()
    }
}

fn available_port() -> u16 {
    // One 1000-port lane per test binary (see bearer_cookie_fallback).
    static NEXT: AtomicU16 = AtomicU16::new(29_000);
    for _ in 0..200 {
        let base = NEXT.fetch_add(4, Ordering::Relaxed);
        if (0..4).all(|off| pylon_runtime::listen::port_is_free(base + off)) {
            return base;
        }
    }
    panic!("no free 4-port block");
}

fn start_server(rt: Arc<Runtime>) -> u16 {
    let port = available_port();
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
    panic!("test server never bound 127.0.0.1:{port}");
}

fn send(
    port: u16,
    method: &str,
    path: &str,
    bearer: Option<&str>,
    body: Option<Value>,
) -> (u16, String) {
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    let mut hdrs = format!(
        "Host: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if let Some(t) = bearer {
        hdrs.push_str(&format!("Authorization: Bearer {t}\r\n"));
    }
    let raw = format!("{method} {path} HTTP/1.1\r\n{hdrs}\r\n{body}");
    let mut s = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).ok();
    s.write_all(raw.as_bytes()).unwrap();
    let mut buf = Vec::new();
    let _ = s.read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf).to_string();
    let status = text
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let body = text
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_string())
        .unwrap_or_default();
    (status, body)
}

fn listed(port: u16) -> Vec<Value> {
    let (status, body) = send(port, "GET", "/_pylon/connections", Some(TOKEN), None);
    assert_eq!(status, 200, "{body}");
    let v: Value = serde_json::from_str(&body).unwrap();
    v["connections"].as_array().unwrap().clone()
}

/// One test: the route reads process env, so the phases run in order.
#[test]
fn management_api_hands_over_app_connections_only() {
    // SAFETY: the only test in this binary that touches env, and it runs
    // before the server thread starts.
    unsafe {
        std::env::set_var("PYLON_ENCRYPTION_KEY", KEY);
        std::env::set_var("PYLON_DEV_MODE", "1");
        std::env::remove_var("PYLON_CONNECTIONS_TOKEN");
        std::env::remove_var("PYLON_CONNECTIONS_BROKER_URL");
        std::env::remove_var("PYLON_CONNECTIONS_BROKER_TOKEN");
    }
    let rt = Arc::new(Runtime::in_memory(manifest()).unwrap());
    let port = start_server(rt);

    // Without the token set, the route does not exist.
    assert_eq!(
        send(port, "GET", "/_pylon/connections", Some(TOKEN), None).0,
        404
    );

    // SAFETY: as above; the server reads the variable per request.
    unsafe { std::env::set_var("PYLON_CONNECTIONS_TOKEN", TOKEN) };
    assert_eq!(send(port, "GET", "/_pylon/connections", None, None).0, 401);
    assert_eq!(
        send(port, "GET", "/_pylon/connections", Some("wrong"), None).0,
        401
    );

    // Only the app connections are listed, none connected.
    let list = listed(port);
    let names: Vec<&str> = list.iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["acme", "square"]);
    assert!(list.iter().all(|c| c["connected"] == false));
    let acme = list.iter().find(|c| c["name"] == "acme").unwrap();
    assert_eq!(acme["kind"], "api_key");
    assert_eq!(acme["fields"], json!(["apiKey", "siteId"]));
    assert_eq!(
        list.iter().find(|c| c["name"] == "square").unwrap()["declared_scopes"],
        "ITEMS_READ"
    );

    // OAuth tokens from the platform.
    let (status, body) = send(
        port,
        "PUT",
        "/_pylon/connections/square",
        Some(TOKEN),
        Some(json!({
            "accessToken": "sq-access-SECRET",
            "refreshToken": "sq-refresh-SECRET",
            "expiresAt": 4_000_000_000u64,
            "accountId": "MERCHANT1",
        })),
    );
    assert_eq!(status, 200, "{body}");
    assert!(!body.contains("SECRET"));

    // Keys: shape-checked, stored, never echoed.
    let (status, body) = send(
        port,
        "PUT",
        "/_pylon/connections/acme",
        Some(TOKEN),
        Some(json!({ "credentials": { "apiKey": "acme-key-SECRET" } })),
    );
    assert_eq!(status, 400, "a missing field is refused: {body}");
    let (status, body) = send(
        port,
        "PUT",
        "/_pylon/connections/acme",
        Some(TOKEN),
        Some(json!({ "credentials": { "apiKey": "acme-key-SECRET", "siteId": "s1" } })),
    );
    assert_eq!(status, 200, "{body}");

    // A token body for a key connection, or keys for an OAuth one, is refused.
    let (status, _) = send(
        port,
        "PUT",
        "/_pylon/connections/acme",
        Some(TOKEN),
        Some(json!({ "accessToken": "x" })),
    );
    assert_eq!(status, 400);
    let (status, _) = send(
        port,
        "PUT",
        "/_pylon/connections/square",
        Some(TOKEN),
        Some(json!({ "credentials": { "apiKey": "x" } })),
    );
    assert_eq!(status, 400);

    // A user's connection and an unknown name are out of reach.
    let (status, _) = send(
        port,
        "PUT",
        "/_pylon/connections/google",
        Some(TOKEN),
        Some(json!({ "accessToken": "x" })),
    );
    assert_eq!(status, 403);
    let (status, _) = send(
        port,
        "PUT",
        "/_pylon/connections/nope",
        Some(TOKEN),
        Some(json!({ "accessToken": "x" })),
    );
    assert_eq!(status, 404);

    let list = listed(port);
    let square = list.iter().find(|c| c["name"] == "square").unwrap();
    assert_eq!(square["connected"], true);
    assert_eq!(square["account_id"], "MERCHANT1");
    assert!(!serde_json::to_string(&list).unwrap().contains("SECRET"));

    let (status, _) = send(
        port,
        "DELETE",
        "/_pylon/connections/square",
        Some(TOKEN),
        None,
    );
    assert_eq!(status, 200);
    let list = listed(port);
    assert_eq!(
        list.iter().find(|c| c["name"] == "square").unwrap()["connected"],
        false
    );
    assert_eq!(
        list.iter().find(|c| c["name"] == "acme").unwrap()["connected"],
        true
    );
}

#[test]
fn old_connection_table_gains_the_new_columns() {
    let dir = std::env::temp_dir().join(format!("pylon-conn-upgrade-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("app.db");
    let _ = std::fs::remove_file(&path);
    {
        // The table as 0.22.28 created it.
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch(
            "CREATE TABLE \"_Connection\" (\"id\" TEXT PRIMARY KEY NOT NULL, \"userId\" TEXT NOT NULL, \
             \"name\" TEXT NOT NULL, \"provider\" TEXT NOT NULL, \"accessToken\" TEXT NOT NULL, \
             \"refreshToken\" TEXT, \"expiresAt\" INTEGER, \"scope\" TEXT, \"updatedAt\" INTEGER NOT NULL);",
        )
        .unwrap();
    }
    // SAFETY: the key is the same value the other test sets.
    unsafe { std::env::set_var("PYLON_ENCRYPTION_KEY", KEY) };
    Runtime::open(path.to_str().unwrap(), manifest()).unwrap();
    let c = rusqlite::Connection::open(&path).unwrap();
    let mut stmt = c.prepare("PRAGMA table_info(\"_Connection\")").unwrap();
    let cols: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for col in ["accountId", "metadata", "credentials"] {
        assert!(cols.iter().any(|c| c == col), "missing {col}: {cols:?}");
    }
    // Opening again changes nothing.
    Runtime::open(path.to_str().unwrap(), manifest()).unwrap();
    let _ = std::fs::remove_file(&path);
}

#[test]
fn boot_refuses_a_misread_connection_declaration() {
    // SAFETY: same value as the other tests.
    unsafe { std::env::set_var("PYLON_ENCRYPTION_KEY", KEY) };
    for bad in [
        conn("x", "square", "App", "", &[]),
        conn("x", "square", "", "apikey", &[]),
        conn("x", "square", "", "oauth", &["apiKey"]),
        conn("x", "acme", "", "api_key", &["a", "a"]),
    ] {
        let mut m = manifest();
        m.connections = vec![bad.clone()];
        let err = Runtime::in_memory(m)
            .err()
            .unwrap_or_else(|| panic!("accepted {bad:?}"));
        assert_eq!(err.code, "INVALID_CONNECTIONS");
    }
    let mut m = manifest();
    m.connections.push(conn("square", "square", "", "", &[]));
    assert!(
        Runtime::in_memory(m).is_err(),
        "a duplicate name is refused"
    );
}
