use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use pylon_auth::{api_key::ApiKeyStore, resolve_bearer_token, AuthContext, SessionStore};
use pylon_policy::{PolicyEngine, PolicyResult};

/// Auth resolution context for WS handshake.
///
/// Bundles the four pieces the shared `resolve_bearer_token` needs:
/// session store, API-key store, optional admin token, optional JWT
/// secret + issuer. Without this, the WS upgrade path used to only
/// validate session tokens — admin/API-key/JWT bearers that worked
/// over HTTP silently failed over WS, plus admin promotion / API-key
/// revocation semantics diverged from the HTTP path. Caught in the
/// 2026-05-10 codex pass-3 audit (P3 REGRESSION).
pub struct WsAuth {
    pub sessions: Arc<SessionStore>,
    pub api_keys: Arc<ApiKeyStore>,
    pub admin_token: Option<String>,
    pub jwt_secret: Option<String>,
    pub jwt_issuer: Option<String>,
}
use pylon_sync::ChangeEvent;
use tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tungstenite::protocol::Role;
use tungstenite::{accept_hdr_with_config, protocol::WebSocketConfig, Message, WebSocket};

use crate::ip_limit::IpConnCounter;

/// Marker trait that lets the WS hub hold sockets from multiple
/// origins behind the same handle: native TCP connections from the
/// dedicated `:4322` listener AND HTTP-upgraded streams that bubble
/// up from tiny_http when a client multiplexes WS on the main port.
///
/// `Send + 'static` are required because reader threads own the
/// stream and broadcasts cross threads via the shard channels.
pub trait WsStream: Read + Write + Send + 'static {}
impl<T: Read + Write + Send + 'static> WsStream for T {}

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
///
/// Auth context is captured at registration time (post-handshake auth
/// resolution) and lives for the connection's lifetime. It feeds the
/// per-client tenant filter on every change-event broadcast — the
/// hub evaluates `policy.check_entity_read(entity, &client.auth, &row)`
/// before pushing each event. Without this, every connected client
/// received every change event from every tenant. Caught in the
/// 2026-05-10 codex pass-3 audit (P0).
pub struct WsClient {
    pub socket: Mutex<WebSocket<Box<dyn WsStream>>>,
    pub auth: AuthContext,
}

