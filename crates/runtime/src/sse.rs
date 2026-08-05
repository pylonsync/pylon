use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use pylon_auth::{AuthContext, SessionStore};
use pylon_policy::{PolicyEngine, PolicyResult};
use pylon_sync::{ChangeEvent, ChangeKind};

use crate::ip_limit::{IpConnCounter, IpConnGuard};

const NUM_SHARDS: usize = 16;

/// Per-client state in the shard map.
///
/// `auth` is captured at connection time (parsed from the initial HTTP
/// request's session cookie / bearer token / `?token=` query param)
/// and feeds the per-client tenant filter on every change-event
/// broadcast. Without it, every connected SSE client received every
/// change event from every tenant. Caught in the 2026-05-10 codex
/// pass-3 audit (P0).
///
/// `_guard` is held for the lifetime of the connection — dropping it
/// (when the client is removed) releases the client's slot in the
/// per-IP connection counter. Without this, a crash-loopy browser
/// could open unlimited SSE streams.
struct SseClient {
    stream: TcpStream,
    auth: AuthContext,
    _guard: Option<IpConnGuard>,
}

/// Same rationale as the WS hub: bounded queue + drop-oldest-on-full so a
/// stuck subscriber can't balloon memory on the broadcast path. Clients
/// that miss events catch up via the change-log cursor protocol on
/// reconnect — SSE is a notify-sooner, not a durable-delivery transport.
const BROADCAST_QUEUE_DEPTH: usize = 1024;

/// Default per-client socket write deadline. Bounds how long the shard
/// worker can block on ONE slow consumer's `write_all`/`flush` while
/// holding the shard lock — past this, the write errors, the client is
/// dropped, and the rest of the shard keeps flowing. 5s is generous for
/// a healthy client (a TLS-terminated CDN edge draining a 1KB frame) but
/// short enough that a wedged socket doesn't stall the shard for long.
const DEFAULT_SSE_WRITE_TIMEOUT_MS: u64 = 5_000;

/// Parse the per-client write deadline from the raw env value. `None`
/// (unset / unparseable) → the 5s default. `"0"` → no deadline (blocking
/// writes — opt-out, not recommended). Any positive integer → that many
/// milliseconds. Pure + side-effect-free so the parsing is unit-testable
/// without touching the process environment.
fn parse_sse_write_timeout(raw: Option<&str>) -> Option<Duration> {
    match raw.map(str::trim) {
        Some("0") => None,
        Some(s) => match s.parse::<u64>() {
            Ok(ms) if ms > 0 => Some(Duration::from_millis(ms)),
            _ => Some(Duration::from_millis(DEFAULT_SSE_WRITE_TIMEOUT_MS)),
        },
        None => Some(Duration::from_millis(DEFAULT_SSE_WRITE_TIMEOUT_MS)),
    }
}

