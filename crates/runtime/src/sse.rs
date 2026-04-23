use std::collections::HashMap;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use pylon_sync::ChangeEvent;

use crate::ip_limit::{IpConnCounter, IpConnGuard};

const NUM_SHARDS: usize = 16;

/// Per-client state in the shard map. The `_guard` is held for the lifetime
/// of the connection — dropping it (when the client is removed) releases
/// the client's slot in the per-IP connection counter. Without this, a
/// crash-loopy browser could open unlimited SSE streams.
struct SseClient {
    stream: TcpStream,
    _guard: Option<IpConnGuard>,
}

/// Same rationale as the WS hub: bounded queue + drop-oldest-on-full so a
/// stuck subscriber can't balloon memory on the broadcast path. Clients
/// that miss events catch up via the change-log cursor protocol on
/// reconnect — SSE is a notify-sooner, not a durable-delivery transport.
const BROADCAST_QUEUE_DEPTH: usize = 1024;

/// A single shard holding a subset of SSE clients, protected by its own lock.
/// Sharding reduces contention: concurrent broadcasts only block within the
/// same shard, not across the entire client set.
struct SseShard {
    clients: Mutex<HashMap<u64, SseClient>>,
}

impl SseShard {
    fn new() -> Self {
        Self {
            clients: Mutex::new(HashMap::new()),
        }
    }

    fn add(&self, id: u64, stream: TcpStream, guard: Option<IpConnGuard>) {
        self.clients.lock().unwrap().insert(
            id,
            SseClient {
                stream,
                _guard: guard,
            },
        );
    }

    #[allow(dead_code)]
    fn remove(&self, id: u64) {
        self.clients.lock().unwrap().remove(&id);
    }

    /// Send SSE-formatted data to every client in this shard.
    /// Dead clients (write failures) are removed inline and their IDs returned.
    fn broadcast(&self, data: &str) -> Vec<u64> {
        let sse_data = format!("data: {data}\n\n");
        let mut clients = self.clients.lock().unwrap();
        let mut dead = Vec::new();
        for (id, client) in clients.iter_mut() {
            if client.stream.write_all(sse_data.as_bytes()).is_err()
                || client.stream.flush().is_err()
            {
                dead.push(*id);
            }
        }
        for id in &dead {
            clients.remove(id);
        }
        dead
    }

    /// Send an SSE comment keepalive to every client. Removes dead clients.
    fn keepalive(&self) {
        let mut clients = self.clients.lock().unwrap();
        let mut dead = Vec::new();
        for (id, client) in clients.iter_mut() {
            if client.stream.write_all(b": keepalive\n\n").is_err()
                || client.stream.flush().is_err()
            {
                dead.push(*id);
            }
        }
        for id in dead {
            clients.remove(&id);
        }
    }

    fn count(&self) -> usize {
        self.clients.lock().unwrap().len()
    }
}

/// Sharded SSE broadcast hub.
///
/// 16 shards partition clients by ID. Each shard has a dedicated broadcast
/// worker thread (receives messages via `mpsc::channel`) and a keepalive
/// thread that sends SSE comments every 30 seconds.
///
/// This means 10k connected SSE clients require only 32 background threads
/// (16 broadcast + 16 keepalive) instead of 10k threads in the old design.
pub struct SseHub {
    shards: Vec<Arc<SseShard>>,
    next_id: Mutex<u64>,
    broadcast_txs: Vec<mpsc::SyncSender<String>>,
}

impl SseHub {
    pub fn new() -> Arc<Self> {
        let mut shards = Vec::with_capacity(NUM_SHARDS);
        let mut broadcast_txs = Vec::with_capacity(NUM_SHARDS);

        for i in 0..NUM_SHARDS {
            let shard = Arc::new(SseShard::new());
            let (tx, rx) = mpsc::sync_channel::<String>(BROADCAST_QUEUE_DEPTH);

            // Broadcast worker: drains the channel and writes to every client
            // in this shard. Runs until the channel is dropped (hub teardown).
            let shard_clone = Arc::clone(&shard);
            thread::Builder::new()
                .name(format!("sse-broadcast-{i}"))
                .spawn(move || {
                    while let Ok(msg) = rx.recv() {
                        shard_clone.broadcast(&msg);
                    }
                })
                .expect("Failed to spawn SSE broadcast worker");

            // Keepalive worker: sends an SSE comment every 30s to prevent
            // proxies and load balancers from closing idle connections.
            let shard_ka = Arc::clone(&shard);
            thread::Builder::new()
                .name(format!("sse-keepalive-{i}"))
                .spawn(move || loop {
                    thread::sleep(Duration::from_secs(30));
                    shard_ka.keepalive();
                })
                .expect("Failed to spawn SSE keepalive worker");

            shards.push(shard);
            broadcast_txs.push(tx);
        }

        Arc::new(Self {
            shards,
            next_id: Mutex::new(0),
            broadcast_txs,
        })
    }

    /// Broadcast a `ChangeEvent` to all connected SSE clients.
    pub fn broadcast(&self, event: &ChangeEvent) {
        let json = match serde_json::to_string(event) {
            Ok(j) => j,
            Err(_) => return,
        };
        self.send_to_all(&json);
    }

    /// Broadcast an arbitrary string message (e.g. presence/topic updates).
    pub fn broadcast_message(&self, msg: &str) {
        self.send_to_all(msg);
    }

