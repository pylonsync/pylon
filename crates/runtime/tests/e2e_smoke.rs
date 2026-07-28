//! End-to-end smoke test across HTTP + WebSocket.
//!
//! Protects the integration seam that unit tests miss: write a row via the
//! HTTP API, and confirm a live WebSocket subscriber receives the matching
//! `ChangeEvent`. Breaks fast if any of {router, notifier, ws hub, auth
//! gate, sync event pipeline} diverges from the rest.
//!
//! Uses the auth-gated /ws added in the pentest round — the bearer token
//! is minted via `/api/auth/guest` so this works without admin config.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::Duration;

use pylon_kernel::{AppManifest, ManifestEntity, ManifestField, ManifestPolicy};
use pylon_runtime::Runtime;
use tungstenite::client::IntoClientRequest;
use tungstenite::{client, Message};

fn test_manifest() -> AppManifest {
    AppManifest {
        manifest_version: 1,
        name: "e2e-ws".into(),
        version: "0.1.0".into(),
        entities: vec![ManifestEntity {
            name: "Todo".into(),
            fields: vec![
                ManifestField {
                    name: "title".into(),
                    field_type: "string".into(),
                    optional: false,
                    unique: false,
                    crdt: None,
                    server_only: false,
                    readonly: false,
                    default: None,
                    enum_values: None,
                    encrypted: false,
                },
                ManifestField {
                    name: "done".into(),
                    field_type: "bool".into(),
                    optional: false,
                    unique: false,
                    crdt: None,
                    server_only: false,
                    readonly: false,
                    default: None,
                    enum_values: None,
                    encrypted: false,
                },
            ],
            indexes: vec![],
            relations: vec![],
            search: None,
            crdt: true,
            sync: true,
            ..Default::default()
        }],
        routes: vec![],
        queries: vec![],
        actions: vec![],
        // `Todo` is intentionally public here: this test exercises the
        // router → change-log → sync-pull seam, not auth. Without an
        // explicit policy the secure `_default_deny` (added with the
        // caller-aware policy work) correctly 403s an anonymous insert,
        // which had silently rotted this test red. `allow: "true"` is the
        // documented opt-in for a genuinely public entity.
        policies: vec![ManifestPolicy {
            name: "public_todo".into(),
            entity: Some("Todo".into()),
            allow: "true".into(),
            ..Default::default()
        }],
        auth: Default::default(),
        llm: Default::default(),
        connections: vec![],
        crons: vec![],
        fonts: vec![],
    }
}

fn available_port() -> u16 {
    static NEXT: AtomicU16 = AtomicU16::new(41_000);
    for _ in 0..200 {
        let base = NEXT.fetch_add(4, Ordering::Relaxed);
        // Confirm the whole 4-port block is free (HTTP + WS + SSE + shardWS).
        let ok = (0..4)
            .all(|off| std::net::TcpListener::bind(format!("127.0.0.1:{}", base + off)).is_ok());
        if ok {
            return base;
        }
    }
    panic!("no free 4-port block");
}

