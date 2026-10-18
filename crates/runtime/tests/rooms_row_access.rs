//! Rooms tied to a row: `@Entity/rowId` (`useRoom({ entity, id })`).
//!
//! - Only callers who may read the row join, set presence, send messages,
//!   list the room or see its members; a malformed `@` name is refused.
//! - The WebSocket subscription checks the same access.
//! - A member who loses access (here: the row changes owner) is refused on
//!   presence, removed on their next heartbeat, and their socket is told
//!   (`room-revoked`) and stops receiving the room's events.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pylon_kernel::{AppManifest, ManifestEntity, ManifestField, ManifestPolicy};
use pylon_runtime::Runtime;
use tungstenite::client::IntoClientRequest;
use tungstenite::{client, Message};

const TEST_ADMIN_TOKEN: &str = "testadmin_rooms_row";

fn string_field(name: &str) -> ManifestField {
    ManifestField {
        name: name.into(),
        field_type: "string".into(),
        optional: false,
        ..Default::default()
    }
}

fn test_manifest() -> AppManifest {
    AppManifest {
        manifest_version: 1,
        name: "rooms-row".into(),
        version: "0.1.0".into(),
        entities: vec![ManifestEntity {
            name: "Note".into(),
            fields: vec![
                string_field("title"),
                string_field("ownerId"),
                string_field("viewerId"),
            ],
            crdt: true,
            sync: true,
            ..Default::default()
        }],
        policies: vec![ManifestPolicy {
            name: "ownerReads".into(),
            entity: Some("Note".into()),
            allow_read: Some("auth.userId == data.ownerId || auth.userId == data.viewerId".into()),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn available_port() -> u16 {
    // Below the ephemeral range (Linux 32768-60999): a port up there can be
    // handed to one of these tests' OWN client sockets between the probe
    // below and the server's bind, which surfaces as EADDRINUSE on CI.
    // One 1000-port lane per test binary so parallel binaries can't overlap.
    static NEXT: AtomicU16 = AtomicU16::new(26_000);
    for _ in 0..200 {
        let base = NEXT.fetch_add(4, Ordering::Relaxed);
        let ok = (0..4).all(|off| pylon_runtime::listen::port_is_free(base + off));
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
    // The server's own error is the only explanation for a bind
    // failure; dropping it leaves "never bound" with no cause.
    let boot_err: std::sync::Arc<std::sync::Mutex<Option<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let boot_err_thread = std::sync::Arc::clone(&boot_err);
    std::thread::spawn(move || {
        let r = pylon_runtime::server::start(rt2, port);
        if let Err(e) = r {
            *boot_err_thread.lock().unwrap() = Some(e.to_string());
        }
    });

    // Wait for HTTP port.
    // 300 x 50ms = 15s. The old budget was 5s AND fell through
    // silently when it ran out, so a slow CI runner walked into a
    // bare `.expect("connect")` panic further down that looked like a
    // product bug. Fail here instead, naming the port.
    {
        let mut ready = false;
        // Bound the wall clock, not the attempt count: a failed connect is
        // not instant on every platform (see csrf_form_route.rs).
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
    // Wait for WS port (HTTP server uses port; dedicated WS is port+1).
    // 300 x 50ms = 15s. The old budget was 5s AND fell through
    // silently when it ran out, so a slow CI runner walked into a
    // bare `.expect("connect")` panic further down that looked like a
    // product bug. Fail here instead, naming the port.
    {
        let mut ready = false;
        // Bound the wall clock, not the attempt count: a failed connect is
        // not instant on every platform (see csrf_form_route.rs).
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        while std::time::Instant::now() < deadline {
            if TcpStream::connect(format!("127.0.0.1:{}", port + 1)).is_ok() {
                ready = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            ready,
            "test server never bound {} within 15s",
            format!("127.0.0.1:{}", port + 1)
        );
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

/// Mint a guest session. Returns (token, user_id).
fn guest_session(base: &str) -> (String, String) {
    let (status, body) =
        http_request_with_auth("POST", &format!("{base}/api/auth/guest"), Some("{}"), None);
    assert_eq!(status, 201, "guest auth: {body}");
    let resp: serde_json::Value = serde_json::from_str(&body).unwrap();
    (
        resp["token"].as_str().expect("guest token").to_string(),
        resp["user_id"].as_str().expect("guest user_id").to_string(),
    )
}

/// Open a WS connection authenticated with a session token.
fn connect_ws_token(
    port: u16,
    token: &str,
) -> tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>> {
    let url = format!("ws://127.0.0.1:{}/", port + 1);
    let mut req = url.into_client_request().expect("ws request");
    req.headers_mut()
        .insert("Authorization", format!("Bearer {token}").parse().unwrap());
    let (ws, _) = client::connect(req).expect("ws connect");
    if let tungstenite::stream::MaybeTlsStream::Plain(ref s) = ws.get_ref() {
        s.set_read_timeout(Some(Duration::from_millis(500))).ok();
        s.set_nodelay(true).ok();
    }
    ws
}

/// Send `room-subscribe` and wait for the `room-snapshot` that confirms
/// the server recorded the subscription.
fn subscribe_room(
    ws: &mut tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>,
    room: &str,
) {
    ws.send(Message::Text(
        serde_json::json!({"type":"room-subscribe","room":room}).to_string(),
    ))
    .unwrap();
    let snap = wait_for_frame(ws, Duration::from_secs(2), |v| {
        v.get("type").and_then(|t| t.as_str()) == Some("room-snapshot")
            && v.get("room").and_then(|r| r.as_str()) == Some(room)
    });
    assert!(snap.is_some(), "subscribe to {room} must return a snapshot");
}

fn post(base: &str, path: &str, body: &str, token: &str) -> (u16, serde_json::Value) {
    let (status, text) =
        http_request_with_auth("POST", &format!("{base}{path}"), Some(body), Some(token));
    (
        status,
        serde_json::from_str(&text).unwrap_or(serde_json::Value::Null),
    )
}

fn get(base: &str, path: &str, token: &str) -> (u16, serde_json::Value) {
    let (status, text) = http_request_with_auth("GET", &format!("{base}{path}"), None, Some(token));
    (
        status,
        serde_json::from_str(&text).unwrap_or(serde_json::Value::Null),
    )
}

fn listed(base: &str, token: &str) -> Vec<String> {
    let (status, rooms) = get(base, "/api/rooms", token);
    assert_eq!(status, 200, "{rooms}");
    rooms
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["name"].as_str().map(str::to_string))
        .collect()
}

#[test]
fn rooms_tied_to_a_row_follow_its_read_policy() {
    let (port, rt) = start_server();
    let base = format!("http://127.0.0.1:{port}");
    let (owner_token, owner) = guest_session(&base);
    let (other_token, _other) = guest_session(&base);
    let note = rt
        .insert(
            "Note",
            &serde_json::json!({ "title": "plan", "ownerId": owner, "viewerId": "" }),
        )
        .unwrap();
    let room = format!("@Note/{note}");
    let encoded = format!("%40Note%2F{note}");

    // The owner joins; someone else may not.
    let (status, body) = post(
        &base,
        "/api/rooms/join",
        &format!(r#"{{"room":"{room}"}}"#),
        &owner_token,
    );
    assert_eq!(status, 200, "{body}");
    let (status, body) = post(
        &base,
        "/api/rooms/join",
        &format!(r#"{{"room":"{room}"}}"#),
        &other_token,
    );
    assert_eq!(
        (status, body["error"]["code"].as_str()),
        (403, Some("ROOM_FORBIDDEN")),
        "{body}"
    );

    // A missing row answers the same; a malformed @ name is refused.
    let (status, body) = post(
        &base,
        "/api/rooms/join",
        r#"{"room":"@Note/no-such-note"}"#,
        &owner_token,
    );
    assert_eq!(
        (status, body["error"]["code"].as_str()),
        (403, Some("ROOM_FORBIDDEN")),
        "{body}"
    );
    let (status, body) = post(
        &base,
        "/api/rooms/join",
        r#"{"room":"@Note"}"#,
        &owner_token,
    );
    assert_eq!(
        (status, body["error"]["code"].as_str()),
        (400, Some("INVALID_ROOM")),
        "{body}"
    );

    // The room list and the member list follow the same rule.
    assert!(listed(&base, &owner_token).contains(&room));
    assert!(!listed(&base, &other_token).contains(&room));
    let (status, members) = get(&base, &format!("/api/rooms/{encoded}"), &owner_token);
    assert_eq!(status, 200, "{members}");
    assert_eq!(
        members["members"][0]["user_id"].as_str(),
        Some(owner.as_str())
    );
    let (status, body) = get(&base, &format!("/api/rooms/{encoded}"), &other_token);
    assert_eq!(status, 403, "{body}");

    // Someone else cannot send into the room either.
    let (status, body) = post(
        &base,
        "/api/rooms/broadcast",
        &format!(r#"{{"room":"{room}","topic":"t","data":{{}}}}"#),
        &other_token,
    );
    assert_eq!(status, 403, "{body}");

    // The owner subscribes over the WebSocket.
    let mut ws = connect_ws_token(port, &owner_token);
    ws.send(Message::Text(
        serde_json::json!({"type":"room-subscribe","room":room}).to_string(),
    ))
    .unwrap();
    let snap = wait_for_frame(&mut ws, Duration::from_secs(2), |v| {
        v.get("type").and_then(|t| t.as_str()) == Some("room-snapshot")
    });
    assert!(snap.is_some(), "the owner gets the room's snapshot");

    // The note changes owner: the former owner loses access.
    rt.update(
        "Note",
        &note,
        &serde_json::json!({ "ownerId": "someone-else" }),
    )
    .unwrap();
    let (status, body) = post(
        &base,
        "/api/rooms/presence",
        &format!(r#"{{"room":"{room}","data":{{"cursor":3}}}}"#),
        &owner_token,
    );
    assert_eq!(status, 403, "presence after losing access: {body}");

    // The next heartbeat removes them, and their socket is told.
    let (status, body) = post(
        &base,
        "/api/rooms/heartbeat",
        &format!(r#"{{"room":"{room}"}}"#),
        &owner_token,
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["alive"], false, "{body}");
    assert_eq!(body["reason"], "ROOM_FORBIDDEN", "{body}");
    let revoked = wait_for_frame(&mut ws, Duration::from_secs(2), |v| {
        v.get("type").and_then(|t| t.as_str()) == Some("room-revoked")
    });
    assert_eq!(
        revoked.expect("room-revoked")["room"].as_str(),
        Some(room.as_str())
    );
    let (status, members) = get(&base, &format!("/api/rooms/{encoded}"), TEST_ADMIN_TOKEN);
    assert_eq!(status, 200, "{members}");
    assert_eq!(members["count"], 0, "{members}");

    // Subscribing again is refused.
    ws.send(Message::Text(
        serde_json::json!({"type":"room-subscribe","room":room}).to_string(),
    ))
    .unwrap();
    let refused = wait_for_frame(&mut ws, Duration::from_secs(2), |v| {
        v.get("type").and_then(|t| t.as_str()) == Some("error")
    });
    assert!(refused.is_some(), "a second subscribe is refused");
}

/// A viewer who loses access stops receiving the room's events at the next
/// event, without waiting for their heartbeat.
#[test]
fn a_viewer_who_loses_access_stops_receiving_events_at_once() {
    let (port, rt) = start_server();
    let base = format!("http://127.0.0.1:{port}");
    let (owner_token, owner) = guest_session(&base);
    let (viewer_token, viewer) = guest_session(&base);
    let note = rt
        .insert(
            "Note",
            &serde_json::json!({ "title": "plan", "ownerId": owner, "viewerId": viewer }),
        )
        .unwrap();
    let room = format!("@Note/{note}");
    for token in [&owner_token, &viewer_token] {
        let (status, body) = post(
            &base,
            "/api/rooms/join",
            &format!(r#"{{"room":"{room}"}}"#),
            token,
        );
        assert_eq!(status, 200, "{body}");
    }
    let mut viewer_ws = connect_ws_token(port, &viewer_token);
    viewer_ws
        .send(Message::Text(
            serde_json::json!({"type":"room-subscribe","room":room}).to_string(),
        ))
        .unwrap();
    assert!(wait_for_frame(&mut viewer_ws, Duration::from_secs(2), |v| {
        v.get("type").and_then(|t| t.as_str()) == Some("room-snapshot")
    })
    .is_some());

    // Unshared.
    rt.update("Note", &note, &serde_json::json!({ "viewerId": "" }))
        .unwrap();

    // The owner moves their cursor: the viewer gets room-revoked, not the
    // cursor.
    let (status, body) = post(
        &base,
        "/api/rooms/presence",
        &format!(r#"{{"room":"{room}","data":{{"cursor":42}}}}"#),
        &owner_token,
    );
    assert_eq!(status, 200, "{body}");
    let mut saw_cursor = false;
    let revoked = wait_for_frame(&mut viewer_ws, Duration::from_secs(2), |v| {
        if v.get("type").and_then(|t| t.as_str()) == Some("room-update") {
            saw_cursor = true;
        }
        v.get("type").and_then(|t| t.as_str()) == Some("room-revoked")
    });
    assert!(revoked.is_some(), "the viewer is told");
    assert!(
        !saw_cursor,
        "the viewer must not see the owner's cursor after losing access"
    );

    // Later events do not reach them either.
    let (status, _) = post(
        &base,
        "/api/rooms/presence",
        &format!(r#"{{"room":"{room}","data":{{"cursor":43}}}}"#),
        &owner_token,
    );
    assert_eq!(status, 200);
    let later = wait_for_frame(&mut viewer_ws, Duration::from_millis(800), |v| {
        v.get("type").and_then(|t| t.as_str()) == Some("room-update")
    });
    assert!(
        later.is_none(),
        "no room events after revocation: {later:?}"
    );
}
