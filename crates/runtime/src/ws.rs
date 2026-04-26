use std::collections::{HashMap, HashSet};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use pylon_auth::SessionStore;
use pylon_sync::ChangeEvent;
use tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tungstenite::{accept_hdr_with_config, protocol::WebSocketConfig, Message, WebSocket};

use crate::ip_limit::IpConnCounter;

// ---------------------------------------------------------------------------
// CRDT subscription manager
//
// Per-client subscriptions to (entity, row_id) pairs. Lets the binary CRDT
// broadcast filter to only the clients that asked, instead of fanning out
// every CRDT write to every connected WS client.
//
// Two reverse maps so both hot paths are O(subscribers per row) and
// O(rows per client): the broadcast looks up subscribers by row, the
// disconnect cleanup walks rows by client.
//
// Subscriptions are explicit and ephemeral — a client subscribes when
// useLoroDoc(entity, id) mounts, unsubscribes on unmount or disconnect.
// Server doesn't persist subscriptions across reconnects; the client
// re-sends them.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct SubsState {
    /// (entity, row_id) → set of client_ids subscribed to that row.
    by_row: HashMap<(String, String), HashSet<u64>>,
    /// client_id → set of (entity, row_id) it subscribes to.
    /// Inverted to make disconnect cleanup O(rows per client) instead of
    /// O(total rows in by_row).
    by_client: HashMap<u64, HashSet<(String, String)>>,
}

pub struct CrdtSubscriptions {
    /// Single mutex covers both reverse maps so any pair of operations
    /// (subscribe + unsubscribe across threads, broadcast + disconnect
    /// cleanup) sees a consistent view. Two separate mutexes would let
    /// `subscribe` land in `by_row` while a concurrent `unsubscribe_all`
    /// snapshots `by_client` mid-update, leaving the maps divergent.
    state: Mutex<SubsState>,
}

impl Default for CrdtSubscriptions {
    fn default() -> Self {
        Self {
            state: Mutex::new(SubsState::default()),
        }
    }
}

impl CrdtSubscriptions {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Register a client's interest in a row. Idempotent — re-subscribing
    /// the same client to the same row is a no-op (HashSet semantics).
    pub fn subscribe(&self, client_id: u64, entity: &str, row_id: &str) {
        let key = (entity.to_string(), row_id.to_string());
        let mut state = self.state.lock().unwrap();
        state
            .by_row
            .entry(key.clone())
            .or_default()
            .insert(client_id);
        state.by_client.entry(client_id).or_default().insert(key);
    }

    /// Drop one subscription. Cleans up empty maps so the working set
    /// stays bounded — long-running connections that subscribe and
    /// unsubscribe to many rows over their lifetime don't accumulate
    /// orphan empty entries.
    pub fn unsubscribe(&self, client_id: u64, entity: &str, row_id: &str) {
        let key = (entity.to_string(), row_id.to_string());
        let mut state = self.state.lock().unwrap();
        if let Some(set) = state.by_row.get_mut(&key) {
            set.remove(&client_id);
            if set.is_empty() {
                state.by_row.remove(&key);
            }
        }
        if let Some(set) = state.by_client.get_mut(&client_id) {
            set.remove(&key);
            if set.is_empty() {
                state.by_client.remove(&client_id);
            }
        }
    }

    /// Drop every subscription for a client (called on WS disconnect or
    /// when a broadcast send fails for that client). Atomic over the
    /// whole client's subscription set — broadcast snapshots taken
    /// concurrently see the client either fully present or fully gone.
    pub fn unsubscribe_all(&self, client_id: u64) {
        let mut state = self.state.lock().unwrap();
        let rows: Vec<(String, String)> = state
            .by_client
            .remove(&client_id)
            .map(|set| set.into_iter().collect())
            .unwrap_or_default();
        for key in rows {
            if let Some(set) = state.by_row.get_mut(&key) {
                set.remove(&client_id);
                if set.is_empty() {
                    state.by_row.remove(&key);
                }
            }
        }
    }

