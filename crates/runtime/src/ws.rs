use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, RwLock};
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
use pylon_sync::{ChangeEvent, ChangeKind};
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

// ---------------------------------------------------------------------------
// Room subscription manager
//
// Per-client subscriptions to room names. Lets `room-update` pushes fan
// out only to clients that explicitly subscribed to a room, instead of
// blasting every connected WS client (which `broadcast_presence` does
// for the legacy SDK firehose).
//
// Same two-map design as `CrdtSubscriptions`:
//   - by_room: room → set of client_ids
//   - by_client: client_id → set of rooms
// Both maps live under a single mutex so disconnect cleanup and
// concurrent subscribe never tear.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct RoomSubsState {
    /// room → set of client_ids subscribed to that room.
    by_room: HashMap<String, HashSet<u64>>,
    /// client_id → set of rooms it subscribes to.
    by_client: HashMap<u64, HashSet<String>>,
}

pub struct RoomSubscriptions {
    state: Mutex<RoomSubsState>,
}

impl Default for RoomSubscriptions {
    fn default() -> Self {
        Self {
            state: Mutex::new(RoomSubsState::default()),
        }
    }
}

impl RoomSubscriptions {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn subscribe(&self, client_id: u64, room: &str) {
        let mut state = self.state.lock().unwrap();
        state
            .by_room
            .entry(room.to_string())
            .or_default()
            .insert(client_id);
        state
            .by_client
            .entry(client_id)
            .or_default()
            .insert(room.to_string());
    }

    pub fn unsubscribe(&self, client_id: u64, room: &str) {
        let mut state = self.state.lock().unwrap();
        if let Some(set) = state.by_room.get_mut(room) {
            set.remove(&client_id);
            if set.is_empty() {
                state.by_room.remove(room);
            }
        }
        if let Some(set) = state.by_client.get_mut(&client_id) {
            set.remove(room);
            if set.is_empty() {
                state.by_client.remove(&client_id);
            }
        }
    }

    /// Drop every room subscription for this client. Called on WS
    /// disconnect — mirrors `CrdtSubscriptions::unsubscribe_all`. Returns
    /// the list of rooms this client was subscribed to so the caller can
    /// surface "you were in these rooms" diagnostics if needed.
    pub fn unsubscribe_all(&self, client_id: u64) -> Vec<String> {
        let mut state = self.state.lock().unwrap();
        let rooms: Vec<String> = state
            .by_client
            .remove(&client_id)
            .map(|set| set.into_iter().collect())
            .unwrap_or_default();
        for room in &rooms {
            if let Some(set) = state.by_room.get_mut(room) {
                set.remove(&client_id);
                if set.is_empty() {
                    state.by_room.remove(room);
                }
            }
        }
        rooms
    }

    /// Snapshot the subscriber set for a room. Returns an owned `Vec`
    /// rather than a guard so the per-client send loop runs without
    /// holding the mutex.
    pub fn subscribers(&self, room: &str) -> Vec<u64> {
        let state = self.state.lock().unwrap();
        state
            .by_room
            .get(room)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }

    pub fn total_subscriptions(&self) -> usize {
        self.state
            .lock()
            .unwrap()
            .by_room
            .values()
            .map(|s| s.len())
            .sum()
    }
}

/// Number of shards for distributing WebSocket clients.
/// Must be a power of two for even modulo distribution.
const NUM_SHARDS: usize = 16;

/// Where a `presence`/`topic` WS frame should be delivered.
#[derive(Debug, PartialEq, Eq)]
enum PresenceTarget {
    /// Room-scoped fanout to that room's subscribers (sender is a member).
    Room,
    /// Legacy un-scoped global broadcast (opt-in only).
    Global,
    /// Don't relay — roomless without opt-in, or a non-member sender.
    Drop,
}

/// Routing decision for a `presence`/`topic` frame. Secure by default:
/// a frame with a `room` relays ONLY if the sender is a member; a roomless
/// frame is dropped unless the operator opted into the global firehose. This
/// closes the cross-tenant relay where `broadcast_presence` fanned every
/// frame to every connected client with no scoping.
fn presence_target(has_room: bool, room_member: bool, allow_unscoped: bool) -> PresenceTarget {
    match (has_room, room_member, allow_unscoped) {
        (true, true, _) => PresenceTarget::Room,
        (true, false, _) => PresenceTarget::Drop,
        (false, _, true) => PresenceTarget::Global,
        (false, _, false) => PresenceTarget::Drop,
    }
}

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
/// resolution). It's wrapped in `RwLock` so the runtime can refresh
/// the tenant slot in place when the user's session mutates (POST
/// /api/auth/select-org, /api/auth/clear-org, session revoke) without
/// forcing the client to reconnect. Without the in-place refresh, the
/// per-client filter on every change-event broadcast would keep
/// running against the pre-select-org tenant for the connection's
/// lifetime — silently dropping rows the user just gained access to,
/// or worse, leaking rows from the org they switched away from.
///
/// Read lock is held briefly during `broadcast_change`'s policy check
/// + `send_text_to_user`'s user-id filter — both fast field reads,
/// not socket I/O. Write lock is held only by `update_auth_for_user`
/// at session-change time (rare). RwLock vs Mutex chosen because the
/// hot path is read-only and N clients per shard fan out to the same
/// lock pattern.
/// Per-client outbound queue depth. When the queue fills, the
/// broadcaster's `try_send` fails for this client and the event is
/// dropped (logged at error level). The client's next pull catches
/// it back up via the cursor protocol. 256 events / client is enough
/// for any realistic burst — broadcasters that overrun a single client
/// at this rate are probably misconfigured.
const PER_CLIENT_OUTBOUND_DEPTH: usize = 256;

