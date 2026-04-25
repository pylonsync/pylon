//! Integration tests for the pylon HTTP server.
//!
//! Each test starts a real in-memory server on a random port and exercises
//! the API over plain HTTP using a minimal `TcpStream`-based client.
//! No external dependencies beyond what the runtime crate already exposes.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use pylon_kernel::*;
use pylon_runtime::Runtime;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// Minimal HTTP/1.1 client for testing. Returns `(status_code, body)`.
///
/// Sends a single request with `Connection: close` and reads until EOF.
/// Deliberately simple -- no redirect following, no chunked decoding.
fn http_request(method: &str, url: &str, body: Option<&str>) -> (u16, String) {
    http_request_with_auth(method, url, body, None)
}

/// Test admin token. `start_test_server` sets `PYLON_ADMIN_TOKEN` to this
/// value the first time it's called, so passing `Some(TEST_ADMIN_TOKEN)` to
/// `http_request_with_auth` unlocks admin-only endpoints like /api/batch,
/// /api/transact, and /api/import.
const TEST_ADMIN_TOKEN: &str = "testadmin_integration";

fn http_request_with_auth(
    method: &str,
    url: &str,
    body: Option<&str>,
    auth: Option<&str>,
) -> (u16, String) {
    let host = url.strip_prefix("http://").unwrap_or(url);
    let (host_port, path) = match host.find('/') {
        Some(i) => (&host[..i], &host[i..]),
        None => (host, "/"),
    };

    let body_str = body.unwrap_or("");
    let content_length = body_str.len();
    let auth_header = match auth {
        Some(token) => format!("Authorization: Bearer {token}\r\n"),
        None => String::new(),
    };
    // Origin is mandatory on state-changing methods — the CSRF plugin
    // rejects POST/PATCH/DELETE without it. Dev mode's allowlist is `*`,
    // so echoing the host back always passes.
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host_port}\r\nOrigin: http://{host_port}\r\nContent-Type: application/json\r\nContent-Length: {content_length}\r\n{auth_header}Connection: close\r\n\r\n{body_str}"
    );

    let mut stream = TcpStream::connect(host_port).expect("Failed to connect");
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    stream
        .write_all(request.as_bytes())
        .expect("Failed to write request");

    let mut response = String::new();
    stream.read_to_string(&mut response).ok();

    // Parse status code from the first line: "HTTP/1.1 200 OK"
    let status: u16 = response
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    // Body is everything after the first blank line (\r\n\r\n).
    let body = response.split("\r\n\r\n").nth(1).unwrap_or("").to_string();

    (status, body)
}

/// Build a test manifest with `Todo` and `User` entities.
fn test_manifest() -> AppManifest {
    AppManifest {
        manifest_version: 1,
        name: "integration-test".into(),
        version: "0.1.0".into(),
        entities: vec![
            ManifestEntity {
                name: "Todo".into(),
                fields: vec![
                    ManifestField {
                        name: "title".into(),
                        field_type: "string".into(),
                        optional: false,
                        unique: false,
                        crdt: None,
                    },
                    ManifestField {
                        name: "done".into(),
                        field_type: "bool".into(),
                        optional: false,
                        unique: false,
                        crdt: None,
                    },
                ],
                indexes: vec![],
                relations: vec![],
                search: None,
                            crdt: true,
            },
            ManifestEntity {
                name: "User".into(),
                fields: vec![
                    ManifestField {
                        name: "email".into(),
                        field_type: "string".into(),
                        optional: false,
                        unique: true,
                        crdt: None,
                    },
                    ManifestField {
                        name: "displayName".into(),
                        field_type: "string".into(),
                        optional: false,
                        unique: false,
                        crdt: None,
                    },
                ],
                indexes: vec![],
                relations: vec![],
                search: None,
                            crdt: true,
            },
        ],
        routes: vec![],
        queries: vec![],
        actions: vec![],
        policies: vec![],
    }
}

/// Find 4 contiguous available TCP ports (server uses port, port+1 WS,
/// port+2 SSE, port+3 shard-WS).
///
/// To avoid TOCTOU races between tests picking the same "free" port, we:
/// 1. Walk forward from a per-process random offset in the ephemeral range.
/// 2. Use a process-wide atomic counter so parallel tests never ask for
///    the same base port.
/// 3. Trial-bind all 4 ports in one shot; if any fails, advance and retry.
///
/// This is bulletproof as long as no other process on the box is also
/// walking the same range, which is fine for `cargo test` on a dev box or
/// in CI.
fn available_port() -> u16 {
    use std::sync::atomic::{AtomicU16, Ordering};
    // Start high in the ephemeral range with a random offset so two parallel
    // `cargo test` invocations (e.g. one local, one triggered by a watcher)
    // don't stomp each other immediately.
    static NEXT_PORT: AtomicU16 = AtomicU16::new(0);
    // Initialize on first use with a random offset in [30000, 40000).
    let seed = NEXT_PORT.load(Ordering::Relaxed);
    if seed == 0 {
        let off: u16 = rand::random::<u16>() % 10_000 + 30_000;
        let _ = NEXT_PORT.compare_exchange(0, off, Ordering::Relaxed, Ordering::Relaxed);
    }

    for _ in 0..200 {
        // Allocate a 4-port block per test (WS is port+1, SSE port+2, shard-WS port+3).
        let base = NEXT_PORT.fetch_add(4, Ordering::Relaxed);
        if base == 0 {
            continue;
        }
        if let Some(p) = try_bind_block(base) {
            return p;
        }
    }
    panic!("could not find 4 free contiguous ports after 200 attempts");
}

