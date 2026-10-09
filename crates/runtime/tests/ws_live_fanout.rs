//! What the live WebSocket fan-out sends a subscribed socket, and when it
//! stops.
//!
//! - `sync: false` entities are never sent: the pull path leaves them out,
//!   and so must the change broadcast and the replication cursor fetch.
//! - A connection whose session ended (sign-out, revocation) is closed,
//!   and its reactive subscriptions stop re-running.

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
        max_length: None,
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
        let ok = (0..4).all(|off| pylon_runtime::listen::port_is_free(base + off));
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
    start_runtime(Arc::new(Runtime::in_memory(manifest()).unwrap()))
}

fn start_runtime(rt: Arc<Runtime>) -> (u16, Arc<Runtime>) {
    static ENV: std::sync::Once = std::sync::Once::new();
    ENV.call_once(|| {
        // SAFETY: once per binary, before any server thread starts.
        unsafe {
            std::env::set_var("PYLON_ADMIN_TOKEN", ADMIN_TOKEN);
            std::env::set_var("PYLON_DEV_MODE", "1");
        }
    });
    let port = available_port();
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

/// A `query` named `feed` that guests may subscribe to. Counts runs.
struct FeedFns {
    calls: std::sync::atomic::AtomicUsize,
}

impl pylon_router::FnOps for FeedFns {
    fn get_fn(&self, name: &str) -> Option<pylon_functions::registry::FnDef> {
        (name == "feed").then(|| pylon_functions::registry::FnDef {
            name: name.into(),
            fn_type: pylon_functions::protocol::FnType::Query,
            args_schema: None,
            internal: false,
            auth: pylon_functions::registry::FnAuthMode::Guest,
            timeout_secs: None,
        })
    }
    fn list_fns(&self) -> Vec<pylon_functions::registry::FnDef> {
        self.get_fn("feed").into_iter().collect()
    }
    fn call(
        &self,
        fn_name: &str,
        _args: serde_json::Value,
        _auth: pylon_functions::protocol::AuthInfo,
        _on_stream: Option<pylon_functions::runner::StreamCallback>,
        _request: Option<pylon_functions::protocol::RequestInfo>,
        _stream_id: Option<String>,
    ) -> Result<
        (serde_json::Value, pylon_functions::trace::FnTrace),
        pylon_functions::runner::FnCallError,
    > {
        let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        Ok((
            serde_json::json!({ "run": n }),
            pylon_functions::trace::FnTrace {
                call_id: "t".into(),
                fn_name: fn_name.into(),
                fn_type: pylon_functions::protocol::FnType::Query,
                user_id: None,
                started_at: 0,
                duration_ms: 0.0,
                outcome: pylon_functions::trace::FnOutcome::Ok { value: None },
                ops: vec![],
                ops_omitted: 0,
                schedules_omitted: 0,
                stream_bytes: 0,
                stream_chunks: 0,
                schedules: vec![],
            },
        ))
    }
    fn recent_traces(&self, _limit: usize) -> Vec<pylon_functions::trace::FnTrace> {
        vec![]
    }
}

fn start_server_with_feed() -> u16 {
    start_server(); // the shared env setup; the server itself is unused
    let port = available_port();
    let rt = Arc::new(Runtime::in_memory(manifest()).unwrap());
    let fns: Arc<dyn pylon_router::FnOps> = Arc::new(FeedFns {
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    std::thread::spawn(move || {
        let _ = pylon_runtime::server::start_server_for_test_with_fn_ops(rt, port, fns);
    });
    wait_for_port(port);
    wait_for_port(port + 1);
    port
}

fn mint_guest(port: u16) -> String {
    let (status, body) = http(port, "POST", "/api/auth/guest", Some("{}"), None);
    assert_eq!(status, 201, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    v["token"].as_str().unwrap().to_string()
}

fn subscribe_feed(ws: &mut Ws) {
    ws.send(Message::Text(
        serde_json::json!({"type": "reactive-subscribe", "sub_id": "s1", "fn_name": "feed"})
            .to_string(),
    ))
    .unwrap();
    let (frames, got) = read_until(ws, Duration::from_secs(10), |v| {
        v["type"] == "reactive-result" || v["type"] == "reactive-error"
    });
    assert!(got, "no reactive reply: {frames:?}");
    assert_eq!(
        frames.last().unwrap()["type"],
        "reactive-result",
        "{frames:?}"
    );
}

/// How a socket ended within `deadline`: `Some(close code)` for a close
/// frame, `Some(None)` for a dropped connection, `None` if still open.
fn ended_within(ws: &mut Ws, deadline: Duration) -> Option<Option<u16>> {
    let start = Instant::now();
    while start.elapsed() < deadline {
        match ws.read() {
            Ok(Message::Close(frame)) => return Some(frame.map(|f| u16::from(f.code))),
            Ok(Message::Ping(d)) => {
                let _ = ws.send(Message::Pong(d));
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(_) => return Some(None),
        }
    }
    None
}

/// Before, a WebSocket kept the identity it connected with: after the
/// session was revoked (sign-out), the socket stayed open and its reactive
/// subscriptions kept re-running under the revoked identity. Now the
/// socket that used the session is closed (Policy, 1008), and one on
/// another session is not.
#[test]
fn signing_out_closes_the_sockets_that_used_the_session() {
    let port = start_server_with_feed();
    let revoked_token = mint_guest(port);
    let other_token = mint_guest(port);
    let mut revoked = connect_ws(port, &revoked_token);
    let mut other = connect_ws(port, &other_token);
    subscribe_feed(&mut revoked);
    subscribe_feed(&mut other);

    let (status, body) = http(
        port,
        "DELETE",
        "/api/auth/session",
        None,
        Some(&revoked_token),
    );
    assert_eq!(status, 200, "{body}");

    let ended = ended_within(&mut revoked, Duration::from_secs(5));
    assert_eq!(
        ended,
        Some(Some(1008)),
        "the socket of the revoked session stayed open or closed without a Policy frame"
    );
    assert_eq!(
        ended_within(&mut other, Duration::from_secs(1)),
        None,
        "a socket on another session was closed"
    );
}

#[test]
fn private_fields_in_old_crdt_history_never_reach_binary_clients() {
    private_history_wire_case(false);
}

#[test]
fn removing_a_private_field_does_not_publish_its_old_crdt_history() {
    private_history_wire_case(true);
}

fn private_history_wire_case(remove_field: bool) {
    use pylon_http::DataStore;
    use serde_json::json;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("privacy.sqlite");
    let mut schema = AppManifest {
        entities: vec![
            ManifestEntity {
                name: "PrivateDoc".into(),
                fields: vec![field("title"), field("secret")],
                crdt: true,
                sync: true,
                ..Default::default()
            },
            ManifestEntity {
                name: "PublicDoc".into(),
                fields: vec![field("title")],
                crdt: true,
                sync: true,
                ..Default::default()
            },
        ],
        policies: vec![open_policy("PrivateDoc"), open_policy("PublicDoc")],
        ..Default::default()
    };
    let old = Runtime::open(path.to_str().unwrap(), schema.clone()).unwrap();
    let private_id = old
        .insert(
            "PrivateDoc",
            &json!({"title":"before", "secret":"stored-private-value"}),
        )
        .unwrap();
    let public_id = old.insert("PublicDoc", &json!({"title":"public"})).unwrap();
    assert!(!DataStore::has_private_crdt_history(&old, "PrivateDoc"));
    schema.entities[0].fields[1].server_only = true;
    let private_runtime = Runtime::open(path.to_str().unwrap(), schema.clone()).unwrap();
    assert!(DataStore::has_private_crdt_history(
        &private_runtime,
        "PrivateDoc"
    ));
    assert!(
        DataStore::has_private_crdt_history(&old, "PrivateDoc"),
        "running instances must see restrictions from another instance"
    );
    assert!(!DataStore::has_private_crdt_history(&old, "PublicDoc"));
    drop(old);
    drop(private_runtime);
    if remove_field {
        schema.entities[0].fields.pop();
        assert!(pylon_router::supports_crdt_replication(
            &schema,
            &schema.auth.user,
            "PrivateDoc"
        ));
    }
    let runtime = Arc::new(Runtime::open(path.to_str().unwrap(), schema).unwrap());
    assert!(DataStore::has_private_crdt_history(
        runtime.as_ref(),
        "PrivateDoc"
    ));
    let snapshot = DataStore::crdt_snapshot(runtime.as_ref(), "PrivateDoc", &private_id)
        .unwrap()
        .unwrap();
    let document = pylon_crdt::loro::LoroDoc::new();
    document.import(&snapshot).unwrap();
    let stored = serde_json::to_value(document.get_deep_value()).unwrap();
    assert_eq!(
        stored[pylon_crdt::ROOT_MAP]["secret"],
        "stored-private-value",
        "the test must cover an existing private document"
    );
    let (port, _runtime) = start_runtime(runtime);
    let mut ws = connect_ws(port, ADMIN_TOKEN);
    for (entity, row) in [("PrivateDoc", &private_id), ("PublicDoc", &public_id)] {
        ws.send(Message::Text(
            json!({"type":"crdt-subscribe", "entity":entity, "rowId":row}).to_string(),
        ))
        .unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut public_snapshot = false;
    while Instant::now() < deadline && !public_snapshot {
        match ws.read() {
            Ok(Message::Binary(bytes)) => {
                let len = u16::from_be_bytes([bytes[1], bytes[2]]) as usize;
                let entity = std::str::from_utf8(&bytes[3..3 + len]).unwrap();
                assert_eq!(
                    entity, "PublicDoc",
                    "private CRDT history reached the socket"
                );
                public_snapshot = true;
            }
            Ok(Message::Ping(bytes)) => {
                ws.send(Message::Pong(bytes)).unwrap();
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) => panic!("websocket failed: {error}"),
        }
    }
    assert!(
        public_snapshot,
        "public CRDT subscriptions must remain available"
    );
    let (status, body) = http(
        port,
        "POST",
        &format!("/api/crdt/PrivateDoc/{private_id}"),
        Some(r#"{"update":"00"}"#),
        Some(ADMIN_TOKEN),
    );
    assert_eq!(status, 403, "{body}");
    assert!(body.contains("CRDT_REPLICATION_DISABLED"));
    let (status, body) = http(
        port,
        "PATCH",
        &format!("/api/entities/PrivateDoc/{private_id}"),
        Some(r#"{"title":"after"}"#),
        Some(ADMIN_TOKEN),
    );
    assert_eq!(status, 200, "{body}");
    assert!(!body.contains("stored-private-value"));
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut saw_json = false;
    while Instant::now() < deadline {
        match ws.read() {
            Ok(Message::Binary(_)) => panic!("private live update emitted a binary frame"),
            Ok(Message::Text(text)) => {
                assert!(!text.contains("stored-private-value"));
                let value: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                if value["entity"] == "PrivateDoc" && value["data"]["title"] == "after" {
                    saw_json = true;
                }
            }
            Ok(Message::Ping(bytes)) => {
                ws.send(Message::Pong(bytes)).unwrap();
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) => panic!("websocket failed: {error}"),
        }
    }
    assert!(saw_json, "projected JSON replication must remain available");
}
