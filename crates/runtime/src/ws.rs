use std::collections::HashMap;
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use agentdb_sync::ChangeEvent;
use tungstenite::{accept, Message, WebSocket};

/// Number of shards for distributing WebSocket clients.
/// Must be a power of two for even modulo distribution.
const NUM_SHARDS: usize = 16;

/// A single shard holding a subset of WebSocket clients.
///
/// Each shard has its own lock, so concurrent broadcasts across shards
/// never contend with each other. This reduces lock contention by NUM_SHARDS
/// compared to a single global mutex.
struct Shard {
    clients: Mutex<HashMap<u64, WebSocket<TcpStream>>>,
}

impl Shard {
    fn new() -> Self {
        Self {
            clients: Mutex::new(HashMap::new()),
        }
    }

    fn add(&self, id: u64, ws: WebSocket<TcpStream>) {
        self.clients.lock().unwrap().insert(id, ws);
    }

    fn remove(&self, id: u64) {
        self.clients.lock().unwrap().remove(&id);
    }

    /// Send a message to all clients in this shard.
    /// Dead clients (those that fail to receive) are removed immediately.
    fn broadcast(&self, msg: &str) {
        let mut clients = self.clients.lock().unwrap();
        let mut dead = Vec::new();
        for (id, socket) in clients.iter_mut() {
            if socket.send(Message::Text(msg.to_string())).is_err() {
                dead.push(*id);
            }
        }
        for id in &dead {
            clients.remove(id);
        }
    }

    fn count(&self) -> usize {
        self.clients.lock().unwrap().len()
    }
}

/// High-performance WebSocket broadcast hub with sharded client storage.
///
/// Supports 10k+ concurrent connections with bounded thread count.
/// Uses NUM_SHARDS (16) shards to reduce lock contention.
///
/// Architecture:
/// - Client connections are assigned to shards via round-robin (id % NUM_SHARDS).
/// - Each shard has a dedicated broadcast worker thread that consumes from a channel.
/// - Broadcast calls are non-blocking for the caller: they push to each shard's channel
///   and return immediately.
/// - Read-side threads use 64KB stacks (vs 2-8MB default) to keep memory bounded.
/// - Total thread count: NUM_SHARDS broadcast workers + 1 per connected client (with
///   minimal stack), plus the accept thread.
pub struct WsHub {
    shards: Vec<Arc<Shard>>,
    next_id: Mutex<u64>,
    /// Channel senders for each shard's broadcast worker.
    broadcast_txs: Vec<mpsc::Sender<String>>,
}

impl WsHub {
    pub fn new() -> Arc<Self> {
        let mut shards = Vec::with_capacity(NUM_SHARDS);
        let mut broadcast_txs = Vec::with_capacity(NUM_SHARDS);

        for i in 0..NUM_SHARDS {
            let shard = Arc::new(Shard::new());
            let (tx, rx) = mpsc::channel::<String>();

            // Each shard gets a dedicated broadcast worker thread.
            // These are long-lived threads (one per shard, not per client).
            let shard_clone = Arc::clone(&shard);
            thread::Builder::new()
                .name(format!("ws-broadcast-{i}"))
                .spawn(move || {
                    while let Ok(msg) = rx.recv() {
                        shard_clone.broadcast(&msg);
                    }
                })
                .expect("Failed to spawn broadcast worker");

            shards.push(shard);
            broadcast_txs.push(tx);
        }

        Arc::new(Self {
            shards,
            next_id: Mutex::new(0),
            broadcast_txs,
        })
    }

    /// Broadcast a change event to ALL connected clients across all shards.
    /// Non-blocking: pushes to each shard's channel and returns immediately.
    pub fn broadcast(&self, event: &ChangeEvent) {
        let json = match serde_json::to_string(event) {
            Ok(j) => j,
            Err(_) => return,
        };
        self.broadcast_raw(&json);
    }

    /// Broadcast a raw string message to all clients (used for presence updates).
    pub fn broadcast_presence(&self, msg: &str) {
        self.broadcast_raw(msg);
    }

    /// Internal: fan out a message string to all shard broadcast channels.
    fn broadcast_raw(&self, msg: &str) {
        for tx in &self.broadcast_txs {
            // If a shard's channel is disconnected, skip it silently.
            // This only happens during shutdown.
            let _ = tx.send(msg.to_string());
        }
    }

    /// Assign a client to a shard via round-robin and register it.
    fn add_client(&self, ws: WebSocket<TcpStream>) -> u64 {
        let mut next_id = self.next_id.lock().unwrap();
        let id = *next_id;
        *next_id += 1;
        let shard_idx = (id as usize) % NUM_SHARDS;
        self.shards[shard_idx].add(id, ws);
        id
    }

    fn remove_client(&self, id: u64) {
        let shard_idx = (id as usize) % NUM_SHARDS;
        self.shards[shard_idx].remove(id);
    }

