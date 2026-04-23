//! Sync protocol correctness tests.
//!
//! These are the contract tests the sync engine needs to pass. Bugs in the
//! protocol silently corrupt client replicas; unit tests of individual
//! pieces don't catch the wiring issues that surface only when the full
//! pipeline runs (HTTP → change log → pull → cursor persistence). Each
//! test exercises one end-to-end scenario and asserts the invariant that
//! matters for local-first apps: the replica eventually matches the server.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;

use serde_json::Value;
use pylon_kernel::{AppManifest, ManifestEntity, ManifestField};
use pylon_runtime::Runtime;

fn test_manifest() -> AppManifest {
    AppManifest {
        manifest_version: 1,
        name: "sync-proto".into(),
        version: "0.1.0".into(),
        entities: vec![ManifestEntity {
            name: "Note".into(),
            fields: vec![
                ManifestField {
                    name: "title".into(),
                    field_type: "string".into(),
                    optional: false,
                    unique: false,
                },
                ManifestField {
                    name: "body".into(),
                    field_type: "string".into(),
                    optional: true,
                    unique: false,
                },
            ],
            indexes: vec![],
            relations: vec![],
        }],
        routes: vec![],
        queries: vec![],
        actions: vec![],
        policies: vec![],
    }
}

fn available_port() -> u16 {
    static NEXT: AtomicU16 = AtomicU16::new(43_000);
    for _ in 0..200 {
        let base = NEXT.fetch_add(4, Ordering::Relaxed);
        let ok = (0..4).all(|off| {
            std::net::TcpListener::bind(format!("127.0.0.1:{}", base + off)).is_ok()
        });
        if ok {
            return base;
        }
    }
    panic!("no free 4-port block");
}

