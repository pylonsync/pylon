use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use pylon_auth::AuthContext;
use pylon_policy::{PolicyEngine, PolicyResult};
use pylon_sync::{ChangeEvent, ChangeKind};

use crate::ip_limit::{IpConnCounter, IpConnGuard};
use crate::request_auth::{AuthResolver, CookieTrust, Credentials, Surface};

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
/// The writer task holds the guard for the lifetime of the connection.
/// When the task ends, it releases the client's slot in the
/// per-IP connection counter. Without this, a crash-loopy browser
/// could open unlimited SSE streams.
struct SseClient {
    tx: tokio::sync::mpsc::Sender<SseFrame>,
    bytes: Arc<tokio::sync::Semaphore>,
    writer: tokio::task::AbortHandle,
    auth: AuthContext,
    #[cfg(test)]
    timeout: Option<Duration>,
}

struct SseFrame {
    data: Arc<str>,
    _bytes: tokio::sync::OwnedSemaphorePermit,
}

impl SseClient {
    fn enqueue(&self, data: &Arc<str>) -> bool {
        // One oversized frame may run alone. All other queued frames count
        // against the byte budget, including the frame being written.
        let weight = data.len().min(CLIENT_QUEUE_BYTES) as u32;
        let Ok(bytes) = Arc::clone(&self.bytes).try_acquire_many_owned(weight) else {
            return false;
        };
        self.tx
            .try_send(SseFrame {
                data: Arc::clone(data),
                _bytes: bytes,
            })
            .is_ok()
    }
}

impl Drop for SseClient {
    fn drop(&mut self) {
        self.writer.abort();
    }
}

/// Full client queues disconnect the client. Its cursor can recover missing
/// events on reconnect. Frame count and retained bytes are both bounded.
const CLIENT_QUEUE_DEPTH: usize = 64;
const CLIENT_QUEUE_BYTES: usize = 1024 * 1024;
const BROADCAST_QUEUE_DEPTH: usize = 1024;

/// Async deadline for one complete frame. A stalled writer cannot hold the
/// shard lock or delay another client's writer.
const DEFAULT_SSE_WRITE_TIMEOUT_MS: u64 = 5_000;