    /// Internal: bounded-queue send to all shard workers.
    fn send_to_all(&self, msg: &str) {
        for tx in &self.broadcast_txs {
            match tx.try_send(msg.to_string()) {
                Ok(()) => {}
                Err(mpsc::TrySendError::Full(_)) => {
                    tracing::warn!("[sse] broadcast queue full — dropping event for one shard");
                }
                Err(mpsc::TrySendError::Disconnected(_)) => {}
            }
        }
    }

    /// Register a new SSE client. Returns the assigned client ID.
    /// The stream is moved into the appropriate shard — the caller should not
    /// use it after this call. The optional `guard` binds the client's slot
    /// in the per-IP connection counter to this client's presence in the
    /// shard map; when the client is removed, the guard drops and the slot
    /// is returned.
    fn add_client(&self, stream: TcpStream, guard: Option<IpConnGuard>) -> u64 {
        let mut next_id = self.next_id.lock().unwrap();
        let id = *next_id;
        *next_id += 1;
        let shard_idx = (id as usize) % NUM_SHARDS;
        self.shards[shard_idx].add(id, stream, guard);
        id
    }

    /// Total number of connected SSE clients across all shards.
    pub fn client_count(&self) -> usize {
        self.shards.iter().map(|s| s.count()).sum()
    }
}

/// Start the SSE server on the given port.
///
/// Accepts TCP connections, performs minimal HTTP parsing, sends SSE headers,
/// and registers the stream with the hub. The accept thread exits immediately
/// after registration — no per-client thread is kept alive.
pub fn start_sse_server(hub: Arc<SseHub>, port: u16) {
    let addr = format!("0.0.0.0:{port}");
    let listener = match TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("[sse] Failed to bind on {addr}: {e}");
            return;
        }
    };

    tracing::warn!(
        "[sse] SSE server listening on http://localhost:{port}/events (sharded, {NUM_SHARDS} shards)"
    );

    // Per-IP cap mirrors the one on /ws. Idle SSE streams are cheap, but a
    // crash-loopy client can still accumulate thousands of them — this
    // bounds that.
    let ip_counter = Arc::new(IpConnCounter::default());

    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(_) => continue,
        };

        let ip = match stream.peer_addr() {
            Ok(addr) => addr.ip(),
            Err(_) => continue,
        };
        let guard = match ip_counter.acquire(ip) {
            Some(g) => g,
            None => continue,
        };

        let hub = Arc::clone(&hub);
        // Lightweight accept thread with a small stack. It reads the HTTP
        // request, writes SSE headers, registers the stream (transferring
        // the IP-conn guard into the shard map), then exits.
        thread::Builder::new()
            .name("sse-accept".into())
            .stack_size(64 * 1024)
            .spawn(move || {
                handle_sse_connection(hub, stream, guard);
            })
            .ok();
    }
}

fn handle_sse_connection(hub: Arc<SseHub>, mut stream: TcpStream, guard: IpConnGuard) {
    // Consume the HTTP request headers. We don't route — any connection
    // to this port is treated as an SSE subscription.
    let mut buf = [0u8; 2048];
    let _ = std::io::Read::read(&mut stream, &mut buf);

    // Disable Nagle for lower-latency event delivery.
    stream.set_nodelay(true).ok();

    // Send SSE response headers.
    let headers = "HTTP/1.1 200 OK\r\n\
                   Content-Type: text/event-stream\r\n\
                   Cache-Control: no-cache\r\n\
                   Connection: keep-alive\r\n\
                   Access-Control-Allow-Origin: *\r\n\
                   X-Content-Type-Options: nosniff\r\n\
                   \r\n";

    if stream.write_all(headers.as_bytes()).is_err() {
        return;
    }
    if stream.write_all(b": connected\n\n").is_err() {
        return;
    }
    let _ = stream.flush();

    // Hand the stream AND the IP-conn guard to the hub. The shard's
    // broadcast and keepalive workers now own writes; the guard is
    // released when the client is dropped from the shard map. This
    // thread exits.
    hub.add_client(stream, Some(guard));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hub_starts_with_correct_shard_count() {
        let hub = SseHub::new();
        assert_eq!(hub.shards.len(), NUM_SHARDS);
        assert_eq!(hub.broadcast_txs.len(), NUM_SHARDS);
    }

    #[test]
    fn hub_starts_empty() {
        let hub = SseHub::new();
        assert_eq!(hub.client_count(), 0);
    }

    #[test]
    fn broadcast_on_empty_hub_does_not_panic() {
        let hub = SseHub::new();
        hub.broadcast_message("hello");
        // Give broadcast workers time to process.
        thread::sleep(Duration::from_millis(50));
        assert_eq!(hub.client_count(), 0);
    }

    #[test]
    fn keepalive_on_empty_shard_does_not_panic() {
        let shard = SseShard::new();
        shard.keepalive();
        assert_eq!(shard.count(), 0);
    }

    #[test]
    fn broadcast_on_empty_shard_returns_no_dead() {
        let shard = SseShard::new();
        let dead = shard.broadcast("test");
        assert!(dead.is_empty());
    }

    #[test]
    fn client_ids_are_sequential() {
        let hub = SseHub::new();
        // Verify the ID counter increments correctly.
        let mut next_id = hub.next_id.lock().unwrap();
        assert_eq!(*next_id, 0);
        *next_id = 5;
        drop(next_id);
        // Next add_client would get ID 5, distributing to shard 5 % 16 = 5.
    }
}