/// The configured per-client write deadline, read from
/// `PYLON_SSE_WRITE_TIMEOUT_MS` (see [`parse_sse_write_timeout`]).
fn sse_write_timeout() -> Option<Duration> {
    parse_sse_write_timeout(std::env::var("PYLON_SSE_WRITE_TIMEOUT_MS").ok().as_deref())
}

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

    fn add(&self, id: u64, stream: TcpStream, auth: AuthContext, guard: Option<IpConnGuard>) {
        self.clients.lock().unwrap().insert(
            id,
            SseClient {
                stream,
                auth,
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
    /// Used for non-tenant-scoped messages (e.g. presence relays).
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

    /// Send a change event to clients in this shard whose stored
    /// `AuthContext` passes the entity's read policy. Closes the
    /// cross-tenant data leak the codex pass-3 audit flagged: every
    /// connected SSE client used to receive every change event from
    /// every tenant. Now each client only sees rows it's allowed to
    /// read. Admins bypass the gate so internal tooling stays unblocked.
    fn broadcast_change(
        &self,
        event: &ChangeEvent,
        json: &str,
        synth_delete_sse: Option<&str>,
        policy: &PolicyEngine,
    ) -> Vec<u64> {
        let sse_data = format!("data: {json}\n\n");
        let mut clients = self.clients.lock().unwrap();
        let mut dead = Vec::new();
        for (id, client) in clients.iter_mut() {
            // Match the WS dual-check: if post denies AND a synth-
            // delete payload is available AND pre allows, ship the
            // synthesized Delete instead so the subscriber drops
            // the stale row from their local replica.
            // Unscoped admin (no active tenant) bypasses; admin-with-tenant is
            // scoped like a member via check_entity_read. See is_unscoped_admin.
            let payload: &str = if client.auth.is_unscoped_admin() {
                &sse_data
            } else {
                let post_allowed = matches!(
                    policy.check_entity_read(&event.entity, &client.auth, event.data.as_ref(),),
                    PolicyResult::Allowed
                );
                if post_allowed {
                    &sse_data
                } else if let Some(synth) = synth_delete_sse {
                    let pre_allowed = matches!(
                        policy.check_entity_read(
                            &event.entity,
                            &client.auth,
                            event.prev_data.as_ref(),
                        ),
                        PolicyResult::Allowed
                    );
                    if pre_allowed {
                        synth
                    } else {
                        continue;
                    }
                } else {
                    continue;
                }
            };
            if client.stream.write_all(payload.as_bytes()).is_err()
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

    /// Test-only: read the write deadline on a registered client's stream
    /// so the head-of-line guard's wiring (`add_client` →
    /// `set_write_timeout`) can be asserted end-to-end.
    #[cfg(test)]
    fn client_write_timeout(&self, id: u64) -> Option<Duration> {
        self.clients
            .lock()
            .unwrap()
            .get(&id)
            .and_then(|c| c.stream.write_timeout().ok().flatten())
    }
}

/// What each broadcast shard worker consumes off its mpsc channel.
/// Mirror of `pylon_runtime::ws::BroadcastJob`. `Change` carries the
/// deserialized event so the worker can run the per-client policy
/// check; `Plain` is unfiltered and goes to every client.
pub enum SseJob {
    Change {
        event: Arc<ChangeEvent>,
        json: Arc<str>,
        /// Pre-serialized SSE-framed `data: {...}\n\n` for the
        /// synthesized Delete (visibility-flip tombstone). See
        /// `BroadcastJob::Change::synth_delete_json` in `ws.rs`.
        synth_delete_sse: Option<Arc<str>>,
    },
    Plain(Arc<str>),
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
    broadcast_txs: Vec<mpsc::SyncSender<SseJob>>,
    /// Policy engine for per-client read checks on every change-event
    /// broadcast.
    policy: Arc<PolicyEngine>,
    /// Manifest snapshot for wire projection at the serialization
    /// edge — same rationale as `WsHub::manifest`. Policies that
    /// reference `serverOnly` fields must see the raw row; projection
    /// happens AFTER the per-client policy check.
    manifest: Arc<pylon_kernel::AppManifest>,
    /// Auth-user manifest config for `maybe_project_user_row`.
    auth_user: pylon_kernel::ManifestAuthUserConfig,
}

impl SseHub {
    pub fn new(
        policy: Arc<PolicyEngine>,
        manifest: Arc<pylon_kernel::AppManifest>,
        auth_user: pylon_kernel::ManifestAuthUserConfig,
    ) -> Arc<Self> {
        let mut shards = Vec::with_capacity(NUM_SHARDS);
        let mut broadcast_txs = Vec::with_capacity(NUM_SHARDS);

        for i in 0..NUM_SHARDS {
            let shard = Arc::new(SseShard::new());
            let (tx, rx) = mpsc::sync_channel::<SseJob>(BROADCAST_QUEUE_DEPTH);

            // Broadcast worker: drains the channel and writes to every client
            // in this shard. Runs until the channel is dropped (hub teardown).
            let shard_clone = Arc::clone(&shard);
            let policy_clone = Arc::clone(&policy);
            thread::Builder::new()
                .name(format!("sse-broadcast-{i}"))
                .spawn(move || {
                    while let Ok(job) = rx.recv() {
                        match job {
                            SseJob::Change {
                                event,
                                json,
                                synth_delete_sse,
                            } => {
                                shard_clone.broadcast_change(
                                    &event,
                                    &json,
                                    synth_delete_sse.as_deref(),
                                    &policy_clone,
                                );
                            }
                            SseJob::Plain(msg) => {
                                shard_clone.broadcast(&msg);
                            }
                        }
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
            policy,
            manifest,
            auth_user,
        })
    }

    /// Broadcast a `ChangeEvent` to clients whose stored auth passes
    /// the entity's read policy. Per-client filtering happens in the
    /// shard worker.
    pub fn broadcast(&self, event: &ChangeEvent) {
        // Project NOW for wire serialization (User allowlist +
        // serverOnly strip), AFTER the per-client policy check in
        // the shard worker. The `event` in the SseJob stays raw so
        // policies referencing `serverOnly` fields evaluate
        // against the unprojected row. `prev_data` is stripped
        // from the wire — server-internal only.
        let projected_data = pylon_router::project_row_for_replication_opt(
            self.manifest.as_ref(),
            &self.auth_user,
            &event.entity,
            event.data.clone(),
        );
        let wire_event = ChangeEvent {
            data: projected_data,
            prev_data: None,
            ..event.clone()
        };
        let json = match serde_json::to_string(&wire_event) {
            Ok(j) => j,
            Err(_) => return,
        };
        let json_arc: Arc<str> = Arc::from(json.into_boxed_str());
        // Pre-serialize the visibility-flip tombstone in SSE wire
        // shape. Synth-delete uses the PROJECTED prev_data as its
        // wire `data` so serverOnly fields on the pre-row don't
        // leak via the tombstone path.
        let synth_delete_sse: Option<Arc<str>> =
            if matches!(event.kind, ChangeKind::Update) && event.prev_data.is_some() {
                let projected_prev = pylon_router::project_row_for_replication_opt(
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
                    .map(|s| Arc::from(format!("data: {s}\n\n").into_boxed_str()))
            } else {
                None
            };
        let event_arc: Arc<ChangeEvent> = Arc::new(event.clone());
        for tx in &self.broadcast_txs {
            let _ = tx.try_send(SseJob::Change {
                event: Arc::clone(&event_arc),
                json: Arc::clone(&json_arc),
                synth_delete_sse: synth_delete_sse.clone(),
            });
        }
    }

    /// Broadcast an arbitrary string message to ALL clients, no
    /// filtering. Used for presence/topic relays where the payload
    /// doesn't carry tenant-scoped row data.
    pub fn broadcast_message(&self, msg: &str) {
        let shared: Arc<str> = Arc::from(msg.to_string().into_boxed_str());
        for tx in &self.broadcast_txs {
            let _ = tx.try_send(SseJob::Plain(Arc::clone(&shared)));
        }
    }

    #[allow(dead_code)]
    fn send_to_all(&self, msg: &str) {
        let shared: Arc<str> = Arc::from(msg.to_string().into_boxed_str());
        for tx in &self.broadcast_txs {
            match tx.try_send(SseJob::Plain(Arc::clone(&shared))) {
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
    /// use it after this call. `auth` is captured at registration so the
    /// per-client filter can evaluate against the same identity that
    /// authenticated this connection. The optional `guard` binds the
    /// client's slot in the per-IP connection counter.
    fn add_client(&self, stream: TcpStream, auth: AuthContext, guard: Option<IpConnGuard>) -> u64 {
        // Bound every per-client socket write so one wedged consumer (full
        // kernel send buffer, dead-but-unreset connection) can't block the
        // shard worker — and the shard's `clients` lock — indefinitely.
        // The broadcast/keepalive paths write `write_all` + `flush`
        // SYNCHRONOUSLY while holding that lock; without a deadline a single
        // stuck socket head-of-lines every other subscriber on the shard
        // and backs the broadcast channel up until events drop for the whole
        // shard. On timeout the write errors → the client is marked dead +
        // removed → it recovers via the change-log cursor on reconnect.
        // `set_write_timeout` failing is non-fatal: the client just keeps
        // the OS default (blocking), same as before this guard.
        let _ = stream.set_write_timeout(sse_write_timeout());
        let mut next_id = self.next_id.lock().unwrap();
        let id = *next_id;
        *next_id += 1;
        let shard_idx = (id as usize) % NUM_SHARDS;
        self.shards[shard_idx].add(id, stream, auth, guard);
        id
    }

    /// Total number of connected SSE clients across all shards.
    pub fn client_count(&self) -> usize {
        self.shards.iter().map(|s| s.count()).sum()
    }
}

/// Start the SSE server on the given port.
///
/// Accepts TCP connections, parses the initial HTTP request to extract
/// auth (Authorization header / session cookie / `?token=` query param),
/// rejects unauthenticated callers (in non-dev mode unless
/// `PYLON_SSE_PORT_ACKNOWLEDGE_UNAUTH=1` is set), then registers the
/// stream with the hub. The accept thread exits immediately after
/// registration — no per-client thread is kept alive.
pub fn start_sse_server(hub: Arc<SseHub>, sessions: Arc<SessionStore>, port: u16) {
    // Dual-stack v6+v4 — without it, macOS clients connecting to
    // `localhost:port` over IPv6 (::1) hit connection refused.
    // See crate::bind_dual_stack_tcp.
    let listener = match crate::bind_dual_stack_tcp(port) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("[sse] Failed to bind on port {port}: {e}");
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

    loop {
        // Panic-proof accept: libstd's accept/peer_addr assert (panic) on a
        // truncated macOS dual-stack sockaddr. See crate::accept_tcp.
        let (stream, peer_ip) = match crate::accept_tcp(&listener) {
            Ok(v) => v,
            Err(_) => {
                std::thread::sleep(std::time::Duration::from_millis(1));
                continue;
            }
        };
        // An unparseable peer (the truncation case) buckets under the
        // unspecified address so the per-IP cap still bounds it.
        let ip = peer_ip.unwrap_or(std::net::IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED));
        let guard = match ip_counter.acquire(ip) {
            Some(g) => g,
            None => continue,
        };

        let hub = Arc::clone(&hub);
        let sessions = Arc::clone(&sessions);
        thread::Builder::new()
            .name("sse-accept".into())
            .stack_size(64 * 1024)
            .spawn(move || {
                handle_sse_connection(hub, sessions, stream, guard);
            })
            .ok();
    }
}

/// Parse the initial HTTP request bytes to extract an auth token, in
/// order: `Authorization: Bearer <t>`, session cookie (configurable
/// name, defaulting to `pylon_session`), `?token=<t>` query param.
/// Returns the first match. Browsers using EventSource can't set
/// custom headers, so cookies + query params are the practical paths.
fn extract_sse_token(buf: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(buf).ok()?;
    // Request line: "GET /events?token=... HTTP/1.1\r\n"
    let mut lines = text.split("\r\n");
    let request_line = lines.next()?;
    // Pull token from query string.
    let query_token = request_line
        .split_whitespace()
        .nth(1)
        .and_then(|path| path.split('?').nth(1))
        .and_then(|q| {
            q.split('&').find_map(|kv| {
                let mut parts = kv.splitn(2, '=');
                let key = parts.next()?;
                let value = parts.next()?;
                if key == "token" {
                    Some(value.to_string())
                } else {
                    None
                }
            })
        });

    let cookie_name =
        std::env::var("PYLON_COOKIE_NAME").unwrap_or_else(|_| "pylon_session".to_string());

    let mut bearer: Option<String> = None;
    let mut cookie_value: Option<String> = None;
    for line in lines {
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("authorization:") {
            // Re-read original-cased value for the token portion.
            let original = &line[line.find(':')? + 1..];
            let trimmed = original.trim();
            if let Some(t) = trimmed.strip_prefix("Bearer ") {
                bearer = Some(t.trim().to_string());
            } else if let Some(t) = trimmed.strip_prefix("bearer ") {
                bearer = Some(t.trim().to_string());
            }
            let _ = rest;
        } else if lower.starts_with("cookie:") {
            let original = &line[line.find(':')? + 1..];
            for kv in original.split(';') {
                let kv = kv.trim();
                let mut parts = kv.splitn(2, '=');
                let key = parts.next()?.trim();
                let value = parts.next()?.trim();
                if key == cookie_name {
                    cookie_value = Some(value.to_string());
                    break;
                }
            }
        }
    }
    bearer.or(cookie_value).or(query_token)
}

fn handle_sse_connection(
    hub: Arc<SseHub>,
    sessions: Arc<SessionStore>,
    mut stream: TcpStream,
    guard: IpConnGuard,
) {
    // Consume the HTTP request headers. We use these to extract an
    // auth token before sending the SSE response.
    let mut buf = [0u8; 2048];
    let n = stream.read(&mut buf).unwrap_or(0);
    let token = extract_sse_token(&buf[..n]);

    // Resolve auth. SessionStore::resolve handles None (anonymous),
    // valid bearer/session tokens, and the admin token.
    let auth_ctx = sessions.resolve(token.as_deref());

    // Reject unauthenticated callers in non-dev mode unless the
    // operator explicitly opted in to anonymous SSE via
    // PYLON_SSE_PORT_ACKNOWLEDGE_UNAUTH=1. This closes the unauth
    // half of the codex pass-3 P0 finding: previously any TCP client
    // got every change event from every tenant.
    let in_dev = std::env::var("PYLON_DEV_MODE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let acknowledged_unauth = std::env::var("PYLON_SSE_PORT_ACKNOWLEDGE_UNAUTH")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let is_authed = auth_ctx.user_id.is_some() || auth_ctx.is_admin;
    if !is_authed && !in_dev && !acknowledged_unauth {
        let body = b"{\"error\":{\"code\":\"AUTH_REQUIRED\",\"message\":\"SSE requires authentication; pass session cookie, Authorization: Bearer, or ?token=\"}}";
        let resp = format!(
            "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(resp.as_bytes());
        let _ = stream.write_all(body);
        let _ = stream.flush();
        return;
    }

    stream.set_nodelay(true).ok();

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

    // Hand the stream + auth + IP-conn guard to the hub. The shard's
    // workers own writes; per-client filtering uses the auth captured
    // here on every change-event broadcast.
    hub.add_client(stream, auth_ctx, Some(guard));
}

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_kernel::AppManifest;

    fn empty_policy() -> Arc<PolicyEngine> {
        Arc::new(PolicyEngine::from_manifest(&AppManifest::default()))
    }

    fn empty_hub() -> Arc<SseHub> {
        let manifest = AppManifest::default();
        let auth_user = manifest.auth.user.clone();
        SseHub::new(empty_policy(), Arc::new(manifest), auth_user)
    }

    #[test]
    fn hub_starts_with_correct_shard_count() {
        let hub = empty_hub();
        assert_eq!(hub.shards.len(), NUM_SHARDS);
        assert_eq!(hub.broadcast_txs.len(), NUM_SHARDS);
    }

    #[test]
    fn hub_starts_empty() {
        let hub = empty_hub();
        assert_eq!(hub.client_count(), 0);
    }

    #[test]
    fn broadcast_on_empty_hub_does_not_panic() {
        let hub = empty_hub();
        hub.broadcast_message("hello");
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
        let hub = empty_hub();
        // Verify the ID counter increments correctly.
        let mut next_id = hub.next_id.lock().unwrap();
        assert_eq!(*next_id, 0);
        *next_id = 5;
        drop(next_id);
        // Next add_client would get ID 5, distributing to shard 5 % 16 = 5.
    }

    // P2 head-of-line guard: the per-client write deadline parsing.
    #[test]
    fn sse_write_timeout_parse() {
        // Unset → the 5s default (never blocking-forever by accident).
        assert_eq!(
            parse_sse_write_timeout(None),
            Some(Duration::from_millis(DEFAULT_SSE_WRITE_TIMEOUT_MS))
        );
        // Explicit "0" → opt out (blocking writes).
        assert_eq!(parse_sse_write_timeout(Some("0")), None);
        // Positive integer → that many ms.
        assert_eq!(
            parse_sse_write_timeout(Some("2000")),
            Some(Duration::from_millis(2000))
        );
        // Whitespace tolerated.
        assert_eq!(
            parse_sse_write_timeout(Some("  750 ")),
            Some(Duration::from_millis(750))
        );
        // Garbage → safe default, never an unbounded (None) write.
        assert_eq!(
            parse_sse_write_timeout(Some("banana")),
            Some(Duration::from_millis(DEFAULT_SSE_WRITE_TIMEOUT_MS))
        );
    }

    // P2 head-of-line guard: the deadline must ACTUALLY bound a write to a
    // wedged socket. A real connected pair whose receiver never reads will
    // fill the kernel send buffer; without `set_write_timeout` the writer
    // blocks forever (the head-of-line stall under the shard lock). With
    // the deadline applied, the write returns an error within ~deadline
    // instead of hanging — proving the mechanism the shard relies on
    // works on this platform, not just that the field is set.
    #[test]
    fn write_deadline_bounds_a_wedged_socket() {
        use std::io::Write;
        use std::net::{TcpListener, TcpStream};

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        // Connect FIRST — the OS completes the handshake into the listen
        // backlog, so connect() returns without us having called accept()
        // yet (avoids the accept-before-connect deadlock).
        let mut client = TcpStream::connect(addr).expect("connect");
        // Accept but NEVER read — the receive side stays full so the
        // sender's kernel buffer backs up and stays backed up. Hold the
        // accepted socket for the test body so it isn't dropped/reset.
        let (_hold, _) = listener.accept().expect("accept");
        client
            .set_write_timeout(parse_sse_write_timeout(Some("300")))
            .expect("set deadline");

        // Blast until the buffers fill and the write deadline trips. A
        // healthy bound returns Err(WouldBlock/TimedOut) within a few
        // hundred ms; an unbounded socket would hang here forever.
        let started = std::time::Instant::now();
        let big = vec![0u8; 256 * 1024];
        let mut errored = false;
        for _ in 0..512 {
            if client.write_all(&big).is_err() {
                errored = true;
                break;
            }
            if started.elapsed() > Duration::from_secs(5) {
                break; // safety valve — should never hit with the deadline
            }
        }
        assert!(
            errored,
            "a bounded write to a never-draining socket must error, not block forever"
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the write deadline must trip well before the 5s safety valve"
        );
    }

    // P2 head-of-line guard: registering a client through the hub must
    // leave a write deadline on its stream. This pins the WIRING
    // (`add_client` → `set_write_timeout`), not just the mechanism —
    // deleting that line drops the deadline to None and fails here.
    #[test]
    fn add_client_sets_a_write_deadline() {
        use std::net::{TcpListener, TcpStream};
        let hub = empty_hub();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let _client = TcpStream::connect(addr).expect("connect");
        let (server_stream, _) = listener.accept().expect("accept");

        let id = hub.add_client(server_stream, AuthContext::anonymous(), None);
        let shard = &hub.shards[(id as usize) % NUM_SHARDS];
        assert!(
            shard.client_write_timeout(id).is_some(),
            "a registered SSE client must carry a bounded write deadline so a \
             wedged consumer can't head-of-line the shard"
        );
    }
}
