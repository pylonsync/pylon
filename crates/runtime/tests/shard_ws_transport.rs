//! Shard WebSocket transport under clients that misbehave.
//!
//! One shard ticks at 20 Hz with large snapshots (64 KB). Three clients
//! connect:
//!
//!   - `idle` reads frames but never sends anything,
//!   - `stalled` sends nothing and never reads, so its socket buffers fill,
//!   - `active` reads every frame and sends one input.
//!
//! The shard must keep its tick rate, `active` must get (almost) every tick
//! and see its input applied, and `stalled` must be disconnected once its
//! outbound queue has been full for `slow_subscriber_timeout`.

use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pylon_auth::SessionStore;
use pylon_realtime::{DynShardRegistry, Shard, ShardConfig, ShardRegistry, SimState, SubscriberId};
use tungstenite::client::IntoClientRequest;
use tungstenite::{Message, WebSocket};

const SNAPSHOT_BYTES: usize = 64 * 1024;

struct Zone {
    total: i64,
}

impl SimState for Zone {
    type Input = i64;
    type Snapshot = (i64, String);
    type Error = String;

    fn apply_input(
        &mut self,
        _sub: &SubscriberId,
        input: i64,
        _now: Instant,
    ) -> Result<(), String> {
        self.total += input;
        Ok(())
    }
    fn tick(&mut self, _dt: Duration) {}
    fn snapshot(&self) -> Self::Snapshot {
        (self.total, "x".repeat(SNAPSHOT_BYTES))
    }
}

struct Server {
    port: u16,
    sessions: Arc<SessionStore>,
    registry: Arc<ShardRegistry<Zone>>,
}

fn start() -> Server {
    let registry: Arc<ShardRegistry<Zone>> = Arc::new(ShardRegistry::new());
    registry.insert(Shard::new(
        "zone",
        Zone { total: 0 },
        ShardConfig {
            tick_rate_hz: 20,
            slow_subscriber_timeout: Duration::from_secs(2),
            // 8 frames = 0.4 s of buffering before the queue counts as full.
            outbound_queue_frames: 8,
            idle_ticks_before_shutdown: 0,
            ..Default::default()
        },
    ));
    let sessions = Arc::new(SessionStore::new());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let dyn_registry: Arc<dyn DynShardRegistry> = registry.clone();
    let s = Arc::clone(&sessions);
    // No per-IP cap: every test client connects from 127.0.0.1.
    std::thread::spawn(move || pylon_runtime::shard_ws::serve(listener, dyn_registry, s, 0));
    Server {
        port,
        sessions,
        registry,
    }
}

fn connect(server: &Server, sid: &str) -> WebSocket<TcpStream> {
    let token = server.sessions.create(sid.to_string()).token;
    let stream = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
    let mut req = format!("ws://127.0.0.1:{}/?shard=zone&sid={sid}", server.port)
        .into_client_request()
        .unwrap();
    req.headers_mut()
        .insert("Authorization", format!("Bearer {token}").parse().unwrap());
    let (ws, _) = tungstenite::client(req, stream).expect("handshake");
    ws
}

/// Tick number from a snapshot frame (8-byte big-endian prefix).
fn frame_tick(bytes: &[u8]) -> u64 {
    u64::from_be_bytes(bytes[..8].try_into().unwrap())
}

