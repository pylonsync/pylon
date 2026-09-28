//! What the live WebSocket fan-out sends a subscribed socket, and when it
//! stops.
//!
//! - `sync: false` entities are never sent: the pull path leaves them out,
//!   and so must the change broadcast and the replication cursor fetch.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pylon_kernel::{AppManifest, ManifestEntity, ManifestField, ManifestPolicy};
use pylon_runtime::Runtime;
use tungstenite::client::IntoClientRequest;
use tungstenite::{client, Message};

const ADMIN_TOKEN: &str = "testadmin_ws_live_fanout";

type Ws = tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>;

fn field(name: &str) -> ManifestField {
    ManifestField {
        name: name.into(),
        field_type: "string".into(),
        optional: false,
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

fn entity(name: &str, sync: bool) -> ManifestEntity {
    ManifestEntity {
        name: name.into(),
        fields: vec![field("title")],
        crdt: false,
        sync,
        ..Default::default()
    }
}

fn open_policy(entity: &str) -> ManifestPolicy {
    ManifestPolicy {
        name: format!("{entity}_open"),
        entity: Some(entity.into()),
        allow_read: Some("true".into()),
        allow_insert: Some("true".into()),
        ..Default::default()
    }
}

fn manifest() -> AppManifest {
    AppManifest {
        manifest_version: 1,
        name: "ws-live-fanout".into(),
        version: "0.1.0".into(),
        entities: vec![entity("Todo", true), entity("Catalog", false)],
        policies: vec![open_policy("Todo"), open_policy("Catalog")],
        ..Default::default()
    }
}

fn available_port() -> u16 {
    // One 1000-port lane per test binary (see rooms_ws_push.rs).
    static NEXT: AtomicU16 = AtomicU16::new(28_000);
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

fn wait_for_port(port: u16) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if TcpStream::connect(format!("127.0.0.1:{port}")).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("test server never bound 127.0.0.1:{port}");
}

fn start_server() -> (u16, Arc<Runtime>) {
    static ENV: std::sync::Once = std::sync::Once::new();
    ENV.call_once(|| {
        // SAFETY: once per binary, before any server thread starts.
        unsafe {
            std::env::set_var("PYLON_ADMIN_TOKEN", ADMIN_TOKEN);
            std::env::set_var("PYLON_DEV_MODE", "1");
        }
    });
    let port = available_port();
    let rt = Arc::new(Runtime::in_memory(manifest()).unwrap());
    let rt2 = Arc::clone(&rt);
    std::thread::spawn(move || {
        let _ = pylon_runtime::server::start(rt2, port);
    });
    wait_for_port(port);
    wait_for_port(port + 1);
    (port, rt)
}

fn http(
    port: u16,
    method: &str,
    path: &str,
    body: Option<&str>,
    token: Option<&str>,
) -> (u16, String) {
    let body = body.unwrap_or("");
    let auth = token
        .map(|t| format!("Authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://127.0.0.1:{port}\r\n\
         {auth}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream = TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    stream.write_all(request.as_bytes()).expect("write");
    let mut response = Vec::new();
    let _ = stream.read_to_end(&mut response);
    let text = String::from_utf8_lossy(&response).to_string();
    let status = text
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body = text
        .find("\r\n\r\n")
        .map(|i| text[i + 4..].to_string())
        .unwrap_or_default();
    (status, body)
}

fn connect_ws(port: u16, token: &str) -> Ws {
    let mut req = format!("ws://127.0.0.1:{}/", port + 1)
        .into_client_request()
        .expect("ws request");
    req.headers_mut()
        .insert("Authorization", format!("Bearer {token}").parse().unwrap());
    let (ws, _resp) = client::connect(req).expect("ws connect");
    if let tungstenite::stream::MaybeTlsStream::Plain(ref s) = ws.get_ref() {
        s.set_read_timeout(Some(Duration::from_millis(200))).ok();
    }
    ws
}

/// Every text frame until `stop` matches one (included) or the deadline.
fn read_until<F>(ws: &mut Ws, deadline: Duration, mut stop: F) -> (Vec<serde_json::Value>, bool)
where
    F: FnMut(&serde_json::Value) -> bool,
{
    let start = Instant::now();
    let mut frames = Vec::new();
    while start.elapsed() < deadline {
        match ws.read() {
            Ok(Message::Text(text)) => {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                    let done = stop(&v);
                    frames.push(v);
                    if done {
                        return (frames, true);
                    }
                }
            }
            Ok(Message::Ping(d)) => {
                let _ = ws.send(Message::Pong(d));
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(_) => return (frames, false),
        }
    }
    (frames, false)
}

/// Open the dedicated SSE stream (port + 2) and return the socket once
/// the response head has arrived.
fn connect_sse(port: u16, token: &str) -> TcpStream {
    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port + 2)).expect("sse connect");
    stream
        .write_all(
            format!("GET /events?token={token} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes(),
        )
        .unwrap();
    stream
        .set_read_timeout(Some(Duration::from_millis(200)))
        .ok();
    let mut head = Vec::new();
    let mut buf = [0u8; 1024];
    let start = Instant::now();
    while !String::from_utf8_lossy(&head).contains("\r\n\r\n") {
        assert!(start.elapsed() < Duration::from_secs(5), "no SSE head");
        if let Ok(n) = stream.read(&mut buf) {
            assert!(n > 0, "SSE closed: {}", String::from_utf8_lossy(&head));
            head.extend_from_slice(&buf[..n]);
        }
    }
    assert!(
        String::from_utf8_lossy(&head).starts_with("HTTP/1.1 200"),
        "{}",
        String::from_utf8_lossy(&head)
    );
    stream
}

/// Raw SSE text until `needle` appears or the deadline passes.
fn read_sse_until(stream: &mut TcpStream, needle: &str, deadline: Duration) -> String {
    let mut acc = String::new();
    let mut buf = [0u8; 16 * 1024];
    let start = Instant::now();
    while !acc.contains(needle) && start.elapsed() < deadline {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => acc.push_str(&String::from_utf8_lossy(&buf[..n])),
            Err(_) => {}
        }
    }
    acc
}

fn excerpt(s: &str) -> &str {
    &s[..s.len().min(400)]
}

/// A batch of writes to a `sync: false` entity sent one change frame per
/// write to every subscribed socket (200 writes, 200 frames), and the
/// client's reconcile then paged the whole table through the replication
/// cursor. Neither may happen: the entity is not replicated.
#[test]
fn sync_false_entities_never_reach_a_subscribed_socket() {
    let (port, _rt) = start_server();
    wait_for_port(port + 2);
    // The admin socket bypasses read policies, so it is the socket most
    // likely to receive everything.
    let mut ws = connect_ws(port, ADMIN_TOKEN);
    let (status, body) = http(port, "POST", "/api/auth/guest", Some("{}"), None);
    assert_eq!(status, 201, "{body}");
    let guest: serde_json::Value = serde_json::from_str(&body).unwrap();
    let mut sse = connect_sse(port, guest["token"].as_str().unwrap());

    for i in 0..200 {
        let (status, body) = http(
            port,
            "POST",
            "/api/entities/Catalog",
            Some(&format!(r#"{{"title":"item {i}"}}"#)),
            Some(ADMIN_TOKEN),
        );
        assert_eq!(status, 201, "catalog insert {i}: {body}");
    }
    // A synced write after the batch. Frames to one socket arrive in
    // order, so every Catalog frame the server sent is read before it.
    let (status, body) = http(
        port,
        "POST",
        "/api/entities/Todo",
        Some(r#"{"title":"marker"}"#),
        Some(ADMIN_TOKEN),
    );
    assert_eq!(status, 201, "todo insert: {body}");

    let (frames, saw_marker) =
        read_until(&mut ws, Duration::from_secs(10), |v| v["entity"] == "Todo");
    assert!(saw_marker, "the synced write never arrived: {frames:?}");
    let catalog = frames.iter().filter(|v| v["entity"] == "Catalog").count();
    assert_eq!(catalog, 0, "sync:false change frames reached the socket");
    let sse_text = read_sse_until(&mut sse, "marker", Duration::from_secs(10));
    assert!(
        sse_text.contains("marker"),
        "SSE never got the synced write: {}",
        excerpt(&sse_text)
    );
    assert!(
        !sse_text.contains("Catalog"),
        "sync:false change events reached the SSE stream: {}",
        excerpt(&sse_text)
    );

    // The replication cursor fetch (reconcile) is empty for the entity;
    // a direct read still returns rows.
    let (status, body) = http(
        port,
        "GET",
        "/api/entities/Catalog/cursor?limit=1000&sync=1",
        None,
        Some(ADMIN_TOKEN),
    );
    assert_eq!(status, 200, "{}", excerpt(&body));
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(page["data"], serde_json::json!([]), "{}", excerpt(&body));
    assert_eq!(page["has_more"], false);
    let (status, body) = http(
        port,
        "GET",
        "/api/entities/Catalog/cursor?limit=10",
        None,
        Some(ADMIN_TOKEN),
    );
    assert_eq!(status, 200, "{body}");
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(page["data"].as_array().map(Vec::len), Some(10), "{body}");
}
