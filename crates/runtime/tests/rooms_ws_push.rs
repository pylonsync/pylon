//! End-to-end tests for the WebSocket-push presence protocol.
//!
//! Replaces the 5s `/api/rooms/<room>` polling loop the SDK used to run.
//! Two clients subscribe over WS; when one joins the room via HTTP, the
//! other receives a `room-update` action:join push without polling.
//!
//! Covered scenarios:
//!   1. Two WS clients subscribe → A joins → B receives room-update
//!      action:join with A's member info.
//!   2. Subscribing to a room you're not in (non-admin) returns
//!      `{ "type": "error", "code": "NOT_IN_ROOM" }` and registers
//!      no subscription.
//!   3. WS disconnect → presence updates fire room-update action:leave
//!      to remaining subscribers.
//!   4. room-snapshot returns the current member list on subscribe.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pylon_kernel::{AppManifest, ManifestEntity, ManifestField};
use pylon_runtime::Runtime;
use tungstenite::client::IntoClientRequest;
use tungstenite::{client, Message};

const TEST_ADMIN_TOKEN: &str = "testadmin_rooms_ws";

fn test_manifest() -> AppManifest {
    AppManifest {
        manifest_version: 1,
        name: "rooms-ws".into(),
        version: "0.1.0".into(),
        entities: vec![ManifestEntity {
            name: "Todo".into(),
            fields: vec![ManifestField {
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
            }],
            indexes: vec![],
            relations: vec![],
            search: None,
            crdt: true,
        }],
        routes: vec![],
        queries: vec![],
        actions: vec![],
        policies: vec![],
        auth: Default::default(),
        llm: Default::default(),
        connections: vec![],
        crons: vec![],
        fonts: vec![],
    }
}

fn available_port() -> u16 {
    static NEXT: AtomicU16 = AtomicU16::new(45_000);
    for _ in 0..200 {
        let base = NEXT.fetch_add(4, Ordering::Relaxed);
        let ok = (0..4)
            .all(|off| std::net::TcpListener::bind(format!("127.0.0.1:{}", base + off)).is_ok());
        if ok {
            return base;
        }
    }
    panic!("no free 4-port block");
}

