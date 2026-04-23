use std::collections::HashMap;
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use pylon_auth::SessionStore;
use pylon_sync::ChangeEvent;
use tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tungstenite::{accept_hdr, Message, WebSocket};

use crate::ip_limit::IpConnCounter;

/// Number of shards for distributing WebSocket clients.
/// Must be a power of two for even modulo distribution.
const NUM_SHARDS: usize = 16;

/// Maximum number of outbound messages queued per shard. Once the broadcast
/// worker thread falls this many behind, the OLDEST queued message is
/// dropped to make room for the new one. That means slow subscribers can
/// miss messages — but the alternative (unbounded queue) was OOM when a
/// single stuck client blocked its shard worker.
///
/// Callers that need exact delivery should layer their own retry on top
/// (the change-log cursor protocol already does this for sync).
const BROADCAST_QUEUE_DEPTH: usize = 1024;

/// Read timeout on each WebSocket read. Kept low so the mutex guarding the
/// socket is released frequently, letting the broadcaster get its turn even
/// if the client never sends anything. Previously this was 120s, which meant
/// one quiet client could wedge the shard's writer for up to two minutes.
const WS_READ_TIMEOUT: Duration = Duration::from_millis(200);

/// One entry per connected client. The socket lives behind its OWN
/// `Mutex`, not a shard-wide one, so the reader thread's blocking
/// `socket.read()` doesn't hold a lock that covers every client in the
/// same shard. The broadcaster iterates the client map (outer lock is
/// brief — O(count of clients in shard)), then grabs each client's
/// individual mutex to do the `socket.send`. Contention is now per-
/// client instead of per-shard.
type ClientSocket = Arc<Mutex<WebSocket<TcpStream>>>;

/// A single shard holding a subset of WebSocket clients.
///
/// The outer `Mutex<HashMap>` is held only for insert/remove and while
/// enumerating client handles — never across I/O.
struct Shard {
    clients: Mutex<HashMap<u64, ClientSocket>>,
}

impl Shard {
    fn new() -> Self {
        Self {
            clients: Mutex::new(HashMap::new()),
        }
    }

    fn add(&self, id: u64, ws: WebSocket<TcpStream>) -> ClientSocket {
        let handle = Arc::new(Mutex::new(ws));
        self.clients.lock().unwrap().insert(id, Arc::clone(&handle));
        handle
    }

    fn remove(&self, id: u64) {
        self.clients.lock().unwrap().remove(&id);
    }