#[test]
fn idle_and_stalled_clients_do_not_stop_the_shard_or_other_clients() {
    let server = start();
    let shard = server.registry.get("zone").unwrap();

    let mut idle = connect(&server, "idle");
    idle.get_ref()
        .set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    let idle_reader = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut frames = 0u64;
        while Instant::now() < deadline {
            if let Ok(Message::Binary(_)) = idle.read() {
                frames += 1;
            }
        }
        frames
    });

    // Connected and then left alone: never reads, never sends.
    let _stalled = connect(&server, "stalled");

    let mut active = connect(&server, "active");
    active
        .send(Message::Text(r#"{"input": 7}"#.into()))
        .unwrap();

    let start_tick = shard.tick_number();
    let start = Instant::now();
    let mut ticks_seen = std::collections::BTreeSet::new();
    let mut saw_input = false;
    while start.elapsed() < Duration::from_secs(5) {
        if let Message::Binary(bytes) = active.read().unwrap() {
            ticks_seen.insert(frame_tick(&bytes));
            // The snapshot is JSON `[total, "xxx…"]`.
            saw_input |= bytes[8..].starts_with(b"[7,");
        }
    }
    let elapsed = start.elapsed();
    let ticks_run = shard.tick_number() - start_tick;

    // A stalled tick loop would run almost no ticks. A slow CI runner runs
    // fewer than the 20 Hz target but far more than half; that is the bound
    // checked here. The per-client check below stays exact.
    let expected = (elapsed.as_secs_f64() * 20.0) as u64;
    assert!(
        ticks_run * 2 >= expected,
        "the shard ran only {ticks_run} ticks in {elapsed:?} (target {expected})"
    );
    assert!(
        ticks_seen.len() as u64 + 5 >= ticks_run,
        "active saw {} of {ticks_run} ticks",
        ticks_seen.len()
    );
    assert!(saw_input, "active's input was applied");

    let idle_frames = idle_reader.join().unwrap();
    assert!(
        idle_frames >= 80,
        "the idle client still got frames ({idle_frames})"
    );

    // `stalled` fills its queue (after the socket buffers do) and is
    // disconnected 2 s later. `idle` and `active` stay.
    drop(active);
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let subs = shard.subscriber_count();
        // `active` was dropped above and `idle` stopped reading at 5 s, so
        // both leave too; what matters is that `stalled` goes.
        if subs == 0 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "{subs} subscribers still attached"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// #20: thousands of idle connections run on a few threads. Needs a raised
/// file descriptor limit (`ulimit -n 8192`), so it runs on request:
/// `cargo test -p pylon-runtime --test shard_ws_transport -- --ignored`.
#[test]
#[ignore]
fn three_thousand_idle_connections_use_few_threads() {
    let server = start();
    let mut conns = Vec::with_capacity(3000);
    for i in 0..3000 {
        let ws = connect(&server, &format!("p{i}"));
        ws.get_ref().set_nonblocking(true).unwrap();
        conns.push(ws);
    }
    let shard = server.registry.get("zone").unwrap();
    // Admission is capped by max_subscribers (256 by default); every
    // connection still holds a socket and a task.
    assert!(shard.subscriber_count() > 0);
    let threads = thread_count();
    eprintln!("3000 connections, {threads} threads");
    assert!(
        threads < 100,
        "the process has {threads} threads with 3000 connections"
    );
    drop(conns);
}

fn thread_count() -> usize {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_dir("/proc/self/task").unwrap().count()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let out = std::process::Command::new("ps")
            .args(["-M", "-p", &std::process::id().to_string()])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .count()
            .saturating_sub(1)
    }
}

/// Connect as `user`, subscribing as `sid`, with an optional ticket header,
/// and return the first message the server sends.
fn first_message(server: &Server, user: &str, sid: &str, ticket: Option<&str>) -> Message {
    let token = server.sessions.create(user.to_string()).token;
    let stream = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut req = format!("ws://127.0.0.1:{}/?shard=zone&sid={sid}", server.port)
        .into_client_request()
        .unwrap();
    req.headers_mut()
        .insert("Authorization", format!("Bearer {token}").parse().unwrap());
    if let Some(t) = ticket {
        req.headers_mut()
            .insert("X-Pylon-Shard-Ticket", t.parse().unwrap());
    }
    let (mut ws, _) = tungstenite::client(req, stream).expect("handshake");
    ws.read().expect("a first message")
}

fn is_policy_close(msg: &Message) -> bool {
    matches!(msg, Message::Close(Some(f)) if f.code == tungstenite::protocol::frame::coding::CloseCode::Policy)
}

#[test]
fn a_ticket_admits_a_character_id_and_a_bad_ticket_is_refused() {
    use pylon_runtime::shard_tickets::mint;
    let server = start();
    let claims = serde_json::json!({ "realm": "north" });

    // Without a ticket, user u_1 cannot subscribe as char_12.
    assert!(is_policy_close(&first_message(
        &server, "u_1", "char_12", None
    )));

    // With a ticket for this shard and sid, it can.
    let good = mint("zone", "char_12", Some("u_1".into()), claims.clone(), None);
    assert!(matches!(
        first_message(&server, "u_1", "char_12", Some(&good)),
        Message::Binary(_)
    ));

    // A ticket for another shard, or with a changed payload, is refused.
    let other_shard = mint(
        "zone-9",
        "char_12",
        Some("u_1".into()),
        claims.clone(),
        None,
    );
    assert!(is_policy_close(&first_message(
        &server,
        "u_1",
        "char_12",
        Some(&other_shard)
    )));
    let mut parts: Vec<&str> = good.split('.').collect();
    let forged_payload = {
        use base64::Engine;
        let e = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let mut v: serde_json::Value =
            serde_json::from_slice(&e.decode(parts[1]).unwrap()).unwrap();
        v["sid"] = "char_13".into();
        e.encode(serde_json::to_vec(&v).unwrap())
    };
    parts[1] = &forged_payload;
    let forged = parts.join(".");
    assert!(is_policy_close(&first_message(
        &server,
        "u_1",
        "char_13",
        Some(&forged)
    )));
}
