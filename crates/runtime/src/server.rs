#[allow(unused_imports)]
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use pylon_auth::SessionStore;
use pylon_http::HttpMethod;
use pylon_plugin::PluginRegistry;
use pylon_policy::PolicyEngine;
use pylon_sync::{ChangeKind, ChangeLog};
use tiny_http::{Header, Method, Response, Server};

use crate::datastore::{
    CacheAdapter, EmailAdapter, LocalFileOps, PluginHooksAdapter, PubSubAdapter,
    RuntimeOpenApiGenerator, ShardOpsAdapter, WsSseNotifier,
};
use crate::jobs::{JobQueue, JobResult, Worker};
use crate::metrics::Metrics;
use crate::pubsub::PubSubBroker;
use crate::rate_limit::RateLimiter;
use crate::rooms::RoomManager;
use crate::scheduler::Scheduler;
use crate::sse::SseHub;
use crate::workflows::WorkflowEngine;
use crate::ws::WsHub;
use crate::Runtime;
use pylon_plugin::builtin::ai_proxy::{AiMessage, AiProxyPlugin};
use pylon_plugin::builtin::cache::CachePlugin;

// ---------------------------------------------------------------------------
// DispatchLimiter — bounds concurrent HTTP request handlers.
//
// Pre-commit, every request ran inline in the single dispatch thread. A slow
// fn call (30s DEFAULT_CALL_TIMEOUT) would back the kernel accept queue up
// and starve `/health`, /metrics, every other request. Now each request
// runs on its own worker thread — but unbounded `thread::spawn` is a DoS
// surface (a flood of slow requests = thousands of threads = OOM). This
// limiter caps total in-flight workers + per-IP fairness. Over the cap →
// respond 503 with `Retry-After: 1` and DON'T spawn.
//
// Defaults: global cap = max(32, min(256, available_parallelism * 16))
//           per-IP cap = 64 (matches IpConnCounter's streaming default)
// Both env-overridable for ops.
// ---------------------------------------------------------------------------

struct DispatchLimiter {
    /// Total in-flight worker threads across all clients.
    in_flight: std::sync::atomic::AtomicUsize,
    /// Hard cap on `in_flight`.
    global_cap: usize,
    /// Per-IP cap reuses the streaming limiter's pattern.
    per_ip: Arc<crate::ip_limit::IpConnCounter>,
}

impl DispatchLimiter {
    fn new() -> Arc<Self> {
        let global_cap = std::env::var("PYLON_HTTP_INFLIGHT_MAX")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| {
                let par = std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(4);
                (par * 16).clamp(32, 256)
            });
        let per_ip_cap = std::env::var("PYLON_HTTP_PER_IP_MAX")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(64u32);
        Arc::new(Self {
            in_flight: std::sync::atomic::AtomicUsize::new(0),
            global_cap,
            per_ip: Arc::new(crate::ip_limit::IpConnCounter::new(per_ip_cap)),
        })
    }

    /// Try to acquire one global slot + one per-IP slot. Returns a guard
    /// pair on success (both drop on worker thread completion → both slots
    /// release). Returns None if either cap is hit — caller responds 503.
    fn acquire(
        self: &Arc<Self>,
        ip: std::net::IpAddr,
    ) -> Option<(GlobalSlot, crate::ip_limit::IpConnGuard)> {
        // Optimistic increment then check — race vs check-then-increment
        // would let `cap+1` requests in before any of them sees the cap.
        let prev = self.in_flight.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        if prev >= self.global_cap {
            // Roll back; the slot belongs to nobody.
            self.in_flight.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
            return None;
        }
        let global = GlobalSlot {
            counter: Arc::clone(self),
        };
        let ip_guard = match self.per_ip.acquire(ip) {
            Some(g) => g,
            None => {
                // global drops naturally
                return None;
            }
        };
        Some((global, ip_guard))
    }

    /// Current in-flight count — used in /health/deep + tests.
    #[allow(dead_code)]
    fn in_flight(&self) -> usize {
        self.in_flight.load(std::sync::atomic::Ordering::Acquire)
    }
}

/// RAII guard for the global in-flight slot. Decrements on drop.
struct GlobalSlot {
    counter: Arc<DispatchLimiter>,
}

impl Drop for GlobalSlot {
    fn drop(&mut self) {
        self.counter
            .in_flight
            .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

// ---------------------------------------------------------------------------
// Streaming body — bridges mpsc::Receiver to std::io::Read for SSE responses
// ---------------------------------------------------------------------------

/// A streaming response body backed by an MPSC channel.
///
/// When used as the body of a `tiny_http::Response`, it causes the server to
/// write data as it arrives through the channel. Dropping the sender closes
/// the stream (EOF).
struct StreamingBody {
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
    buf: Vec<u8>,
    pos: usize,
}

impl StreamingBody {
    fn new(rx: std::sync::mpsc::Receiver<Vec<u8>>) -> Self {
        Self {
            rx,
            buf: Vec::new(),
            pos: 0,
        }
    }
}

impl std::io::Read for StreamingBody {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // Drain any leftover data from a previous recv that was larger than
        // the caller's buffer.
        if self.pos < self.buf.len() {
            let remaining = &self.buf[self.pos..];
            let n = remaining.len().min(buf.len());
            buf[..n].copy_from_slice(&remaining[..n]);
            self.pos += n;
            if self.pos >= self.buf.len() {
                self.buf.clear();
                self.pos = 0;
            }
            return Ok(n);
        }

        // Block until the next chunk arrives or the sender is dropped.
        match self.rx.recv() {
            Ok(data) if data.is_empty() => Ok(0),
            Ok(data) => {
                let n = data.len().min(buf.len());
                buf[..n].copy_from_slice(&data[..n]);
                if n < data.len() {
                    self.buf = data;
                    self.pos = n;
                }
                Ok(n)
            }
            Err(_) => Ok(0), // Channel closed = EOF
        }
    }
}

/// Global shutdown flag. Set via `request_shutdown()` to trigger graceful exit.
static SHUTDOWN: AtomicBool = AtomicBool::new(false);

/// Request a graceful shutdown of the running server.
///
/// This sets the shutdown flag and, if a server handle has been stashed, calls
/// `unblock()` to wake the request loop. Safe to call from any thread or signal
/// handler.
pub fn request_shutdown() {
    SHUTDOWN.store(true, Ordering::SeqCst);
    // If a server handle is available, unblock the request loop so it can
    // observe the flag immediately rather than waiting for the next request.
    if let Some(srv) = SERVER_HANDLE.get() {
        srv.unblock();
    }
}

/// Global handle to the `tiny_http::Server` so `request_shutdown()` can call
/// `unblock()` without requiring callers to hold a reference.
static SERVER_HANDLE: std::sync::OnceLock<Arc<Server>> = std::sync::OnceLock::new();

// ---------------------------------------------------------------------------
// Security headers
// ---------------------------------------------------------------------------

/// Resolve the real client IP behind `trust_proxy_hops` reverse
/// proxies. Returns an owned String; empty when no IP can be
/// determined (callers fall back to "anon" identity downstream).
///
/// `trust_proxy_hops == 0` is the safe default: we ignore XFF
/// entirely and use the socket address. Set to N when N trusted
/// proxies sit in front of Pylon — we take the Nth-from-the-right
/// XFF entry, which is the address the closest trusted proxy
/// observed. Honoring the leftmost (or just trusting the whole
/// header) lets any caller spoof their source IP by sending an
/// `X-Forwarded-For: 1.2.3.4` header themselves.
fn resolve_client_ip(request: &tiny_http::Request, trust_proxy_hops: usize) -> String {
    let socket_ip = request
        .remote_addr()
        .map(|a| a.ip().to_string())
        .unwrap_or_default();
    if trust_proxy_hops == 0 {
        return socket_ip;
    }
    // tiny_http stores field names as AsciiStr; cast back to &str so
    // we can do the case-insensitive compare RFC 7230 calls for.
    let xff = request
        .headers()
        .iter()
        .find(|h| {
            h.field
                .as_str()
                .as_str()
                .eq_ignore_ascii_case("X-Forwarded-For")
        })
        .map(|h| h.value.as_str().to_string());
    let Some(xff) = xff else {
        return socket_ip;
    };
    // XFF is "client, proxy1, proxy2" — the leftmost is whatever the
    // first hop SAID was the client (untrusted), and each subsequent
    // entry is what the next hop saw. With N trusted proxies, the
    // Nth-from-right is the IP our closest trusted proxy verified.
    let entries: Vec<&str> = xff.split(',').map(str::trim).collect();
    if entries.len() < trust_proxy_hops {
        // XFF doesn't have enough hops — operator misconfiguration
        // or a request that bypassed the expected proxy chain.
        // Fall back to socket IP rather than trusting whatever's
        // there.
        return socket_ip;
    }
    let candidate = entries[entries.len() - trust_proxy_hops];
    // Validate it parses as an IP before using as a bucket key —
    // garbage-in would let attackers poison the rate-limit map.
    if candidate.parse::<std::net::IpAddr>().is_ok() {
        candidate.to_string()
    } else {
        socket_ip
    }
}

/// Common security headers applied to every response.
///
/// `Referrer-Policy` and `Permissions-Policy` are defense-in-depth.
/// `Strict-Transport-Security` is intentionally NOT set here — Pylon
/// is typically reached through a TLS-terminating proxy (Fly LB,
/// CloudFront) that owns the HSTS decision; setting it from the
/// origin would force every plaintext-loopback test deploy to fight
/// the browser cache.
fn security_headers() -> Vec<Header> {
    vec![
        Header::from_bytes("X-Content-Type-Options", "nosniff").unwrap(),
        Header::from_bytes("X-Frame-Options", "DENY").unwrap(),
        Header::from_bytes("X-XSS-Protection", "1; mode=block").unwrap(),
        // Don't leak the full URL to cross-origin destinations on
        // navigation; same-origin still gets the path so internal
        // analytics keep working.
        Header::from_bytes("Referrer-Policy", "strict-origin-when-cross-origin").unwrap(),
        // Deny every powerful browser API by default. Apps that need
        // camera/mic/geolocation override per-route via their own
        // Permissions-Policy header.
        Header::from_bytes(
            "Permissions-Policy",
            "accelerometer=(), camera=(), geolocation=(), gyroscope=(), microphone=(), payment=(), usb=()",
        )
        .unwrap(),
    ]
}

/// Add security headers to a response.
fn with_security_headers<R: std::io::Read>(response: Response<R>) -> Response<R> {
    let mut resp = response;
    for header in security_headers() {
        resp = resp.with_header(header);
    }
    resp
}

/// Start the dev server on the given port. Blocks until shutdown.
pub fn start(runtime: Arc<Runtime>, port: u16) -> Result<(), String> {
    start_with_plugins(runtime, port, None)
}

/// Start the dev server with optional plugins. Blocks until shutdown.
pub fn start_with_plugins(
    runtime: Arc<Runtime>,
    port: u16,
    plugins: Option<Arc<PluginRegistry>>,
) -> Result<(), String> {
    start_server(runtime, port, plugins, None)
}

/// Start the dev server with plugins and a shard registry for real-time
/// simulations (games, MMO zones, etc.). Blocks until shutdown.
pub fn start_with_shards(
    runtime: Arc<Runtime>,
    port: u16,
    plugins: Option<Arc<PluginRegistry>>,
    shard_registry: Arc<dyn pylon_realtime::DynShardRegistry>,
) -> Result<(), String> {
    start_server(runtime, port, plugins, Some(shard_registry))
}

fn start_server(
    runtime: Arc<Runtime>,
    port: u16,
    plugins: Option<Arc<PluginRegistry>>,
    shard_registry: Option<Arc<dyn pylon_realtime::DynShardRegistry>>,
) -> Result<(), String> {
    // Run the tracing-exporter hook BEFORE anything else emits spans. The
    // operator registers it via `pylon_observability::set_tracing_hook`
    // at process init; here we invoke it exactly once on startup. No-op
    // if nothing was registered.
    pylon_observability::run_tracing_hook();

    // Optional Tinybird request-log shipper. No-op unless the env is
    // set (PYLON_TINYBIRD_TOKEN + PYLON_PROJECT_ID); on Pylon Cloud
    // these are set per-machine at provision time so every customer
    // app ships request rows to the central workspace.
    crate::metrics::init_tinybird_logger();

    // In-process request-log ring buffer. Backs the
    // /admin/logs/tail endpoint, which the Pylon Cloud dashboard
    // polls for live log tail instead of hitting Tinybird on every
    // refresh. Always-on (memory cost is ~200 KB), gated only by
    // the route's admin-auth check.
    crate::log_ring::init_log_ring();

    // Dual-stack bind. `[::]:port` accepts IPv6 AND (on Linux, by
    // default) IPv4-mapped connections to the same socket. Without
    // this, a v4-only `0.0.0.0:port` bind silently breaks Fly.io —
    // their fly-proxy reaches machines via the private IPv6 net,
    // sees no listener, and reports "no known healthy instances".
    // Falls back to v4-only when v6 isn't available (older test
    // environments without IPv6 socket support).
    let addr = format!("[::]:{port}");
    let server = match Server::http(&addr) {
        Ok(s) => s,
        Err(_) => {
            let v4_addr = format!("0.0.0.0:{port}");
            Server::http(&v4_addr).map_err(|e| format!("Failed to start server: {e}"))?
        }
    };
    let server = Arc::new(server);

    // Stash a handle so `request_shutdown()` can unblock the loop.
    let _ = SERVER_HANDLE.set(Arc::clone(&server));

    let session_lifetime = runtime.manifest().auth.session.expires_in;
    // Boot-time warning: at-rest encryption (per-org SSO + SAML
    // secrets, etc.) falls back to a `plain:` envelope when
    // PYLON_SECRET is unset. Loud here so production deployments
    // running with SSO enabled but without the key set don't silently
    // persist OAuth client secrets in plaintext.
    //
    // PYLON_SSO_ENCRYPTION_KEY is the legacy name — still honoured by
    // sso_encryption_key() in org_sso.rs but no longer the
    // recommended env to set.
    let in_prod = std::env::var("PYLON_DEV_MODE")
        .map(|v| v != "1" && !v.eq_ignore_ascii_case("true"))
        .unwrap_or(true);
    // Validate PYLON_SECRET (or its legacy PYLON_SSO_ENCRYPTION_KEY alias)
    // up front. Three outcomes:
    //   * Unset → at-rest encryption is disabled. Loud warning in prod so
    //     deployments running with SSO enabled don't silently persist
    //     OAuth client secrets in plaintext.
    //   * Set but unparseable → fail loud. Previously the resolver returned
    //     `None` for malformed values, silently downgrading to `plain:`
    //     while the operator believed encryption was on. Refuse to start.
    //   * Set and valid → no-op.
    match pylon_auth::org_sso::resolve_sso_encryption_key() {
        Ok(None) => {
            if in_prod {
                tracing::warn!(
                    "[pylon] PYLON_SECRET is unset — at-rest encryption (per-org \
                     SSO + SAML secrets, etc.) is disabled and values are persisted \
                     in plaintext (`plain:` envelope). Set PYLON_SECRET to a \
                     32-byte hex/base64 value (e.g. `openssl rand -hex 32`)."
                );
            }
        }
        Ok(Some(_)) => {}
        Err(msg) => {
            tracing::error!(
                "[pylon] PYLON_SECRET parse failed at startup: {msg}. Refusing to \
                 boot — a misconfigured secret would cause `seal_secret` to silently \
                 emit `plain:` envelopes."
            );
            panic!("PYLON_SECRET is set but invalid: {msg}");
        }
    }
    let auth_stores = build_auth_stores(runtime.db_path().as_deref(), session_lifetime)?;
    let session_store = auth_stores.session_store;
    let magic_codes = auth_stores.magic_codes;
    let oauth_state = auth_stores.oauth_state;
    let account_store = auth_stores.account_store;
    let api_keys = auth_stores.api_keys;
    // OrgStore reads + writes through the manifest's entity layer
    // (Org / OrgMember / OrgInvite by default). The runtime itself is
    // the DataStore — wire it now that we have it.
    let orgs = Arc::new(pylon_auth::org::OrgStore::new(
        runtime.clone() as Arc<dyn pylon_http::DataStore>,
        runtime.manifest().auth.org.clone(),
    ));
    let siwe = auth_stores.siwe;
    let phone_codes = auth_stores.phone_codes;
    let passkeys = auth_stores.passkeys;
    let verification = auth_stores.verification;
    let audit = auth_stores.audit;
    let trusted_devices = auth_stores.trusted_devices;
    let org_sso = auth_stores.org_sso;
    let saml = auth_stores.saml;
    let policy_engine = Arc::new(PolicyEngine::from_manifest(runtime.manifest()));
    // Wire the row-store-backed `PolicyDataResolver` so `exists(...)`
    // predicates in app policies can hit the database. Without this
    // call, any `exists(Entity where ...)` policy expression
    // unconditionally denies — useful as a fail-closed default but
    // useless for the actual use case (membership checks). The
    // resolver wraps the same `Runtime` we're already using as the
    // app-facing `DataStore`.
    policy_engine.set_resolver(Arc::new(crate::datastore::DataStoreResolver::new(
        Arc::clone(&runtime) as Arc<dyn pylon_http::DataStore>,
    )));
    // Postgres backends: wire a global SEQUENCE-backed seq provider so
    // every instance writing to the same database shares one
    // monotonically-increasing seq space. Without this, instance A and
    // instance B can independently mint seq=N for different events;
    // a client connected to one and pulling from the other drops
    // events as "duplicate seq" (codex P1). SQLite / single-instance
    // deployments stay on the in-memory atomic counter.
    let mut change_log_builder = ChangeLog::new();
    if runtime.is_postgres() {
        if let Err(e) = runtime.bootstrap_global_change_seq() {
            tracing::warn!(
                "[change_log] failed to bootstrap PG SEQUENCE; falling back to per-instance seq: {}",
                e.message
            );
        } else {
            let rt_for_provider = Arc::clone(&runtime);
            let initial = runtime.current_global_change_seq().unwrap_or(0);
            let local_fallback = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(initial));
            let local_for_provider = std::sync::Arc::clone(&local_fallback);
            let provider: pylon_sync::SeqProvider = std::sync::Arc::new(move || {
                // Try Postgres first; on transient failure fall back
                // to a local atomic incremented from the last known PG
                // value. Single-instance behavior under outage; the
                // next successful PG call resyncs via the max() guard
                // inside ChangeLog::append.
                rt_for_provider.next_global_change_seq().unwrap_or_else(|| {
                    local_for_provider.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1
                })
            });
            change_log_builder = change_log_builder
                .with_seq(provider)
                .with_initial_seq(initial);
            tracing::info!(
                "[change_log] using PG SEQUENCE pylon_change_seq (initial seq = {initial})"
            );
        }
    }
    let change_log = Arc::new(change_log_builder);