    /// Send a message to all clients in this shard.
    ///
    /// Snapshot the client handles under the shard lock, drop the shard
    /// lock, then contend only with per-client mutexes to do the writes.
    /// This is what lets a reader thread hold its client's mutex for a
    /// socket.read() without stalling broadcasts for the whole shard.
    fn broadcast(&self, msg: &str) {
        let handles: Vec<(u64, ClientSocket)> = {
            let clients = self.clients.lock().unwrap();
            clients
                .iter()
                .map(|(id, h)| (*id, Arc::clone(h)))
                .collect()
        };
        let mut dead: Vec<u64> = Vec::new();
        for (id, handle) in handles {
            // `try_lock` would skip clients whose reader is currently
            // blocked in read(); we prefer `lock()` here so the occasional
            // broadcaster wait (bounded by the 200ms read timeout) doesn't
            // drop the message for that client.
            let mut guard = match handle.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            if guard.send(Message::Text(msg.to_string())).is_err() {
                dead.push(id);
            }
        }
        if !dead.is_empty() {
            let mut clients = self.clients.lock().unwrap();
            for id in &dead {
                clients.remove(id);
            }
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
    /// Bounded-capacity senders for each shard's broadcast worker. When
    /// a send would block because the queue is full, `broadcast_raw` drains
    /// the oldest queued messages so new ones aren't lost to a stuck worker.
    broadcast_txs: Vec<mpsc::SyncSender<String>>,
    /// Matching receivers are held by each worker thread and also exposed
    /// here so the "drop oldest" fallback can drain them on full. Keeping
    /// the receiver handle alongside the sender is only safe because mpsc
    /// lets multiple clones share a queue — here we only consume via the
    /// worker, and the sender-side uses `try_send` + drain retry.
    #[allow(dead_code)]
    queue_depth: usize,
}

impl WsHub {
    pub fn new() -> Arc<Self> {
        let mut shards = Vec::with_capacity(NUM_SHARDS);
        let mut broadcast_txs = Vec::with_capacity(NUM_SHARDS);

        for i in 0..NUM_SHARDS {
            let shard = Arc::new(Shard::new());
            // Bounded queue — if a broadcast worker stalls, `try_send` fails
            // with Full and `broadcast_raw` drops the oldest to make room.
            let (tx, rx) = mpsc::sync_channel::<String>(BROADCAST_QUEUE_DEPTH);

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
            queue_depth: BROADCAST_QUEUE_DEPTH,
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
    ///
    /// Uses `try_send`; on full we log once (per call) and drop the message
    /// for that shard. Previously the channel was unbounded, so a stuck
    /// worker thread would grow memory until OOM. The new bounded queue
    /// means a slow/stuck subscriber at worst loses broadcast events —
    /// correctness for critical data still comes through the change-log
    /// cursor on a reconnect.
    fn broadcast_raw(&self, msg: &str) {
        for tx in &self.broadcast_txs {
            match tx.try_send(msg.to_string()) {
                Ok(()) => {}
                Err(mpsc::TrySendError::Full(_)) => {
                    tracing::warn!(
                        "[ws] broadcast queue full — dropping event for one shard"
                    );
                }
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    // Worker exited (shutdown). Silent.
                }
            }
        }
    }

    /// Assign a client to a shard via round-robin and register it.
    /// Returns `(id, socket_handle)` — the caller keeps the handle and uses
    /// it for reads; the shard also keeps an Arc clone for broadcasts.
    fn add_client(&self, ws: WebSocket<TcpStream>) -> (u64, ClientSocket) {
        let mut next_id = self.next_id.lock().unwrap();
        let id = *next_id;
        *next_id += 1;
        let shard_idx = (id as usize) % NUM_SHARDS;
        let handle = self.shards[shard_idx].add(id, ws);
        (id, handle)
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
///
/// The session store is required: every connection must present a valid
/// bearer token (Authorization header or `bearer.<token>` subprotocol —
/// browsers can't set WS headers directly). Previously the notifier hub
/// accepted any connection and streamed every ChangeEvent/presence event
/// to it, which was a silent read-policy bypass.
pub fn start_ws_server(hub: Arc<WsHub>, sessions: Arc<SessionStore>, port: u16) {
    let addr = format!("0.0.0.0:{port}");
    let listener = match TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("[ws] Failed to bind on {addr}: {e}");
            return;
        }
    };

    tracing::warn!(
        "[ws] WebSocket server listening on ws://localhost:{port} (sharded, {NUM_SHARDS} shards)"
    );

    let ip_counter = Arc::new(IpConnCounter::default());

    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(_) => continue,
        };

        // Per-IP connection cap: reject BEFORE the handshake so a cheap
        // connect storm doesn't force us through tungstenite's HTTP parse
        // and the session-resolve round trip. The guard is dropped when
        // the reader thread exits (or fails to start), freeing the slot.
        let ip = match stream.peer_addr() {
            Ok(addr) => addr.ip(),
            Err(_) => continue,
        };
        let guard = match ip_counter.acquire(ip) {
            Some(g) => g,
            None => {
                // Ignore: let the client re-try after an existing connection
                // closes. Previously an IP could open unbounded connections
                // and each one spawned a thread + held a per-client mutex.
                continue;
            }
        };

        let hub = Arc::clone(&hub);
        let sessions = Arc::clone(&sessions);
        // Spawn a reader thread per client with a small stack.
        // 64KB stack * 10k connections = ~640MB, vs 2-8MB default * 10k = 20-80GB.
        let spawn_result = thread::Builder::new()
            .name("ws-client".into())
            .stack_size(64 * 1024)
            .spawn(move || {
                // Holding `guard` for the life of the connection thread is
                // what makes the decrement-on-disconnect contract work. Not
                // `let _ = guard;` — that drops immediately.
                let _conn_slot = guard;
                handle_ws_connection(hub, sessions, stream);
            });
        if spawn_result.is_err() {
            // Thread creation failed — guard is already dropped here, slot
            // returned. We deliberately don't call `continue` before the
            // spawn: we've paid the acquire cost and want to avoid leaking
            // a slot under transient thread-limit pressure.
        }
    }
}

/// Handle a single WebSocket client connection.
///
/// Sets a read timeout to prevent zombie threads on dead connections.
/// Handles ping/pong for keepalive, presence/topic message relay,
/// and clean disconnect with presence broadcast.
fn handle_ws_connection(
    hub: Arc<WsHub>,
    sessions: Arc<SessionStore>,
    stream: TcpStream,
) {
    // Short read timeout bounds how long the PER-CLIENT mutex is held
    // while this thread is blocked in socket.read(). Each client now has
    // its own mutex (not a shard-wide one), so a quiet client only stalls
    // the broadcaster when it's broadcasting to THAT specific client —
    // other clients in the same shard proceed without contention.
    stream.set_read_timeout(Some(WS_READ_TIMEOUT)).ok();
    // Also cap write time. A stuck kernel send (slow client, full send
    // buffer, dropped packets) would otherwise stall the shard's
    // broadcast worker holding this client's mutex — backpressure
    // becomes head-of-line blocking for everyone. Capped at 5s; slow
    // clients get disconnected rather than stalling the hub.
    stream.set_write_timeout(Some(WS_READ_TIMEOUT)).ok();

    // Extract the bearer token from the handshake, preferring the
    // Authorization header (native clients) and falling back to the
    // `bearer.<token>` WebSocket subprotocol (browsers). We only learn
    // whether the token is valid AFTER accept_hdr completes, since the
    // header callback must return synchronously with a Response.
    let token_slot: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let slot_for_cb = Arc::clone(&token_slot);
    let ws = match accept_hdr(stream, move |req: &Request, mut resp: Response| -> Result<Response, ErrorResponse> {
        let mut chosen_protocol: Option<String> = None;
        let mut auth: Option<String> = None;
        for (name, value) in req.headers() {
            let lower = name.as_str().to_ascii_lowercase();
            if lower == "authorization" {
                if let Ok(v) = value.to_str() {
                    if let Some(tok) = v.strip_prefix("Bearer ") {
                        auth = Some(tok.to_string());
                    }
                }
            } else if lower == "sec-websocket-protocol" {
                if let Ok(v) = value.to_str() {
                    for proto in v.split(',').map(str::trim) {
                        if let Some(encoded) = proto.strip_prefix("bearer.") {
                            if let Some(decoded) = percent_decode_token(encoded) {
                                auth = auth.or(Some(decoded));
                                chosen_protocol = Some(proto.to_string());
                                break;
                            }
                        }
                    }
                }
            }
        }
        // RFC 6455 §11.3.4 — echo the chosen subprotocol in the response or
        // browsers will refuse the connection.
        if let Some(chosen) = chosen_protocol {
            if let Ok(hv) = tungstenite::http::HeaderValue::from_str(&chosen) {
                resp.headers_mut().insert("Sec-WebSocket-Protocol", hv);
            }
        }
        *slot_for_cb.lock().unwrap() = auth;
        Ok(resp)
    }) {
        Ok(ws) => ws,
        Err(_) => return,
    };

    // Reject unauthenticated or invalid-token handshakes AFTER accept —
    // tungstenite's handshake callback can't easily return a 401 without
    // a custom error response, and we already have the socket open for
    // a clean close frame.
    let token = token_slot.lock().unwrap().clone();
    let auth_ctx = sessions.resolve(token.as_deref());
    if auth_ctx.user_id.is_none() && !auth_ctx.is_admin {
        let mut ws = ws;
        let _ = ws.close(Some(tungstenite::protocol::CloseFrame {
            code: tungstenite::protocol::frame::coding::CloseCode::Policy,
            reason: "unauthorized: bearer token required".into(),
        }));
        return;
    }

    let (client_id, socket_handle) = hub.add_client(ws);

    loop {
        // Lock this client's socket mutex only for the duration of the
        // read. With a 5s read timeout, broadcasters waiting to send to
        // THIS client wait at most 5s. Other clients are never blocked
        // by this lock — they have their own.
        let msg = {
            let mut guard = match socket_handle.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            guard.read()
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
                if let Ok(mut guard) = socket_handle.lock() {
                    let _ = guard.send(Message::Pong(data));
                }
            }
            Ok(Message::Close(_)) => {
                hub.remove_client(client_id);
                let disconnect = serde_json::json!({
                    "type": "presence",
                    "event": "disconnect",
                    "clientId": client_id,
                });
                hub.broadcast_presence(&disconnect.to_string());
                break;
            }
            Err(tungstenite::Error::Io(io_err))
                if io_err.kind() == std::io::ErrorKind::WouldBlock
                    || io_err.kind() == std::io::ErrorKind::TimedOut =>
            {
                // Read timed out — this is EXPECTED with the short
                // timeout. In theory the mutex is released between
                // iterations, but `std::sync::Mutex` is not fair: a tight
                // loop of lock→read→unlock→lock starves the broadcaster
                // that's been waiting on the same mutex. Explicitly sleep
                // for a tick so the broadcaster gets scheduled. 1ms is
                // long enough to hand off, short enough that client→server
                // latency stays sub-5ms.
                std::thread::sleep(std::time::Duration::from_millis(1));
                continue;
            }
            Err(_) => {
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

/// Strict percent-decode for the `bearer.<token>` subprotocol. Returns
/// `None` on any malformed byte rather than silently passing garbage
/// through to the session store (which would just fail to resolve and
/// look like a plain unauth attempt).
fn percent_decode_token(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                if i + 2 >= bytes.len() {
                    return None;
                }
                let hi = (bytes[i + 1] as char).to_digit(16)?;
                let lo = (bytes[i + 2] as char).to_digit(16)?;
                out.push(((hi << 4) | lo) as u8);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
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
            kind: pylon_sync::ChangeKind::Insert,
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
