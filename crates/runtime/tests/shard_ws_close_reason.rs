//! The request log names why a shard connection ended.
//!
//! Its own test binary: the log ring is process-wide, and other transport
//! tests log "client too slow" on purpose.

use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pylon_auth::SessionStore;
use pylon_realtime::{DynShardRegistry, Shard, ShardConfig, ShardRegistry, SimState, SubscriberId};
use tungstenite::client::IntoClientRequest;

struct Zone;

impl SimState for Zone {
    type Input = i64;
    type Snapshot = String;
    type Error = String;

    fn apply_input(&mut self, _: &SubscriberId, _: i64, _: Instant) -> Result<(), String> {
        Ok(())
    }
    fn tick(&mut self, _dt: Duration) {}
    fn snapshot(&self) -> String {
        // Large, so a client that stops reading fills the socket buffers.
        "x".repeat(64 * 1024)
    }
}

/// A shard that stops while a connection's writer is stuck in a send (its
/// client stopped reading) logs "shard stopped", not "client too slow".
/// The reader sees the closed queue first; it used to name a slow client.
#[test]
fn a_stopped_shard_logs_shard_stopped() {
    pylon_runtime::log_ring::init_log_ring();
    let registry: Arc<ShardRegistry<Zone>> = Arc::new(ShardRegistry::new());
    registry.insert(Shard::new(
        "zone",
        Zone,
        ShardConfig {
            tick_rate_hz: 20,
            idle_ticks_before_shutdown: 0,
            // Longer than the test: the queue must not close for being
            // full, only for the stop.
            slow_subscriber_timeout: Duration::from_secs(60),
            outbound_queue_frames: 1024,
            ..Default::default()
        },
    ));
    let sessions = Arc::new(SessionStore::new());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let dyn_registry: Arc<dyn DynShardRegistry> = registry.clone();
    let s = Arc::clone(&sessions);
    std::thread::spawn(move || {
        pylon_runtime::shard_ws::serve(
            listener,
            dyn_registry,
            pylon_runtime::request_auth::AuthResolver::from_sessions(s),
            0,
        )
    });

    let token = sessions.create("watcher".to_string()).token;
    let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut req = format!("ws://127.0.0.1:{port}/?shard=zone&sid=watcher")
        .into_client_request()
        .unwrap();
    req.headers_mut()
        .insert("Authorization", format!("Bearer {token}").parse().unwrap());
    let (mut ws, _) = tungstenite::client(req, stream).expect("handshake");
    ws.read().expect("a first snapshot");
    // Stop reading until the socket buffers are full and the writer is
    // stuck in a send.
    std::thread::sleep(Duration::from_secs(2));

    registry.get("zone").unwrap().stop();
    // Keep the writer stuck past the reader's next check of the queue.
    std::thread::sleep(Duration::from_millis(1500));
    ws.get_ref()
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    while ws.read().is_ok() {}

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let rows = pylon_runtime::log_ring::log_ring()
            .unwrap()
            .tail_since(None);
        let ends: Vec<_> = rows.iter().filter(|r| r.method == "WS").collect();
        if let Some(row) = ends.first() {
            assert_eq!(row.error.as_deref(), Some("shard stopped"), "{ends:?}");
            break;
        }
        assert!(Instant::now() < deadline, "no log row for the connection");
        std::thread::sleep(Duration::from_millis(20));
    }
}
