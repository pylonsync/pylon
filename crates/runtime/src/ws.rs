use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::Duration;

use pylon_auth::{AuthContext, SessionStore};
use pylon_policy::{PolicyEngine, PolicyResult};

pub use crate::request_auth::AuthEnricher;
use crate::request_auth::{AuthResolver, Credentials, Identity, Surface};
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
// plus user_by_client (client_id → the user it subscribed as), so a
// closing connection can tell whether another connection of the same
// user still holds a room. All maps live under a single mutex so
// disconnect cleanup and concurrent subscribe never tear.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct RoomSubsState {
    /// room → set of client_ids subscribed to that room.
    by_room: HashMap<String, HashSet<u64>>,
    /// client_id → set of rooms it subscribes to.
    by_client: HashMap<u64, HashSet<String>>,
    /// client_id → user id the client subscribed as. No entry for a
    /// client that subscribed without a user id (an admin context with no user).
    user_by_client: HashMap<u64, String>,
}

/// The result of [`RoomSubscriptions::release_client`].
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ReleasedRooms {
    /// The user the client subscribed as, if any.
    pub user_id: Option<String>,
    /// Rooms the client subscribed to where no other client of the same
    /// user is still subscribed. Empty when `user_id` is `None`.
    pub last_rooms: Vec<String>,
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

    /// Record that `client_id` receives pushes for `room`. `user_id` is
    /// the user the connection is authenticated as; `release_client`
    /// uses it to decide whether the user still holds the room.
    pub fn subscribe(&self, client_id: u64, room: &str, user_id: Option<&str>) {
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
        if let Some(uid) = user_id {
            state.user_by_client.insert(client_id, uid.to_string());
        }
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
                state.user_by_client.remove(&client_id);
            }
        }
    }

    /// Drop every room subscription for this client. Called on WS
    /// disconnect. Returns the client's user and the rooms that user no
    /// longer holds through any other client, in one step under the
    /// lock, so two connections of one user closing at the same time
    /// cannot both conclude that the other still holds the room.
    pub fn release_client(&self, client_id: u64) -> ReleasedRooms {
        let mut guard = self.state.lock().unwrap();
        let state = &mut *guard;
        let user_id = state.user_by_client.remove(&client_id);
        let rooms: Vec<String> = state
            .by_client
            .remove(&client_id)
            .map(|set| set.into_iter().collect())
            .unwrap_or_default();
        let mut last_rooms = Vec::new();
        for room in rooms {
            let mut held_elsewhere = false;
            if let Some(set) = state.by_room.get_mut(&room) {
                set.remove(&client_id);
                if let Some(uid) = user_id.as_deref() {
                    held_elsewhere = set.iter().any(|other| {
                        state.user_by_client.get(other).map(String::as_str) == Some(uid)
                    });
                }
                if set.is_empty() {
                    state.by_room.remove(&room);
                }
            }
            if user_id.is_some() && !held_elsewhere {
                last_rooms.push(room);
            }
        }
        ReleasedRooms {
            user_id,
            last_rooms,
        }
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

/// A queued payload. Shared data becomes an owned WebSocket message only
/// when the writer removes it from the queue.
#[derive(Debug)]
pub enum OutboundMessage {
    Text(Arc<str>),
    Binary(Arc<[u8]>),
    Control(Message),
}

impl OutboundMessage {
    fn into_message(self) -> Message {
        match self {
            Self::Text(text) => Message::Text(text.to_string()),
            Self::Binary(bytes) => Message::Binary(bytes.to_vec()),
            Self::Control(message) => message,
        }
    }
}

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
    pub outbound_tx: mpsc::SyncSender<OutboundMessage>,
    /// Receiver half of the outbound queue, parked here so the reader
    /// thread can take ownership during `run_authenticated_session`.
    /// `mpsc::Receiver` is `!Sync`, so we hand it out once via `take()`
    /// rather than try to share it. After the reader takes the Receiver
    /// this slot is `None`; broadcasters only use `outbound_tx`.
    pub outbound_rx: Mutex<Option<mpsc::Receiver<OutboundMessage>>>,
    pub auth: RwLock<AuthContext>,
    /// The bearer token the connection authenticated with. Re-resolved
    /// when a session of this user ends (see [`WsHub::revalidate_user`]).
    token: Option<String>,
    /// Set when the connection's token stopped resolving. The hub has
    /// already dropped the client; the reader thread ends the session at
    /// its next iteration and processes no further control messages.
    revoked: std::sync::atomic::AtomicBool,
}

impl WsClient {
    fn is_revoked(&self) -> bool {
        self.revoked.load(std::sync::atomic::Ordering::SeqCst)
    }
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

