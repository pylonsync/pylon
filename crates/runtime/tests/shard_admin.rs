//! Shard numbers and operator actions over HTTP: `/metrics` (Prometheus and
//! JSON), the admin-only `/api/shards` list and detail, and stopping a shard
//! and kicking a subscriber.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pylon_kernel::AppManifest;
use pylon_realtime::{DynShardRegistry, Shard, ShardConfig, ShardRegistry, SimState, SubscriberId};
use pylon_runtime::Runtime;

const ADMIN: &str = "shard-admin-test-token-0123456789";

struct Zone(u64);

impl SimState for Zone {
    type Input = u64;
    type Snapshot = u64;
    type Error = String;

    fn apply_input(&mut self, _s: &SubscriberId, i: u64, _now: Instant) -> Result<(), String> {
        self.0 += i;
        Ok(())
    }
    fn tick(&mut self, _dt: Duration) {}
    fn snapshot(&self) -> u64 {
        self.0
    }
}

fn available_port() -> u16 {
    static NEXT: AtomicU16 = AtomicU16::new(27_000);
    for _ in 0..200 {
        let base = NEXT.fetch_add(4, Ordering::Relaxed);
        if (0..4).all(|off| std::net::TcpListener::bind(("127.0.0.1", base + off)).is_ok()) {
            return base;
        }
    }
    panic!("no free 4-port block");
}

fn request(port: u16, method: &str, path: &str, headers: &str, body: &str) -> (u16, String) {
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://127.0.0.1:{port}\r\n{headers}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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

fn bearer(token: &str) -> String {
    format!("Authorization: Bearer {token}\r\n")
}

fn json(body: &str) -> serde_json::Value {
    serde_json::from_str(body).unwrap_or_else(|e| panic!("{e}: {body}"))
}

fn ws(port: u16, token: &str, sid: &str) -> tungstenite::WebSocket<TcpStream> {
    use tungstenite::client::IntoClientRequest;
    let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut req = format!("ws://127.0.0.1:{port}/shard?shard=zone&sid={sid}&v=2")
        .into_client_request()
        .unwrap();
    req.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        format!("bearer.{token}").parse().unwrap(),
    );
    tungstenite::client(req, stream)
        .expect("handshake on /shard")
        .0
}

