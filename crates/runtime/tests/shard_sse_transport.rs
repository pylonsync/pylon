//! Shard SSE transport (`GET /api/shards/:id/connect`) with a client that
//! stops reading.
//!
//! One shard ticks at 20 Hz with 64 KB snapshots. A `stalled` client opens
//! the stream and never reads; an `active` client reads it. The active
//! client must get (almost) every tick, and the stalled subscriber must be
//! removed once its outbound queue has been full for
//! `slow_subscriber_timeout`.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pylon_kernel::AppManifest;
use pylon_realtime::{DynShardRegistry, Shard, ShardConfig, ShardRegistry, SimState, SubscriberId};
use pylon_runtime::Runtime;

struct Zone;

impl SimState for Zone {
    type Input = i64;
    type Snapshot = String;
    type Error = String;

    fn apply_input(&mut self, _s: &SubscriberId, _i: i64, _now: Instant) -> Result<(), String> {
        Ok(())
    }
    fn tick(&mut self, _dt: Duration) {}
    fn snapshot(&self) -> String {
        "x".repeat(64 * 1024)
    }
}

fn available_port() -> u16 {
    // One lane per test binary, below the ephemeral range.
    static NEXT: AtomicU16 = AtomicU16::new(26_000);
    for _ in 0..200 {
        let base = NEXT.fetch_add(4, Ordering::Relaxed);
        if (0..4).all(|off| std::net::TcpListener::bind(("127.0.0.1", base + off)).is_ok()) {
            return base;
        }
    }
    panic!("no free 4-port block");
}

fn http(port: u16, method: &str, path: &str, token: Option<&str>, body: &str) -> (u16, String) {
    let auth = token
        .map(|t| format!("Authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://127.0.0.1:{port}\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).ok();
    s.write_all(req.as_bytes()).unwrap();
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    let text = String::from_utf8_lossy(&out).to_string();
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let body = text
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_string())
        .unwrap_or_default();
    (status, body)
}

/// A guest session: (token, user id).
fn guest(port: u16) -> (String, String) {
    let (status, body) = http(port, "POST", "/api/auth/guest", None, "{}");
    assert_eq!(status, 201, "guest auth: {body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    (
        v["token"].as_str().unwrap().to_string(),
        v["user_id"].as_str().unwrap().to_string(),
    )
}

/// Open the SSE stream and return the socket after the request is sent.
fn open_stream(port: u16, token: &str, sid: &str) -> TcpStream {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let req = format!(
        "GET /api/shards/zone/connect?sid={sid} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nAccept: text/event-stream\r\n\r\n"
    );
    s.write_all(req.as_bytes()).unwrap();
    s
}

/// Start a server with one 20 Hz shard `zone`. Returns the port and the
/// registry.
fn start(config: ShardConfig) -> (u16, Arc<ShardRegistry<Zone>>) {
    unsafe {
        std::env::set_var("PYLON_DEV_MODE", "1");
    }
    let registry: Arc<ShardRegistry<Zone>> = Arc::new(ShardRegistry::new());
    registry.insert(Shard::new("zone", Zone, config));
    let port = available_port();
    let rt = Arc::new(
        Runtime::in_memory(AppManifest {
            manifest_version: 1,
            name: "shards".into(),
            version: "0".into(),
            ..Default::default()
        })
        .unwrap(),
    );
    let dyn_registry: Arc<dyn DynShardRegistry> = registry.clone();
    std::thread::spawn(move || {
        let _ = pylon_runtime::server::start_with_shards(rt, port, None, dyn_registry);
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < deadline, "server never bound {port}");
        std::thread::sleep(Duration::from_millis(50));
    }
    (port, registry)
}

#[test]
fn a_stalled_sse_client_does_not_hold_back_the_others() {
    let (port, registry) = start(ShardConfig {
        tick_rate_hz: 20,
        slow_subscriber_timeout: Duration::from_secs(2),
        // 8 frames = 0.4 s of buffering before the queue counts as full.
        outbound_queue_frames: 8,
        idle_ticks_before_shutdown: 0,
        ..Default::default()
    });
    let shard = registry.get("zone").unwrap();

    let (stalled_token, stalled_id) = guest(port);
    let _stalled = open_stream(port, &stalled_token, &stalled_id);

    let (token, id) = guest(port);
    let mut active = open_stream(port, &token, &id);
    active
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();

    // Read for 4 s, collecting the `id: <tick>` lines.
    let start_tick = shard.tick_number();
    let start = Instant::now();
    let mut buf = Vec::new();
    let mut chunk = vec![0u8; 256 * 1024];
    while start.elapsed() < Duration::from_secs(4) {
        match active.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) => {}
        }
    }
    let ticks_run = shard.tick_number() - start_tick;
    let text = String::from_utf8_lossy(&buf);
    let ticks_seen = text.matches("\nid: ").count() + usize::from(text.starts_with("id: "));

    // A stalled tick loop would run almost no ticks. A slow CI runner runs
    // fewer than 80 but far more than half; that is the bound checked here.
    // The per-client check below stays exact.
    assert!(
        ticks_run >= 40,
        "the shard ran only {ticks_run} ticks in 4 s (target 80)"
    );
    assert!(
        ticks_seen as u64 + 5 >= ticks_run,
        "the active client saw {ticks_seen} of {ticks_run} ticks"
    );
    // The stalled queue fills (after the socket and stream buffers do),
    // then closes 2 s later.
    let deadline = Instant::now() + Duration::from_secs(15);
    while shard.subscriber_count() > 1 {
        assert!(
            Instant::now() < deadline,
            "the stalled subscriber was never removed ({} subscribers)",
            shard.subscriber_count()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn http_inputs_over_the_subscriber_limit_get_429() {
    let (port, _registry) = start(ShardConfig {
        tick_rate_hz: 20,
        input_rate_per_subscriber: 1.0,
        input_burst_per_subscriber: 3.0,
        idle_ticks_before_shutdown: 0,
        ..Default::default()
    });
    let (token, id) = guest(port);
    // The input route only accepts inputs from an attached subscriber.
    let _stream = open_stream(port, &token, &id);
    std::thread::sleep(Duration::from_millis(300));

    let statuses: Vec<u16> = (0..6)
        .map(|_| {
            http(
                port,
                "POST",
                "/api/shards/zone/input",
                Some(&token),
                r#"{"input": 1}"#,
            )
            .0
        })
        .collect();
    assert_eq!(
        &statuses[..3],
        &[200, 200, 200],
        "the burst is accepted: {statuses:?}"
    );
    assert!(
        statuses[3..].iter().all(|s| *s == 429),
        "then 429: {statuses:?}"
    );
}