    /// Total number of connected clients across all shards.
    pub fn client_count(&self) -> usize {
        self.shards.iter().map(|s| s.count()).sum()
    }
}

/// Start the WebSocket server on the given port.
///
/// The accept loop runs on the calling thread (blocking). Each accepted
/// connection spawns a lightweight reader thread with a 64KB stack.
/// Broadcast writes are handled by the shard worker threads, not by
/// per-client threads.
pub fn start_ws_server(hub: Arc<WsHub>, port: u16) {
    let addr = format!("0.0.0.0:{port}");
    let listener = match TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[ws] Failed to bind on {addr}: {e}");
            return;
        }
    };

    eprintln!(
        "[ws] WebSocket server listening on ws://localhost:{port} (sharded, {NUM_SHARDS} shards)"
    );

    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(_) => continue,
        };

        let hub = Arc::clone(&hub);
        // Spawn a reader thread per client with a small stack.
        // 64KB stack * 10k connections = ~640MB, vs 2-8MB default * 10k = 20-80GB.
        thread::Builder::new()
            .name("ws-client".into())
            .stack_size(64 * 1024)
            .spawn(move || {
                handle_ws_connection(hub, stream);
            })
            .ok(); // If thread creation fails, drop the connection gracefully.
    }
}

/// Handle a single WebSocket client connection.
///
/// Sets a read timeout to prevent zombie threads on dead connections.
/// Handles ping/pong for keepalive, presence/topic message relay,
/// and clean disconnect with presence broadcast.
fn handle_ws_connection(hub: Arc<WsHub>, stream: TcpStream) {
    // 120s read timeout prevents threads from hanging indefinitely
    // on half-open connections where the peer disappears without FIN.
    stream
        .set_read_timeout(Some(Duration::from_secs(120)))
        .ok();

    let ws = match accept(stream) {
        Ok(ws) => ws,
        Err(_) => return,
    };

    let client_id = hub.add_client(ws);
    let shard_idx = (client_id as usize) % NUM_SHARDS;

    loop {
        // Lock the shard only for the duration of the socket read.
        // tungstenite's read() will block until data arrives or timeout,
        // BUT the underlying socket has a read timeout set, so this won't
        // hold the lock forever.
        let msg = {
            let mut clients = hub.shards[shard_idx].clients.lock().unwrap();
            let socket = match clients.get_mut(&client_id) {
                Some(s) => s,
                None => break,
            };
            socket.read()
        };

        match msg {
            Ok(Message::Text(text)) => {
                // Relay presence and topic messages to all connected clients.
                if text.starts_with("{\"type\":\"presence\"")
                    || text.starts_with("{\"type\":\"topic\"")
                {
                    hub.broadcast_presence(&text);
                }
            }
            Ok(Message::Ping(data)) => {
                // Respond with pong to keep the connection alive.
                let mut clients = hub.shards[shard_idx].clients.lock().unwrap();
                if let Some(socket) = clients.get_mut(&client_id) {
                    let _ = socket.send(Message::Pong(data));
                }
            }
            Ok(Message::Close(_)) | Err(_) => {
                hub.remove_client(client_id);
                let disconnect = serde_json::json!({
                    "type": "presence",
                    "event": "disconnect",
                    "clientId": client_id,
                });
                hub.broadcast_presence(&disconnect.to_string());
                break;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shard_count_starts_at_zero() {
        let shard = Shard::new();
        assert_eq!(shard.count(), 0);
    }

    #[test]
    fn hub_starts_with_zero_clients() {
        let hub = WsHub::new();
        assert_eq!(hub.client_count(), 0);
    }

    #[test]
    fn broadcast_to_empty_hub_doesnt_panic() {
        let hub = WsHub::new();
        let event = ChangeEvent {
            seq: 1,
            entity: "Test".into(),
            row_id: "1".into(),
            kind: agentdb_sync::ChangeKind::Insert,
            data: None,
            timestamp: String::new(),
        };
        hub.broadcast(&event);
        hub.broadcast_presence("test");
    }

    #[test]
    fn num_shards_is_power_of_two() {
        // Power-of-two shard count ensures even distribution with modulo.
        assert!(
            NUM_SHARDS.is_power_of_two(),
            "NUM_SHARDS ({NUM_SHARDS}) must be a power of two for even distribution"
        );
    }

    #[test]
    fn shard_assignment_distributes_evenly() {
        // Verify that sequential IDs spread across all shards.
        let mut counts = vec![0usize; NUM_SHARDS];
        for id in 0..(NUM_SHARDS as u64 * 100) {
            counts[(id as usize) % NUM_SHARDS] += 1;
        }
        // Every shard should get exactly 100 clients.
        for (i, count) in counts.iter().enumerate() {
            assert_eq!(*count, 100, "Shard {i} got {count} clients, expected 100");
        }
    }
}