    fn add(
        &self,
        id: u64,
        ws: WebSocket<Box<dyn WsStream>>,
        auth: AuthContext,
        token: Option<String>,
    ) -> ClientSocket {
        let (outbound_tx, outbound_rx) = mpsc::sync_channel(PER_CLIENT_OUTBOUND_DEPTH);
        let handle = Arc::new(WsClient {
            socket: Mutex::new(ws),
            outbound_tx,
            outbound_rx: Mutex::new(Some(outbound_rx)),
            auth: RwLock::new(auth),
            token,
            revoked: std::sync::atomic::AtomicBool::new(false),
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
                .try_send(OutboundMessage::Text(Arc::clone(msg)))
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
            // An admin with NO active tenant bypasses every policy gate; an
            // admin WITH an active tenant is scoped like a member (matches the
            // policy engine + the entity-list/sync read paths — see
            // AuthContext::is_unscoped_admin). A bare `is_admin` here would let
            // an admin-with-tenant receive every tenant's change events.
            let payload: &Arc<str> = if auth.is_unscoped_admin() {
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
                .try_send(OutboundMessage::Text(Arc::clone(payload)))
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
            // Unscoped admin (no active tenant) bypasses; admin-with-tenant is
            // scoped via check_entity_read below. See is_unscoped_admin.
            if auth.is_unscoped_admin() {
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
            match handle
                .outbound_tx
                .try_send(OutboundMessage::Binary(Arc::clone(msg)))
            {
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
    fn send_text_to_one(&self, client_id: u64, text: &Arc<str>) {
        let handle = {
            let clients = self.clients.lock().unwrap();
            clients.get(&client_id).map(Arc::clone)
        };
        let Some(handle) = handle else {
            return;
        };
        match handle
            .outbound_tx
            .try_send(OutboundMessage::Text(Arc::clone(text)))
        {
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

    fn send_text_to_many(&self, ids: &[u64], text: &Arc<str>) {
        if ids.is_empty() {
            return;
        }
        let handles: Vec<_> = {
            let clients = self.clients.lock().unwrap();
            ids.iter()
                .filter_map(|id| clients.get(id).map(|handle| (*id, Arc::clone(handle))))
                .collect()
        };
        let mut dead = Vec::new();
        for (id, handle) in handles {
            match handle
                .outbound_tx
                .try_send(OutboundMessage::Text(Arc::clone(text)))
            {
                Ok(()) => {}
                Err(mpsc::TrySendError::Disconnected(_)) => dead.push((id, handle)),
                Err(mpsc::TrySendError::Full(_)) => {
                    tracing::error!(
                        client_id = id,
                        "[ws] per-client outbound full — dropping room frame"
                    );
                }
            }
        }
        if !dead.is_empty() {
            let mut clients = self.clients.lock().unwrap();
            for (id, handle) in dead {
                if clients
                    .get(&id)
                    .is_some_and(|current| Arc::ptr_eq(current, &handle))
                {
                    clients.remove(&id);
                }
            }
        }
    }

    /// Binary fanout for CRDT updates. Same per-client outbound-queue
    /// pattern as `broadcast` above. Queues share the payload; the writer
    /// copies it into the owned buffer required by tungstenite.
    fn broadcast_binary(&self, msg: &Arc<[u8]>) {
        let handles: Vec<(u64, ClientSocket)> = {
            let clients = self.clients.lock().unwrap();
            clients.iter().map(|(id, h)| (*id, Arc::clone(h))).collect()
        };
        let mut dead: Vec<u64> = Vec::new();
        for (id, handle) in handles {
            match handle
                .outbound_tx
                .try_send(OutboundMessage::Binary(Arc::clone(msg)))
            {
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
                .try_send(OutboundMessage::Text(Arc::clone(msg)))
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

    fn all_clients(&self) -> Vec<(u64, ClientSocket)> {
        let clients = self.clients.lock().unwrap();
        clients.iter().map(|(id, h)| (*id, Arc::clone(h))).collect()
    }

    /// The connections in this shard authenticated as one of `users`.
    fn clients_of_users(&self, users: &HashSet<&str>) -> Vec<(u64, ClientSocket)> {
        let clients = self.clients.lock().unwrap();
        clients
            .iter()
            .filter(|(_, h)| {
                let auth = match h.auth.read() {
                    Ok(g) => g,
                    Err(poisoned) => poisoned.into_inner(),
                };
                auth.user_id.as_deref().is_some_and(|u| users.contains(u))
            })
            .map(|(id, h)| (*id, Arc::clone(h)))
            .collect()
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
    fn update_tenant_for_user(
        &self,
        user_id: &str,
        new_tenant: Option<&str>,
        enrich: Option<&AuthEnricher>,
    ) -> usize {
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
            // Roles are per-ACTIVE-ORG, so the old org's role must not
            // survive the switch — `enrich_active_org_role` no-ops when
            // `roles` is already populated, so clear before re-running.
            // Skipped when no enricher is installed: clearing without a
            // way to repopulate would only lose information.
            if let Some(enrich) = enrich {
                auth.roles.clear();
                enrich(&mut auth);
            }
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
    /// Manifest snapshot for client wire projection. Workers check the
    /// raw row against each client's policy before sending the projected frame.
    /// Pre-fix the notifier projected before broadcast, stripping
    /// fields from the policy check input.
    manifest: Arc<pylon_kernel::AppManifest>,
    crdt_private_entities: std::sync::OnceLock<Arc<dyn Fn(&str) -> bool + Send + Sync>>,
    /// Auth-user manifest config, used by `maybe_project_user_row`
    /// inside the wire projection.
    auth_user: pylon_kernel::ManifestAuthUserConfig,
    /// Identity-completion hook re-applied when a client's active org
    /// changes (`update_tenant_for_user`). See [`AuthEnricher`].
    auth_enricher: Mutex<Option<AuthEnricher>>,
    /// The token resolver the connections authenticated with, used to
    /// re-resolve them when a session ends ([`WsHub::revalidate_user`]).
    /// Set once at boot by the server.
    auth_resolver: Mutex<Option<Arc<AuthResolver>>>,
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
            crdt_private_entities: std::sync::OnceLock::new(),
            auth_user,
            auth_enricher: Mutex::new(None),
            auth_resolver: Mutex::new(None),
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
        // Resolve all recipients in each shard under one lock.
        let mut by_shard: Vec<Vec<u64>> = (0..NUM_SHARDS).map(|_| Vec::new()).collect();
        for id in &subscribers {
            by_shard[(*id as usize) % NUM_SHARDS].push(*id);
        }
        let shared: Arc<str> = Arc::from(text);
        for (idx, ids) in by_shard.iter().enumerate() {
            self.shards[idx].send_text_to_many(ids, &shared);
        }
        // The reader removes room subscriptions when it observes EOF.
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

    pub(crate) fn set_crdt_private_history(&self, check: Arc<dyn Fn(&str) -> bool + Send + Sync>) {
        assert!(
            self.crdt_private_entities.set(check).is_ok(),
            "CRDT privacy history is set once before accepting clients"
        );
    }

    pub(crate) fn supports_crdt_replication(&self, entity: &str) -> bool {
        pylon_router::supports_crdt_replication(&self.manifest, &self.auth_user, entity)
            && !self
                .crdt_private_entities
                .get()
                .is_some_and(|check| check(entity))
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
        // `sync: false` entities never reach a client replica; the pull
        // path skips them, and so does the live fan-out.
        if !pylon_router::is_replicated_entity(&self.manifest, &event.entity) {
            return;
        }
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
        let projected_data = pylon_router::project_row_for_replication_opt_ref(
            self.manifest.as_ref(),
            &self.auth_user,
            &event.entity,
            event.data.as_ref(),
        );
        let wire_event_for_clients = ChangeEvent {
            seq: event.seq,
            entity: event.entity.clone(),
            row_id: event.row_id.clone(),
            kind: event.kind.clone(),
            data: projected_data,
            prev_data: None,
            timestamp: event.timestamp.clone(),
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
                let projected_prev = pylon_router::project_row_for_replication_opt_ref(
                    self.manifest.as_ref(),
                    &self.auth_user,
                    &event.entity,
                    event.prev_data.as_ref(),
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
        if !self.supports_crdt_replication(entity) || client_ids.is_empty() {
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
        let enrich = match self.auth_enricher.lock() {
            Ok(g) => g.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        self.shards
            .iter()
            .map(|s| s.update_tenant_for_user(user_id, new_tenant, enrich.as_ref()))
            .sum()
    }

    /// Install the resolver used to re-check connection tokens when a
    /// session ends. Set once at boot by the server.
    pub fn set_auth_resolver(&self, auth: Arc<AuthResolver>) {
        match self.auth_resolver.lock() {
            Ok(mut g) => *g = Some(auth),
            Err(poisoned) => *poisoned.into_inner() = Some(auth),
        }
    }

    /// Re-resolve the token of every connection authenticated as
    /// `user_id`, and end each connection whose token no longer resolves
    /// to that user (logout, session revocation, password change). Returns
    /// the ended connections' ids so the caller can drop their reactive
    /// subscriptions.
    ///
    /// An ended connection is dropped from the hub at once (no further
    /// broadcast, per-user push, or CRDT / room frame reaches it) and
    /// gets a Policy close frame. The client reconnects with whatever
    /// token it holds now: a refreshed one, or none after a logout. Its
    /// reader thread processes no further messages.
    pub fn revalidate_user(&self, user_id: &str) -> Vec<u64> {
        self.revalidate_users(&[user_id.to_string()])
    }

    /// [`WsHub::revalidate_user`] for several users in one pass over the
    /// connections (a session sweep ends many at once).
    pub fn revalidate_users(&self, user_ids: &[String]) -> Vec<u64> {
        let users: HashSet<&str> = user_ids.iter().map(String::as_str).collect();
        self.revalidate(Some(&users))
    }

    /// [`WsHub::revalidate_user`] for every authenticated connection.
    /// Catches what no session event reports: an expired session or JWT,
    /// a revoked API key.
    pub fn revalidate_all_clients(&self) -> Vec<u64> {
        self.revalidate(None)
    }

    fn revalidate(&self, only_users: Option<&HashSet<&str>>) -> Vec<u64> {
        let resolver = match self.auth_resolver.lock() {
            Ok(g) => g.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        let Some(resolver) = resolver else {
            return Vec::new();
        };
        let mut ended = Vec::new();
        for shard in &self.shards {
            let handles: Vec<(u64, ClientSocket)> = match only_users {
                Some(users) => shard.clients_of_users(users),
                None => shard.all_clients(),
            };
            for (id, handle) in handles {
                // An anonymous connection has no credential to end.
                if handle.token.is_none() {
                    continue;
                }
                let user_id = match handle.auth.read() {
                    Ok(g) => g.user_id.clone(),
                    Err(poisoned) => poisoned.into_inner().user_id.clone(),
                };
                let still_valid = resolver
                    .recheck_token(handle.token.as_deref(), Surface::App)
                    .is_ok_and(|ctx| ctx.user_id == user_id);
                if !still_valid {
                    self.end_client(id, &handle);
                    ended.push(id);
                }
            }
        }
        ended
    }

    /// Drop a connection whose credentials ended: out of the hub and every
    /// subscription registry, marked revoked, and sent a Policy close.
    fn end_client(&self, id: u64, handle: &ClientSocket) {
        handle
            .revoked
            .store(true, std::sync::atomic::Ordering::SeqCst);
        // Room subscriptions stay until the reader's `end_session`: only it
        // holds the room bridge that announces the leaves.
        self.subscriptions.unsubscribe_all(id);
        self.remove_client(id);
        let _ = handle
            .outbound_tx
            .try_send(OutboundMessage::Control(Message::Close(Some(
                tungstenite::protocol::CloseFrame {
                    code: tungstenite::protocol::frame::coding::CloseCode::Policy,
                    reason: "session ended".into(),
                },
            ))));
    }

    /// Install the identity-completion hook used when a connection's
    /// active org changes. Set once at boot by the server, which owns the
    /// org store; see [`AuthEnricher`].
    pub fn set_auth_enricher(&self, enrich: AuthEnricher) {
        match self.auth_enricher.lock() {
            Ok(mut g) => *g = Some(enrich),
            Err(poisoned) => *poisoned.into_inner() = Some(enrich),
        }
    }

    pub fn send_text_to(&self, client_id: u64, text: &str) {
        let shard_idx = (client_id as usize) % NUM_SHARDS;
        self.shards[shard_idx].send_text_to_one(client_id, &Arc::from(text));
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
        token: Option<String>,
    ) -> (u64, ClientSocket) {
        let mut next_id = self.next_id.lock().unwrap();
        let id = *next_id;
        *next_id += 1;
        let shard_idx = (id as usize) % NUM_SHARDS;
        let handle = self.shards[shard_idx].add(id, ws, auth, token);
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

    /// Remove the user from the room and deliver `room-update`
    /// action:leave to the room's subscribers. Returns false, and
    /// delivers nothing, when the user was not in the room. Called at
    /// WS close for each room the closing connection was the user's
    /// last subscriber to.
    fn leave(&self, room: &str, user_id: &str) -> bool;
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
pub fn start_ws_server(
    hub: Arc<WsHub>,
    auth: Arc<AuthResolver>,
    port: u16,
    snapshot_fetcher: Option<SnapshotFetcher>,
    reactive: Option<Arc<crate::reactive::ReactiveRegistry>>,
    rooms: Option<Arc<dyn RoomBridge>>,
    scope: &crate::listen::ListenScope,
) {
    // Dual-stack v6+v4 for `ListenScope::All`. The Yapless Mac app + any
    // client that resolves `localhost` to `::1` first (the macOS default)
    // would otherwise see "connection refused" on the WS port even though
    // the HTTP server on the next port up is reachable. See
    // crate::bind_dual_stack_tcp for the rationale. `Loopback` binds
    // 127.0.0.1 and ::1 for the same reason.
    let listener = match crate::listen::Listeners::bind(port, scope) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("[ws] Failed to bind on port {port}: {e}");
            return;
        }
    };

    tracing::info!(
        "[ws] WebSocket server listening on ws://localhost:{port} (sharded, {NUM_SHARDS} shards)"
    );

    let ip_counter = Arc::new(IpConnCounter::default());

    loop {
        // Panic-proof accept. libstd's accept/peer_addr `sockaddr` conversion
        // ASSERTS (panics, not errors) when the kernel returns a short
        // address — observed on macOS dual-stack `[::]` when the peer
        // disconnects mid-accept. crate::accept_tcp accepts with a null addr
        // (nothing to parse) and decodes the peer IP defensively.
        let (stream, peer_ip) = match listener.accept() {
            Ok(v) => v,
            Err(_) => {
                std::thread::sleep(std::time::Duration::from_millis(1));
                continue;
            }
        };

        // Per-IP connection cap: reject BEFORE the handshake so a cheap
        // connect storm doesn't force us through tungstenite's HTTP parse
        // and the session-resolve round trip. The guard is dropped when
        // the reader thread exits (or fails to start), freeing the slot. An
        // unparseable peer (the truncation case) buckets under the
        // unspecified address so the cap still bounds it.
        let ip = peer_ip.unwrap_or(std::net::IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED));
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
        // 256 KiB, like the server's stream threads. 64 KiB overflowed on
        // Windows debug builds once connection setup grew (ws_live_fanout:
        // "thread 'ws-client' has overflowed its stack"). The size is
        // reserved address space; pages are committed only as used.
        let spawn_result = thread::Builder::new()
            .name("ws-client".into())
            .stack_size(256 * 1024)
            .spawn(move || {
                let _conn_slot = guard;
                handle_ws_connection(
                    hub,
                    auth,
                    stream,
                    peer_ip.map(|ip| ip.to_string()).unwrap_or_default(),
                    fetcher,
                    reactive_cl,
                    rooms_cl,
                );
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
    auth: Arc<AuthResolver>,
    stream: TcpStream,
    // The socket peer's address; empty when it could not be read.
    socket_ip: String,
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

    // The handshake's credentials: the Authorization header (native
    // clients) or the `bearer.<token>` subprotocol (browsers). This listener
    // takes no session cookie. They are resolved AFTER accept_hdr completes,
    // since the header callback must return synchronously with a Response.
    let token_slot: Arc<Mutex<Credentials>> = Arc::new(Mutex::new(Credentials::default()));
    let slot_for_cb = Arc::clone(&token_slot);
    // The client's address as the HTTP server would resolve it (trusted
    // proxy hops, client-IP headers), from the handshake headers.
    let ip_slot: Arc<Mutex<String>> = Arc::new(Mutex::new(socket_ip.clone()));
    let ip_for_cb = Arc::clone(&ip_slot);
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
            let creds = Credentials::from_headers(
                req.headers()
                    .iter()
                    .filter_map(|(name, value)| Some((name.as_str(), value.to_str().ok()?))),
                &[],
            );
            // RFC 6455 §11.3.4 — echo the chosen subprotocol in the response or
            // browsers will refuse the connection.
            if let Some(chosen) = &creds.subprotocol {
                if let Ok(hv) = tungstenite::http::HeaderValue::from_str(chosen) {
                    resp.headers_mut().insert("Sec-WebSocket-Protocol", hv);
                }
            }
            *slot_for_cb.lock().unwrap() = creds;
            *ip_for_cb.lock().unwrap() = handshake_client_ip(
                req,
                socket_ip.clone(),
                crate::server::client_ip_headers(),
                crate::server::trust_proxy_hops_from_env(),
            );
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
    let creds = token_slot.lock().unwrap().clone();
    let identity = auth.identify(&creds, Surface::App, || false);
    let client_ip = Some(ip_slot.lock().unwrap().clone()).filter(|ip| !ip.is_empty());

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
        identity,
        client_ip,
        snapshot_fetcher,
        reactive,
        rooms,
        write_stream,
    );
}

/// The client IP of a WebSocket handshake on the dedicated listener,
/// resolved as the HTTP server resolves a request's.
fn handshake_client_ip(
    req: &Request,
    socket_ip: String,
    client_ip_headers: &[String],
    trust_proxy_hops: usize,
) -> String {
    crate::server::client_ip_from(
        socket_ip,
        |name| {
            req.headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        },
        client_ip_headers,
        trust_proxy_hops,
    )
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
#[allow(clippy::too_many_arguments)]
fn run_authenticated_session(
    ws: WebSocket<Box<dyn WsStream>>,
    hub: Arc<WsHub>,
    auth: Arc<AuthResolver>,
    // The handshake's identity, from `AuthResolver::identify`.
    identity: Result<Identity, &'static str>,
    // The client's address, for the per-function rate limit of an
    // anonymous caller's reactive subscriptions. `None` when unknown.
    client_ip: Option<String>,
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
    let Identity {
        ctx: mut auth_ctx,
        token,
        ..
    } = match identity {
        Ok(identity) => identity,
        Err(reason) => {
            let mut ws = ws;
            let _ = ws.close(Some(tungstenite::protocol::CloseFrame {
                code: tungstenite::protocol::frame::coding::CloseCode::Policy,
                reason: format!("unauthorized: {reason}").into(),
            }));
            return;
        }
    };
    // Admin lift + active-org role, as every transport does. Without the
    // role, every `auth.hasAnyRole(...)` read policy denies the broadcast.
    auth.enrich(&mut auth_ctx);
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
    let (client_id, socket_handle) = hub.add_client(ws, auth_ctx.clone(), token);

    // Dual-thread path: spawn the writer thread on the cloned TcpStream.
    // It owns `outbound_rx` and `WebSocket::from_raw_socket`-wraps the
    // cloned stream — no shared mutex with the reader, so broadcasts
    // wake the writer instantly via `recv()` instead of waiting for the
    // reader's drain block to run between blocking reads.
    let outbound_rx_for_reader = if let Some(write_stream) = dual_write_stream {
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
        let mut writer_ws = WebSocket::from_raw_socket(write_stream, Role::Server, Some(ws_config));
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
                    if writer_ws.send(msg.into_message()).is_err() {
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
        // The hub ended this connection (its session ended). It is already
        // out of the hub and every registry; drop what the reader still
        // holds and stop.
        if socket_handle.is_revoked() {
            // The hub already dropped this connection from its client map
            // and CRDT registry; release its rooms (leaving only those no
            // other connection holds), its reactive subscriptions, and
            // announce the disconnect, as every other exit does.
            end_session(&hub, client_id, reactive.as_ref(), rooms.as_ref());
            if let Some(ref outbound_rx) = outbound_rx_for_reader {
                // Single-thread mode: this thread owns the send side.
                let mut guard = match socket_handle.socket.lock() {
                    Ok(g) => g,
                    Err(poisoned) => poisoned.into_inner(),
                };
                while let Ok(out_msg) = outbound_rx.try_recv() {
                    if guard.send(out_msg.into_message()).is_err() {
                        break;
                    }
                }
            }
            break;
        }
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
                if guard.send(out_msg.into_message()).is_err() {
                    // Socket dead. Drop the guard, sweep the client,
                    // and exit the session. The remaining queued
                    // messages will be discarded when the channel
                    // half is dropped.
                    drop(guard);
                    end_session(&hub, client_id, reactive.as_ref(), rooms.as_ref());
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
                // A message that arrived after the session ended is not
                // processed under the ended identity.
                if socket_handle.is_revoked() {
                    continue;
                }
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
                            // Rate-limit key: the user, else the address,
                            // matching `POST /api/fn/<name>`.
                            let rate_identity = reactive_rate_identity(
                                auth_ctx.user_id.as_deref(),
                                client_ip.as_deref(),
                                client_id,
                            );
                            handle_reactive_control(
                                reg,
                                &hub,
                                client_id,
                                &auth_ctx,
                                &rate_identity,
                                kind,
                                &parsed,
                            );
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
                let _ = socket_handle
                    .outbound_tx
                    .try_send(OutboundMessage::Control(Message::Pong(data)));
            }
            Ok(Message::Close(_)) => {
                end_session(&hub, client_id, reactive.as_ref(), rooms.as_ref());
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
                end_session(&hub, client_id, reactive.as_ref(), rooms.as_ref());
                break;
            }
            _ => {}
        }
    }
}

/// Clean up after a WS connection ends, for every way the reader loop
/// exits.
///
/// CRDT and room subscriptions are dropped BEFORE `remove_client`, so
/// the broadcast path never looks up a stale client_id between the two
/// steps. The user leaves each room this connection was their last
/// subscriber to (see [`leave_released_rooms`]).
fn end_session(
    hub: &Arc<WsHub>,
    client_id: u64,
    reactive: Option<&Arc<crate::reactive::ReactiveRegistry>>,
    rooms: Option<&Arc<dyn RoomBridge>>,
) {
    hub.subscriptions.unsubscribe_all(client_id);
    let released = hub.room_subscriptions.release_client(client_id);
    if let Some(reg) = reactive {
        reg.disconnect_client(client_id);
    }
    leave_released_rooms(&released, rooms);
    hub.remove_client(client_id);
    let disconnect = serde_json::json!({
        "type": "presence",
        "event": "disconnect",
        "clientId": client_id,
    });
    hub.broadcast_presence(&disconnect.to_string());
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
            if !hub.supports_crdt_replication(entity) {
                return;
            }
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
                if !hub.supports_crdt_replication(entity) {
                    hub.subscriptions.unsubscribe(client_id, entity, row_id);
                    return;
                }
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
            hub.room_subscriptions
                .subscribe(client_id, room, auth_ctx.user_id.as_deref());
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

/// Auto-leave on WS close. For each room in `released.last_rooms` (rooms
/// the closing connection subscribed to, with no other connection of
/// the same user still subscribed), the bridge removes the user and
/// pushes `room-update` action:leave to the room's subscribers.
///
/// Rooms the connection never subscribed to are not touched: a user
/// can be in a room over HTTP only, or hold it from another device.
/// The idle sweep removes members that stop sending heartbeats.
///
/// A connection that subscribed without a user id (an admin context) has
/// `user_id: None` and leaves nothing.
fn leave_released_rooms(released: &ReleasedRooms, rooms: Option<&Arc<dyn RoomBridge>>) {
    let Some(bridge) = rooms else {
        return;
    };
    let Some(user_id) = released.user_id.as_deref() else {
        return;
    };
    for room in &released.last_rooms {
        bridge.leave(room, user_id);
    }
}

/// The rate-limit key for a connection's reactive subscriptions: the
/// user, else the client IP, else this connection alone. A shared
/// fallback key would let one client with no known address use up the
/// allowance of every other such client.
fn reactive_rate_identity(
    user_id: Option<&str>,
    client_ip: Option<&str>,
    client_id: u64,
) -> String {
    match (user_id, client_ip) {
        (Some(user), _) => user.to_string(),
        (None, Some(ip)) => ip.to_string(),
        (None, None) => format!("ws-client:{client_id}"),
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
    rate_identity: &str,
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
            if let Err((code, message)) =
                reg.check_subscribe(&fn_name, auth_ctx, rate_identity, client_id, &sub_id, &args)
            {
                let frame = serde_json::json!({
                    "type": "reactive-error",
                    "sub_id": sub_id,
                    "code": code,
                    "message": message,
                })
                .to_string();
                hub.send_text_to(client_id, &frame);
                return;
            }
            // Map AuthContext → AuthInfo. Carries the FULL identity
            // (roles included) so RBAC-style policies see the same
            // values on re-run as on the first run.
            let auth_info = pylon_functions::protocol::AuthInfo {
                user_id: auth_ctx.user_id.clone(),
                is_admin: auth_ctx.is_admin,
                tenant_id: auth_ctx.tenant_id.clone(),
                roles: auth_ctx.roles.clone(),
                is_guest: auth_ctx.is_guest,
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
pub(crate) fn percent_decode_token(s: &str) -> Option<String> {
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
pub(crate) fn ws_accept_value(client_key: &str) -> String {
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
    /// The handshake's credentials: `Authorization: Bearer`, the
    /// `bearer.<token>` subprotocol (echoed back, or browsers refuse the
    /// connection), and the session cookie. The dispatch site resolves them
    /// with `AuthResolver::identify`, trusting the cookie only from a trusted
    /// Origin: a browser attaches the victim's cookie to a cross-origin
    /// handshake (cross-site WebSocket hijacking), while an explicit token
    /// is not ambient and stays Origin-agnostic for native clients.
    pub credentials: Credentials,
    /// The client's address as the HTTP server resolved it (trusted
    /// proxy hops applied). Set by the dispatch site; `None` from
    /// [`inspect_ws_upgrade`].
    pub client_ip: Option<String>,
}

/// Pull the headers we need to perform a WS upgrade. Returns `None`
/// when the request isn't a WebSocket upgrade attempt (no
/// `Sec-WebSocket-Key`, or no `Upgrade: websocket`). `cookie_names` are the
/// session cookies to read, first match wins (see `Credentials`).
pub fn inspect_ws_upgrade(
    headers: &[tiny_http::Header],
    cookie_names: &[&str],
) -> Option<WsUpgradeRequest> {
    let header = |name: &str| {
        headers
            .iter()
            .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
            .map(|h| h.value.as_str())
    };
    if !header("Upgrade").is_some_and(|v| v.eq_ignore_ascii_case("websocket")) {
        return None;
    }
    let sec_key = header("Sec-WebSocket-Key")?.to_string();
    let credentials = Credentials::from_headers(
        headers
            .iter()
            .map(|h| (h.field.as_str().as_str(), h.value.as_str())),
        cookie_names,
    );
    Some(WsUpgradeRequest {
        sec_key,
        credentials,
        client_ip: None,
    })
}

/// Hijack a tiny_http request as a WebSocket. Writes the 101
/// response, takes ownership of the raw stream, wraps in tungstenite
/// without re-handshaking, and runs the standard per-client loop.
/// Spawn this on its own thread — the loop blocks on `socket.read()`.
#[allow(clippy::too_many_arguments)]
pub fn handle_http_upgrade(
    request: tiny_http::Request,
    upgrade: WsUpgradeRequest,
    identity: Result<Identity, &'static str>,
    hub: Arc<WsHub>,
    auth: Arc<AuthResolver>,
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
    if let Some(proto) = &upgrade.credentials.subprotocol {
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
        identity,
        upgrade.client_ip,
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

    fn queued_client(shard: &Shard, id: u64) -> ClientSocket {
        let stream: Box<dyn WsStream> = Box::new(std::io::Cursor::new(Vec::<u8>::new()));
        shard.add(
            id,
            WebSocket::from_raw_socket(stream, Role::Server, None),
            AuthContext::admin(),
            None,
        )
    }

    #[test]
    fn privacy_change_during_snapshot_fetch_blocks_bootstrap() {
        use std::sync::atomic::Ordering;
        let manifest = Arc::new(pylon_kernel::AppManifest {
            entities: vec![pylon_kernel::ManifestEntity {
                name: "Doc".into(),
                crdt: true,
                sync: true,
                ..Default::default()
            }],
            ..Default::default()
        });
        let hub = WsHub::new(
            Arc::new(PolicyEngine::from_manifest(&manifest)),
            manifest.clone(),
            manifest.auth.user.clone(),
        );
        let restricted = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let check = restricted.clone();
        hub.set_crdt_private_history(Arc::new(move |_| check.load(Ordering::SeqCst)));
        let client = queued_client(&hub.shards[0], 0);
        let rx = client.outbound_rx.lock().unwrap().take().unwrap();
        let fetches = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls = fetches.clone();
        let fetcher: SnapshotFetcher = Arc::new(move |_, _, _| {
            if calls.fetch_add(1, Ordering::SeqCst) == 1 {
                restricted.store(true, Ordering::SeqCst);
            }
            Some(b"private-history".to_vec())
        });
        handle_crdt_control(
            &hub,
            0,
            &AuthContext::admin(),
            "crdt-subscribe",
            &serde_json::json!({"entity":"Doc","rowId":"row"}),
            Some(&fetcher),
        );
        assert_eq!(fetches.load(Ordering::SeqCst), 2);
        assert!(hub.subscriptions.subscribers("Doc", "row").is_empty());
        assert!(matches!(rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
    }

    #[test]
    fn private_crdt_history_is_denied_on_subscribe_and_incoming_cluster_frames() {
        use pylon_kernel::{AppManifest, ManifestEntity, ManifestField};
        struct Bus(Mutex<Option<pylon_cluster::SubscriberHandler>>);
        impl pylon_cluster::ClusterBus for Bus {
            fn publish(&self, _: &pylon_cluster::Envelope) {}
            fn subscribe(&self, handler: pylon_cluster::SubscriberHandler) {
                *self.0.lock().unwrap() = Some(handler);
            }
            fn instance_id(&self) -> &str {
                "local"
            }
            fn is_active(&self) -> bool {
                true
            }
        }
        let manifest = Arc::new(AppManifest {
            entities: vec![
                ManifestEntity {
                    name: "Restricted".into(),
                    crdt: true,
                    sync: true,
                    fields: vec![ManifestField {
                        name: "secret".into(),
                        field_type: "string".into(),
                        server_only: true,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                ManifestEntity {
                    name: "FormerlyPrivate".into(),
                    crdt: true,
                    sync: true,
                    ..Default::default()
                },
                ManifestEntity {
                    name: "Public".into(),
                    crdt: true,
                    sync: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
        let policy = Arc::new(PolicyEngine::from_manifest(&manifest));
        let hub = WsHub::new(policy.clone(), manifest.clone(), manifest.auth.user.clone());
        hub.set_crdt_private_history(Arc::new(|entity| entity == "FormerlyPrivate"));
        let client = queued_client(&hub.shards[0], 0);
        let rx = client.outbound_rx.lock().unwrap().take().unwrap();
        let bus = Arc::new(Bus(Mutex::new(None)));
        let dyn_bus: Arc<dyn pylon_cluster::ClusterBus> = bus.clone();
        let sse = crate::sse::SseHub::new(policy, manifest.clone(), manifest.auth.user.clone());
        crate::datastore::install_cluster_bus_subscriber(
            &dyn_bus,
            hub.clone(),
            sse,
            Arc::new(pylon_sync::ChangeLog::new()),
            manifest.auth.user.clone(),
            manifest,
            None,
            None,
        );
        let handler = bus.0.lock().unwrap().clone().unwrap();
        for entity in ["Restricted", "FormerlyPrivate", "User", "Unknown"] {
            handle_crdt_control(
                &hub,
                0,
                &AuthContext::admin(),
                "crdt-subscribe",
                &serde_json::json!({"entity":entity,"rowId":"row"}),
                None,
            );
            assert!(hub.subscriptions.subscribers(entity, "row").is_empty());
            // Simulate a subscription registered before the restriction.
            hub.subscriptions.subscribe(0, entity, "row");
            handler(pylon_cluster::Envelope::crdt(
                "peer",
                entity,
                "row",
                b"private-history",
            ));
            assert!(matches!(rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
        }
        hub.subscriptions.subscribe(0, "Public", "row");
        handler(pylon_cluster::Envelope::crdt(
            "peer",
            "Public",
            "row",
            b"public-history",
        ));
        assert!(
            rx.try_recv().is_ok(),
            "public peer frames must still reach the client"
        );
    }

    #[test]
    fn slow_clients_share_queued_payload_allocations() {
        let shard = Shard::new();
        let clients: Vec<_> = (0..16).map(|id| queued_client(&shard, id)).collect();
        let text: Arc<str> = Arc::from("x".repeat(32 * 1024));
        let binary: Arc<[u8]> = Arc::from(vec![42; 32 * 1024]);
        // Leave every receiver idle until all queues reach their limit.
        for _ in 0..PER_CLIENT_OUTBOUND_DEPTH / 2 {
            shard.broadcast(&text);
            shard.broadcast_binary(&binary);
        }
        let copies = clients.len() * (PER_CLIENT_OUTBOUND_DEPTH / 2);
        assert_eq!(Arc::strong_count(&text), copies + 1);
        assert_eq!(Arc::strong_count(&binary), copies + 1);
        for client in clients {
            let rx = client.outbound_rx.lock().unwrap().take().unwrap();
            for _ in 0..PER_CLIENT_OUTBOUND_DEPTH / 2 {
                match rx.try_recv().unwrap() {
                    OutboundMessage::Text(queued) => assert!(Arc::ptr_eq(&text, &queued)),
                    other => panic!("expected text, got {other:?}"),
                }
                match rx.try_recv().unwrap() {
                    OutboundMessage::Binary(queued) => assert!(Arc::ptr_eq(&binary, &queued)),
                    other => panic!("expected binary, got {other:?}"),
                }
            }
            assert!(matches!(rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
        }
        assert_eq!(Arc::strong_count(&text), 1);
        assert_eq!(Arc::strong_count(&binary), 1);
    }

    #[test]
    fn targeted_text_batch_preserves_recipients_and_queue_behavior() {
        let shard = Shard::new();
        let healthy = queued_client(&shard, 1);
        let full = queued_client(&shard, 2);
        let disconnected = queued_client(&shard, 3);
        let other = queued_client(&shard, 4);
        drop(disconnected.outbound_rx.lock().unwrap().take().unwrap());
        let text: Arc<str> = Arc::from("room update");
        for _ in 0..PER_CLIENT_OUTBOUND_DEPTH {
            full.outbound_tx
                .try_send(OutboundMessage::Text(text.clone()))
                .unwrap();
        }
        shard.send_text_to_many(&[1, 2, 3, 999], &text);
        let rx = healthy.outbound_rx.lock().unwrap().take().unwrap();
        match rx.try_recv().unwrap() {
            OutboundMessage::Text(queued) => assert!(Arc::ptr_eq(&text, &queued)),
            message => panic!("unexpected message: {message:?}"),
        }
        assert!(matches!(rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
        assert!(matches!(
            other
                .outbound_rx
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        let clients = shard.clients.lock().unwrap();
        assert!(clients.contains_key(&1));
        assert!(clients.contains_key(&2));
        assert!(!clients.contains_key(&3));
        assert!(clients.contains_key(&4));
    }

    #[test]
    fn full_queue_returns_shared_payload_without_conversion() {
        let (tx, rx) = mpsc::sync_channel(1);
        tx.try_send(OutboundMessage::Control(Message::Pong(vec![1])))
            .unwrap();
        let text: Arc<str> = Arc::from("queued text");
        let binary: Arc<[u8]> = Arc::from(vec![2; 32 * 1024]);
        match tx.try_send(OutboundMessage::Text(Arc::clone(&text))) {
            Err(mpsc::TrySendError::Full(OutboundMessage::Text(rejected))) => {
                assert!(Arc::ptr_eq(&text, &rejected));
            }
            other => panic!("expected full text queue, got {other:?}"),
        }
        match tx.try_send(OutboundMessage::Binary(Arc::clone(&binary))) {
            Err(mpsc::TrySendError::Full(OutboundMessage::Binary(rejected))) => {
                assert!(Arc::ptr_eq(&binary, &rejected));
            }
            other => panic!("expected full binary queue, got {other:?}"),
        }
        assert_eq!(Arc::strong_count(&text), 1);
        assert_eq!(Arc::strong_count(&binary), 1);
        assert_eq!(rx.recv().unwrap().into_message(), Message::Pong(vec![1]));
    }

    #[test]
    fn queued_data_and_control_frames_keep_order_and_contents() {
        let shard = Shard::new();
        let client = queued_client(&shard, 1);
        let text: Arc<str> = Arc::from("hello");
        let binary: Arc<[u8]> = Arc::from(vec![2, 3]);
        shard.send_text_to_one(1, &text);
        client
            .outbound_tx
            .try_send(OutboundMessage::Control(Message::Pong(vec![4])))
            .unwrap();
        shard.send_binary_to(&[1], &binary);
        let close = Message::Close(Some(tungstenite::protocol::CloseFrame {
            code: tungstenite::protocol::frame::coding::CloseCode::Policy,
            reason: "session ended".into(),
        }));
        client
            .outbound_tx
            .try_send(OutboundMessage::Control(close.clone()))
            .unwrap();
        let rx = client.outbound_rx.lock().unwrap().take().unwrap();
        for expected in [
            Message::Text("hello".into()),
            Message::Pong(vec![4]),
            Message::Binary(vec![2, 3]),
            close,
        ] {
            assert_eq!(rx.recv().unwrap().into_message(), expected);
        }
    }

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
    fn upgrade_credentials_resolve_through_the_shared_rules() {
        // CSWSH gate: the cookie counts only when the dispatch site trusts
        // the Origin; an explicit token (header or subprotocol) is not
        // ambient and needs no Origin.
        fn hdr(name: &str, value: &str) -> tiny_http::Header {
            tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes()).unwrap()
        }
        let sessions = Arc::new(SessionStore::new());
        let cookie_session = sessions.create("user-cookie".into());
        let bearer_session = sessions.create("user-bearer".into());
        let auth = AuthResolver {
            sessions: Arc::clone(&sessions),
            api_keys: Arc::new(pylon_auth::api_key::ApiKeyStore::new()),
            admin_token: None,
            jwt_secret: None,
            jwt_issuer: None,
            accounts: None,
            enrich: None,
        };
        let upgrade = |extra: Vec<tiny_http::Header>| {
            let mut headers = vec![
                hdr("Upgrade", "websocket"),
                hdr("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ=="),
            ];
            headers.extend(extra);
            inspect_ws_upgrade(&headers, &["app_session"]).unwrap()
        };
        let cookie = || hdr("Cookie", &format!("app_session={}", cookie_session.token));
        let user = |r: &WsUpgradeRequest, trusted: bool| {
            auth.identify(&r.credentials, Surface::App, || trusted)
                .unwrap()
        };

        // Cookie only: used from a trusted Origin, refused otherwise.
        let r = upgrade(vec![cookie()]);
        let id = user(&r, true);
        assert_eq!(id.ctx.user_id.as_deref(), Some("user-cookie"));
        assert!(id.cookie_auth);
        assert!(user(&r, false).cookie_refused);

        // A valid explicit token wins over the cookie, from any Origin.
        let r = upgrade(vec![
            hdr(
                "Sec-WebSocket-Protocol",
                &format!("bearer.{}", bearer_session.token),
            ),
            cookie(),
        ]);
        let id = user(&r, false);
        assert_eq!(id.ctx.user_id.as_deref(), Some("user-bearer"));
        assert!(!id.cookie_auth);
        assert_eq!(
            r.credentials.subprotocol.as_deref(),
            Some(format!("bearer.{}", bearer_session.token).as_str())
        );

        // A stale explicit token gives way to the cookie only from a
        // trusted Origin; the stale subprotocol is still echoed.
        let r = upgrade(vec![
            hdr("Sec-WebSocket-Protocol", "bearer.stale-token"),
            cookie(),
        ]);
        let id = user(&r, true);
        assert_eq!(id.ctx.user_id.as_deref(), Some("user-cookie"));
        assert!(id.cookie_auth && id.explicit_rejected);
        assert_eq!(
            r.credentials.subprotocol.as_deref(),
            Some("bearer.stale-token")
        );
        let id = user(&r, false);
        assert_eq!(id.ctx.user_id, None);
        assert_eq!(id.token.as_deref(), Some("stale-token"));

        // A revoked cookie is no fallback.
        sessions.revoke(&cookie_session.token);
        let id = user(&r, true);
        assert_eq!(id.ctx.user_id, None);

        // Not an upgrade without `Upgrade: websocket`.
        assert!(inspect_ws_upgrade(&[hdr("Sec-WebSocket-Key", "k")], &["app_session"]).is_none());
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
                        sync_omit: false,
                        max_length: None,
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
                        sync_omit: false,
                        max_length: None,
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

    /// Regression: a WS connection must resolve its identity the SAME
    /// way HTTP does, including the caller's role in their active org.
    ///
    /// `resolve_bearer_token` yields user + tenant but an EMPTY `roles`
    /// vec — org membership lives in the app's entities, not the session
    /// store. HTTP and SSR each ran `enrich_active_org_role` afterwards;
    /// the WS handshake did not. Any read policy calling
    /// `auth.hasAnyRole(...)` therefore denied EVERY change broadcast to
    /// that socket, which read as "realtime is dead" while unfiltered
    /// presence frames kept arriving on the same connection.
    #[test]
    fn role_gated_read_needs_the_ws_auth_enricher() {
        use pylon_kernel::{ManifestEntity, ManifestField, ManifestPolicy};

        let field = |name: &str| ManifestField {
            name: name.into(),
            field_type: "string".into(),
            optional: false,
            unique: false,
            crdt: None,
            server_only: false,
            readonly: false,
            default: None,
            enum_values: None,
            encrypted: false,
            sync_omit: false,
            max_length: None,
        };
        let manifest = pylon_kernel::AppManifest {
            manifest_version: pylon_kernel::MANIFEST_VERSION,
            name: "t".into(),
            version: "0".into(),
            entities: vec![ManifestEntity {
                name: "Room".into(),
                fields: vec![field("id"), field("orgId")],
                ..Default::default()
            }],
            // The exact shape a role-gated multi-tenant app uses.
            policies: vec![ManifestPolicy {
                name: "room_read".into(),
                entity: Some("Room".into()),
                allow_read: Some(
                    r#"auth.tenantId == data.orgId && auth.hasAnyRole("owner","admin")"#.into(),
                ),
                ..Default::default()
            }],
            ..Default::default()
        };
        let policy = PolicyEngine::from_manifest(&manifest);
        let row = serde_json::json!({"id": "r1", "orgId": "org1"});

        // What the WS handshake produced before the fix: right user, right
        // tenant, no roles.
        let mut ctx = AuthContext::user("alice".into()).with_tenant("org1".into());
        assert!(
            matches!(
                policy.check_entity_read("Room", &ctx, Some(&row)),
                PolicyResult::Denied { .. }
            ),
            "an un-enriched socket identity must be denied — this is the bug"
        );

        // Applying an enricher (server installs one that reads the org
        // store) completes the identity and the same row now passes.
        let enrich: AuthEnricher = Arc::new(|c: &mut AuthContext| {
            if c.roles.is_empty() {
                c.roles = vec!["owner".to_string()];
            }
        });
        enrich(&mut ctx);
        assert!(
            matches!(
                policy.check_entity_read("Room", &ctx, Some(&row)),
                PolicyResult::Allowed
            ),
            "enriched identity must receive the broadcast"
        );

        // Org switch: the previous org's role must not survive, or a
        // member of org1 keeps owner rights while scoped to org2. The
        // enricher no-ops on a non-empty `roles`, which is why
        // `update_tenant_for_user` clears before re-running it.
        ctx.tenant_id = Some("org2".into());
        ctx.roles.clear();
        let deny_enrich: AuthEnricher = Arc::new(|_c: &mut AuthContext| {});
        deny_enrich(&mut ctx);
        let other_org_row = serde_json::json!({"id": "r2", "orgId": "org2"});
        assert!(
            matches!(
                policy.check_entity_read("Room", &ctx, Some(&other_org_row)),
                PolicyResult::Denied { .. }
            ),
            "stale roles must not carry across an org switch"
        );
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
        subs.subscribe(1, "channel:foo", None);
        subs.subscribe(1, "channel:foo", None);
        assert_eq!(subs.subscribers("channel:foo"), vec![1]);
        assert_eq!(subs.total_subscriptions(), 1);
    }

    #[test]
    fn room_subs_indexes_by_room() {
        // O(subscribers per room) — verify many clients on one room
        // all show up via the room key.
        let subs = RoomSubscriptions::default();
        subs.subscribe(1, "channel:foo", None);
        subs.subscribe(2, "channel:foo", None);
        subs.subscribe(3, "channel:foo", None);
        subs.subscribe(4, "channel:bar", None); // unrelated
        let mut ids = subs.subscribers("channel:foo");
        ids.sort();
        assert_eq!(ids, vec![1, 2, 3]);
        // The unrelated subscription doesn't leak across rooms.
        assert_eq!(subs.subscribers("channel:bar"), vec![4]);
    }

    #[test]
    fn room_subs_unsubscribe_removes_empty_rooms() {
        let subs = RoomSubscriptions::default();
        subs.subscribe(1, "channel:foo", None);
        subs.unsubscribe(1, "channel:foo");
        assert!(subs.subscribers("channel:foo").is_empty());
        // total drops, empty room entry cleaned up — long-running
        // connections that churn through rooms don't leak.
        assert_eq!(subs.total_subscriptions(), 0);
    }

    #[test]
    fn room_subs_release_client_drops_every_room() {
        let subs = RoomSubscriptions::default();
        subs.subscribe(1, "channel:a", Some("alice"));
        subs.subscribe(1, "channel:b", Some("alice"));
        subs.subscribe(2, "channel:a", Some("bob"));
        let mut released = subs.release_client(1);
        released.last_rooms.sort();
        assert_eq!(released.user_id.as_deref(), Some("alice"));
        assert_eq!(released.last_rooms, vec!["channel:a", "channel:b"]);
        // Client 2 must still be present on channel:a.
        assert_eq!(subs.subscribers("channel:a"), vec![2]);
        assert!(subs.subscribers("channel:b").is_empty());
    }

    /// Two connections of one user subscribed to a room: releasing one
    /// does not report the room, releasing the second does. A room only
    /// the released connection held is reported at once.
    #[test]
    fn room_subs_release_client_keeps_rooms_held_by_another_connection() {
        let subs = RoomSubscriptions::default();
        subs.subscribe(1, "channel:shared", Some("alice"));
        subs.subscribe(1, "channel:only-1", Some("alice"));
        subs.subscribe(2, "channel:shared", Some("alice"));
        subs.subscribe(3, "channel:shared", Some("bob"));

        let first = subs.release_client(1);
        assert_eq!(first.user_id.as_deref(), Some("alice"));
        assert_eq!(first.last_rooms, vec!["channel:only-1"]);

        let second = subs.release_client(2);
        assert_eq!(second.last_rooms, vec!["channel:shared"]);

        // Bob's connection is unaffected by alice's.
        assert_eq!(subs.subscribers("channel:shared"), vec![3]);
    }

    #[test]
    fn room_subs_release_client_without_user_reports_no_rooms() {
        let subs = RoomSubscriptions::default();
        subs.subscribe(1, "channel:a", None);
        let released = subs.release_client(1);
        assert_eq!(released, ReleasedRooms::default());
        assert!(subs.subscribers("channel:a").is_empty());
    }

    /// An explicit room-unsubscribe of the connection's last room
    /// forgets its user, so a later release reports nothing stale.
    #[test]
    fn room_subs_unsubscribe_last_room_forgets_the_user() {
        let subs = RoomSubscriptions::default();
        subs.subscribe(1, "channel:a", Some("alice"));
        subs.subscribe(2, "channel:a", Some("alice"));
        subs.unsubscribe(1, "channel:a");
        let released = subs.release_client(2);
        assert_eq!(released.last_rooms, vec!["channel:a"]);
        assert_eq!(subs.release_client(1), ReleasedRooms::default());
    }

    #[test]
    fn room_subs_unsubscribe_unknown_is_noop() {
        let subs = RoomSubscriptions::default();
        assert_eq!(subs.release_client(99), ReleasedRooms::default());
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

    /// FnOps that only answers `get_fn`; the reactive runner isn't started.
    struct GateOnlyFns;

    impl pylon_router::FnOps for GateOnlyFns {
        fn get_fn(&self, name: &str) -> Option<pylon_functions::registry::FnDef> {
            use pylon_functions::protocol::FnType;
            let fn_type = match name {
                "feed" => FnType::Query,
                "deleteAll" => FnType::Mutation,
                _ => return None,
            };
            Some(pylon_functions::registry::FnDef {
                name: name.into(),
                fn_type,
                args_schema: None,
                internal: false,
                auth: pylon_functions::registry::FnAuthMode::Public,
                timeout_secs: None,
            })
        }
        fn list_fns(&self) -> Vec<pylon_functions::registry::FnDef> {
            vec![]
        }
        fn call(
            &self,
            _: &str,
            _: serde_json::Value,
            _: pylon_functions::protocol::AuthInfo,
            _: Option<pylon_functions::runner::StreamCallback>,
            _: Option<pylon_functions::protocol::RequestInfo>,
            _: Option<String>,
        ) -> Result<
            (serde_json::Value, pylon_functions::trace::FnTrace),
            pylon_functions::runner::FnCallError,
        > {
            unreachable!("runner not started")
        }
        fn recent_traces(&self, _: usize) -> Vec<pylon_functions::trace::FnTrace> {
            vec![]
        }
    }

    fn handshake(headers: &[(&str, &str)]) -> Request {
        let mut req = Request::builder().uri("/");
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        req.body(()).unwrap()
    }

    /// Review P2: the dedicated WS listener keyed an anonymous
    /// subscriber's rate limit on the raw socket address (behind a proxy,
    /// every client shared the proxy's) and on the literal "anon" when the
    /// address was unknown. It now resolves the client IP as HTTP does.
    #[test]
    fn the_ws_listener_resolves_the_client_ip_like_http() {
        let proxy = "10.0.0.2".to_string();
        let xff = handshake(&[("X-Forwarded-For", "203.0.113.9, 10.0.0.1")]);
        // No trusted hop: the socket address, whatever the header says.
        assert_eq!(handshake_client_ip(&xff, proxy.clone(), &[], 0), "10.0.0.2");
        // One trusted proxy: the address it saw.
        assert_eq!(handshake_client_ip(&xff, proxy.clone(), &[], 1), "10.0.0.1");
        assert_eq!(
            handshake_client_ip(&xff, proxy.clone(), &[], 2),
            "203.0.113.9"
        );
        // A configured client-IP header wins.
        let tci = handshake(&[("True-Client-IP", "198.51.100.4")]);
        assert_eq!(
            handshake_client_ip(&tci, proxy.clone(), &["true-client-ip".to_string()], 0),
            "198.51.100.4"
        );
        // CF-Connecting-IP counts only from a Cloudflare edge peer.
        let cf = handshake(&[("CF-Connecting-IP", "198.51.100.4")]);
        let chain = ["cf-connecting-ip".to_string()];
        assert_eq!(handshake_client_ip(&cf, proxy, &chain, 0), "10.0.0.2");
        assert_eq!(
            handshake_client_ip(&cf, "162.158.0.9".to_string(), &chain, 0),
            "198.51.100.4"
        );

        assert_eq!(reactive_rate_identity(Some("u1"), Some("1.2.3.4"), 7), "u1");
        assert_eq!(reactive_rate_identity(None, Some("1.2.3.4"), 7), "1.2.3.4");
        assert_eq!(reactive_rate_identity(None, None, 7), "ws-client:7");
        assert_ne!(
            reactive_rate_identity(None, None, 7),
            reactive_rate_identity(None, None, 8)
        );
    }

    #[test]
    fn reactive_subscribe_is_gated_before_registration() {
        let hub = make_test_hub();
        let reg = crate::reactive::ReactiveRegistry::new(Arc::clone(&hub));
        reg.set_fn_ops(Arc::new(GateOnlyFns));
        let anon = pylon_auth::AuthContext::anonymous();
        for name in ["deleteAll", "missing"] {
            handle_reactive_control(
                &reg,
                &hub,
                7,
                &anon,
                "127.0.0.1",
                "reactive-subscribe",
                &serde_json::json!({ "sub_id": name, "fn_name": name }),
            );
        }
        assert_eq!(reg.len(), 0, "refused subscriptions must not register");
        handle_reactive_control(
            &reg,
            &hub,
            7,
            &anon,
            "127.0.0.1",
            "reactive-subscribe",
            &serde_json::json!({ "sub_id": "s1", "fn_name": "feed" }),
        );
        assert_eq!(reg.len(), 1);
    }

    /// Stub RoomBridge for handle_room_control tests.
    ///
    /// Records each (room, user_id) passed to `leave` so the test can
    /// assert which rooms the auto-leave path leaves, and as whom.
    struct StubBridge {
        is_member: bool,
        peers: Vec<serde_json::Value>,
        leave_log: Mutex<Vec<(String, String)>>,
    }

    impl RoomBridge for StubBridge {
        fn members(&self, _room: &str) -> Vec<serde_json::Value> {
            self.peers.clone()
        }
        fn is_in_room(&self, _room: &str, _user_id: &str) -> bool {
            self.is_member
        }
        fn leave(&self, room: &str, user_id: &str) -> bool {
            self.leave_log
                .lock()
                .unwrap()
                .push((room.to_string(), user_id.to_string()));
            self.is_member
        }
    }

    #[test]
    fn room_subscribe_records_membership_when_in_room() {
        // Member of the room → subscribe succeeds, registry records id.
        let hub = make_test_hub();
        let bridge: Arc<dyn RoomBridge> = Arc::new(StubBridge {
            is_member: true,
            peers: vec![],
            leave_log: Mutex::new(Vec::new()),
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
            leave_log: Mutex::new(Vec::new()),
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
            leave_log: Mutex::new(Vec::new()),
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
            leave_log: Mutex::new(Vec::new()),
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
        hub.room_subscriptions.subscribe(42, "channel:foo", None);
        let bridge: Arc<dyn RoomBridge> = Arc::new(StubBridge {
            is_member: true,
            peers: vec![],
            leave_log: Mutex::new(Vec::new()),
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

    fn subscribe_as(
        hub: &Arc<WsHub>,
        bridge: &Arc<dyn RoomBridge>,
        client_id: u64,
        auth_ctx: &pylon_auth::AuthContext,
        room: &str,
    ) {
        handle_room_control(
            hub,
            client_id,
            auth_ctx,
            "room-subscribe",
            &serde_json::json!({ "type": "room-subscribe", "room": room }),
            Some(bridge),
        );
    }

    /// Closing one of a user's two connections leaves only the rooms no
    /// other connection of that user subscribed to. Closing the second
    /// connection leaves the shared room.
    #[test]
    fn end_session_leaves_only_rooms_no_other_connection_holds() {
        let hub = make_test_hub();
        let bridge_struct = Arc::new(StubBridge {
            is_member: true,
            peers: vec![],
            leave_log: Mutex::new(Vec::new()),
        });
        let bridge: Arc<dyn RoomBridge> = bridge_struct.clone();
        let alice = pylon_auth::AuthContext::user("alice".into());
        subscribe_as(&hub, &bridge, 1, &alice, "channel:shared");
        subscribe_as(&hub, &bridge, 1, &alice, "channel:tab-1");
        subscribe_as(&hub, &bridge, 2, &alice, "channel:shared");

        end_session(&hub, 1, None, Some(&bridge));
        assert_eq!(
            *bridge_struct.leave_log.lock().unwrap(),
            vec![("channel:tab-1".to_string(), "alice".to_string())]
        );

        end_session(&hub, 2, None, Some(&bridge));
        assert_eq!(
            bridge_struct.leave_log.lock().unwrap().last(),
            Some(&("channel:shared".to_string(), "alice".to_string()))
        );
        assert_eq!(bridge_struct.leave_log.lock().unwrap().len(), 2);
    }

    #[test]
    fn end_session_without_a_user_leaves_nothing() {
        // An admin context with no user id subscribes; its
        // close must not remove anyone from the room.
        let hub = make_test_hub();
        let bridge_struct = Arc::new(StubBridge {
            is_member: true,
            peers: vec![],
            leave_log: Mutex::new(Vec::new()),
        });
        let bridge: Arc<dyn RoomBridge> = bridge_struct.clone();
        let admin = pylon_auth::AuthContext {
            user_id: None,
            ..pylon_auth::AuthContext::admin()
        };
        subscribe_as(&hub, &bridge, 1, &admin, "channel:a");
        assert_eq!(hub.room_subscriptions().subscribers("channel:a"), vec![1]);
        end_session(&hub, 1, None, Some(&bridge));
        assert!(bridge_struct.leave_log.lock().unwrap().is_empty());
        assert!(hub.room_subscriptions().subscribers("channel:a").is_empty());
    }

    #[test]
    fn end_session_without_subscriptions_leaves_nothing() {
        // A connection that never subscribed to a room leaves nothing,
        // even though its user may be in rooms over HTTP.
        let hub = make_test_hub();
        let bridge_struct = Arc::new(StubBridge {
            is_member: true,
            peers: vec![],
            leave_log: Mutex::new(Vec::new()),
        });
        let bridge: Arc<dyn RoomBridge> = bridge_struct.clone();
        end_session(&hub, 7, None, Some(&bridge));
        assert!(bridge_struct.leave_log.lock().unwrap().is_empty());
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