    // Seed the change log with one synthetic insert per extant row so that
    // a pull from seq=0 after a restart reconstructs current state. The
    // change log is in-memory — restarting the process without this would
    // leave SQLite rows unreachable via /api/sync/pull (clients would
    // pull nothing and see an empty replica). Seqs here are fresh; clients
    // whose cursors are ahead of `self.seq` get a 410 and full resync,
    // which then hits this seeded log and gets every current row back.
    // Documented exception to the "all writes go through `apply_mutation`"
    // invariant: this is startup seeding, not a mutation. No clients are
    // connected yet, so there's nothing to broadcast — we're seeding the
    // change log so cursors at last_seq=0 see the current rows when they
    // pull. Policy / plugin hooks / CRDT broadcast don't apply: nothing
    // is changing, this is "the state at boot."
    for entity in runtime.manifest().entities.iter() {
        match runtime.list(&entity.name) {
            Ok(rows) => {
                for row in rows {
                    if let Some(id) = row.get("id").and_then(|v| v.as_str()) {
                        change_log.append(&entity.name, id, ChangeKind::Insert, Some(row.clone()));
                    }
                }
            }
            Err(_) => {
                // Entity table may not exist yet on first boot — skip.
            }
        }
    }
    // Snapshot the manifest once for every component that needs to
    // read it on the broadcast hot path. The hubs use it at the
    // wire-serialization step to project `serverOnly` fields — after
    // the per-client policy check has evaluated the raw row.
    let shared_manifest: Arc<pylon_kernel::AppManifest> = Arc::new(runtime.manifest().clone());
    let ws_hub = WsHub::new(
        Arc::clone(&policy_engine),
        Arc::clone(&shared_manifest),
        runtime.manifest().auth.user.clone(),
    );
    let sse_hub = SseHub::new(
        Arc::clone(&policy_engine),
        Arc::clone(&shared_manifest),
        runtime.manifest().auth.user.clone(),
    );
    // Default-register the rate-limit plugin when no custom registry was
    // supplied. Without this, self-hosted deployments would launch with
    // auth endpoints (/api/auth/magic/send, /api/auth/magic/verify,
    // /api/auth/session) wide open to brute force and enumeration.
    //
    // Tiered limits:
    //   - Authenticated (per-user bucket): 1000/min default. Polling
    //     dashboards + tab fanout + ctx.* lookups easily exceed 100/min
    //     for a single legit user; the old single-cap default locked
    //     real apps out the moment a user opened more than one page of
    //     a richer dashboard. Override via PYLON_RATE_LIMIT_MAX_AUTHED.
    //   - Anonymous (per-IP bucket): 100/min default. Kept tight on
    //     purpose — anon traffic against /api/auth/* is the brute-force
    //     surface and the population that historically warranted a
    //     conservative cap. Override via PYLON_RATE_LIMIT_MAX.
    //   - Dev (PYLON_DEV_MODE truthy): both effectively off (100k/min).
    //
    // Probe dev mode NOW — defined for real at line ~300 but plugin
    // registration below needs it. Same env-var, same logic.
    let is_dev_early = std::env::var("PYLON_DEV_MODE")
        .map(|v| v == "1" || v == "true")
        .unwrap_or(true);
    let plugin_rl_max_authed: u32 = if is_dev_early {
        100_000
    } else {
        std::env::var("PYLON_RATE_LIMIT_MAX_AUTHED")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1_000)
    };
    let plugin_rl_max_anon: u32 = if is_dev_early {
        100_000
    } else {
        std::env::var("PYLON_RATE_LIMIT_MAX")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(100)
    };
    let plugin_reg: Arc<PluginRegistry> = plugins.unwrap_or_else(|| {
        let mut reg = PluginRegistry::new(runtime.manifest().clone());
        reg.register(Arc::new(
            pylon_plugin::builtin::rate_limit::RateLimitPlugin::tiered(
                plugin_rl_max_authed,
                plugin_rl_max_anon,
                std::time::Duration::from_secs(60),
            ),
        ));
        // Auto-scope any entity that declares a `tenantId` field. This is
        // how multi-tenant isolation becomes a default posture rather than
        // an opt-in: drop the field on the entity and the plugin takes it
        // from there (stamps inserts, rejects cross-tenant writes).
        reg.register(Arc::new(
            pylon_plugin::builtin::tenant_scope::TenantScopePlugin::from_manifest(
                runtime.manifest(),
            ),
        ));
        Arc::new(reg)
    });
    let room_mgr = Arc::new(RoomManager::new(120)); // 2 min idle timeout
    let ws_port = port + 1;
    let sse_port = port + 2;

    // Record server start time for the health endpoint.
    let start_time = Instant::now();

    let metrics = Arc::new(Metrics::new());

    // Cache and pub/sub shared instances.
    let cache = Arc::new(CachePlugin::new(100_000));
    let pubsub_broker = Arc::new(PubSubBroker::new(100));

    // Job queue, scheduler, and background workers.
    let job_queue = Arc::new(JobQueue::new(1000));

    // Persistent job store. Colocate with the app DB so `./app.db` gets
    // `./app.db.jobs.db` automatically — otherwise jobs land in CWD, which
    // is wherever the server was launched from (confusing and fragile).
    // In-memory runtimes and the `PYLON_JOBS_IN_MEMORY=1` opt-out both
    // skip persistence.
    let jobs_in_memory = std::env::var("PYLON_JOBS_IN_MEMORY")
        .map(|v| v == "1" || v == "true")
        .unwrap_or(false);
    if !jobs_in_memory {
        let jobs_db_path = std::env::var("PYLON_JOBS_DB").ok().unwrap_or_else(|| {
            runtime
                .db_path()
                .map(|p| format!("{p}.jobs.db"))
                .unwrap_or_else(|| "pylon.jobs.db".into())
        });
        match crate::job_store::JobStore::open(&jobs_db_path) {
            Ok(store) => {
                let store = Arc::new(store);
                let restored = job_queue.restore_from(&store);
                if restored > 0 {
                    tracing::info!("[jobs] Restored {restored} pending job(s) from {jobs_db_path}");
                }
                job_queue.attach_store(store);
            }
            Err(e) => {
                tracing::warn!(
                    "[jobs] Could not open job store at {jobs_db_path}: {e} — running without persistence"
                );
            }
        }
    }

    // Register built-in framework jobs.
    {
        let cache_ref = Arc::clone(&cache);
        job_queue.register(
            "pylon.cache.cleanup",
            Arc::new(move |_job| {
                cache_ref.cleanup_expired();
                JobResult::Success
            }),
        );
        let rooms_ref = Arc::clone(&room_mgr);
        job_queue.register(
            "pylon.rooms.cleanup",
            Arc::new(move |_job| {
                rooms_ref.cleanup_idle();
                JobResult::Success
            }),
        );
    }

    let scheduler = Arc::new(Scheduler::new(Arc::clone(&job_queue)));
    // Schedule built-in tasks.
    let _ = scheduler.schedule(
        "pylon.cache.cleanup",
        "*/10 * * * *",
        Arc::new(|_| JobResult::Success),
    );
    let _ = scheduler.schedule(
        "pylon.rooms.cleanup",
        "*/5 * * * *",
        Arc::new(|_| JobResult::Success),
    );

    // Start 2 background workers.
    let _worker_handles: Vec<_> = (0..2)
        .map(|i| {
            let w = Worker::new(Arc::clone(&job_queue), &format!("worker-{i}"));
            w.start()
        })
        .collect();

    // Start the scheduler.
    let _scheduler_handle = Arc::clone(&scheduler).start();

    // Workflow engine: TS runner URL configurable via env, defaults to local Bun server.
    let wf_runner_url = std::env::var("PYLON_WORKFLOW_RUNNER_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:9876/run".to_string());
    let workflow_engine = Arc::new(WorkflowEngine::new(&wf_runner_url, 10_000));

    // Rate limiter: per-IP outer cap on total requests.
    //
    // Defaults:
    //   - Dev mode: effectively off (100k/min) so a React app's initial
    //     bundle load + sync pulls + user clicks don't immediately 429.
    //     100/min blew through during a single login + first sync pull.
    //   - Prod: 600/min (10 req/sec average). Still tight, but a real app
    //     should override with PYLON_RATE_LIMIT_MAX anyway.
    //
    // Override with PYLON_RATE_LIMIT_MAX + PYLON_RATE_LIMIT_WINDOW.
    let default_rl_max = if is_dev_early { 100_000 } else { 600 };
    let rl_max: u32 = std::env::var("PYLON_RATE_LIMIT_MAX")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default_rl_max);
    let rl_window: u64 = std::env::var("PYLON_RATE_LIMIT_WINDOW")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    let rate_limiter = Arc::new(RateLimiter::new(rl_max, rl_window));

    // Per-function rate limiter: separate bucket per (caller, function) pair.
    // Defaults to a stricter cap because functions are heavier than reads.
    // Override via PYLON_FN_RATE_LIMIT_MAX / PYLON_FN_RATE_LIMIT_WINDOW.
    let fn_rl_max: u32 = std::env::var("PYLON_FN_RATE_LIMIT_MAX")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    let fn_rl_window: u64 = std::env::var("PYLON_FN_RATE_LIMIT_WINDOW")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    let fn_rate_limiter = Arc::new(RateLimiter::new(fn_rl_max, fn_rl_window));

    // /api/ai/stream rate limiter: per-user (or per-IP for unauth — but
    // we now require auth, so effectively per-user). AI endpoints spend
    // real money per call; default 30/hour caps cost at ~$5/user/day on
    // typical pricing. Operators tune via PYLON_AI_RATE_LIMIT_MAX +
    // PYLON_AI_RATE_LIMIT_WINDOW. Caught in the 2026-05-10 codex pass-3
    // audit (P2 NEW: any logged-in user could burn shared spend).
    let ai_rl_max: u32 = std::env::var("PYLON_AI_RATE_LIMIT_MAX")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    let ai_rl_window: u64 = std::env::var("PYLON_AI_RATE_LIMIT_WINDOW")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3600);
    let ai_rate_limiter = Arc::new(RateLimiter::new(ai_rl_max, ai_rl_window));

    // LlmClient built once at boot — env reads + ureq agent
    // construction (with connection pooling) happen here. The route
    // below reuses the same Arc per request. Codex P1-2.
    let llm_client_route: Option<crate::llm::LlmClient> =
        crate::llm::LlmClient::from_env_with_manifest(Some(&runtime.manifest().llm));

    // Cluster bus: enables horizontal scaling. When PYLON_CLUSTER_BUS
    // points at a Redis URL, every change event / presence relay /
    // CRDT frame from this machine gets published; peer pylon
    // machines' subscriber threads receive + re-broadcast to their
    // own WS/SSE clients. Default = Noop (zero-overhead single-
    // machine).
    //
    // Operators on Fly autoscale, K8s replicas, blue/green rollouts —
    // any deploy where >1 pylon process serves the same app — MUST
    // configure this. Without it, a mutation on machine A is
    // invisible to clients connected to machine B, and the client's
    // reconcile() backstop is the only mechanism that eventually
    // closes the gap (on next reconnect / tab-refocus). Live UX
    // requires the bus.
    let cluster_bus: Arc<dyn pylon_cluster::ClusterBus> = build_cluster_bus();
    // Reactive query registry — backs useReactiveQuery hooks. Created
    // before the notifier so the notifier can hold a strong ref and
    // forward every change event into the registry's `on_change`.
    let reactive_registry = crate::reactive::ReactiveRegistry::new(Arc::clone(&ws_hub));
    let fn_notifier: Arc<dyn pylon_router::ChangeNotifier> = Arc::new(
        crate::datastore::WsSseNotifier::with_cluster_bus(
            Arc::clone(&ws_hub),
            Arc::clone(&sse_hub),
            runtime.manifest().auth.user.clone(),
            Arc::clone(&cluster_bus),
        )
        .with_reactive(Arc::clone(&reactive_registry))
        // Per-CRDT-broadcast re-auth needs the policy engine on the
        // notifier so a subscriber whose entity-read permission is
        // revoked mid-session stops receiving frames immediately.
        .with_policy(Arc::clone(&policy_engine)),
    );
    // Subscriber: inbound peer events → local hubs. Idempotent —
    // calling subscribe registers a handler; Noop never delivers, so
    // single-machine builds pay nothing for the call.
    crate::datastore::install_cluster_bus_subscriber(
        &cluster_bus,
        Arc::clone(&ws_hub),
        Arc::clone(&sse_hub),
        Arc::clone(&change_log),
        runtime.manifest().auth.user.clone(),
        Arc::clone(&shared_manifest),
        Some(Arc::clone(&reactive_registry)),
        Some(Arc::clone(&policy_engine)),
    );
    // Single EmailAdapter for the whole runtime: the function runner's
    // ctx.email.send hook + the per-request route handlers below both
    // share it. Constructing per-request was fine when adapters were
    // pure; once we pass it across the FFI boundary into the function
    // runner we want one identity for cleaner logs + future request
    // batching.
    let fn_email_adapter = Arc::new(crate::datastore::EmailAdapter::from_env());
    let fn_ops_maybe = crate::datastore::try_spawn_functions(
        Arc::clone(&runtime),
        Arc::clone(&job_queue),
        Arc::clone(&fn_rate_limiter),
        Arc::clone(&change_log),
        fn_notifier,
        Arc::clone(&fn_email_adapter),
        Arc::clone(&plugin_reg),
        // Wire the caller-aware policy gate. Off-by-default
        // (gated by PYLON_STRICT_FN_POLICIES=1 inside the runner);
        // passing the engine here so the gate is reachable when
        // operators flip the env.
        Arc::clone(&policy_engine),
    );

    // Reactive registry needs FnOps to invoke handlers for initial
    // run + re-run. Wire it now that fn_ops is built, then spawn the
    // re-runner thread. If functions aren't available (no functions/
    // dir, no Bun), reactive subscribes will fail at the FnOps lookup
    // — that's the correct behavior. The registry itself still works
    // for the rest of the codebase that holds an Arc.
    if let Some(ref ops) = fn_ops_maybe {
        let dyn_ops: Arc<dyn pylon_router::FnOps> = Arc::clone(ops) as Arc<dyn pylon_router::FnOps>;
        reactive_registry.set_fn_ops(dyn_ops);
    }
    reactive_registry.start_runner();

    // Dev mode flag. Gates a *lot* of permissive behavior: magic codes
    // appear in JSON responses, /studio is open without admin auth,
    // POST /api/auth/session can mint sessions for arbitrary user_ids,
    // OAuth callback accepts a caller-supplied email, CORS defaults to
    // `*`, etc. Defaulting to `true` meant a prod deploy that simply
    // forgot the env var was trivially compromisable — flip to safe-
    // by-default and let the CLI's `pylon dev` opt in explicitly.
    let is_dev = std::env::var("PYLON_DEV_MODE")
        .map(|v| v == "1" || v == "true")
        .unwrap_or(false);

    // CORS origin. Resolution order:
    //   1. PYLON_CORS_ORIGIN (comma-separated) — operator override.
    //   2. manifest.auth.trustedOrigins — unified declarative source
    //      that also feeds the CSRF + OAuth-redirect gates.
    //   3. Dev-mode default: `*` (loopback is auto-trusted by every
    //      gate anyway via `is_localhost_origin`, but `*` keeps
    //      curl/Postman from custom hosts working).
    //   4. Prod with no manifest entries and no env: hard error.
    //
    // Wildcard + credentials is a spec violation some browsers
    // tolerate; we refuse it in prod because the server also accepts
    // `Authorization: Bearer …` so `*` would let any origin drive
    // bearer-auth APIs.
    let manifest_trusted_origins: Vec<String> = runtime.manifest().auth.trusted_origins.clone();
    let cors_origin_env = match std::env::var("PYLON_CORS_ORIGIN") {
        Ok(v) => Some(v),
        Err(_) => None,
    };
    let cors_allowlist: Vec<String> = if let Some(v) = cors_origin_env.as_deref() {
        v.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    } else if !manifest_trusted_origins.is_empty() {
        manifest_trusted_origins.clone()
    } else if is_dev {
        vec!["*".to_string()]
    } else {
        return Err(
            "CORS gate has no trusted origins. Declare them in your manifest \
             (auth({trustedOrigins: [...]})) or set PYLON_CORS_ORIGIN. \
             PYLON_DEV_MODE=true relaxes this for local development."
                .into(),
        );
    };
    if !is_dev && cors_allowlist.iter().any(|o| o == "*") {
        return Err("CORS gate refuses wildcard `*` in production. \
            Declare explicit origins in manifest.auth.trustedOrigins, or \
            set PYLON_CORS_ORIGIN to a comma-separated list."
            .into());
    }
    if cors_allowlist.is_empty() {
        return Err("CORS gate parsed an empty allowlist from PYLON_CORS_ORIGIN".into());
    }

    // File-storage env validation. Fail fast at boot if PYLON_FILES_PROVIDER
    // names a backend without its required companion vars — see
    // pylon_storage::files::validate_provider_env for the full contract.
    // Catching this here keeps deploys from booting with file uploads
    // that will silently 400 on every request.
    if let Err(msg) = pylon_storage::files::validate_provider_env() {
        return Err(msg);
    }
    // Validate each entry is a valid HTTP header value so per-request
    // header construction never panics on bad bytes.
    for origin in &cors_allowlist {
        if Header::from_bytes("Access-Control-Allow-Origin", origin.as_bytes().to_vec()).is_err() {
            return Err(format!(
                "PYLON_CORS_ORIGIN entry {origin:?} contains bytes that are not a valid HTTP header value"
            ));
        }
    }
    // Browsers forbid combining `Access-Control-Allow-Origin: *` with
    // `Access-Control-Allow-Credentials: true`. Cookie-based auth needs
    // credentials, so we only emit credentials when the allowlist
    // doesn't include `*` (which would force the wildcard-no-credentials
    // path).
    let allow_credentials = !cors_allowlist.iter().any(|o| o == "*");
    let cors_allowlist = Arc::new(cors_allowlist);

    // Admin token: read once at startup, not per-request.
    let admin_token: Option<String> = std::env::var("PYLON_ADMIN_TOKEN").ok();

    // Trusted proxy hops for resolving the real client IP behind a
    // reverse proxy (Fly LB, nginx, CloudFront, etc.). Default 0 =
    // ignore X-Forwarded-For and use the socket peer (safe-by-default;
    // an unconfigured prod deploy can't be tricked into trusting
    // attacker-supplied XFF). Set to N when there are exactly N
    // trusted proxies in front of Pylon — the resolver takes the
    // Nth-from-the-right address in XFF, which is the IP the closest
    // trusted proxy actually saw the request from. Without this, every
    // unauth caller behind the proxy shares one rate-limit bucket.
    let trust_proxy_hops: usize = std::env::var("PYLON_TRUST_PROXY_HOPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    // Session cookie config — built once. Cookie name defaults to
    // `${app_name}_session` so multiple Pylon apps on the same parent
    // domain don't clobber each other's cookies. Browsers receive an
    // HttpOnly+Secure+SameSite=Lax cookie by default; the same opaque
    // session token continues to work via `Authorization: Bearer …`
    // for CLI / mobile / server-to-server callers.
    let cookie_config = Arc::new({
        let app_name = runtime.manifest().name.as_str();
        pylon_auth::CookieConfig::from_env(&pylon_auth::CookieConfig::default_name_for(app_name))
    });

    // CSRF protection. Enforced inline at the HTTP layer because the
    // plugin trait's `on_request` hook doesn't see request headers.
    // For state-changing methods (POST/PATCH/PUT/DELETE) we check
    // Origin, then Referer, against the allowlist.
    //
    // Allowlist resolution (same shape as CORS / OAuth redirect — the
    // three gates share a single declarative source):
    //   1. PYLON_CSRF_ORIGINS (comma-separated) — per-gate override.
    //   2. manifest.auth.trustedOrigins ∪ CORS allowlist (every
    //      origin allowed for fetch is also allowed to drive a
    //      cross-origin POST).
    //   3. Dev: allow-any so localhost tooling on unusual ports
    //      isn't blocked. Loopback is auto-trusted by the plugin
    //      regardless of this list — see CsrfPlugin::is_allowed_origin.
    let csrf_origins: Vec<String> = match std::env::var("PYLON_CSRF_ORIGINS") {
        Ok(v) => v
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        Err(_) => {
            if is_dev {
                vec!["*".to_string()]
            } else {
                let mut merged: Vec<String> = cors_allowlist
                    .iter()
                    .filter(|o| o.as_str() != "*")
                    .cloned()
                    .collect();
                for m in &manifest_trusted_origins {
                    if !m.is_empty() && !merged.contains(m) {
                        merged.push(m.clone());
                    }
                }
                merged
            }
        }
    };
    let csrf = Arc::new(pylon_plugin::builtin::csrf::CsrfPlugin::new(csrf_origins));

    // Trusted origins for OAuth `?callback=` / `?error_callback=`
    // redirect URLs. Required if any OAuth provider is configured —
    // an unconfigured list with a configured provider means every
    // sign-in attempt 403s with UNTRUSTED_REDIRECT, which is
    // operator-visible and recoverable. We don't auto-derive from
    // PYLON_CORS_ORIGIN: the CORS origin is the API caller's origin,
    // which may differ from the dashboard's (e.g. dashboard at
    // /dashboard, API at api.example.com). Better-auth's `trustedOrigins`
    // is the model here — explicit allowlist, no implicit trust.
    // Manifest-declared trusted origins (from auth({trustedOrigins:
    // [...]}) in app.ts) get merged with the env list. Manifest is
    // the type-safe declarative source; env is the operator override
    // for ops-only deploys. Loopback origins are always auto-trusted
    // by `validate_trusted_redirect` regardless of this list, so a
    // fresh `pylon dev` completes OAuth without any config.
    let trusted_origins_env: Vec<String> = std::env::var("PYLON_TRUSTED_ORIGINS")
        .map(|v| {
            v.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let mut combined: Vec<String> = trusted_origins_env;
    for m in &manifest_trusted_origins {
        if !m.is_empty() && !combined.contains(m) {
            combined.push(m.clone());
        }
    }
    let trusted_origins = Arc::new(combined);

    // Start WebSocket server on port+1.
    //
    // The snapshot fetcher gives the WS reader a way to ship the current
    // CRDT snapshot to a client the instant it subscribes — without it
    // the new tab would have to wait for the next write before catching
    // up to the converged state. Encodes into the same length-prefixed
    // wire frame as the broadcast path so the client decoder is shared.
    //
    // Authz is enforced HERE, not at the WS layer: the closure runs the
    // row through `check_entity_read` against the caller's auth ctx and
    // returns None on deny. The caller (handle_crdt_control) treats
    // None as "don't subscribe" — so a denied client can't sit on a
    // subscription waiting for a future write to leak state.
    // Build the CRDT snapshot-fetcher closure once and share it with
    // both WebSocket entry points: the dedicated `:4322` listener AND
    // the HTTP-multiplexed `/api/sync/ws` route on the main port. The
    // closure does the policy + storage round-trip when a client sends
    // `crdt-subscribe` so a denied client can't sit on a subscription
    // waiting for a future write to leak state.
    let snapshot_fetcher: crate::ws::SnapshotFetcher = {
        let runtime_for_fetcher = Arc::clone(&runtime);
        let pe_for_fetcher = Arc::clone(&policy_engine);
        let auth_user_for_fetcher = runtime.manifest().auth.user.clone();
        Arc::new(move |auth_ctx, entity, row_id| {
            use pylon_http::DataStore;
            // P0 leak guard: never ship raw CRDT snapshots for the
            // User entity, even on the initial `crdt-subscribe`
            // bootstrap. The snapshot is a Loro doc carrying every
            // non-id field on the row — `passwordHash`,
            // `_secret`-prefixed columns, anything the JSON broadcast
            // path's User projection strips. `WsSseNotifier::notify_crdt`
            // applies the same guard for live updates; this is the
            // matching guard for the initial subscribe payload. A
            // denied subscribe returns None → the WS handler doesn't
            // register the subscription, so subsequent writes also
            // never leak.
            if entity == auth_user_for_fetcher.entity {
                return None;
            }
            // Fetch the row first so the policy engine can evaluate
            // row-level predicates (`data.authorId == auth.userId`
            // etc). Missing row → deny silently; the client just
            // never gets a frame and can't probe existence.
            let row = match runtime_for_fetcher.get_by_id(entity, row_id) {
                Ok(Some(v)) => v,
                _ => return None,
            };
            if !matches!(
                pe_for_fetcher.check_entity_read(entity, auth_ctx, Some(&row)),
                pylon_policy::PolicyResult::Allowed
            ) {
                return None;
            }
            let snap = match runtime_for_fetcher.crdt_snapshot(entity, row_id) {
                Ok(Some(bytes)) => bytes,
                _ => return None,
            };
            pylon_router::encode_crdt_frame(
                pylon_router::CRDT_FRAME_SNAPSHOT,
                entity,
                row_id,
                &snap,
            )
            .ok()
        })
    };
    // Build a shared WS auth bundle so the WS handshake resolves
    // bearer tokens the same way HTTP does — admin token, API key,
    // JWT, session. Pre-v0.3.72 the WS path only validated session
    // tokens, leaving admin/API-key/JWT bearers silently broken on
    // WS. Caught in the 2026-05-10 codex pass-3 audit (P3).
    let ws_auth = Arc::new(crate::ws::WsAuth {
        sessions: Arc::clone(&session_store),
        api_keys: Arc::clone(&api_keys),
        admin_token: admin_token.clone(),
        jwt_secret: jwt_secret().cloned(),
        jwt_issuer: jwt_issuer().cloned(),
    });
    {
        let hub = Arc::clone(&ws_hub);
        let auth = Arc::clone(&ws_auth);
        let fetcher = snapshot_fetcher.clone();
        let reactive = Arc::clone(&reactive_registry);
        std::thread::spawn(move || {
            crate::ws::start_ws_server(hub, auth, ws_port, Some(fetcher), Some(reactive));
        });
    }

    // Start SSE server on port+2 unless explicitly disabled.
    //
    // SECURITY NOTE: the dedicated SSE port currently has NO
    // authentication and NO per-client tenant filtering — every
    // connected client receives every change event from every
    // tenant, including row data. The 2026-05-10 codex pass-3
    // audit flagged this as P0 for any operator who exposes the
    // port to untrusted networks. Pylon Cloud does NOT expose it
    // (only port+1 for WS is bound externally), so the cloud
    // surface is unaffected. Per-client filter + auth gate is
    // landing in v0.3.72.
    //
    // Stop-gap mitigations shipped today:
    // - PYLON_SSE_PORT_DISABLE=1 — disables the listener entirely.
    //   Use this if you don't need the SSE fallback transport
    //   (Pylon Cloud, deployments behind a private network, etc.).
    // - Loud boot-time warning when the port is bound in
    //   non-dev mode without an explicit "I-accept-the-leak" opt-in.
    let sse_disabled = std::env::var("PYLON_SSE_PORT_DISABLE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if !sse_disabled {
        let in_prod_for_sse = std::env::var("PYLON_DEV_MODE")
            .map(|v| v != "1" && !v.eq_ignore_ascii_case("true"))
            .unwrap_or(true);
        let acknowledged = std::env::var("PYLON_SSE_PORT_ACKNOWLEDGE_UNAUTH")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        if in_prod_for_sse && !acknowledged {
            tracing::warn!(
                "[sse] Dedicated SSE port :{sse_port} is binding WITHOUT authentication or \
                 per-client tenant filtering — every connected client receives every change \
                 event from every tenant. If you do not need this port (most deploys do not), \
                 set PYLON_SSE_PORT_DISABLE=1. If you need it and accept the leak risk, set \
                 PYLON_SSE_PORT_ACKNOWLEDGE_UNAUTH=1 to silence this warning. Per-client \
                 filter + auth gate ships in v0.3.72."
            );
        }
        let hub = Arc::clone(&sse_hub);
        let sessions = Arc::clone(&session_store);
        std::thread::spawn(move || {
            crate::sse::start_sse_server(hub, sessions, sse_port);
        });
    } else {
        tracing::info!("[sse] Dedicated SSE port :{sse_port} disabled by PYLON_SSE_PORT_DISABLE=1");
    }

    // Start shard WebSocket server on port+3 when a registry is provided.
    let shard_ws_port = port + 3;
    if let Some(reg) = shard_registry.clone() {
        let sessions = Arc::clone(&session_store);
        std::thread::spawn(move || {
            crate::shard_ws::start_shard_ws_server(reg, sessions, shard_ws_port);
        });
    }

    tracing::warn!("pylon dev server listening on http://localhost:{port}");
    tracing::info!("  WebSocket: ws://localhost:{ws_port}");
    tracing::info!("  Studio: http://localhost:{port}/studio");
    tracing::info!("  API:    http://localhost:{port}/api/entities/<entity>");
    tracing::info!("  Auth:   http://localhost:{port}/api/auth/session");

    // Per-request bounded concurrency. Each request now runs on its own
    // worker thread (commit follow-up of refactor 326064ff) so a slow fn
    // call can't starve /health, /metrics, or other in-flight requests.
    // The limiter caps total in-flight + per-IP so a flood of slow
    // requests can't OOM the box.
    let dispatch_limiter = DispatchLimiter::new();
    tracing::info!(
        "  HTTP cap: global={} per_ip={}",
        dispatch_limiter.global_cap,
        std::env::var("PYLON_HTTP_PER_IP_MAX")
            .ok()
            .unwrap_or_else(|| "64".to_string())
    );

    // Use recv() in a loop instead of incoming_requests() so we can share
    // the Arc<Server> with the shutdown path (incoming_requests borrows &self
    // which prevents moving the Arc into another thread).
    loop {
        if SHUTDOWN.load(Ordering::Relaxed) {
            break;
        }

        let mut request = match server.recv() {
            Ok(rq) => rq,
            Err(_) => {
                // recv() returns Err when unblocked or the socket is closed.
                break;
            }
        };

        if SHUTDOWN.load(Ordering::Relaxed) {
            break;
        }

        // Acquire a dispatch slot BEFORE spawning the worker. If the global
        // cap or per-IP cap is hit, respond 503 with Retry-After: 1
        // immediately on the dispatch thread — spawning would have
        // produced the same response after thread setup overhead.
        let peer_ip_str = resolve_client_ip(&request, trust_proxy_hops);
        let peer_ip: std::net::IpAddr = peer_ip_str
            .parse()
            .unwrap_or_else(|_| std::net::IpAddr::V4(std::net::Ipv4Addr::new(0, 0, 0, 0)));
        let limiter_guards = match dispatch_limiter.acquire(peer_ip) {
            Some(g) => g,
            None => {
                let body = json_error(
                    "OVERLOADED",
                    "Server is at its per-IP or global concurrency cap. Retry shortly.",
                );
                let response = with_security_headers(
                    Response::from_string(&body)
                        .with_status_code(503u16)
                        .with_header(
                            Header::from_bytes("Content-Type", "application/json").unwrap(),
                        )
                        .with_header(Header::from_bytes("Retry-After", "1").unwrap()),
                );
                let method_str = request.method().as_str().to_string();
                let _ = request.respond(response);
                metrics.record_request(&method_str, 503);
                continue;
            }
        };

        // Shadow-rebind every outer Arc<T>/Vec/etc. used by the request
        // handler so the `move ||` closure below captures the local
        // shadows (one fresh Arc::clone per iteration) rather than the
        // outer originals (which the next loop iteration still needs).
        // The existing per-iteration aliases (rt, ss, pe …) stay so the
        // body's existing references compile unchanged.
        let runtime = Arc::clone(&runtime);
        let session_store = Arc::clone(&session_store);
        let policy_engine = Arc::clone(&policy_engine);
        let change_log = Arc::clone(&change_log);
        let ws_hub = Arc::clone(&ws_hub);
        let sse_hub = Arc::clone(&sse_hub);
        let cluster_bus = Arc::clone(&cluster_bus);
        let reactive_registry = Arc::clone(&reactive_registry);
        let shared_manifest = Arc::clone(&shared_manifest);
        let magic_codes = Arc::clone(&magic_codes);
        let plugin_reg = Arc::clone(&plugin_reg);
        let room_mgr = Arc::clone(&room_mgr);
        let metrics = Arc::clone(&metrics);
        let oauth_state = Arc::clone(&oauth_state);
        let account_store = Arc::clone(&account_store);
        let api_keys = Arc::clone(&api_keys);
        let orgs = Arc::clone(&orgs);
        let siwe = Arc::clone(&siwe);
        let phone_codes = Arc::clone(&phone_codes);
        let passkeys = Arc::clone(&passkeys);
        let verification = Arc::clone(&verification);
        let audit = Arc::clone(&audit);
        let trusted_devices = Arc::clone(&trusted_devices);
        let org_sso = Arc::clone(&org_sso);
        let saml = Arc::clone(&saml);
        let trusted_origins = Arc::clone(&trusted_origins);
        let cache = Arc::clone(&cache);
        let pubsub_broker = Arc::clone(&pubsub_broker);
        let job_queue = Arc::clone(&job_queue);
        let scheduler = Arc::clone(&scheduler);
        let workflow_engine = Arc::clone(&workflow_engine);
        let fn_ops_maybe = fn_ops_maybe.clone();
        let shard_registry = shard_registry.clone();
        let cors_allowlist = cors_allowlist.clone();
        let cookie_config = Arc::clone(&cookie_config);
        let ws_auth = Arc::clone(&ws_auth);
        let snapshot_fetcher = snapshot_fetcher.clone();
        let admin_token = admin_token.clone();
        let rate_limiter = Arc::clone(&rate_limiter);
        let csrf = Arc::clone(&csrf);
        let ai_rate_limiter = Arc::clone(&ai_rate_limiter);
        let llm_client_route = llm_client_route.clone();

        // Per-request handler thread. Each request runs on its own worker;
        // a slow fn call here can't block /health or other requests.
        // `limiter_guards` is moved into the worker so the global +
        // per-IP slots stay reserved for the request's lifetime and
        // release when the worker thread exits.
        let _ = std::thread::Builder::new()
            .name("pylon-http-worker".into())
            .stack_size(512 * 1024)
            .spawn(move || {
                // Hold the slots for the worker's lifetime.
                let _limiter_guards = limiter_guards;

        let rt = Arc::clone(&runtime);
        let ss = Arc::clone(&session_store);
        let pe = Arc::clone(&policy_engine);
        let cl = Arc::clone(&change_log);
        let wh = Arc::clone(&ws_hub);
        let sh = Arc::clone(&sse_hub);
        let cb = Arc::clone(&cluster_bus);
        let rr = Arc::clone(&reactive_registry);
        let sm = Arc::clone(&shared_manifest);
        let mc = Arc::clone(&magic_codes);
        let pr = Arc::clone(&plugin_reg);
        let rm = Arc::clone(&room_mgr);
        let mt = Arc::clone(&metrics);
        let os = Arc::clone(&oauth_state);
        let acc = Arc::clone(&account_store);
        let ak = Arc::clone(&api_keys);
        let og = Arc::clone(&orgs);
        let sw = Arc::clone(&siwe);
        let pcd = Arc::clone(&phone_codes);
        let pks = Arc::clone(&passkeys);
        let vrf = Arc::clone(&verification);
        let aud = Arc::clone(&audit);
        let td = Arc::clone(&trusted_devices);
        let osso = Arc::clone(&org_sso);
        let sml = Arc::clone(&saml);
        let trusted_origins_ref = Arc::clone(&trusted_origins);
        let ca = Arc::clone(&cache);
        let ps = Arc::clone(&pubsub_broker);
        let jq = Arc::clone(&job_queue);
        let sc = Arc::clone(&scheduler);
        let we = Arc::clone(&workflow_engine);
        let fn_ops_ref = fn_ops_maybe.clone();
        let shards_ref = shard_registry.clone();
        // Compute the per-request CORS origin to echo back. Match the
        // request's Origin header against the allowlist; loopback is
        // always trusted via `is_localhost_origin` so dev tools on
        // unusual ports work without manifest config. On miss we
        // still emit a header (the first allowlist entry) so the
        // browser surfaces the mismatch in DevTools — and we log
        // `CORS_ORIGIN_NOT_ALLOWED` with the gate name + remediation
        // so the operator doesn't have to guess which of three
        // gates rejected the request.
        let req_origin_header = request
            .headers()
            .iter()
            .find(|h| h.field.equiv("Origin"))
            .map(|h| h.value.as_str().to_string());
        let cors_origin: String = {
            // Always reflect the request's Origin when it matches the
            // allowlist OR is a localhost origin — even in dev mode
            // where the allowlist contains "*". Browsers refuse to
            // combine `Access-Control-Allow-Origin: *` with
            // `Access-Control-Allow-Credentials: true`, so the literal
            // "*" path silently breaks every credentialed request
            // (db.useQuery with cookie-auth, /api/auth/me on prod-style
            // setups, every request the SyncEngine makes). Reflecting
            // the origin keeps credentials working and stays equally
            // permissive — `is_localhost_origin` already covers every
            // dev origin we care about.
            let wildcard = cors_allowlist.iter().any(|o| o == "*");
            match &req_origin_header {
                Some(o)
                    if pylon_auth::is_localhost_origin(o)
                        || cors_allowlist.iter().any(|a| a == o) =>
                {
                    o.clone()
                }
                Some(o) if wildcard => o.clone(),
                Some(o) => {
                    tracing::warn!(
                        "[cors] gate rejected origin {o:?} — add to \
                         manifest.auth.trustedOrigins or PYLON_CORS_ORIGIN"
                    );
                    cors_allowlist
                        .first()
                        .cloned()
                        .unwrap_or_else(|| "null".to_string())
                }
                None if wildcard => "*".to_string(),
                None => cors_allowlist
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "null".to_string()),
            }
        };
        let cookie_config = Arc::clone(&cookie_config);
        // Credentials can ride only when the response Origin is a
        // concrete value, never literal "*". The boot-time
        // `allow_credentials` flag captured "no `*` in the static
        // allowlist"; with the dev-mode origin reflection above we also
        // need to allow credentials whenever the per-request resolved
        // `cors_origin` isn't `*`.
        let allow_credentials = allow_credentials || cors_origin != "*";
        let is_dev = is_dev;

        let method = request.method().clone();
        let url = request.url().to_string();

        // Per-request access log — visibility into what's hitting the
        // server, mirroring Next.js's `GET /login 200 in 27ms` style.
        // Suppress for noisy paths (/health, /metrics, the log-tail
        // endpoint) so dev-mode logs don't drown in proxy/scrape/
        // dashboard-poll traffic. The log-tail endpoint specifically
        // would echo itself into the very ring it serves, doubling
        // every entry. Status + duration get logged separately by
        // `metrics.record_request` so we don't need to thread them
        // through every response branch.
        //
        // Per-request peer IP keeps it useful for debugging
        // multi-origin setups (CSRF rejections, rate-limit hits) —
        // resolve_client_ip already honors PYLON_TRUST_PROXY_HOPS so
        // this matches what the rest of the server sees.
        let request_peer_ip = resolve_client_ip(&request, trust_proxy_hops);
        let request_started_at = std::time::Instant::now();
        let is_noisy = url == "/health"
            || url == "/health/deep"
            || url == "/metrics"
            || url.starts_with("/admin/logs/tail");
        if !is_noisy {
            tracing::info!("→ {} {} from {}", method.as_str(), url, request_peer_ip);
            // Stash for the response log (`record_request` reads this
            // thread-local to emit method/url/status/duration in one
            // line, like Next.js's `GET /login 200 in 27ms`).
            crate::metrics::set_current_request(&url, request_started_at);
        }

        // --- WebSocket multiplex on the main HTTP port ---
        //
        // Reverse proxies that pass `Upgrade: websocket` through (Cloudflare,
        // Caddy, recent Vercel rewrites) can't reach `:4322` — they only
        // see the public 443. Accepting `wss://<host>/api/sync/ws` here
        // means clients don't need a separate WS host config: the same
        // origin serves HTTP + WS, the framework's deriveWsUrl in
        // @pylonsync/sync defaults to this path, and the dedicated `:4322`
        // listener stays as a fallback for raw-TCP deployments.
        //
        // Path is /api/sync/ws so it sits under existing `/api/*` rewrite
        // rules without forcing operators to add a new proxy entry.
        if url == "/api/sync/ws" && method == Method::Get {
            if let Some(upgrade_req) =
                crate::ws::inspect_ws_upgrade(request.headers(), &cookie_config.name)
            {
                let hub = Arc::clone(&ws_hub);
                let auth = Arc::clone(&ws_auth);
                let fetcher = snapshot_fetcher.clone();
                let reactive = Arc::clone(&reactive_registry);
                std::thread::Builder::new()
                    .name("ws-upgrade".into())
                    .stack_size(64 * 1024)
                    .spawn(move || {
                        crate::ws::handle_http_upgrade(
                            request,
                            upgrade_req,
                            hub,
                            auth,
                            Some(fetcher),
                            Some(reactive),
                        );
                    })
                    .ok();
                mt.record_request("GET", 101);
                return;
            }
            // Missing Upgrade headers — fall through to a plain 400.
            let response = with_security_headers(
                Response::from_string(json_error(
                    "BAD_UPGRADE",
                    "Sec-WebSocket-Key + Upgrade headers required",
                ))
                .with_status_code(400u16)
                .with_header(Header::from_bytes("Content-Type", "application/json").unwrap()),
            );
            let _ = request.respond(response);
            mt.record_request("GET", 400);
            return;
        }

        // --- Deep health: shallow /health + a 500ms responsive-
        // ness probe on the bun function runtime. Distinct route so
        // operators can choose: /health for "is the listener up?"
        // (Fly's default — fast, never blocks on app state), or
        // /health/deep for "is the runtime responsive enough to
        // serve real requests?". Today's incident showed why both
        // matter: /health stayed green while every function call
        // got killed at the 30s timeout, taking 150 functions
        // offline during respawn — flipping Fly's probe to
        // /health/deep makes the proxy see the thrash and route
        // around the machine until it recovers.
        if url == "/health/deep" && method == Method::Get {
            let uptime = start_time.elapsed().as_secs();
            let probe = match fn_ops_ref.as_deref() {
                Some(ops) => ops.pool.health_probe(std::time::Duration::from_millis(500)),
                None => Ok(()), // no runtime configured = nothing to probe
            };
            let (status, runtime_status, reason) = match probe {
                Ok(()) => (200u16, "ok", String::new()),
                Err(msg) => (503u16, "degraded", msg),
            };
            let body = serde_json::json!({
                "status": runtime_status,
                "version": "0.1.0",
                "uptime_secs": uptime,
                "runtime": {
                    "alive": runtime_status == "ok",
                    "reason": reason,
                },
            })
            .to_string();
            let response = with_security_headers(
                Response::from_string(&body)
                    .with_status_code(status)
                    .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
                    .with_header(
                        Header::from_bytes(
                            "Access-Control-Allow-Origin",
                            cors_origin.as_bytes().to_vec(),
                        )
                        .unwrap(),
                    ),
            );
            let _ = request.respond(response);
            mt.record_request("GET", status);
            return;
        }

        // --- Health check: fast path before auth or body parsing ---
        if url == "/health" && method == Method::Get {
            let uptime = start_time.elapsed().as_secs();
            let body = serde_json::json!({
                "status": "ok",
                "version": "0.1.0",
                "uptime_secs": uptime,
            })
            .to_string();

            let response = with_security_headers(
                Response::from_string(&body)
                    .with_status_code(200u16)
                    .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
                    .with_header(
                        Header::from_bytes(
                            "Access-Control-Allow-Origin",
                            cors_origin.as_bytes().to_vec(),
                        )
                        .unwrap(),
                    ),
            );
            let _ = request.respond(response);
            return;
        }

        // --- Metrics endpoint: fast path before rate-limit / body parsing.
        // Gate behind admin auth in non-dev to prevent leakage of function
        // names, request volumes, and error rates to the public internet.
        // Dev mode stays open so local Prometheus scrapers just work.
        if url == "/metrics" && method == Method::Get {
            // /metrics accepts THREE auth paths (see
            // `verify_admin_or_metrics_auth`):
            //   1. PYLON_ADMIN_TOKEN bearer  — operator / Prometheus
            //   2. PYLON_METRICS_TOKEN bearer — Pylon Cloud's
            //      per-project read-only path
            //   3. Session cookie → admin user — lets Studio admins
            //      poll /metrics from the dashboard without holding
            //      the bare admin token.
            // Dev-mode keeps the endpoint open so local Prometheus
            // scrapers just work.
            if !is_dev
                && !verify_admin_or_metrics_auth(
                    &request,
                    admin_token.as_deref(),
                    &cookie_config,
                    &session_store,
                    runtime.as_ref(),
                )
            {
                let body = json_error(
                    "UNAUTHORIZED",
                    "/metrics requires PYLON_ADMIN_TOKEN or PYLON_METRICS_TOKEN bearer in non-dev mode",
                );
                let response = with_security_headers(
                    Response::from_string(&body)
                        .with_status_code(401u16)
                        .with_header(
                            Header::from_bytes("Content-Type", "application/json").unwrap(),
                        ),
                );
                let _ = request.respond(response);
                return;
            }
            let prefers_prometheus = request.headers().iter().any(|h| {
                (h.field.as_str() == "Accept" || h.field.as_str() == "accept")
                    && (h.value.as_str().contains("text/plain")
                        || h.value.as_str().contains("application/openmetrics-text"))
            });
            let (body, content_type) = if prefers_prometheus {
                (mt.prometheus(), "text/plain; version=0.0.4")
            } else {
                // Augment the bare HTTP snapshot with live operational
                // stats so the Studio Overview can render jobs/workflows/
                // ws/sse without N more round-trips. Each accessor is a
                // cheap atomic load or single mutex snapshot — fine to
                // call on every /metrics fetch.
                let mut snap = mt.snapshot();
                if let Some(obj) = snap.as_object_mut() {
                    let job_stats = job_queue.stats();
                    obj.insert(
                        "jobs".to_string(),
                        serde_json::json!({
                            "pending": job_stats.pending,
                            "running": job_stats.running,
                            "completed": job_stats.completed,
                            "failed": job_stats.failed,
                            "dead": job_stats.dead,
                            "handlers": job_stats.handlers,
                        }),
                    );
                    // Bucket workflow instances by their status variant
                    // for the dashboard. One pass over the in-memory list;
                    // workflows are bounded by max_history so this is
                    // O(few thousand) at worst.
                    let mut wf_pending = 0usize;
                    let mut wf_running = 0usize;
                    let mut wf_waiting = 0usize;
                    let mut wf_sleeping = 0usize;
                    let mut wf_completed = 0usize;
                    let mut wf_failed = 0usize;
                    let mut wf_cancelled = 0usize;
                    for inst in workflow_engine.list(None) {
                        match inst.status {
                            crate::workflows::WorkflowStatus::Pending => wf_pending += 1,
                            crate::workflows::WorkflowStatus::Running => wf_running += 1,
                            crate::workflows::WorkflowStatus::WaitingForEvent => wf_waiting += 1,
                            crate::workflows::WorkflowStatus::Sleeping => wf_sleeping += 1,
                            crate::workflows::WorkflowStatus::Completed => wf_completed += 1,
                            crate::workflows::WorkflowStatus::Failed => wf_failed += 1,
                            crate::workflows::WorkflowStatus::Cancelled => wf_cancelled += 1,
                        }
                    }
                    obj.insert(
                        "workflows".to_string(),
                        serde_json::json!({
                            "pending": wf_pending,
                            "running": wf_running,
                            "waiting": wf_waiting,
                            "sleeping": wf_sleeping,
                            "completed": wf_completed,
                            "failed": wf_failed,
                            "cancelled": wf_cancelled,
                        }),
                    );
                    obj.insert(
                        "realtime".to_string(),
                        serde_json::json!({
                            "ws_connections": ws_hub.client_count(),
                            "sse_connections": sse_hub.client_count(),
                        }),
                    );
                }
                (snap.to_string(), "application/json")
            };
            let response = with_security_headers(
                Response::from_string(&body)
                    .with_status_code(200u16)
                    .with_header(Header::from_bytes("Content-Type", content_type).unwrap())
                    .with_header(
                        Header::from_bytes(
                            "Access-Control-Allow-Origin",
                            cors_origin.as_bytes().to_vec(),
                        )
                        .unwrap(),
                    ),
            );
            let _ = request.respond(response);
            mt.record_request("GET", 200);
            return;
        }

        // --- /admin/logs/tail: live request-log tail backed by the
        // in-process ring buffer (see `log_ring.rs`). Same auth as
        // /metrics: admin/metrics bearer or cookie-admin. Returns
        // entries strictly newer than `?since=<iso-8601>` (or all
        // available when omitted), newest-first.
        //
        // Replaces the Pylon Cloud dashboard's pre-existing pattern of
        // hitting Tinybird on every 2s refresh — that pattern burns
        // ~64k Tinybird queries/day per actively-tailed project. The
        // ring serves the same shape directly from process memory.
        if url.starts_with("/admin/logs/tail") && method == Method::Get {
            if !is_dev
                && !verify_admin_or_metrics_auth(
                    &request,
                    admin_token.as_deref(),
                    &cookie_config,
                    &session_store,
                    runtime.as_ref(),
                )
            {
                let body = json_error(
                    "UNAUTHORIZED",
                    "/admin/logs/tail requires PYLON_ADMIN_TOKEN or PYLON_METRICS_TOKEN bearer in non-dev mode",
                );
                let response = with_security_headers(
                    Response::from_string(&body)
                        .with_status_code(401u16)
                        .with_header(
                            Header::from_bytes("Content-Type", "application/json").unwrap(),
                        ),
                );
                let _ = request.respond(response);
                mt.record_request("GET", 401);
                return;
            }
            // Parse `since=<iso-8601>` from the query string. Anything
            // else in the query string is ignored — `since` is the
            // only knob the dashboard sends and treating unknown
            // params as errors would break casual curl debugging.
            let since: Option<String> = url.split_once('?').map(|(_, q)| q).and_then(|q| {
                q.split('&').find_map(|kv| {
                    let (k, v) = kv.split_once('=')?;
                    if k == "since" {
                        Some(percent_decode_str(v))
                    } else {
                        None
                    }
                })
            });
            let entries = crate::log_ring::log_ring()
                .map(|r| r.tail_since(since.as_deref()))
                .unwrap_or_default();
            // Cursor = the newest timestamp we just shipped, or echo
            // the caller's `since` back when the tail was empty so a
            // subsequent poll doesn't rewind. Mirrors the Tinybird-
            // backed listProjectLogs response shape so the dashboard
            // cursor logic is backend-agnostic.
            let cursor: serde_json::Value = entries
                .first()
                .map(|e| serde_json::Value::String(e.timestamp.clone()))
                .or_else(|| since.clone().map(serde_json::Value::String))
                .unwrap_or(serde_json::Value::Null);
            let body = serde_json::json!({
                "rows": entries,
                "cursor": cursor,
                "configured": true,
            })
            .to_string();
            let response = with_security_headers(
                Response::from_string(&body)
                    .with_status_code(200u16)
                    .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
                    .with_header(
                        Header::from_bytes(
                            "Access-Control-Allow-Origin",
                            cors_origin.as_bytes().to_vec(),
                        )
                        .unwrap(),
                    ),
            );
            let _ = request.respond(response);
            mt.record_request("GET", 200);
            return;
        }

        // --- /admin/entities: list every entity declared in the
        // manifest, so the dashboard's data browser can render
        // a left-nav of tables without having to scrape the
        // manifest separately. Read-only, same auth as the
        // per-entity browse route below.
        if url == "/admin/entities" && method == Method::Get {
            if !is_dev
                && !verify_admin_or_metrics_auth(
                    &request,
                    admin_token.as_deref(),
                    &cookie_config,
                    &session_store,
                    runtime.as_ref(),
                )
            {
                let body = json_error(
                    "UNAUTHORIZED",
                    "/admin/entities requires PYLON_ADMIN_TOKEN or PYLON_METRICS_TOKEN bearer",
                );
                let response = with_security_headers(
                    Response::from_string(body)
                        .with_status_code(401u16)
                        .with_header(
                            Header::from_bytes("Content-Type", "application/json").unwrap(),
                        ),
                );
                let _ = request.respond(response);
                mt.record_request("GET", 401);
                return;
            }
            let entities: Vec<serde_json::Value> = runtime
                .manifest()
                .entities
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "name": e.name,
                        "fields": e.fields.iter().map(|f| serde_json::json!({
                            "name": f.name,
                            "type": f.field_type,
                            "optional": f.optional,
                        })).collect::<Vec<_>>(),
                    })
                })
                .collect();
            let body = serde_json::to_string(&entities).unwrap_or_else(|_| "[]".into());
            let response = with_security_headers(
                Response::from_string(body)
                    .with_status_code(200u16)
                    .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
                    .with_header(
                        Header::from_bytes(
                            "Access-Control-Allow-Origin",
                            cors_origin.as_bytes().to_vec(),
                        )
                        .unwrap(),
                    ),
            );
            let _ = request.respond(response);
            mt.record_request("GET", 200);
            return;
        }

        // --- /admin/entities/<E>: paginated row browse for the
        // cloud dashboard's data browser. Same auth shape as
        // /admin/logs/tail + /admin/fn/traces — accepts the
        // metrics token via `verify_admin_or_metrics_auth`.
        //
        // Bypasses entity read-policy because the dashboard
        // operator IS the operator. Forcing them to write a
        // permissive read policy just to render rows would widen
        // everyone's attack surface needlessly; the auth perimeter
        // (PYLON_METRICS_TOKEN) is what's locked down.
        //
        // GET /admin/entities/<EntityName>[?limit=N][&after=<id>]
        if let Some(rest) = url.strip_prefix("/admin/entities/") {
            let path = rest.split('?').next().unwrap_or(rest);
            // Bare entity name (no nested path segments). The full
            // CRUD surface (PATCH/DELETE for inline edit) is a v2
            // follow-up; v1 is read-only browse.
            let is_list = !path.is_empty() && !path.contains('/');
            if is_list && method == Method::Get {
                if !is_dev
                    && !verify_admin_or_metrics_auth(
                        &request,
                        admin_token.as_deref(),
                        &cookie_config,
                        &session_store,
                        runtime.as_ref(),
                    )
                {
                    let body = json_error(
                        "UNAUTHORIZED",
                        "/admin/entities/<E> requires PYLON_ADMIN_TOKEN or PYLON_METRICS_TOKEN bearer",
                    );
                    let response = with_security_headers(
                        Response::from_string(body)
                            .with_status_code(401u16)
                            .with_header(
                                Header::from_bytes("Content-Type", "application/json").unwrap(),
                            ),
                    );
                    let _ = request.respond(response);
                    mt.record_request("GET", 401);
                    return;
                }
                let qs = url.split_once('?').map(|(_, q)| q).unwrap_or("");
                let limit: usize = qs
                    .split('&')
                    .find_map(|kv| {
                        let (k, v) = kv.split_once('=')?;
                        if k == "limit" {
                            v.parse().ok()
                        } else {
                            None
                        }
                    })
                    .unwrap_or(50)
                    .clamp(1, 200);
                let after: Option<String> = qs.split('&').find_map(|kv| {
                    let (k, v) = kv.split_once('=')?;
                    if k == "after" {
                        Some(percent_decode_str(v))
                    } else {
                        None
                    }
                });
                use pylon_http::DataStore as _;
                let rows = runtime
                    .list_after(path, after.as_deref(), limit + 1)
                    .unwrap_or_default();
                let has_more = rows.len() > limit;
                let page: Vec<serde_json::Value> = rows.into_iter().take(limit).collect();
                let next_cursor: Option<String> = if has_more {
                    page.last()
                        .and_then(|r| r.get("id"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                } else {
                    None
                };
                let body = serde_json::json!({
                    "data": page,
                    "next_cursor": next_cursor,
                    "has_more": has_more,
                })
                .to_string();
                let response = with_security_headers(
                    Response::from_string(body)
                        .with_status_code(200u16)
                        .with_header(
                            Header::from_bytes("Content-Type", "application/json").unwrap(),
                        )
                        .with_header(
                            Header::from_bytes(
                                "Access-Control-Allow-Origin",
                                cors_origin.as_bytes().to_vec(),
                            )
                            .unwrap(),
                        ),
                );
                let _ = request.respond(response);
                mt.record_request("GET", 200);
                return;
            }
        }

        // --- /admin/fn/traces: recent function execution traces.
        //
        // Companion to /api/fn/traces but accepts the METRICS token
        // too, so Pylon Cloud's dashboard can stream traces over the
        // same read-only credential it uses for /metrics and
        // /admin/logs/tail. Returns the same FnTrace JSON shape.
        //
        // Why this exists separate from /api/fn/traces: the existing
        // endpoint requires full PYLON_ADMIN_TOKEN (writes + drops
        // permitted). Pylon Cloud stamps a read-only metrics token at
        // provision time, never the full admin token — keeping the
        // dashboard from being able to drop tables would be moot if
        // we required full admin to read traces. New endpoint, same
        // auth shape as /admin/logs/tail, no behavior change on the
        // existing one.
        if url.starts_with("/admin/fn/traces") && method == Method::Get {
            if !is_dev
                && !verify_admin_or_metrics_auth(
                    &request,
                    admin_token.as_deref(),
                    &cookie_config,
                    &session_store,
                    runtime.as_ref(),
                )
            {
                let body = json_error(
                    "UNAUTHORIZED",
                    "/admin/fn/traces requires PYLON_ADMIN_TOKEN or PYLON_METRICS_TOKEN bearer in non-dev mode",
                );
                let response = with_security_headers(
                    Response::from_string(&body)
                        .with_status_code(401u16)
                        .with_header(
                            Header::from_bytes("Content-Type", "application/json").unwrap(),
                        ),
                );
                let _ = request.respond(response);
                mt.record_request("GET", 401);
                return;
            }
            let limit: usize = url
                .split_once('?')
                .map(|(_, q)| q)
                .and_then(|q| {
                    q.split('&').find_map(|kv| {
                        let (k, v) = kv.split_once('=')?;
                        if k == "limit" {
                            v.parse().ok()
                        } else {
                            None
                        }
                    })
                })
                .unwrap_or(100)
                .min(500);
            let traces = match fn_ops_maybe.as_ref() {
                Some(ops) => {
                    use pylon_router::FnOps as _;
                    let traces = ops.recent_traces(limit);
                    serde_json::to_string(&traces).unwrap_or_else(|_| "[]".into())
                }
                None => "[]".into(),
            };
            let response = with_security_headers(
                Response::from_string(traces)
                    .with_status_code(200u16)
                    .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
                    .with_header(
                        Header::from_bytes(
                            "Access-Control-Allow-Origin",
                            cors_origin.as_bytes().to_vec(),
                        )
                        .unwrap(),
                    ),
            );
            let _ = request.respond(response);
            mt.record_request("GET", 200);
            return;
        }

        // --- /admin/jobs[/...]: read-only job-queue surface for the
        // cloud dashboard. Same auth shape as /admin/fn/traces (accepts
        // PYLON_METRICS_TOKEN via verify_admin_or_metrics_auth). The
        // existing /api/jobs/* surface requires full admin token and
        // also handles writes; this is the read-only sibling.
        //
        //   GET /admin/jobs                   → list jobs
        //   GET /admin/jobs/stats             → stats counters
        //   GET /admin/jobs/dead              → dead-letter queue
        //   GET /admin/jobs/<id>              → one job detail
        if url.starts_with("/admin/jobs") && method == Method::Get {
            if !is_dev
                && !verify_admin_or_metrics_auth(
                    &request,
                    admin_token.as_deref(),
                    &cookie_config,
                    &session_store,
                    runtime.as_ref(),
                )
            {
                let body = json_error(
                    "UNAUTHORIZED",
                    "/admin/jobs requires PYLON_ADMIN_TOKEN or PYLON_METRICS_TOKEN bearer",
                );
                let response = with_security_headers(
                    Response::from_string(body)
                        .with_status_code(401u16)
                        .with_header(
                            Header::from_bytes("Content-Type", "application/json").unwrap(),
                        ),
                );
                let _ = request.respond(response);
                mt.record_request("GET", 401);
                return;
            }
            let path = url.split('?').next().unwrap_or(url.as_str());
            let body: String = if path == "/admin/jobs/stats" {
                serde_json::to_string(&job_queue.stats()).unwrap_or_else(|_| "{}".into())
            } else if path == "/admin/jobs/dead" {
                serde_json::to_string(&job_queue.dead_letters()).unwrap_or_else(|_| "[]".into())
            } else if path == "/admin/jobs" {
                let status_filter = url
                    .split("status=")
                    .nth(1)
                    .and_then(|s| s.split('&').next());
                let queue_filter = url.split("queue=").nth(1).and_then(|s| s.split('&').next());
                let limit: usize = url
                    .split("limit=")
                    .nth(1)
                    .and_then(|s| s.split('&').next())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(100)
                    .min(500);
                serde_json::to_string(&job_queue.list_jobs(status_filter, queue_filter, limit))
                    .unwrap_or_else(|_| "[]".into())
            } else if let Some(job_id) = path.strip_prefix("/admin/jobs/") {
                if job_id.is_empty() {
                    "{}".into()
                } else {
                    match job_queue.get_job(job_id) {
                        Some(job) => serde_json::to_string(&job).unwrap_or_else(|_| "{}".into()),
                        None => {
                            let nf = json_error("NOT_FOUND", "Job not found");
                            let response = with_security_headers(
                                Response::from_string(nf)
                                    .with_status_code(404u16)
                                    .with_header(
                                        Header::from_bytes("Content-Type", "application/json")
                                            .unwrap(),
                                    ),
                            );
                            let _ = request.respond(response);
                            mt.record_request("GET", 404);
                            return;
                        }
                    }
                }
            } else {
                "{}".into()
            };
            let response = with_security_headers(
                Response::from_string(body)
                    .with_status_code(200u16)
                    .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
                    .with_header(
                        Header::from_bytes(
                            "Access-Control-Allow-Origin",
                            cors_origin.as_bytes().to_vec(),
                        )
                        .unwrap(),
                    ),
            );
            let _ = request.respond(response);
            mt.record_request("GET", 200);
            return;
        }

        // --- /admin/workflows[/...]: read-only workflow surface.
        //
        //   GET /admin/workflows/definitions  → registered workflow defs
        //   GET /admin/workflows              → instances (optional ?status=)
        //   GET /admin/workflows/<id>         → one instance detail
        if url.starts_with("/admin/workflows") && method == Method::Get {
            if !is_dev
                && !verify_admin_or_metrics_auth(
                    &request,
                    admin_token.as_deref(),
                    &cookie_config,
                    &session_store,
                    runtime.as_ref(),
                )
            {
                let body = json_error(
                    "UNAUTHORIZED",
                    "/admin/workflows requires PYLON_ADMIN_TOKEN or PYLON_METRICS_TOKEN bearer",
                );
                let response = with_security_headers(
                    Response::from_string(body)
                        .with_status_code(401u16)
                        .with_header(
                            Header::from_bytes("Content-Type", "application/json").unwrap(),
                        ),
                );
                let _ = request.respond(response);
                mt.record_request("GET", 401);
                return;
            }
            let path = url.split('?').next().unwrap_or(url.as_str());
            let body: String = if path == "/admin/workflows/definitions" {
                serde_json::to_string(&workflow_engine.definitions())
                    .unwrap_or_else(|_| "[]".into())
            } else if path == "/admin/workflows" {
                let status_filter: Option<crate::workflows::WorkflowStatus> = url
                    .split("status=")
                    .nth(1)
                    .and_then(|s| s.split('&').next())
                    .and_then(|s| match s {
                        "running" => Some(crate::workflows::WorkflowStatus::Running),
                        "completed" => Some(crate::workflows::WorkflowStatus::Completed),
                        "failed" => Some(crate::workflows::WorkflowStatus::Failed),
                        "waiting" => Some(crate::workflows::WorkflowStatus::WaitingForEvent),
                        "sleeping" => Some(crate::workflows::WorkflowStatus::Sleeping),
                        "pending" => Some(crate::workflows::WorkflowStatus::Pending),
                        "cancelled" => Some(crate::workflows::WorkflowStatus::Cancelled),
                        _ => None,
                    });
                serde_json::to_string(&workflow_engine.list(status_filter.as_ref()))
                    .unwrap_or_else(|_| "[]".into())
            } else if let Some(wf_id) = path.strip_prefix("/admin/workflows/") {
                if wf_id.is_empty() {
                    "{}".into()
                } else {
                    match workflow_engine.get(wf_id) {
                        Some(wf) => serde_json::to_string(&wf).unwrap_or_else(|_| "{}".into()),
                        None => {
                            let nf = json_error("NOT_FOUND", "Workflow instance not found");
                            let response = with_security_headers(
                                Response::from_string(nf)
                                    .with_status_code(404u16)
                                    .with_header(
                                        Header::from_bytes("Content-Type", "application/json")
                                            .unwrap(),
                                    ),
                            );
                            let _ = request.respond(response);
                            mt.record_request("GET", 404);
                            return;
                        }
                    }
                }
            } else {
                "{}".into()
            };
            let response = with_security_headers(
                Response::from_string(body)
                    .with_status_code(200u16)
                    .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
                    .with_header(
                        Header::from_bytes(
                            "Access-Control-Allow-Origin",
                            cors_origin.as_bytes().to_vec(),
                        )
                        .unwrap(),
                    ),
            );
            let _ = request.respond(response);
            mt.record_request("GET", 200);
            return;
        }

        // --- Rate limiting: check per-IP request count ---
        // peer_ip honors PYLON_TRUST_PROXY_HOPS so a deploy behind a
        // load balancer (Fly, nginx, CloudFront) gets per-client
        // limiting instead of putting every request through one
        // bucket keyed by the proxy's IP.
        let peer_ip = resolve_client_ip(&request, trust_proxy_hops);

        // OPTIONS preflights are browser infrastructure, not user intent.
        // Rate-limiting them makes a normal page effectively halve its
        // budget (preflight + real request per call) and returns a 429
        // that the browser can't interpret as a valid CORS response —
        // the user-visible symptom is "Failed to fetch" on login. Skip.
        let is_preflight = matches!(method, Method::Options);
        if !is_preflight {
            if let Err(retry_after) = rate_limiter.check(&peer_ip) {
                let err_body = json_error(
                    "RATE_LIMITED",
                    &format!("Too many requests. Retry after {retry_after} seconds."),
                );
                let response = with_security_headers(
                    Response::from_string(&err_body)
                        .with_status_code(429u16)
                        .with_header(
                            Header::from_bytes("Content-Type", "application/json").unwrap(),
                        )
                        .with_header(
                            Header::from_bytes(
                                "Access-Control-Allow-Origin",
                                cors_origin.as_bytes().to_vec(),
                            )
                            .unwrap(),
                        )
                        .with_header(
                            Header::from_bytes(
                                "Access-Control-Allow-Methods",
                                "GET, POST, PATCH, DELETE, OPTIONS",
                            )
                            .unwrap(),
                        )
                        .with_header(
                            Header::from_bytes(
                                "Access-Control-Allow-Headers",
                                "Content-Type, Authorization",
                            )
                            .unwrap(),
                        )
                        .with_header(
                            Header::from_bytes(
                                "Retry-After",
                                retry_after.to_string().as_bytes().to_vec(),
                            )
                            .unwrap(),
                        ),
                );
                let _ = request.respond(response);
                mt.record_request(method.as_str(), 429);
                return;
            }
        } // end: if !is_preflight

        // --- CSRF check on state-changing requests ---
        //
        // Browsers forbid cross-origin POST/PATCH/PUT/DELETE unless CORS
        // allows it, but an attacker controlling another origin can still
        // ship credentials-bearing requests if the server is permissive.
        // The CSRF plugin validates Origin (then Referer) against an explicit
        // allowlist — this is the check that was missing because the Plugin
        // trait's `on_request` hook has no access to headers.
        //
        // The Authorization header carries bearer tokens, so CSRF mostly
        // matters for cookie-based sessions — but we enforce globally: a
        // request that misses Origin/Referer on a state-changing method is
        // rejected, which is the safer default.
        {
            let method_str = method.as_str();
            let is_bearer = request.headers().iter().any(|h| {
                (h.field.as_str() == "Authorization" || h.field.as_str() == "authorization")
                    && h.value.as_str().starts_with("Bearer ")
            });
            // Bearer-authenticated requests are not CSRF-vulnerable in the
            // classic sense — browsers don't auto-attach bearer tokens. Skip
            // the check for them so server-to-server API callers keep working
            // without needing Origin headers.
            if !is_bearer && !matches!(method, Method::Get | Method::Head | Method::Options) {
                let origin = request
                    .headers()
                    .iter()
                    .find(|h| h.field.as_str() == "Origin" || h.field.as_str() == "origin")
                    .map(|h| h.value.as_str().to_string());
                let referer = request
                    .headers()
                    .iter()
                    .find(|h| h.field.as_str() == "Referer" || h.field.as_str() == "referer")
                    .map(|h| h.value.as_str().to_string());
                if let Err(err) = csrf.check(method_str, origin.as_deref(), referer.as_deref()) {
                    let body = json_error(&err.code, &err.message);
                    let response = with_security_headers(
                        Response::from_string(&body)
                            .with_status_code(err.status)
                            .with_header(
                                Header::from_bytes("Content-Type", "application/json").unwrap(),
                            )
                            .with_header(
                                Header::from_bytes(
                                    "Access-Control-Allow-Origin",
                                    cors_origin.as_bytes().to_vec(),
                                )
                                .unwrap(),
                            ),
                    );
                    let _ = request.respond(response);
                    mt.record_request(method_str, err.status);
                    return;
                }
            }
        }

        // Extract auth token + auth context EARLY so every fast path (upload,
        // shard SSE, fn streaming, AI streaming) can enforce auth the same
        // way the router does. Previously these paths ran before auth
        // extraction and bypassed the plugin/router auth chain entirely.
        //
        // Two transports for the same opaque session token:
        //   1. `Authorization: Bearer <token>` — CLI, mobile, server-to-server
        //   2. `Cookie: <name>=<token>` — browsers (HttpOnly, XSS can't read)
        // Bearer wins when both are present (explicit beats ambient).
        let bearer_token: Option<String> = request
            .headers()
            .iter()
            .find(|h| h.field.as_str() == "Authorization" || h.field.as_str() == "authorization")
            .and_then(|h| {
                let val = h.value.as_str();
                val.strip_prefix("Bearer ").map(|t| t.to_string())
            });
        let cookie_token: Option<String> = if bearer_token.is_some() {
            None
        } else {
            request
                .headers()
                .iter()
                .find(|h| h.field.as_str() == "Cookie" || h.field.as_str() == "cookie")
                .and_then(|h| {
                    pylon_auth::extract_session_cookie(h.value.as_str(), &cookie_config.name)
                })
        };
        // Studio admin cookie — set by POST /studio/login when the
        // user submits the right PYLON_ADMIN_TOKEN. Stored under
        // `${app_name}_admin` so it doesn't collide with the regular
        // session cookie. When present + matches admin_token, the
        // dispatcher returns AuthContext::admin() (handled below).
        let admin_cookie_name = format!("{}_admin", &runtime.manifest().name);
        let admin_cookie_token: Option<String> = request
            .headers()
            .iter()
            .find(|h| h.field.as_str() == "Cookie" || h.field.as_str() == "cookie")
            .and_then(|h| pylon_auth::extract_session_cookie(h.value.as_str(), &admin_cookie_name));
        let auth_token: Option<String> = bearer_token.or(cookie_token);
        // Token dispatcher (in priority order):
        //   1. Admin token → AuthContext::admin
        //   2. `pk.…` API key → AuthContext::from_api_key (401 on bad)
        //   3. Looks-like-JWT + PYLON_JWT_SECRET set → JWT verify
        //   4. Otherwise → session store lookup
        // pk. check happens BEFORE looks_like_jwt because an api-key
        // token also has 3 dot-separated segments and would otherwise
        // be misrouted.
        let auth_ctx_result: Result<pylon_auth::AuthContext, &'static str> = if admin_token
            .is_some()
            && admin_cookie_token.is_some()
            && pylon_auth::constant_time_eq(
                admin_cookie_token.as_deref().unwrap_or("").as_bytes(),
                admin_token.as_deref().unwrap_or("").as_bytes(),
            ) {
            // Studio admin cookie matched. Same auth as Bearer admin —
            // we just got here via the /studio/login form instead of an
            // Authorization header.
            Ok(pylon_auth::AuthContext::admin())
        } else if admin_token.is_some()
            && auth_token.is_some()
            && pylon_auth::constant_time_eq(
                auth_token.as_deref().unwrap_or("").as_bytes(),
                admin_token.as_deref().unwrap_or("").as_bytes(),
            )
        {
            Ok(pylon_auth::AuthContext::admin())
        } else if let Some(t) = auth_token.as_deref() {
            if t.starts_with("pk.") {
                match ak.verify(t) {
                    Ok(key) => Ok(pylon_auth::AuthContext::from_api_key(
                        key.user_id,
                        key.id,
                        key.scopes,
                    )),
                    Err(_) => Err("INVALID_API_KEY"),
                }
            } else if pylon_auth::jwt::looks_like_jwt(t) && jwt_secret().is_some() {
                // P0-6 (codex Wave-5 review): require PYLON_JWT_ISSUER
                // when JWT auth is enabled. Without it, tokens minted
                // with the same HS256 secret for ANY issuer would
                // verify, letting a JWT minted for "external-system"
                // log in as that system's `sub`. Refuse on misconfig.
                let Some(issuer) = jwt_issuer() else {
                    tracing::warn!(
                        "[auth] PYLON_JWT_SECRET set but PYLON_JWT_ISSUER missing — \
                         refusing JWT verify (set both to enable JWT sessions)"
                    );
                    // Pre-refactor used `Err("JWT_MISCONFIGURED")?` which
                    // propagated to start()'s Result and crashed the server.
                    // Now we're inside a per-request closure returning () —
                    // reject just this request and let the operator see the
                    // misconfig in the warn log + their failed requests.
                    let body = json_error(
                        "JWT_MISCONFIGURED",
                        "JWT auth requires both PYLON_JWT_SECRET and PYLON_JWT_ISSUER",
                    );
                    let response = with_security_headers(
                        Response::from_string(&body)
                            .with_status_code(503u16)
                            .with_header(
                                Header::from_bytes("Content-Type", "application/json").unwrap(),
                            ),
                    );
                    let _ = request.respond(response);
                    mt.record_request(method.as_str(), 503);
                    return;
                };
                let secret = jwt_secret().expect("checked above");
                match pylon_auth::jwt::verify(t, secret.as_bytes(), Some(issuer)) {
                    Ok(claims) => {
                        let mut ctx = pylon_auth::AuthContext::authenticated(claims.sub);
                        ctx.roles = claims.roles;
                        if let Some(t) = claims.tenant_id {
                            ctx = ctx.with_tenant(t);
                        }
                        Ok(ctx)
                    }
                    Err(_) => Err("INVALID_JWT"),
                }
            } else {
                Ok(ss.resolve(Some(t)))
            }
        } else {
            Ok(ss.resolve(None))
        };
        let mut auth_ctx = match auth_ctx_result {
            Ok(c) => c,
            Err(reason) => {
                let body = format!(
                    r#"{{"error":{{"code":"{reason}","message":"Bearer token is malformed, expired, or revoked"}}}}"#
                );
                let resp = tiny_http::Response::from_string(body)
                    .with_status_code(401)
                    .with_header(
                        "Content-Type: application/json"
                            .parse::<tiny_http::Header>()
                            .unwrap(),
                    );
                let _ = request.respond(resp);
                return;
            }
        };

        // Wave-7 E: trusted-device cookie. Read `pylon_trusted_device=<token>`,
        // resolve it, and if the record's user_id matches the current
        // session's user_id stamp `auth_ctx.is_trusted_device = true`. App
        // code uses this to skip the TOTP step for repeat sign-ins from
        // the same browser. Bound to the user — a stale cookie left from
        // a previous account on the same browser quietly degrades to
        // untrusted (we don't actively reject, just don't trust).
        if let Some(uid) = auth_ctx.user_id.as_deref() {
            let trust_token: Option<String> = request
                .headers()
                .iter()
                .find(|h| h.field.as_str() == "Cookie" || h.field.as_str() == "cookie")
                .and_then(|h| {
                    pylon_auth::extract_session_cookie(
                        h.value.as_str(),
                        pylon_auth::trusted_device::TRUST_COOKIE_NAME,
                    )
                });
            if let Some(token) = trust_token {
                if let Some(record) = td.find(&token) {
                    if record.user_id == uid {
                        auth_ctx.is_trusted_device = true;
                    }
                }
            }
        }
        // Per-user admin resolution. Two independent paths feed into
        // is_admin; either one is enough to lift it.
        //
        // 1. `auth.user.admin_field` (manifest config) — load the
        //    User row, check the configured boolean/string/role-array
        //    field, lift is_admin when truthy. Apps with a
        //    User.isAdmin column (or "admin" role) configure this so
        //    platform admins sign in with their regular account and
        //    Studio respects the role.
        //
        // 2. `PYLON_ADMIN_EMAILS` env var — comma-separated allowlist
        //    of verified emails. On every successful auth resolution,
        //    a matched user gets is_admin lifted AND (when
        //    admin_field is configured) the User row's flag is
        //    flipped to true so the promotion survives env-var
        //    removal. Operator-friendly: every Pylon app gets
        //    "designate admins by email" without writing app code.
        //
        // PYLON_ADMIN_TOKEN remains as the operator-token escape
        // hatch for cron/migration scripts.
        //
        // SECURITY: API-key contexts are excluded from admin
        // promotion. A leaked / scoped `pk.*` token for an admin-
        // allowlisted user must NOT escalate to is_admin — the
        // API-key issuer minted that token with a specific scope,
        // not "act as this user across every privileged route".
        // Caught in the 2026-05-09 codex security audit. Apps that
        // genuinely want API keys with admin power should explicitly
        // mint them via the admin-token path.
        if !auth_ctx.is_admin && !auth_ctx.is_api_key_auth() {
            if let Some(uid) = auth_ctx.user_id.clone() {
                let user_entity = runtime.manifest().auth.user.entity.clone();
                let admin_field = runtime
                    .manifest()
                    .auth
                    .user
                    .admin_field
                    .clone()
                    .filter(|f| !f.is_empty());
                use pylon_http::DataStore as _;
                let row = runtime.get_by_id(&user_entity, &uid).ok().flatten();

                // Path 1: admin_field on the User row.
                if let (Some(ref field), Some(ref row)) = (&admin_field, &row) {
                    let truthy = match row.get(field.as_str()) {
                        Some(v) if v.is_boolean() => v.as_bool().unwrap_or(false),
                        Some(v) if v.is_string() => {
                            let s = v.as_str().unwrap_or("").to_ascii_lowercase();
                            s == "true" || s == "1" || s == "admin"
                        }
                        Some(v) if v.is_number() => v.as_i64().map(|n| n != 0).unwrap_or(false),
                        Some(v) if v.is_array() => v
                            .as_array()
                            .map(|items| items.iter().any(|x| x.as_str() == Some("admin")))
                            .unwrap_or(false),
                        _ => false,
                    };
                    if truthy {
                        auth_ctx.is_admin = true;
                    }
                }

                // Path 2: PYLON_ADMIN_EMAILS allowlist. Only fires when
                // the User row carries a verified email — we never
                // promote on an unverified claim. Match is case-
                // insensitive and tolerates whitespace around entries.
                if !auth_ctx.is_admin {
                    let allow: Vec<String> = std::env::var("PYLON_ADMIN_EMAILS")
                        .unwrap_or_default()
                        .split(',')
                        .map(|s| s.trim().to_ascii_lowercase())
                        .filter(|s| !s.is_empty())
                        .collect();
                    if !allow.is_empty() {
                        if let Some(ref row) = row {
                            let email = row
                                .get("email")
                                .and_then(|v| v.as_str())
                                .map(|s| s.trim().to_ascii_lowercase())
                                .filter(|s| !s.is_empty());
                            let verified = row
                                .get("emailVerified")
                                .or_else(|| row.get("email_verified"))
                                .map(|v| match v {
                                    serde_json::Value::Bool(b) => *b,
                                    // Treat any non-empty timestamp / "true" /
                                    // "1" string as verified (consistent with
                                    // how the rest of the framework reads
                                    // "verified" timestamps).
                                    serde_json::Value::String(s) => {
                                        !s.is_empty() && !s.eq_ignore_ascii_case("false")
                                    }
                                    serde_json::Value::Number(n) => {
                                        n.as_i64().map(|n| n != 0).unwrap_or(false)
                                    }
                                    serde_json::Value::Null => false,
                                    _ => false,
                                })
                                .unwrap_or(false);
                            if verified {
                                if let Some(email) = email {
                                    if allow.iter().any(|a| a == &email) {
                                        auth_ctx.is_admin = true;
                                        // Persist when an admin_field is
                                        // configured. Best-effort — a write
                                        // failure shouldn't fail the request,
                                        // we already lifted in-memory. The
                                        // spec says "removing an email from
                                        // the env doesn't demote", and the
                                        // persisted flag is what makes that
                                        // true.
                                        if let Some(field) = &admin_field {
                                            let already_truthy = row
                                                .get(field.as_str())
                                                .and_then(|v| v.as_bool())
                                                .unwrap_or(false);
                                            if !already_truthy {
                                                let payload = serde_json::json!({
                                                    field: true,
                                                });
                                                let _ =
                                                    runtime.update(&user_entity, &uid, &payload);
                                                tracing::info!(
                                                    target: "pylon::auth",
                                                    "[admin-emails] promoted {email} via PYLON_ADMIN_EMAILS"
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        let auth_ctx = auth_ctx;

        // --- Test-reset endpoint — in-memory + dev mode + localhost only ---
        //
        // `pylon test` sets PYLON_IN_MEMORY=1 + PYLON_DEV_MODE=true.
        // The TS helper `resetDb()` posts here between `test(...)` blocks
        // to isolate cases. Gates:
        //   1. dev mode (production refuses outright)
        //   2. in-memory DB (belt-and-braces against accidental file wipes)
        //   3. peer IP is loopback (a dev laptop often has localhost:4321
        //      reachable; without this, a browser visiting a malicious
        //      site could cross-site-POST a reset via a bare form —
        //      blind CSRF that doesn't care about the response)
        //
        // Positioned AFTER the rate limiter and CSRF check on purpose so
        // those middlewares apply — the earlier placement skipped both.
        if url == "/api/__test__/reset" && method == Method::Post {
            let is_loopback = peer_ip == "127.0.0.1"
                || peer_ip == "::1"
                || peer_ip.starts_with("127.")
                || peer_ip == "localhost";
            if !is_dev || !rt.is_in_memory() || !is_loopback {
                let body = json_error(
                    "RESET_REFUSED",
                    "reset endpoint is only available in dev mode + in-memory DB + from loopback",
                );
                let response = with_security_headers(
                    Response::from_string(&body)
                        .with_status_code(403u16)
                        .with_header(
                            Header::from_bytes("Content-Type", "application/json").unwrap(),
                        )
                        .with_header(
                            Header::from_bytes(
                                "Access-Control-Allow-Origin",
                                cors_origin.as_bytes().to_vec(),
                            )
                            .unwrap(),
                        ),
                );
                let _ = request.respond(response);
                mt.record_request("POST", 403);
                return;
            }
            let (status, body) = match rt.reset_for_tests() {
                Ok(()) => (200u16, "{\"reset\":true}".to_string()),
                Err(e) => (500u16, json_error(&e.code, &e.message)),
            };
            let response = with_security_headers(
                Response::from_string(&body)
                    .with_status_code(status)
                    .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
                    .with_header(
                        Header::from_bytes(
                            "Access-Control-Allow-Origin",
                            cors_origin.as_bytes().to_vec(),
                        )
                        .unwrap(),
                    ),
            );
            let _ = request.respond(response);
            mt.record_request("POST", status);
            return;
        }

        // --- File upload: 3-step flow (init → client PUT → confirm) ---
        //
        // Pre-0.3.91 pylon proxied multipart uploads through its own
        // process: `POST /api/files/upload` parsed the body in memory
        // then forwarded bytes to Stack0/local. 30MB+ uploads OOM'd
        // the multipart parser. v0.3.91 switched to a direct-to-S3
        // flow:
        //
        //   1. POST /api/files/init  →  {uploadUrl, assetId, cdnUrl}
        //   2. client PUTs raw bytes to uploadUrl (S3 for Stack0,
        //      pylon's `/api/files/local-put/<id>` for local)
        //   3. POST /api/files/confirm {assetId}  →  {id, url, size}
        //
        // Bytes never transit pylon's process for Stack0. Local backend
        // still receives bytes via PUT, but as a single binary stream
        // (no multipart parsing overhead).

        // POST /api/files/init — step 1.
        if url == "/api/files/init" && method == Method::Post {
            if auth_ctx.user_id.is_none() {
                let err = json_error(
                    "AUTH_REQUIRED",
                    "/api/files/init requires an authenticated session",
                );
                let response = with_security_headers(
                    Response::from_string(&err)
                        .with_status_code(401u16)
                        .with_header(
                            Header::from_bytes("Content-Type", "application/json").unwrap(),
                        )
                        .with_header(
                            Header::from_bytes(
                                "Access-Control-Allow-Origin",
                                cors_origin.as_bytes().to_vec(),
                            )
                            .unwrap(),
                        ),
                );
                let _ = request.respond(response);
                mt.record_request("POST", 401);
                return;
            }
            use std::io::Read;
            let mut body_bytes = Vec::new();
            let _ = request
                .as_reader()
                .take(64 * 1024)
                .read_to_end(&mut body_bytes);
            let body_str = String::from_utf8_lossy(&body_bytes);
            let v: serde_json::Value = match serde_json::from_str(&body_str) {
                Ok(v) => v,
                Err(e) => {
                    let err = json_error("INVALID_JSON", &e.to_string());
                    let response = with_security_headers(
                        Response::from_string(&err)
                            .with_status_code(400u16)
                            .with_header(
                                Header::from_bytes("Content-Type", "application/json").unwrap(),
                            )
                            .with_header(
                                Header::from_bytes(
                                    "Access-Control-Allow-Origin",
                                    cors_origin.as_bytes().to_vec(),
                                )
                                .unwrap(),
                            ),
                    );
                    let _ = request.respond(response);
                    mt.record_request("POST", 400);
                    return;
                }
            };
            let filename = v["filename"].as_str().unwrap_or("upload");
            let mime_type = v["mimeType"].as_str().unwrap_or("application/octet-stream");
            let size = v["size"].as_u64().unwrap_or(0) as usize;

            // Enforce upload-max BEFORE handing out the URL — otherwise
            // a client could PUT 10GB to S3 and pylon would have no
            // mechanism to stop it. Stack0 enforces its own limits
            // server-side, but checking here gives operators one knob
            // (`PYLON_MAX_UPLOAD_BYTES`) regardless of backend.
            let upload_max: usize = std::env::var("PYLON_MAX_UPLOAD_BYTES")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(200 * 1024 * 1024);
            if size > upload_max {
                let err = json_error(
                    "PAYLOAD_TOO_LARGE",
                    &format!("size {size} exceeds upload max of {upload_max}"),
                );
                let response = with_security_headers(
                    Response::from_string(&err)
                        .with_status_code(413u16)
                        .with_header(
                            Header::from_bytes("Content-Type", "application/json").unwrap(),
                        )
                        .with_header(
                            Header::from_bytes(
                                "Access-Control-Allow-Origin",
                                cors_origin.as_bytes().to_vec(),
                            )
                            .unwrap(),
                        ),
                );
                let _ = request.respond(response);
                mt.record_request("POST", 413);
                return;
            }

            let storage = pylon_storage::files::select_from_env();
            let (status, body) = match storage.init_upload(filename, mime_type, size) {
                Ok(init) => (
                    200u16,
                    serde_json::to_string(&init).unwrap_or_else(|_| "{}".into()),
                ),
                Err(e) => (500u16, json_error(&e.code, &e.message)),
            };
            let response = with_security_headers(
                Response::from_string(&body)
                    .with_status_code(status)
                    .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
                    .with_header(
                        Header::from_bytes(
                            "Access-Control-Allow-Origin",
                            cors_origin.as_bytes().to_vec(),
                        )
                        .unwrap(),
                    ),
            );
            let _ = request.respond(response);
            mt.record_request("POST", status);
            return;
        }

        // POST /api/files/confirm — step 3. Owner gets recorded here.
        if url == "/api/files/confirm" && method == Method::Post {
            if auth_ctx.user_id.is_none() {
                let err = json_error(
                    "AUTH_REQUIRED",
                    "/api/files/confirm requires an authenticated session",
                );
                let response = with_security_headers(
                    Response::from_string(&err)
                        .with_status_code(401u16)
                        .with_header(
                            Header::from_bytes("Content-Type", "application/json").unwrap(),
                        )
                        .with_header(
                            Header::from_bytes(
                                "Access-Control-Allow-Origin",
                                cors_origin.as_bytes().to_vec(),
                            )
                            .unwrap(),
                        ),
                );
                let _ = request.respond(response);
                mt.record_request("POST", 401);
                return;
            }
            use std::io::Read;
            let mut body_bytes = Vec::new();
            let _ = request
                .as_reader()
                .take(64 * 1024)
                .read_to_end(&mut body_bytes);
            let body_str = String::from_utf8_lossy(&body_bytes);
            let v: serde_json::Value = match serde_json::from_str(&body_str) {
                Ok(v) => v,
                Err(e) => {
                    let err = json_error("INVALID_JSON", &e.to_string());
                    let response = with_security_headers(
                        Response::from_string(&err)
                            .with_status_code(400u16)
                            .with_header(
                                Header::from_bytes("Content-Type", "application/json").unwrap(),
                            )
                            .with_header(
                                Header::from_bytes(
                                    "Access-Control-Allow-Origin",
                                    cors_origin.as_bytes().to_vec(),
                                )
                                .unwrap(),
                            ),
                    );
                    let _ = request.respond(response);
                    mt.record_request("POST", 400);
                    return;
                }
            };
            let asset_id = match v["assetId"].as_str() {
                Some(s) if !s.is_empty() => s.to_string(),
                _ => {
                    let err = json_error("MISSING_ASSET_ID", "Body must include `assetId`");
                    let response = with_security_headers(
                        Response::from_string(&err)
                            .with_status_code(400u16)
                            .with_header(
                                Header::from_bytes("Content-Type", "application/json").unwrap(),
                            )
                            .with_header(
                                Header::from_bytes(
                                    "Access-Control-Allow-Origin",
                                    cors_origin.as_bytes().to_vec(),
                                )
                                .unwrap(),
                            ),
                    );
                    let _ = request.respond(response);
                    mt.record_request("POST", 400);
                    return;
                }
            };

            let storage = pylon_storage::files::select_from_env();
            let storage: &dyn pylon_storage::files::FileStorage = storage.as_ref();
            let (status, body) = match storage.confirm_upload(&asset_id) {
                Ok(stored) => {
                    if let Some(uid) = auth_ctx.user_id.as_ref() {
                        let owner = pylon_storage::files::FileOwner {
                            user_id: uid.clone(),
                            tenant_id: auth_ctx.tenant_id.clone(),
                        };
                        if let Err(e) = storage.record_owner(&stored.id, &owner) {
                            tracing::error!(
                                file_id = %stored.id,
                                error = %e.message,
                                "Failed to record file owner on confirm"
                            );
                            let _ = storage.delete(&stored.id);
                            (500u16, json_error("OWNERSHIP_RECORD_FAILED", &e.message))
                        } else {
                            (
                                200u16,
                                serde_json::to_string(&stored).unwrap_or_else(|_| "{}".into()),
                            )
                        }
                    } else {
                        // Unreachable — we already 401'd above when user_id is None.
                        (500u16, json_error("INTERNAL", "auth lost between checks"))
                    }
                }
                Err(e) => (500u16, json_error(&e.code, &e.message)),
            };
            let response = with_security_headers(
                Response::from_string(&body)
                    .with_status_code(status)
                    .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
                    .with_header(
                        Header::from_bytes(
                            "Access-Control-Allow-Origin",
                            cors_origin.as_bytes().to_vec(),
                        )
                        .unwrap(),
                    ),
            );
            let _ = request.respond(response);
            mt.record_request("POST", status);
            return;
        }

        // PUT /api/files/local-put/<id> — local backend's byte
        // receiver. Stack0 callers PUT to S3 directly so this path
        // only fires for the local backend. Auth required so an
        // unauth'd attacker can't spam disk writes.
        if method == Method::Put {
            if let Some(rest) = url.strip_prefix("/api/files/local-put/") {
                let asset_id = rest.split('?').next().unwrap_or(rest);
                if auth_ctx.user_id.is_none() {
                    let err = json_error(
                        "AUTH_REQUIRED",
                        "PUT /api/files/local-put requires an authenticated session",
                    );
                    let response = with_security_headers(
                        Response::from_string(&err)
                            .with_status_code(401u16)
                            .with_header(
                                Header::from_bytes("Content-Type", "application/json").unwrap(),
                            )
                            .with_header(
                                Header::from_bytes(
                                    "Access-Control-Allow-Origin",
                                    cors_origin.as_bytes().to_vec(),
                                )
                                .unwrap(),
                            ),
                    );
                    let _ = request.respond(response);
                    mt.record_request("PUT", 401);
                    return;
                }
                let upload_max: usize = std::env::var("PYLON_MAX_UPLOAD_BYTES")
                    .ok()
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(200 * 1024 * 1024);
                if let Some(declared) = request.body_length() {
                    if declared > upload_max {
                        let err = json_error(
                            "PAYLOAD_TOO_LARGE",
                            &format!(
                                "Content-Length {declared} exceeds upload max of {upload_max}"
                            ),
                        );
                        let response = with_security_headers(
                            Response::from_string(&err)
                                .with_status_code(413u16)
                                .with_header(
                                    Header::from_bytes("Content-Type", "application/json").unwrap(),
                                )
                                .with_header(
                                    Header::from_bytes(
                                        "Access-Control-Allow-Origin",
                                        cors_origin.as_bytes().to_vec(),
                                    )
                                    .unwrap(),
                                ),
                        );
                        let _ = request.respond(response);
                        mt.record_request("PUT", 413);
                        return;
                    }
                }
                use std::io::Read;
                let mut bytes: Vec<u8> = Vec::with_capacity(8192);
                let mut limited = request.as_reader().take((upload_max as u64) + 1);
                let _ = limited.read_to_end(&mut bytes);
                if bytes.len() > upload_max {
                    let err = json_error(
                        "PAYLOAD_TOO_LARGE",
                        &format!("Body exceeds upload max of {upload_max} bytes"),
                    );
                    let response = with_security_headers(
                        Response::from_string(&err)
                            .with_status_code(413u16)
                            .with_header(
                                Header::from_bytes("Content-Type", "application/json").unwrap(),
                            )
                            .with_header(
                                Header::from_bytes(
                                    "Access-Control-Allow-Origin",
                                    cors_origin.as_bytes().to_vec(),
                                )
                                .unwrap(),
                            ),
                    );
                    let _ = request.respond(response);
                    mt.record_request("PUT", 413);
                    return;
                }
                let local = pylon_storage::files::local_from_env();
                let (status, body) = match local.write_bytes(asset_id, &bytes) {
                    Ok(()) => (204u16, String::new()),
                    Err(e) => (500u16, json_error(&e.code, &e.message)),
                };
                let response = with_security_headers(
                    Response::from_string(&body)
                        .with_status_code(status)
                        .with_header(
                            Header::from_bytes("Content-Type", "application/json").unwrap(),
                        )
                        .with_header(
                            Header::from_bytes(
                                "Access-Control-Allow-Origin",
                                cors_origin.as_bytes().to_vec(),
                            )
                            .unwrap(),
                        ),
                );
                let _ = request.respond(response);
                mt.record_request("PUT", status);
                return;
            }
        }

        // DELETE /api/files/<assetId> — owner-gated delete that
        // routes through the active backend.
        if method == Method::Delete {
            if let Some(rest) = url.strip_prefix("/api/files/") {
                let asset_id = rest.split('?').next().unwrap_or(rest);
                if auth_ctx.user_id.is_none() {
                    let err = json_error(
                        "AUTH_REQUIRED",
                        "DELETE /api/files requires an authenticated session",
                    );
                    let response = with_security_headers(
                        Response::from_string(&err)
                            .with_status_code(401u16)
                            .with_header(
                                Header::from_bytes("Content-Type", "application/json").unwrap(),
                            )
                            .with_header(
                                Header::from_bytes(
                                    "Access-Control-Allow-Origin",
                                    cors_origin.as_bytes().to_vec(),
                                )
                                .unwrap(),
                            ),
                    );
                    let _ = request.respond(response);
                    mt.record_request("DELETE", 401);
                    return;
                }
                let storage = pylon_storage::files::select_from_env();
                let storage: &dyn pylon_storage::files::FileStorage = storage.as_ref();
                // Owner check: backends that record ownership (local) must
                // match the requester. Stack0 returns None and we fall
                // through — its API-key gate covers access.
                if storage.requires_owner_check() {
                    match storage.owner_of(asset_id) {
                        Ok(Some(owner)) => {
                            if !auth_ctx.is_admin
                                && Some(&owner.user_id) != auth_ctx.user_id.as_ref()
                            {
                                let err = json_error("NOT_FOUND", "File not found");
                                let response = with_security_headers(
                                    Response::from_string(&err)
                                        .with_status_code(404u16)
                                        .with_header(
                                            Header::from_bytes("Content-Type", "application/json")
                                                .unwrap(),
                                        )
                                        .with_header(
                                            Header::from_bytes(
                                                "Access-Control-Allow-Origin",
                                                cors_origin.as_bytes().to_vec(),
                                            )
                                            .unwrap(),
                                        ),
                                );
                                let _ = request.respond(response);
                                mt.record_request("DELETE", 404);
                                return;
                            }
                        }
                        Ok(None) => {
                            // No owner record — for local backend this means
                            // the file doesn't exist OR predates ownership
                            // tracking. Either way: 404 to avoid leaking
                            // existence.
                            let err = json_error("NOT_FOUND", "File not found");
                            let response = with_security_headers(
                                Response::from_string(&err)
                                    .with_status_code(404u16)
                                    .with_header(
                                        Header::from_bytes("Content-Type", "application/json")
                                            .unwrap(),
                                    )
                                    .with_header(
                                        Header::from_bytes(
                                            "Access-Control-Allow-Origin",
                                            cors_origin.as_bytes().to_vec(),
                                        )
                                        .unwrap(),
                                    ),
                            );
                            let _ = request.respond(response);
                            mt.record_request("DELETE", 404);
                            return;
                        }
                        Err(_) => {}
                    }
                }
                let (status, body) = match storage.delete(asset_id) {
                    Ok(true) => (204u16, String::new()),
                    Ok(false) => (404u16, json_error("NOT_FOUND", "File not found")),
                    Err(e) => (500u16, json_error(&e.code, &e.message)),
                };
                let response = with_security_headers(
                    Response::from_string(&body)
                        .with_status_code(status)
                        .with_header(
                            Header::from_bytes("Content-Type", "application/json").unwrap(),
                        )
                        .with_header(
                            Header::from_bytes(
                                "Access-Control-Allow-Origin",
                                cors_origin.as_bytes().to_vec(),
                            )
                            .unwrap(),
                        ),
                );
                let _ = request.respond(response);
                mt.record_request("DELETE", status);
                return;
            }
        }

        // GET /api/files/<id> — retrieve a file. For backends with a
        // CDN URL (Stack0), 302-redirect to it so bytes go direct
        // from the CDN to the client. For local backend, stream the
        // bytes from disk through pylon. Owner check enforced for
        // backends that require it.
        if method == Method::Get {
            if let Some(rest) = url.strip_prefix("/api/files/") {
                let asset_id = rest.split('?').next().unwrap_or(rest);
                // Skip the reserved sub-paths handled separately above
                // (e.g. /api/files/init, /api/files/confirm,
                // /api/files/local-put/...).
                let is_reserved = asset_id == "init"
                    || asset_id == "confirm"
                    || asset_id.starts_with("local-put/");
                if !asset_id.is_empty() && !is_reserved {
                    if auth_ctx.user_id.is_none() {
                        let err = json_error(
                            "AUTH_REQUIRED",
                            "GET /api/files requires an authenticated session",
                        );
                        let response = with_security_headers(
                            Response::from_string(&err)
                                .with_status_code(401u16)
                                .with_header(
                                    Header::from_bytes("Content-Type", "application/json").unwrap(),
                                )
                                .with_header(
                                    Header::from_bytes(
                                        "Access-Control-Allow-Origin",
                                        cors_origin.as_bytes().to_vec(),
                                    )
                                    .unwrap(),
                                ),
                        );
                        let _ = request.respond(response);
                        mt.record_request("GET", 401);
                        return;
                    }
                    let storage = pylon_storage::files::select_from_env();
                    let storage: &dyn pylon_storage::files::FileStorage = storage.as_ref();
                    // Owner check for backends that track ownership.
                    if storage.requires_owner_check() && !auth_ctx.is_admin {
                        if let Ok(Some(owner)) = storage.owner_of(asset_id) {
                            if Some(&owner.user_id) != auth_ctx.user_id.as_ref() {
                                let err = json_error("NOT_FOUND", "File not found");
                                let response = with_security_headers(
                                    Response::from_string(&err)
                                        .with_status_code(404u16)
                                        .with_header(
                                            Header::from_bytes("Content-Type", "application/json")
                                                .unwrap(),
                                        )
                                        .with_header(
                                            Header::from_bytes(
                                                "Access-Control-Allow-Origin",
                                                cors_origin.as_bytes().to_vec(),
                                            )
                                            .unwrap(),
                                        ),
                                );
                                let _ = request.respond(response);
                                mt.record_request("GET", 404);
                                return;
                            }
                        }
                    }
                    // CDN-backed backends: 302 redirect to direct_url so
                    // bytes never transit pylon. Pre-0.3.91 pylon's
                    // get() proxied bytes through the process — a
                    // 30MB asset doubled memory pressure for no reason.
                    match storage.direct_url(asset_id) {
                        Ok(Some(target)) => {
                            let response = with_security_headers(
                                Response::from_string("")
                                    .with_status_code(302u16)
                                    .with_header(
                                        Header::from_bytes("Location", target.as_bytes().to_vec())
                                            .unwrap(),
                                    )
                                    .with_header(
                                        Header::from_bytes(
                                            "Access-Control-Allow-Origin",
                                            cors_origin.as_bytes().to_vec(),
                                        )
                                        .unwrap(),
                                    ),
                            );
                            let _ = request.respond(response);
                            mt.record_request("GET", 302);
                            return;
                        }
                        Ok(None) => {}
                        Err(e) => {
                            let err = json_error(&e.code, &e.message);
                            let response = with_security_headers(
                                Response::from_string(&err)
                                    .with_status_code(500u16)
                                    .with_header(
                                        Header::from_bytes("Content-Type", "application/json")
                                            .unwrap(),
                                    )
                                    .with_header(
                                        Header::from_bytes(
                                            "Access-Control-Allow-Origin",
                                            cors_origin.as_bytes().to_vec(),
                                        )
                                        .unwrap(),
                                    ),
                            );
                            let _ = request.respond(response);
                            mt.record_request("GET", 500);
                            return;
                        }
                    }
                    // No direct URL — local backend. Stream the bytes.
                    let (status, body, ct) = match storage.get(asset_id) {
                        Ok(content) => (200u16, content, "application/octet-stream".to_string()),
                        Err(e) if e.code == "NOT_FOUND" => (
                            404u16,
                            json_error("NOT_FOUND", "File not found").into_bytes(),
                            "application/json".to_string(),
                        ),
                        Err(e) => (
                            500u16,
                            json_error(&e.code, &e.message).into_bytes(),
                            "application/json".to_string(),
                        ),
                    };
                    let response = with_security_headers(
                        Response::from_data(body)
                            .with_status_code(status)
                            .with_header(Header::from_bytes("Content-Type", ct.as_bytes()).unwrap())
                            .with_header(
                                Header::from_bytes(
                                    "Access-Control-Allow-Origin",
                                    cors_origin.as_bytes().to_vec(),
                                )
                                .unwrap(),
                            ),
                    );
                    let _ = request.respond(response);
                    mt.record_request("GET", status);
                    return;
                }
            }
        }

        // Legacy multipart upload — removed in v0.3.91. The 3-endpoint
        // flow above replaces it. Returns a clear migration hint so
        // upgraders don't silently regress (and so any old client
        // sees a useful error instead of a 404).
        if url == "/api/files/upload" && method == Method::Post {
            let err = json_error(
                "UPLOAD_DEPRECATED",
                "POST /api/files/upload was removed in pylon 0.3.91 — the multipart proxy OOM'd on large uploads. \
                 Use the new 3-step flow: POST /api/files/init → client PUTs bytes to the returned uploadUrl → POST /api/files/confirm. \
                 See https://docs.pylonsync.com/concepts/files.",
            );
            let response = with_security_headers(
                Response::from_string(&err)
                    .with_status_code(410u16)
                    .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
                    .with_header(
                        Header::from_bytes(
                            "Access-Control-Allow-Origin",
                            cors_origin.as_bytes().to_vec(),
                        )
                        .unwrap(),
                    ),
            );
            let _ = request.respond(response);
            mt.record_request("POST", 410);
            return;
        }

        // Read body before routing (request is consumed by respond).
        // Skip for methods that cannot have a body.
        //
        // Size enforcement runs in TWO layers so a malicious client can't
        // stream 10 GiB into memory before we reject it:
        //   1. Content-Length header is compared to MAX_BODY_SIZE up front.
        //   2. The actual read uses `.take(MAX_BODY_SIZE + 1)` so a lying
        //      or chunked stream is capped at MAX + 1 bytes; if we read that
        //      many, we reject.
        const MAX_BODY_SIZE: usize = 10 * 1024 * 1024;

        if let Some(declared) = request.body_length() {
            if declared > MAX_BODY_SIZE {
                let err_body = json_error(
                    "PAYLOAD_TOO_LARGE",
                    &format!("Content-Length {declared} exceeds max of {MAX_BODY_SIZE}"),
                );
                let response = with_security_headers(
                    Response::from_string(&err_body)
                        .with_status_code(413u16)
                        .with_header(
                            Header::from_bytes(
                                "Access-Control-Allow-Origin",
                                cors_origin.as_bytes().to_vec(),
                            )
                            .unwrap(),
                        ),
                );
                let _ = request.respond(response);
                mt.record_request(method.as_str(), 413);
                return;
            }
        }

        let mut body = String::new();
        if !matches!(
            method,
            Method::Get | Method::Head | Method::Options | Method::Delete
        ) {
            use std::io::Read;
            let mut limited = request.as_reader().take((MAX_BODY_SIZE as u64) + 1);
            let _ = limited.read_to_string(&mut body);
        }
        // Stamp bytes_in for the shipper's per-request rollup. The
        // central dispatch loop is the only path that reads the body,
        // so this is the single right place to capture its size.
        crate::metrics::set_current_request_bytes(body.len());

        if body.len() > MAX_BODY_SIZE {
            let err_body = json_error(
                "PAYLOAD_TOO_LARGE",
                &format!(
                    "Request body exceeds maximum size of {} bytes",
                    MAX_BODY_SIZE,
                ),
            );
            let response = with_security_headers(
                Response::from_string(&err_body)
                    .with_status_code(413u16)
                    .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
                    .with_header(
                        Header::from_bytes(
                            "Access-Control-Allow-Origin",
                            cors_origin.as_bytes().to_vec(),
                        )
                        .unwrap(),
                    ),
            );
            let _ = request.respond(response);
            mt.record_request(method.as_str(), 413);
            return;
        }

        // (auth_token + auth_ctx were resolved above, before the fast paths.)

        // --- GET /api/shards/:id/connect — SSE snapshot stream ---
        if method == Method::Get {
            if let Some(rest) = url.strip_prefix("/api/shards/") {
                let rest = rest.split('?').next().unwrap_or(rest);
                if let Some(shard_id) = rest.strip_suffix("/connect") {
                    // Require an authenticated user. Shard SSE streams state
                    // snapshots tick-by-tick; an anonymous subscriber can
                    // both read that state AND influence via push_input (see
                    // the WS handler). Gate at the transport layer.
                    if auth_ctx.user_id.is_none() {
                        let err = json_error(
                            "AUTH_REQUIRED",
                            "Shard connect requires an authenticated session",
                        );
                        let response = with_security_headers(
                            Response::from_string(&err)
                                .with_status_code(401u16)
                                .with_header(
                                    Header::from_bytes("Content-Type", "application/json").unwrap(),
                                )
                                .with_header(
                                    Header::from_bytes(
                                        "Access-Control-Allow-Origin",
                                        cors_origin.as_bytes().to_vec(),
                                    )
                                    .unwrap(),
                                ),
                        );
                        let _ = request.respond(response);
                        mt.record_request("GET", 401);
                        return;
                    }
                    let shards = match &shards_ref {
                        Some(s) => Arc::clone(s),
                        None => {
                            let err = json_error(
                                "SHARDS_NOT_AVAILABLE",
                                "Shard system is not configured",
                            );
                            let response = with_security_headers(
                                Response::from_string(&err)
                                    .with_status_code(503u16)
                                    .with_header(
                                        Header::from_bytes("Content-Type", "application/json")
                                            .unwrap(),
                                    )
                                    .with_header(
                                        Header::from_bytes(
                                            "Access-Control-Allow-Origin",
                                            cors_origin.as_bytes().to_vec(),
                                        )
                                        .unwrap(),
                                    ),
                            );
                            let _ = request.respond(response);
                            mt.record_request("GET", 503);
                            return;
                        }
                    };
                    let shard = match shards.get(shard_id) {
                        Some(s) => s,
                        None => {
                            let err = json_error(
                                "SHARD_NOT_FOUND",
                                &format!("Shard \"{shard_id}\" not found"),
                            );
                            let response = with_security_headers(
                                Response::from_string(&err)
                                    .with_status_code(404u16)
                                    .with_header(
                                        Header::from_bytes("Content-Type", "application/json")
                                            .unwrap(),
                                    )
                                    .with_header(
                                        Header::from_bytes(
                                            "Access-Control-Allow-Origin",
                                            cors_origin.as_bytes().to_vec(),
                                        )
                                        .unwrap(),
                                    ),
                            );
                            let _ = request.respond(response);
                            mt.record_request("GET", 404);
                            return;
                        }
                    };

                    // Subscriber ID from ?sid= query param, else the authed user,
                    // else a generated anonymous ID.
                    let sub_id = url
                        .split("sid=")
                        .nth(1)
                        .and_then(|s| s.split('&').next())
                        .map(|s| s.to_string())
                        .or_else(|| auth_ctx.user_id.clone())
                        .unwrap_or_else(|| {
                            format!(
                                "anon_{}",
                                std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_nanos()
                            )
                        });
                    let subscriber_id = pylon_realtime::SubscriberId::new(sub_id);

                    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
                    let streaming_body = StreamingBody::new(rx);

                    let tx_clone = tx.clone();
                    let sink: pylon_realtime::SnapshotSink =
                        Box::new(move |tick: u64, bytes: &[u8]| {
                            // Format as SSE with an id: line carrying the tick
                            // number so clients can resume with Last-Event-ID.
                            let mut frame = format!("id: {tick}\ndata: ").into_bytes();
                            frame.extend_from_slice(bytes);
                            frame.extend_from_slice(b"\n\n");
                            let _ = tx_clone.send(frame);
                        });

                    let shard_auth = pylon_realtime::ShardAuth {
                        user_id: auth_ctx.user_id.clone(),
                        is_admin: auth_ctx.is_admin,
                    };
                    if let Err(e) = shard.add_subscriber(subscriber_id.clone(), sink, &shard_auth) {
                        let (status, code) = match &e {
                            pylon_realtime::ShardError::Unauthorized(_) => (403u16, "UNAUTHORIZED"),
                            _ => (429u16, "SUBSCRIBE_FAILED"),
                        };
                        let err = json_error(code, &e.to_string());
                        let response = with_security_headers(
                            Response::from_string(&err)
                                .with_status_code(status)
                                .with_header(
                                    Header::from_bytes("Content-Type", "application/json").unwrap(),
                                )
                                .with_header(
                                    Header::from_bytes(
                                        "Access-Control-Allow-Origin",
                                        cors_origin.as_bytes().to_vec(),
                                    )
                                    .unwrap(),
                                ),
                        );
                        let _ = request.respond(response);
                        mt.record_request("GET", status);
                        return;
                    }

                    // Auto-unsubscribe when the client disconnects: we watch
                    // for mpsc channel disconnection in a sentinel thread.
                    {
                        let shard_cleanup = Arc::clone(&shard);
                        let sub_id_cleanup = subscriber_id.clone();
                        let tx_liveness = tx.clone();
                        std::thread::spawn(move || {
                            // Send a heartbeat every 30s; if send fails, the
                            // channel is closed (client disconnected).
                            loop {
                                std::thread::sleep(std::time::Duration::from_secs(30));
                                if tx_liveness.send(b": heartbeat\n\n".to_vec()).is_err() {
                                    shard_cleanup.remove_subscriber(&sub_id_cleanup);
                                    return;
                                }
                                if !shard_cleanup.is_running() {
                                    return;
                                }
                            }
                        });
                    }

                    let response = with_security_headers(Response::new(
                        tiny_http::StatusCode(200),
                        vec![
                            Header::from_bytes("Content-Type", "text/event-stream").unwrap(),
                            Header::from_bytes("Cache-Control", "no-cache").unwrap(),
                            Header::from_bytes("Connection", "keep-alive").unwrap(),
                            Header::from_bytes(
                                "Access-Control-Allow-Origin",
                                cors_origin.as_bytes().to_vec(),
                            )
                            .unwrap(),
                        ],
                        streaming_body,
                        None,
                        None,
                    ));
                    let _ = request.respond(response);
                    mt.record_request("GET", 200);
                    return;
                }
            }
        }

        // --- POST /api/fn/:name with Accept: text/event-stream — streaming functions ---
        if method == Method::Post
            && url.starts_with("/api/fn/")
            && url != "/api/fn/traces"
            && request.headers().iter().any(|h| {
                (h.field.as_str() == "Accept" || h.field.as_str() == "accept")
                    && h.value.as_str().contains("text/event-stream")
            })
        {
            let fn_name = url
                .strip_prefix("/api/fn/")
                .unwrap_or("")
                .split('?')
                .next()
                .unwrap_or("")
                .to_string();

            if let Some(fn_ops) = &fn_ops_maybe {
                // Mirror the router's gates so the streaming fast path doesn't
                // become a way to bypass function auth / rate limits.
                // 1. Function must exist (otherwise 404, not a hung SSE).
                let fn_def = pylon_router::FnOps::get_fn(fn_ops.as_ref(), &fn_name);
                if fn_def.is_none() {
                    let err = json_error(
                        "FN_NOT_FOUND",
                        &format!("Function \"{fn_name}\" is not registered"),
                    );
                    let response = with_security_headers(
                        Response::from_string(&err)
                            .with_status_code(404u16)
                            .with_header(
                                Header::from_bytes("Content-Type", "application/json").unwrap(),
                            )
                            .with_header(
                                Header::from_bytes(
                                    "Access-Control-Allow-Origin",
                                    cors_origin.as_bytes().to_vec(),
                                )
                                .unwrap(),
                            ),
                    );
                    let _ = request.respond(response);
                    mt.record_request("POST", 404);
                    return;
                }
                // 1b. `internal: true` functions reachable only from
                // admin contexts. The non-streaming router path (in
                // routes/functions.rs) already enforces this; the
                // SSE fast path missed the same gate, which let any
                // caller invoke an internal function by setting
                // `Accept: text/event-stream`. Caught in the
                // 2026-05-09 codex security audit.
                //
                // Match the router's response shape exactly: 404
                // FN_NOT_FOUND, NOT 403 FN_INTERNAL. A 403 confirms
                // the function exists, letting an attacker enumerate
                // the internal-function namespace by scanning for
                // 403-vs-404. Caught in the 2026-05-10 codex pass-2
                // audit (P2 regression).
                if let Some(def) = fn_def.as_ref() {
                    if def.internal && !auth_ctx.is_admin {
                        let err = json_error(
                            "FN_NOT_FOUND",
                            &format!("Function \"{fn_name}\" is not registered"),
                        );
                        let response = with_security_headers(
                            Response::from_string(&err)
                                .with_status_code(404u16)
                                .with_header(
                                    Header::from_bytes("Content-Type", "application/json").unwrap(),
                                )
                                .with_header(
                                    Header::from_bytes(
                                        "Access-Control-Allow-Origin",
                                        cors_origin.as_bytes().to_vec(),
                                    )
                                    .unwrap(),
                                ),
                        );
                        let _ = request.respond(response);
                        mt.record_request("POST", 404);
                        return;
                    }
                }
                // 2. Per-function rate limit. Match router identity:
                // user_id when authed, peer_ip when anonymous, "anon"
                // only as a last-resort fallback. Previously the SSE
                // path used a global "anon" bucket — one bad actor
                // would rate-limit every other anonymous caller off
                // the SSE endpoint. Caught in the 2026-05-10 codex
                // pass-2 audit (P2 regression). `peer_ip` is bound
                // earlier in the request loop from `resolve_client_ip`.
                let identity = auth_ctx.user_id.as_deref().unwrap_or_else(|| {
                    if peer_ip.is_empty() {
                        "anon"
                    } else {
                        peer_ip.as_str()
                    }
                });
                if let Err(retry_after) =
                    pylon_router::FnOps::check_rate_limit(fn_ops.as_ref(), &fn_name, identity)
                {
                    let body = format!(
                        r#"{{"error":{{"code":"RATE_LIMITED","message":"Function \"{fn_name}\" rate limit exceeded","retry_after_secs":{retry_after}}}}}"#
                    );
                    let response = with_security_headers(
                        Response::from_string(&body)
                            .with_status_code(429u16)
                            .with_header(
                                Header::from_bytes("Content-Type", "application/json").unwrap(),
                            )
                            .with_header(
                                Header::from_bytes(
                                    "Access-Control-Allow-Origin",
                                    cors_origin.as_bytes().to_vec(),
                                )
                                .unwrap(),
                            ),
                    );
                    let _ = request.respond(response);
                    mt.record_request("POST", 429);
                    return;
                }

                let args: serde_json::Value =
                    serde_json::from_str(&body).unwrap_or(serde_json::json!({}));

                let auth = pylon_functions::protocol::AuthInfo {
                    user_id: auth_ctx.user_id.clone(),
                    is_admin: auth_ctx.is_admin,
                    tenant_id: auth_ctx.tenant_id.clone(),
                    roles: auth_ctx.roles.clone(),
                };

                let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
                let streaming_body = StreamingBody::new(rx);

                let fn_ops_cl = Arc::clone(fn_ops);
                let tx_stream = tx.clone();
                std::thread::spawn(move || {
                    let tx_cb = tx_stream.clone();
                    let on_stream: Box<dyn FnMut(&str) + Send> = Box::new(move |chunk: &str| {
                        let sse = format!("data: {}\n\n", chunk);
                        let _ = tx_cb.send(sse.into_bytes());
                    });

                    let result = pylon_router::FnOps::call(
                        fn_ops_cl.as_ref(),
                        &fn_name,
                        args,
                        auth,
                        Some(on_stream),
                        None, // streaming /api/fn/:name never carries HTTP request metadata
                    );
                    match result {
                        Ok((value, _trace)) => {
                            let done = format!(
                                "event: result\ndata: {}\n\n",
                                serde_json::to_string(&value).unwrap_or_else(|_| "null".into())
                            );
                            let _ = tx_stream.send(done.into_bytes());
                        }
                        Err(e) => {
                            let err = format!(
                                "event: error\ndata: {}\n\n",
                                serde_json::json!({"code": e.code, "message": e.message})
                            );
                            let _ = tx_stream.send(err.into_bytes());
                        }
                    }
                });

                let response = with_security_headers(Response::new(
                    tiny_http::StatusCode(200),
                    vec![
                        Header::from_bytes("Content-Type", "text/event-stream").unwrap(),
                        Header::from_bytes("Cache-Control", "no-cache").unwrap(),
                        Header::from_bytes("Connection", "keep-alive").unwrap(),
                        Header::from_bytes(
                            "Access-Control-Allow-Origin",
                            cors_origin.as_bytes().to_vec(),
                        )
                        .unwrap(),
                    ],
                    streaming_body,
                    None,
                    None,
                ));
                let _ = request.respond(response);
                mt.record_request("POST", 200);
                return;
            }
        }

        // --- POST /api/connections/<name>/auth-url ---
        //
        // Authed-only. Mints a CSRF state token, persists payload,
        // returns the URL the browser should navigate to. The
        // user's OAuth consent flow then redirects to /callback.
        if method == Method::Post
            && url.starts_with("/api/connections/")
            && url.ends_with("/auth-url")
        {
            if auth_ctx.user_id.is_none() {
                let err = json_error(
                    "AUTH_REQUIRED",
                    "/api/connections/*/auth-url requires an authenticated session",
                );
                let response = with_security_headers(
                    Response::from_string(&err)
                        .with_status_code(401u16)
                        .with_header(
                            Header::from_bytes("Content-Type", "application/json").unwrap(),
                        )
                        .with_header(
                            Header::from_bytes(
                                "Access-Control-Allow-Origin",
                                cors_origin.as_bytes().to_vec(),
                            )
                            .unwrap(),
                        ),
                );
                let _ = request.respond(response);
                mt.record_request("POST", 401);
                return;
            }
            let name = url
                .trim_start_matches("/api/connections/")
                .trim_end_matches("/auth-url");
            let post_redirect = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| {
                    v.get("post_redirect")
                        .and_then(|p| p.as_str())
                        .map(String::from)
                });
            let mgr = match crate::datastore::build_connection_manager(&runtime) {
                Some(m) => m,
                None => {
                    let err = json_error(
                        "CONNECTIONS_NOT_CONFIGURED",
                        "No defineConnection(...) entries in the manifest.",
                    );
                    let response = with_security_headers(
                        Response::from_string(&err)
                            .with_status_code(503u16)
                            .with_header(
                                Header::from_bytes("Content-Type", "application/json").unwrap(),
                            )
                            .with_header(
                                Header::from_bytes(
                                    "Access-Control-Allow-Origin",
                                    cors_origin.as_bytes().to_vec(),
                                )
                                .unwrap(),
                            ),
                    );
                    let _ = request.respond(response);
                    mt.record_request("POST", 503);
                    return;
                }
            };
            let result = mgr.build_auth_url(
                name,
                auth_ctx.user_id.as_deref().unwrap_or(""),
                post_redirect.as_deref(),
            );
            let (status, body) = match result {
                Ok(u) => (200u16, serde_json::json!({"url": u}).to_string()),
                Err(e) => {
                    let code = e.code();
                    let status = match code {
                        "CONNECTION_UNKNOWN" => 404,
                        "PROVIDER_NOT_CONFIGURED" => 503,
                        "ENCRYPTION_REQUIRED" => 503,
                        _ => 400,
                    };
                    (status, json_error(code, &e.to_string()))
                }
            };
            let response = with_security_headers(
                Response::from_string(&body)
                    .with_status_code(status)
                    .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
                    .with_header(
                        Header::from_bytes(
                            "Access-Control-Allow-Origin",
                            cors_origin.as_bytes().to_vec(),
                        )
                        .unwrap(),
                    ),
            );
            let _ = request.respond(response);
            mt.record_request("POST", status);
            return;
        }

        // --- GET /api/connections/<name>/callback?code=...&state=... ---
        //
        // Called by the OAuth provider after consent. No prior auth
        // session — the user might land here in an unauth'd
        // incognito window — so the state token serves as the only
        // identity binding (mapped back to user_id via the state
        // payload). On success: stores the row + redirects to the
        // app-supplied post-callback URL or `/`.
        if method == Method::Get
            && url.starts_with("/api/connections/")
            && url.contains("/callback")
        {
            let path_no_query = url.split('?').next().unwrap_or(&url);
            let name = path_no_query
                .trim_start_matches("/api/connections/")
                .trim_end_matches("/callback");
            let qs = url.split_once('?').map(|(_, q)| q).unwrap_or("");
            let mut code: Option<String> = None;
            let mut state: Option<String> = None;
            for pair in qs.split('&') {
                if let Some((k, v)) = pair.split_once('=') {
                    let decoded = percent_decode_str(v);
                    match k {
                        "code" => code = Some(decoded),
                        "state" => state = Some(decoded),
                        _ => {}
                    }
                }
            }
            let mgr = match crate::datastore::build_connection_manager(&runtime) {
                Some(m) => m,
                None => {
                    let err = json_error(
                        "CONNECTIONS_NOT_CONFIGURED",
                        "No defineConnection(...) entries in the manifest.",
                    );
                    let response = with_security_headers(
                        Response::from_string(&err)
                            .with_status_code(503u16)
                            .with_header(
                                Header::from_bytes("Content-Type", "application/json").unwrap(),
                            ),
                    );
                    let _ = request.respond(response);
                    mt.record_request("GET", 503);
                    return;
                }
            };
            let (code, state) = match (code, state) {
                (Some(c), Some(s)) => (c, s),
                _ => {
                    let err = json_error(
                        "MISSING_PARAMS",
                        "Callback requires both `code` and `state`",
                    );
                    let response = with_security_headers(
                        Response::from_string(&err)
                            .with_status_code(400u16)
                            .with_header(
                                Header::from_bytes("Content-Type", "application/json").unwrap(),
                            ),
                    );
                    let _ = request.respond(response);
                    mt.record_request("GET", 400);
                    return;
                }
            };
            let runtime_for_store = Arc::clone(&runtime);
            let _name_owned = name.to_string();
            let outcome = mgr.complete_callback(&code, &state, move |row| {
                let id = crate::connections::stable_id(&row.user_id, &row.name);
                let payload = serde_json::json!({
                    "id": id.clone(),
                    "userId": row.user_id,
                    "name": row.name,
                    "provider": row.provider,
                    "accessToken": row.access_token,
                    "refreshToken": row.refresh_token,
                    "expiresAt": row.expires_at,
                    "scope": row.scope,
                    "updatedAt": row.updated_at,
                });
                let updated = runtime_for_store
                    .update("_Connection", &id, &payload)
                    .map_err(|e| e.message)?;
                if !updated {
                    runtime_for_store
                        .insert("_Connection", &payload)
                        .map_err(|e| e.message)?;
                }
                Ok(())
            });
            let (status, location, body): (u16, Option<String>, String) = match outcome {
                Ok(o) => {
                    // Defense-in-depth: even though build_auth_url
                    // validated post_redirect before minting state,
                    // re-validate here. If the state row was tampered,
                    // refuse to honor a foreign-origin redirect.
                    let redirect = if o.post_redirect.is_empty()
                        || !crate::connections::is_safe_relative_redirect(&o.post_redirect)
                    {
                        "/".to_string()
                    } else {
                        o.post_redirect.clone()
                    };
                    (302, Some(redirect), String::new())
                }
                Err(e) => {
                    let status = match e.code() {
                        "AUTH_FAILED" => 400,
                        "CONNECTION_UNKNOWN" => 404,
                        "PROVIDER_NOT_CONFIGURED" => 503,
                        "ENCRYPTION_REQUIRED" => 503,
                        _ => 400,
                    };
                    (status, None, json_error(e.code(), &e.to_string()))
                }
            };
            let mut resp = Response::from_string(&body).with_status_code(status);
            if let Some(loc) = location.as_deref() {
                resp = resp.with_header(Header::from_bytes("Location", loc.as_bytes()).unwrap());
            }
            resp =
                resp.with_header(Header::from_bytes("Content-Type", "application/json").unwrap());
            let response = with_security_headers(resp);
            let _ = request.respond(response);
            mt.record_request("GET", status);
            return;
        }

        // --- POST /api/llm/complete — non-streaming LLM completion ---
        //
        // Anthropic-shaped JSON in/out. Supports tool_use loops via
        // content blocks. Same auth + rate-limit + model-allowlist
        // gates as /api/ai/stream — clients that already authed for
        // streaming can use this when they don't need progressive
        // output (sync agent loops, batch jobs).
        if url == "/api/llm/complete" && method == Method::Post {
            if auth_ctx.user_id.is_none() {
                let err = json_error(
                    "AUTH_REQUIRED",
                    "/api/llm/complete requires an authenticated session",
                );
                let response = with_security_headers(
                    Response::from_string(&err)
                        .with_status_code(401u16)
                        .with_header(
                            Header::from_bytes("Content-Type", "application/json").unwrap(),
                        )
                        .with_header(
                            Header::from_bytes(
                                "Access-Control-Allow-Origin",
                                cors_origin.as_bytes().to_vec(),
                            )
                            .unwrap(),
                        ),
                );
                let _ = request.respond(response);
                mt.record_request("POST", 401);
                return;
            }
            let ai_identity = auth_ctx.user_id.as_deref().unwrap_or(&peer_ip);
            if !auth_ctx.is_admin {
                if let Err(retry_after) = ai_rate_limiter.check(ai_identity) {
                    let body = format!(
                        r#"{{"error":{{"code":"RATE_LIMITED","message":"AI requests rate limit exceeded","retry_after_secs":{retry_after}}}}}"#
                    );
                    let response = with_security_headers(
                        Response::from_string(&body)
                            .with_status_code(429u16)
                            .with_header(
                                Header::from_bytes("Content-Type", "application/json").unwrap(),
                            )
                            .with_header(
                                Header::from_bytes(
                                    "Access-Control-Allow-Origin",
                                    cors_origin.as_bytes().to_vec(),
                                )
                                .unwrap(),
                            ),
                    );
                    let _ = request.respond(response);
                    mt.record_request("POST", 429);
                    return;
                }
            }
            let client = match llm_client_route.clone() {
                Some(c) => c,
                None => {
                    let err = json_error(
                        "LLM_NOT_CONFIGURED",
                        "Set PYLON_LLM_PROVIDER + ANTHROPIC_API_KEY (or OPENAI_API_KEY)",
                    );
                    let response = with_security_headers(
                        Response::from_string(&err)
                            .with_status_code(503u16)
                            .with_header(
                                Header::from_bytes("Content-Type", "application/json").unwrap(),
                            )
                            .with_header(
                                Header::from_bytes(
                                    "Access-Control-Allow-Origin",
                                    cors_origin.as_bytes().to_vec(),
                                )
                                .unwrap(),
                            ),
                    );
                    let _ = request.respond(response);
                    mt.record_request("POST", 503);
                    return;
                }
            };
            let parsed: serde_json::Value = match serde_json::from_str(&body) {
                Ok(v) => v,
                Err(_) => {
                    let err = json_error("INVALID_JSON", "Invalid request body");
                    let response = with_security_headers(
                        Response::from_string(&err)
                            .with_status_code(400u16)
                            .with_header(
                                Header::from_bytes("Content-Type", "application/json").unwrap(),
                            )
                            .with_header(
                                Header::from_bytes(
                                    "Access-Control-Allow-Origin",
                                    cors_origin.as_bytes().to_vec(),
                                )
                                .unwrap(),
                            ),
                    );
                    let _ = request.respond(response);
                    mt.record_request("POST", 400);
                    return;
                }
            };
            // Model-allowlist gate — env + manifest merged (codex P1-1).
            if let Some(req_model) = parsed.get("model").and_then(|m| m.as_str()) {
                if !req_model.is_empty() && !auth_ctx.is_admin {
                    let env_allowed = std::env::var("PYLON_AI_MODELS_ALLOWED").unwrap_or_default();
                    let mut allowed_set: std::collections::HashSet<String> = env_allowed
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    for m in client.manifest_allowed_models() {
                        allowed_set.insert(m.clone());
                    }
                    if allowed_set.is_empty() {
                        let err = json_error(
                            "MODEL_OVERRIDE_FORBIDDEN",
                            "Client model override requires PYLON_AI_MODELS_ALLOWED env or llm({ allowedModels: [...] }) in the manifest",
                        );
                        let response = with_security_headers(
                            Response::from_string(&err)
                                .with_status_code(403u16)
                                .with_header(
                                    Header::from_bytes("Content-Type", "application/json").unwrap(),
                                )
                                .with_header(
                                    Header::from_bytes(
                                        "Access-Control-Allow-Origin",
                                        cors_origin.as_bytes().to_vec(),
                                    )
                                    .unwrap(),
                                ),
                        );
                        let _ = request.respond(response);
                        mt.record_request("POST", 403);
                        return;
                    }
                    if !allowed_set.contains(req_model) {
                        let err = json_error(
                            "MODEL_NOT_ALLOWED",
                            &format!("Model \"{req_model}\" is not in the allowlist"),
                        );
                        let response = with_security_headers(
                            Response::from_string(&err)
                                .with_status_code(403u16)
                                .with_header(
                                    Header::from_bytes("Content-Type", "application/json").unwrap(),
                                )
                                .with_header(
                                    Header::from_bytes(
                                        "Access-Control-Allow-Origin",
                                        cors_origin.as_bytes().to_vec(),
                                    )
                                    .unwrap(),
                                ),
                        );
                        let _ = request.respond(response);
                        mt.record_request("POST", 403);
                        return;
                    }
                }
            }
            let req_obj: crate::llm::LlmCompleteRequest = match serde_json::from_value(parsed) {
                Ok(r) => r,
                Err(e) => {
                    let err = json_error(
                        "INVALID_REQUEST",
                        &format!("Failed to parse LLM request: {e}"),
                    );
                    let response = with_security_headers(
                        Response::from_string(&err)
                            .with_status_code(400u16)
                            .with_header(
                                Header::from_bytes("Content-Type", "application/json").unwrap(),
                            )
                            .with_header(
                                Header::from_bytes(
                                    "Access-Control-Allow-Origin",
                                    cors_origin.as_bytes().to_vec(),
                                )
                                .unwrap(),
                            ),
                    );
                    let _ = request.respond(response);
                    mt.record_request("POST", 400);
                    return;
                }
            };
            let (status, body) = match client.complete(req_obj) {
                Ok(resp) => (200u16, serde_json::to_string(&resp).unwrap_or_default()),
                Err(e) => {
                    // Codex P2-E: a provider 401 must NOT propagate as
                    // 401 to the caller — they'd assume their auth is
                    // wrong, not the server's API key. All upstream
                    // errors map to 502 Bad Gateway (or 504 on
                    // timeouts) with the typed code preserved in the
                    // JSON body. Caller still gets actionable signal.
                    // Codex P1-M: caller never sees the redacted
                    // provider body — that stays in server logs.
                    let status_code = if e.code == "PROVIDER_UNREACHABLE" {
                        504
                    } else if e.code.starts_with("PROVIDER_HTTP_") {
                        502
                    } else {
                        500
                    };
                    tracing::warn!(
                        "[llm] /api/llm/complete upstream error code={} detail={}",
                        e.code,
                        e.message
                    );
                    (
                        status_code,
                        json_error(&e.code, "Upstream LLM provider returned an error"),
                    )
                }
            };
            let response = with_security_headers(
                Response::from_string(&body)
                    .with_status_code(status)
                    .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
                    .with_header(
                        Header::from_bytes(
                            "Access-Control-Allow-Origin",
                            cors_origin.as_bytes().to_vec(),
                        )
                        .unwrap(),
                    ),
            );
            let _ = request.respond(response);
            mt.record_request("POST", status);
            return;
        }

        // --- POST /api/ai/stream — SSE streaming AI completion ---
        if url == "/api/ai/stream" && method == Method::Post {
            // AI endpoints spend real money per call. Require auth so a
            // drive-by caller can't burn through the provider budget.
            if auth_ctx.user_id.is_none() {
                let err = json_error(
                    "AUTH_REQUIRED",
                    "/api/ai/stream requires an authenticated session",
                );
                let response = with_security_headers(
                    Response::from_string(&err)
                        .with_status_code(401u16)
                        .with_header(
                            Header::from_bytes("Content-Type", "application/json").unwrap(),
                        )
                        .with_header(
                            Header::from_bytes(
                                "Access-Control-Allow-Origin",
                                cors_origin.as_bytes().to_vec(),
                            )
                            .unwrap(),
                        ),
                );
                let _ = request.respond(response);
                mt.record_request("POST", 401);
                return;
            }
            // Per-user rate limit. Default 30/hour caps a runaway client
            // (or compromised session) at ~$5/day on typical pricing.
            // Admins skip the limit so internal tooling isn't blocked.
            let ai_identity = auth_ctx.user_id.as_deref().unwrap_or(&peer_ip);
            if !auth_ctx.is_admin {
                if let Err(retry_after) = ai_rate_limiter.check(ai_identity) {
                    let body = format!(
                        r#"{{"error":{{"code":"RATE_LIMITED","message":"AI requests rate limit exceeded","retry_after_secs":{retry_after}}}}}"#
                    );
                    let response = with_security_headers(
                        Response::from_string(&body)
                            .with_status_code(429u16)
                            .with_header(
                                Header::from_bytes("Content-Type", "application/json").unwrap(),
                            )
                            .with_header(
                                Header::from_bytes(
                                    "Access-Control-Allow-Origin",
                                    cors_origin.as_bytes().to_vec(),
                                )
                                .unwrap(),
                            ),
                    );
                    let _ = request.respond(response);
                    mt.record_request("POST", 429);
                    return;
                }
            }
            let ai_provider = std::env::var("PYLON_AI_PROVIDER").unwrap_or_default();
            let ai_key = std::env::var("PYLON_AI_API_KEY").unwrap_or_default();
            let ai_model = std::env::var("PYLON_AI_MODEL").unwrap_or_default();
            let ai_base = std::env::var("PYLON_AI_BASE_URL").unwrap_or_default();

            if ai_key.is_empty() && ai_provider != "custom" {
                let err = json_error(
                    "AI_NOT_CONFIGURED",
                    "Set PYLON_AI_PROVIDER and PYLON_AI_API_KEY",
                );
                let response = with_security_headers(
                    Response::from_string(&err)
                        .with_status_code(503u16)
                        .with_header(
                            Header::from_bytes("Content-Type", "application/json").unwrap(),
                        )
                        .with_header(
                            Header::from_bytes(
                                "Access-Control-Allow-Origin",
                                cors_origin.as_bytes().to_vec(),
                            )
                            .unwrap(),
                        ),
                );
                let _ = request.respond(response);
                mt.record_request("POST", 503);
                return;
            }

            let parsed: serde_json::Value = match serde_json::from_str(&body) {
                Ok(v) => v,
                Err(_) => {
                    let err = json_error("INVALID_JSON", "Invalid request body");
                    let response = with_security_headers(
                        Response::from_string(&err)
                            .with_status_code(400u16)
                            .with_header(
                                Header::from_bytes("Content-Type", "application/json").unwrap(),
                            )
                            .with_header(
                                Header::from_bytes(
                                    "Access-Control-Allow-Origin",
                                    cors_origin.as_bytes().to_vec(),
                                )
                                .unwrap(),
                            ),
                    );
                    let _ = request.respond(response);
                    mt.record_request("POST", 400);
                    return;
                }
            };

            let messages: Vec<AiMessage> = match parsed.get("messages").and_then(|m| m.as_array()) {
                Some(arr) => arr
                    .iter()
                    .filter_map(|m| {
                        let role = m.get("role")?.as_str()?.to_string();
                        let content = m.get("content")?.as_str()?.to_string();
                        Some(AiMessage { role, content })
                    })
                    .collect(),
                None => {
                    let err = json_error("MISSING_FIELD", "\"messages\" array is required");
                    let response = with_security_headers(
                        Response::from_string(&err)
                            .with_status_code(400u16)
                            .with_header(
                                Header::from_bytes("Content-Type", "application/json").unwrap(),
                            )
                            .with_header(
                                Header::from_bytes(
                                    "Access-Control-Allow-Origin",
                                    cors_origin.as_bytes().to_vec(),
                                )
                                .unwrap(),
                            ),
                    );
                    let _ = request.respond(response);
                    mt.record_request("POST", 400);
                    return;
                }
            };

            // Model selection. Operators set PYLON_AI_MODEL as the
            // default; PYLON_AI_MODELS_ALLOWED (comma-separated) opts
            // into client-supplied overrides. Without the allowlist,
            // a logged-in user could request the most expensive model
            // available to the API key — caught in the 2026-05-10
            // codex pass-3 audit (P2 NEW). Admins skip the gate so
            // internal tooling can target any model.
            let requested_model = parsed
                .get("model")
                .and_then(|m| m.as_str())
                .map(|s| s.to_string());
            let model = match requested_model {
                Some(req) if !auth_ctx.is_admin => {
                    let allowed = std::env::var("PYLON_AI_MODELS_ALLOWED").unwrap_or_default();
                    if allowed.is_empty() {
                        // No allowlist — refuse the override.
                        let err = json_error(
                            "MODEL_OVERRIDE_FORBIDDEN",
                            "Client model override requires PYLON_AI_MODELS_ALLOWED to be set; using server default",
                        );
                        let response = with_security_headers(
                            Response::from_string(&err)
                                .with_status_code(403u16)
                                .with_header(
                                    Header::from_bytes("Content-Type", "application/json").unwrap(),
                                )
                                .with_header(
                                    Header::from_bytes(
                                        "Access-Control-Allow-Origin",
                                        cors_origin.as_bytes().to_vec(),
                                    )
                                    .unwrap(),
                                ),
                        );
                        let _ = request.respond(response);
                        mt.record_request("POST", 403);
                        return;
                    }
                    let allowed_set: std::collections::HashSet<&str> = allowed
                        .split(',')
                        .map(|s| s.trim())
                        .filter(|s| !s.is_empty())
                        .collect();
                    if !allowed_set.contains(req.as_str()) {
                        let err = json_error(
                            "MODEL_NOT_ALLOWED",
                            &format!("Model \"{req}\" is not in PYLON_AI_MODELS_ALLOWED"),
                        );
                        let response = with_security_headers(
                            Response::from_string(&err)
                                .with_status_code(403u16)
                                .with_header(
                                    Header::from_bytes("Content-Type", "application/json").unwrap(),
                                )
                                .with_header(
                                    Header::from_bytes(
                                        "Access-Control-Allow-Origin",
                                        cors_origin.as_bytes().to_vec(),
                                    )
                                    .unwrap(),
                                ),
                        );
                        let _ = request.respond(response);
                        mt.record_request("POST", 403);
                        return;
                    }
                    req
                }
                Some(req) => req, // admin override
                None => ai_model,
            };

            let proxy = match ai_provider.as_str() {
                "anthropic" => AiProxyPlugin::anthropic(&ai_key, &model),
                "openai" => AiProxyPlugin::openai(&ai_key, &model),
                "custom" => AiProxyPlugin::custom_with_model(&ai_base, &ai_key, &model),
                _ => AiProxyPlugin::openai(&ai_key, &model),
            };

            // Set up a channel-based streaming body so tiny_http streams
            // data to the client as chunks arrive from the AI provider.
            let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
            let streaming_body = StreamingBody::new(rx);

            // Spawn the provider request on a background thread. Each chunk
            // is formatted as an SSE event and pushed through the channel.
            std::thread::spawn(move || {
                let result = proxy.stream_completion(&messages, &mut |chunk| {
                    let sse = format!(
                        "data: {}

",
                        serde_json::json!({
                            "choices": [{"index": 0, "delta": {"content": chunk}}]
                        })
                    );
                    let _ = tx.send(sse.into_bytes());
                });

                // Send a final event indicating completion or error.
                match result {
                    Ok(_) => {
                        let _ = tx.send(
                            b"data: [DONE]

"
                            .to_vec(),
                        );
                    }
                    Err(e) => {
                        let err_event = format!(
                            "data: {}

",
                            serde_json::json!({"error": {"message": e, "type": "stream_error"}})
                        );
                        let _ = tx.send(err_event.into_bytes());
                    }
                }
                // tx is dropped here, which causes StreamingBody::read to return 0 (EOF).
            });

            let response = with_security_headers(Response::new(
                tiny_http::StatusCode(200),
                vec![
                    Header::from_bytes("Content-Type", "text/event-stream").unwrap(),
                    Header::from_bytes("Cache-Control", "no-cache").unwrap(),
                    Header::from_bytes("Connection", "keep-alive").unwrap(),
                    Header::from_bytes(
                        "Access-Control-Allow-Origin",
                        cors_origin.as_bytes().to_vec(),
                    )
                    .unwrap(),
                ],
                streaming_body,
                None, // unknown content length = chunked transfer
                None,
            ));
            let _ = request.respond(response);
            mt.record_request("POST", 200);
            return;
        }

        // Studio route (returns HTML, not JSON).
        //
        // Privileged admin UI. It renders the full schema and lets the
        // operator run mutations against the data browser. In production we
        // require an admin token; in dev mode we leave it open so
        // `pylon dev` remains friction-free for the single-user case.
        //
        // Serving a WWW-Authenticate Basic realm isn't useful here because
        // admin auth is bearer-token based. Callers get a 401 and should
        // retry with `Authorization: Bearer <PYLON_ADMIN_TOKEN>`.
        // Studio login form. Browsers can't easily set Authorization
        // headers, so this is the path users hit when they visit
        // /studio without admin auth — the 401 below redirects here.
        // POST takes the token, sets the admin cookie, redirects to
        // /studio. GET renders the form.
        if url == "/studio/login" && method == Method::Get {
            let html = studio_login_html(None);
            let response = with_security_headers(
                Response::from_string(html)
                    .with_status_code(200u16)
                    .with_header(
                        Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap(),
                    ),
            );
            let _ = request.respond(response);
            mt.record_request("GET", 200);
            return;
        }
        if url == "/studio/login" && method == Method::Post {
            let mut body_bytes = Vec::new();
            let _ = request.as_reader().read_to_end(&mut body_bytes);
            let body_str = String::from_utf8_lossy(&body_bytes);
            // form is `token=<value>` URL-encoded
            let submitted = body_str
                .split('&')
                .filter_map(|p| p.split_once('='))
                .find(|(k, _)| *k == "token")
                .map(|(_, v)| {
                    // URL-decode the basic case (+ and %xx). The token
                    // is base64url-ish so + isn't really expected, but
                    // safe to handle.
                    let decoded = v.replace('+', " ");
                    percent_decode_str(&decoded)
                })
                .unwrap_or_default();
            let admin = admin_token.as_deref().unwrap_or("");
            if admin.is_empty() {
                let html = studio_login_html(Some(
                    "Studio is not configured for admin auth (PYLON_ADMIN_TOKEN unset on this Pylon).",
                ));
                let response = with_security_headers(
                    Response::from_string(html)
                        .with_status_code(503u16)
                        .with_header(
                            Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap(),
                        ),
                );
                let _ = request.respond(response);
                mt.record_request("POST", 503);
                return;
            }
            if !pylon_auth::constant_time_eq(submitted.as_bytes(), admin.as_bytes()) {
                let html = studio_login_html(Some("Invalid admin token."));
                let response = with_security_headers(
                    Response::from_string(html)
                        .with_status_code(401u16)
                        .with_header(
                            Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap(),
                        ),
                );
                let _ = request.respond(response);
                mt.record_request("POST", 401);
                return;
            }
            // Token verified. Set the admin cookie + redirect.
            let admin_cookie = format!(
                "{}={}; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=28800",
                admin_cookie_name, submitted,
            );
            let response = with_security_headers(
                Response::from_string("")
                    .with_status_code(303u16)
                    .with_header(Header::from_bytes("Location", "/studio").unwrap())
                    .with_header(Header::from_bytes("Set-Cookie", admin_cookie).unwrap()),
            );
            let _ = request.respond(response);
            mt.record_request("POST", 303);
            return;
        }
        if url == "/studio/logout" && (method == Method::Get || method == Method::Post) {
            let cleared = format!(
                "{}=; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=0",
                admin_cookie_name,
            );
            // Send the user back to wherever they'd go to sign in.
            // For cookie-authed cloud users this is the host's
            // /login page — they need to sign in as a different
            // (admin) account, not paste an admin token.
            let target = rt
                .studio_config()
                .login_url
                .clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "/studio/login".to_string());
            let response = with_security_headers(
                Response::from_string("")
                    .with_status_code(303u16)
                    .with_header(
                        Header::from_bytes("Location", target.as_bytes().to_vec()).unwrap(),
                    )
                    .with_header(Header::from_bytes("Set-Cookie", cleared).unwrap()),
            );
            let _ = request.respond(response);
            mt.record_request("GET", 303);
            return;
        }

        // HEAD /studio: respond 200 with empty body. Health checks
        // (Fly probes, uptime monitors, Vercel rewrite warmups) commonly
        // probe with HEAD; without this they get 404 and the dashboard
        // metrics show a phantom error rate. Skipping the full studio
        // HTML generation keeps the probe path cheap, and per RFC 7231
        // a HEAD response MUST omit the body anyway.
        if (url == "/studio" || url == "/studio/") && method == Method::Head {
            let response = with_security_headers(
                Response::from_string("")
                    .with_status_code(200u16)
                    .with_header(
                        Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap(),
                    ),
            );
            let _ = request.respond(response);
            mt.record_request("HEAD", 200);
            return;
        }

        let (status, response_body, content_type, is_studio, extra_headers) = if (url == "/studio"
            || url == "/studio/")
            && method == Method::Get
        {
            // Three-state Studio gate. The React bundle ships the full
            // manifest (entity names, function names, policy names) at
            // build time; serving it to anyone but admins leaks the
            // entire data-model shape, AND drops the user into a dead
            // "paste your admin token" dialog that doesn't match how
            // most production apps actually authenticate operators.
            //
            // Cases:
            //   1. is_admin: serve the studio HTML (bundle loads
            //      normally; React calls /api/auth/me and renders
            //      admin tabs).
            //   2. authed-but-not-admin: serve a small "access denied"
            //      HTML page with a logout link. No point sending them
            //      back to a login they're already past.
            //   3. anonymous: 303 redirect. Prefer the app's
            //      `studio.config.ts -> loginUrl` (e.g. Pylon Cloud's
            //      `/login` dashboard page) so users land on the real
            //      email/password form and the existing session cookie
            //      lifts them via `auth.user.adminField` on the way
            //      back. Falls back to `/studio/login` (the built-in
            //      admin-token form) for stand-alone Pylon apps.
            //
            // Dev mode is exempt — `pylon dev` is single-user local and
            // pre-gating here is just friction.
            if !is_dev && !auth_ctx.is_admin {
                let studio_cfg = rt.studio_config();
                if auth_ctx.user_id.is_some() {
                    // Authed but not admin. Render a static page so the
                    // user understands they can't escalate by reloading
                    // — they need a different account.
                    let html = studio_no_access_html();
                    let response = with_security_headers(
                        Response::from_string(html)
                            .with_status_code(403u16)
                            .with_header(
                                Header::from_bytes("Content-Type", "text/html; charset=utf-8")
                                    .unwrap(),
                            ),
                    );
                    let _ = request.respond(response);
                    mt.record_request("GET", 403);
                    return;
                }
                let target = studio_cfg
                    .login_url
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .map(|s| {
                        // Append ?next=/studio so the host app can
                        // bounce back. Keep it dumb — no query parsing
                        // because most app login URLs don't already
                        // carry a query.
                        let sep = if s.contains('?') { '&' } else { '?' };
                        format!("{s}{sep}next=/studio")
                    })
                    .unwrap_or_else(|| "/studio/login".to_string());
                let response = with_security_headers(
                    Response::from_string("")
                        .with_status_code(303u16)
                        .with_header(
                            Header::from_bytes("Location", target.as_bytes().to_vec()).unwrap(),
                        ),
                );
                let _ = request.respond(response);
                mt.record_request("GET", 303);
                return;
            }

            // Derive the public base URL from the request's headers.
            // Prefer X-Forwarded-Host over Host because reverse proxies
            // (Vercel external rewrites, Cloudflare Workers, ALBs) may
            // pass the upstream host in Host while preserving the
            // original public host in X-Forwarded-Host. Without this,
            // a Pylon backend behind cloud.example.com → api.example.com
            // serves the studio bundle wired to api.example.com — the
            // browser then makes cross-origin fetches that get killed
            // by CSP AND drop the user's same-origin session cookie.
            // Same fix logic for the scheme.
            let mut x_fwd_host: Option<String> = None;
            let mut req_host: Option<String> = None;
            let mut x_fwd_proto: Option<String> = None;
            for h in request.headers().iter() {
                if h.field.equiv("X-Forwarded-Host") {
                    x_fwd_host = Some(h.value.as_str().to_string());
                } else if h.field.equiv("Host") {
                    req_host = Some(h.value.as_str().to_string());
                } else if h.field.equiv("X-Forwarded-Proto") {
                    x_fwd_proto = Some(h.value.as_str().to_string());
                }
            }
            let host = x_fwd_host
                .or(req_host)
                .unwrap_or_else(|| format!("localhost:{port}"));
            let scheme = x_fwd_proto.unwrap_or_else(|| {
                if host.starts_with("localhost") {
                    "http".to_string()
                } else {
                    "https".to_string()
                }
            });
            let base = format!("{scheme}://{host}");
            let studio_cfg = rt.studio_config();
            let html = pylon_studio_api::generate_studio_html(rt.manifest(), &studio_cfg, &base);
            (
                200u16,
                html,
                "text/html",
                true,
                Vec::<(String, String)>::new(),
            )
        } else if url == "/studio/extensions.js" && method == Method::Get {
            // Bundled `studio.entry.tsx` — produced by the CLI's
            // `bun build` pass. Same admin gate as /studio in
            // production (the bundle can carry custom React components
            // that introspect the live API surface — admin-only).
            if !is_dev && !auth_ctx.is_admin {
                let body = json_error(
                    "AUTH_REQUIRED",
                    "/studio/extensions.js requires admin auth in production",
                );
                let response = with_security_headers(
                    Response::from_string(&body)
                        .with_status_code(401u16)
                        .with_header(
                            Header::from_bytes("Content-Type", "application/json").unwrap(),
                        )
                        .with_header(
                            Header::from_bytes(
                                "Access-Control-Allow-Origin",
                                cors_origin.as_bytes().to_vec(),
                            )
                            .unwrap(),
                        ),
                );
                let _ = request.respond(response);
                mt.record_request("GET", 401);
                return;
            }
            match rt.studio_entry_bytes() {
                Some(bytes) => {
                    let body = String::from_utf8_lossy(&bytes).to_string();
                    (
                        200u16,
                        body,
                        "application/javascript",
                        true,
                        Vec::<(String, String)>::new(),
                    )
                }
                None => (
                    404u16,
                    json_error(
                        "STUDIO_EXT_NOT_FOUND",
                        "No studio.entry.tsx bundle is configured for this project.",
                    ),
                    "application/json",
                    false,
                    Vec::new(),
                ),
            }
        } else {
            // Run plugin middleware with per-request metadata so rate-limit
            // plugins can bucket by peer IP (not just user id) when the
            // caller is anonymous.
            let meta = pylon_plugin::RequestMeta {
                peer_ip: peer_ip.as_str(),
            };
            if let Err(e) = pr.run_on_request_with_meta(method.as_str(), &url, &auth_ctx, &meta) {
                (
                    e.status,
                    json_error(&e.code, &e.message),
                    "application/json",
                    false,
                    Vec::new(),
                )
            } else if let Some((s, b)) =
                pr.try_handle_route(method.as_str(), &url, &body, &auth_ctx)
            {
                // Plugin handled the route.
                (s, b, "application/json", false, Vec::new())
            } else {
                let notifier = WsSseNotifier::with_cluster_bus(
                    Arc::clone(&wh),
                    Arc::clone(&sh),
                    rt.manifest().auth.user.clone(),
                    Arc::clone(&cb),
                )
                .with_reactive(Arc::clone(&rr))
                .with_policy(Arc::clone(&pe));
                let _ = &sm; // sm is plumbed through for future use; hubs own projection now
                let openapi_gen = RuntimeOpenApiGenerator {
                    manifest: rt.manifest(),
                };
                let file_ops = LocalFileOps::new_default();
                let cache_adapter = CacheAdapter(Arc::clone(&ca));
                let pubsub_adapter = PubSubAdapter(Arc::clone(&ps));
                let email_adapter = EmailAdapter::from_env();
                let fn_ops: Option<&dyn pylon_router::FnOps> =
                    fn_ops_ref.as_deref().map(|f| f as &dyn pylon_router::FnOps);
                let shard_adapter = shards_ref.as_ref().map(|reg| ShardOpsAdapter {
                    registry: Arc::clone(reg),
                });
                let shard_ops: Option<&dyn pylon_router::ShardOps> = shard_adapter
                    .as_ref()
                    .map(|a| a as &dyn pylon_router::ShardOps);
                let plugin_hooks = PluginHooksAdapter(Arc::clone(&pr));
                // Snapshot request headers as (name, value) pairs for the
                // router to forward into webhook-invoked actions. Header
                // names are left as-sent; the router lowercases + merges
                // duplicates per RFC 7230 when constructing RequestInfo.
                let request_headers: Vec<(String, String)> = request
                    .headers()
                    .iter()
                    .map(|h| (h.field.as_str().to_string(), h.value.as_str().to_string()))
                    .collect();
                let router_ctx = pylon_router::RouterContext {
                    store: rt.as_ref(),
                    session_store: &ss,
                    magic_codes: &mc,
                    oauth_state: &os,
                    account_store: &acc,
                    api_keys: &ak,
                    orgs: &og,
                    siwe: &sw,
                    phone_codes: &pcd,
                    passkeys: &pks,
                    verification: &vrf,
                    audit: &aud,
                    trusted_devices: td.as_ref(),
                    org_sso: osso.as_ref(),
                    saml: sml.as_ref(),
                    policy_engine: &pe,
                    change_log: &cl,
                    notifier: &notifier,
                    rooms: rm.as_ref(),
                    cache: &cache_adapter,
                    pubsub: &pubsub_adapter,
                    jobs: jq.as_ref(),
                    scheduler: sc.as_ref(),
                    workflows: we.as_ref(),
                    files: &file_ops,
                    openapi: &openapi_gen,
                    functions: fn_ops,
                    email: &email_adapter,
                    shards: shard_ops,
                    plugin_hooks: &plugin_hooks,
                    auth_ctx: &auth_ctx,
                    trusted_origins: &trusted_origins_ref,
                    is_dev,
                    request_headers: &request_headers,
                    peer_ip: peer_ip.as_str(),
                    cookie_config: cookie_config.as_ref(),
                    response_headers: std::cell::RefCell::new(Vec::new()),
                };
                let http_method = HttpMethod::from_str(method.as_str());
                let (s, b, _ct) = pylon_router::route(
                    &router_ctx,
                    http_method,
                    &url,
                    &body,
                    auth_token.as_deref(),
                );
                let extra_headers = router_ctx.take_response_headers();
                (s, b, "application/json", false, extra_headers)
            }
        };

        // Stamp bytes_out for the shipper's per-request rollup. This is
        // the central path where every router response body becomes a
        // wire frame, so capturing the length here covers entity CRUD,
        // /api/fn/*, /api/sync/*, /api/auth/* — every JSON-shaped
        // response. The streaming/upload paths (/api/files/*, SSE) have
        // their own respond sites; they'll stay at 0 until separately
        // stamped, which is an acceptable known undercount for now.
        crate::metrics::set_current_response_bytes(response_body.len());
        let mut response = Response::from_string(&response_body)
            .with_status_code(status)
            .with_header(Header::from_bytes("Content-Type", content_type).unwrap())
            .with_header(
                Header::from_bytes(
                    "Access-Control-Allow-Origin",
                    cors_origin.as_bytes().to_vec(),
                )
                .unwrap(),
            )
            .with_header(
                Header::from_bytes(
                    "Access-Control-Allow-Methods",
                    "GET, POST, PATCH, DELETE, OPTIONS",
                )
                .unwrap(),
            )
            .with_header(
                Header::from_bytes(
                    "Access-Control-Allow-Headers",
                    "Content-Type, Authorization",
                )
                .unwrap(),
            )
            .with_header(
                Header::from_bytes(
                    "Access-Control-Expose-Headers",
                    // X-Pylon-Change-Seq carries the post-write change-log
                    // seq number on every mutating response. The SDK reads
                    // it (across-origin via this expose allow-list) and
                    // triggers an immediate pull when its local cursor is
                    // behind, killing the latency window between an action
                    // HTTP response landing and the WS broadcast of the
                    // same events arriving. Without expose-headers the
                    // browser strips the header on cross-origin reads.
                    "X-Pylon-Change-Seq",
                )
                .unwrap(),
            );
        // Cookie-based auth requires `Access-Control-Allow-Credentials:
        // true` on the response, paired with a specific origin. Vary
        // ensures intermediaries don't cache one origin's response and
        // serve it back to a different origin's browser.
        if allow_credentials {
            response = response
                .with_header(
                    Header::from_bytes("Access-Control-Allow-Credentials", "true").unwrap(),
                )
                .with_header(Header::from_bytes("Vary", "Origin").unwrap());
        }

        // Apply any extra headers handlers attached via the router context
        // (Set-Cookie on login/logout, Location on OAuth GET callback).
        // Bytes from these headers come from server-built strings — bad
        // bytes here would be a programming bug, not request-driven, so a
        // failed Header::from_bytes is silently dropped rather than
        // poisoning the response.
        for (name, value) in extra_headers {
            if let Ok(h) = Header::from_bytes(name.as_bytes(), value.as_bytes().to_vec()) {
                response = response.with_header(h);
            }
        }

        // Add Content-Security-Policy for Studio HTML responses.
        //
        // Studio talks to the same Rust process over HTTP (same origin)
        // AND a sibling WebSocket port (port+1, scheme ws:). CSP's
        // `default-src` covers `connect-src` by fallback, so any
        // directive we set there must include the WS scheme or the
        // browser silently blocks the live-sync connection.
        //
        // `ws:` + `wss:` cover localhost dev + TLS deploys without
        // hard-coding ports. Same-origin `'self'` keeps HTTP fetches
        // allowed. Inline + eval stay for the Tailwind/Babel CDN scripts
        // the current Studio HTML includes.
        if is_studio {
            response = response.with_header(
                Header::from_bytes(
                    "Content-Security-Policy",
                    "default-src 'self' 'unsafe-inline' 'unsafe-eval' https://cdn.tailwindcss.com https://unpkg.com ws: wss:",
                ).unwrap(),
            );
        }

        let response = with_security_headers(response);

        let _ = request.respond(response);
        mt.record_request(method.as_str(), status);
            }); // end of thread::spawn closure
    }

    tracing::warn!("Shutting down gracefully...");

    // --- Drain phase ---
    // Stop accepting new work, let in-flight finish, close subsystems cleanly.
    let drain_timeout = std::time::Duration::from_secs(
        std::env::var("PYLON_DRAIN_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(10),
    );
    let start = Instant::now();

    // Stop any running shards so their tick loops exit.
    if let Some(reg) = &shard_registry {
        for id in reg.ids() {
            if let Some(shard) = reg.get(&id) {
                shard.stop();
            }
        }
    }

    // Let the scheduler finish its current cycle.
    let _ = &scheduler; // drop Arc at end of scope

    // Wait for outstanding workers to idle, up to drain_timeout.
    while start.elapsed() < drain_timeout {
        let pending_jobs = job_queue.stats().pending;
        if pending_jobs == 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    let elapsed = start.elapsed();
    tracing::warn!(
        "Drain complete in {:.1}s (timeout {}s)",
        elapsed.as_secs_f32(),
        drain_timeout.as_secs()
    );
    Ok(())
}

// The route() function has been extracted to the `pylon-router` crate.
// See `pylon_router::route()` for the platform-agnostic routing logic.
// The server now delegates to it via a `RouterContext`.

fn json_error(code: &str, message: &str) -> String {
    pylon_router::json_error(code, message)
}

/// Construct the cluster bus from env.
///
/// Resolution order:
///  - `PYLON_CLUSTER_BUS=redis://...` → [`pylon_cluster::RedisBus`].
///  - unset or empty → [`pylon_cluster::NoopBus`].
///
/// `PYLON_CLUSTER_NAMESPACE` optionally prefixes the Redis channel —
/// set this when multiple unrelated pylon deploys share a Redis
/// instance (otherwise their mutations cross-talk and every machine
/// rebroadcasts every other deploy's events).
///
/// Connection failures are fatal at boot — pylon refuses to start if
/// you ask for cluster fanout but Redis is unreachable. Better to
/// surface the misconfiguration loudly than to silently degrade to
/// single-machine mode where peer machines stay deaf.
fn build_cluster_bus() -> Arc<dyn pylon_cluster::ClusterBus> {
    let url = std::env::var("PYLON_CLUSTER_BUS").unwrap_or_default();
    if url.is_empty() {
        tracing::info!(
            "[cluster] PYLON_CLUSTER_BUS unset — running with single-machine fanout (NoopBus)"
        );
        return Arc::new(pylon_cluster::NoopBus::new());
    }
    let namespace = std::env::var("PYLON_CLUSTER_NAMESPACE").ok();
    if url.starts_with("redis://") || url.starts_with("rediss://") {
        match pylon_cluster::RedisBus::connect(&url, namespace.as_deref()) {
            Ok(bus) => Arc::new(bus),
            Err(e) => {
                tracing::error!(
                    "[cluster] PYLON_CLUSTER_BUS=\"{url}\" failed to connect: {e}. \
                     Refusing to boot — multi-machine deploys MUST have a working bus, \
                     and falling back to NoopBus would silently break cross-machine sync. \
                     Either fix the URL or unset PYLON_CLUSTER_BUS to run single-machine."
                );
                std::process::exit(1);
            }
        }
    } else {
        tracing::error!(
            "[cluster] PYLON_CLUSTER_BUS=\"{url}\" — only redis:// and rediss:// URLs are supported. \
             Refusing to boot."
        );
        std::process::exit(1);
    }
}

/// Verify a request carries one of: PYLON_ADMIN_TOKEN bearer,
/// PYLON_METRICS_TOKEN bearer, or a session cookie resolving to an
/// admin user (per `auth.user.adminField` OR PYLON_ADMIN_EMAILS
/// allowlist + emailVerified).
///
/// Returns `true` if authorized. The three paths are the same set
/// `/metrics` accepts: this helper exists so other read-only admin
/// surfaces (e.g. `/admin/logs/tail`) can opt into the same set
/// without re-implementing the auth shape and drifting from it.
///
/// Dev-mode bypass is the caller's responsibility — this helper
/// always enforces, so test harnesses that want open access in dev
/// must check `is_dev` before calling.
fn verify_admin_or_metrics_auth(
    request: &tiny_http::Request,
    admin_token: Option<&str>,
    cookie_config: &pylon_auth::CookieConfig,
    session_store: &pylon_auth::SessionStore,
    runtime: &Runtime,
) -> bool {
    let admin_bytes = admin_token.unwrap_or("").as_bytes();
    let metrics_token_owned = std::env::var("PYLON_METRICS_TOKEN")
        .ok()
        .unwrap_or_default();
    let metrics_bytes = metrics_token_owned.as_bytes();
    let bearer_ok = request.headers().iter().any(|h| {
        let name = h.field.as_str().as_str();
        if !name.eq_ignore_ascii_case("Authorization") {
            return false;
        }
        let token = match h.value.as_str().strip_prefix("Bearer ") {
            Some(t) => t,
            None => return false,
        };
        let admin_match =
            !admin_bytes.is_empty() && pylon_auth::constant_time_eq(token.as_bytes(), admin_bytes);
        let metrics_match = !metrics_bytes.is_empty()
            && pylon_auth::constant_time_eq(token.as_bytes(), metrics_bytes);
        admin_match || metrics_match
    });
    if bearer_ok {
        return true;
    }
    // Session-cookie path. Resolves the configured cookie name to a
    // session, looks up the User row, and checks the manifest's
    // `auth.user.adminField` OR the PYLON_ADMIN_EMAILS allowlist
    // (the same two paths the main dispatcher uses for
    // `ctx.is_admin`).
    let cookie_token = request
        .headers()
        .iter()
        .find(|h| h.field.as_str() == "Cookie" || h.field.as_str() == "cookie")
        .and_then(|h| pylon_auth::extract_session_cookie(h.value.as_str(), &cookie_config.name));
    let session_user_id = cookie_token
        .as_deref()
        .and_then(|t| session_store.get(t))
        .map(|s| s.user_id);
    let Some(uid) = session_user_id.as_deref() else {
        return false;
    };
    let user_entity = runtime.manifest().auth.user.entity.as_str();
    use pylon_http::DataStore as _;
    let row = runtime.get_by_id(user_entity, uid).ok().flatten();
    let admin_field = runtime
        .manifest()
        .auth
        .user
        .admin_field
        .as_deref()
        .filter(|f| !f.is_empty());
    let field_ok = match (admin_field, row.as_ref()) {
        (Some(field), Some(row)) => match row.get(field) {
            Some(v) if v.is_boolean() => v.as_bool().unwrap_or(false),
            Some(v) if v.is_string() => {
                let s = v.as_str().unwrap_or("").to_ascii_lowercase();
                s == "true" || s == "1" || s == "admin"
            }
            _ => false,
        },
        _ => false,
    };
    if field_ok {
        return true;
    }
    // PYLON_ADMIN_EMAILS allowlist (case-insensitive, emailVerified
    // required). Matches the dispatcher's promotion rules — same
    // user set "is admin" via either path.
    let allow: Vec<String> = std::env::var("PYLON_ADMIN_EMAILS")
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    if allow.is_empty() {
        return false;
    }
    let Some(row) = row else {
        return false;
    };
    let email = row
        .get("email")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty());
    let verified = row
        .get("emailVerified")
        .or_else(|| row.get("email_verified"))
        .map(|v| match v {
            serde_json::Value::Bool(b) => *b,
            serde_json::Value::String(s) => !s.is_empty() && !s.eq_ignore_ascii_case("false"),
            _ => false,
        })
        .unwrap_or(false);
    verified && email.map(|e| allow.contains(&e)).unwrap_or(false)
}

/// Render the Studio login HTML form. Plain HTML (no framework) so it
/// renders without any JS dependency — important because /studio's
/// own JS runs only AFTER admin auth is established. The form POSTs
/// `token=<value>` to /studio/login.
///
/// Style is intentionally minimal: a single centered card. Pylon's
/// design tokens aren't reachable from a static HTML string, so the
/// styling is inline and conservative.
fn studio_login_html(error: Option<&str>) -> String {
    let err_html = error
        .map(|e| {
            format!(
                r#"<div style="margin:12px 0;padding:10px 12px;border:1px solid #f4a4a4;background:#fff5f5;border-radius:6px;color:#a40000;font:14px/1.4 ui-sans-serif,system-ui;">{}</div>"#,
                escape_html(e),
            )
        })
        .unwrap_or_default();
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Pylon Studio · sign in</title>
<style>
  body {{ margin: 0; min-height: 100vh; display: grid; place-items: center; background: #fafaf9; color: #1a1a1a; font: 14px/1.5 ui-sans-serif, system-ui, sans-serif; }}
  .card {{ width: 360px; max-width: 90vw; padding: 24px; background: #fff; border: 1px solid #e7e5e0; border-radius: 8px; box-shadow: 0 1px 2px rgba(0,0,0,0.04); }}
  h1 {{ margin: 0 0 4px; font-size: 18px; letter-spacing: -0.01em; }}
  p.lead {{ margin: 0 0 16px; color: #666; font-size: 13px; }}
  label {{ display: block; font-size: 11px; text-transform: uppercase; letter-spacing: 0.06em; color: #666; margin: 12px 0 6px; }}
  input[type=password] {{ width: 100%; box-sizing: border-box; padding: 8px 10px; font-family: ui-monospace, monospace; font-size: 13px; border: 1px solid #d4d2cd; border-radius: 6px; background: #fafafa; }}
  input[type=password]:focus {{ outline: none; border-color: #2a5fdf; background: #fff; }}
  button {{ margin-top: 16px; width: 100%; padding: 9px 12px; background: #2a5fdf; color: #fff; border: 0; border-radius: 6px; font-size: 13px; font-weight: 500; cursor: pointer; }}
  button:hover {{ background: #1f4cb8; }}
  .hint {{ margin-top: 16px; font-size: 11.5px; color: #888; }}
  code {{ font-family: ui-monospace, monospace; font-size: 11.5px; background: #f4f3f0; padding: 1px 4px; border-radius: 3px; }}
</style>
</head>
<body>
<form class="card" method="POST" action="/studio/login" autocomplete="off">
  <h1>Studio</h1>
  <p class="lead">Sign in with your Pylon admin token.</p>
  {err_html}
  <label for="token">Admin token</label>
  <input id="token" name="token" type="password" autofocus required>
  <button type="submit">Sign in</button>
  <div class="hint">Set on the server as <code>PYLON_ADMIN_TOKEN</code>.</div>
</form>
</body>
</html>"#,
    )
}

/// "You're signed in but not an admin" page. Rendered when the /studio
/// gate sees a resolved user without is_admin — the user has a working
/// session, so we don't bounce them back through login. Instead we tell
/// them what's wrong and offer a logout link to switch accounts. Plain
/// HTML so it doesn't depend on the Studio bundle (which we're refusing
/// to serve).
fn studio_no_access_html() -> String {
    r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Pylon Studio · access denied</title>
<style>
  body { margin: 0; min-height: 100vh; display: grid; place-items: center; background: #fafaf9; color: #1a1a1a; font: 14px/1.5 ui-sans-serif, system-ui, sans-serif; }
  .card { width: 420px; max-width: 90vw; padding: 24px; background: #fff; border: 1px solid #e7e5e0; border-radius: 8px; box-shadow: 0 1px 2px rgba(0,0,0,0.04); text-align: center; }
  h1 { margin: 0 0 8px; font-size: 18px; letter-spacing: -0.01em; }
  p { margin: 0 0 12px; color: #555; font-size: 13px; }
  a.btn { display: inline-block; margin-top: 12px; padding: 8px 14px; background: #2a5fdf; color: #fff; border-radius: 6px; text-decoration: none; font-size: 13px; font-weight: 500; }
  a.btn:hover { background: #1f4cb8; }
  .muted { color: #888; font-size: 11.5px; margin-top: 14px; }
</style>
</head>
<body>
<div class="card">
  <h1>Studio access denied</h1>
  <p>Your account is signed in but doesn't have admin privileges on this Pylon.</p>
  <a class="btn" href="/studio/logout">Sign out and try a different account</a>
  <div class="muted">Need access? Ask whoever runs this Pylon to mark your account as admin.</div>
</div>
</body>
</html>"##.to_string()
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Tiny URL-decoder for the studio login form's `token=…` field.
/// Handles `%xx` escapes; `+` → space conversion happens at the
/// caller. Doesn't allocate when there's nothing to decode.
fn percent_decode_str(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_digit(bytes[i + 1]), hex_digit(bytes[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Bundle of the four auth-state stores. Built in one place so backend
/// selection (Postgres vs. SQLite) is consistent across them — there's
/// no scenario where sessions live in PG but accounts live in a sibling
/// SQLite file. Selection rules, in priority:
///
/// 1. `DATABASE_URL=postgres://…` → all four stores point at PG.
/// 2. `PYLON_SESSION_DB=path/to/file.db` → SQLite, explicit path.
/// 3. `<app_db_path>.sessions.db` → SQLite alongside the app DB.
/// 4. `PYLON_SESSION_IN_MEMORY=1` or no app DB → in-memory.
struct AuthStores {
    session_store: Arc<SessionStore>,
    magic_codes: Arc<pylon_auth::MagicCodeStore>,
    oauth_state: Arc<pylon_auth::OAuthStateStore>,
    account_store: Arc<pylon_auth::AccountStore>,
    api_keys: Arc<pylon_auth::api_key::ApiKeyStore>,
    siwe: Arc<pylon_auth::siwe::NonceStore>,
    phone_codes: Arc<pylon_auth::phone::PhoneCodeStore>,
    passkeys: Arc<pylon_auth::webauthn::PasskeyStore>,
    verification: Arc<pylon_auth::verification::VerificationStore>,
    audit: Arc<pylon_auth::audit::AuditStore>,
    trusted_devices: Arc<dyn pylon_auth::trusted_device::TrustedDeviceStore>,
    org_sso: Arc<dyn pylon_auth::org_sso::OrgSsoStore>,
    saml: Arc<dyn pylon_auth::saml::SamlStore>,
}

// Memoized env reads — auth resolver runs PER REQUEST so we can't
// afford `std::env::var` syscalls there. OnceLock initialized
// lazily on first lookup; tests that mutate env between cases
// should use process-level isolation, not in-process mutation.
fn jwt_secret() -> Option<&'static String> {
    static CELL: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    CELL.get_or_init(|| {
        std::env::var("PYLON_JWT_SECRET")
            .ok()
            .filter(|s| !s.is_empty())
    })
    .as_ref()
}

fn jwt_issuer() -> Option<&'static String> {
    static CELL: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    CELL.get_or_init(|| {
        std::env::var("PYLON_JWT_ISSUER")
            .ok()
            .filter(|s| !s.is_empty())
    })
    .as_ref()
}

fn build_auth_stores(
    app_db_path: Option<&str>,
    session_lifetime: u64,
) -> Result<AuthStores, String> {
    // Forced in-memory escape hatch — used by integration tests that
    // never want to touch disk.
    let force_in_memory = std::env::var("PYLON_SESSION_IN_MEMORY")
        .map(|v| v == "1" || v == "true")
        .unwrap_or(false);

    // Postgres path — wins over PYLON_SESSION_DB when both are set so the
    // multi-replica deploy doesn't silently fall back to per-replica SQLite.
    let pg_url = std::env::var("DATABASE_URL")
        .ok()
        .filter(|u| u.starts_with("postgres://") || u.starts_with("postgresql://"));

    if let Some(url) = pg_url {
        if force_in_memory {
            // Tests that explicitly opt out of persistence shouldn't be
            // overridden by an ambient DATABASE_URL in CI.
            return Ok(in_memory_auth_stores(session_lifetime));
        }
        return build_pg_auth_stores(&url, session_lifetime);
    }

    let sqlite_path = std::env::var("PYLON_SESSION_DB")
        .ok()
        .or_else(|| app_db_path.map(|p| format!("{p}.sessions.db")));

    match (force_in_memory, sqlite_path) {
        (true, _) | (_, None) => Ok(in_memory_auth_stores(session_lifetime)),
        (false, Some(path)) => build_sqlite_auth_stores(&path, session_lifetime),
    }
}

fn in_memory_auth_stores(session_lifetime: u64) -> AuthStores {
    AuthStores {
        session_store: Arc::new(SessionStore::new().with_lifetime(session_lifetime)),
        magic_codes: Arc::new(pylon_auth::MagicCodeStore::new()),
        oauth_state: Arc::new(pylon_auth::OAuthStateStore::new()),
        account_store: Arc::new(pylon_auth::AccountStore::new()),
        api_keys: Arc::new(pylon_auth::api_key::ApiKeyStore::new()),
        siwe: Arc::new(pylon_auth::siwe::NonceStore::new()),
        phone_codes: Arc::new(pylon_auth::phone::PhoneCodeStore::new()),
        passkeys: Arc::new(pylon_auth::webauthn::PasskeyStore::new()),
        verification: Arc::new(pylon_auth::verification::VerificationStore::new()),
        audit: Arc::new(pylon_auth::audit::AuditStore::new()),
        trusted_devices: Arc::new(pylon_auth::trusted_device::InMemoryTrustedDeviceStore::new()),
        org_sso: Arc::new(pylon_auth::org_sso::InMemoryOrgSsoStore::new()),
        saml: Arc::new(pylon_auth::saml::InMemorySamlStore::new()),
    }
}

/// Open every SQLite-backed auth store at `path`. Fails fast at boot
/// if any one can't be opened — pre-0.3.93 we silently fell back to
/// in-memory, which produced cross-process state loss that surfaced
/// later as `OAUTH_INVALID_STATE` (state minted on one process,
/// validated on a different one after a machine restart). No more
/// silent degradation.
fn build_sqlite_auth_stores(path: &str, session_lifetime: u64) -> Result<AuthStores, String> {
    let map_err = |what: &str, e: String| format!("[pylon] {what} SQLite backend at {path}: {e}");

    let session_store = SessionStore::with_backend(Box::new(
        crate::session_backend::SqliteSessionBackend::open(path)
            .map_err(|e| map_err("session", e))?,
    ))
    .with_lifetime(session_lifetime);
    tracing::info!("[pylon] Auth state (SQLite): {path}");
    let magic_codes = pylon_auth::MagicCodeStore::with_backend(Box::new(
        crate::magic_code_backend::SqliteMagicCodeBackend::open(path)
            .map_err(|e| map_err("magic-code", e))?,
    ));
    let oauth_state = pylon_auth::OAuthStateStore::with_backend(Box::new(
        crate::oauth_backend::SqliteOAuthBackend::open(path)
            .map_err(|e| map_err("OAuth state", e))?,
    ));
    let account_store = pylon_auth::AccountStore::with_backend(Box::new(
        crate::account_backend::SqliteAccountBackend::open(path)
            .map_err(|e| map_err("account-link", e))?,
    ));
    let api_keys = pylon_auth::api_key::ApiKeyStore::with_backend(Box::new(
        crate::api_key_backend::SqliteApiKeyBackend::open(path)
            .map_err(|e| map_err("api-key", e))?,
    ));
    let verification = pylon_auth::verification::VerificationStore::with_backend(Box::new(
        crate::verification_backend::SqliteVerificationBackend::open(path)
            .map_err(|e| map_err("verification", e))?,
    ));
    let audit = pylon_auth::audit::AuditStore::with_backend(Box::new(
        crate::audit_backend::SqliteAuditBackend::open(path).map_err(|e| map_err("audit", e))?,
    ));
    let trusted_devices: Arc<dyn pylon_auth::trusted_device::TrustedDeviceStore> = Arc::new(
        crate::trusted_device_backend::SqliteTrustedDeviceBackend::open(path)
            .map_err(|e| map_err("trusted-device", e))?,
    );
    let org_sso: Arc<dyn pylon_auth::org_sso::OrgSsoStore> = Arc::new(
        crate::org_sso_backend::SqliteOrgSsoBackend::open(path)
            .map_err(|e| map_err("org-SSO", e))?,
    );
    let saml: Arc<dyn pylon_auth::saml::SamlStore> = Arc::new(
        crate::saml_backend::SqliteSamlBackend::open(path).map_err(|e| map_err("SAML", e))?,
    );
    Ok(AuthStores {
        session_store: Arc::new(session_store),
        magic_codes: Arc::new(magic_codes),
        oauth_state: Arc::new(oauth_state),
        account_store: Arc::new(account_store),
        api_keys: Arc::new(api_keys),
        siwe: Arc::new(pylon_auth::siwe::NonceStore::new()),
        phone_codes: Arc::new(pylon_auth::phone::PhoneCodeStore::new()),
        passkeys: Arc::new(pylon_auth::webauthn::PasskeyStore::new()),
        verification: Arc::new(verification),
        audit: Arc::new(audit),
        trusted_devices,
        org_sso,
        saml,
    })
}

/// Connect every Postgres-backed auth store. Fail-fast at boot if
/// any connection fails — pre-0.3.93 we silently fell back to
/// in-memory backends per-store, which is exactly the OAUTH_INVALID_STATE
/// failure pylon-cloud hit: PG connection error at boot → state lives
/// in-process → machine auto-stop + cold boot wipes state → every
/// OAuth callback fails with "Invalid, expired, or already-consumed
/// state" and the operator has no signal that PG was never reachable.
///
/// Each backend opens its own connection. Sessions/oauth-state/magic-codes/
/// accounts are low-frequency relative to entity CRUD — keeping them on
/// separate connections avoids a "oauth lookup blocks an entity write"
/// false-sharing scenario at the cost of a few idle PG connections.
fn build_pg_auth_stores(url: &str, session_lifetime: u64) -> Result<AuthStores, String> {
    let map_err = |what: &str, e: String| {
        format!(
            "[pylon] {what} Postgres backend at {url}: {e}. \
             DATABASE_URL is set but the connection failed — pylon refuses \
             to boot rather than silently fall back to in-memory state \
             that would lose every OAuth flow on restart."
        )
    };

    let session_store = SessionStore::with_backend(Box::new(
        crate::session_backend::PostgresSessionBackend::connect(url)
            .map_err(|e| map_err("session", e))?,
    ))
    .with_lifetime(session_lifetime);
    tracing::info!("[pylon] Auth state (Postgres): {url}");
    let magic_codes = pylon_auth::MagicCodeStore::with_backend(Box::new(
        crate::magic_code_backend::PostgresMagicCodeBackend::connect(url)
            .map_err(|e| map_err("magic-code", e))?,
    ));
    let oauth_state = pylon_auth::OAuthStateStore::with_backend(Box::new(
        crate::oauth_backend::PostgresOAuthBackend::connect(url)
            .map_err(|e| map_err("OAuth state", e))?,
    ));
    let account_store = pylon_auth::AccountStore::with_backend(Box::new(
        crate::account_backend::PostgresAccountBackend::connect(url)
            .map_err(|e| map_err("account-link", e))?,
    ));
    let api_keys = pylon_auth::api_key::ApiKeyStore::with_backend(Box::new(
        crate::api_key_backend::PostgresApiKeyBackend::connect(url)
            .map_err(|e| map_err("api-key", e))?,
    ));
    let verification = pylon_auth::verification::VerificationStore::with_backend(Box::new(
        crate::verification_backend::PostgresVerificationBackend::connect(url)
            .map_err(|e| map_err("verification", e))?,
    ));
    let audit = pylon_auth::audit::AuditStore::with_backend(Box::new(
        crate::audit_backend::PostgresAuditBackend::connect(url)
            .map_err(|e| map_err("audit", e))?,
    ));
    let trusted_devices: Arc<dyn pylon_auth::trusted_device::TrustedDeviceStore> = Arc::new(
        crate::trusted_device_backend::PostgresTrustedDeviceBackend::connect(url)
            .map_err(|e| map_err("trusted-device", e))?,
    );
    let org_sso: Arc<dyn pylon_auth::org_sso::OrgSsoStore> = Arc::new(
        crate::org_sso_backend::PostgresOrgSsoBackend::connect(url)
            .map_err(|e| map_err("org-SSO", e))?,
    );
    let saml: Arc<dyn pylon_auth::saml::SamlStore> = Arc::new(
        crate::saml_backend::PostgresSamlBackend::connect(url).map_err(|e| map_err("SAML", e))?,
    );
    Ok(AuthStores {
        session_store: Arc::new(session_store),
        magic_codes: Arc::new(magic_codes),
        oauth_state: Arc::new(oauth_state),
        account_store: Arc::new(account_store),
        api_keys: Arc::new(api_keys),
        siwe: Arc::new(pylon_auth::siwe::NonceStore::new()),
        phone_codes: Arc::new(pylon_auth::phone::PhoneCodeStore::new()),
        passkeys: Arc::new(pylon_auth::webauthn::PasskeyStore::new()),
        verification: Arc::new(verification),
        audit: Arc::new(audit),
        trusted_devices,
        org_sso,
        saml,
    })
}

/// Build the session store. Persists by default for file-backed runtimes —
/// sessions live in a sibling `<db>.sessions.db` file next to the app DB
/// unless `PYLON_SESSION_DB` overrides the path or
/// `PYLON_SESSION_IN_MEMORY=1` opts out. In-memory runtimes (tests)
/// get an in-memory session store.
///
/// This used to be opt-in, which silently broke every app after a server
/// restart: tokens in browser localStorage resolved to anonymous, pulls
/// came back empty under policy, and mutations 400'd with UNAUTHENTICATED.
#[allow(dead_code)]
fn build_session_store(app_db_path: Option<&str>) -> SessionStore {
    if std::env::var("PYLON_SESSION_IN_MEMORY")
        .map(|v| v == "1" || v == "true")
        .unwrap_or(false)
    {
        return SessionStore::new();
    }
    let explicit = std::env::var("PYLON_SESSION_DB").ok();
    let default_path = app_db_path.map(|p| format!("{p}.sessions.db"));
    let path = match explicit.or(default_path) {
        Some(p) => p,
        None => return SessionStore::new(),
    };
    match crate::session_backend::SqliteSessionBackend::open(&path) {
        Ok(backend) => {
            tracing::info!("[pylon] Session persistence enabled: {path}");
            SessionStore::with_backend(Box::new(backend))
        }
        Err(e) => {
            tracing::warn!(
                "[pylon] could not open session DB {path}: {e}. Falling back to in-memory sessions."
            );
            SessionStore::new()
        }
    }
}

// Multipart parser + its tests removed in v0.3.91 along with the
// legacy `POST /api/files/upload` endpoint. The new 3-step flow
// (init → client PUT → confirm) never goes through a multipart body,
// so the parser had no remaining caller.
