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

    let addr = format!("0.0.0.0:{port}");
    let server = Server::http(&addr).map_err(|e| format!("Failed to start server: {e}"))?;
    let server = Arc::new(server);

    // Stash a handle so `request_shutdown()` can unblock the loop.
    let _ = SERVER_HANDLE.set(Arc::clone(&server));

    let session_lifetime = runtime.manifest().auth.session.expires_in;
    let auth_stores = build_auth_stores(runtime.db_path().as_deref(), session_lifetime);
    let session_store = auth_stores.session_store;
    let magic_codes = auth_stores.magic_codes;
    let oauth_state = auth_stores.oauth_state;
    let account_store = auth_stores.account_store;
    let api_keys = auth_stores.api_keys;
    let orgs = auth_stores.orgs;
    let siwe = auth_stores.siwe;
    let phone_codes = auth_stores.phone_codes;
    let passkeys = auth_stores.passkeys;
    let verification = auth_stores.verification;
    let audit = auth_stores.audit;
    let policy_engine = Arc::new(PolicyEngine::from_manifest(runtime.manifest()));
    let change_log = Arc::new(ChangeLog::new());

    // Seed the change log with one synthetic insert per extant row so that
    // a pull from seq=0 after a restart reconstructs current state. The
    // change log is in-memory — restarting the process without this would
    // leave SQLite rows unreachable via /api/sync/pull (clients would
    // pull nothing and see an empty replica). Seqs here are fresh; clients
    // whose cursors are ahead of `self.seq` get a 410 and full resync,
    // which then hits this seeded log and gets every current row back.
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
    let ws_hub = WsHub::new();
    let sse_hub = SseHub::new();
    // Default-register the rate-limit plugin when no custom registry was
    // supplied. Without this, self-hosted deployments would launch with
    // auth endpoints (/api/auth/magic/send, /api/auth/magic/verify,
    // /api/auth/session) wide open to brute force and enumeration.
    //
    // Dev: 100k/min so a React app's initial bundle + auth + sync pulls
    // (each worth ~6-10 requests) doesn't immediately 429 the dev. Prod:
    // 100/min per IP — tight enough to crush burst attackers, loose
    // enough for legitimate multi-tab UIs. Callers passing their own
    // registry are responsible for their own limits.
    // Probe dev mode NOW — defined for real at line ~300 but plugin
    // registration below needs it. Same env-var, same logic.
    let is_dev_early = std::env::var("PYLON_DEV_MODE")
        .map(|v| v == "1" || v == "true")
        .unwrap_or(true);
    let plugin_rl_max: u32 = if is_dev_early { 100_000 } else { 100 };
    let plugin_reg: Arc<PluginRegistry> = plugins.unwrap_or_else(|| {
        let mut reg = PluginRegistry::new(runtime.manifest().clone());
        reg.register(Arc::new(
            pylon_plugin::builtin::rate_limit::RateLimitPlugin::new(
                plugin_rl_max,
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

    // TypeScript function runtime: optional Bun process that loads functions/*.ts
    // If no `functions/` directory exists or Bun isn't installed, this is None
    // and /api/fn/* routes return 503.
    // Build a notifier adapter so function mutations (`ctx.db.insert/update/delete`)
    // emit change events to WS + SSE subscribers on COMMIT. Without this,
    // functions write to the DB but sync clients never see the update
    // live — they only catch up on the next refetch.
    let fn_notifier: Arc<dyn pylon_router::ChangeNotifier> =
        Arc::new(crate::datastore::WsSseNotifier {
            ws: Arc::clone(&ws_hub),
            sse: Arc::clone(&sse_hub),
        });
    let fn_ops_maybe = crate::datastore::try_spawn_functions(
        Arc::clone(&runtime),
        Arc::clone(&job_queue),
        Arc::clone(&fn_rate_limiter),
        Arc::clone(&change_log),
        fn_notifier,
    );

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

    // CORS origin. Defaults to `*` in dev for convenience; in prod we refuse
    // to start with a wildcard because the server sends `Access-Control-
    // Allow-Credentials: true` elsewhere and also accepts `Authorization:
    // Bearer <session>`. The combination of `*` + credentials is a spec
    // violation that some browsers tolerate, and even when they don't it
    // lets any origin drive bearer-auth APIs.
    let cors_origin = match std::env::var("PYLON_CORS_ORIGIN") {
        Ok(v) => v,
        Err(_) if is_dev => "*".to_string(),
        Err(_) => {
            return Err(
                "PYLON_CORS_ORIGIN must be set in production (non-dev mode). \
                Set it to your frontend's origin, or set PYLON_DEV_MODE=true \
                for local development."
                    .into(),
            );
        }
    };
    if !is_dev && cors_origin == "*" {
        return Err("PYLON_CORS_ORIGIN=\"*\" is refused in production mode. \
            Set it to an explicit origin (https://app.example.com)."
            .into());
    }
    // Browsers forbid combining `Access-Control-Allow-Origin: *` with
    // `Access-Control-Allow-Credentials: true`. Cookie-based auth needs
    // credentials, so we only emit the credentials header when the origin
    // is specific. In dev with `*` we lose cookies-from-cross-origin
    // (acceptable: dev typically uses same-origin proxying), but we
    // refuse to send a header combo browsers will reject either way.
    let allow_credentials = cors_origin != "*";
    // Validate the origin once so per-request header construction can never
    // panic on bad bytes. Previously every `Header::from_bytes(...).unwrap()`
    // was a potential request-triggered DoS via env misconfiguration.
    if Header::from_bytes(
        "Access-Control-Allow-Origin",
        cors_origin.as_bytes().to_vec(),
    )
    .is_err()
    {
        return Err(format!(
            "PYLON_CORS_ORIGIN={cors_origin:?} contains bytes that are not a valid HTTP header value"
        ));
    }

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

    // CSRF protection. Enforced inline at the HTTP layer because the plugin
    // trait's `on_request` hook doesn't see request headers. For
    // state-changing methods (POST/PATCH/PUT/DELETE) we check Origin, then
    // Referer, against the allowlist.
    //
    // Allowlist resolution:
    //   - PYLON_CSRF_ORIGINS (comma-separated) if set
    //   - otherwise PYLON_CORS_ORIGIN (already validated above)
    //   - in dev, fall back to allow-any to avoid breaking local tooling.
    let csrf_origins: Vec<String> = match std::env::var("PYLON_CSRF_ORIGINS") {
        Ok(v) => v
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        Err(_) => {
            if is_dev {
                vec!["*".to_string()]
            } else if cors_origin != "*" {
                vec![cors_origin.clone()]
            } else {
                // Non-dev + wildcard was already rejected, but guard anyway.
                vec![]
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
    // Manifest-declared trusted origins (from auth({trustedOrigins: [...]})
    // in app.ts) get merged with the env list. Manifest is the
    // type-safe declarative source; env is the operator override for
    // ops-only deploys.
    let manifest_trusted: Vec<String> = runtime.manifest().auth.trusted_origins.clone();
    let trusted_origins: Vec<String> = std::env::var("PYLON_TRUSTED_ORIGINS")
        .map(|v| {
            v.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_else(|_| {
            // Dev-mode default: trust localhost on the conventional
            // ports so `pylon dev` + `next dev` works without env
            // surgery. Production (PYLON_DEV_MODE=false or unset) gets
            // an empty list, which fails-closed at the OAuth start
            // endpoint with a clear error pointing the operator at
            // PYLON_TRUSTED_ORIGINS.
            if is_dev_early {
                vec![
                    "http://localhost:3000".to_string(),
                    "http://localhost:4321".to_string(),
                    "http://localhost:5173".to_string(),
                    "http://127.0.0.1:3000".to_string(),
                ]
            } else {
                Vec::new()
            }
        });
    // Combine env + manifest, dedup, drop empties.
    let mut combined: Vec<String> = trusted_origins;
    for m in manifest_trusted {
        if !m.is_empty() && !combined.contains(&m) {
            combined.push(m);
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
    {
        let hub = Arc::clone(&ws_hub);
        let sessions = Arc::clone(&session_store);
        let runtime_for_fetcher = Arc::clone(&runtime);
        let pe_for_fetcher = Arc::clone(&policy_engine);
        let fetcher: crate::ws::SnapshotFetcher = Arc::new(move |auth_ctx, entity, row_id| {
            use pylon_http::DataStore;
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
        });
        std::thread::spawn(move || {
            crate::ws::start_ws_server(hub, sessions, ws_port, Some(fetcher));
        });
    }

    // Start SSE server on port+2.
    {
        let hub = Arc::clone(&sse_hub);
        std::thread::spawn(move || {
            crate::sse::start_sse_server(hub, sse_port);
        });
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

        let rt = Arc::clone(&runtime);
        let ss = Arc::clone(&session_store);
        let pe = Arc::clone(&policy_engine);
        let cl = Arc::clone(&change_log);
        let wh = Arc::clone(&ws_hub);
        let sh = Arc::clone(&sse_hub);
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
        let trusted_origins_ref = Arc::clone(&trusted_origins);
        let ca = Arc::clone(&cache);
        let ps = Arc::clone(&pubsub_broker);
        let jq = Arc::clone(&job_queue);
        let sc = Arc::clone(&scheduler);
        let we = Arc::clone(&workflow_engine);
        let fn_ops_ref = fn_ops_maybe.clone();
        let shards_ref = shard_registry.clone();
        let cors_origin = cors_origin.clone();
        let cookie_config = Arc::clone(&cookie_config);
        let allow_credentials = allow_credentials;
        let is_dev = is_dev;

        let method = request.method().clone();
        let url = request.url().to_string();

        // Per-request access log — visibility into what's hitting the
        // server, mirroring Next.js's `GET /login 200 in 27ms` style.
        // Suppress for noisy paths (/health, /metrics) so dev-mode logs
        // don't drown in proxy/scrape traffic. Status + duration get
        // logged separately by `metrics.record_request` so we don't
        // need to thread them through every response branch.
        //
        // Per-request peer IP keeps it useful for debugging
        // multi-origin setups (CSRF rejections, rate-limit hits) —
        // resolve_client_ip already honors PYLON_TRUST_PROXY_HOPS so
        // this matches what the rest of the server sees.
        let request_peer_ip = resolve_client_ip(&request, trust_proxy_hops);
        let request_started_at = std::time::Instant::now();
        if url != "/health" && url != "/metrics" {
            tracing::info!("→ {} {} from {}", method.as_str(), url, request_peer_ip);
            // Stash for the response log (`record_request` reads this
            // thread-local to emit method/url/status/duration in one
            // line, like Next.js's `GET /login 200 in 27ms`).
            crate::metrics::set_current_request(&url, request_started_at);
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
            continue;
        }

        // --- Metrics endpoint: fast path before rate-limit / body parsing.
        // Gate behind admin auth in non-dev to prevent leakage of function
        // names, request volumes, and error rates to the public internet.
        // Dev mode stays open so local Prometheus scrapers just work.
        if url == "/metrics" && method == Method::Get {
            if !is_dev {
                let admin_bytes = admin_token.as_deref().unwrap_or("").as_bytes();
                let auth_ok = !admin_bytes.is_empty()
                    && request.headers().iter().any(|h| {
                        let name = h.field.as_str().as_str();
                        name.eq_ignore_ascii_case("Authorization")
                            && h.value
                                .as_str()
                                .strip_prefix("Bearer ")
                                .map(|t| pylon_auth::constant_time_eq(t.as_bytes(), admin_bytes))
                                .unwrap_or(false)
                    });
                if !auth_ok {
                    let body = json_error(
                        "UNAUTHORIZED",
                        "/metrics requires admin bearer token in non-dev mode",
                    );
                    let response = with_security_headers(
                        Response::from_string(&body)
                            .with_status_code(401u16)
                            .with_header(
                                Header::from_bytes("Content-Type", "application/json").unwrap(),
                            ),
                    );
                    let _ = request.respond(response);
                    continue;
                }
            }
            let prefers_prometheus = request.headers().iter().any(|h| {
                (h.field.as_str() == "Accept" || h.field.as_str() == "accept")
                    && (h.value.as_str().contains("text/plain")
                        || h.value.as_str().contains("application/openmetrics-text"))
            });
            let (body, content_type) = if prefers_prometheus {
                (mt.prometheus(), "text/plain; version=0.0.4")
            } else {
                (mt.snapshot().to_string(), "application/json")
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
            continue;
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
                continue;
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
                    continue;
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
        let auth_token: Option<String> = bearer_token.or(cookie_token);
        // Token dispatcher (in priority order):
        //   1. Admin token → AuthContext::admin
        //   2. `pk.…` API key → AuthContext::from_api_key (401 on bad)
        //   3. Looks-like-JWT + PYLON_JWT_SECRET set → JWT verify
        //   4. Otherwise → session store lookup
        // pk. check happens BEFORE looks_like_jwt because an api-key
        // token also has 3 dot-separated segments and would otherwise
        // be misrouted.
        let auth_ctx_result: Result<pylon_auth::AuthContext, &'static str> = if admin_token.is_some()
            && auth_token.is_some()
            && pylon_auth::constant_time_eq(
                auth_token.as_deref().unwrap_or("").as_bytes(),
                admin_token.as_deref().unwrap_or("").as_bytes(),
            ) {
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
                    Err("JWT_MISCONFIGURED")?;
                    unreachable!();
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
        let auth_ctx = match auth_ctx_result {
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
                continue;
            }
        };

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
                continue;
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
            continue;
        }

        // --- File upload fast path: handle binary body before string conversion ---
        // Uploads come in two shapes:
        //   1. Direct binary body with X-Filename / Content-Type headers
        //   2. multipart/form-data with a file part
        //
        // Require an authenticated user. Uploads write to the files backend
        // (and into the plugin audit log for soft-delete etc.), so
        // unauthenticated callers cannot use this route.
        if url == "/api/files/upload" && method == Method::Post {
            const UPLOAD_MAX: usize = 10 * 1024 * 1024;
            // Enforce size BEFORE reading the body so a 10 GiB stream can't
            // buffer into memory. Content-Length pre-check, then bounded read.
            if let Some(declared) = request.body_length() {
                if declared > UPLOAD_MAX {
                    let err = json_error(
                        "PAYLOAD_TOO_LARGE",
                        &format!("Content-Length {declared} exceeds upload max of {UPLOAD_MAX}"),
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
                    continue;
                }
            }
            if auth_ctx.user_id.is_none() {
                let err = json_error(
                    "AUTH_REQUIRED",
                    "/api/files/upload requires an authenticated session",
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
                continue;
            }
            // Read up to UPLOAD_MAX + 1 bytes. If we read the full +1 we know
            // the client lied about Content-Length (or used chunked encoding
            // and overran). Reject in that case instead of continuing with a
            // truncated file.
            use std::io::Read;
            let mut bytes: Vec<u8> = Vec::with_capacity(8192);
            let mut limited = request.as_reader().take((UPLOAD_MAX as u64) + 1);
            let _ = limited.read_to_end(&mut bytes);

            const MAX: usize = UPLOAD_MAX;
            if bytes.len() > MAX {
                let err = json_error("PAYLOAD_TOO_LARGE", "File exceeds 10 MB limit");
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
                continue;
            }

            // Headers.
            let content_type = request
                .headers()
                .iter()
                .find(|h| h.field.as_str() == "Content-Type" || h.field.as_str() == "content-type")
                .map(|h| h.value.as_str().to_string())
                .unwrap_or_else(|| "application/octet-stream".into());
            let filename = request
                .headers()
                .iter()
                .find(|h| h.field.as_str() == "X-Filename" || h.field.as_str() == "x-filename")
                .map(|h| h.value.as_str().to_string())
                .unwrap_or_else(|| "upload".into());

            // If multipart, extract the first file part. Otherwise use bytes directly.
            let (name, ct, payload) = if content_type.starts_with("multipart/form-data") {
                match parse_multipart_first_file(&bytes, &content_type) {
                    Some(p) => p,
                    None => {
                        let err = json_error("INVALID_MULTIPART", "Could not parse multipart body");
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
                        continue;
                    }
                }
            } else {
                (filename, content_type, bytes)
            };

            let storage = pylon_storage::files::LocalFileStorage::new(
                &std::env::var("PYLON_FILES_DIR").unwrap_or_else(|_| "uploads".into()),
                &std::env::var("PYLON_FILES_URL_PREFIX").unwrap_or_else(|_| "/api/files".into()),
            );

            let (status, body) =
                match pylon_storage::files::FileStorage::store(&storage, &name, &payload, &ct) {
                    Ok(stored) => (
                        201u16,
                        serde_json::to_string(&stored).unwrap_or_else(|_| "{}".into()),
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
            continue;
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
                continue;
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
            continue;
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
                        continue;
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
                            continue;
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
                            continue;
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
                        continue;
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
                    continue;
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
                if pylon_router::FnOps::get_fn(fn_ops.as_ref(), &fn_name).is_none() {
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
                    continue;
                }
                // 2. Per-function rate limit (identity = user_id or "anon").
                let identity = auth_ctx.user_id.as_deref().unwrap_or("anon");
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
                    continue;
                }

                let args: serde_json::Value =
                    serde_json::from_str(&body).unwrap_or(serde_json::json!({}));

                let auth = pylon_functions::protocol::AuthInfo {
                    user_id: auth_ctx.user_id.clone(),
                    is_admin: auth_ctx.is_admin,
                    tenant_id: auth_ctx.tenant_id.clone(),
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
                continue;
            }
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
                continue;
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
                continue;
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
                    continue;
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
                    continue;
                }
            };

            // Override model from request body if provided.
            let model = parsed
                .get("model")
                .and_then(|m| m.as_str())
                .map(|s| s.to_string())
                .unwrap_or(ai_model);

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
            continue;
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
        let (status, response_body, content_type, is_studio, extra_headers) = if (url == "/studio"
            || url == "/studio/")
            && method == Method::Get
        {
            if !is_dev && !auth_ctx.is_admin {
                let body = json_error(
                    "AUTH_REQUIRED",
                    "/studio requires admin auth in production (set PYLON_ADMIN_TOKEN and pass it as Bearer)",
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
                continue;
            }
            // Derive the public base URL from the request's Host header +
            // X-Forwarded-Proto (Fly / any HTTPS terminator sets this).
            // Hardcoding `http://localhost:{port}` here meant the studio
            // HTML served from pylon-crm.fly.dev tried to fetch
            // http://localhost:4321/api/* from the browser, which CSP
            // rightly blocks.
            let host = request
                .headers()
                .iter()
                .find(|h| h.field.equiv("Host"))
                .map(|h| h.value.as_str().to_string())
                .unwrap_or_else(|| format!("localhost:{port}"));
            let scheme = request
                .headers()
                .iter()
                .find(|h| h.field.equiv("X-Forwarded-Proto"))
                .map(|h| h.value.as_str().to_string())
                .unwrap_or_else(|| "http".to_string());
            let base = format!("{scheme}://{host}");
            let html = pylon_studio_api::generate_studio_html(rt.manifest(), &base);
            (
                200u16,
                html,
                "text/html",
                true,
                Vec::<(String, String)>::new(),
            )
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
                let notifier = WsSseNotifier {
                    ws: Arc::clone(&wh),
                    sse: Arc::clone(&sh),
                };
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
    orgs: Arc<pylon_auth::org::OrgStore>,
    siwe: Arc<pylon_auth::siwe::NonceStore>,
    phone_codes: Arc<pylon_auth::phone::PhoneCodeStore>,
    passkeys: Arc<pylon_auth::webauthn::PasskeyStore>,
    verification: Arc<pylon_auth::verification::VerificationStore>,
    audit: Arc<pylon_auth::audit::AuditStore>,
}

// Memoized env reads — auth resolver runs PER REQUEST so we can't
// afford `std::env::var` syscalls there. OnceLock initialized
// lazily on first lookup; tests that mutate env between cases
// should use process-level isolation, not in-process mutation.
fn jwt_secret() -> Option<&'static String> {
    static CELL: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    CELL.get_or_init(|| std::env::var("PYLON_JWT_SECRET").ok().filter(|s| !s.is_empty()))
        .as_ref()
}

fn jwt_issuer() -> Option<&'static String> {
    static CELL: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    CELL.get_or_init(|| std::env::var("PYLON_JWT_ISSUER").ok().filter(|s| !s.is_empty()))
        .as_ref()
}

fn build_auth_stores(app_db_path: Option<&str>, session_lifetime: u64) -> AuthStores {
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
            return in_memory_auth_stores(session_lifetime);
        }
        return build_pg_auth_stores(&url, session_lifetime);
    }

    let sqlite_path = std::env::var("PYLON_SESSION_DB")
        .ok()
        .or_else(|| app_db_path.map(|p| format!("{p}.sessions.db")));

    match (force_in_memory, sqlite_path) {
        (true, _) | (_, None) => in_memory_auth_stores(session_lifetime),
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
        orgs: Arc::new(pylon_auth::org::OrgStore::new()),
        siwe: Arc::new(pylon_auth::siwe::NonceStore::new()),
        phone_codes: Arc::new(pylon_auth::phone::PhoneCodeStore::new()),
        passkeys: Arc::new(pylon_auth::webauthn::PasskeyStore::new()),
        verification: Arc::new(pylon_auth::verification::VerificationStore::new()),
        audit: Arc::new(pylon_auth::audit::AuditStore::new()),
    }
}

fn build_sqlite_auth_stores(path: &str, session_lifetime: u64) -> AuthStores {
    let session_store = match crate::session_backend::SqliteSessionBackend::open(path) {
        Ok(b) => {
            tracing::info!("[pylon] Auth state (SQLite): {path}");
            SessionStore::with_backend(Box::new(b)).with_lifetime(session_lifetime)
        }
        Err(e) => {
            tracing::warn!("[pylon] could not open session DB {path}: {e}. In-memory fallback.");
            SessionStore::new().with_lifetime(session_lifetime)
        }
    };
    let magic_codes = match crate::magic_code_backend::SqliteMagicCodeBackend::open(path) {
        Ok(b) => pylon_auth::MagicCodeStore::with_backend(Box::new(b)),
        Err(e) => {
            tracing::warn!("[pylon] magic-code SQLite backend unavailable: {e}");
            pylon_auth::MagicCodeStore::new()
        }
    };
    let oauth_state = match crate::oauth_backend::SqliteOAuthBackend::open(path) {
        Ok(b) => pylon_auth::OAuthStateStore::with_backend(Box::new(b)),
        Err(e) => {
            tracing::warn!("[pylon] OAuth state SQLite backend unavailable: {e}");
            pylon_auth::OAuthStateStore::new()
        }
    };
    let account_store = match crate::account_backend::SqliteAccountBackend::open(path) {
        Ok(b) => pylon_auth::AccountStore::with_backend(Box::new(b)),
        Err(e) => {
            tracing::warn!("[pylon] account-link SQLite backend unavailable: {e}");
            pylon_auth::AccountStore::new()
        }
    };
    let api_keys = match crate::api_key_backend::SqliteApiKeyBackend::open(path) {
        Ok(b) => pylon_auth::api_key::ApiKeyStore::with_backend(Box::new(b)),
        Err(e) => {
            tracing::warn!("[pylon] api-key SQLite backend unavailable: {e}");
            pylon_auth::api_key::ApiKeyStore::new()
        }
    };
    let orgs = match crate::org_backend::SqliteOrgBackend::open(path) {
        Ok(b) => pylon_auth::org::OrgStore::with_backend(Box::new(b)),
        Err(e) => {
            tracing::warn!("[pylon] org SQLite backend unavailable: {e}");
            pylon_auth::org::OrgStore::new()
        }
    };
    let verification = match crate::verification_backend::SqliteVerificationBackend::open(path) {
        Ok(b) => pylon_auth::verification::VerificationStore::with_backend(Box::new(b)),
        Err(e) => {
            tracing::warn!("[pylon] verification SQLite backend unavailable: {e}");
            pylon_auth::verification::VerificationStore::new()
        }
    };
    let audit = match crate::audit_backend::SqliteAuditBackend::open(path) {
        Ok(b) => pylon_auth::audit::AuditStore::with_backend(Box::new(b)),
        Err(e) => {
            tracing::warn!("[pylon] audit SQLite backend unavailable: {e}");
            pylon_auth::audit::AuditStore::new()
        }
    };
    AuthStores {
        session_store: Arc::new(session_store),
        magic_codes: Arc::new(magic_codes),
        oauth_state: Arc::new(oauth_state),
        account_store: Arc::new(account_store),
        api_keys: Arc::new(api_keys),
        orgs: Arc::new(orgs),
        siwe: Arc::new(pylon_auth::siwe::NonceStore::new()),
        phone_codes: Arc::new(pylon_auth::phone::PhoneCodeStore::new()),
        passkeys: Arc::new(pylon_auth::webauthn::PasskeyStore::new()),
        verification: Arc::new(verification),
        audit: Arc::new(audit),
    }
}

fn build_pg_auth_stores(url: &str, session_lifetime: u64) -> AuthStores {
    // Each backend opens its own connection. Sessions/oauth-state/magic-codes/
    // accounts are low-frequency relative to entity CRUD — keeping them on
    // separate connections avoids a "oauth lookup blocks an entity write"
    // false-sharing scenario at the cost of a few idle PG connections.
    let session_store = match crate::session_backend::PostgresSessionBackend::connect(url) {
        Ok(b) => {
            tracing::info!("[pylon] Auth state (Postgres): {url}");
            SessionStore::with_backend(Box::new(b)).with_lifetime(session_lifetime)
        }
        Err(e) => {
            tracing::warn!("[pylon] PG session backend unavailable: {e}. In-memory fallback.");
            SessionStore::new().with_lifetime(session_lifetime)
        }
    };
    let magic_codes = match crate::magic_code_backend::PostgresMagicCodeBackend::connect(url) {
        Ok(b) => pylon_auth::MagicCodeStore::with_backend(Box::new(b)),
        Err(e) => {
            tracing::warn!("[pylon] PG magic-code backend unavailable: {e}");
            pylon_auth::MagicCodeStore::new()
        }
    };
    let oauth_state = match crate::oauth_backend::PostgresOAuthBackend::connect(url) {
        Ok(b) => pylon_auth::OAuthStateStore::with_backend(Box::new(b)),
        Err(e) => {
            tracing::warn!("[pylon] PG OAuth state backend unavailable: {e}");
            pylon_auth::OAuthStateStore::new()
        }
    };
    let account_store = match crate::account_backend::PostgresAccountBackend::connect(url) {
        Ok(b) => pylon_auth::AccountStore::with_backend(Box::new(b)),
        Err(e) => {
            tracing::warn!("[pylon] PG account-link backend unavailable: {e}");
            pylon_auth::AccountStore::new()
        }
    };
    let api_keys = match crate::api_key_backend::PostgresApiKeyBackend::connect(url) {
        Ok(b) => pylon_auth::api_key::ApiKeyStore::with_backend(Box::new(b)),
        Err(e) => {
            tracing::warn!("[pylon] PG api-key backend unavailable: {e}");
            pylon_auth::api_key::ApiKeyStore::new()
        }
    };
    let orgs = match crate::org_backend::PostgresOrgBackend::connect(url) {
        Ok(b) => pylon_auth::org::OrgStore::with_backend(Box::new(b)),
        Err(e) => {
            tracing::warn!("[pylon] PG org backend unavailable: {e}");
            pylon_auth::org::OrgStore::new()
        }
    };
    let verification =
        match crate::verification_backend::PostgresVerificationBackend::connect(url) {
            Ok(b) => pylon_auth::verification::VerificationStore::with_backend(Box::new(b)),
            Err(e) => {
                tracing::warn!("[pylon] PG verification backend unavailable: {e}");
                pylon_auth::verification::VerificationStore::new()
            }
        };
    let audit = match crate::audit_backend::PostgresAuditBackend::connect(url) {
        Ok(b) => pylon_auth::audit::AuditStore::with_backend(Box::new(b)),
        Err(e) => {
            tracing::warn!("[pylon] PG audit backend unavailable: {e}");
            pylon_auth::audit::AuditStore::new()
        }
    };
    AuthStores {
        session_store: Arc::new(session_store),
        magic_codes: Arc::new(magic_codes),
        oauth_state: Arc::new(oauth_state),
        account_store: Arc::new(account_store),
        api_keys: Arc::new(api_keys),
        orgs: Arc::new(orgs),
        siwe: Arc::new(pylon_auth::siwe::NonceStore::new()),
        phone_codes: Arc::new(pylon_auth::phone::PhoneCodeStore::new()),
        passkeys: Arc::new(pylon_auth::webauthn::PasskeyStore::new()),
        verification: Arc::new(verification),
        audit: Arc::new(audit),
    }
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

/// Parse a `multipart/form-data` body and return the first file part found.
///
/// Returns `(filename, content_type, bytes)` on success, `None` if the body
/// can't be parsed or no file part exists.
///
/// Handles the common RFC 7578 subset used by browsers and curl:
/// - `--boundary` part separators
/// - `Content-Disposition: form-data; name=...; filename=...`
/// - `Content-Type: ...`
/// - blank line, then raw bytes, then `\r\n--boundary` terminator
fn parse_multipart_first_file(
    body: &[u8],
    content_type_header: &str,
) -> Option<(String, String, Vec<u8>)> {
    // Extract the boundary parameter.
    let boundary_param = content_type_header
        .split(';')
        .find_map(|p| p.trim().strip_prefix("boundary="))?;
    let boundary = boundary_param.trim_matches('"');
    let delimiter = format!("--{boundary}");
    let delimiter_bytes = delimiter.as_bytes();

    // Find each part between delimiters.
    let mut pos = 0usize;
    while pos < body.len() {
        // Find the next delimiter.
        let next = find_subslice(&body[pos..], delimiter_bytes)?;
        let part_start = pos + next + delimiter_bytes.len();
        // Skip CRLF or -- (terminator) after the delimiter.
        if part_start + 2 > body.len() {
            return None;
        }
        if &body[part_start..part_start + 2] == b"--" {
            return None; // end of parts, no file found
        }
        let header_start = part_start + skip_crlf(&body[part_start..]);

        // Find end-of-headers (blank line).
        let header_end_offset = find_subslice(&body[header_start..], b"\r\n\r\n")?;
        let headers = &body[header_start..header_start + header_end_offset];
        let data_start = header_start + header_end_offset + 4;

        // Find the next delimiter — that's where this part's data ends.
        let next_delim_offset = find_subslice(&body[data_start..], delimiter_bytes)?;
        // Strip the trailing CRLF before the delimiter.
        let mut data_end = data_start + next_delim_offset;
        if data_end >= 2 && &body[data_end - 2..data_end] == b"\r\n" {
            data_end -= 2;
        }

        // Parse headers we care about.
        let headers_str = std::str::from_utf8(headers).ok()?;
        let mut filename: Option<String> = None;
        let mut part_ct = String::from("application/octet-stream");
        let mut has_file = false;
        for line in headers_str.split("\r\n") {
            let lower = line.to_ascii_lowercase();
            if let Some(rest) = lower.strip_prefix("content-disposition:") {
                if rest.contains("filename=") {
                    has_file = true;
                    // Extract filename="xxx"
                    if let Some(start) = line.find("filename=\"") {
                        let from = start + 10;
                        if let Some(end_offset) = line[from..].find('"') {
                            filename = Some(line[from..from + end_offset].to_string());
                        }
                    }
                }
            } else if let Some(rest) = lower.strip_prefix("content-type:") {
                part_ct = rest.trim().to_string();
            }
        }

        if has_file {
            let name = filename.unwrap_or_else(|| "upload".into());
            return Some((name, part_ct, body[data_start..data_end].to_vec()));
        }

        pos = data_end;
    }
    None
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn skip_crlf(buf: &[u8]) -> usize {
    if buf.len() >= 2 && &buf[0..2] == b"\r\n" {
        2
    } else if !buf.is_empty() && buf[0] == b'\n' {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod multipart_tests {
    use super::*;

    #[test]
    fn parses_single_file() {
        let body = b"--bnd\r\n\
Content-Disposition: form-data; name=\"file\"; filename=\"hello.txt\"\r\n\
Content-Type: text/plain\r\n\
\r\n\
Hello world\r\n\
--bnd--\r\n";
        let ct = "multipart/form-data; boundary=bnd";
        let (name, content_type, bytes) = parse_multipart_first_file(body, ct).unwrap();
        assert_eq!(name, "hello.txt");
        assert_eq!(content_type, "text/plain");
        assert_eq!(bytes, b"Hello world");
    }

    #[test]
    fn returns_none_without_file_part() {
        let body = b"--bnd\r\n\
Content-Disposition: form-data; name=\"field\"\r\n\
\r\n\
just text\r\n\
--bnd--\r\n";
        let ct = "multipart/form-data; boundary=bnd";
        assert!(parse_multipart_first_file(body, ct).is_none());
    }

    #[test]
    fn returns_none_when_no_boundary() {
        assert!(parse_multipart_first_file(b"anything", "application/json").is_none());
    }
}
