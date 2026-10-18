//! Live re-delivery when access changes through another entity.
//!
//! `Note` is readable by its owner and by anyone with a `Share` row naming
//! them (`exists(Share where noteId == data.id and userId == auth.userId)`).
//! Creating or deleting a share changes who may read the note without a
//! change event for the note, so the server sends the named user's sockets
//! `{"type":"sync-reconcile","entities":["Note"]}` and the client re-syncs.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pylon_kernel::{AppManifest, ManifestEntity, ManifestField, ManifestPolicy};
use pylon_runtime::Runtime;
use tungstenite::client::IntoClientRequest;
use tungstenite::{client, Message};

const TEST_ADMIN_TOKEN: &str = "testadmin_sync_dependency";

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
        name: "sync-dependency".into(),
        version: "0.1.0".into(),
        entities: vec![
            ManifestEntity {
                name: "Note".into(),
                fields: vec![string_field("title"), string_field("ownerId")],
                sync: true,
                ..Default::default()
            },
            // Not replicated: the hint must still go out.
            ManifestEntity {
                name: "Share".into(),
                fields: vec![
                    string_field("noteId"),
                    string_field("userId"),
                    string_field("label"),
                ],
                sync: false,
                ..Default::default()
            },
            // Names the reader, but the reader may not read it.
            ManifestEntity {
                name: "Assignment".into(),
                fields: vec![string_field("noteId"), string_field("userId")],
                sync: false,
                ..Default::default()
            },
        ],
        policies: vec![
            ManifestPolicy {
                name: "noteRead".into(),
                entity: Some("Note".into()),
                allow_read: Some(
                    "auth.userId == data.ownerId || exists(Share where noteId == data.id and userId == auth.userId) || exists(Assignment where noteId == data.id and userId == auth.userId)"
                        .into(),
                ),
                ..Default::default()
            },
            ManifestPolicy {
                name: "shareNamedReader".into(),
                entity: Some("Share".into()),
                allow_read: Some("auth.userId == data.userId".into()),
                ..Default::default()
            },
            ManifestPolicy {
                name: "assignmentPrivate".into(),
                entity: Some("Assignment".into()),
                allow_read: Some("false".into()),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn available_port() -> u16 {
    // Below the ephemeral range (Linux 32768-60999): a port up there can be
    // handed to one of these tests' OWN client sockets between the probe
    // below and the server's bind, which surfaces as EADDRINUSE on CI.
    // One 1000-port lane per test binary so parallel binaries can't overlap.
    static NEXT: AtomicU16 = AtomicU16::new(30_000);
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
fn post(base: &str, path: &str, body: &str, token: &str) -> (u16, serde_json::Value) {
    let (status, text) =
        http_request_with_auth("POST", &format!("{base}{path}"), Some(body), Some(token));
    (
        status,
        serde_json::from_str(&text).unwrap_or(serde_json::Value::Null),
    )
}

fn is_hint(v: &serde_json::Value) -> bool {
    v.get("type").and_then(|t| t.as_str()) == Some("sync-reconcile")
}

#[test]
fn a_share_and_an_unshare_tell_the_named_user_to_resync() {
    let (port, rt) = start_server();
    let base = format!("http://127.0.0.1:{port}");
    let (_owner_token, owner) = guest_session(&base);
    let (reader_token, reader) = guest_session(&base);
    let (other_token, _other) = guest_session(&base);
    let note = rt
        .insert(
            "Note",
            &serde_json::json!({ "title": "plan", "ownerId": owner }),
        )
        .unwrap();
    let mut reader_ws = connect_ws_token(port, &reader_token);
    let mut other_ws = connect_ws_token(port, &other_token);

    // Shared: the reader is told to re-sync Note.
    let (status, body) = post(
        &base,
        "/api/entities/Share",
        &serde_json::json!({ "noteId": note, "userId": reader, "label": "x" }).to_string(),
        TEST_ADMIN_TOKEN,
    );
    assert!(status == 200 || status == 201, "{status} {body}");
    let share = body["id"].as_str().expect("share id").to_string();
    let hint =
        wait_for_frame(&mut reader_ws, Duration::from_secs(3), is_hint).expect("hint on share");
    assert_eq!(hint["entities"], serde_json::json!(["Note"]));

    // Nobody else is named, so nobody else is told.
    assert!(wait_for_frame(&mut other_ws, Duration::from_millis(800), is_hint).is_none());

    // A change that touches no condition field changes no one's access.
    let (status, body) = http_request_with_auth(
        "PATCH",
        &format!("{base}/api/entities/Share/{share}"),
        Some(r#"{"label":"renamed"}"#),
        Some(TEST_ADMIN_TOKEN),
    );
    assert_eq!(status, 200, "{body}");
    assert!(wait_for_frame(&mut reader_ws, Duration::from_millis(800), is_hint).is_none());

    // A row the reader may not read is never hinted, even though it names
    // them: the hint would reveal it.
    let (status, body) = post(
        &base,
        "/api/entities/Assignment",
        &serde_json::json!({ "noteId": note, "userId": reader }).to_string(),
        TEST_ADMIN_TOKEN,
    );
    assert!(status == 200 || status == 201, "{status} {body}");
    assert!(wait_for_frame(&mut reader_ws, Duration::from_millis(800), is_hint).is_none());

    // Unshared: the deleted row still names the reader.
    let (status, body) = http_request_with_auth(
        "DELETE",
        &format!("{base}/api/entities/Share/{share}"),
        None,
        Some(TEST_ADMIN_TOKEN),
    );
    assert!(status == 200 || status == 204, "{status} {body}");
    let hint =
        wait_for_frame(&mut reader_ws, Duration::from_secs(3), is_hint).expect("hint on unshare");
    assert_eq!(hint["entities"], serde_json::json!(["Note"]));
}