/// Parse the per-client write deadline from the raw env value. `None`
/// (unset / unparseable) → the 5s default. `"0"` → no deadline
/// (the queue limits still apply). Any positive integer → that many
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

    fn add(
        self: &Arc<Self>,
        id: u64,
        stream: TcpStream,
        auth: AuthContext,
        guard: Option<IpConnGuard>,
        timeout: Option<Duration>,
    ) {
        use tokio::io::AsyncWriteExt;
        let Some(runtime) = crate::io_runtime::runtime() else {
            return;
        };
        if stream.set_nonblocking(true).is_err() {
            return;
        }
        let entered = runtime.enter();
        let Ok(mut stream) = tokio::net::TcpStream::from_std(stream) else {
            return;
        };
        drop(entered);
        let (tx, mut rx) = tokio::sync::mpsc::channel::<SseFrame>(CLIENT_QUEUE_DEPTH);
        let shard = Arc::downgrade(self);
        // Hold the map lock until registration completes. A writer that fails
        // immediately must remove the registered entry, not race its insertion.
        let mut clients = self.clients.lock().unwrap();
        let writer = runtime
            .spawn(async move {
                let _guard = guard;
                while let Some(frame) = rx.recv().await {
                    let result = if let Some(deadline) = timeout {
                        match tokio::time::timeout(
                            deadline,
                            stream.write_all(frame.data.as_bytes()),
                        )
                        .await
                        {
                            Ok(result) => result,
                            Err(_) => break,
                        }
                    } else {
                        stream.write_all(frame.data.as_bytes()).await
                    };
                    if result.is_err() {
                        break;
                    }
                }
                if let Some(shard) = shard.upgrade() {
                    shard.remove(id);
                }
            })
            .abort_handle();
        clients.insert(
            id,
            SseClient {
                tx,
                bytes: Arc::new(tokio::sync::Semaphore::new(CLIENT_QUEUE_BYTES)),
                writer,
                auth,
                #[cfg(test)]
                timeout,
            },
        );
    }

    #[allow(dead_code)]
    fn remove(&self, id: u64) {
        self.clients.lock().unwrap().remove(&id);
    }

    /// Send SSE-formatted data to every client in this shard.
    /// Full or closed client queues are removed and their IDs returned.
    /// Used for non-tenant-scoped messages (e.g. presence relays).
    fn broadcast(&self, frame: &Arc<str>) -> Vec<u64> {
        let mut clients = self.clients.lock().unwrap();
        let mut dead = Vec::new();
        for (id, client) in clients.iter_mut() {
            if !client.enqueue(frame) {
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
        frame: &Arc<str>,
        synth_delete_sse: Option<&Arc<str>>,
        policy: &PolicyEngine,
    ) -> Vec<u64> {
        let mut clients = self.clients.lock().unwrap();
        let mut dead = Vec::new();
        for (id, client) in clients.iter_mut() {
            // Match the WS dual-check: if post denies AND a synth-
            // delete payload is available AND pre allows, ship the
            // synthesized Delete instead so the subscriber drops
            // the stale row from their local replica.
            // Unscoped admin (no active tenant) bypasses; admin-with-tenant is
            // scoped like a member via check_entity_read. See is_unscoped_admin.
            let payload: &Arc<str> = if client.auth.is_unscoped_admin() {
                frame
            } else {
                let post_allowed = matches!(
                    policy.check_entity_read(&event.entity, &client.auth, event.data.as_ref(),),
                    PolicyResult::Allowed
                );
                if post_allowed {
                    frame
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
            if !client.enqueue(payload) {
                dead.push(*id);
            }
        }
        for id in &dead {
            clients.remove(id);
        }
        dead
    }

    /// Queue a keepalive comment. Remove clients whose queues are full or closed.
    fn keepalive(&self) {
        let frame: Arc<str> = Arc::from(": keepalive\n\n");
        let mut clients = self.clients.lock().unwrap();
        let mut dead = Vec::new();
        for (id, client) in clients.iter_mut() {
            if !client.enqueue(&frame) {
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

    /// Test-only: inspect the deadline used by the async writer.
    #[cfg(test)]
    fn client_write_timeout(&self, id: u64) -> Option<Duration> {
        self.clients
            .lock()
            .unwrap()
            .get(&id)
            .and_then(|c| c.timeout)
    }
}

/// What each broadcast shard worker consumes off its mpsc channel.
/// Mirror of `pylon_runtime::ws::BroadcastJob`. `Change` carries the
/// deserialized event so the worker can run the per-client policy
/// check; `Plain` is unfiltered and goes to every client.
pub enum SseJob {
    Change {
        event: Arc<ChangeEvent>,
        frame: Arc<str>,
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
/// Socket writers run as async tasks on the shared connection runtime.
/// Client count does not increase the number of writer threads.
pub struct SseHub {
    shards: Vec<Arc<SseShard>>,
    next_id: Mutex<u64>,
    broadcast_txs: Vec<mpsc::SyncSender<SseJob>>,
    /// Policy engine for per-client read checks on every change-event
    /// broadcast.
    policy: Arc<PolicyEngine>,
    /// Manifest snapshot for client wire projection. Workers check the
    /// raw row against each client's policy before sending the projected frame.
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

            // Check policies and queue frames. Socket writers run separately.
            // This worker exits when the hub drops its sender.
            let shard_clone = Arc::clone(&shard);
            let policy_clone = Arc::clone(&policy);
            thread::Builder::new()
                .name(format!("sse-broadcast-{i}"))
                .spawn(move || {
                    while let Ok(job) = rx.recv() {
                        match job {
                            SseJob::Change {
                                event,
                                frame,
                                synth_delete_sse,
                            } => {
                                shard_clone.broadcast_change(
                                    &event,
                                    &frame,
                                    synth_delete_sse.as_ref(),
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
            let shard_ka = Arc::downgrade(&shard);
            thread::Builder::new()
                .name(format!("sse-keepalive-{i}"))
                .spawn(move || loop {
                    thread::sleep(Duration::from_secs(30));
                    let Some(shard) = shard_ka.upgrade() else {
                        break;
                    };
                    shard.keepalive();
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
        // `sync: false` entities never reach a client replica (see
        // `WsHub::broadcast`).
        if !pylon_router::is_replicated_entity(&self.manifest, &event.entity) {
            return;
        }
        // Prepare the projected frame. The worker checks each client's
        // policy before sending it. The `event` in the SseJob stays raw so
        // policies referencing `serverOnly` fields evaluate
        // against the unprojected row. `prev_data` is stripped
        // from the wire — server-internal only.
        let projected_data = pylon_router::project_row_for_replication_opt_ref(
            self.manifest.as_ref(),
            &self.auth_user,
            &event.entity,
            event.data.as_ref(),
        );
        let wire_event = ChangeEvent {
            seq: event.seq,
            entity: event.entity.clone(),
            row_id: event.row_id.clone(),
            kind: event.kind.clone(),
            data: projected_data,
            prev_data: None,
            timestamp: event.timestamp.clone(),
        };
        let json = match serde_json::to_string(&wire_event) {
            Ok(j) => j,
            Err(_) => return,
        };
        let frame: Arc<str> = Arc::from(format!("data: {json}\n\n"));
        // Pre-serialize the visibility-flip tombstone in SSE wire
        // shape. Synth-delete uses the PROJECTED prev_data as its
        // wire `data` so serverOnly fields on the pre-row don't
        // leak via the tombstone path.
        let synth_delete_sse: Option<Arc<str>> =
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
                    .map(|s| Arc::from(format!("data: {s}\n\n").into_boxed_str()))
            } else {
                None
            };
        let event_arc: Arc<ChangeEvent> = Arc::new(event.clone());
        for tx in &self.broadcast_txs {
            let _ = tx.try_send(SseJob::Change {
                event: Arc::clone(&event_arc),
                frame: Arc::clone(&frame),
                synth_delete_sse: synth_delete_sse.clone(),
            });
        }
    }

    /// Broadcast an arbitrary string message to ALL clients, no
    /// filtering. Used for presence/topic relays where the payload
    /// doesn't carry tenant-scoped row data.
    pub fn broadcast_message(&self, msg: &str) {
        let shared: Arc<str> = Arc::from(format!("data: {msg}\n\n"));
        for tx in &self.broadcast_txs {
            let _ = tx.try_send(SseJob::Plain(Arc::clone(&shared)));
        }
    }

    #[allow(dead_code)]
    fn send_to_all(&self, msg: &str) {
        let shared: Arc<str> = Arc::from(format!("data: {msg}\n\n"));
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
        let mut next_id = self.next_id.lock().unwrap();
        let id = *next_id;
        *next_id += 1;
        let shard_idx = (id as usize) % NUM_SHARDS;
        self.shards[shard_idx].add(id, stream, auth, guard, sse_write_timeout());
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
pub fn start_sse_server(
    hub: Arc<SseHub>,
    auth: Arc<AuthResolver>,
    cookie_name: String,
    cookie_trust: CookieTrust,
    port: u16,
    scope: &crate::listen::ListenScope,
) {
    // Dual-stack v6+v4 for `ListenScope::All` — without it, macOS clients
    // connecting to `localhost:port` over IPv6 (::1) hit connection refused.
    // See crate::bind_dual_stack_tcp. `Loopback` binds 127.0.0.1 and ::1.
    let listener = match crate::listen::Listeners::bind(port, scope) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("[sse] Failed to bind on port {port}: {e}");
            return;
        }
    };

    tracing::info!(
        "[sse] SSE server listening on http://localhost:{port}/events (sharded, {NUM_SHARDS} shards)"
    );

    // Per-IP cap mirrors the one on /ws. Idle SSE streams are cheap, but a
    // crash-loopy client can still accumulate thousands of them — this
    // bounds that.
    let ip_counter = Arc::new(IpConnCounter::default());

    loop {
        // Panic-proof accept: libstd's accept/peer_addr assert (panic) on a
        // truncated macOS dual-stack sockaddr. See crate::accept_tcp.
        let (stream, peer_ip) = match listener.accept() {
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
        let auth = Arc::clone(&auth);
        let cookie_name = cookie_name.clone();
        let cookie_trust = Arc::clone(&cookie_trust);
        // Authentication and first-use async runtime setup need the default
        // thread stack. This thread exits after it registers the connection.
        thread::Builder::new()
            .name("sse-accept".into())
            .spawn(move || {
                handle_sse_connection(hub, &auth, &cookie_name, &cookie_trust, stream, guard);
            })
            .ok();
    }
}

/// The parts of an SSE request that auth needs: its headers and the
/// `?token=` query parameter (EventSource can't set headers, so browsers use
/// the cookie or the query). `None` when the bytes are not a request.
struct SseRequest {
    headers: Vec<(String, String)>,
    query_token: Option<String>,
}

fn parse_sse_request(buf: &[u8]) -> Option<SseRequest> {
    let text = std::str::from_utf8(buf).ok()?;
    let mut lines = text.split("\r\n");
    // Request line: "GET /events?token=... HTTP/1.1"
    let request_line = lines.next()?;
    let query_token = request_line
        .split_whitespace()
        .nth(1)
        .and_then(|path| path.split_once('?'))
        .and_then(|(_, q)| {
            q.split('&')
                .find_map(|kv| kv.strip_prefix("token="))
                .map(str::to_string)
        });
    let headers = lines
        .take_while(|line| !line.is_empty())
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_string(), value.trim().to_string()))
        })
        .collect();
    Some(SseRequest {
        headers,
        query_token,
    })
}

fn handle_sse_connection(
    hub: Arc<SseHub>,
    auth: &AuthResolver,
    cookie_name: &str,
    cookie_trust: &CookieTrust,
    mut stream: TcpStream,
    guard: IpConnGuard,
) {
    // Consume the HTTP request headers. We use these to extract the
    // credentials before sending the SSE response.
    let mut buf = [0u8; 2048];
    let n = stream.read(&mut buf).unwrap_or(0);
    let request = parse_sse_request(&buf[..n]).unwrap_or(SseRequest {
        headers: Vec::new(),
        query_token: None,
    });
    let header = |name: &str| {
        request
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    };
    let creds = Credentials::from_headers(
        request
            .headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str())),
        &[cookie_name],
    )
    .or_query_token(request.query_token.clone());

    // The same rules as every transport (see `crate::request_auth`); the
    // session cookie counts only from a trusted Origin.
    let reject = |stream: &mut TcpStream, status: &str, code: &str, message: &str| {
        let body = format!(r#"{{"error":{{"code":"{code}","message":"{message}"}}}}"#);
        let resp = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(resp.as_bytes());
        let _ = stream.flush();
    };
    let identity = match auth.identify(&creds, Surface::App, || {
        cookie_trust(header("Origin"), header("Host"))
    }) {
        Ok(identity) => identity,
        Err(code) => {
            reject(
                &mut stream,
                "401 Unauthorized",
                code,
                "the token is malformed, expired, or revoked",
            );
            return;
        }
    };
    if identity.cookie_refused {
        reject(
            &mut stream,
            "403 Forbidden",
            "SSE_ORIGIN_FORBIDDEN",
            "SSE with cookie auth requires a trusted Origin",
        );
        return;
    }
    let mut auth_ctx = identity.ctx;
    auth.enrich(&mut auth_ctx);

    // Reject unauthenticated callers in non-dev mode unless the
    // operator explicitly opted in to anonymous SSE via
    // PYLON_SSE_PORT_ACKNOWLEDGE_UNAUTH=1. This closes the unauth
    // half of the codex pass-3 P0 finding: previously any TCP client
    // got every change event from every tenant.
    let in_dev = crate::dev_access::dev_mode_enabled();
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

    fn socket_pair() -> (TcpStream, TcpStream) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        (client, listener.accept().unwrap().0)
    }

    fn stalled_socket_pair() -> (TcpStream, TcpStream) {
        let (reader, writer) = socket_pair();
        // Bound both buffers so the test frame cannot fit without a reader.
        // Windows can otherwise accept the whole frame into its send buffer.
        socket2::SockRef::from(&reader)
            .set_recv_buffer_size(1024)
            .unwrap();
        socket2::SockRef::from(&writer)
            .set_send_buffer_size(1024)
            .unwrap();
        (reader, writer)
    }

    fn tenant_policy() -> PolicyEngine {
        PolicyEngine::from_manifest(&AppManifest {
            policies: vec![pylon_kernel::ManifestPolicy {
                name: "tenant_read".into(),
                entity: Some("Doc".into()),
                allow_read: Some("auth.tenantId == data.tenantId".into()),
                ..Default::default()
            }],
            ..Default::default()
        })
    }

    fn tenant_event() -> ChangeEvent {
        ChangeEvent {
            seq: 1,
            entity: "Doc".into(),
            row_id: "row".into(),
            kind: ChangeKind::Update,
            data: Some(serde_json::json!({"tenantId":"b"})),
            prev_data: Some(serde_json::json!({"tenantId":"a"})),
            timestamp: String::new(),
        }
    }

    fn read_frame(stream: &mut TcpStream, expected: &str) {
        let mut bytes = vec![0; expected.len()];
        stream.read_exact(&mut bytes).unwrap();
        assert_eq!(bytes, expected.as_bytes());
    }

    #[test]
    fn hub_shares_framed_payloads_across_all_shards() {
        let manifest = Arc::new(AppManifest {
            entities: vec![pylon_kernel::ManifestEntity {
                name: "Doc".into(),
                fields: vec![pylon_kernel::ManifestField {
                    name: "tenantId".into(),
                    field_type: "string".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        });
        let (senders, receivers): (Vec<_>, Vec<_>) = (0..NUM_SHARDS)
            .map(|_| mpsc::sync_channel(BROADCAST_QUEUE_DEPTH))
            .unzip();
        let hub = SseHub {
            shards: Vec::new(),
            next_id: Mutex::new(0),
            broadcast_txs: senders,
            policy: Arc::new(tenant_policy()),
            auth_user: manifest.auth.user.clone(),
            manifest,
        };
        let event = tenant_event();
        let wire = ChangeEvent {
            prev_data: None,
            ..event.clone()
        };
        let delete = ChangeEvent {
            kind: ChangeKind::Delete,
            data: event.prev_data.clone(),
            prev_data: None,
            ..event.clone()
        };
        let expected = format!("data: {}\n\n", serde_json::to_string(&wire).unwrap());
        let expected_delete = format!("data: {}\n\n", serde_json::to_string(&delete).unwrap());
        hub.broadcast(&event);
        let mut first: Option<(Arc<str>, Arc<str>)> = None;
        for receiver in &receivers {
            let SseJob::Change {
                event: raw,
                frame,
                synth_delete_sse: Some(delete),
            } = receiver.try_recv().unwrap()
            else {
                panic!("missing change frame");
            };
            assert!(raw.prev_data.is_some(), "policy checks retain the old row");
            assert_eq!(frame.as_ref(), expected);
            assert_eq!(delete.as_ref(), expected_delete);
            if let Some((first_frame, first_delete)) = &first {
                assert!(Arc::ptr_eq(first_frame, &frame));
                assert!(Arc::ptr_eq(first_delete, &delete));
            } else {
                first = Some((frame, delete));
            }
        }
        hub.broadcast_message("presence");
        let mut first_plain = None;
        for receiver in &receivers {
            let SseJob::Plain(frame) = receiver.try_recv().unwrap() else {
                panic!("missing plain frame");
            };
            assert_eq!(frame.as_ref(), "data: presence\n\n");
            if let Some(first) = &first_plain {
                assert!(Arc::ptr_eq(first, &frame));
            } else {
                first_plain = Some(frame);
            }
        }
    }

    #[test]
    fn a_blocked_writer_does_not_delay_another_client_or_shard_access() {
        let shard = Arc::new(SseShard::new());
        let (_slow_reader, slow_writer) = stalled_socket_pair();
        let (mut healthy_reader, healthy_writer) = socket_pair();
        let counter = Arc::new(IpConnCounter::new(1));
        let ip = "192.0.2.1".parse().unwrap();
        shard.add(
            1,
            slow_writer,
            AuthContext::user("slow".into()).with_tenant("a".into()),
            counter.acquire(ip),
            None,
        );
        shard.add(
            2,
            healthy_writer,
            AuthContext::user("fast".into()).with_tenant("b".into()),
            None,
            Some(Duration::from_secs(1)),
        );
        let huge: Arc<str> = Arc::from("x".repeat(16 * 1024 * 1024));
        assert!(shard
            .clients
            .lock()
            .unwrap()
            .get(&1)
            .unwrap()
            .enqueue(&huge));
        // No reader drains this frame. The writer still holds its byte permits.
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while shard.clients.lock().unwrap().get(&1).unwrap().tx.capacity() != CLIENT_QUEUE_DEPTH
            && std::time::Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            shard.clients.lock().unwrap().get(&1).unwrap().tx.capacity(),
            CLIENT_QUEUE_DEPTH
        );
        assert_eq!(
            shard
                .clients
                .lock()
                .unwrap()
                .get(&1)
                .unwrap()
                .bytes
                .available_permits(),
            0
        );
        {
            let clients = shard.clients.lock().unwrap();
            let slow = clients.get(&1).unwrap();
            assert!(!slow.enqueue(&Arc::from("another frame")));
            let empty = Arc::from("");
            // The oversized frame is already being written, leaving 64 slots.
            for _ in 0..CLIENT_QUEUE_DEPTH {
                assert!(slow.enqueue(&empty));
            }
            assert!(!slow.enqueue(&empty));
        }
        let start = std::time::Instant::now();
        shard.broadcast_change(
            &tenant_event(),
            &Arc::from("data: healthy\n\n"),
            None,
            &tenant_policy(),
        );
        read_frame(&mut healthy_reader, "data: healthy\n\n");
        assert_eq!(shard.count(), 2, "the blocked writer is still active");
        assert!(start.elapsed() < Duration::from_secs(1));
        // A full byte budget disconnects only the stalled client.
        shard.keepalive();
        read_frame(&mut healthy_reader, ": keepalive\n\n");
        assert_eq!(shard.count(), 1);
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while counter.get(ip) != 0 && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            counter.get(ip),
            0,
            "cancelled writer releases its connection slot"
        );
    }

    #[test]
    fn async_deadline_removes_a_stalled_writer_and_releases_its_slot() {
        let shard = Arc::new(SseShard::new());
        let (_reader, writer) = stalled_socket_pair();
        let counter = Arc::new(IpConnCounter::new(1));
        let ip = "192.0.2.2".parse().unwrap();
        shard.add(
            1,
            writer,
            AuthContext::anonymous(),
            counter.acquire(ip),
            Some(Duration::from_millis(50)),
        );
        let huge: Arc<str> = Arc::from("x".repeat(16 * 1024 * 1024));
        assert!(shard
            .clients
            .lock()
            .unwrap()
            .get(&1)
            .unwrap()
            .enqueue(&huge));
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while counter.get(ip) != 0 && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(shard.count(), 0);
        assert_eq!(counter.get(ip), 0);
    }

    #[test]
    fn queued_changes_preserve_tenant_gates_and_visibility_tombstones() {
        let shard = Arc::new(SseShard::new());
        let (mut old_reader, old_writer) = socket_pair();
        let (mut new_reader, new_writer) = socket_pair();
        let (mut other_reader, other_writer) = socket_pair();
        other_reader
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        for (id, tenant, writer) in [
            (1, "a", old_writer),
            (2, "b", new_writer),
            (3, "c", other_writer),
        ] {
            shard.add(
                id,
                writer,
                AuthContext::user(tenant.into()).with_tenant(tenant.into()),
                None,
                Some(Duration::from_secs(1)),
            );
        }
        shard.broadcast_change(
            &tenant_event(),
            &Arc::from("data: post-row\n\n"),
            Some(&Arc::from("data: deleted\n\n")),
            &tenant_policy(),
        );
        read_frame(&mut old_reader, "data: deleted\n\n");
        read_frame(&mut new_reader, "data: post-row\n\n");
        let mut byte = [0];
        let error = other_reader.read(&mut byte).unwrap_err();
        assert!(matches!(
            error.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
        ));
        shard.keepalive();
        read_frame(&mut old_reader, ": keepalive\n\n");
        read_frame(&mut new_reader, ": keepalive\n\n");
        read_frame(&mut other_reader, ": keepalive\n\n");
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
        let dead = shard.broadcast(&Arc::from("data: test\n\n"));
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

    // Registration must pass the configured deadline to the async writer.
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
            "a registered SSE client must carry a bounded async write deadline"
        );
    }
}