    /// Snapshot the subscriber set for a row. Returns an owned `Vec`
    /// rather than a guard so the broadcast hot path doesn't hold the
    /// mutex during the per-client send loop.
    pub fn subscribers(&self, entity: &str, row_id: &str) -> Vec<u64> {
        let key = (entity.to_string(), row_id.to_string());
        let state = self.state.lock().unwrap();
        state
            .by_row
            .get(&key)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Diagnostic: total number of (client, row) pairs.
    pub fn total_subscriptions(&self) -> usize {
        self.state
            .lock()
            .unwrap()
            .by_row
            .values()
            .map(|s| s.len())
            .sum()
    }
}

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
    ///
    /// `msg` is `Arc<str>` rather than `&str` so the caller can serialize
    /// the JSON exactly once and share the same allocation across all
    /// 16 shards. Per-client `Message::Text` still allocates an owned
    /// String (tungstenite 0.24 requires it), but the broadcast no
    /// longer pays N copies of the JSON across shard channels.
    fn broadcast(&self, msg: &Arc<str>) {
        let handles: Vec<(u64, ClientSocket)> = {
            let clients = self.clients.lock().unwrap();
            clients.iter().map(|(id, h)| (*id, Arc::clone(h))).collect()
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
            // Owned String per send is the tungstenite 0.24 contract.
            // The clone here copies the string contents; sharing the
            // raw bytes via Utf8Bytes would be the next-level
            // optimization but requires a tungstenite version bump.
            if guard.send(Message::Text((**msg).to_string())).is_err() {
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

    /// Send a binary frame to a SPECIFIC subset of this shard's clients.
    /// Used by the per-client subscription path — `WsHub::broadcast_binary_to`
    /// computes which ids each shard owns and calls this with just those.
    ///
    /// Same per-client lock pattern as `broadcast` / `broadcast_binary`,
    /// just filtered up front instead of iterating the whole shard.
    ///
    /// Returns the list of client ids whose send failed so the caller
    /// can also clear those ids from the CRDT subscription registry —
    /// without that step a dead client's subscription entries linger
    /// until the reader thread notices the EOF and runs unsubscribe_all,
    /// which can take up to one read-timeout (200ms) longer than the
    /// send-side death detection.
    fn send_binary_to(&self, ids: &[u64], msg: &Arc<[u8]>) -> Vec<u64> {
        let handles: Vec<(u64, ClientSocket)> = {
            let clients = self.clients.lock().unwrap();
            ids.iter()
                .filter_map(|id| clients.get(id).map(|h| (*id, Arc::clone(h))))
                .collect()
        };
        let mut dead: Vec<u64> = Vec::new();
        for (id, handle) in handles {
            let mut guard = match handle.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            if guard.send(Message::Binary(msg.to_vec())).is_err() {
                dead.push(id);
            }
        }
        if !dead.is_empty() {
            let mut clients = self.clients.lock().unwrap();
            for id in &dead {
                clients.remove(id);
            }
        }
        dead
    }

    /// Binary fanout for CRDT updates. Same per-client lock pattern as
    /// `broadcast` above; the only difference is `Message::Binary` and
    /// the payload is `Arc<[u8]>` so a single Loro snapshot allocates
    /// once and the per-client send pays a refcount bump + the
    /// tungstenite-required Vec clone.
    fn broadcast_binary(&self, msg: &Arc<[u8]>) {
        let handles: Vec<(u64, ClientSocket)> = {
            let clients = self.clients.lock().unwrap();
            clients.iter().map(|(id, h)| (*id, Arc::clone(h))).collect()
        };
        let mut dead: Vec<u64> = Vec::new();
        for (id, handle) in handles {
            let mut guard = match handle.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            if guard.send(Message::Binary(msg.to_vec())).is_err() {
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
    ///
    /// Carries `Arc<str>` so a single broadcast event allocates the JSON
    /// once and the 16 shard sends are cheap refcount bumps. Was a 16×
    /// String clone hotspot under high write rates with thousands of
    /// subscribers per shard.
    broadcast_txs: Vec<mpsc::SyncSender<Arc<str>>>,
    /// Matching receivers are held by each worker thread and also exposed
    /// here so the "drop oldest" fallback can drain them on full. Keeping
    /// the receiver handle alongside the sender is only safe because mpsc
    /// lets multiple clones share a queue — here we only consume via the
    /// worker, and the sender-side uses `try_send` + drain retry.
    #[allow(dead_code)]
    queue_depth: usize,
    /// Per-client CRDT subscriptions. Reader threads register `(entity,
    /// row_id)` pairs as the client mounts/unmounts useLoroDoc hooks;
    /// the binary CRDT broadcast path uses `subscribers()` to filter the
    /// fanout. Wrapped in Arc so the notifier (which holds `Arc<WsHub>`)
    /// can read the subscriber set without taking an extra lock layer.
    subscriptions: Arc<CrdtSubscriptions>,
}

impl WsHub {
    pub fn new() -> Arc<Self> {
        let mut shards = Vec::with_capacity(NUM_SHARDS);
        let mut broadcast_txs = Vec::with_capacity(NUM_SHARDS);

        for i in 0..NUM_SHARDS {
            let shard = Arc::new(Shard::new());
            // Bounded queue — if a broadcast worker stalls, `try_send` fails
            // with Full and `broadcast_raw` drops the oldest to make room.
            let (tx, rx) = mpsc::sync_channel::<Arc<str>>(BROADCAST_QUEUE_DEPTH);

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
            subscriptions: CrdtSubscriptions::new(),
        })
    }

    /// Access the per-client CRDT subscription registry. The notifier
    /// looks up subscribers via `subscriptions().subscribers(entity, row)`
    /// and feeds them to `broadcast_binary_to`.
    pub fn subscriptions(&self) -> &Arc<CrdtSubscriptions> {
        &self.subscriptions
    }

    /// Broadcast a change event to ALL connected clients across all shards.
    /// Non-blocking: pushes to each shard's channel and returns immediately.
    ///
    /// Serializes the event JSON exactly once into an `Arc<str>` and
    /// shares it across the 16 shard senders. Each shard's worker
    /// thread receives the same Arc and pays only a refcount bump.
    pub fn broadcast(&self, event: &ChangeEvent) {
        let json = match serde_json::to_string(event) {
            Ok(j) => j,
            Err(_) => return,
        };
        let shared: Arc<str> = Arc::from(json.into_boxed_str());
        self.broadcast_shared(shared);
    }

    /// Broadcast a raw string message to all clients (used for presence updates).
    pub fn broadcast_presence(&self, msg: &str) {
        let shared: Arc<str> = Arc::from(msg.to_string().into_boxed_str());
        self.broadcast_shared(shared);
    }

    /// Broadcast a binary frame to every connected client across all
    /// shards. Used for CRDT updates (see `pylon_router::encode_crdt_frame`
    /// for the wire shape). The bytes are wrapped in an `Arc` so each
    /// shard's per-client fanout shares one allocation; the per-send
    /// `to_vec()` cost is the tungstenite 0.24 contract.
    ///
    /// Synchronous fanout — iterates shards directly rather than going
    /// through the per-shard mpsc workers. CRDT writes happen at most
    /// once per logical mutation so the throughput shape is "occasional
    /// burst" not "every keystroke", and direct fanout avoids growing a
    /// second per-shard channel (Arc<[u8]> can't share the Arc<str>
    /// channel without an enum, which costs more than the bypass).
    pub fn broadcast_binary(&self, bytes: Vec<u8>) {
        let shared: Arc<[u8]> = Arc::from(bytes.into_boxed_slice());
        for shard in &self.shards {
            shard.broadcast_binary(&shared);
        }
    }

    /// Send a binary frame to a specific subset of client IDs only.
    /// Used by the CRDT broadcast path to fan out only to clients
    /// subscribed to the row that just changed (instead of every
    /// connected client). Routes each id to its owning shard via
    /// `id % NUM_SHARDS`.
    ///
    /// `client_ids` typically comes from `CrdtSubscriptions::subscribers`.
    /// An empty list is a no-op — the row had no subscribers, so the
    /// CRDT write is durable on the server but no client sees the
    /// binary frame (they'll learn about the change via the JSON
    /// change-event broadcast which always fires).
    pub fn broadcast_binary_to(&self, client_ids: &[u64], bytes: Vec<u8>) {
        if client_ids.is_empty() {
            return;
        }
        let shared: Arc<[u8]> = Arc::from(bytes.into_boxed_slice());
        // Group ids by shard so each shard's per-client lock is only
        // grabbed once even if many subscribers landed in the same one.
        let mut by_shard: Vec<Vec<u64>> = (0..NUM_SHARDS).map(|_| Vec::new()).collect();
        for id in client_ids {
            by_shard[(*id as usize) % NUM_SHARDS].push(*id);
        }
        for (idx, ids) in by_shard.iter().enumerate() {
            if ids.is_empty() {
                continue;
            }
            for dead_id in self.shards[idx].send_binary_to(ids, &shared) {
                // Drop the dead client's subscription entries too —
                // otherwise they leak until the reader thread's read
                // timeout fires and runs unsubscribe_all on its own,
                // and a future broadcast might re-attempt the dead id.
                self.subscriptions.unsubscribe_all(dead_id);
            }
        }
    }

    /// Send a binary frame to a single client by id. Used by the
    /// subscribe path: when a client subscribes to a row, the server
    /// immediately ships the current snapshot so the new subscriber
    /// has the up-to-date state without waiting for the next write.
    pub fn send_binary_to_one(&self, client_id: u64, bytes: Vec<u8>) {
        let shared: Arc<[u8]> = Arc::from(bytes.into_boxed_slice());
        let shard_idx = (client_id as usize) % NUM_SHARDS;
        for dead_id in self.shards[shard_idx].send_binary_to(&[client_id], &shared) {
            self.subscriptions.unsubscribe_all(dead_id);
        }
    }

    /// Internal: fan out a single shared message to every shard worker.
    ///
    /// Uses `try_send`; on full we log once (per call) and drop the message
    /// for that shard. Previously the channel was unbounded, so a stuck
    /// worker thread would grow memory until OOM. The new bounded queue
    /// means a slow/stuck subscriber at worst loses broadcast events —
    /// correctness for critical data still comes through the change-log
    /// cursor on a reconnect.
    fn broadcast_shared(&self, msg: Arc<str>) {
        for tx in &self.broadcast_txs {
            match tx.try_send(Arc::clone(&msg)) {
                Ok(()) => {}
                Err(mpsc::TrySendError::Full(_)) => {
                    tracing::warn!("[ws] broadcast queue full — dropping event for one shard");
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

/// Snapshot fetcher: given the caller's auth context + `(entity,
/// row_id)`, return the encoded binary CRDT frame for the row's
/// current state, or `None` if either the caller can't read the row
/// (read policy denies) or the row has no snapshot (uninitialized
/// CRDT or non-CRDT entity).
///
/// Auth context is passed in (rather than checked at the WS layer)
/// because the policy engine + DataStore handles live in the runtime
/// crate. Without this check an authenticated client could subscribe
/// to any `(entity, row_id)` and receive every binary CRDT frame
/// even for rows their query policy would reject — a silent read-
/// policy bypass.
///
/// Wrapped in an Arc<dyn Fn> so the runtime can build it once, capturing
/// the LoroStore + PolicyEngine handles, and hand the same closure to
/// every accepted connection.
pub type SnapshotFetcher =
    Arc<dyn Fn(&pylon_auth::AuthContext, &str, &str) -> Option<Vec<u8>> + Send + Sync>;

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
///
/// `snapshot_fetcher` is optional — when present, the reader will ship
/// the current CRDT snapshot to the subscribing client immediately on
/// `crdt-subscribe`, so the new tab sees the latest converged state
/// without waiting for the next write. When absent, subscribe is still
/// recorded but the catch-up frame is skipped.
pub fn start_ws_server(
    hub: Arc<WsHub>,
    sessions: Arc<SessionStore>,
    port: u16,
    snapshot_fetcher: Option<SnapshotFetcher>,
) {
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
        let fetcher = snapshot_fetcher.clone();
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
                handle_ws_connection(hub, sessions, stream, fetcher);
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
    snapshot_fetcher: Option<SnapshotFetcher>,
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
    // Cap WebSocket frame size to bound memory per connection. The
    // tungstenite default (64 MiB) is too generous — a single client
    // can shovel huge frames and starve other connections. The cap
    // applies BIDIRECTIONALLY (server-sent CRDT snapshots are
    // checked against it too), so the default must accommodate the
    // largest legitimate snapshot — 16 MiB covers Loro docs with
    // long histories. Operators tune via PYLON_WS_MAX_FRAME (bytes)
    // when they have unusually large or unusually small docs.
    let max_frame: usize = std::env::var("PYLON_WS_MAX_FRAME")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(16 * 1024 * 1024);
    let ws_config = WebSocketConfig {
        max_message_size: Some(max_frame),
        max_frame_size: Some(max_frame),
        ..Default::default()
    };
    let ws = match accept_hdr_with_config(
        stream,
        move |req: &Request, mut resp: Response| -> Result<Response, ErrorResponse> {
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
        },
        Some(ws_config),
    ) {
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
                // Parse once and dispatch on the type field instead of
                // matching prefix bytes — that approach silently dropped
                // valid JSON with whitespace, key reordering, or any
                // other formatting variation. Non-object / no-`type`
                // messages are ignored.
                let parsed: serde_json::Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let kind = parsed.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match kind {
                    "presence" | "topic" => {
                        // Stamp the authenticated sender server-side,
                        // overriding any client-provided `from`. Without
                        // this, any client could spoof presence/topic
                        // events as another user — every connected
                        // client would see a forged "alice typed…"
                        // message attributed to alice.
                        let mut stamped = parsed.clone();
                        if let Some(obj) = stamped.as_object_mut() {
                            let from = auth_ctx
                                .user_id
                                .clone()
                                .unwrap_or_else(|| "admin".to_string());
                            obj.insert("from".into(), serde_json::Value::String(from));
                        }
                        hub.broadcast_presence(&stamped.to_string());
                    }
                    "crdt-subscribe" | "crdt-unsubscribe" => handle_crdt_control(
                        &hub,
                        client_id,
                        &auth_ctx,
                        kind,
                        &parsed,
                        snapshot_fetcher.as_ref(),
                    ),
                    _ => {}
                }
            }
            Ok(Message::Ping(data)) => {
                // Respond with pong to keep the connection alive.
                if let Ok(mut guard) = socket_handle.lock() {
                    let _ = guard.send(Message::Pong(data));
                }
            }
            Ok(Message::Close(_)) => {
                // Drop every CRDT subscription this client held BEFORE
                // remove_client so the broadcast path can never look up
                // a stale client_id between the two ops.
                hub.subscriptions.unsubscribe_all(client_id);
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
                hub.subscriptions.unsubscribe_all(client_id);
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

/// Apply a parsed `crdt-subscribe` / `crdt-unsubscribe` control
/// message. Both messages have the shape:
///
///   { "type": "crdt-subscribe",   "entity": "<E>", "rowId": "<id>" }
///   { "type": "crdt-unsubscribe", "entity": "<E>", "rowId": "<id>" }
///
/// On subscribe the snapshot fetcher checks read policy for the
/// caller's auth context — if the caller can't read the row we
/// register no subscription and ship nothing back, so a malicious
/// client can't peek at a row their query policy would block by
/// just subscribing to its CRDT stream.
///
/// Malformed messages are silently dropped — there's no client-visible
/// ACK protocol, so a typo in the payload would just look like a
/// row that never receives updates. Logging would invite a noise
/// channel for misbehaving clients.
fn handle_crdt_control(
    hub: &Arc<WsHub>,
    client_id: u64,
    auth_ctx: &pylon_auth::AuthContext,
    kind: &str,
    parsed: &serde_json::Value,
    snapshot_fetcher: Option<&SnapshotFetcher>,
) {
    let entity = match parsed.get("entity").and_then(|v| v.as_str()) {
        Some(e) if !e.is_empty() => e,
        _ => return,
    };
    let row_id = match parsed
        .get("rowId")
        .or_else(|| parsed.get("row_id"))
        .and_then(|v| v.as_str())
    {
        Some(r) if !r.is_empty() => r,
        _ => return,
    };

    match kind {
        "crdt-subscribe" => {
            // Authz check happens INSIDE the fetcher (it has access to
            // the policy engine + DataStore). When a fetcher is wired
            // and returns None, the caller is either denied or the row
            // doesn't exist — in both cases we refuse to register the
            // subscription so a denied caller can't silently hold an
            // open slot waiting for future writes.
            //
            // When no fetcher is wired (test harnesses, future
            // workers backend without DataStore access) we trust the
            // caller and register without the auth gate. Production
            // server.rs always wires one, so this loophole is
            // unreachable in deployed configurations.
            let snapshot = snapshot_fetcher.and_then(|f| f(auth_ctx, entity, row_id));
            let allow_subscribe = snapshot_fetcher.is_none() || snapshot.is_some();
            if allow_subscribe {
                hub.subscriptions.subscribe(client_id, entity, row_id);
                if let Some(bytes) = snapshot {
                    hub.send_binary_to_one(client_id, bytes);
                }
            }
        }
        "crdt-unsubscribe" => {
            hub.subscriptions.unsubscribe(client_id, entity, row_id);
        }
        _ => {}
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
    fn crdt_subscriptions_subscribe_dedups() {
        let subs = CrdtSubscriptions::default();
        subs.subscribe(1, "Channel", "abc");
        subs.subscribe(1, "Channel", "abc");
        assert_eq!(subs.subscribers("Channel", "abc"), vec![1]);
        assert_eq!(subs.total_subscriptions(), 1);
    }

    #[test]
    fn crdt_subscriptions_returns_all_subscribers() {
        let subs = CrdtSubscriptions::default();
        subs.subscribe(1, "Channel", "abc");
        subs.subscribe(2, "Channel", "abc");
        subs.subscribe(3, "Channel", "abc");
        let mut ids = subs.subscribers("Channel", "abc");
        ids.sort();
        assert_eq!(ids, vec![1, 2, 3]);
    }

    #[test]
    fn crdt_subscriptions_unsubscribe_cleans_empty_rows() {
        let subs = CrdtSubscriptions::default();
        subs.subscribe(1, "Channel", "abc");
        subs.unsubscribe(1, "Channel", "abc");
        assert!(subs.subscribers("Channel", "abc").is_empty());
        // total should drop the empty by_row entry, not leave a 0-set
        // around forever.
        assert_eq!(subs.total_subscriptions(), 0);
    }

    #[test]
    fn crdt_subscriptions_unsubscribe_all_drops_every_row() {
        let subs = CrdtSubscriptions::default();
        subs.subscribe(1, "Channel", "a");
        subs.subscribe(1, "Channel", "b");
        subs.subscribe(1, "Message", "m1");
        subs.subscribe(2, "Channel", "a"); // someone else, must survive
        subs.unsubscribe_all(1);
        assert!(subs.subscribers("Channel", "b").is_empty());
        assert!(subs.subscribers("Message", "m1").is_empty());
        // Client 2 is still there.
        assert_eq!(subs.subscribers("Channel", "a"), vec![2]);
    }

    #[test]
    fn crdt_subscriptions_unsubscribe_unknown_client_is_noop() {
        let subs = CrdtSubscriptions::default();
        subs.unsubscribe(99, "Channel", "abc");
        subs.unsubscribe_all(99);
        assert_eq!(subs.total_subscriptions(), 0);
    }

    #[test]
    fn crdt_subscriptions_concurrent_subscribe_and_unsubscribe() {
        // Hammer subscribe + unsubscribe from many threads to verify
        // the single-mutex design keeps by_row and by_client in sync.
        // Previous two-mutex version could leave the maps divergent
        // under interleaving.
        let subs = Arc::new(CrdtSubscriptions::default());
        let mut handles = Vec::new();
        for client_id in 0..16u64 {
            let subs = Arc::clone(&subs);
            handles.push(std::thread::spawn(move || {
                for i in 0..200 {
                    let row = format!("row-{i}");
                    subs.subscribe(client_id, "Channel", &row);
                    subs.unsubscribe(client_id, "Channel", &row);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        // Every subscribe paired with an unsubscribe — registry must be
        // fully drained.
        assert_eq!(subs.total_subscriptions(), 0);
    }

    #[test]
    fn crdt_subscriptions_unsubscribe_all_after_concurrent_subscribes() {
        let subs = Arc::new(CrdtSubscriptions::default());
        let mut handles = Vec::new();
        for client_id in 0..8u64 {
            let subs = Arc::clone(&subs);
            handles.push(std::thread::spawn(move || {
                for i in 0..100 {
                    let row = format!("row-{i}");
                    subs.subscribe(client_id, "Channel", &row);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        // Now wipe each client and confirm no orphan rows remain.
        for client_id in 0..8u64 {
            subs.unsubscribe_all(client_id);
        }
        assert_eq!(subs.total_subscriptions(), 0);
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