/// Try to bind 4 contiguous ports starting at `base`. Returns the base port
/// if all bind, or None otherwise. All listeners are dropped before returning
/// so the actual server can bind immediately afterwards.
fn try_bind_block(base: u16) -> Option<u16> {
    let mut listeners = Vec::with_capacity(4);
    for offset in 0..4 {
        match std::net::TcpListener::bind(format!("127.0.0.1:{}", base + offset)) {
            Ok(l) => listeners.push(l),
            Err(_) => return None,
        }
    }
    drop(listeners);
    Some(base)
}

/// Start a test server in a background thread. Returns the base URL
/// (e.g. `http://127.0.0.1:54321`).
///
/// Blocks until the server is accepting connections (up to 2.5 s).
fn start_test_server() -> String {
    // All tests share the same admin token so they can exercise admin-only
    // endpoints (batch, transact, import). Setting this once is safe across
    // threads — same value, never mutated — but we funnel through `Once` so
    // it's clear a single test run only writes the env var once.
    static INIT_ADMIN: std::sync::Once = std::sync::Once::new();
    INIT_ADMIN.call_once(|| {
        // Safety: called before any server thread is spawned. No other
        // thread can be reading `PYLON_ADMIN_TOKEN` concurrently.
        unsafe {
            std::env::set_var("PYLON_ADMIN_TOKEN", TEST_ADMIN_TOKEN);
        }
    });

    let port = available_port();
    let manifest = test_manifest();
    let runtime = Arc::new(Runtime::in_memory(manifest).unwrap());

    let rt = Arc::clone(&runtime);
    std::thread::spawn(move || {
        let _ = pylon_runtime::server::start(rt, port);
    });

    // Poll until the server is accepting connections.
    let base = format!("http://127.0.0.1:{port}");
    for _ in 0..50 {
        if TcpStream::connect(format!("127.0.0.1:{port}")).is_ok() {
            return base;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("Server failed to start on port {port}");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn crud_lifecycle() {
    let base = start_test_server();

    // INSERT
    let (status, body) = http_request(
        "POST",
        &format!("{base}/api/entities/Todo"),
        Some(r#"{"title": "Buy milk", "done": false}"#),
    );
    assert_eq!(status, 201, "INSERT should return 201: {body}");
    let resp: serde_json::Value = serde_json::from_str(&body).unwrap();
    let id = resp["id"]
        .as_str()
        .expect("response must contain id")
        .to_string();

    // GET
    let (status, body) = http_request("GET", &format!("{base}/api/entities/Todo/{id}"), None);
    assert_eq!(status, 200, "GET should return 200: {body}");
    let row: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(row["title"], "Buy milk");

    // UPDATE (PATCH)
    let (status, body) = http_request(
        "PATCH",
        &format!("{base}/api/entities/Todo/{id}"),
        Some(r#"{"done": true}"#),
    );
    assert_eq!(status, 200, "PATCH should return 200: {body}");

    // Verify the update
    let (_, body) = http_request("GET", &format!("{base}/api/entities/Todo/{id}"), None);
    let row: serde_json::Value = serde_json::from_str(&body).unwrap();
    // SQLite stores booleans as integers; `true` becomes `1`.
    assert_eq!(row["done"], 1);

    // DELETE
    let (status, _) = http_request("DELETE", &format!("{base}/api/entities/Todo/{id}"), None);
    assert_eq!(status, 200, "DELETE should return 200");

    // Verify deletion
    let (status, _) = http_request("GET", &format!("{base}/api/entities/Todo/{id}"), None);
    assert_eq!(status, 404, "GET after DELETE should return 404");
}

#[test]
fn list_and_pagination() {
    let base = start_test_server();

    // Insert 5 todos.
    for i in 0..5 {
        let (status, _) = http_request(
            "POST",
            &format!("{base}/api/entities/Todo"),
            Some(&format!(r#"{{"title": "Todo {i}", "done": false}}"#)),
        );
        assert_eq!(status, 201, "insert {i} failed");
    }

    // List all (no pagination params).
    let (status, body) = http_request("GET", &format!("{base}/api/entities/Todo"), None);
    assert_eq!(status, 200);
    let resp: serde_json::Value = serde_json::from_str(&body).unwrap();
    // Response shape is {data, count, offset, limit} — `count` is "rows
    // returned" not "total in table"; cursor pagination at
    // /api/entities/:e/cursor is the right path for total counts.
    assert_eq!(resp["count"], 5);

    // Offset/limit pagination.
    let (_, body) = http_request(
        "GET",
        &format!("{base}/api/entities/Todo?limit=2&offset=0"),
        None,
    );
    let resp: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(resp["data"].as_array().unwrap().len(), 2);

    // Cursor-based pagination.
    let (status, body) = http_request(
        "GET",
        &format!("{base}/api/entities/Todo/cursor?limit=2"),
        None,
    );
    assert_eq!(status, 200);
    let resp: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(resp["data"].as_array().unwrap().len(), 2);
    assert!(resp["has_more"].as_bool().unwrap());
}

#[test]
fn auth_session_flow() {
    let base = start_test_server();

    // Create a named session.
    let (status, body) = http_request(
        "POST",
        &format!("{base}/api/auth/session"),
        Some(r#"{"user_id": "user-1"}"#),
    );
    assert!(
        status == 200 || status == 201,
        "session create: status={status} body={body}"
    );
    let resp: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(resp["token"].as_str().is_some(), "token missing");

    // Create a guest session.
    let (status, body) = http_request("POST", &format!("{base}/api/auth/guest"), None);
    assert!(
        status == 200 || status == 201,
        "guest session: status={status} body={body}"
    );
    let resp: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(resp["guest"].as_bool().unwrap());
}

#[test]
fn health_and_metrics() {
    let base = start_test_server();

    // Health check.
    let (status, body) = http_request("GET", &format!("{base}/health"), None);
    assert_eq!(status, 200, "health: {body}");
    let resp: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(resp["status"], "ok");

    // Make a few requests so the metrics counter is non-zero.
    http_request("GET", &format!("{base}/health"), None);
    http_request("GET", &format!("{base}/metrics"), None);

    let (status, body) = http_request("GET", &format!("{base}/metrics"), None);
    assert_eq!(status, 200, "metrics: {body}");
    let resp: serde_json::Value = serde_json::from_str(&body).unwrap();
    // The metrics snapshot must record at least the requests we just made.
    assert!(
        resp["requests"]["total"].as_u64().unwrap_or(0) > 0,
        "expected non-zero total requests"
    );
}

#[test]
fn error_handling() {
    let base = start_test_server();

    // Unknown entity.
    let (status, _) = http_request("GET", &format!("{base}/api/entities/Nonexistent"), None);
    assert!(
        status == 400 || status == 404,
        "unknown entity should be 400 or 404, got {status}"
    );

    // Invalid JSON body.
    let (status, _) = http_request(
        "POST",
        &format!("{base}/api/entities/Todo"),
        Some("not json"),
    );
    assert_eq!(status, 400, "invalid JSON should be 400");

    // Missing required field (SQLite NOT NULL constraint).
    let (status, _) = http_request(
        "POST",
        &format!("{base}/api/entities/Todo"),
        Some(r#"{"done": false}"#),
    );
    assert_eq!(status, 400, "missing required field should be 400");

    // Non-existent row.
    let (status, _) = http_request(
        "GET",
        &format!("{base}/api/entities/Todo/nonexistent-id"),
        None,
    );
    assert_eq!(status, 404, "non-existent row should be 404");

    // Unknown route.
    let (status, _) = http_request("GET", &format!("{base}/api/doesnotexist"), None);
    assert_eq!(status, 404, "unknown route should be 404");
}

#[test]
fn batch_operations() {
    let base = start_test_server();

    // /api/batch is admin-only — use the test admin token.
    let (status, body) = http_request_with_auth(
        "POST",
        &format!("{base}/api/batch"),
        Some(
            r#"{
                "operations": [
                    {"op": "insert", "entity": "Todo", "data": {"title": "A", "done": false}},
                    {"op": "insert", "entity": "Todo", "data": {"title": "B", "done": false}}
                ]
            }"#,
        ),
        Some(TEST_ADMIN_TOKEN),
    );
    assert_eq!(status, 200, "batch: {body}");
    let resp: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(resp["succeeded"], 2);
}

/// The `/api/transact` endpoint acquires `lock_conn_pub()` and then calls
/// `rt.insert()` which internally calls `lock_conn()` on the same
/// non-reentrant `Mutex`. This deadlocks the request thread, causing the
/// Tests atomic transactions via /api/transact.
/// Previously deadlocked because the handler held the conn lock and called
/// rt.insert() which tried to re-lock. Fixed by using _with_conn variants.
#[test]
fn transaction() {
    let base = start_test_server();

    // /api/transact is admin-only — use the test admin token.
    let (status, body) = http_request_with_auth(
        "POST",
        &format!("{base}/api/transact"),
        Some(
            r#"[
                {"op": "insert", "entity": "Todo", "data": {"title": "TX1", "done": false}},
                {"op": "insert", "entity": "Todo", "data": {"title": "TX2", "done": false}}
            ]"#,
        ),
        Some(TEST_ADMIN_TOKEN),
    );
    assert_eq!(status, 200, "transact: {body}");
    let resp: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(resp["committed"].as_bool().unwrap());
}

#[test]
fn cache_via_http() {
    let base = start_test_server();

    // SET
    let (status, body) = http_request(
        "POST",
        &format!("{base}/api/cache"),
        Some(r#"{"cmd": "SET", "key": "test_key", "value": "hello"}"#),
    );
    assert_eq!(status, 200, "cache SET: {body}");

    // GET
    let (status, body) = http_request("GET", &format!("{base}/api/cache/test_key"), None);
    assert_eq!(status, 200, "cache GET: {body}");
    let resp: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(resp["result"], "hello");

    // DEL
    let (status, _) = http_request("DELETE", &format!("{base}/api/cache/test_key"), None);
    assert_eq!(status, 200, "cache DEL should succeed");

    // GET after DEL (miss).
    let (status, _) = http_request("GET", &format!("{base}/api/cache/test_key"), None);
    assert_eq!(status, 404, "cache GET after DEL should be 404");
}

#[test]
fn rooms_via_http() {
    let base = start_test_server();

    // Rooms are auth-gated — admin token works the same as a user session
    // for this endpoint and avoids a separate guest-auth round-trip.
    let (status, body) = http_request_with_auth(
        "POST",
        &format!("{base}/api/rooms/join"),
        Some(r#"{"room": "lobby", "user_id": "alice"}"#),
        Some(TEST_ADMIN_TOKEN),
    );
    assert_eq!(status, 200, "join: {body}");
    let resp: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(resp["joined"], "lobby");

    // List rooms.
    let (status, body) = http_request_with_auth(
        "GET",
        &format!("{base}/api/rooms"),
        None,
        Some(TEST_ADMIN_TOKEN),
    );
    assert_eq!(status, 200, "list rooms: {body}");

    // Leave the room.
    let (status, body) = http_request_with_auth(
        "POST",
        &format!("{base}/api/rooms/leave"),
        Some(r#"{"room": "lobby", "user_id": "alice"}"#),
        Some(TEST_ADMIN_TOKEN),
    );
    assert_eq!(status, 200, "leave: {body}");
}

#[test]
fn openapi_spec() {
    let base = start_test_server();

    let (status, body) = http_request("GET", &format!("{base}/api/openapi.json"), None);
    assert_eq!(status, 200, "openapi: {body}");
    let spec: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(spec["openapi"], "3.0.3");
    assert!(
        spec["paths"].as_object().unwrap().len() > 10,
        "expected >10 path entries in OpenAPI spec"
    );
}

#[test]
fn sync_pull() {
    let base = start_test_server();

    // Insert some data so the change log is non-empty.
    http_request(
        "POST",
        &format!("{base}/api/entities/Todo"),
        Some(r#"{"title": "Sync test", "done": false}"#),
    );

    // Pull changes since sequence 0.
    let (status, body) = http_request("GET", &format!("{base}/api/sync/pull?since=0"), None);
    assert_eq!(status, 200, "sync pull: {body}");
    let resp: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        !resp["changes"].as_array().unwrap().is_empty(),
        "expected at least one change"
    );
}

#[test]
fn cors_headers_present() {
    let base = start_test_server();

    // Make a raw request and inspect response headers.
    let host = base.strip_prefix("http://").unwrap();
    let mut stream = TcpStream::connect(host).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    write!(
        stream,
        "GET /health HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();

    let mut response = String::new();
    stream.read_to_string(&mut response).ok();

    assert!(
        response.contains("Access-Control-Allow-Origin"),
        "missing CORS header in response:\n{response}"
    );
    assert!(
        response.contains("X-Content-Type-Options: nosniff"),
        "missing nosniff header"
    );
    assert!(
        response.contains("X-Frame-Options: DENY"),
        "missing X-Frame-Options header"
    );
}

#[test]
fn body_size_limit_normal_request_accepted() {
    let base = start_test_server();

    // A normal-sized request should succeed.
    let (status, body) = http_request(
        "POST",
        &format!("{base}/api/entities/Todo"),
        Some(r#"{"title": "Normal sized", "done": false}"#),
    );
    assert_eq!(status, 201, "normal body should be accepted: {body}");
}