type ClientSocket = Arc<WsClient>;

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

    fn add(&self, id: u64, ws: WebSocket<Box<dyn WsStream>>, auth: AuthContext) -> ClientSocket {
        let handle = Arc::new(WsClient {
            socket: Mutex::new(ws),
            auth,
        });
        self.clients.lock().unwrap().insert(id, Arc::clone(&handle));
        handle
    }

    fn remove(&self, id: u64) {
        self.clients.lock().unwrap().remove(&id);
    }

    /// Send a string message to ALL clients in this shard, no filtering.
    /// Used for presence/topic relays where every client in the room
    /// genuinely should see the message — those messages don't carry
    /// row data. Change events go through `broadcast_change` instead so
    /// the per-client tenant filter runs.
    ///
    /// Snapshot the client handles under the shard lock, drop the shard
    /// lock, then contend only with per-client mutexes to do the writes.
    /// This is what lets a reader thread hold its client's mutex for a
    /// socket.read() without stalling broadcasts for the whole shard.
    fn broadcast(&self, msg: &Arc<str>) {
        let handles: Vec<(u64, ClientSocket)> = {
            let clients = self.clients.lock().unwrap();
            clients.iter().map(|(id, h)| (*id, Arc::clone(h))).collect()
        };
        let mut dead: Vec<u64> = Vec::new();
        for (id, handle) in handles {
            let mut guard = match handle.socket.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
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

    /// Send a change event to clients in this shard whose stored
    /// `AuthContext` passes the entity's read policy. Skips clients
    /// without permission so client A never sees client B's tenant
    /// data. The pre-serialized `json` is reused across allowed
    /// clients; serialization happened once at the hub level.
    fn broadcast_change(&self, event: &ChangeEvent, json: &Arc<str>, policy: &PolicyEngine) {
        let handles: Vec<(u64, ClientSocket)> = {
            let clients = self.clients.lock().unwrap();
            clients.iter().map(|(id, h)| (*id, Arc::clone(h))).collect()
        };
        let mut dead: Vec<u64> = Vec::new();
        for (id, handle) in handles {
            // Per-client policy check. The event's `data` field carries
            // the row payload (or None for deletes — policy still
            // evaluates against entity-level rules). `is_admin`
            // bypasses everything (admins legitimately see all
            // tenants). All other clients run through the engine.
            if !handle.auth.is_admin {
                let row = event.data.as_ref();
                match policy.check_entity_read(&event.entity, &handle.auth, row) {
                    PolicyResult::Allowed => {}
                    PolicyResult::Denied { .. } => continue,
                }
            }
            let mut guard = match handle.socket.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            if guard.send(Message::Text((**json).to_string())).is_err() {
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
            let mut guard = match handle.socket.lock() {
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

    /// Send a single text frame to one client by id. Reactive query
    /// push path: re-runner calls this with the new JSON result
    /// envelope. No-op if the id has already been swept out (dead
    /// connection) — caller doesn't need to know about delivery.
    fn send_text_to_one(&self, client_id: u64, text: &str) {
        let handle = {
            let clients = self.clients.lock().unwrap();
            clients.get(&client_id).map(Arc::clone)
        };
        let Some(handle) = handle else {
            return;
        };
        let mut guard = match handle.socket.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if guard.send(Message::Text(text.to_string())).is_err() {
            // Drop guard before re-locking clients (avoid lock
            // inversion vs other paths that take clients lock first).
            drop(guard);
            let mut clients = self.clients.lock().unwrap();
            clients.remove(&client_id);
        }
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
            let mut guard = match handle.socket.lock() {
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

    /// Send a text message to every client in this shard whose
    /// authenticated user_id matches. Used by the session-changed
    /// push: when SessionStore mutates a session's tenant, the hub
    /// fans the new state to all of that user's connected tabs so
    /// each engine refreshes without app-level notifySessionChanged
    /// calls. Admin-token connections (no user_id) are skipped.
    fn send_text_to_user(&self, user_id: &str, msg: &Arc<str>) {
        let handles: Vec<(u64, ClientSocket)> = {
            let clients = self.clients.lock().unwrap();
            clients
                .iter()
                .filter(|(_, h)| h.auth.user_id.as_deref() == Some(user_id))
                .map(|(id, h)| (*id, Arc::clone(h)))
                .collect()
        };
        let mut dead: Vec<u64> = Vec::new();
        for (id, handle) in handles {
            let mut guard = match handle.socket.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
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
/// What each broadcast shard worker consumes off its mpsc channel.
///
/// `Change` carries a deserialized event PLUS the pre-serialized JSON.
/// Workers iterate clients in their shard, run the policy engine
/// against each client's stored auth + the event's row data, and only
/// forward the JSON to clients that pass. `Plain` skips the filter and
/// goes to every client — used for presence/topic relay where the
/// payload doesn't carry row data.
pub enum BroadcastJob {
    Change {
        event: Arc<ChangeEvent>,
        json: Arc<str>,
    },
    Plain(Arc<str>),
    /// Per-user text fanout: deliver `msg` to every client in this
    /// shard whose authenticated user_id matches `user_id`. Used by
    /// `notify_session_changed` — has to run on the worker thread
    /// (not the HTTP request thread) because the per-client socket
    /// send can block on a slow/dead WS client and would otherwise
    /// peg the HTTP handler that emitted the notification. Hung
    /// `POST /api/auth/select-org` symptom on 2026-05-18: a single
    /// stuck WS client blocked the request thread, then the next
    /// requests piled up behind it, then the health check failed
    /// 12s later and Fly's LB pulled the machine.
    PerUser {
        user_id: Arc<str>,
        msg: Arc<str>,
    },
}

pub struct WsHub {
    shards: Vec<Arc<Shard>>,
    next_id: Mutex<u64>,
    /// Bounded-capacity senders for each shard's broadcast worker. The
    /// `Change` variant carries the event so the worker can run the
    /// per-client tenant filter; the `Plain` variant is unfiltered.
    broadcast_txs: Vec<mpsc::SyncSender<BroadcastJob>>,
    #[allow(dead_code)]
    queue_depth: usize,
    /// Policy engine for per-client read checks on every change-event
    /// broadcast. Wrapped in Arc so worker threads can clone cheaply.
    /// `None` is a hard error path now — change events without policy
    /// would re-open the cross-tenant leak. Construction sites in
    /// `server.rs` always pass one.
    policy: Arc<PolicyEngine>,
    /// Per-client CRDT subscriptions. Reader threads register `(entity,
    /// row_id)` pairs as the client mounts/unmounts useLoroDoc hooks;
    /// the binary CRDT broadcast path uses `subscribers()` to filter the
    /// fanout. Wrapped in Arc so the notifier (which holds `Arc<WsHub>`)
    /// can read the subscriber set without taking an extra lock layer.
    subscriptions: Arc<CrdtSubscriptions>,
}

impl WsHub {
    pub fn new(policy: Arc<PolicyEngine>) -> Arc<Self> {
        let mut shards = Vec::with_capacity(NUM_SHARDS);
        let mut broadcast_txs = Vec::with_capacity(NUM_SHARDS);

        for i in 0..NUM_SHARDS {
            let shard = Arc::new(Shard::new());
            let (tx, rx) = mpsc::sync_channel::<BroadcastJob>(BROADCAST_QUEUE_DEPTH);

            let shard_clone = Arc::clone(&shard);
            let policy_clone = Arc::clone(&policy);
            thread::Builder::new()
                .name(format!("ws-broadcast-{i}"))
                .spawn(move || {
                    while let Ok(job) = rx.recv() {
                        match job {
                            BroadcastJob::Change { event, json } => {
                                shard_clone.broadcast_change(&event, &json, &policy_clone);
                            }
                            BroadcastJob::Plain(msg) => {
                                shard_clone.broadcast(&msg);
                            }
                            BroadcastJob::PerUser { user_id, msg } => {
                                shard_clone.send_text_to_user(&user_id, &msg);
                            }
                        }
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
            policy,
            subscriptions: CrdtSubscriptions::new(),
        })
    }

    /// Access the per-client CRDT subscription registry. The notifier
    /// looks up subscribers via `subscriptions().subscribers(entity, row)`
    /// and feeds them to `broadcast_binary_to`.
    pub fn subscriptions(&self) -> &Arc<CrdtSubscriptions> {
        &self.subscriptions
    }

    /// Broadcast a change event to clients whose stored auth passes the
    /// entity's read policy. Non-blocking: pushes to each shard's channel
    /// and returns immediately.
    ///
    /// Serializes the event JSON once and ships it via `BroadcastJob::Change`
    /// alongside the deserialized event. Workers run the policy engine
    /// per-client before sending, so a client without read access never
    /// sees the row data. This is the v0.3.72 fix for the cross-tenant
    /// data leak the codex pass-3 audit flagged.
    pub fn broadcast(&self, event: &ChangeEvent) {
        let json = match serde_json::to_string(event) {
            Ok(j) => j,
            Err(_) => return,
        };
        let json_arc: Arc<str> = Arc::from(json.into_boxed_str());
        let event_arc: Arc<ChangeEvent> = Arc::new(event.clone());
        for tx in &self.broadcast_txs {
            let job = BroadcastJob::Change {
                event: Arc::clone(&event_arc),
                json: Arc::clone(&json_arc),
            };
            match tx.try_send(job) {
                Ok(()) => {}
                Err(mpsc::TrySendError::Full(_)) => {
                    tracing::warn!("[ws] broadcast queue full — dropping event for one shard");
                }
                Err(mpsc::TrySendError::Disconnected(_)) => {}
            }
        }
    }

    /// Broadcast a raw string message to ALL clients, no per-client
    /// filter. Used for presence/topic relays where the message
    /// doesn't carry tenant-scoped row data and every connected client
    /// is a legitimate recipient.
    pub fn broadcast_presence(&self, msg: &str) {
        let shared: Arc<str> = Arc::from(msg.to_string().into_boxed_str());
        for tx in &self.broadcast_txs {
            let _ = tx.try_send(BroadcastJob::Plain(Arc::clone(&shared)));
        }
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

    /// Send a text frame (JSON) to a single client by id. Used by the
    /// reactive query re-runner — when a sub's result changes, push
    /// the new payload directly to the subscribed client rather than
    /// broadcasting to every WS connection. Silently drops if the
    /// client has disconnected since the registry indexed it.
    /// Fan a text message to every connected client whose authenticated
    /// user_id matches. Used by the session-changed push so all of a
    /// user's open tabs refresh in lockstep when their session mutates.
    ///
    /// Non-blocking: each shard's send happens on its broadcast worker
    /// thread, not the caller's thread. CRITICAL — the per-client
    /// socket send can block on a slow/dead WS client, so doing this
    /// on an HTTP request thread (where session-changed gets called)
    /// would peg the HTTP handler. 2026-05-18 outage: a single stuck
    /// WS client blocked POST /api/auth/select-org, subsequent
    /// requests stacked behind it, health check failed 12s later,
    /// Fly LB pulled the machine. Now identical fire-and-forget
    /// semantics as `broadcast()` / `broadcast_change()`.
    ///
    /// Backpressure: try_send drops the job if the worker channel is
    /// full (worker thread is behind). That's acceptable for the
    /// session-changed surface — the SDK already pulls /api/auth/me
    /// on focus + periodically, so a dropped push delays update by
    /// at most one pull cycle. Logging suppressed because dropping
    /// per-user pushes is expected under load.
    pub fn send_text_to_user(&self, user_id: &str, text: &str) {
        let msg: Arc<str> = Arc::from(text);
        let user: Arc<str> = Arc::from(user_id);
        for tx in &self.broadcast_txs {
            let _ = tx.try_send(BroadcastJob::PerUser {
                user_id: Arc::clone(&user),
                msg: Arc::clone(&msg),
            });
        }
    }

    pub fn send_text_to(&self, client_id: u64, text: &str) {
        let shard_idx = (client_id as usize) % NUM_SHARDS;
        self.shards[shard_idx].send_text_to_one(client_id, text);
    }

    /// Assign a client to a shard via round-robin and register it.
    /// Returns `(id, socket_handle)` — the caller keeps the handle and uses
    /// it for reads; the shard also keeps an Arc clone for broadcasts.
    /// `auth` is captured at registration time and lives for the
    /// connection's lifetime so per-client filtering can evaluate
    /// against the same identity that authenticated the handshake.
    fn add_client(
        &self,
        ws: WebSocket<Box<dyn WsStream>>,
        auth: AuthContext,
    ) -> (u64, ClientSocket) {
        let mut next_id = self.next_id.lock().unwrap();
        let id = *next_id;
        *next_id += 1;
        let shard_idx = (id as usize) % NUM_SHARDS;
        let handle = self.shards[shard_idx].add(id, ws, auth);
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
    auth: Arc<WsAuth>,
    port: u16,
    snapshot_fetcher: Option<SnapshotFetcher>,
    reactive: Option<Arc<crate::reactive::ReactiveRegistry>>,
) {
    // Dual-stack v6+v4. The Yapless Mac app + any client that
    // resolves `localhost` to `::1` first (the macOS default) would
    // otherwise see "connection refused" on the WS port even though
    // the HTTP server on the next port up is reachable. See
    // crate::bind_dual_stack_tcp for the rationale.
    let listener = match crate::bind_dual_stack_tcp(port) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("[ws] Failed to bind on port {port}: {e}");
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
        let auth = Arc::clone(&auth);
        let fetcher = snapshot_fetcher.clone();
        let reactive_cl = reactive.as_ref().map(Arc::clone);
        let spawn_result = thread::Builder::new()
            .name("ws-client".into())
            .stack_size(64 * 1024)
            .spawn(move || {
                let _conn_slot = guard;
                handle_ws_connection(hub, auth, stream, fetcher, reactive_cl);
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
    auth: Arc<WsAuth>,
    stream: TcpStream,
    snapshot_fetcher: Option<SnapshotFetcher>,
    reactive: Option<Arc<crate::reactive::ReactiveRegistry>>,
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
    // Box the TcpStream as a `WsStream` so the hub can hold native and
    // HTTP-upgraded clients behind the same handle. Tungstenite owns
    // the boxed stream after the handshake; the dyn dispatch overhead
    // is one virtual call per socket op and not measurable next to
    // the actual network I/O.
    let stream: Box<dyn WsStream> = Box::new(stream);
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
    run_authenticated_session(ws, hub, auth, token, snapshot_fetcher, reactive);
}

/// Take an already-handshaken WebSocket, resolve its bearer token,
/// and run the per-client message loop. Shared between the dedicated
/// `:4322` listener and the HTTP-multiplexed entry point on the main
/// port — both arrive here once the WS handshake is done and the
/// only remaining work is auth + the message pump.
///
/// Sends a clean close frame with a Policy code on auth failure so
/// the client can surface a sensible error instead of a generic
/// network drop.
fn run_authenticated_session(
    ws: WebSocket<Box<dyn WsStream>>,
    hub: Arc<WsHub>,
    auth: Arc<WsAuth>,
    token: Option<String>,
    snapshot_fetcher: Option<SnapshotFetcher>,
    reactive: Option<Arc<crate::reactive::ReactiveRegistry>>,
) {
    // Use the shared bearer resolver so WS sees the same identities
    // HTTP does — admin token / API key / JWT / session — not only
    // session tokens like the pre-v0.3.72 path. Caught in the
    // 2026-05-10 codex pass-3 audit (P3 REGRESSION).
    let auth_ctx = match resolve_bearer_token(
        token.as_deref(),
        &auth.sessions,
        &auth.api_keys,
        auth.admin_token.as_deref(),
        auth.jwt_secret.as_deref(),
        auth.jwt_issuer.as_deref(),
    ) {
        Ok(ctx) => ctx,
        Err(reason) => {
            let mut ws = ws;
            let _ = ws.close(Some(tungstenite::protocol::CloseFrame {
                code: tungstenite::protocol::frame::coding::CloseCode::Policy,
                reason: format!("unauthorized: {reason}").into(),
            }));
            return;
        }
    };
    if auth_ctx.user_id.is_none() && !auth_ctx.is_admin {
        let mut ws = ws;
        let _ = ws.close(Some(tungstenite::protocol::CloseFrame {
            code: tungstenite::protocol::frame::coding::CloseCode::Policy,
            reason: "unauthorized: bearer token required".into(),
        }));
        return;
    }

    let (client_id, socket_handle) = hub.add_client(ws, auth_ctx.clone());

    loop {
        // Lock this client's socket mutex only for the duration of the
        // read. With a 5s read timeout, broadcasters waiting to send to
        // THIS client wait at most 5s. Other clients are never blocked
        // by this lock — they have their own.
        let msg = {
            let mut guard = match socket_handle.socket.lock() {
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
                    "reactive-subscribe" | "reactive-unsubscribe" => {
                        if let Some(reg) = reactive.as_ref() {
                            handle_reactive_control(reg, &hub, client_id, &auth_ctx, kind, &parsed);
                        } else {
                            // Reactive not wired (binary built without
                            // the function runtime, or no
                            // functions/ directory). Tell the client
                            // explicitly so the React hook can fall
                            // back to a one-shot fetch instead of
                            // hanging on a sub_id that will never
                            // deliver.
                            let sub_id =
                                parsed.get("sub_id").and_then(|v| v.as_str()).unwrap_or("");
                            let frame = serde_json::json!({
                                "type": "reactive-error",
                                "sub_id": sub_id,
                                "code": "REACTIVE_UNAVAILABLE",
                                "message": "reactive queries require the TS function runtime",
                            })
                            .to_string();
                            hub.send_text_to(client_id, &frame);
                        }
                    }
                    _ => {}
                }
            }
            Ok(Message::Ping(data)) => {
                // Respond with pong to keep the connection alive.
                if let Ok(mut guard) = socket_handle.socket.lock() {
                    let _ = guard.send(Message::Pong(data));
                }
            }
            Ok(Message::Close(_)) => {
                // Drop every CRDT subscription this client held BEFORE
                // remove_client so the broadcast path can never look up
                // a stale client_id between the two ops.
                hub.subscriptions.unsubscribe_all(client_id);
                if let Some(reg) = reactive.as_ref() {
                    reg.disconnect_client(client_id);
                }
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
                if let Some(reg) = reactive.as_ref() {
                    reg.disconnect_client(client_id);
                }
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

/// Apply a parsed `reactive-subscribe` / `reactive-unsubscribe`
/// control message.
///
/// Subscribe shape:
///
///     {
///       "type": "reactive-subscribe",
///       "sub_id": "<client-minted id>",
///       "fn_name": "getMessagesWithAuthors",
///       "args": { ... }
///     }
///
/// The server runs the handler under the connection's auth context,
/// records the dep set, and pushes the initial result back. From then
/// on, any change event matching the dep set triggers a re-run + push.
///
/// Auth carry-over: the `auth_ctx` for re-runs is captured here
/// (the WS connection's resolved identity). Mutations from other
/// users do NOT cause re-runs under those users' auth — re-runs
/// always use the original subscriber's identity, so a sub gated
/// behind `auth.userId == row.ownerId` keeps that gate every time.
///
/// Unsubscribe shape:
///
///     { "type": "reactive-unsubscribe", "sub_id": "..." }
///
/// Errors push a `reactive-error` frame so the client can surface
/// the failure instead of waiting indefinitely for a `reactive-result`.
fn handle_reactive_control(
    reg: &Arc<crate::reactive::ReactiveRegistry>,
    hub: &Arc<WsHub>,
    client_id: u64,
    auth_ctx: &pylon_auth::AuthContext,
    kind: &str,
    parsed: &serde_json::Value,
) {
    let sub_id = match parsed.get("sub_id").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return,
    };
    match kind {
        "reactive-subscribe" => {
            let fn_name = parsed
                .get("fn_name")
                .or_else(|| parsed.get("fnName"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if fn_name.is_empty() {
                let frame = serde_json::json!({
                    "type": "reactive-error",
                    "sub_id": sub_id,
                    "code": "MISSING_FN_NAME",
                    "message": "reactive-subscribe requires fn_name",
                })
                .to_string();
                hub.send_text_to(client_id, &frame);
                return;
            }
            let args = parsed.get("args").cloned().unwrap_or(serde_json::json!({}));
            // Map AuthContext → AuthInfo. Carries the FULL identity
            // (roles included) so RBAC-style policies see the same
            // values on re-run as on the first run.
            let auth_info = pylon_functions::protocol::AuthInfo {
                user_id: auth_ctx.user_id.clone(),
                is_admin: auth_ctx.is_admin,
                tenant_id: auth_ctx.tenant_id.clone(),
                roles: auth_ctx.roles.clone(),
            };
            // Dispatch the initial run to the re-runner thread —
            // the WS reader thread MUST NOT block on fn_ops.call.
            // The re-runner picks it up, runs the handler, captures
            // deps, and pushes the result via reactive-result.
            let outcome = reg.register_pending(sub_id.clone(), fn_name, args, auth_info, client_id);
            if outcome == crate::reactive::RegisterOutcome::OverLimit {
                let frame = serde_json::json!({
                    "type": "reactive-error",
                    "sub_id": sub_id,
                    "code": "REACTIVE_LIMIT",
                    "message": "per-client reactive subscription limit reached",
                })
                .to_string();
                hub.send_text_to(client_id, &frame);
            }
        }
        "reactive-unsubscribe" => {
            reg.unsubscribe(client_id, &sub_id);
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

// ---------------------------------------------------------------------------
// HTTP-multiplexed WebSocket entry point
//
// The dedicated `:4322` listener stays — single-port deploys (Vercel
// rewrites, naive reverse proxies that don't pass through `Upgrade`)
// can still rely on a separate WS port. But for proxies that DO carry
// the upgrade through (Cloudflare, Caddy, modern Vercel rewrites),
// running WS on the same `:4321` port the HTTP server uses means the
// `wss://<host>/api/sync/ws` URL just works without per-deployment
// config.
//
// Flow:
//   1. server.rs detects `Upgrade: websocket` on a /api/sync/ws GET
//      and hands the tiny_http::Request here along with the bearer
//      token already extracted from headers / subprotocol.
//   2. We compute Sec-WebSocket-Accept ourselves (sha1 + base64 of
//      the client's Sec-WebSocket-Key + the magic GUID), build a
//      101 response, and call request.upgrade("websocket", response)
//      which writes the response and hijacks the underlying socket.
//   3. The hijacked stream wraps in WebSocket::from_raw_socket
//      (bypassing tungstenite's accept handshake — we already did it).
//   4. From there the per-client lifecycle is identical to the
//      :4322 path via `run_authenticated_session`.
// ---------------------------------------------------------------------------

const WS_GUID: &[u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// Compute the `Sec-WebSocket-Accept` header value per RFC 6455 §4.2.2.
/// Returns a base64-encoded sha1 of `<client-key><GUID>`.
fn ws_accept_value(client_key: &str) -> String {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use sha1::{Digest, Sha1};
    let mut hasher = Sha1::new();
    hasher.update(client_key.as_bytes());
    hasher.update(WS_GUID);
    let digest = hasher.finalize();
    STANDARD.encode(digest)
}

/// Result of inspecting an incoming HTTP request for a WS upgrade.
pub struct WsUpgradeRequest {
    pub sec_key: String,
    pub bearer_token: Option<String>,
    /// First subprotocol from `Sec-WebSocket-Protocol` we want to
    /// echo back. Browsers refuse the connection if a subprotocol
    /// they offered isn't echoed.
    pub chosen_protocol: Option<String>,
}

/// Pull the headers we need to perform a WS upgrade. Returns `None`
/// when the request isn't a WebSocket upgrade attempt (no
/// `Sec-WebSocket-Key`).
///
/// Bearer-token resolution priority (matches the HTTP request loop):
///   1. `Authorization: Bearer <token>` header (native clients)
///   2. `Sec-WebSocket-Protocol: bearer.<percent-encoded-token>` (browsers
///      can't set Authorization on WebSocket, so the SDK encodes the token
///      as a subprotocol name)
///   3. `Cookie: <session_cookie_name>=<token>` (browsers using cookie
///      auth — required for the Pylon Cloud dashboard's WS multiplex,
///      otherwise the WS upgrade lands as anonymous and the auth gate
///      closes the socket immediately, producing a tight reconnect loop)
///
/// Pass `cookie_name` so the framework's cookie-config-driven name
/// (`<app>_session` by default) works without a magic constant here.
pub fn inspect_ws_upgrade(
    headers: &[tiny_http::Header],
    cookie_name: &str,
) -> Option<WsUpgradeRequest> {
    let mut sec_key: Option<String> = None;
    let mut upgrade_ok = false;
    let mut bearer_token: Option<String> = None;
    let mut chosen_protocol: Option<String> = None;
    let mut cookie_header: Option<String> = None;
    for h in headers {
        let name = h.field.as_str().as_str().to_ascii_lowercase();
        let value = h.value.as_str();
        if name == "sec-websocket-key" {
            sec_key = Some(value.to_string());
        } else if name == "upgrade" && value.eq_ignore_ascii_case("websocket") {
            upgrade_ok = true;
        } else if name == "authorization" {
            if let Some(tok) = value.strip_prefix("Bearer ") {
                bearer_token = Some(tok.to_string());
            }
        } else if name == "sec-websocket-protocol" {
            for proto in value.split(',').map(str::trim) {
                if let Some(encoded) = proto.strip_prefix("bearer.") {
                    if let Some(decoded) = percent_decode_token(encoded) {
                        if bearer_token.is_none() {
                            bearer_token = Some(decoded);
                        }
                        chosen_protocol = Some(proto.to_string());
                        break;
                    }
                }
            }
        } else if name == "cookie" {
            cookie_header = Some(value.to_string());
        }
    }
    // Cookie fallback runs ONLY if no bearer was found via the
    // header / subprotocol path — bearer wins so explicit auth can
    // override the ambient cookie when both are present.
    if bearer_token.is_none() {
        if let Some(cookies) = cookie_header.as_deref() {
            bearer_token = pylon_auth::extract_session_cookie(cookies, cookie_name);
        }
    }
    if !upgrade_ok {
        return None;
    }
    sec_key.map(|sec_key| WsUpgradeRequest {
        sec_key,
        bearer_token,
        chosen_protocol,
    })
}

/// Hijack a tiny_http request as a WebSocket. Writes the 101
/// response, takes ownership of the raw stream, wraps in tungstenite
/// without re-handshaking, and runs the standard per-client loop.
/// Spawn this on its own thread — the loop blocks on `socket.read()`.
pub fn handle_http_upgrade(
    request: tiny_http::Request,
    upgrade: WsUpgradeRequest,
    hub: Arc<WsHub>,
    auth: Arc<WsAuth>,
    snapshot_fetcher: Option<SnapshotFetcher>,
    reactive: Option<Arc<crate::reactive::ReactiveRegistry>>,
) {
    let accept = ws_accept_value(&upgrade.sec_key);
    let mut response = tiny_http::Response::empty(101)
        .with_header(tiny_http::Header::from_bytes(&b"Upgrade"[..], &b"websocket"[..]).unwrap())
        .with_header(tiny_http::Header::from_bytes(&b"Connection"[..], &b"Upgrade"[..]).unwrap())
        .with_header(
            tiny_http::Header::from_bytes(&b"Sec-WebSocket-Accept"[..], accept.as_bytes()).unwrap(),
        );
    if let Some(proto) = &upgrade.chosen_protocol {
        if let Ok(h) =
            tiny_http::Header::from_bytes(&b"Sec-WebSocket-Protocol"[..], proto.as_bytes())
        {
            response = response.with_header(h);
        }
    }
    let stream = request.upgrade("websocket", response);
    // tiny_http hands back `Box<dyn ReadWrite + Send>`; that satisfies
    // our `WsStream` blanket impl. Tungstenite never sees a raw socket
    // here — we already wrote the 101 above.
    let stream: Box<dyn WsStream> = Box::new(WsStreamAdapter(stream));
    let max_frame: usize = std::env::var("PYLON_WS_MAX_FRAME")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(16 * 1024 * 1024);
    let ws_config = WebSocketConfig {
        max_message_size: Some(max_frame),
        max_frame_size: Some(max_frame),
        ..Default::default()
    };
    let ws = WebSocket::from_raw_socket(stream, Role::Server, Some(ws_config));
    run_authenticated_session(
        ws,
        hub,
        auth,
        upgrade.bearer_token,
        snapshot_fetcher,
        reactive,
    );
}

/// Adapter so a `Box<dyn tiny_http::ReadWrite + Send>` satisfies our
/// `WsStream` bound. tiny_http's ReadWrite is just `Read + Write`
/// without the `+ Send` part exposed in the trait, so we wrap with
/// a thin newtype to forward the I/O.
struct WsStreamAdapter(Box<dyn tiny_http::ReadWrite + Send>);

impl Read for WsStreamAdapter {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for WsStreamAdapter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
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
        let hub = WsHub::new(Arc::new(PolicyEngine::from_manifest(
            &pylon_kernel::AppManifest::default(),
        )));
        assert_eq!(hub.client_count(), 0);
    }

    #[test]
    fn broadcast_to_empty_hub_doesnt_panic() {
        let hub = WsHub::new(Arc::new(PolicyEngine::from_manifest(
            &pylon_kernel::AppManifest::default(),
        )));
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

    /// Per-client tenant filter regression. Without it, the WS shard's
    /// `broadcast_change` would fan a tenant-A row to a tenant-B
    /// subscriber. The `Shard::broadcast_change` path runs the policy
    /// engine against each client's stored auth — this test verifies
    /// the gate by constructing a manifest with a tenant-scoped read
    /// rule, two clients with different tenants, and asserting only
    /// the matching one would be allowed.
    #[test]
    fn change_event_filters_per_client_tenant() {
        use pylon_kernel::{ManifestEntity, ManifestField, ManifestPolicy};

        let manifest = pylon_kernel::AppManifest {
            manifest_version: pylon_kernel::MANIFEST_VERSION,
            name: "t".into(),
            version: "0".into(),
            entities: vec![ManifestEntity {
                name: "Doc".into(),
                fields: vec![
                    ManifestField {
                        name: "id".into(),
                        field_type: "string".into(),
                        optional: false,
                        unique: false,
                        crdt: None,
                    },
                    ManifestField {
                        name: "tenantId".into(),
                        field_type: "string".into(),
                        optional: false,
                        unique: false,
                        crdt: None,
                    },
                ],
                ..Default::default()
            }],
            policies: vec![ManifestPolicy {
                name: "doc_tenant_read".into(),
                entity: Some("Doc".into()),
                allow_read: Some("auth.tenantId == data.tenantId".into()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let policy = Arc::new(PolicyEngine::from_manifest(&manifest));

        // Tenant-A client should see the row; tenant-B client should not.
        let auth_a = AuthContext::user("alice".into()).with_tenant("tA".into());
        let auth_b = AuthContext::user("bob".into()).with_tenant("tB".into());
        let row = serde_json::json!({"id": "r1", "tenantId": "tA"});

        match policy.check_entity_read("Doc", &auth_a, Some(&row)) {
            PolicyResult::Allowed => {}
            PolicyResult::Denied { reason, .. } => {
                panic!("tenant-A should be allowed: {reason}")
            }
        }
        match policy.check_entity_read("Doc", &auth_b, Some(&row)) {
            PolicyResult::Allowed => panic!("tenant-B must be denied"),
            PolicyResult::Denied { .. } => {}
        }
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