fn wait_until(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ok() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn operators_read_shard_numbers_stop_shards_and_kick_subscribers() {
    unsafe {
        std::env::set_var("PYLON_DEV_MODE", "1");
        std::env::set_var("PYLON_ADMIN_TOKEN", ADMIN);
        std::env::set_var("PYLON_SHARD_WS_MAX_PER_IP", "0");
    }
    let registry: Arc<ShardRegistry<Zone>> = Arc::new(ShardRegistry::new());
    registry.insert(Shard::new(
        "zone",
        Zone(0),
        ShardConfig {
            tick_rate_hz: 20,
            idle_ticks_before_shutdown: 0,
            ..Default::default()
        },
    ));
    // A match id with `:`, which clients percent-encode in the path.
    registry.insert(Shard::new("match:42", Zone(0), ShardConfig::default()));
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
    wait_until("the server", || {
        TcpStream::connect(("127.0.0.1", port)).is_ok()
    });
    let shard = registry.get("zone").unwrap();

    // A player connects and gets frames.
    let (status, body) = request(port, "POST", "/api/auth/guest", "", "{}");
    assert_eq!(status, 201, "{body}");
    let g = json(&body);
    let (token, uid) = (g["token"].as_str().unwrap(), g["user_id"].as_str().unwrap());
    let mut player = ws(port, token, uid);
    player.read().expect("a frame");
    wait_until("some ticks with the player", || {
        shard.stats().bytes_total > 0
    });

    // The list and the detail are admin-only.
    for path in ["/api/shards", "/api/shards/zone"] {
        let (status, _) = request(port, "GET", path, &bearer(token), "");
        assert_eq!(status, 403, "{path} as a player");
    }
    let (status, body) = request(port, "GET", "/api/shards", &bearer(ADMIN), "");
    assert_eq!(status, 200, "{body}");
    let list = json(&body);
    let zone = list
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "zone")
        .unwrap();
    assert_eq!(zone["subscribers"], 1);
    assert!(zone["stats"]["ticks"].as_u64().unwrap() > 0);
    assert!(zone["stats"]["bytes_total"].as_u64().unwrap() > 0);
    assert!(zone["stats"]["tick_ms"]["p99"].as_f64().is_some());

    let (status, body) = request(port, "GET", "/api/shards/zone", &bearer(ADMIN), "");
    assert_eq!(status, 200, "{body}");
    let detail = json(&body);
    assert_eq!(detail["subscriber_ids"][0]["id"], uid);
    assert_eq!(detail["subscriber_ids"][0]["connections"], 1);

    // Prometheus text and the JSON Studio reads.
    let (status, text) = request(
        port,
        "GET",
        "/metrics",
        &format!("{}Accept: text/plain\r\n", bearer(ADMIN)),
        "",
    );
    assert_eq!(status, 200);
    for family in [
        "pylon_shards 2",
        "pylon_shard_subscribers{shard=\"zone\",kind=\"\"} 1",
        "pylon_shard_ticks_total{shard=\"zone\"",
        "pylon_shard_tick_duration_seconds{shard=\"zone\",kind=\"\",quantile=\"0.99\"}",
        "pylon_shard_phase_seconds_total{shard=\"zone\",kind=\"\",phase=\"encode\"}",
        "pylon_shard_subscriber_tick_bytes{shard=\"zone\",kind=\"\",quantile=\"0.5\"}",
        "pylon_shard_dropped_inputs_total{shard=\"zone\",kind=\"\",reason=\"queue_full\"} 0",
    ] {
        assert!(text.contains(family), "{family} not in\n{text}");
    }
    let (_, body) = request(port, "GET", "/metrics", &bearer(ADMIN), "");
    let shards = json(&body)["shards"].clone();
    assert!(shards.as_array().unwrap().iter().any(|s| s["id"] == "zone"));

    // Kick: admin-only, and it closes the player's connection.
    let kick = format!("{{\"subscriber\":\"{uid}\"}}");
    let (status, _) = request(port, "POST", "/api/shards/zone/kick", &bearer(token), &kick);
    assert_eq!(status, 403);
    let (status, body) = request(port, "POST", "/api/shards/zone/kick", &bearer(ADMIN), &kick);
    assert_eq!(status, 200, "{body}");
    wait_until("the player removed", || shard.subscriber_count() == 0);
    let closed = (0..100).any(|_| player.read().is_err());
    assert!(closed, "the kicked player's socket closes");
    let (status, _) = request(port, "POST", "/api/shards/zone/kick", &bearer(ADMIN), &kick);
    assert_eq!(status, 404, "kicking someone not there");

    // Stop: admin-only.
    let (status, _) = request(port, "POST", "/api/shards/zone/stop", &bearer(token), "{}");
    assert_eq!(status, 403);
    assert!(shard.is_running());
    let (status, body) = request(port, "POST", "/api/shards/zone/stop", &bearer(ADMIN), "{}");
    assert_eq!(status, 200, "{body}");
    assert!(!shard.is_running());
    let (status, _) = request(port, "POST", "/api/shards/nope/stop", &bearer(ADMIN), "{}");
    assert_eq!(status, 404);

    // Encoded ids reach the shard.
    let (status, body) = request(port, "GET", "/api/shards/match%3A42", &bearer(ADMIN), "");
    assert_eq!(status, 200, "{body}");
    assert_eq!(json(&body)["id"], "match:42");
    let (status, _) = request(
        port,
        "POST",
        "/api/shards/match%3A42/stop",
        &bearer(ADMIN),
        "{}",
    );
    assert_eq!(status, 200);
    assert!(!registry.get("match:42").unwrap().is_running());
}