fn start_server(rt: Arc<Runtime>) -> u16 {
    let port = available_port();
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

fn http(port: u16, method: &str, path: &str, auth: Option<&str>, body: Option<&str>) -> (u16, String) {
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

fn mint_guest(port: u16) -> String {
    let (status, body) = http(port, "POST", "/api/auth/guest", None, None);
    assert_eq!(status, 201, "guest mint failed: {body}");
    let v: Value = serde_json::from_str(&body).unwrap();
    v["token"].as_str().unwrap().to_string()
}

fn pull(port: u16, token: &str, since: u64) -> (u16, Value) {
    let (status, body) = http(
        port,
        "GET",
        &format!("/api/sync/pull?since={since}"),
        Some(token),
        None,
    );
    let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    (status, v)
}

fn insert_note(port: u16, token: &str, title: &str) -> String {
    let (status, body) = http(
        port,
        "POST",
        "/api/entities/Note",
        Some(token),
        Some(&format!(r#"{{"title":"{title}","body":""}}"#)),
    );
    assert!(
        status == 200 || status == 201,
        "insert failed status={status} body={body}"
    );
    let v: Value = serde_json::from_str(&body).unwrap();
    v["id"].as_str().unwrap().to_string()
}

// ---------------------------------------------------------------------------
// 1. Fresh client with empty server gets empty state, not errors
// ---------------------------------------------------------------------------
#[test]
fn fresh_pull_on_empty_server_returns_no_changes() {
    let rt = Arc::new(Runtime::in_memory(test_manifest()).unwrap());
    let port = start_server(rt);
    let token = mint_guest(port);

    let (status, resp) = pull(port, &token, 0);
    assert_eq!(status, 200);
    assert_eq!(resp["changes"].as_array().unwrap().len(), 0);
    assert_eq!(resp["cursor"]["last_seq"].as_u64().unwrap(), 0);
}

// ---------------------------------------------------------------------------
// 2. Insert → pull sees the change and advances cursor
// ---------------------------------------------------------------------------
#[test]
fn insert_then_pull_returns_change() {
    let rt = Arc::new(Runtime::in_memory(test_manifest()).unwrap());
    let port = start_server(rt);
    let token = mint_guest(port);

    let id = insert_note(port, &token, "hello");
    let (status, resp) = pull(port, &token, 0);
    assert_eq!(status, 200);
    let changes = resp["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["row_id"].as_str().unwrap(), id);
    assert_eq!(changes[0]["kind"].as_str().unwrap(), "insert");
    let new_cursor = resp["cursor"]["last_seq"].as_u64().unwrap();
    assert!(new_cursor > 0, "cursor must advance past 0");

    // Subsequent pull at advanced cursor returns nothing.
    let (_, resp2) = pull(port, &token, new_cursor);
    assert_eq!(resp2["changes"].as_array().unwrap().len(), 0);
}

// ---------------------------------------------------------------------------
// 3. Server restart with persisted DB: fresh client sees old rows
//    (validates the "seed change log from SQLite on startup" fix)
// ---------------------------------------------------------------------------
#[test]
fn server_restart_still_delivers_prior_rows_to_fresh_clients() {
    let tmpdir = tempfile::tempdir().unwrap();
    let db_path = tmpdir.path().join("sync.db");
    let db_str = db_path.to_str().unwrap();

    // First server lifetime: insert 3 rows, record cursor.
    let ids: Vec<String> = {
        let rt = Arc::new(Runtime::open(db_str, test_manifest()).unwrap());
        let port = start_server(rt);
        let token = mint_guest(port);
        let a = insert_note(port, &token, "one");
        let b = insert_note(port, &token, "two");
        let c = insert_note(port, &token, "three");
        vec![a, b, c]
    };
    // Let the first server's thread finish writes to disk. There's no
    // clean shutdown API; the runtime Arc goes out of scope, the HTTP
    // thread leaks, but SQLite writes are already committed.
    std::thread::sleep(Duration::from_millis(200));

    // Second lifetime: reopen the DB file. A fresh client should pull
    // all 3 rows via the seeded change log.
    let rt2 = Arc::new(Runtime::open(db_str, test_manifest()).unwrap());
    let port2 = start_server(rt2);
    let token2 = mint_guest(port2);
    let (status, resp) = pull(port2, &token2, 0);
    assert_eq!(status, 200);
    let changes = resp["changes"].as_array().unwrap();
    assert_eq!(
        changes.len(),
        3,
        "fresh pull after restart must surface all seeded rows; got: {:?}",
        changes
    );
    let got_ids: Vec<String> = changes
        .iter()
        .map(|c| c["row_id"].as_str().unwrap().to_string())
        .collect();
    for id in &ids {
        assert!(got_ids.contains(id), "missing {id}");
    }
}

// ---------------------------------------------------------------------------
// 4. Stale cursor from a previous server lifetime: get 410 (not silent empty)
// ---------------------------------------------------------------------------
#[test]
fn cursor_from_previous_lifetime_forces_resync() {
    let tmpdir = tempfile::tempdir().unwrap();
    let db_path = tmpdir.path().join("sync.db");
    let db_str = db_path.to_str().unwrap();

    // First lifetime: capture a cursor after some inserts.
    let first_cursor: u64 = {
        let rt = Arc::new(Runtime::open(db_str, test_manifest()).unwrap());
        let port = start_server(rt);
        let token = mint_guest(port);
        for i in 0..5 {
            insert_note(port, &token, &format!("row{i}"));
        }
        let (_, resp) = pull(port, &token, 0);
        resp["cursor"]["last_seq"].as_u64().unwrap()
    };
    assert!(first_cursor >= 5);
    std::thread::sleep(Duration::from_millis(200));

    // Second lifetime: the restart seeds `first_cursor` events again, so
    // the new seq counter also reaches that value. To force the "cursor
    // from an older lifetime" case we present a cursor beyond it.
    let rt2 = Arc::new(Runtime::open(db_str, test_manifest()).unwrap());
    let port2 = start_server(rt2);
    let token2 = mint_guest(port2);

    let stale = first_cursor + 1_000_000;
    let (status, resp) = pull(port2, &token2, stale);
    assert_eq!(
        status, 410,
        "stale cursor must force 410 RESYNC_REQUIRED, got {status}: {resp:?}"
    );
    assert_eq!(resp["error"]["code"].as_str().unwrap(), "RESYNC_REQUIRED");
}

// ---------------------------------------------------------------------------
// 5b. Session tokens survive server restart when runtime is file-backed.
//     Regression: SessionStore was in-memory by default; every dev-server
//     restart invalidated every browser token even though the app DB
//     carried on unchanged. Now persistence is automatic unless
//     PYLON_SESSION_IN_MEMORY=1.
// ---------------------------------------------------------------------------
#[test]
fn sessions_survive_server_restart_by_default() {
    let tmpdir = tempfile::tempdir().unwrap();
    let db_path = tmpdir.path().join("sync.db");
    let db_str = db_path.to_str().unwrap();
    // Make sure no stray env var forces us into the opt-out path.
    // Safety: tests run in-process; no other thread reads this var
    // between here and the server spawn.
    unsafe {
        std::env::remove_var("PYLON_SESSION_IN_MEMORY");
        std::env::remove_var("PYLON_SESSION_DB");
    }

    // First lifetime: mint a guest, confirm it resolves.
    let token = {
        let rt = Arc::new(Runtime::open(db_str, test_manifest()).unwrap());
        let port = start_server(rt);
        let t = mint_guest(port);
        let (s, body) = http(port, "GET", "/api/auth/me", Some(&t), None);
        assert_eq!(s, 200);
        let me: Value = serde_json::from_str(&body).unwrap();
        assert!(me["user_id"].is_string(), "guest session should resolve");
        t
    };
    std::thread::sleep(Duration::from_millis(200));

    // Second lifetime: reopen the same DB. The old token must still resolve
    // via the sibling sessions file created on first boot.
    let rt2 = Arc::new(Runtime::open(db_str, test_manifest()).unwrap());
    let port2 = start_server(rt2);
    let (s, body) = http(port2, "GET", "/api/auth/me", Some(&token), None);
    assert_eq!(s, 200);
    let me: Value = serde_json::from_str(&body).unwrap();
    assert!(
        me["user_id"].is_string(),
        "token minted under previous lifetime must still resolve: {body}"
    );
}

// ---------------------------------------------------------------------------
// 6. Cursor advances on empty (policy-filtered) pulls instead of sticking
// ---------------------------------------------------------------------------
#[test]
fn cursor_advances_even_when_response_is_empty() {
    let rt = Arc::new(Runtime::in_memory(test_manifest()).unwrap());
    let port = start_server(rt);
    let token = mint_guest(port);

    // Insert three rows and drain them.
    insert_note(port, &token, "a");
    insert_note(port, &token, "b");
    insert_note(port, &token, "c");
    let (_, resp) = pull(port, &token, 0);
    let cur = resp["cursor"]["last_seq"].as_u64().unwrap();
    assert!(cur >= 3);

    // Second pull at the advanced cursor: no new events, cursor should
    // echo back the same value (not 0). Clients rely on this — the
    // previous `changes.length > 0`-gated cursor assignment was the bug.
    let (_, resp2) = pull(port, &token, cur);
    assert_eq!(resp2["changes"].as_array().unwrap().len(), 0);
    assert_eq!(resp2["cursor"]["last_seq"].as_u64().unwrap(), cur);
    assert_eq!(resp2["has_more"].as_bool().unwrap(), false);
}