fn start_server() -> (u16, Arc<Runtime>) {
    static INIT_ADMIN: std::sync::Once = std::sync::Once::new();
    INIT_ADMIN.call_once(|| {
        // Safety: called before any server thread is spawned. The token
        // value is constant; no concurrent reader can observe a partial
        // write.
        unsafe {
            std::env::set_var("PYLON_ADMIN_TOKEN", TEST_ADMIN_TOKEN);
            std::env::set_var("PYLON_DEV_MODE", "1");
        }
    });

    let port = available_port();
    let manifest = test_manifest();
    let rt = Arc::new(Runtime::in_memory(manifest).unwrap());
    let rt2 = Arc::clone(&rt);
    std::thread::spawn(move || {
        let _ = pylon_runtime::server::start(rt2, port);
    });

    // Wait for HTTP port.
    for _ in 0..100 {
        if TcpStream::connect(format!("127.0.0.1:{port}")).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // Wait for WS port (HTTP server uses port; dedicated WS is port+1).
    for _ in 0..100 {
        if TcpStream::connect(format!("127.0.0.1:{}", port + 1)).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    (port, rt)
}

fn http_request_with_auth(
    method: &str,
    url: &str,
    body: Option<&str>,
    token: Option<&str>,
) -> (u16, String) {
    let host = url.strip_prefix("http://").unwrap_or(url);
    let (host_port, path) = match host.find('/') {
        Some(i) => (&host[..i], &host[i..]),
        None => (host, "/"),
    };
    let body_str = body.unwrap_or("");
    let auth_header = token
        .map(|t| format!("Authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host_port}\r\nOrigin: http://{host_port}\r\n{auth_header}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body_str}",
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

/// Open an authenticated WS connection to the dedicated WS port.
///
/// Uses the admin token via the `bearer.<token>` subprotocol path —
/// matches how browsers authenticate the WS upgrade. The admin token
/// gates the connection past `WsAuth::resolve_bearer_token`.
fn connect_ws_admin(
    port: u16,
) -> tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>> {
    let url = format!("ws://127.0.0.1:{}/", port + 1);
    let mut req = url.into_client_request().expect("ws request");
    req.headers_mut().insert(
        "Authorization",
        format!("Bearer {TEST_ADMIN_TOKEN}").parse().unwrap(),
    );
    let (ws, _resp) = client::connect(req).expect("ws connect");
    if let tungstenite::stream::MaybeTlsStream::Plain(ref s) = ws.get_ref() {
        s.set_read_timeout(Some(Duration::from_millis(800))).ok();
        s.set_nodelay(true).ok();
    }
    ws
}

/// Read frames until one passes `pred` or the deadline elapses. Skips
/// frames that don't match — the WS firehose also carries legacy
/// `broadcast_presence` frames, session-changed pings, etc, so the test
/// needs to filter.
fn wait_for_frame<F>(
    ws: &mut tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>,
    deadline: Duration,
    mut pred: F,
) -> Option<serde_json::Value>
where
    F: FnMut(&serde_json::Value) -> bool,
{
    let start = Instant::now();
    while start.elapsed() < deadline {
        match ws.read() {
            Ok(Message::Text(text)) => {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                    if pred(&v) {
                        return Some(v);
                    }
                }
            }
            Ok(Message::Ping(d)) => {
                let _ = ws.send(Message::Pong(d));
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(io_err))
                if io_err.kind() == std::io::ErrorKind::WouldBlock
                    || io_err.kind() == std::io::ErrorKind::TimedOut =>
            {
                // Spin past the read timeout — total budget is enforced
                // by the outer `deadline`.
                continue;
            }
            Err(_) => return None,
        }
    }
    None
}

/// Scenario 1 + 4: two clients subscribe → A joins → B receives a
/// `room-update` action:join; both got a `room-snapshot` first.
#[test]
fn subscribe_and_push_on_join() {
    let (port, _rt) = start_server();
    let base = format!("http://127.0.0.1:{port}");
    let room = "channel:room1";

    // Pre-create the room with a third user so subscribers (admin) have
    // something to snapshot AND so non-admin membership tests below
    // have a real room to work with.
    let (status, body) = http_request_with_auth(
        "POST",
        &format!("{base}/api/rooms/join"),
        Some(&format!(
            r#"{{"room":"{room}","user_id":"seed","data":{{"role":"observer"}}}}"#
        )),
        Some(TEST_ADMIN_TOKEN),
    );
    assert_eq!(status, 200, "seed join: {body}");

    // Both clients connect with admin auth — admin bypasses the
    // membership gate on subscribe, so neither needs to join the room
    // first. This isolates the WS push from the membership check
    // (covered in scenario 2 below).
    let mut client_a = connect_ws_admin(port);
    let mut client_b = connect_ws_admin(port);

    // Subscribe both to the room. The server pushes a `room-snapshot`
    // immediately with the current member list (just "seed" so far).
    client_a
        .send(Message::Text(
            serde_json::json!({"type":"room-subscribe","room":room}).to_string(),
        ))
        .unwrap();
    client_b
        .send(Message::Text(
            serde_json::json!({"type":"room-subscribe","room":room}).to_string(),
        ))
        .unwrap();

    let snap_b = wait_for_frame(&mut client_b, Duration::from_secs(2), |v| {
        v.get("type").and_then(|t| t.as_str()) == Some("room-snapshot")
            && v.get("room").and_then(|r| r.as_str()) == Some(room)
    });
    let snap_b = snap_b.expect("client B must receive room-snapshot on subscribe");
    let members = snap_b["members"]
        .as_array()
        .expect("snapshot members array");
    assert_eq!(members.len(), 1, "snapshot has the seed member: {snap_b}");
    assert_eq!(
        members[0]["user_id"].as_str(),
        Some("seed"),
        "snapshot member is seed: {snap_b}"
    );

    // Also drain client A's snapshot so the next wait_for_frame on A
    // doesn't accidentally match it.
    let _ = wait_for_frame(&mut client_a, Duration::from_secs(2), |v| {
        v.get("type").and_then(|t| t.as_str()) == Some("room-snapshot")
    });

    // Now ALICE joins the room via HTTP. The notifier fans the
    // `room-update` to subscribers — including client B.
    let (status, body) = http_request_with_auth(
        "POST",
        &format!("{base}/api/rooms/join"),
        Some(&format!(
            r#"{{"room":"{room}","user_id":"alice","data":{{"color":"red"}}}}"#
        )),
        Some(TEST_ADMIN_TOKEN),
    );
    assert_eq!(status, 200, "alice join: {body}");

    let push_b = wait_for_frame(&mut client_b, Duration::from_secs(3), |v| {
        v.get("type").and_then(|t| t.as_str()) == Some("room-update")
            && v.get("action").and_then(|a| a.as_str()) == Some("join")
            && v.get("room").and_then(|r| r.as_str()) == Some(room)
    });
    let push_b = push_b.expect("client B must receive room-update action:join");
    assert_eq!(
        push_b["member"]["user_id"].as_str(),
        Some("alice"),
        "push carries alice's user_id: {push_b}"
    );
    assert_eq!(
        push_b["member"]["data"]["color"].as_str(),
        Some("red"),
        "push carries alice's join data: {push_b}"
    );

    let _ = client_a.close(None);
    let _ = client_b.close(None);
}

/// Scenario 2: a non-admin user subscribes to a room they didn't join.
/// Server replies with `{"type":"error","code":"NOT_IN_ROOM"}` and
/// records no subscription, so they never see future pushes.
#[test]
fn subscribe_rejects_non_member() {
    let (port, _rt) = start_server();
    let base = format!("http://127.0.0.1:{port}");
    let room = "channel:private";

    // Create a session for a real user (non-admin). The simplest path
    // is `/api/auth/guest` which mints a guest session token.
    let (status, body) =
        http_request_with_auth("POST", &format!("{base}/api/auth/guest"), Some("{}"), None);
    assert_eq!(status, 201, "guest auth: {body}");
    let resp: serde_json::Value = serde_json::from_str(&body).unwrap();
    let token = resp["token"]
        .as_str()
        .expect("guest response carries token")
        .to_string();

    // Connect as the guest. Use bearer.<token> subprotocol to mirror
    // browser auth path (Authorization on WS isn't supported in
    // browsers, so the SDK encodes it as a subprotocol — exercising
    // the same path catches regressions in the subprotocol decode).
    let url = format!("ws://127.0.0.1:{}/", port + 1);
    let mut req = url.into_client_request().expect("ws request");
    req.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        format!("bearer.{token}").parse().unwrap(),
    );
    let (mut ws, _resp) = client::connect(req).expect("ws connect");
    if let tungstenite::stream::MaybeTlsStream::Plain(ref s) = ws.get_ref() {
        s.set_read_timeout(Some(Duration::from_millis(800))).ok();
    }

    // Subscribe to a room the user never joined.
    ws.send(Message::Text(
        serde_json::json!({"type":"room-subscribe","room":room}).to_string(),
    ))
    .unwrap();

    let err = wait_for_frame(&mut ws, Duration::from_secs(2), |v| {
        v.get("type").and_then(|t| t.as_str()) == Some("error")
    });
    let err = err.expect("non-member subscribe must produce error frame");
    assert_eq!(
        err["code"].as_str(),
        Some("NOT_IN_ROOM"),
        "error code is NOT_IN_ROOM: {err}"
    );
    assert_eq!(
        err["room"].as_str(),
        Some(room),
        "error carries the room name: {err}"
    );

    // Even if someone joins the room afterwards, this guest must not
    // receive a push (no subscription registered).
    let (status, body) = http_request_with_auth(
        "POST",
        &format!("{base}/api/rooms/join"),
        Some(&format!(r#"{{"room":"{room}","user_id":"someone_else"}}"#)),
        Some(TEST_ADMIN_TOKEN),
    );
    assert_eq!(status, 200, "join: {body}");

    let leaked = wait_for_frame(&mut ws, Duration::from_secs(1), |v| {
        v.get("type").and_then(|t| t.as_str()) == Some("room-update")
            && v.get("room").and_then(|r| r.as_str()) == Some(room)
    });
    assert!(
        leaked.is_none(),
        "non-member must not receive room-update for a room they were denied: got {leaked:?}"
    );

    let _ = ws.close(None);
}

/// Scenario 3: WS disconnect triggers `room-update` action:leave to
/// remaining subscribers of every room the user was in.
///
/// Without this, a client that drops without calling /api/rooms/leave
/// would leave stale presence entries until the 2-minute idle sweep,
/// and other subscribers would only see the leave on that same timer.
#[test]
fn disconnect_fires_room_update_leave() {
    let (port, _rt) = start_server();
    let base = format!("http://127.0.0.1:{port}");
    let room = "channel:leave-test";

    // Mint a guest user; they'll connect, join, then drop.
    let (status, body) =
        http_request_with_auth("POST", &format!("{base}/api/auth/guest"), Some("{}"), None);
    assert_eq!(status, 201, "guest auth: {body}");
    let resp: serde_json::Value = serde_json::from_str(&body).unwrap();
    let guest_token = resp["token"].as_str().expect("guest token").to_string();
    let guest_user_id = resp["user_id"].as_str().expect("guest user_id").to_string();

    // Have the guest join the room (via HTTP — the WS push is the
    // result, not the input).
    let (status, body) = http_request_with_auth(
        "POST",
        &format!("{base}/api/rooms/join"),
        Some(&format!(r#"{{"room":"{room}"}}"#)),
        Some(&guest_token),
    );
    assert_eq!(status, 200, "guest join: {body}");

    // Watcher: admin connection subscribed to the room, expects to
    // receive the leave push when the guest's WS drops.
    let mut watcher = connect_ws_admin(port);
    watcher
        .send(Message::Text(
            serde_json::json!({"type":"room-subscribe","room":room}).to_string(),
        ))
        .unwrap();
    let _ = wait_for_frame(&mut watcher, Duration::from_secs(2), |v| {
        v.get("type").and_then(|t| t.as_str()) == Some("room-snapshot")
    });

    // Open the guest's WS. The connection's auth context carries the
    // guest's user_id; on close, fanout_room_leaves_on_disconnect
    // calls RoomManager::disconnect with that user_id.
    let url = format!("ws://127.0.0.1:{}/", port + 1);
    let mut req = url.into_client_request().expect("ws request");
    req.headers_mut().insert(
        "Authorization",
        format!("Bearer {guest_token}").parse().unwrap(),
    );
    let (mut guest_ws, _) = client::connect(req).expect("guest ws connect");
    if let tungstenite::stream::MaybeTlsStream::Plain(ref s) = guest_ws.get_ref() {
        s.set_read_timeout(Some(Duration::from_millis(500))).ok();
    }

    // Give the server a moment to register the guest's connection.
    std::thread::sleep(Duration::from_millis(200));

    // Drop the guest's WS abruptly — this is the path the cleanup
    // must handle (close frame, then read loop's Close arm fires
    // fanout_room_leaves_on_disconnect).
    let _ = guest_ws.close(None);
    // Read a couple frames so the close handshake completes.
    for _ in 0..3 {
        if guest_ws.read().is_err() {
            break;
        }
    }
    drop(guest_ws);

    // Watcher should observe the leave.
    let leave = wait_for_frame(&mut watcher, Duration::from_secs(3), |v| {
        v.get("type").and_then(|t| t.as_str()) == Some("room-update")
            && v.get("action").and_then(|a| a.as_str()) == Some("leave")
            && v.get("room").and_then(|r| r.as_str()) == Some(room)
    });
    let leave = leave.expect("watcher must receive room-update action:leave on guest disconnect");
    assert_eq!(
        leave["member"]["user_id"].as_str(),
        Some(guest_user_id.as_str()),
        "leave push carries the disconnected guest's user_id: {leave}"
    );

    let _ = watcher.close(None);
}