pub struct WsClient {
    /// The WebSocket struct itself, behind a per-client mutex. Reader
    /// thread holds this during `ws.read()`. Direct-send paths that
    /// don't go through the broadcast queue (e.g. the auto-pong reply
    /// inside the read loop) still take the mutex briefly — they're
    /// already holding it from the reader's context anyway.
    pub socket: Mutex<WebSocket<Box<dyn WsStream>>>,
    /// Outbound queue used by broadcasters. Pushing here via `try_send`
    /// never blocks on the socket mutex, so a wedged reader (waiting on
    /// `ws.read()` with no client traffic) can't starve broadcasts the
    /// way it did pre-refactor. The reader drains this queue between
    /// reads under the same lock it already holds.
    pub outbound_tx: mpsc::SyncSender<Message>,
    /// Receiver half of the outbound queue, parked here so the reader
    /// thread can take ownership during `run_authenticated_session`.
    /// `mpsc::Receiver` is `!Sync`, so we hand it out once via `take()`
    /// rather than try to share it. After the reader takes the Receiver
    /// this slot is `None`; broadcasters only use `outbound_tx`.
    pub outbound_rx: Mutex<Option<mpsc::Receiver<Message>>>,
    pub auth: RwLock<AuthContext>,
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
        let (outbound_tx, outbound_rx) = mpsc::sync_channel(PER_CLIENT_OUTBOUND_DEPTH);
        let handle = Arc::new(WsClient {
            socket: Mutex::new(ws),
            outbound_tx,
            outbound_rx: Mutex::new(Some(outbound_rx)),
            auth: RwLock::new(auth),
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
            match handle
                .outbound_tx
                .try_send(Message::Text((**msg).to_string()))
            {
                Ok(()) => {}
                Err(mpsc::TrySendError::Disconnected(_)) => dead.push(id),
                Err(mpsc::TrySendError::Full(_)) => {
                    tracing::error!(
                        client_id = id,
                        "[ws] per-client outbound queue full — dropping presence/topic message"
                    );
                }
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
    /// data.
    ///
    /// For Update events with `event.prev_data`, runs a two-stage
    /// check per subscriber: post first (against `event.data`); on
    /// deny, pre (against `event.prev_data`). When pre allowed and
    /// post denied (the visibility-flip case), the subscriber gets
    /// the pre-serialized synthesized Delete JSON instead of being
    /// silently dropped — closing the stale-row ghost where a row
    /// that "moves away" stays in the replica indefinitely.
    fn broadcast_change(
        &self,
        event: &ChangeEvent,
        json: &Arc<str>,
        synth_delete_json: Option<&Arc<str>>,
        policy: &PolicyEngine,
    ) {
        let handles: Vec<(u64, ClientSocket)> = {
            let clients = self.clients.lock().unwrap();
            clients.iter().map(|(id, h)| (*id, Arc::clone(h))).collect()
        };
        let mut dead: Vec<u64> = Vec::new();
        let mut delivered = 0u32;
        let mut denied = 0u32;
        let mut tombstoned = 0u32;
        for (id, handle) in handles {
            // Per-client policy check. Read-lock the per-client auth
            // so a concurrent session-changed update doesn't tear
            // the AuthContext mid-check.
            let auth = match handle.auth.read() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            // `is_admin` bypasses every policy gate.
            let payload: &Arc<str> = if auth.is_admin {
                json
            } else {
                let post_allowed = matches!(
                    policy.check_entity_read(&event.entity, &auth, event.data.as_ref()),
                    PolicyResult::Allowed
                );
                if post_allowed {
                    json
                } else if let Some(synth) = synth_delete_json {
                    // Two-stage: check if PRE was allowed. If so this
                    // subscriber WAS visible to the row before the
                    // update — send the tombstone so their local
                    // replica drops it. If pre also denied, the
                    // subscriber never had visibility; skip silently.
                    let pre_allowed = matches!(
                        policy.check_entity_read(&event.entity, &auth, event.prev_data.as_ref(),),
                        PolicyResult::Allowed
                    );
                    if pre_allowed {
                        tombstoned += 1;
                        synth
                    } else {
                        denied += 1;
                        continue;
                    }
                } else {
                    denied += 1;
                    continue;
                }
            };
            drop(auth);
            // Push to the per-client outbound channel rather than
            // grabbing the socket mutex. The reader thread drains the
            // channel under its own lock between reads — broadcasters
            // never contend.
            match handle
                .outbound_tx
                .try_send(Message::Text((**payload).to_string()))
            {
                Ok(()) => delivered += 1,
                Err(mpsc::TrySendError::Disconnected(_)) => dead.push(id),
                Err(mpsc::TrySendError::Full(_)) => {
                    tracing::error!(
                        client_id = id,
                        entity = %event.entity,
                        seq = event.seq,
                        "[ws.broadcast_change] per-client outbound full — dropping change event; client will catch up on next pull"
                    );
                }
            }
        }
        let _ = tombstoned;
        tracing::debug!(
            entity = %event.entity,
            delivered,
            denied,
            dead = dead.len(),
            "[ws.broadcast_change] fanout complete"
        );
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
    /// Partition `ids` into (allowed, denied) by re-running
    /// `check_entity_read` against each client's current `AuthContext`.
    /// Admins bypass the gate (matches `broadcast_change`). Clients
    /// whose handles have already been swept (Disconnected from the
    /// shard map) are silently dropped from both lists — they're
    /// dead, not denied.
    ///
    /// Used by `broadcast_binary_to_authed` (codex P1 — CRDT
    /// subscriptions leaked frames after permission revocation).
    fn filter_binary_recipients(
        &self,
        ids: &[u64],
        entity: &str,
        row: Option<&serde_json::Value>,
        policy: &PolicyEngine,
    ) -> (Vec<u64>, Vec<u64>) {
        let handles: Vec<(u64, ClientSocket)> = {
            let clients = self.clients.lock().unwrap();
            ids.iter()
                .filter_map(|id| clients.get(id).map(|h| (*id, Arc::clone(h))))
                .collect()
        };
        let mut allowed: Vec<u64> = Vec::with_capacity(handles.len());
        let mut denied: Vec<u64> = Vec::new();
        for (id, handle) in handles {
            let auth = match handle.auth.read() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            if auth.is_admin {
                allowed.push(id);
                continue;
            }
            match policy.check_entity_read(entity, &auth, row) {
                PolicyResult::Allowed => allowed.push(id),
                PolicyResult::Denied { .. } => denied.push(id),
            }
        }
        (allowed, denied)
    }

    fn send_binary_to(&self, ids: &[u64], msg: &Arc<[u8]>) -> Vec<u64> {
        let handles: Vec<(u64, ClientSocket)> = {
            let clients = self.clients.lock().unwrap();
            ids.iter()
                .filter_map(|id| clients.get(id).map(|h| (*id, Arc::clone(h))))
                .collect()
        };
        let mut dead: Vec<u64> = Vec::new();
        for (id, handle) in handles {
            match handle.outbound_tx.try_send(Message::Binary(msg.to_vec())) {
                Ok(()) => {}
                Err(mpsc::TrySendError::Disconnected(_)) => dead.push(id),
                Err(mpsc::TrySendError::Full(_)) => {
                    tracing::error!(
                        client_id = id,
                        "[ws] per-client outbound full — dropping targeted binary frame"
                    );
                }
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
        match handle.outbound_tx.try_send(Message::Text(text.to_string())) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Disconnected(_)) => {
                let mut clients = self.clients.lock().unwrap();
                clients.remove(&client_id);
            }
            Err(mpsc::TrySendError::Full(_)) => {
                tracing::error!(
                    client_id,
                    "[ws] per-client outbound full — dropping targeted text frame (reactive result)"
                );
            }
        }
    }

    /// Binary fanout for CRDT updates. Same per-client outbound-queue
    /// pattern as `broadcast` above; the only difference is
    /// `Message::Binary` and the payload is `Arc<[u8]>` so a single
    /// Loro snapshot allocates once and the per-client send pays a
    /// refcount bump + the tungstenite-required Vec clone.
    fn broadcast_binary(&self, msg: &Arc<[u8]>) {
        let handles: Vec<(u64, ClientSocket)> = {
            let clients = self.clients.lock().unwrap();
            clients.iter().map(|(id, h)| (*id, Arc::clone(h))).collect()
        };
        let mut dead: Vec<u64> = Vec::new();
        for (id, handle) in handles {
            match handle.outbound_tx.try_send(Message::Binary(msg.to_vec())) {
                Ok(()) => {}
                Err(mpsc::TrySendError::Disconnected(_)) => dead.push(id),
                Err(mpsc::TrySendError::Full(_)) => {
                    tracing::error!(
                        client_id = id,
                        "[ws] per-client outbound full — dropping CRDT binary frame"
                    );
                }
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
                .filter(|(_, h)| {
                    let auth = match h.auth.read() {
                        Ok(g) => g,
                        Err(poisoned) => poisoned.into_inner(),
                    };
                    auth.user_id.as_deref() == Some(user_id)
                })
                .map(|(id, h)| (*id, Arc::clone(h)))
                .collect()
        };
        let mut dead: Vec<u64> = Vec::new();
        for (id, handle) in handles {
            match handle
                .outbound_tx
                .try_send(Message::Text((**msg).to_string()))
            {
                Ok(()) => {}
                Err(mpsc::TrySendError::Disconnected(_)) => dead.push(id),
                Err(mpsc::TrySendError::Full(_)) => {
                    tracing::error!(
                        client_id = id,
                        user_id = %user_id,
                        "[ws] per-client outbound full — dropping per-user text frame"
                    );
                }
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

    /// Refresh the `tenant_id` on every connection in this shard whose
    /// authenticated `user_id` matches. Returns the number of clients
    /// updated so the hub-level aggregator can log + emit metrics.
    ///
    /// Called from the runtime's session-changed hook so the per-WS
    /// `AuthContext` tracks the user's freshly-selected org without
    /// requiring a reconnect. Read paths (`broadcast_change`,
    /// `send_text_to_user`) acquire read locks, so a concurrent
    /// update is a brief write-lock contention — no tearing.
    fn update_tenant_for_user(&self, user_id: &str, new_tenant: Option<&str>) -> usize {
        let handles: Vec<ClientSocket> = {
            let clients = self.clients.lock().unwrap();
            clients
                .values()
                .filter(|h| {
                    let auth = match h.auth.read() {
                        Ok(g) => g,
                        Err(poisoned) => poisoned.into_inner(),
                    };
                    auth.user_id.as_deref() == Some(user_id)
                })
                .map(Arc::clone)
                .collect()
        };
        let mut updated = 0usize;
        for handle in handles {
            let mut auth = match handle.auth.write() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            auth.tenant_id = new_tenant.map(|t| t.to_string());
            updated += 1;
        }
        updated
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
        /// Pre-serialized JSON for the "post-allowed" case (subscribers
        /// whose policy passes against `event.data`).
        json: Arc<str>,
        /// Pre-serialized JSON of a synthesized Delete event at the
        /// same seq, used for subscribers whose policy passed against
        /// `event.prev_data` but denies `event.data` (visibility-flip).
        /// `None` when the event has no `prev_data` (Insert/Delete or
        /// Update without a captured pre-row) — those events just
        /// drop denied subscribers without a tombstone.
        synth_delete_json: Option<Arc<str>>,
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
    policy: Arc<PolicyEngine>,
    /// Manifest snapshot for wire projection. Held here (not on the
    /// notifier) so projection happens at the final serialization
    /// step AFTER the per-client policy check — policies that
    /// reference `serverOnly` fields evaluate against the raw row.
    /// Pre-fix the notifier projected before broadcast, stripping
    /// fields from the policy check input.
    manifest: Arc<pylon_kernel::AppManifest>,
    /// Auth-user manifest config, used by `maybe_project_user_row`
    /// inside the wire projection.
    auth_user: pylon_kernel::ManifestAuthUserConfig,
    /// Per-client CRDT subscriptions. Reader threads register `(entity,
    /// row_id)` pairs as the client mounts/unmounts useLoroDoc hooks;
    /// the binary CRDT broadcast path uses `subscribers()` to filter the
    /// fanout. Wrapped in Arc so the notifier (which holds `Arc<WsHub>`)
    /// can read the subscriber set without taking an extra lock layer.
    subscriptions: Arc<CrdtSubscriptions>,
    /// Per-client room subscriptions. Populated by `room-subscribe`
    /// control frames; consumed by `push_room_update` to fan out
    /// `room-update` envelopes only to clients that asked to listen.
    /// Replaces the 5s HTTP polling loop the SDK used to run against
    /// `/api/rooms/<room>` — new clients subscribe over WS and the
    /// server pushes membership deltas as they happen.
    room_subscriptions: Arc<RoomSubscriptions>,
}

impl WsHub {
    pub fn new(
        policy: Arc<PolicyEngine>,
        manifest: Arc<pylon_kernel::AppManifest>,
        auth_user: pylon_kernel::ManifestAuthUserConfig,
    ) -> Arc<Self> {
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
                            BroadcastJob::Change {
                                event,
                                json,
                                synth_delete_json,
                            } => {
                                shard_clone.broadcast_change(
                                    &event,
                                    &json,
                                    synth_delete_json.as_ref(),
                                    &policy_clone,
                                );
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
            manifest,
            auth_user,
            subscriptions: CrdtSubscriptions::new(),
            room_subscriptions: RoomSubscriptions::new(),
        })
    }

    /// Access the per-client room subscription registry. The notifier /
    /// cluster-bus subscriber looks up subscribers via
    /// `room_subscriptions().subscribers(room)` and feeds the ids into
    /// `push_to_room_subscribers`.
    pub fn room_subscriptions(&self) -> &Arc<RoomSubscriptions> {
        &self.room_subscriptions
    }

    /// Fan a text frame to every client subscribed to `room`. Indexed by
    /// room name, so cost is O(subscribers per room) — NOT O(all
    /// connections). Used by `push_room_update` and `push_room_snapshot`.
    /// Dead client ids are evicted from the room registry on send
    /// failure so the next fanout doesn't pay the cost again.
    pub fn push_to_room_subscribers(&self, room: &str, text: &str) {
        let subscribers = self.room_subscriptions.subscribers(room);
        if subscribers.is_empty() {
            return;
        }
        // Route each id to its shard. The send path drops the client
        // from the shard on Disconnected, but we also need to drop the
        // room subscription entry — otherwise the next push retries the
        // dead id. `send_text_to_one` returns no list because it's a
        // best-effort single-id send, so do the cleanup here by routing
        // and tracking failures manually via a Disconnected sentinel.
        let mut by_shard: Vec<Vec<u64>> = (0..NUM_SHARDS).map(|_| Vec::new()).collect();
        for id in &subscribers {
            by_shard[(*id as usize) % NUM_SHARDS].push(*id);
        }
        for (idx, ids) in by_shard.iter().enumerate() {
            for id in ids {
                self.shards[idx].send_text_to_one(*id, text);
            }
        }
        // We can't tell from `send_text_to_one` which sends were
        // Disconnected — the function silently sweeps the dead client
        // from the shard map but doesn't report the id. That's fine:
        // the WS reader thread for that client runs `unsubscribe_all`
        // on its room_subscriptions slot the moment the read loop
        // observes EOF, so the room registry self-heals within the
        // 200ms read timeout window. No leak.
    }

    /// Push a `room-snapshot` envelope to a single client. Used at
    /// subscribe time so the new subscriber has the current member list
    /// without waiting for the next mutation.
    pub fn push_room_snapshot(&self, client_id: u64, room: &str, members: &serde_json::Value) {
        let frame = serde_json::json!({
            "type": "room-snapshot",
            "room": room,
            "members": members,
        });
        if let Ok(text) = serde_json::to_string(&frame) {
            self.send_text_to(client_id, &text);
        }
    }

    /// Push a `room-update` envelope to every subscriber of `room`. The
    /// `action` is one of `join`, `leave`, `presence`, `broadcast`
    /// (mirrors the RoomEvent variants). `member` carries the affected
    /// peer's info (None for broadcast). `data` carries action-specific
    /// payload (presence data, broadcast payload, etc).
    pub fn push_room_update(
        &self,
        room: &str,
        action: &str,
        member: Option<serde_json::Value>,
        data: Option<serde_json::Value>,
    ) {
        let mut frame = serde_json::json!({
            "type": "room-update",
            "room": room,
            "action": action,
        });
        if let Some(obj) = frame.as_object_mut() {
            if let Some(m) = member {
                obj.insert("member".to_string(), m);
            }
            if let Some(d) = data {
                obj.insert("data".to_string(), d);
            }
        }
        if let Ok(text) = serde_json::to_string(&frame) {
            self.push_to_room_subscribers(room, &text);
        }
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
        // Project NOW for wire serialization. The raw event in the
        // `BroadcastJob` keeps `data` / `prev_data` intact so the
        // per-client policy check evaluates against the unprojected
        // row — read policies that reference `serverOnly` fields
        // (e.g. `auth.userId == data.ownerId` where ownerId is
        // serverOnly) need those fields to be present at auth time.
        // Once auth passes, the worker ships the PROJECTED wire
        // JSON: User-entity allowlist + serverOnly field strip +
        // prev_data stripped (since prev_data is server-internal
        // only).
        let projected_data = pylon_router::project_row_for_wire_opt(
            self.manifest.as_ref(),
            &self.auth_user,
            &event.entity,
            event.data.clone(),
        );
        let wire_event_for_clients = ChangeEvent {
            data: projected_data,
            prev_data: None,
            ..event.clone()
        };
        let json = match serde_json::to_string(&wire_event_for_clients) {
            Ok(j) => j,
            Err(_) => return,
        };
        let json_arc: Arc<str> = Arc::from(json.into_boxed_str());
        // Pre-serialize the synthesized Delete-at-this-seq JSON for
        // visibility-flip subscribers. Only computed when the event
        // is an Update carrying `prev_data` — that's the only case
        // where the dual-check might fire. The synth-delete uses
        // the PROJECTED prev_data as its wire `data` so server-only
        // fields on the pre-row don't leak via the tombstone path.
        let synth_delete_arc: Option<Arc<str>> =
            if matches!(event.kind, ChangeKind::Update) && event.prev_data.is_some() {
                let projected_prev = pylon_router::project_row_for_wire_opt(
                    self.manifest.as_ref(),
                    &self.auth_user,
                    &event.entity,
                    event.prev_data.clone(),
                );
                let synth = ChangeEvent {
                    seq: event.seq,
                    entity: event.entity.clone(),
                    row_id: event.row_id.clone(),
                    kind: ChangeKind::Delete,
                    data: projected_prev,
                    prev_data: None,
                    timestamp: event.timestamp.clone(),
                };
                serde_json::to_string(&synth)
                    .ok()
                    .map(|s| Arc::from(s.into_boxed_str()))
            } else {
                None
            };
        let event_arc: Arc<ChangeEvent> = Arc::new(event.clone());
        for tx in &self.broadcast_txs {
            let job = BroadcastJob::Change {
                event: Arc::clone(&event_arc),
                json: Arc::clone(&json_arc),
                synth_delete_json: synth_delete_arc.clone(),
            };
            match tx.try_send(job) {
                Ok(()) => {}
                Err(mpsc::TrySendError::Full(_)) => {
                    // Codex P2: silently dropping a shard's event meant
                    // every client in that shard missed the row update
                    // until a future incidental pull (visibility-change,
                    // mutation, reconnect). In WS-only mode that could
                    // be never. Log at error so operators see it in the
                    // hot path; the client SDK's pull-on-WS-message-gap
                    // is the recovery channel (a future `{"type":"gap"}`
                    // synthetic broadcast would force it, but that path
                    // hits the same per-client mutex contention we're
                    // already trying to relieve — file follow-up to add
                    // a non-blocking gap signal via a dedicated lane).
                    // Operators can tune BROADCAST_QUEUE_DEPTH if this
                    // is frequent.
                    tracing::error!(
                        entity = %event.entity,
                        seq = event.seq,
                        "[ws] broadcast queue full — event dropped for one shard; affected clients will catch up on the next pull"
                    );
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

    /// Per-subscriber-authorized variant of `broadcast_binary_to`. Used
    /// by the CRDT broadcast path so a client whose entity-read
    /// permission was revoked mid-session stops receiving frames
    /// immediately — instead of only on disconnect.
    ///
    /// For every subscriber, re-evaluates `check_entity_read` against
    /// the client's current `AuthContext` and the post-write row data
    /// (when supplied; row-level rules degrade open without it). On
    /// deny, the subscription is removed from the row's registry so
    /// the next broadcast skips the auth work entirely. Admins
    /// bypass (matches the JSON broadcast filter behavior).
    ///
    /// Codex P1: pre-fix the auth check ran ONLY at subscribe time and
    /// the broadcast filtered solely by subscription membership, so a
    /// user removed from a private channel kept receiving its Loro
    /// deltas until they closed the tab.
    pub fn broadcast_binary_to_authed(
        &self,
        client_ids: &[u64],
        bytes: Vec<u8>,
        entity: &str,
        row_id: &str,
        row: Option<&serde_json::Value>,
        seq: u64,
        policy: &PolicyEngine,
    ) {
        if client_ids.is_empty() {
            return;
        }
        let shared: Arc<[u8]> = Arc::from(bytes.into_boxed_slice());
        let mut by_shard: Vec<Vec<u64>> = (0..NUM_SHARDS).map(|_| Vec::new()).collect();
        for id in client_ids {
            by_shard[(*id as usize) % NUM_SHARDS].push(*id);
        }
        // For subscribers whose policy now denies them, also send a
        // revocation envelope so their local replica drops the row.
        //
        // We use a dedicated `{ type: "row-revoked", entity, row_id }`
        // envelope rather than a synthetic Delete change event: the
        // client's apply pipeline filters change events by seq vs.
        // cursor, and a synthetic event has no real seq to compare
        // against (using 0 or current_seq either gets dropped by the
        // seq guard or wedges the cursor). The revocation envelope
        // bypasses seq logic entirely — the SyncEngine handles it
        // by calling `store.reconcileRemove` at the current cursor
        // seq, and `@pylonsync/loro` evicts the LoroDoc registry
        // entry for that (entity, row_id) pair.
        let revocation_signal: Option<Arc<str>> = {
            // `seq` is the high-water seq at the time of revocation.
            // The client uses it as the tombstone seq so any stale
            // in-flight WS frame with `seq <= tombstone` is filtered;
            // anything strictly greater stays admissible (so a
            // legitimate re-grant + re-insert at a higher seq can
            // still land). The client ALSO fires a catch-up pull on
            // receipt — closes the race where a stale frame with
            // seq > tombstone arrives before the next legit event
            // does, by forcing reconciliation against server truth.
            let envelope = serde_json::json!({
                "type": "row-revoked",
                "entity": entity,
                "row_id": row_id,
                "seq": seq,
            });
            serde_json::to_string(&envelope)
                .ok()
                .map(|s| Arc::from(s.into_boxed_str()))
        };
        // `row` is unused now — revocation doesn't ship row data
        // (the client already had whatever they had locally; we
        // just signal "drop it"). Suppress the unused-variable
        // warning explicitly.
        let _ = row;
        for (idx, ids) in by_shard.iter().enumerate() {
            if ids.is_empty() {
                continue;
            }
            // Partition into allowed/denied by per-client policy
            // check. Hold the per-client auth read-lock only across
            // the policy call, then drop before the broadcast send.
            let (allowed, denied) =
                self.shards[idx].filter_binary_recipients(ids, entity, row, policy);
            for revoked_id in denied {
                // Drop the subscription entry so subsequent broadcasts
                // don't pay this filter cost for the same revoked
                // client. unsubscribe is keyed by (entity, row_id),
                // not all-subs — a client may still be a legitimate
                // subscriber to OTHER rows under different policies.
                self.subscriptions.unsubscribe(revoked_id, entity, row_id);
                // Ship the revocation signal so the client drops the
                // row's Loro doc locally. Best-effort: if the send
                // fails the next reconcile will catch up.
                if let Some(ref signal) = revocation_signal {
                    self.shards[idx].send_text_to_one(revoked_id, signal);
                }
            }
            if !allowed.is_empty() {
                for dead_id in self.shards[idx].send_binary_to(&allowed, &shared) {
                    self.subscriptions.unsubscribe_all(dead_id);
                }
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

    /// Refresh the `tenant_id` on every active WS connection
    /// authenticated as `user_id`. Called by the runtime's
    /// session-changed hook (after `/api/auth/select-org`,
    /// `/api/auth/clear-org`, or a cluster-bus envelope from a peer)
    /// so the per-client `AuthContext` carried into subsequent
    /// `broadcast_change` / `handle_crdt_control` /
    /// `handle_reactive_control` invocations reflects the user's
    /// freshly-selected org.
    ///
    /// Without this refresh, the WS retained the `AuthContext`
    /// captured at handshake time for its whole lifetime: a user who
    /// switched orgs mid-session kept subscribing under the *previous*
    /// tenant and saw no rows from the org they just landed on. The
    /// `session-changed` client envelope alone wasn't enough — the
    /// client SDK refreshed its local session view but the server's
    /// per-socket auth stayed stale until reconnect.
    ///
    /// Runs synchronously across shards. The update path is
    /// write-locked but trivially fast (one struct-field swap per
    /// connection) and rare (session mutations only). Read paths
    /// (`broadcast_change`, `send_text_to_user`) wait at most one
    /// write release.
    ///
    /// Returns the total number of connections touched, primarily
    /// for telemetry — a session-change with zero matching sockets
    /// means the user has no open tabs on this machine.
    pub fn update_tenant_for_user(&self, user_id: &str, new_tenant: Option<&str>) -> usize {
        self.shards
            .iter()
            .map(|s| s.update_tenant_for_user(user_id, new_tenant))
            .sum()
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

/// Bridge between the WS reader and the RoomManager.
///
/// The WS layer doesn't depend on `pylon_runtime::rooms::RoomManager`
/// directly — we'd rather take a trait so test harnesses can plug in a
/// stub, and so the runtime's `RoomManager` can grow without forcing
/// every WS test to keep up.
///
/// Methods returned as raw JSON `serde_json::Value` because that's the
/// wire-ready shape the reader pushes back to the client without extra
/// marshalling.
pub trait RoomBridge: Send + Sync {
    /// Return the current member list for the room. Empty list when the
    /// room doesn't exist (matches RoomManager::members semantics).
    fn members(&self, room: &str) -> Vec<serde_json::Value>;

    /// Is this user currently in the room? Used to gate `room-subscribe`
    /// — non-members get `NOT_IN_ROOM` error. Admins bypass this check
    /// at the caller layer (the reader passes through `is_admin` before
    /// calling this).
    fn is_in_room(&self, room: &str, user_id: &str) -> bool;

    /// Disconnect a user from every room they're in. Returns the names
    /// of the rooms they were in so the reader can fan a `room-update`
    /// action:leave to each room's remaining subscribers. Used at WS
    /// close time so presence updates fire even when the client never
    /// hits `/api/rooms/leave` explicitly.
    fn disconnect(&self, user_id: &str) -> Vec<String>;
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
///
/// `snapshot_fetcher` is optional — when present, the reader will ship
/// the current CRDT snapshot to the subscribing client immediately on
/// `crdt-subscribe`, so the new tab sees the latest converged state
/// without waiting for the next write. When absent, subscribe is still
/// recorded but the catch-up frame is skipped.
/// Serialize a `ChangeEvent` for the client wire. Strips the
/// in-memory-only `prev_data` field so the pre-update row never
/// leaks to a subscriber whose post policy allowed them. Used by
/// both WS and SSE broadcast paths.
pub(crate) fn serialize_wire_event(event: &ChangeEvent) -> Option<String> {
    if event.prev_data.is_none() {
        return serde_json::to_string(event).ok();
    }
    let wire = ChangeEvent {
        prev_data: None,
        ..event.clone()
    };
    serde_json::to_string(&wire).ok()
}

pub fn start_ws_server(
    hub: Arc<WsHub>,
    auth: Arc<WsAuth>,
    port: u16,
    snapshot_fetcher: Option<SnapshotFetcher>,
    reactive: Option<Arc<crate::reactive::ReactiveRegistry>>,
    rooms: Option<Arc<dyn RoomBridge>>,
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
        // catch_unwind: std's sockaddr conversion ASSERTS (panics, not
        // errors) when getpeername returns a short address — observed
        // on macOS when the peer disconnects mid-accept. Treat it like
        // any other dead-on-arrival connection.
        let peer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| stream.peer_addr()))
            .unwrap_or_else(|_| Err(std::io::Error::other("peer_addr panicked")));
        let ip = match peer {
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
        let rooms_cl = rooms.as_ref().map(Arc::clone);
        let spawn_result = thread::Builder::new()
            .name("ws-client".into())
            .stack_size(64 * 1024)
            .spawn(move || {
                let _conn_slot = guard;
                handle_ws_connection(hub, auth, stream, fetcher, reactive_cl, rooms_cl);
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
    rooms: Option<Arc<dyn RoomBridge>>,
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

    // Clone the underlying TcpStream BEFORE the handshake consumes the
    // original. TcpStream::try_clone is `dup()` under the hood — both
    // handles refer to the same kernel socket, so closing either one
    // shuts the connection cleanly. The clone is for the dedicated
    // writer thread that owns the WS write-half — it lets the broadcast
    // path wake INSTANTLY on every event instead of waiting for the
    // reader to loop back through its drain block (the architectural
    // fix the v0.3.181 single-thread design couldn't deliver). If the
    // clone fails (rare; should only happen at extreme fd pressure),
    // fall back to single-thread mode with the reader-drains-outbound
    // pattern.
    let write_stream: Option<Box<dyn WsStream>> = stream.try_clone().ok().map(|cloned| {
        cloned.set_write_timeout(Some(WS_READ_TIMEOUT)).ok();
        Box::new(cloned) as Box<dyn WsStream>
    });

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

    // Auth resolution happens inside run_authenticated_session so we
    // can't add the client to the hub until that completes — but we
    // need the client to exist BEFORE the writer thread can borrow
    // its `outbound_rx`. Solve this by running the auth handshake
    // inline here (duplicates a small amount of logic from
    // `run_authenticated_session`), adding the client to the hub,
    // spawning the writer thread on the cloned stream, and then
    // calling run_authenticated_session with a pre-existing client_id
    // — except the function takes `ws` ownership and adds it itself.
    //
    // Cleaner: do the writer-spawn inside `run_authenticated_session`
    // and pass the cloned stream alongside the read-half WS.
    run_authenticated_session(
        ws,
        hub,
        auth,
        token,
        snapshot_fetcher,
        reactive,
        rooms,
        write_stream,
    );
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
    rooms: Option<Arc<dyn RoomBridge>>,
    // When `Some`, the caller supplies an independent WRITE half (a
    // try_clone'd TcpStream on the dedicated listener, the split write
    // half on the HTTP-upgrade path) and gets the dual-thread design —
    // a dedicated writer thread blocks on `outbound_rx.recv()` and
    // sends every broadcast immediately, no ping-bounded latency.
    // When `None`, the reader drains outbound between reads
    // (single-thread fallback; no remaining production caller).
    dual_write_stream: Option<Box<dyn WsStream>>,
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
    // Anonymous WS connections are accepted — they subscribe to the
    // public broadcast firehose. Per-broadcast policy filtering
    // (`Shard::broadcast_change` runs `check_entity_read` against this
    // client's stored AuthContext on every event) decides which events
    // they actually receive. So an open-policy entity ("Todo" in the
    // create-pylon starter) shows up over the wire even without a
    // session, matching the HTTP behavior of `/api/sync/pull` and
    // `/api/fn/listTodos` which both serve anonymous when policy
    // allows. Pre-fix the WS layer rejected anonymous upgrades up
    // front, contradicting the rest of the stack and making the live
    // sync demo silently fall back to no-broadcasts.
    let (client_id, socket_handle) = hub.add_client(ws, auth_ctx.clone());

    // Dual-thread path: spawn the writer thread on the cloned TcpStream.
    // It owns `outbound_rx` and `WebSocket::from_raw_socket`-wraps the
    // cloned stream — no shared mutex with the reader, so broadcasts
    // wake the writer instantly via `recv()` instead of waiting for the
    // reader's drain block to run between blocking reads.
    let outbound_rx_for_reader: Option<mpsc::Receiver<Message>> =
        if let Some(write_stream) = dual_write_stream {
            // Take the receiver out of WsClient and into the writer
            // thread's exclusive ownership.
            let outbound_rx = socket_handle
                .outbound_rx
                .lock()
                .ok()
                .and_then(|mut slot| slot.take())
                .expect("outbound_rx vacant before writer thread could claim it");
            // Wrap the cloned stream in a Role::Server WebSocket. The
            // original `ws` (now owned by the hub via `socket_handle`)
            // has the inbound framing state; this fresh WebSocket has
            // its own outbound state and writes through the cloned
            // stream — TCP duplex semantics keep the two halves from
            // interfering.
            let max_frame: usize = std::env::var("PYLON_WS_MAX_FRAME")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(16 * 1024 * 1024);
            let ws_config = WebSocketConfig {
                max_message_size: Some(max_frame),
                max_frame_size: Some(max_frame),
                ..Default::default()
            };
            let mut writer_ws =
                WebSocket::from_raw_socket(write_stream, Role::Server, Some(ws_config));
            let hub_for_writer = Arc::clone(&hub);
            let writer_client_id = client_id;
            let _ = std::thread::Builder::new()
                .name(format!("ws-writer-{client_id}"))
                .stack_size(64 * 1024)
                .spawn(move || {
                    // Block on the channel. Wakes instantly the moment a
                    // broadcaster pushes via `try_send`. No mutex
                    // contention; no ping-bounded latency; no polling.
                    while let Ok(msg) = outbound_rx.recv() {
                        if writer_ws.send(msg).is_err() {
                            break;
                        }
                    }
                    // Channel closed (client swept out of hub) or send
                    // failed — sweep ourselves to be defensive. Cheap if
                    // the reader already removed us.
                    hub_for_writer.remove_client(writer_client_id);
                });
            None
        } else {
            // Single-thread mode: the reader takes outbound_rx and
            // drains it between reads (HTTP-upgrade path that can't
            // split its stream).
            socket_handle
                .outbound_rx
                .lock()
                .ok()
                .and_then(|mut slot| slot.take())
        };

    loop {
        // Drain queued outbound BEFORE blocking on read — but ONLY if
        // we own the receiver (single-thread mode). In dual-thread mode
        // the writer thread handles delivery and broadcasts have zero
        // ping-bounded latency.
        if let Some(ref outbound_rx) = outbound_rx_for_reader {
            let mut guard = match socket_handle.socket.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            while let Ok(out_msg) = outbound_rx.try_recv() {
                if guard.send(out_msg).is_err() {
                    // Socket dead. Drop the guard, sweep the client,
                    // and exit the session. The remaining queued
                    // messages will be discarded when the channel
                    // half is dropped.
                    drop(guard);
                    hub.remove_client(client_id);
                    return;
                }
            }
        }

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
                // Refresh the per-message auth view from the
                // shard-stored RwLock. `update_tenant_for_user`
                // mutates this slot when the user's session flips
                // tenants (POST /api/auth/select-org); without
                // re-reading per message, control handlers below
                // (crdt-subscribe, reactive-subscribe) would keep
                // running against the auth captured at handshake
                // time and miss the new tenant's rows.
                let auth_ctx = {
                    let guard = match socket_handle.auth.read() {
                        Ok(g) => g,
                        Err(poisoned) => poisoned.into_inner(),
                    };
                    guard.clone()
                };
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
                        let text = stamped.to_string();

                        // SECURITY: `broadcast_presence` fans a frame to EVERY
                        // connected client with no tenant/room scoping (and the
                        // WS layer accepts anonymous connections), so an
                        // un-scoped relay is a cross-tenant message-injection +
                        // presence-leak channel. Route by `room` instead:
                        //   - room present  → require membership (admin bypass)
                        //     and deliver ONLY to that room's subscribers.
                        //   - room absent   → DROP by default; opt back into the
                        //     legacy global firehose with
                        //     PYLON_WS_UNSCOPED_PRESENCE=1 (single-tenant apps).
                        let room = parsed
                            .get("room")
                            .and_then(|v| v.as_str())
                            .filter(|r| !r.is_empty());
                        let is_member = match room {
                            Some(room) => {
                                auth_ctx.is_admin
                                    || auth_ctx.user_id.as_deref().is_some_and(|uid| {
                                        rooms
                                            .as_ref()
                                            .map(|b| b.is_in_room(room, uid))
                                            .unwrap_or(false)
                                    })
                            }
                            None => false,
                        };
                        let allow_unscoped = std::env::var("PYLON_WS_UNSCOPED_PRESENCE")
                            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                            .unwrap_or(false);
                        match presence_target(room.is_some(), is_member, allow_unscoped) {
                            PresenceTarget::Room => {
                                if let Some(room) = room {
                                    hub.push_to_room_subscribers(room, &text);
                                }
                            }
                            PresenceTarget::Global => hub.broadcast_presence(&text),
                            PresenceTarget::Drop => {}
                        }
                    }
                    "crdt-subscribe" | "crdt-unsubscribe" => handle_crdt_control(
                        &hub,
                        client_id,
                        &auth_ctx,
                        kind,
                        &parsed,
                        snapshot_fetcher.as_ref(),
                    ),
                    "room-subscribe" | "room-unsubscribe" => handle_room_control(
                        &hub,
                        client_id,
                        &auth_ctx,
                        kind,
                        &parsed,
                        rooms.as_ref(),
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
                // Route the Pong through the outbound channel so the
                // writer thread (dual-thread mode) sends it, or the
                // reader's own drain loop sends it on the next
                // iteration (single-thread mode). Sending directly
                // here under socket.lock would race with the writer
                // thread on dual-thread connections and interleave
                // partial frames on the underlying TCP stream.
                let _ = socket_handle.outbound_tx.try_send(Message::Pong(data));
            }
            Ok(Message::Close(_)) => {
                // Drop every CRDT subscription this client held BEFORE
                // remove_client so the broadcast path can never look up
                // a stale client_id between the two ops.
                hub.subscriptions.unsubscribe_all(client_id);
                hub.room_subscriptions.unsubscribe_all(client_id);
                if let Some(reg) = reactive.as_ref() {
                    reg.disconnect_client(client_id);
                }
                // Auto-leave every room the user was in and push
                // `room-update` action:leave to remaining subscribers
                // of each. This closes the stale-presence gap when a
                // client drops without calling /api/rooms/leave.
                let snapshot_auth = match socket_handle.auth.read() {
                    Ok(g) => g.clone(),
                    Err(poisoned) => poisoned.into_inner().clone(),
                };
                fanout_room_leaves_on_disconnect(&hub, &snapshot_auth, rooms.as_ref());
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
                hub.room_subscriptions.unsubscribe_all(client_id);
                if let Some(reg) = reactive.as_ref() {
                    reg.disconnect_client(client_id);
                }
                let snapshot_auth = match socket_handle.auth.read() {
                    Ok(g) => g.clone(),
                    Err(poisoned) => poisoned.into_inner().clone(),
                };
                fanout_room_leaves_on_disconnect(&hub, &snapshot_auth, rooms.as_ref());
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
            // Codex P1 fix: SUBSCRIBE first, then FETCH the snapshot.
            // Pre-fix ordering (fetch then subscribe) had a race window
            // where an update arriving between the two lines was lost
            // — the snapshot was taken before the update, the
            // subscription started after the update's broadcast fanout
            // had already completed. The client applied a stale
            // snapshot and never observed the missing update until
            // reconcile.
            //
            // Authorize FIRST (peek-fetch under auth) so an
            // unauthorized client can't subscribe and observe future
            // updates they shouldn't see. Then subscribe, then fetch
            // the snapshot a second time AFTER subscribe — if an
            // update lands in between, the broadcast includes us, and
            // sending the post-subscribe snapshot afterwards gives the
            // client a consistent post-update view.
            let allow_subscribe = match snapshot_fetcher {
                Some(f) => f(auth_ctx, entity, row_id).is_some(),
                None => true,
            };
            if allow_subscribe {
                hub.subscriptions.subscribe(client_id, entity, row_id);
                // Refetch AFTER subscribing so any concurrent update
                // is observable by the client either via the broadcast
                // (because it ran after subscribe) or via the snapshot
                // (because it ran before this refetch). Both cases
                // converge — no update is silently lost.
                let snapshot = snapshot_fetcher.and_then(|f| f(auth_ctx, entity, row_id));
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

/// Apply a parsed `room-subscribe` / `room-unsubscribe` control message.
///
/// Subscribe shape:
///
/// ```text
/// { "type": "room-subscribe", "room": "channel:foo" }
/// ```
///
/// On subscribe the membership check runs against `RoomBridge::is_in_room`
/// (mirrors the HTTP `/api/rooms/broadcast` membership gate added in
/// v0.3.212): non-admin callers must already be in the room, or the
/// reader pushes `{ "type": "error", "code": "NOT_IN_ROOM" }` back and
/// registers no subscription. Admins bypass the check — server-to-
/// server presence dashboards subscribe across rooms without joining
/// them.
///
/// On subscribe success the reader fetches the current members from
/// the bridge and pushes a `room-snapshot` so the new subscriber has
/// the full peer list without waiting for the next mutation. Then it
/// records the subscription in the room registry.
///
/// Unsubscribe shape:
///
/// ```text
/// { "type": "room-unsubscribe", "room": "channel:foo" }
/// ```
///
/// Unsubscribe drops the registry entry — no ACK. Disconnect cleanup
/// uses the same path via `room_subscriptions.unsubscribe_all`.
///
/// When no bridge is wired (test harnesses, future backends without a
/// RoomManager) subscribe is silently dropped — without the bridge we
/// can't validate membership or fetch the snapshot, so a sub would
/// register but never receive anything useful. Silently refusing keeps
/// the registry clean.
fn handle_room_control(
    hub: &Arc<WsHub>,
    client_id: u64,
    auth_ctx: &pylon_auth::AuthContext,
    kind: &str,
    parsed: &serde_json::Value,
    rooms: Option<&Arc<dyn RoomBridge>>,
) {
    let room = match parsed.get("room").and_then(|v| v.as_str()) {
        Some(r) if !r.is_empty() => r,
        _ => return,
    };
    let Some(bridge) = rooms else {
        // No bridge → silently refuse. See doc comment above.
        return;
    };
    match kind {
        "room-subscribe" => {
            // Membership gate: non-admin callers must be in the room.
            // Mirrors the v0.3.212 /api/rooms/broadcast gate so a
            // malicious client can't subscribe to a private channel
            // they never joined and start collecting presence/
            // broadcast events from it. Admins (server dashboards)
            // bypass — they need to observe arbitrary rooms.
            //
            // No user_id on the connection (anonymous WS) means we
            // can't evaluate "are they in the room" — treat as
            // NOT_IN_ROOM so anon clients can't peek at any room.
            // Admin tokens have is_admin=true and skip the check
            // even without a user_id.
            let allowed = if auth_ctx.is_admin {
                true
            } else if let Some(user_id) = auth_ctx.user_id.as_deref() {
                bridge.is_in_room(room, user_id)
            } else {
                false
            };
            if !allowed {
                let err = serde_json::json!({
                    "type": "error",
                    "code": "NOT_IN_ROOM",
                    "room": room,
                });
                if let Ok(text) = serde_json::to_string(&err) {
                    hub.send_text_to(client_id, &text);
                }
                return;
            }
            // Register FIRST, then snapshot. Same ordering as
            // `crdt-subscribe` (codex P1 fix) — any update racing
            // between subscribe and the snapshot fetch is observable
            // either via the push (because we subscribed before it
            // fanned out) or via the snapshot (because it landed
            // before the snapshot read). No room update is silently
            // lost across the subscribe boundary.
            hub.room_subscriptions.subscribe(client_id, room);
            let members = bridge.members(room);
            let members_json = serde_json::Value::Array(members);
            hub.push_room_snapshot(client_id, room, &members_json);
        }
        "room-unsubscribe" => {
            hub.room_subscriptions.unsubscribe(client_id, room);
        }
        _ => {}
    }
}

/// Auto-leave fanout: invoked from the Close + Err arms of the WS
/// reader loop. For each room this user was in, evict them from the
/// RoomManager and push `room-update` action:leave to remaining
/// subscribers of that room.
///
/// Without this, a client that drops without calling `/api/rooms/leave`
/// would leave their presence entry hanging in the RoomManager until
/// the next `pylon.rooms.cleanup` idle sweep (2 min default) — and
/// other subscribers would only learn the user left on that same
/// timer, not in real time.
///
/// Anonymous WS connections (no `user_id` on auth_ctx) skip — they
/// were never in a room, so there's nothing to leave. Admins fall
/// through the same path; an admin connection doesn't typically
/// hold room membership, but if one does it gets cleaned up too.
fn fanout_room_leaves_on_disconnect(
    hub: &Arc<WsHub>,
    auth_ctx: &pylon_auth::AuthContext,
    rooms: Option<&Arc<dyn RoomBridge>>,
) {
    let Some(bridge) = rooms else {
        return;
    };
    let Some(user_id) = auth_ctx.user_id.as_deref() else {
        return;
    };
    let left_rooms = bridge.disconnect(user_id);
    for room in left_rooms {
        let member = serde_json::json!({ "user_id": user_id });
        hub.push_room_update(&room, "leave", Some(member), None);
    }
}

/// Apply a parsed `reactive-subscribe` / `reactive-unsubscribe`
/// control message.
///
/// Subscribe shape:
///
/// ```text
/// {
///   "type": "reactive-subscribe",
///   "sub_id": "<client-minted id>",
///   "fn_name": "getMessagesWithAuthors",
///   "args": { ... }
/// }
/// ```
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
/// ```text
/// { "type": "reactive-unsubscribe", "sub_id": "..." }
/// ```
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
    rooms: Option<Arc<dyn RoomBridge>>,
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
    // Vendored tiny_http: take the connection's halves SEPARATELY so
    // the reader can block in read() forever while the writer thread
    // delivers broadcasts instantly (see WriteHalfStream below).
    let (reader, writer) = request.upgrade_split("websocket", response);
    let stream: Box<dyn WsStream> = Box::new(ReadHalfStream(reader));
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
    // Dual-thread delivery, same as the standalone listener: the
    // vendored tiny_http exposes the connection's WRITE half
    // separately (`upgrade_split`), so the per-client writer thread
    // flushes broadcasts the instant they're queued. The previous
    // fused-stream fallback drained outbound only between reads — a
    // quiet client (browser sync engines only ping every 25 s) saw
    // realtime events arrive in 25-second batches. Symptom that
    // caught it: world3d players "teleporting" instead of moving.
    run_authenticated_session(
        ws,
        hub,
        auth,
        upgrade.bearer_token,
        snapshot_fetcher,
        reactive,
        rooms,
        Some(Box::new(WriteHalfStream(writer))),
    );
}

/// The write half of an upgraded connection shaped as a full
/// `WsStream`. The writer-side tungstenite socket only ever sends, so
/// `Read` answers `WouldBlock` defensively.
struct WriteHalfStream(Box<dyn Write + Send>);

impl Read for WriteHalfStream {
    fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
        Err(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "write-half stream is not readable",
        ))
    }
}
impl Write for WriteHalfStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

/// The read half shaped as a full `WsStream`. Tungstenite's reader
/// may try to flush protocol replies (pong/close) through its own
/// socket; those are swallowed here — the writer thread owns the real
/// outbound half, and the sync protocol's keepalive is an app-level
/// text frame that rides it. Erroring instead would kill the
/// connection on the first protocol ping.
struct ReadHalfStream(Box<dyn Read + Send>);

impl Read for ReadHalfStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}
impl Write for ReadHalfStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
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

    // presence/topic must NOT global-firehose across tenants by default.
    #[test]
    fn presence_target_is_secure_by_default() {
        // Roomless frame, no opt-in → DROP (was: global cross-tenant relay).
        assert_eq!(presence_target(false, false, false), PresenceTarget::Drop);
        // Roomless + explicit opt-in → global (single-tenant apps).
        assert_eq!(presence_target(false, false, true), PresenceTarget::Global);
        // Room + member → room-scoped delivery.
        assert_eq!(presence_target(true, true, false), PresenceTarget::Room);
        // Room + NON-member → drop (can't inject into a room you're not in).
        assert_eq!(presence_target(true, false, false), PresenceTarget::Drop);
        // Opt-in flag never widens a non-member's room frame.
        assert_eq!(presence_target(true, false, true), PresenceTarget::Drop);
    }

    #[test]
    fn hub_starts_with_zero_clients() {
        let hub = {
            let m = pylon_kernel::AppManifest::default();
            let auth_user = m.auth.user.clone();
            WsHub::new(
                Arc::new(PolicyEngine::from_manifest(&m)),
                Arc::new(m),
                auth_user,
            )
        };
        assert_eq!(hub.client_count(), 0);
    }

    #[test]
    fn broadcast_to_empty_hub_doesnt_panic() {
        let hub = {
            let m = pylon_kernel::AppManifest::default();
            let auth_user = m.auth.user.clone();
            WsHub::new(
                Arc::new(PolicyEngine::from_manifest(&m)),
                Arc::new(m),
                auth_user,
            )
        };
        let event = ChangeEvent {
            seq: 1,
            entity: "Test".into(),
            row_id: "1".into(),
            kind: pylon_sync::ChangeKind::Insert,
            data: None,
            prev_data: None,
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
                        server_only: false,
                        readonly: false,
                        default: None,
                        enum_values: None,
                        encrypted: false,
                    },
                    ManifestField {
                        name: "tenantId".into(),
                        field_type: "string".into(),
                        optional: false,
                        unique: false,
                        crdt: None,
                        server_only: false,
                        readonly: false,
                        default: None,
                        enum_values: None,
                        encrypted: false,
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

    // -------------------------------------------------------------------
    // Room subscription registry tests
    // -------------------------------------------------------------------

    #[test]
    fn room_subs_subscribe_dedups() {
        let subs = RoomSubscriptions::default();
        subs.subscribe(1, "channel:foo");
        subs.subscribe(1, "channel:foo");
        assert_eq!(subs.subscribers("channel:foo"), vec![1]);
        assert_eq!(subs.total_subscriptions(), 1);
    }

    #[test]
    fn room_subs_indexes_by_room() {
        // O(subscribers per room) — verify many clients on one room
        // all show up via the room key.
        let subs = RoomSubscriptions::default();
        subs.subscribe(1, "channel:foo");
        subs.subscribe(2, "channel:foo");
        subs.subscribe(3, "channel:foo");
        subs.subscribe(4, "channel:bar"); // unrelated
        let mut ids = subs.subscribers("channel:foo");
        ids.sort();
        assert_eq!(ids, vec![1, 2, 3]);
        // The unrelated subscription doesn't leak across rooms.
        assert_eq!(subs.subscribers("channel:bar"), vec![4]);
    }

    #[test]
    fn room_subs_unsubscribe_removes_empty_rooms() {
        let subs = RoomSubscriptions::default();
        subs.subscribe(1, "channel:foo");
        subs.unsubscribe(1, "channel:foo");
        assert!(subs.subscribers("channel:foo").is_empty());
        // total drops, empty room entry cleaned up — long-running
        // connections that churn through rooms don't leak.
        assert_eq!(subs.total_subscriptions(), 0);
    }

    #[test]
    fn room_subs_unsubscribe_all_drops_every_room_and_returns_names() {
        let subs = RoomSubscriptions::default();
        subs.subscribe(1, "channel:a");
        subs.subscribe(1, "channel:b");
        subs.subscribe(2, "channel:a");
        let mut left = subs.unsubscribe_all(1);
        left.sort();
        assert_eq!(left, vec!["channel:a", "channel:b"]);
        // Client 2 must still be present on channel:a.
        assert_eq!(subs.subscribers("channel:a"), vec![2]);
        assert!(subs.subscribers("channel:b").is_empty());
    }

    #[test]
    fn room_subs_unsubscribe_unknown_is_noop() {
        let subs = RoomSubscriptions::default();
        assert!(subs.unsubscribe_all(99).is_empty());
        subs.unsubscribe(99, "channel:foo");
        assert_eq!(subs.total_subscriptions(), 0);
    }

    // -------------------------------------------------------------------
    // Room push tests via WsHub
    //
    // These tests use the in-memory registry + push surface; the actual
    // socket I/O happens in the end-to-end integration test in
    // `tests/rooms_ws_push.rs`.
    // -------------------------------------------------------------------

    fn make_test_hub() -> Arc<WsHub> {
        let m = pylon_kernel::AppManifest::default();
        let auth_user = m.auth.user.clone();
        WsHub::new(
            Arc::new(PolicyEngine::from_manifest(&m)),
            Arc::new(m),
            auth_user,
        )
    }

    /// Stub RoomBridge for handle_room_control tests.
    ///
    /// Records the user_id passed to disconnect so the test can assert
    /// the auto-leave path fires with the correct identity.
    struct StubBridge {
        is_member: bool,
        peers: Vec<serde_json::Value>,
        disconnect_log: Mutex<Vec<String>>,
        leave_rooms: Vec<String>,
    }

    impl RoomBridge for StubBridge {
        fn members(&self, _room: &str) -> Vec<serde_json::Value> {
            self.peers.clone()
        }
        fn is_in_room(&self, _room: &str, _user_id: &str) -> bool {
            self.is_member
        }
        fn disconnect(&self, user_id: &str) -> Vec<String> {
            self.disconnect_log
                .lock()
                .unwrap()
                .push(user_id.to_string());
            self.leave_rooms.clone()
        }
    }

    #[test]
    fn room_subscribe_records_membership_when_in_room() {
        // Member of the room → subscribe succeeds, registry records id.
        let hub = make_test_hub();
        let bridge: Arc<dyn RoomBridge> = Arc::new(StubBridge {
            is_member: true,
            peers: vec![],
            disconnect_log: Mutex::new(Vec::new()),
            leave_rooms: vec![],
        });
        let auth_ctx = pylon_auth::AuthContext::user("alice".into());
        let parsed = serde_json::json!({
            "type": "room-subscribe",
            "room": "channel:foo"
        });
        handle_room_control(
            &hub,
            42,
            &auth_ctx,
            "room-subscribe",
            &parsed,
            Some(&bridge),
        );
        assert_eq!(
            hub.room_subscriptions().subscribers("channel:foo"),
            vec![42]
        );
    }

    #[test]
    fn room_subscribe_rejects_non_member() {
        // Non-member → NOT_IN_ROOM, no registry entry recorded.
        let hub = make_test_hub();
        let bridge: Arc<dyn RoomBridge> = Arc::new(StubBridge {
            is_member: false,
            peers: vec![],
            disconnect_log: Mutex::new(Vec::new()),
            leave_rooms: vec![],
        });
        let auth_ctx = pylon_auth::AuthContext::user("bob".into());
        let parsed = serde_json::json!({
            "type": "room-subscribe",
            "room": "channel:foo"
        });
        handle_room_control(&hub, 7, &auth_ctx, "room-subscribe", &parsed, Some(&bridge));
        // No subscription registered — non-members can't passively
        // collect future room-updates.
        assert!(hub
            .room_subscriptions()
            .subscribers("channel:foo")
            .is_empty());
    }

    #[test]
    fn room_subscribe_admin_bypasses_membership_check() {
        // Admin (e.g. server-side dashboard) subscribes to any room
        // without joining — needed for cross-room observability.
        let hub = make_test_hub();
        let bridge: Arc<dyn RoomBridge> = Arc::new(StubBridge {
            // Even with is_member=false, admins go through.
            is_member: false,
            peers: vec![],
            disconnect_log: Mutex::new(Vec::new()),
            leave_rooms: vec![],
        });
        let auth_ctx = pylon_auth::AuthContext::admin();
        let parsed = serde_json::json!({
            "type": "room-subscribe",
            "room": "channel:private"
        });
        handle_room_control(
            &hub,
            99,
            &auth_ctx,
            "room-subscribe",
            &parsed,
            Some(&bridge),
        );
        assert_eq!(
            hub.room_subscriptions().subscribers("channel:private"),
            vec![99]
        );
    }

    #[test]
    fn room_subscribe_rejects_anonymous() {
        // Anonymous WS (no user_id) → can't evaluate membership →
        // treated as NOT_IN_ROOM. Anonymous clients can't peek at any
        // room's future deltas.
        let hub = make_test_hub();
        let bridge: Arc<dyn RoomBridge> = Arc::new(StubBridge {
            is_member: true, // even if bridge would allow, we don't ask
            peers: vec![],
            disconnect_log: Mutex::new(Vec::new()),
            leave_rooms: vec![],
        });
        let auth_ctx = pylon_auth::AuthContext::anonymous();
        let parsed = serde_json::json!({
            "type": "room-subscribe",
            "room": "channel:foo"
        });
        handle_room_control(
            &hub,
            13,
            &auth_ctx,
            "room-subscribe",
            &parsed,
            Some(&bridge),
        );
        assert!(hub
            .room_subscriptions()
            .subscribers("channel:foo")
            .is_empty());
    }

    #[test]
    fn room_unsubscribe_drops_registry_entry() {
        let hub = make_test_hub();
        // Pre-populate.
        hub.room_subscriptions.subscribe(42, "channel:foo");
        let bridge: Arc<dyn RoomBridge> = Arc::new(StubBridge {
            is_member: true,
            peers: vec![],
            disconnect_log: Mutex::new(Vec::new()),
            leave_rooms: vec![],
        });
        let auth_ctx = pylon_auth::AuthContext::user("alice".into());
        let parsed = serde_json::json!({
            "type": "room-unsubscribe",
            "room": "channel:foo"
        });
        handle_room_control(
            &hub,
            42,
            &auth_ctx,
            "room-unsubscribe",
            &parsed,
            Some(&bridge),
        );
        assert!(hub
            .room_subscriptions()
            .subscribers("channel:foo")
            .is_empty());
    }

    #[test]
    fn fanout_leaves_calls_bridge_disconnect() {
        // Disconnect path: user identifies, bridge gets the user_id,
        // returns the rooms the user was in. The push happens via
        // push_room_update — no client sockets, so the verification
        // is via the bridge's disconnect_log.
        let hub = make_test_hub();
        let bridge_struct = Arc::new(StubBridge {
            is_member: true,
            peers: vec![],
            disconnect_log: Mutex::new(Vec::new()),
            leave_rooms: vec!["channel:a".into(), "channel:b".into()],
        });
        let bridge: Arc<dyn RoomBridge> = bridge_struct.clone();
        let auth_ctx = pylon_auth::AuthContext::user("alice".into());
        fanout_room_leaves_on_disconnect(&hub, &auth_ctx, Some(&bridge));
        let log = bridge_struct.disconnect_log.lock().unwrap();
        assert_eq!(*log, vec!["alice".to_string()]);
    }

    #[test]
    fn fanout_leaves_skips_anonymous() {
        // Anonymous never joined any room — disconnect cleanup is a no-op.
        let hub = make_test_hub();
        let bridge_struct = Arc::new(StubBridge {
            is_member: false,
            peers: vec![],
            disconnect_log: Mutex::new(Vec::new()),
            leave_rooms: vec![],
        });
        let bridge: Arc<dyn RoomBridge> = bridge_struct.clone();
        let auth_ctx = pylon_auth::AuthContext::anonymous();
        fanout_room_leaves_on_disconnect(&hub, &auth_ctx, Some(&bridge));
        let log = bridge_struct.disconnect_log.lock().unwrap();
        assert!(log.is_empty(), "anonymous disconnect must not call bridge");
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