fn start_server() -> (u16, Arc<Runtime>) {
    let port = available_port();
    let manifest = test_manifest();
    // Server defaults to non-dev (production-safe), which requires
    // PYLON_CORS_ORIGIN. Tests don't set one, so opt into dev mode.
    // SAFETY: tests run single-threaded for env mutations like this;
    // setting once before the server thread spawns is fine.
    unsafe {
        std::env::set_var("PYLON_DEV_MODE", "1");
    }
    let rt = Arc::new(Runtime::in_memory(manifest).unwrap());
    let rt2 = Arc::clone(&rt);
    std::thread::spawn(move || {
        let _ = pylon_runtime::server::start(rt2, port);
    });

    // Wait for HTTP socket.
    for _ in 0..100 {
        if TcpStream::connect(format!("127.0.0.1:{port}")).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // Also wait for the WS port (port+1) — the spawn is async.
    for _ in 0..100 {
        if TcpStream::connect(format!("127.0.0.1:{}", port + 1)).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    (port, rt)
}

fn http_request(method: &str, url: &str, body: Option<&str>) -> (u16, String) {
    let host = url.strip_prefix("http://").unwrap_or(url);
    let (host_port, path) = match host.find('/') {
        Some(i) => (&host[..i], &host[i..]),
        None => (host, "/"),
    };
    let body_str = body.unwrap_or("");
    // CSRF plugin: state-changing requests must either have NO
    // origin/referer (server-to-server callers) or a present
    // origin that's in the allowlist. Dev mode allowlists `*`, so
    // `http://<host:port>` always passes. We send Origin
    // explicitly here so the test exercises the validate path
    // rather than the bypass path.
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host_port}\r\nOrigin: http://{host_port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body_str}",
        body_str.len()
    );
    let mut stream = TcpStream::connect(host_port).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    stream.write_all(request.as_bytes()).expect("write");
    let mut response = Vec::new();
    let _ = stream.read_to_end(&mut response);
    let text = String::from_utf8_lossy(&response).to_string();
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

/// Seam check: HTTP insert → sync-pull returns the change. Covers the
/// router policy gate, change log append, and sync API — the core
/// real-time path without WebSocket client quirks.
///
/// A true WebSocket fan-out test is covered by unit tests at the WsHub
/// level (crates/runtime/src/ws.rs) and in the sync crate; the wire-level
/// WS handshake + per-shard client registration is exercised here only
/// via the auth-rejection smoke test below.
#[test]
fn http_insert_appears_in_sync_pull() {
    let (port, _rt) = start_server();
    let base = format!("http://127.0.0.1:{port}");

    let (status, body) = http_request(
        "POST",
        &format!("{base}/api/entities/Todo"),
        Some(r#"{"title": "e2e-smoke", "done": false}"#),
    );
    assert_eq!(status, 201, "insert: {body}");

    let (status, body) = http_request("GET", &format!("{base}/api/sync/pull?since=0"), None);
    assert_eq!(status, 200, "sync pull: {body}");
    let resp: serde_json::Value = serde_json::from_str(&body).unwrap();
    let changes = resp["changes"].as_array().expect("changes array");
    let todo_insert = changes
        .iter()
        .find(|c| c["entity"] == "Todo" && c["kind"] == "insert");
    assert!(
        todo_insert.is_some(),
        "sync pull must surface the Todo insert: {resp}"
    );
}

/// Regression: a MULTIPLEXED WebSocket subscriber (`/api/sync/ws` on
/// the main HTTP port — the path every browser sync engine uses by
/// default) must receive change events IMMEDIATELY, without sending
/// any traffic of its own.
///
/// The bug this pins: the HTTP-upgrade path used tiny_http's fused
/// `upgrade()` stream, which can't be split or given read timeouts —
/// so the per-client outbound queue only drained when the CLIENT sent
/// something (the sync engine's 25 s keepalive ping). Realtime events
/// arrived in 25-second batches; world3d players "teleported" instead
/// of moving. Fixed by the vendored `upgrade_split` + a dedicated
/// writer thread. A quiet subscriber receiving an event within 2 s is
/// exactly what the old code could not do.
#[test]
fn multiplexed_ws_delivers_changes_to_quiet_subscriber() {
    let (port, _rt) = start_server();
    let base = format!("http://127.0.0.1:{port}");

    // Mint a guest session for the WS bearer handshake.
    let (status, body) = http_request("POST", &format!("{base}/api/auth/guest"), Some("{}"));
    assert_eq!(status, 201, "guest: {body}");
    let guest: serde_json::Value = serde_json::from_str(&body).unwrap();
    let token = guest["token"].as_str().expect("guest token");

    // Connect through the MAIN port's HTTP-upgrade path (not :port+1).
    let ws_url = format!("ws://127.0.0.1:{port}/api/sync/ws");
    let mut req = ws_url.into_client_request().expect("client request");
    req.headers_mut().insert(
        "Authorization",
        format!("Bearer {token}").parse().expect("bearer header"),
    );
    let (mut socket, _resp) = client::connect(req).expect("multiplexed ws connect");
    // Bound the read so a broken fan-out fails the test instead of
    // hanging it. 2 s is far below the old 25 s ping-bounded latency.
    match socket.get_ref() {
        tungstenite::stream::MaybeTlsStream::Plain(s) => {
            s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        }
        _ => panic!("expected plain TCP stream"),
    }

    // Write a row over HTTP — the quiet subscriber must see it live.
    let (status, body) = http_request(
        "POST",
        &format!("{base}/api/entities/Todo"),
        Some(r#"{"title": "ws-instant", "done": false}"#),
    );
    assert_eq!(status, 201, "insert: {body}");

    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let mut saw_insert = false;
    while std::time::Instant::now() < deadline {
        match socket.read() {
            Ok(Message::Text(text)) => {
                if text.contains("\"entity\":\"Todo\"") && text.contains("\"kind\":\"insert\"") {
                    saw_insert = true;
                    break;
                }
            }
            Ok(_) => {}
            Err(_) => break, // read timeout — fall through to the assert
        }
    }
    assert!(
        saw_insert,
        "quiet multiplexed WS subscriber must receive the Todo insert within 2s \
         (ping-bounded delivery regression)"
    );
}

/// Defense-in-depth: an unauthenticated WS connection attempt is rejected.
/// Regression test for the pentest fix that added bearer-token auth on /ws.
#[test]
fn ws_rejects_unauthenticated() {
    let (port, _rt) = start_server();
    let ws_url = format!("ws://127.0.0.1:{}/", port + 1);

    // No Authorization header, no subprotocol → unauth. Handshake itself
    // succeeds; the server closes immediately with policy code.
    let req = ws_url.into_client_request().expect("ws request");
    let connect_result = client::connect(req);
    match connect_result {
        Ok((mut ws, _)) => {
            // Reading must return a Close (or connection error) quickly.
            if let tungstenite::stream::MaybeTlsStream::Plain(ref s) = ws.get_ref() {
                s.set_read_timeout(Some(Duration::from_secs(2))).ok();
            }
            match ws.read() {
                Ok(Message::Close(_)) => { /* expected */ }
                Ok(other) => panic!("expected Close, got {other:?}"),
                Err(_) => { /* connection dropped — also acceptable */ }
            }
        }
        Err(_) => {
            // Handshake rejection is also acceptable depending on timing.
        }
    }
}
