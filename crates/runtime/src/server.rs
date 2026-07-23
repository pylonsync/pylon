#[allow(unused_imports)]
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
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
        let prev = self
            .in_flight
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        if prev >= self.global_cap {
            // Roll back; the slot belongs to nobody.
            self.in_flight
                .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
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
// StreamLimiter — separate cap for long-lived streaming responses.
//
// Streaming endpoints (SSE for pubsub shards, /api/llm streaming, /admin/logs/
// tail) block their worker for the stream lifetime because tiny_http drains
// the response body inline. Sharing the DispatchLimiter pool with normal
// requests would let a flood of streams starve short-lived traffic — 256
// concurrent streams = full dispatch pool, /health gets 503.
//
// This limiter has its own counters: global = 4× DispatchLimiter, per-IP =
// 16 (a single client typically opens 1–3 streams per tab). The helper
// `spawn_streaming_response` below moves request + response onto a fresh
// thread holding the stream slot, then the worker returns and frees its
// dispatch slot for the next short request.
// ---------------------------------------------------------------------------

struct StreamLimiter {
    in_flight: std::sync::atomic::AtomicUsize,
    global_cap: usize,
    per_ip: Arc<crate::ip_limit::IpConnCounter>,
}

impl StreamLimiter {
    fn new(dispatch_global_cap: usize) -> Arc<Self> {
        let global_cap = std::env::var("PYLON_STREAM_INFLIGHT_MAX")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(dispatch_global_cap * 4);
        let per_ip_cap = std::env::var("PYLON_STREAM_PER_IP_MAX")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(16u32);
        Arc::new(Self {
            in_flight: std::sync::atomic::AtomicUsize::new(0),
            global_cap,
            per_ip: Arc::new(crate::ip_limit::IpConnCounter::new(per_ip_cap)),
        })
    }

    fn acquire(
        self: &Arc<Self>,
        ip: std::net::IpAddr,
    ) -> Option<(StreamGlobalSlot, crate::ip_limit::IpConnGuard)> {
        let prev = self
            .in_flight
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        if prev >= self.global_cap {
            self.in_flight
                .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
            return None;
        }
        let global = StreamGlobalSlot {
            counter: Arc::clone(self),
        };
        let ip_guard = self.per_ip.acquire(ip)?;
        Some((global, ip_guard))
    }
}

struct StreamGlobalSlot {
    counter: Arc<StreamLimiter>,
}

impl Drop for StreamGlobalSlot {
    fn drop(&mut self) {
        self.counter
            .in_flight
            .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

/// Move a streaming response onto a dedicated thread holding a StreamLimiter
/// slot. Returns `true` on successful spawn (caller should EXIT the worker
/// closure immediately so the dispatch slot frees), `false` on cap hit
/// (caller should respond 503 inline using the existing dispatch slot).
///
/// Caller responsibility:
///   - The Response body MUST be a streaming one (StreamingBody) — passing a
///     fixed-size body works too but defeats the point.
///   - Caller MUST NOT touch `request` or call `respond` after this returns
///     true; both have been moved into the stream thread.
fn spawn_streaming_response<R: std::io::Read + Send + 'static>(
    request: tiny_http::Request,
    response: tiny_http::Response<R>,
    stream_limiter: &Arc<StreamLimiter>,
    peer_ip: std::net::IpAddr,
    metrics: Arc<Metrics>,
    method: String,
    status: u16,
) -> Result<(), tiny_http::Request> {
    let slot = match stream_limiter.acquire(peer_ip) {
        Some(g) => g,
        None => return Err(request),
    };
    let _ = std::thread::Builder::new()
        .name("pylon-stream".into())
        .stack_size(256 * 1024)
        .spawn(move || {
            let _slot = slot;
            let _ = request.respond(response);
            metrics.record_request(&method, status);
        });
    Ok(())
}

// ---------------------------------------------------------------------------
// Streaming body — bridges mpsc::Receiver to std::io::Read for SSE responses
// ---------------------------------------------------------------------------

/// Realtime snapshots are disposable once a client falls behind: disconnecting
/// lets the client resume from its last event ID without retaining an unbounded
/// backlog of stale state.
const SHARD_STREAM_BUFFER_CAPACITY: usize = 8;

/// Function and AI streams are finite and ordered, so their producers apply
/// backpressure when this many chunks are waiting for the HTTP writer.
const FINITE_STREAM_BUFFER_CAPACITY: usize = 64;

/// A streaming response body backed by an MPSC channel.
///
/// When used as the body of a `tiny_http::Response`, it causes the server to
/// write data as it arrives through the channel. Dropping the sender closes
/// the stream (EOF).
pub(crate) struct StreamingBody {
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
    buf: Vec<u8>,
    pos: usize,
}

impl StreamingBody {
    pub(crate) fn new(rx: std::sync::mpsc::Receiver<Vec<u8>>) -> Self {
        Self {
            rx,
            buf: Vec::new(),
            pos: 0,
        }
    }
}

fn bounded_stream(capacity: usize) -> (std::sync::mpsc::SyncSender<Vec<u8>>, StreamingBody) {
    let (tx, rx) = std::sync::mpsc::sync_channel(capacity);
    (tx, StreamingBody::new(rx))
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

#[cfg(test)]
mod bounded_stream_tests {
    use super::bounded_stream;
    use std::io::Read;

    #[test]
    fn bounded_stream_applies_backpressure_and_preserves_order() {
        let (tx, mut body) = bounded_stream(1);
        tx.try_send(b"first".to_vec()).unwrap();
        assert!(matches!(
            tx.try_send(b"second".to_vec()),
            Err(std::sync::mpsc::TrySendError::Full(_))
        ));

        let mut first = [0; 5];
        body.read_exact(&mut first).unwrap();
        assert_eq!(&first, b"first");

        tx.try_send(b"second".to_vec()).unwrap();
        drop(tx);
        let mut rest = Vec::new();
        body.read_to_end(&mut rest).unwrap();
        assert_eq!(rest, b"second");
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
    if let Some(srv) = current_server_handle() {
        srv.unblock();
    }
}

/// Global handle to the *current* `tiny_http::Server` so `request_shutdown()`
/// can call `unblock()` without holding a reference — and so the request loop
/// can swap in a freshly-built server after a recv() give-up (see
/// `build_http_server` / the recv loop below) without losing the shutdown wire.
///
/// A `Mutex<Option<_>>` rather than a `OnceLock`: the handle is *replaced* when
/// the listener is rebuilt, which OnceLock's set-once contract forbids.
static SERVER_HANDLE: Mutex<Option<Arc<Server>>> = Mutex::new(None);

/// Snapshot the live server handle (clones the Arc under the lock so callers
/// don't hold the mutex across `unblock()`).
fn current_server_handle() -> Option<Arc<Server>> {
    SERVER_HANDLE.lock().ok().and_then(|g| g.clone())
}

/// Install `srv` as the live server handle, replacing any previous one.
fn set_server_handle(srv: &Arc<Server>) {
    if let Ok(mut g) = SERVER_HANDLE.lock() {
        *g = Some(Arc::clone(srv));
    }
}

/// Build a `tiny_http::Server` bound dual-stack (`[::]:port`), falling back to
/// v4-only (`0.0.0.0:port`) when IPv6 sockets aren't available. Shared by the
/// initial boot and the recv-loop rebuild path so both bind identically.
fn build_http_server(port: u16) -> Result<Arc<Server>, String> {
    // Dual-stack bind. `[::]:port` accepts IPv6 AND (on Linux, by default)
    // IPv4-mapped connections to the same socket. Without this, a v4-only
    // `0.0.0.0:port` bind silently breaks Fly.io — their fly-proxy reaches
    // machines via the private IPv6 net, sees no listener, and reports "no
    // known healthy instances". Falls back to v4-only when v6 isn't available
    // (older test environments without IPv6 socket support).
    let addr = format!("[::]:{port}");
    let server = match Server::http(&addr) {
        Ok(s) => s,
        Err(_) => {
            let v4_addr = format!("0.0.0.0:{port}");
            Server::http(&v4_addr).map_err(|e| format!("Failed to start server: {e}"))?
        }
    };
    Ok(Arc::new(server))
}

/// Rebuild the HTTP listener after `recv()` returned an error that wasn't a
/// shutdown — tiny_http gives up its accept loop and drops its listener after
/// ~64 consecutive `EINVAL`s (e.g. a sustained connection-reset storm on
/// macOS's `[::]` dual-stack socket), which would otherwise silently kill the
/// dev server. We re-bind the same port with bounded exponential backoff and
/// return the new handle. Returns `None` only if a shutdown is requested while
/// we're retrying.
fn rebuild_with_retry(port: u16) -> Option<Arc<Server>> {
    let mut delay_ms = 50u64;
    loop {
        if SHUTDOWN.load(Ordering::Relaxed) {
            return None;
        }
        match build_http_server(port) {
            Ok(server) => {
                tracing::warn!(
                    "HTTP listener rebuilt on port {port} after recv() give-up \
                     (connection-reset storm or transient accept error)"
                );
                return Some(server);
            }
            Err(e) => {
                // Bind may transiently fail while the old socket lingers in
                // TIME_WAIT. Back off (capped at 2s) and keep trying — the dev
                // server staying up is worth the wait.
                tracing::warn!(
                    "HTTP listener rebuild on port {port} failed ({e}); \
                     retrying in {delay_ms}ms"
                );
                std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                delay_ms = (delay_ms * 2).min(2000);
            }
        }
    }
}

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
/// Priority list of client-IP headers (`PYLON_CLIENT_IP_HEADER`, comma-
/// separated, e.g. `cf-connecting-ip,fly-client-ip`). `resolve_client_ip`
/// returns the first one present + valid on the request, so an app reachable
/// BOTH through CloudFlare (real client in `CF-Connecting-IP`) AND directly
/// via Fly (real client in `Fly-Client-IP`) keys per-real-client on either
/// path — a single header can't.
///
/// Default when unset:
///  - On Fly (`FLY_APP_NAME` / `FLY_MACHINE_ID` present): `["fly-client-ip"]`.
///    Fly's proxy is always the last hop and stamps the connecting client
///    there; a client can't forge it, so it's safe with no origin lockdown.
///    This alone turns the rate-limiter from ONE global bucket (the Fly proxy
///    IP every request shares) into per-client. CloudFlare-fronted deploys
///    should prepend `cf-connecting-ip` (Pylon Cloud sets the full chain).
///  - Otherwise: empty → fall through to the X-Forwarded-For + trust-hops
///    logic below (self-hosted behavior unchanged).
///
/// SECURITY: any header earlier than `fly-client-ip` in the chain (e.g.
/// `cf-connecting-ip`) is honored even on a request that reached the origin
/// directly, so it's spoofable unless the origin is reachable ONLY through
/// that edge (Authenticated Origin Pulls / CF-IP allowlist). That's an
/// accepted tradeoff for rate-limit *bucket keying* — the global dispatch +
/// per-IP concurrency caps still bound total load; the spoofer can only
/// scatter their own requests across buckets. For strict per-IP enforcement,
/// lock the origin to the edge or pin a single unspoofable header.
fn client_ip_headers() -> &'static [String] {
    static H: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    H.get_or_init(|| {
        let on_fly = std::env::var_os("FLY_APP_NAME").is_some()
            || std::env::var_os("FLY_MACHINE_ID").is_some();
        parse_client_ip_headers(
            std::env::var("PYLON_CLIENT_IP_HEADER").ok().as_deref(),
            on_fly,
        )
    })
}

/// Pure parse of the client-IP-header chain (split out so it's testable
/// without touching process env). Explicit config always wins; otherwise a
/// Fly deploy defaults to the unspoofable `fly-client-ip`.
fn parse_client_ip_headers(raw: Option<&str>, on_fly: bool) -> Vec<String> {
    let explicit: Vec<String> = raw
        .unwrap_or("")
        .split(',')
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    if !explicit.is_empty() {
        return explicit;
    }
    if on_fly {
        return vec!["fly-client-ip".to_string()];
    }
    Vec::new()
}

#[cfg(test)]
mod client_ip_header_tests {
    use super::parse_client_ip_headers;

    #[test]
    fn explicit_single_header_lowercased() {
        assert_eq!(
            parse_client_ip_headers(Some("CF-Connecting-IP"), false),
            vec!["cf-connecting-ip"]
        );
    }

    #[test]
    fn explicit_chain_preserves_priority_order_trims_blanks() {
        assert_eq!(
            parse_client_ip_headers(Some(" CF-Connecting-IP , , Fly-Client-IP "), true),
            vec!["cf-connecting-ip", "fly-client-ip"]
        );
    }

    #[test]
    fn explicit_overrides_the_fly_default() {
        // Operator pinned a single header — Fly default must NOT be appended.
        assert_eq!(
            parse_client_ip_headers(Some("true-client-ip"), true),
            vec!["true-client-ip"]
        );
    }

    #[test]
    fn unset_on_fly_defaults_to_fly_client_ip() {
        assert_eq!(parse_client_ip_headers(None, true), vec!["fly-client-ip"]);
        assert_eq!(
            parse_client_ip_headers(Some(""), true),
            vec!["fly-client-ip"]
        );
        assert_eq!(
            parse_client_ip_headers(Some("  , "), true),
            vec!["fly-client-ip"]
        );
    }

    #[test]
    fn unset_off_fly_is_empty_preserving_xff_behavior() {
        assert!(parse_client_ip_headers(None, false).is_empty());
        assert!(parse_client_ip_headers(Some(""), false).is_empty());
    }
}

fn resolve_client_ip(request: &tiny_http::Request, trust_proxy_hops: usize) -> String {
    let socket_ip = request
        .remote_addr()
        .map(|a| a.ip().to_string())
        .unwrap_or_default();
    // Edge client-IP header(s) take precedence, tried in priority order (e.g.
    // Cloudflare's CF-Connecting-IP, then Fly-Client-IP). Use the first one
    // present + parseable on this request; a configured header that's absent
    // (a direct hit that bypassed that edge) falls through to the next, then
    // to the X-Forwarded-For logic — never trust a missing value.
    for hdr in client_ip_headers() {
        if let Some(v) = request
            .headers()
            .iter()
            .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(hdr))
            .map(|h| h.value.as_str().trim().to_string())
        {
            if v.parse::<std::net::IpAddr>().is_ok() {
                return v;
            }
        }
    }
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

/// The opt-in canonical host (`PYLON_CANONICAL_HOST`), read + normalized once.
/// When set, a request arriving on the apex↔www counterpart of this host is
/// 308-redirected to it (Vercel-style "redirect non-primary domain to the
/// primary"). Pylon Cloud sets this from the project's canonical custom domain;
/// self-hosted apps that don't set it get no redirect. Lowercased, port
/// stripped; empty/unset → `None`.
fn canonical_host() -> Option<&'static str> {
    static CANONICAL: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    CANONICAL
        .get_or_init(|| {
            std::env::var("PYLON_CANONICAL_HOST")
                .ok()
                .map(|s| {
                    s.trim()
                        .trim_start_matches("https://")
                        .trim_start_matches("http://")
                        .split('/')
                        .next()
                        .unwrap_or("")
                        .split(':')
                        .next()
                        .unwrap_or("")
                        .to_ascii_lowercase()
                })
                .filter(|s| !s.is_empty() && s.contains('.'))
        })
        .as_deref()
}

/// True when `host` is the apex↔www counterpart of `canonical` (they differ
/// only by a leading `www.`), so it should redirect to `canonical`. Both must
/// already be lowercased + port-stripped. Returns false when `host` already IS
/// the canonical host, or is any unrelated host (so `<app>.fly.dev`,
/// `<slug>.pyln.dev`, the Fly health-check host, localhost, and other custom
/// domains are never redirected — only the exact www/apex sibling is).
fn is_apex_www_counterpart(canonical: &str, host: &str) -> bool {
    if host.is_empty() || host == canonical {
        return false;
    }
    match canonical.strip_prefix("www.") {
        // canonical is the www host → redirect its bare apex.
        Some(apex) => host == apex,
        // canonical is the apex → redirect its www sibling.
        None => host.strip_prefix("www.") == Some(canonical),
    }
}

/// Compute the absolute `Location` for an apex↔www redirect, or `None` when no
/// redirect applies (`host` is already canonical or not the sibling). Pure +
/// testable. CRITICAL: `request.url()` is taken verbatim by tiny_http with no
/// leading-`/` guarantee, so a crafted path like `@evil.com/` would make
/// `https://{canonical}{url}` resolve to a FOREIGN host (CWE-601 open
/// redirect). We reuse the in-tree same-origin guard and fall back to the
/// canonical root for any path that isn't a safe relative path.
fn canonical_redirect_target(canonical: &str, host: &str, url: &str) -> Option<String> {
    if !is_apex_www_counterpart(canonical, host) {
        return None;
    }
    let path = if crate::connections::is_safe_relative_redirect(url) {
        url
    } else {
        "/"
    };
    Some(format!("https://{canonical}{path}"))
}

/// Whether a WebSocket upgrade that authenticated via the AMBIENT session
/// cookie may proceed, given its `Origin` header. CSWSH defense: a browser
/// auto-attaches the victim's cookie to a cross-origin WS handshake, so a
/// cookie-authed upgrade must come from a trusted Origin or it could let any
/// website drive an authenticated socket as the victim. Mirrors the CORS
/// reflect allowlist (localhost OR explicit allowlist OR wildcard `*`).
///
/// Absent Origin → NOT trusted (fail closed): real browsers always send
/// Origin on a WS handshake, and this gates ONLY the ambient-cookie path —
/// non-browser/native clients authenticate with an explicit bearer token
/// (Authorization / `bearer.<token>` subprotocol), which is non-ambient and
/// never reaches this check. So a missing Origin on a cookie-authed upgrade
/// is a crafted client trying to dodge the gate, not a legitimate caller.
fn ws_cookie_origin_trusted(origin: Option<&str>, allowlist: &[String]) -> bool {
    match origin {
        Some(o) => {
            pylon_auth::is_localhost_origin(o) || allowlist.iter().any(|a| a == o || a == "*")
        }
        None => false,
    }
}

/// Authorization decision for `GET /api/files/<id>` on an ownership-tracking
/// backend. FAIL CLOSED: serve only when the asset has a recorded owner that
/// matches the caller's user + active tenant. A missing owner (`Ok(None)` — written-but-unconfirmed,
/// or a `store()`'d file with no sidecar) or an unreadable sidecar (`Err`)
/// must NOT serve the bytes (the IDOR this closes). Admin + non-ownership
/// backends are gated by the caller before this is consulted.
fn file_read_authorized(
    owner: &Result<Option<pylon_storage::files::FileOwner>, pylon_storage::files::FileStorageError>,
    caller_user_id: Option<&str>,
    caller_tenant_id: Option<&str>,
) -> bool {
    matches!(owner, Ok(Some(o)) if file_owner_matches(o, caller_user_id, caller_tenant_id))
}

fn file_owner_matches(
    owner: &pylon_storage::files::FileOwner,
    caller_user_id: Option<&str>,
    caller_tenant_id: Option<&str>,
) -> bool {
    Some(owner.user_id.as_str()) == caller_user_id && owner.tenant_id.as_deref() == caller_tenant_id
}

/// Authorization decision for `POST /api/files/confirm` on an ownership-
/// tracking backend. Confirmation is stricter than ordinary file access: the
/// caller must be the exact user + tenant that initialized the upload. There
/// is deliberately no admin bypass because confirmation must never double as
/// an implicit ownership-transfer operation.
fn file_confirm_authorized(
    owner: &Result<Option<pylon_storage::files::FileOwner>, pylon_storage::files::FileStorageError>,
    caller_user_id: Option<&str>,
    caller_tenant_id: Option<&str>,
) -> bool {
    matches!(
        owner,
        Ok(Some(o)) if file_owner_matches(o, caller_user_id, caller_tenant_id)
    )
}

const DEFAULT_UPLOAD_MAX_BYTES: usize = 200 * 1024 * 1024;

fn upload_max_bytes_from(value: Option<&str>) -> usize {
    value
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(DEFAULT_UPLOAD_MAX_BYTES)
}

fn upload_max_bytes() -> usize {
    let configured = std::env::var("PYLON_MAX_UPLOAD_BYTES").ok();
    upload_max_bytes_from(configured.as_deref())
}

fn upload_size_allowed(actual: usize, maximum: usize) -> bool {
    actual <= maximum
}

#[cfg(test)]
mod file_auth_tests {
    use super::{
        ephemeral_sessions_boot_check, file_confirm_authorized, file_read_authorized,
        local_put_owned_by_other, upload_max_bytes_from, upload_size_allowed,
        DEFAULT_UPLOAD_MAX_BYTES,
    };
    use pylon_storage::files::{FileOwner, FileStorageError};

    fn owner(uid: &str) -> Result<Option<FileOwner>, FileStorageError> {
        Ok(Some(FileOwner {
            user_id: uid.into(),
            tenant_id: None,
        }))
    }

    fn tenant_owner(
        uid: &str,
        tenant: Option<&str>,
    ) -> Result<Option<FileOwner>, FileStorageError> {
        Ok(Some(FileOwner {
            user_id: uid.into(),
            tenant_id: tenant.map(str::to_owned),
        }))
    }

    #[test]
    fn file_confirm_authorized_requires_exact_initiator() {
        assert!(file_confirm_authorized(
            &tenant_owner("u1", Some("t1")),
            Some("u1"),
            Some("t1")
        ));
        assert!(file_confirm_authorized(
            &tenant_owner("u1", None),
            Some("u1"),
            None
        ));
        assert!(!file_confirm_authorized(
            &tenant_owner("u2", Some("t1")),
            Some("u1"),
            Some("t1")
        ));
        assert!(!file_confirm_authorized(
            &tenant_owner("u1", Some("t2")),
            Some("u1"),
            Some("t1")
        ));
        assert!(!file_confirm_authorized(
            &tenant_owner("u1", None),
            None,
            None
        ));
        assert!(!file_confirm_authorized(&Ok(None), Some("u1"), None));
        assert!(!file_confirm_authorized(
            &Err(FileStorageError {
                code: "IO".into(),
                message: "boom".into(),
            }),
            Some("u1"),
            None,
        ));
    }

    #[test]
    fn upload_max_parser_preserves_default_and_valid_overrides() {
        assert_eq!(upload_max_bytes_from(None), DEFAULT_UPLOAD_MAX_BYTES);
        assert_eq!(upload_max_bytes_from(Some("123")), 123);
        assert_eq!(upload_max_bytes_from(Some("0")), 0);
        assert_eq!(
            upload_max_bytes_from(Some("invalid")),
            DEFAULT_UPLOAD_MAX_BYTES
        );
        assert_eq!(
            upload_max_bytes_from(Some("999999999999999999999999999999999999999")),
            DEFAULT_UPLOAD_MAX_BYTES
        );
    }

    #[test]
    fn upload_max_accepts_boundary_and_rejects_larger_actual_size() {
        assert!(upload_size_allowed(0, 0));
        assert!(upload_size_allowed(100, 100));
        assert!(!upload_size_allowed(101, 100));
    }

    #[test]
    fn file_get_fails_closed_on_missing_or_unreadable_owner() {
        // Owner present + matches → allowed.
        assert!(file_read_authorized(&owner("u1"), Some("u1"), None));
        // Owner present + mismatch → denied.
        assert!(!file_read_authorized(&owner("u2"), Some("u1"), None));
        // NO owner recorded (Ok(None)) → MUST deny (the IDOR — pre-fix served).
        assert!(!file_read_authorized(&Ok(None), Some("u1"), None));
        // Unreadable sidecar (Err) → MUST deny.
        assert!(!file_read_authorized(
            &Err(FileStorageError {
                code: "IO".into(),
                message: "boom".into(),
            }),
            Some("u1"),
            None,
        ));
        // Anonymous caller against an owned asset → denied.
        assert!(!file_read_authorized(&owner("u1"), None, None));
        // Anonymous caller against an unowned asset → still denied (no
        // `None == None` foot-gun serving ownerless files to anon).
        assert!(!file_read_authorized(&Ok(None), None, None));

        assert!(!file_read_authorized(
            &tenant_owner("u1", Some("tenant-a")),
            Some("u1"),
            Some("tenant-b"),
        ));
    }

    #[test]
    fn local_put_refuses_overwrite_of_another_users_asset() {
        // Owned by a DIFFERENT user → MUST refuse (the overwrite hole —
        // pre-fix local-put wrote bytes with no owner check at all).
        assert!(local_put_owned_by_other(
            &owner("victim"),
            "attacker",
            None,
            false
        ));
        // Owned by the caller → allowed (the legitimate upload completing).
        assert!(!local_put_owned_by_other(&owner("me"), "me", None, false));
        // Admin may overwrite anyone's asset (matches DELETE's admin bypass).
        assert!(!local_put_owned_by_other(
            &owner("victim"),
            "admin",
            None,
            true
        ));
        // Unowned (Ok(None)) → allowed; the caller claims it on write.
        assert!(!local_put_owned_by_other(&Ok(None), "me", None, false));
        // Unreadable sidecar (Err) → fail CLOSED, refuse the write.
        assert!(local_put_owned_by_other(
            &Err(FileStorageError {
                code: "IO".into(),
                message: "boom".into(),
            }),
            "me",
            None,
            false,
        ));

        assert!(local_put_owned_by_other(
            &tenant_owner("me", Some("tenant-a")),
            "me",
            Some("tenant-b"),
            false,
        ));
    }

    #[test]
    fn file_fences_scope_admin_acting_within_a_tenant() {
        // Regression for the #354/#355 class on the FILE paths: every file
        // owner-check fence (local-put WRITE, DELETE, GET — server.rs — and the
        // router get_file path) now consumes `is_unscoped_admin()`, NOT bare
        // `is_admin`. An admin with an active tenant must be held to the owner
        // check; only an admin with NO active tenant keeps the cross-owner
        // bypass. Pre-fix these fences used bare `is_admin`, letting an
        // admin-with-tenant read/overwrite/delete any user's file on the local
        // backend — bypassing the tenant-impersonation scoping #354 established.
        use pylon_auth::AuthContext;

        // Unscoped admin (no active tenant) — full bypass preserved.
        let admin = AuthContext::admin();
        assert!(admin.is_unscoped_admin());
        // WRITE: may overwrite anyone's asset.
        assert!(!local_put_owned_by_other(
            &owner("victim"),
            "anyone",
            None,
            admin.is_unscoped_admin()
        ));

        // Admin acting WITHIN a tenant — must be SCOPED like a normal user.
        let admin_scoped = AuthContext {
            tenant_id: Some("t-acme".into()),
            ..AuthContext::admin()
        };
        assert!(!admin_scoped.is_unscoped_admin());
        // WRITE: cannot overwrite a victim's asset.
        assert!(local_put_owned_by_other(
            &owner("victim"),
            admin_scoped.user_id.as_deref().unwrap_or_default(),
            admin_scoped.tenant_id.as_deref(),
            admin_scoped.is_unscoped_admin(),
        ));
        // READ: the GET gate is `requires_owner_check() && !is_unscoped_admin()`,
        // so with the tenant active the owner check runs and a non-owned asset
        // is denied.
        assert!(!file_read_authorized(
            &owner("victim"),
            admin_scoped.user_id.as_deref(),
            admin_scoped.tenant_id.as_deref(),
        ));
    }

    #[test]
    fn in_memory_sessions_refused_in_prod_allowed_in_dev_or_optin() {
        // Production boot (not dev) with NO persistent backend + no explicit
        // opt-in → MUST refuse (the silent footgun: sessions evaporate on
        // restart / aren't shared across replicas). Pre-fix this returned an
        // in-memory store with no warning.
        assert!(ephemeral_sessions_boot_check(false, false).is_err());
        // Dev boot → ephemeral is fine (matches the in-memory datastore).
        assert!(ephemeral_sessions_boot_check(false, true).is_ok());
        // Explicit opt-in (PYLON_SESSION_IN_MEMORY=1) → allowed even in prod.
        assert!(ephemeral_sessions_boot_check(true, false).is_ok());
        assert!(ephemeral_sessions_boot_check(true, true).is_ok());
    }
}

#[cfg(test)]
mod ws_origin_tests {
    use super::ws_cookie_origin_trusted;

    #[test]
    fn cookie_authed_ws_requires_a_trusted_origin() {
        let allow = vec!["https://app.example.com".to_string()];

        // Same-origin (in the allowlist) → trusted.
        assert!(ws_cookie_origin_trusted(
            Some("https://app.example.com"),
            &allow
        ));
        // Localhost dev origins → trusted (mirrors CORS reflection).
        assert!(ws_cookie_origin_trusted(
            Some("http://localhost:3000"),
            &allow
        ));
        assert!(ws_cookie_origin_trusted(
            Some("http://127.0.0.1:5173"),
            &allow
        ));

        // The CSWSH case: an attacker page's Origin is NOT in the allowlist
        // → rejected, so its (browser-attached) cookie can't drive the socket.
        assert!(!ws_cookie_origin_trusted(
            Some("https://evil.example"),
            &allow
        ));
        // A near-miss sibling subdomain is still untrusted unless allowlisted.
        assert!(!ws_cookie_origin_trusted(
            Some("https://evil.app.example.com"),
            &allow
        ));
        // Absent Origin on a cookie-authed upgrade → fail closed.
        assert!(!ws_cookie_origin_trusted(None, &allow));

        // Wildcard allowlist (dev only — prod boot refuses `*`) → reflect any.
        let star = vec!["*".to_string()];
        assert!(ws_cookie_origin_trusted(
            Some("https://anything.test"),
            &star
        ));
        // …but absent Origin is still fail-closed even under wildcard.
        assert!(!ws_cookie_origin_trusted(None, &star));
    }
}

#[cfg(test)]
mod canonical_redirect_tests {
    use super::{canonical_redirect_target, is_apex_www_counterpart};

    #[test]
    fn apex_canonical_redirects_www_only() {
        let c = "notbehind.com";
        assert!(is_apex_www_counterpart(c, "www.notbehind.com"));
        assert!(!is_apex_www_counterpart(c, "notbehind.com")); // already canonical
        assert!(!is_apex_www_counterpart(c, "app.notbehind.com")); // other subdomain
        assert!(!is_apex_www_counterpart(c, "pylon-notbehind.fly.dev"));
        assert!(!is_apex_www_counterpart(c, "notbehind.pyln.dev"));
        assert!(!is_apex_www_counterpart(c, "evil.com"));
        assert!(!is_apex_www_counterpart(c, ""));
    }

    #[test]
    fn www_canonical_redirects_apex_only() {
        let c = "www.notbehind.com";
        assert!(is_apex_www_counterpart(c, "notbehind.com"));
        assert!(!is_apex_www_counterpart(c, "www.notbehind.com")); // already canonical
        assert!(!is_apex_www_counterpart(c, "app.notbehind.com"));
        assert!(!is_apex_www_counterpart(c, "www.evil.com"));
    }

    #[test]
    fn target_preserves_safe_path_and_query() {
        assert_eq!(
            canonical_redirect_target("notbehind.com", "www.notbehind.com", "/m/1?x=2"),
            Some("https://notbehind.com/m/1?x=2".to_string()),
        );
        assert_eq!(
            canonical_redirect_target("www.notbehind.com", "notbehind.com", "/"),
            Some("https://www.notbehind.com/".to_string()),
        );
    }

    #[test]
    fn target_none_when_not_sibling_or_already_canonical() {
        assert_eq!(
            canonical_redirect_target("notbehind.com", "notbehind.com", "/a"),
            None,
        );
        assert_eq!(
            canonical_redirect_target("notbehind.com", "app.notbehind.com", "/a"),
            None,
        );
    }

    #[test]
    fn target_blocks_open_redirect_paths_falling_back_to_root() {
        // CWE-601: a crafted path must NOT escape the canonical host.
        for evil in ["@evil.com/", "//evil.com", "/\\evil.com", "", "evil.com/"] {
            assert_eq!(
                canonical_redirect_target("notbehind.com", "www.notbehind.com", evil),
                Some("https://notbehind.com/".to_string()),
                "path {evil:?} must fall back to canonical root, not escape origin",
            );
        }
    }
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
    start_server(runtime, port, plugins, None, None)
}

/// Start the dev server with plugins and a shard registry for real-time
/// simulations (games, MMO zones, etc.). Blocks until shutdown.
pub fn start_with_shards(
    runtime: Arc<Runtime>,
    port: u16,
    plugins: Option<Arc<PluginRegistry>>,
    shard_registry: Arc<dyn pylon_realtime::DynShardRegistry>,
) -> Result<(), String> {
    start_server(runtime, port, plugins, Some(shard_registry), None)
}

/// Test-only entrypoint: start the server with a caller-supplied `FnOps`
/// wired into the frontend dispatcher (SSR pages + `route.ts` form handlers),
/// skipping the Bun runner spawn. Lets integration tests drive the real HTTP
/// request loop — routing, the CSRF gate, and form dispatch — against a stub
/// runtime instead of a live Bun process. Blocks until shutdown.
#[doc(hidden)]
pub fn start_server_for_test_with_fn_ops(
    runtime: Arc<Runtime>,
    port: u16,
    fn_ops: Arc<dyn pylon_router::FnOps>,
) -> Result<(), String> {
    start_server(runtime, port, None, None, Some(fn_ops))
}

/// Lift `is_admin` on a resolved auth context via the two admin-designation
/// paths, persisting an env-allowlisted promotion to the User row when an
/// `adminField` is configured. Shared by the main HTTP request handler AND
/// `crate::frontend::resolve_request_auth` (the SSR/frontend path) so both
/// resolve `is_admin` identically for a given cookie + admin config.
///
/// Two independent paths feed `is_admin`; either one is enough to lift it:
///
/// 1. `auth.user.admin_field` (manifest config) — load the User row, check the
///    configured boolean/string/role-array field, lift `is_admin` when truthy.
///    Apps with a `User.isAdmin` column (or `"admin"` role) configure this so
///    platform admins sign in with their regular account and Studio respects
///    the role.
///
/// 2. `PYLON_ADMIN_EMAILS` env var — comma-separated allowlist of verified
///    emails. A matched user gets `is_admin` lifted AND (when `admin_field` is
///    configured) the User row's flag flipped to true so the promotion
///    survives env-var removal. Operator-friendly: every Pylon app gets
///    "designate admins by email" without writing app code.
///
/// SECURITY: API-key contexts are excluded from admin promotion. A leaked /
/// scoped `pk.*` token for an admin-allowlisted user must NOT escalate to
/// `is_admin` — the API-key issuer minted that token with a specific scope,
/// not "act as this user across every privileged route" (2026-05-09 codex
/// audit). No-op for anonymous, API-key, or already-admin contexts.
pub(crate) fn lift_admin(runtime: &Runtime, auth_ctx: &mut pylon_auth::AuthContext) {
    if auth_ctx.is_admin || auth_ctx.is_api_key_auth() {
        return;
    }
    let Some(uid) = auth_ctx.user_id.clone() else {
        return;
    };
    let user_entity = runtime.manifest().auth.user.entity.clone();
    let admin_field = runtime
        .manifest()
        .auth
        .user
        .admin_field
        .clone()
        .filter(|f| !f.is_empty());
    let row = runtime.get_by_id(&user_entity, &uid).ok().flatten();

    // Path 1: admin_field on the User row.
    if let (Some(field), Some(row)) = (&admin_field, &row) {
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

    // Path 2: PYLON_ADMIN_EMAILS allowlist. Only fires when the User row
    // carries a verified email — we never promote on an unverified claim.
    // Match is case-insensitive and tolerates whitespace around entries.
    if auth_ctx.is_admin {
        return;
    }
    let allow: Vec<String> = std::env::var("PYLON_ADMIN_EMAILS")
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    if allow.is_empty() {
        return;
    }
    let Some(row) = &row else {
        return;
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
            // Treat any non-empty timestamp / "true" / "1" string as verified
            // (consistent with how the rest of the framework reads "verified"
            // timestamps).
            serde_json::Value::String(s) => !s.is_empty() && !s.eq_ignore_ascii_case("false"),
            serde_json::Value::Number(n) => n.as_i64().map(|n| n != 0).unwrap_or(false),
            serde_json::Value::Null => false,
            _ => false,
        })
        .unwrap_or(false);
    if !verified {
        return;
    }
    let Some(email) = email else {
        return;
    };
    if !allow.iter().any(|a| a == &email) {
        return;
    }
    auth_ctx.is_admin = true;
    // Persist when an admin_field is configured. Best-effort — a write failure
    // shouldn't fail the request, we already lifted in-memory. The spec says
    // "removing an email from the env doesn't demote", and the persisted flag
    // is what makes that true.
    if let Some(field) = &admin_field {
        let already_truthy = row
            .get(field.as_str())
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !already_truthy {
            let payload = serde_json::json!({ field: true });
            let _ = runtime.update(&user_entity, &uid, &payload);
            tracing::info!(
                target: "pylon::auth",
                "[admin-emails] promoted {email} via PYLON_ADMIN_EMAILS"
            );
        }
    }
}

/// Build the boot-time change log: seq provider + persistent on-disk
/// store + per-entity seed. This is the exact wiring `start_server`
/// uses; it's a standalone function so the lifecycle test harness
/// (`crate::lifecycle_scenario`) drives the real code path rather than
/// a reimplementation. Every restart/reload correctness P0 (the
/// reentrant-write_conn deadlock, the per-database seed gate that hid
/// new entities, the hydrate-from-disk path) lives somewhere in here,
/// so a test that reimplemented the wiring would test the wrong thing.
///
/// Returns an `Arc<ChangeLog>` ready to hand to the WS hub + routes.
pub(crate) fn build_persistent_change_log(runtime: &Arc<Runtime>) -> Arc<ChangeLog> {
    // Postgres backends: wire a global SEQUENCE-backed seq provider so
    // every instance writing to the same database shares one
    // monotonically-increasing seq space. Without this, instance A and
    // instance B can independently mint seq=N for different events;
    // a client connected to one and pulling from the other drops
    // events as "duplicate seq" (codex P1).
    //
    // SQLite backends: wire a _pylon_change_seq-row-backed provider so
    // seqs persist across process restarts. Without this, every deploy
    // resets the in-memory seq counter to 0 + seeds to ~entity_count;
    // any cached client cursor ahead of the new seed range fires a
    // permanent 410 RESYNC_REQUIRED → reset → re-pull (visible churn
    // on every deploy). Persisted seqs are strictly monotonic across
    // boots so the 410 path only ever fires under genuine retention
    // eviction.
    let mut change_log_builder = ChangeLog::new();
    if runtime.is_postgres() {
        if let Err(e) = runtime.bootstrap_global_change_seq() {
            tracing::warn!(
                "[change_log] failed to bootstrap PG SEQUENCE; falling back to per-instance seq: {}",
                e.message
            );
        } else {
            let rt_for_provider = Arc::clone(runtime);
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
    } else if !runtime.is_in_memory() {
        // File-backed SQLite: persist a high-water mark on disk via a
        // background thread so seqs stay monotonic across process
        // restarts (no 410 RESYNC_REQUIRED storm on redeploy).
        //
        // Persistence MUST stay off the hot path: persisting INLINE inside
        // the SeqProvider closure — which the mutation pipeline calls while
        // holding write_conn — recursively acquires write_conn and deadlocks
        // every mutation. So an in-memory atomic issues seqs from a reserved
        // chunk, and a bg thread writes the next chunk's high-water
        // mark to disk before that chunk gets used. The bg thread has
        // its own write_conn lock acquisition, queued behind any
        // active mutation tx — no recursive lock, no deadlock.
        // See crates/runtime/src/seq_allocator.rs for the full design.
        if let Err(e) = runtime.bootstrap_sqlite_change_seq() {
            tracing::warn!(
                "[change_log] failed to bootstrap SQLite _pylon_change_seq; falling back to per-instance seq: {}",
                e.message
            );
        } else {
            match crate::seq_allocator::SqliteSeqAllocator::new(Arc::clone(runtime)) {
                Some(allocator) => {
                    let allocator = std::sync::Arc::new(allocator);
                    // Seed the change-log snapshot cursor to the allocator's
                    // FLOOR (the value its first `next()` issues one above),
                    // NOT the persisted `_pylon_change_seq` value — that was
                    // already bumped by `+chunk_size` when the allocator
                    // reserved its first chunk inside `new()`. Using the
                    // post-reservation value seeds the cursor a full chunk
                    // (1000) above the seqs we actually issue (1, 2, 3, …), so
                    // every delta in the first chunk lands below the client's
                    // cursor and gets dropped as "already seen" — live queries
                    // silently never update on SQLite (`pylon dev`) until 1000+
                    // writes. (Postgres reads `initial` before wiring, so it's
                    // unaffected.)
                    let initial = allocator.floor_seq();
                    let alloc_for_provider = std::sync::Arc::clone(&allocator);
                    let provider: pylon_sync::SeqProvider =
                        std::sync::Arc::new(move || alloc_for_provider.next());
                    change_log_builder = change_log_builder
                        .with_seq(provider)
                        .with_initial_seq(initial);
                    // Hold the allocator past this scope so the bg
                    // thread isn't dropped immediately. The closure
                    // holds its own Arc to the allocator; this binding
                    // keeps the bg thread alive for the runtime's
                    // lifetime by escaping into the ChangeLog's seq
                    // provider field.
                    std::mem::forget(allocator);
                    tracing::info!(
                        "[change_log] using SQLite _pylon_change_seq (initial reservation past = {initial})"
                    );
                }
                None => {
                    tracing::warn!(
                        "[change_log] failed to initialize SqliteSeqAllocator; falling back to per-instance seq"
                    );
                }
            }
        }
    }

    // Attach the SQLite-backed persistent change log so deltas
    // survive restarts. Without this, the in-memory ring buffer
    // resets on every process boot; clients with a persisted
    // cursor get a `ResyncRequired` and lose any events that
    // landed post-restart but pre-reconnect.
    //
    // Bootstrap is idempotent (CREATE TABLE IF NOT EXISTS). The
    // hydrate-from-disk inside `with_store` seeds the in-memory
    // ring with the most recent `capacity` events ordered ASC, so
    // post-boot pulls cover the same window the previous process
    // was serving.
    let mut change_log_persistent = false;
    enum PersistedBackend {
        None,
        Sqlite,
        Postgres,
    }
    let mut persisted_backend = PersistedBackend::None;
    if runtime.is_postgres() {
        // Pg deployment: persistence lives in `pylon_change_log`.
        // Without this, every rolling deploy silently desyncs every
        // connected client (the cluster-path equivalent of the SQLite
        // high-water-mark persistence above).
        if let Err(e) = runtime.bootstrap_pg_change_log() {
            tracing::warn!(
                "[change_log] failed to bootstrap pylon_change_log; PG persistent log disabled: {}",
                e.message
            );
        } else {
            use pylon_sync::ChangeLogStore as _;
            let store = std::sync::Arc::new(crate::change_log_store::PgChangeLogStore::new(
                Arc::clone(runtime),
            ));
            let hydrated_any = !store.load_recent(1).is_empty();
            change_log_builder = change_log_builder.with_store(store);
            change_log_persistent = true;
            persisted_backend = PersistedBackend::Postgres;
            if hydrated_any {
                tracing::info!("[change_log] hydrated in-memory ring from pylon_change_log (PG)");
            } else {
                tracing::info!("[change_log] using PG pylon_change_log (empty on first boot)");
            }
        }
    } else if !runtime.is_in_memory() {
        if let Err(e) = runtime.bootstrap_sqlite_change_log() {
            tracing::warn!(
                "[change_log] failed to bootstrap _pylon_change_log; persistent log disabled: {}",
                e.message
            );
        } else {
            use pylon_sync::ChangeLogStore as _;
            let store = std::sync::Arc::new(crate::change_log_store::SqliteChangeLogStore::new(
                Arc::clone(runtime),
            ));
            let hydrated_any = !store.load_recent(1).is_empty();
            change_log_builder = change_log_builder.with_store(store);
            change_log_persistent = true;
            persisted_backend = PersistedBackend::Sqlite;
            if hydrated_any {
                tracing::info!("[change_log] hydrated in-memory ring from _pylon_change_log");
            } else {
                tracing::info!("[change_log] using SQLite _pylon_change_log (empty on first boot)");
            }
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
    //
    // PER-ENTITY gating when persistence is on: skip seeding for
    // entities whose rows are already covered by events in
    // `_pylon_change_log`, but DO seed entities that are absent from
    // the persisted log (e.g. a new entity added to the manifest
    // after the previous boot, a table restored from a snapshot,
    // an entity migrated in via out-of-band SQL). A binary
    // had-any-persisted-event gate would silently make new
    // entities invisible to /api/sync/pull until the next write.
    for entity in runtime.manifest().entities.iter() {
        let already_seeded = match persisted_backend {
            PersistedBackend::Sqlite => runtime.sqlite_change_log_has_entity(&entity.name),
            PersistedBackend::Postgres => runtime.pg_change_log_has_entity(&entity.name),
            PersistedBackend::None => false,
        };
        let _ = change_log_persistent;
        if already_seeded {
            continue;
        }
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

    change_log
}

#[cfg(test)]
mod change_log_wiring_tests {
    use super::build_persistent_change_log;
    use crate::Runtime;
    use pylon_sync::ChangeKind;
    use std::sync::Arc;

    fn minimal_manifest() -> pylon_kernel::AppManifest {
        use pylon_kernel::*;
        AppManifest {
            manifest_version: 1,
            name: "t".into(),
            version: "0.1.0".into(),
            entities: vec![ManifestEntity {
                name: "T".into(),
                fields: vec![ManifestField {
                    name: "x".into(),
                    field_type: "string".into(),
                    optional: false,
                    unique: false,
                    crdt: None,
                    server_only: false,
                    readonly: false,
                    default: None,
                    enum_values: None,
                    encrypted: false,
                }],
                indexes: vec![],
                relations: vec![],
                search: None,
                crdt: true,
                sync: true,
            }],
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            auth: Default::default(),
            llm: Default::default(),
            connections: vec![],
            crons: vec![],
            fonts: vec![],
        }
    }

    /// The end-to-end regression the SQLite seq-floor bug needed and didn't
    /// have. It exercises the REAL `serve()` wiring — `build_persistent_change_log`
    /// — not a hand-rolled copy, so reverting `server.rs` back to seeding the
    /// snapshot cursor from the post-reservation `_pylon_change_seq` value trips
    /// it. Invariant: a freshly-connecting client adopts `current_seq()` as its
    /// cursor on the first pull; the next write MUST land ABOVE that cursor, or
    /// every live-query delta in the first reservation chunk (1000 writes) is
    /// silently dropped as "already seen" — the bug that made realtime never
    /// update on `pylon dev` and on SQLite-backed Pylon Cloud apps.
    #[test]
    fn sqlite_change_log_first_delta_lands_above_the_client_snapshot_cursor() {
        let tmp = std::env::temp_dir().join("pylon-changelog-wiring.sqlite");
        let _ = std::fs::remove_file(&tmp);
        let rt = Arc::new(Runtime::open(tmp.to_str().unwrap(), minimal_manifest()).unwrap());

        // Wire the change log exactly as the server does at boot.
        let change_log = build_persistent_change_log(&rt);

        // The cursor a client adopts from its initial snapshot pull.
        let snapshot_cursor = change_log.current_seq();
        let first_seq = change_log.append(
            "T",
            "row-1",
            ChangeKind::Insert,
            Some(serde_json::json!({ "x": 1 })),
        );

        assert!(
            first_seq > snapshot_cursor,
            "first SQLite change-log delta seq ({first_seq}) must be ABOVE the client \
             snapshot cursor ({snapshot_cursor}); below it, live-query deltas in the \
             first reservation chunk are dropped and realtime silently never updates"
        );
        let _ = std::fs::remove_file(&tmp);
    }
}

fn start_server(
    runtime: Arc<Runtime>,
    port: u16,
    plugins: Option<Arc<PluginRegistry>>,
    shard_registry: Option<Arc<dyn pylon_realtime::DynShardRegistry>>,
    // Test-only: when set, this `FnOps` backs the frontend dispatcher (SSR +
    // form handlers) and the Bun runner pool spawn is skipped. `None` in prod.
    fn_ops_override: Option<Arc<dyn pylon_router::FnOps>>,
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

    // Bind the HTTP listener (dual-stack `[::]`, v4-only fallback). `mut`
    // because the recv loop below rebuilds it in place if tiny_http gives up
    // its accept loop under a connection-reset storm (see `rebuild_with_retry`).
    let mut server = build_http_server(port)?;

    // Stash a handle so `request_shutdown()` can unblock the loop.
    set_server_handle(&server);

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
    // Build the persistent change log: seq provider (PG SEQUENCE /
    // SQLite high-water allocator) + on-disk store (hydrate the
    // in-memory ring) + per-entity seed. Extracted so the lifecycle
    // test harness exercises the exact wiring a real boot does — the
    // four restart/reload P0s all lived in this seam, so the harness
    // must drive it directly, not a reimplementation. See
    // `build_persistent_change_log`.
    let change_log = build_persistent_change_log(&runtime);

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
        // Auto-stamp any field declared `field.X().owner()`. This is what
        // makes optimistic, local-first writes the DEFAULT for owned data:
        // the client `db.insert`s with its own id (instant local paint)
        // and the server overwrites/validates the owner from the session,
        // so a forged owner id is rejected. Same posture as tenant_scope —
        // drop the annotation on the field and isolation is automatic.
        reg.register(Arc::new(
            pylon_plugin::builtin::owner_stamp::OwnerStampPlugin::from_manifest(runtime.manifest()),
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

    // Cron leadership: multi-machine Postgres deploys elect ONE cron
    // firer via an advisory lock; SQLite is single-machine by
    // definition and skips the machinery entirely.
    let scheduler_leadership: Arc<dyn crate::leader::Leadership> =
        match std::env::var("DATABASE_URL")
            .ok()
            .filter(|u| u.starts_with("postgres://") || u.starts_with("postgresql://"))
        {
            Some(url) => {
                tracing::info!(
                    "[scheduler] postgres mode — contending for cluster cron leadership"
                );
                Arc::new(crate::leader::PgAdvisoryLeader::spawn(&url))
            }
            None => Arc::new(crate::leader::AlwaysLeader),
        };
    let scheduler = Arc::new(Scheduler::with_leadership(
        Arc::clone(&job_queue),
        scheduler_leadership,
    ));
    // Schedule built-in cleanup tasks. Pass the REAL handler to `schedule()`,
    // which registers it with the job queue itself — a separate
    // `job_queue.register(name, real)` BEFORE the schedule call is silently
    // OVERWRITTEN by schedule()'s own registration (last-write-wins HashMap),
    // so the prior pattern (register real handler, then schedule a no-op) made
    // the cache + rooms cleanup never actually run → unbounded growth.
    {
        let cache_ref = Arc::clone(&cache);
        let _ = scheduler.schedule(
            "pylon.cache.cleanup",
            "*/10 * * * *",
            Arc::new(move |_job| {
                cache_ref.cleanup_expired();
                JobResult::Success
            }),
        );
        let rooms_ref = Arc::clone(&room_mgr);
        let _ = scheduler.schedule(
            "pylon.rooms.cleanup",
            "*/5 * * * *",
            Arc::new(move |_job| {
                rooms_ref.cleanup_idle();
                JobResult::Success
            }),
        );
        // Prune the jobs table itself. The cleanups above each leave a
        // `completed` row every run; unpruned they accumulate into thousands of
        // rows + a multi-MB WAL that slows boot. Keep ~1h of history.
        let jq_ref = Arc::clone(&job_queue);
        let _ = scheduler.schedule(
            "pylon.jobs.cleanup",
            "*/15 * * * *",
            Arc::new(move |_job| {
                jq_ref.cleanup_completed_jobs(3600);
                JobResult::Success
            }),
        );
    }

    // NOTE: background workers + the scheduler are intentionally NOT started
    // here. They start AFTER every job handler is registered (built-in cleanup
    // crons above, function handlers in `try_spawn_functions`, and app crons in
    // `register_app_crons` below). Starting them now would let a worker dequeue
    // a RESTORED function job before its handler exists; `fail()` re-enqueues
    // with no delay, so the job can burn all its retries in a few hundred ms
    // and dead-letter before the Bun runner finishes spawning (~1-2s). See the
    // start site after `register_app_crons`.

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

    // Periodically drop idle per-IP buckets from the rate limiters. `check()`
    // prunes timestamps within each bucket, but only `cleanup()` removes the
    // now-empty IP keys — without this the bucket map grows one entry per
    // distinct client IP forever. The scheduler reads its task list each tick,
    // so registering after `start()` (the limiters are built post-start) works.
    {
        let rl = Arc::clone(&rate_limiter);
        let frl = Arc::clone(&fn_rate_limiter);
        let arl = Arc::clone(&ai_rate_limiter);
        let _ = scheduler.schedule(
            "pylon.ratelimit.cleanup",
            "*/10 * * * *",
            Arc::new(move |_job| {
                rl.cleanup();
                frl.cleanup();
                arl.cleanup();
                JobResult::Success
            }),
        );
    }

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
    // App-facing email: backs the function runner's ctx.email.send hook,
    // reading PYLON_EMAIL_* only (arbitrary recipient + body → must be the
    // customer's own provider). One identity across the FFI boundary into
    // the runner for cleaner logs + future request batching. Auth-flow
    // email (codes/reset/invites) is a SEPARATE channel built per-request
    // via EmailAdapter::for_auth() so a shared platform auth key never
    // backs app code's ctx.email.
    let fn_email_adapter = Arc::new(crate::datastore::EmailAdapter::from_env());
    // A test override (see `start_server_for_test_with_fn_ops`) supplies the
    // frontend dispatcher's FnOps directly, so skip spawning the Bun runner
    // pool — the integration test drives the real request loop (routing, CSRF
    // gate, form dispatch) without a live Bun process.
    let t_runner = std::time::Instant::now();
    let fn_ops_maybe = if fn_ops_override.is_some() {
        None
    } else {
        crate::datastore::try_spawn_functions(
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
        )
    };
    if std::env::var("PYLON_DEV_TIMING").is_ok() {
        eprintln!(
            "[dev-timing] server runner_pool_ready {:.1}ms",
            t_runner.elapsed().as_secs_f64() * 1000.0
        );
    }

    // Reactive registry needs FnOps to invoke handlers for initial
    // run + re-run. Wire it now that fn_ops is built, then spawn the
    // re-runner thread. If functions aren't available (no functions/
    // dir, no Bun), reactive subscribes will fail at the FnOps lookup
    // — that's the correct behavior. The registry itself still works
    // for the rest of the codebase that holds an Arc.
    if let Some(ref ops) = fn_ops_maybe {
        let dyn_ops: Arc<dyn pylon_router::FnOps> = Arc::clone(ops) as Arc<dyn pylon_router::FnOps>;
        reactive_registry.set_fn_ops(dyn_ops);
        // Register app-declared cron jobs (manifest.crons) now that functions
        // are loaded. The scheduler is already running (started above); adding
        // tasks to it is picked up on the next tick.
        crate::datastore::register_app_crons(&scheduler, ops, &runtime.manifest().crons);
    }

    // Now that EVERY job handler is registered (built-in cleanup crons,
    // function handlers from `try_spawn_functions`, and app crons), start the
    // background workers and the scheduler. Doing this here — not at queue
    // construction — closes the boot race where a restored function job could
    // dead-letter against a not-yet-registered handler.
    let _worker_handles: Vec<_> = (0..2)
        .map(|i| {
            let w = Worker::new(Arc::clone(&job_queue), &format!("worker-{i}"));
            w.start()
        })
        .collect();
    let _scheduler_handle = Arc::clone(&scheduler).start();

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
    // The app's own public origin (PYLON_PUBLIC_URL) is always trusted:
    // it's the origin this server advertises for its OAuth callbacks and
    // email links, so redirecting the user back to it can't be an open
    // redirect. Without this, a deploy that gains a custom domain (Pylon
    // Cloud rewrites PYLON_PUBLIC_URL when one goes ready) 403s
    // UNTRUSTED_REDIRECT on absolute callbacks to its own site unless the
    // operator also hand-maintains PYLON_TRUSTED_ORIGINS.
    if let Ok(public) = std::env::var("PYLON_PUBLIC_URL") {
        if let Some(origin) = pylon_auth::http_origin_of(&public) {
            if !combined.contains(&origin) {
                combined.push(origin);
            }
        }
    }
    // A scheme-less entry ("app.example.com") never matches anything —
    // the matcher requires a parseable scheme://host[:port]. Warn at boot
    // instead of letting the operator debug a gate that silently rejects
    // every sign-in.
    for o in &combined {
        if !pylon_auth::is_valid_trusted_origin(o) {
            tracing::warn!(
                "[auth] trusted origin {o:?} is not a valid scheme://host[:port] origin \
                 and will never match — did you mean \"https://{o}\"?"
            );
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
        let rooms_bridge: Arc<dyn crate::ws::RoomBridge> = Arc::clone(&room_mgr) as _;
        std::thread::spawn(move || {
            crate::ws::start_ws_server(
                hub,
                auth,
                ws_port,
                Some(fetcher),
                Some(reactive),
                Some(rooms_bridge),
            );
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
        // The dedicated SSE port authenticates every connection (401 for
        // anonymous callers in prod) and per-client policy-filters every change
        // event — see `sse::handle_sse_connection`. The ONLY way it serves
        // anonymous clients is when the operator sets
        // PYLON_SSE_PORT_ACKNOWLEDGE_UNAUTH=1, which DISABLES the auth gate. So
        // warn loudly in THAT (and only that) case.
        if sse_unauth_warning_warranted(in_prod_for_sse, acknowledged) {
            tracing::warn!(
                "[sse] PYLON_SSE_PORT_ACKNOWLEDGE_UNAUTH=1 — the dedicated SSE port \
                 :{sse_port} accepts UNAUTHENTICATED clients (the per-connection auth gate \
                 is disabled). Authenticated clients are still per-client policy-filtered, \
                 but anonymous clients receive every change event your read policies expose \
                 to anonymous callers. Unset the flag to require auth, or set \
                 PYLON_SSE_PORT_DISABLE=1 if you don't use this port (most deploys don't)."
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

    // Every backend that bootstraps tables has run by now — release the
    // cluster boot-DDL lock so a peer machine's boot can proceed.
    // No-op on SQLite (never acquired).
    crate::pg_boot_guard::release();

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
    let stream_limiter = StreamLimiter::new(dispatch_limiter.global_cap);
    tracing::info!(
        "  HTTP cap: dispatch={} per_ip={}, stream={} per_ip={}",
        dispatch_limiter.global_cap,
        std::env::var("PYLON_HTTP_PER_IP_MAX")
            .ok()
            .unwrap_or_else(|| "64".to_string()),
        stream_limiter.global_cap,
        std::env::var("PYLON_STREAM_PER_IP_MAX")
            .ok()
            .unwrap_or_else(|| "16".to_string()),
    );

    // Resolve the unified full-stack server's frontend config. When the
    // project has a built `web/dist/` (or `PYLON_FRONTEND_DEV_PROXY` is
    // set for dev), non-API GETs serve the SPA on the same port. When
    // neither is configured the runtime stays API-only — backwards
    // compatible with deployments that haven't added a frontend yet.
    let frontend_config = {
        let app_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        // Collect SSR routes (mode == "ssr") into a shared list so the
        // frontend dispatcher can match them on every GET. Wire fn_ops
        // through too — the SSR branch calls render_route on it.
        // Empty list + None fn_ops are both no-ops in try_handle, so
        // projects without SSR routes pay nothing.
        let ssr_routes: Vec<pylon_kernel::ManifestRoute> = shared_manifest
            .routes
            .iter()
            .filter(|r| r.mode == "ssr")
            .cloned()
            .collect();
        let fn_ops_arc: Option<Arc<dyn pylon_router::FnOps>> =
            fn_ops_override.clone().or_else(|| {
                fn_ops_maybe
                    .as_ref()
                    .map(|f| Arc::clone(f) as Arc<dyn pylon_router::FnOps>)
            });
        Arc::new(
            crate::frontend::FrontendConfig::from_env(&app_dir)
                .with_ssr(Arc::new(ssr_routes), fn_ops_arc)
                .with_session(Arc::clone(&session_store), Arc::clone(&cookie_config))
                .with_orgs(Arc::clone(&orgs))
                .with_runtime(Arc::clone(&runtime)),
        )
    };
    if frontend_config.is_active() {
        if let Some(dir) = frontend_config.dir.as_deref() {
            tracing::info!("  Frontend: serving from {}", dir.display());
        }
        if let Some(proxy) = frontend_config.dev_proxy.as_deref() {
            tracing::info!("  Frontend: proxying to {proxy}");
        }
    }

    // Warm the SSR client (hydration) bundle off the request path. It is
    // otherwise built lazily on the first request that needs it — a cold
    // `Bun.build` that, on a fresh artifact deploy, runs against an empty
    // `.pylon/client-build/`. That cold build racing the request timeout was a
    // contributing factor in a production first-paint outage. Building it now,
    // during boot, means the first real user request finds a ready manifest.
    // Fire-and-forget on a dedicated thread so boot isn't blocked; a failure is
    // logged and harmless because the lazy first-request path is still the
    // fallback. Only when the project actually has SSR routes + a functions
    // backend wired — API-only and legacy `web/dist` apps skip it.
    if !frontend_config.ssr_routes.is_empty() {
        // #277 Stage 2: drop on-disk ISR entries from previous deploys so a new
        // build never serves a prior build's HTML (cache is build-id-namespaced;
        // this reclaims the stale namespaces' disk). Cheap, synchronous, safe.
        crate::ssr_cache::wipe_stale_namespaces();
        if let Some(fn_ops_warm) = frontend_config.fn_ops.clone() {
            // The route dir the manifest was built with (`app`, or a
            // subdir like `web/app` for a namespaced full-stack app). The
            // client bundler must walk the SAME dir or it finds no routes
            // and ships no hydration bundle.
            let warm_app_dir = crate::frontend::derive_app_dir(&frontend_config.ssr_routes);
            let _ = std::thread::Builder::new()
                .name("ssr-bundle-warm".into())
                .spawn(move || {
                    let started = std::time::Instant::now();
                    // Populates the asset route's outdir cache + writes the
                    // manifest, so neither the first asset request nor the
                    // first render re-triggers the build (see warm_client_bundle).
                    match crate::frontend::warm_client_bundle(&fn_ops_warm, &warm_app_dir) {
                        Ok(()) => tracing::info!(
                            "  SSR client bundle warmed in {:?}",
                            started.elapsed()
                        ),
                        Err(e) => tracing::warn!(
                            "SSR client bundle warm failed (will build lazily on first request): {e}"
                        ),
                    }
                });
        }
    }

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
                // recv() errors on either an intentional `unblock()` (shutdown)
                // or tiny_http giving up its accept loop and dropping the
                // listener after ~64 consecutive accept errors (an EINVAL /
                // connection-reset storm on the `[::]` dual-stack socket). The
                // former: exit. The latter: rebuild the listener so the dev
                // server keeps serving instead of silently going dark.
                if SHUTDOWN.load(Ordering::Relaxed) {
                    break;
                }
                match rebuild_with_retry(port) {
                    Some(new_server) => {
                        set_server_handle(&new_server);
                        server = new_server;
                        continue;
                    }
                    None => break, // shutdown requested mid-rebuild
                }
            }
        };

        if SHUTDOWN.load(Ordering::Relaxed) {
            break;
        }

        // Acquire a dispatch slot BEFORE spawning the worker. If the global
        // cap or per-IP cap is hit, respond 503 with Retry-After: 1
        // immediately on the dispatch thread — spawning would have
        // produced the same response after thread setup overhead.
        // Pre-acquisition: read client IP once so it can flow into BOTH the
        // dispatch-limiter (here) and the stream-limiter (later, inside the
        // closure). Bound name `dispatch_peer_ip` avoids conflict with the
        // body's per-request `peer_ip` String AND the access-log
        // `request_peer_ip` String — both pre-existing — we just need the
        // parsed IpAddr for the limiters.
        let dispatch_peer_ip_str = resolve_client_ip(&request, trust_proxy_hops);
        let dispatch_peer_ip: std::net::IpAddr = dispatch_peer_ip_str
            .parse()
            .unwrap_or_else(|_| std::net::IpAddr::V4(std::net::Ipv4Addr::new(0, 0, 0, 0)));
        let limiter_guards = match dispatch_limiter.acquire(dispatch_peer_ip) {
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
        let stream_limiter = Arc::clone(&stream_limiter);
        let frontend_config = Arc::clone(&frontend_config);

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
                // Re-bind `request` as mut so the SPA hand-back path
                // (`request = returned;`) compiles — `move` capture
                // doesn't carry the outer `let mut`.
                let mut request = request;

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
        // Dev mode keeps /admin/* + /metrics open for local convenience — but
        // ONLY when no operator token is configured (see
        // `dev_admin_endpoints_open`).
        let dev_metrics_token = std::env::var("PYLON_METRICS_TOKEN").ok();
        let dev_admin_open =
            dev_admin_endpoints_open(is_dev, admin_token.as_deref(), dev_metrics_token.as_deref());

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

        // --- Canonical apex↔www redirect (opt-in: PYLON_CANONICAL_HOST) ---
        // When a project picks a canonical custom domain (Vercel-style), the
        // OTHER sibling (apex↔www) 308-redirects to it. Scoped to EXACTLY that
        // sibling — `<app>.fly.dev`, `<slug>.pyln.dev`, the Fly health-check
        // host, localhost, and unrelated domains are never touched. WS upgrades
        // and the /health + /metrics probes are skipped so infra paths keep
        // answering on any host. 308 preserves method + body + path + query.
        if let Some(canonical) = canonical_host() {
            let is_probe = url == "/health" || url == "/health/deep" || url == "/metrics";
            let is_ws_upgrade = request.headers().iter().any(|h| {
                h.field.equiv("Upgrade")
                    && h.value.as_str().eq_ignore_ascii_case("websocket")
            });
            if !is_probe && !is_ws_upgrade {
                // Public host: prefer X-Forwarded-Host (Fly's edge sets it).
                let mut fwd_host: Option<String> = None;
                let mut raw_host: Option<String> = None;
                for h in request.headers().iter() {
                    if h.field.equiv("X-Forwarded-Host") {
                        fwd_host = Some(h.value.as_str().to_string());
                    } else if h.field.equiv("Host") {
                        raw_host = Some(h.value.as_str().to_string());
                    }
                }
                let host = fwd_host
                    .or(raw_host)
                    .map(|h| h.split(':').next().unwrap_or("").to_ascii_lowercase())
                    .unwrap_or_default();
                if let Some(target) = canonical_redirect_target(canonical, &host, &url) {
                    let response = with_security_headers(
                        Response::from_string("")
                            .with_status_code(308u16)
                            .with_header(
                                Header::from_bytes("Location", target.as_bytes().to_vec())
                                    .unwrap(),
                            ),
                    );
                    let _ = request.respond(response);
                    mt.record_request(method.as_str(), 308);
                    return;
                }
            }
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
                // CSWSH defense: an upgrade authenticated by the AMBIENT
                // session cookie must originate from a trusted Origin.
                // Browsers auto-attach the victim's cookie to a cross-origin
                // WS handshake, so without this gate an attacker page could
                // open an authenticated socket as the victim (the HTTP API is
                // already CORS/allowlist-gated; the WS upgrade was not).
                // Explicit bearer/subprotocol auth is non-ambient and exempt.
                if upgrade_req.cookie_auth
                    && !ws_cookie_origin_trusted(req_origin_header.as_deref(), &cors_allowlist)
                {
                    tracing::warn!(
                        "[ws] rejected cookie-authed upgrade from untrusted origin {:?} — \
                         add it to manifest.auth.trustedOrigins or PYLON_CORS_ORIGIN",
                        req_origin_header
                    );
                    let response = with_security_headers(
                        Response::from_string(json_error(
                            "WS_ORIGIN_FORBIDDEN",
                            "WebSocket upgrade with cookie auth requires a trusted Origin",
                        ))
                        .with_status_code(403u16)
                        .with_header(
                            Header::from_bytes("Content-Type", "application/json").unwrap(),
                        ),
                    );
                    let _ = request.respond(response);
                    mt.record_request("GET", 403);
                    return;
                }
                let hub = Arc::clone(&ws_hub);
                let auth = Arc::clone(&ws_auth);
                let fetcher = snapshot_fetcher.clone();
                let reactive = Arc::clone(&reactive_registry);
                let rooms_bridge: Arc<dyn crate::ws::RoomBridge> = Arc::clone(&rm) as _;
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
                            Some(rooms_bridge),
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
            if !dev_admin_open
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
            if !dev_admin_open
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
            if !dev_admin_open
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
                if !dev_admin_open
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
            if !dev_admin_open
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
            if !dev_admin_open
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
            if !dev_admin_open
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
                // Browser navigation (Accept: text/html) → friendly HTML page,
                // not the raw API JSON envelope. API/fetch callers still get
                // JSON. The HTML is the framework default; an app can ship
                // app/rate-limit.tsx to override the styling.
                if crate::frontend::request_prefers_html(&request) {
                    let response = with_security_headers(
                        crate::frontend::rate_limited_html_response(retry_after, &cors_origin),
                    );
                    let _ = request.respond(response);
                    mt.record_request(method.as_str(), 429);
                    return;
                }
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
        // Runs after rate-limiting and ABOVE `try_handle`: `route.ts`
        // form/method handlers (#276) are dispatched inside `try_handle` for
        // non-GET methods and WRITE to the DB, so the Origin/Referer gate MUST
        // clear before they run — otherwise a cross-site POST to a form route
        // bypasses CSRF entirely. GET/HEAD SPA assets are unaffected (safe
        // methods skip the check below). Auth is still resolved further down.
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

        // --- SPA / form-route serving (unified full-stack) ---
        //
        // The handler is self-contained — it consumes `request` on success or
        // hands it back (`Err(request)`) when the URL is API-bound so existing
        // routing continues unchanged. SPA assets are public-by-design (anyone
        // hitting the URL can fetch them — the SPA's own code proves identity
        // once the bundle runs); state-changing form routes already cleared the
        // CSRF gate above.
        if frontend_config.is_active() {
            let method_str_for_metric = method.as_str().to_string();
            match crate::frontend::try_handle(&frontend_config, request, &cors_origin) {
                Ok(()) => {
                    // Status is logged inside the module; we record a
                    // 200 here as a conservative aggregate (precise per-
                    // asset status codes flow through `tracing::info!`
                    // in dev). The frontend module never panics — a
                    // 502 from the proxy still flows through Ok(()).
                    metrics.record_request(&method_str_for_metric, 200);
                    return;
                }
                Err(returned) => {
                    request = returned;
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
        // Per-user admin resolution. Shared with the SSR/frontend auth resolver
        // (`crate::frontend::resolve_request_auth`) via `lift_admin` so both
        // paths resolve `is_admin` identically — see the helper's doc for the
        // two designation paths + the API-key exclusion.
        lift_admin(&runtime, &mut auth_ctx);

        // Multi-tenant role resolution. Surface the caller's role in their
        // active org as `auth_ctx.roles` (see the helper's doc comment for why
        // the session store can't do this itself). Without it, `auth.roles`
        // stays empty for org members and every role gate is dead — including
        // hiding the invite UI from the org's own owner.
        pylon_auth::org::enrich_active_org_role(&og, &mut auth_ctx);
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
        // A direct-to-S3 flow so large upload bytes never buffer in pylon's
        // process (the source of OOMs):
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

            // Enforce upload-max BEFORE handing out the URL. S3 binds this
            // declared size into the presigned PUT's Content-Length, while
            // Stack0 enforces its own limits server-side. Confirmation still
            // checks stored metadata as defense-in-depth across backends.
            let upload_max = upload_max_bytes();
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
                Ok(init) => {
                    // Bind the freshly-minted asset to its initiator NOW so the
                    // `local-put` byte-receiver can reject a write from any
                    // OTHER user, and so a confirmed file can never later be
                    // overwritten by a different caller. Without this the
                    // owner was only recorded at confirm-time (after the PUT),
                    // leaving a window where any authed user who knew/guessed
                    // the id could overwrite another user's bytes. Ownership-
                    // less backends (Stack0/public S3 — gated by the opaque URL
                    // or intentionally public CDN) skip via
                    // `requires_owner_check()`.
                    let owner_result = if storage.requires_owner_check() {
                        match auth_ctx.user_id.as_ref() {
                            Some(uid) => storage.record_owner(
                                &init.asset_id,
                                &pylon_storage::files::FileOwner {
                                    user_id: uid.clone(),
                                    tenant_id: auth_ctx.tenant_id.clone(),
                                },
                            ),
                            None => Err(pylon_storage::files::FileStorageError {
                                code: "INTERNAL".into(),
                                message: "auth lost between checks".into(),
                            }),
                        }
                    } else {
                        Ok(())
                    };

                    match owner_result {
                        Ok(()) => (
                            200u16,
                            serde_json::to_string(&init).unwrap_or_else(|_| "{}".into()),
                        ),
                        Err(e) => {
                            tracing::error!(
                                file_id = %init.asset_id,
                                error = %e.message,
                                "Failed to bind file owner at init"
                            );
                            let _ = storage.delete(&init.asset_id);
                            (
                                500u16,
                                json_error("OWNERSHIP_RECORD_FAILED", &e.message),
                            )
                        }
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

        // POST /api/files/confirm — step 3. Ownership-tracking backends only
        // allow the exact init-time user + tenant to complete the upload.
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
            let requires_owner_check = storage.requires_owner_check();
            if requires_owner_check
                && !file_confirm_authorized(
                    &storage.owner_of(&asset_id),
                    auth_ctx.user_id.as_deref(),
                    auth_ctx.tenant_id.as_deref(),
                )
            {
                let err = json_error("NOT_FOUND", "File not found");
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

            let (status, body) = match storage.confirm_upload(&asset_id) {
                Ok(stored) => {
                    let upload_max = upload_max_bytes();
                    if !upload_size_allowed(stored.size, upload_max) {
                        // S3 binds the admitted size into the upload signature;
                        // this metadata check is defense-in-depth. Destructive
                        // cleanup is safe only after the owner check above has
                        // proven this caller's init-time claim. Public S3 and
                        // Stack0 have no such claim, so a caller-supplied ID
                        // must never be allowed to delete an existing asset.
                        if requires_owner_check {
                            if let Err(e) = storage.delete(&stored.id) {
                                tracing::error!(
                                    file_id = %stored.id,
                                    error = %e.message,
                                    "Failed to delete oversized upload"
                                );
                            }
                        }
                        (
                            413u16,
                            json_error(
                                "PAYLOAD_TOO_LARGE",
                                &format!(
                                    "actual size {} exceeds upload max of {upload_max}",
                                    stored.size
                                ),
                            ),
                        )
                    } else if requires_owner_check {
                        // The init-time owner is authoritative. Rewriting it
                        // here would turn confirmation into an ownership claim.
                        (
                            200u16,
                            serde_json::to_string(&stored).unwrap_or_else(|_| "{}".into()),
                        )
                    } else if let Some(uid) = auth_ctx.user_id.as_ref() {
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
                let upload_max = upload_max_bytes();
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
                // Owner gate: the byte-receiver must refuse a write to an asset
                // owned by a DIFFERENT user — otherwise any authenticated user
                // could overwrite another user's pending upload OR a confirmed
                // file by knowing/guessing its id. Fresh assets are bound to
                // their initiator at `/api/files/init`; unowned assets (legacy,
                // or a direct `store()`) are CLAIMED on first write below. Fail
                // CLOSED on a lookup error.
                use pylon_storage::files::FileStorage as _;
                let caller = auth_ctx.user_id.clone().unwrap_or_default();
                let prior_owner = local.owner_of(asset_id);
                // `is_unscoped_admin()`, not bare `is_admin`: an admin acting
                // WITHIN a tenant context must stay scoped to the owner check
                // (the #354/#355 access-control rule). Only an admin with NO
                // active tenant may overwrite another user's asset.
                if local_put_owned_by_other(
                    &prior_owner,
                    &caller,
                    auth_ctx.tenant_id.as_deref(),
                    auth_ctx.is_unscoped_admin(),
                ) {
                    // 404 (not 403) to avoid confirming the asset exists —
                    // matches the GET/DELETE owner-mismatch behaviour.
                    let err = json_error("NOT_FOUND", "File not found");
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
                    mt.record_request("PUT", 404);
                    return;
                }
                let (status, body) = match local.write_bytes(asset_id, &bytes) {
                    Ok(()) => {
                        // Claim an unowned asset for the caller so a later,
                        // different user can't overwrite it (the asset had no
                        // init-time binding — legacy or direct write).
                        if matches!(prior_owner, Ok(None)) && !caller.is_empty() {
                            let owner = pylon_storage::files::FileOwner {
                                user_id: caller.clone(),
                                tenant_id: auth_ctx.tenant_id.clone(),
                            };
                            let _ = local.record_owner(asset_id, &owner);
                        }
                        (204u16, String::new())
                    }
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
                            // `is_unscoped_admin()`, not bare `is_admin`: an
                            // admin acting within a tenant context stays scoped
                            // to the owner check (#354/#355). Only an admin with
                            // no active tenant may delete another user's asset.
                            if !auth_ctx.is_unscoped_admin()
                                && !file_owner_matches(
                                    &owner,
                                    auth_ctx.user_id.as_deref(),
                                    auth_ctx.tenant_id.as_deref(),
                                )
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
                        Err(e) => {
                            tracing::error!(
                                file_id = %asset_id,
                                error = %e.message,
                                "Failed to authorize file deletion"
                            );
                            let err = json_error(
                                "FILE_AUTH_FAILED",
                                "Unable to authorize file deletion",
                            );
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
                            mt.record_request("DELETE", 500);
                            return;
                        }
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
                    // Owner check for backends that track ownership. FAIL
                    // CLOSED: serve only when the asset has a recorded owner
                    // that matches the caller. Previously this denied only on
                    // a present-but-mismatched owner and fell through (served
                    // the bytes) when owner_of returned Ok(None) — no owner
                    // recorded, e.g. a file written via local-put before
                    // /confirm, or any store()'d file — or Err (unreadable
                    // sidecar). That let any authenticated user read those
                    // assets (IDOR). The sibling DELETE handler already fails
                    // closed on Ok(None); GET now matches it.
                    // `is_unscoped_admin()`, not bare `is_admin`: an admin
                    // acting within a tenant context stays scoped to the owner
                    // check (#354/#355). Only an admin with no active tenant
                    // gets the cross-owner read bypass.
                    if storage.requires_owner_check() && !auth_ctx.is_unscoped_admin() {
                        let owned_by_caller = file_read_authorized(
                            &storage.owner_of(asset_id),
                            auth_ctx.user_id.as_deref(),
                            auth_ctx.tenant_id.as_deref(),
                        );
                        if !owned_by_caller {
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

        // Deprecated multipart upload — the 3-endpoint flow above replaces
        // it. Returns a clear migration hint so any old client sees a
        // useful error instead of a 404.
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
        //
        // Default 10 MB; PYLON_HTTP_BODY_MAX_BYTES raises it for
        // deployments that legitimately take large request bodies (the
        // canonical case: a control plane accepting CLI source uploads,
        // where the tarball rides base64-inside-JSON at ~1.37x its size).
        // It stays a hard bound either way — just a configurable one.
        static BODY_MAX: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
        let max_body_size: usize = *BODY_MAX.get_or_init(|| {
            std::env::var("PYLON_HTTP_BODY_MAX_BYTES")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .filter(|v| *v >= 1024)
                .unwrap_or(10 * 1024 * 1024)
        });

        if let Some(declared) = request.body_length() {
            if declared > max_body_size {
                let err_body = json_error(
                    "PAYLOAD_TOO_LARGE",
                    &format!("Content-Length {declared} exceeds max of {max_body_size}"),
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
            let mut limited = request.as_reader().take((max_body_size as u64) + 1);
            let _ = limited.read_to_string(&mut body);
        }
        // Stamp bytes_in for the shipper's per-request rollup. The
        // central dispatch loop is the only path that reads the body,
        // so this is the single right place to capture its size.
        crate::metrics::set_current_request_bytes(body.len());

        if body.len() > max_body_size {
            let err_body = json_error(
                "PAYLOAD_TOO_LARGE",
                &format!(
                    "Request body exceeds maximum size of {} bytes",
                    max_body_size,
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

                    let (tx, streaming_body) =
                        bounded_stream(SHARD_STREAM_BUFFER_CAPACITY);
                    let (disconnect_tx, disconnect_rx) =
                        std::sync::mpsc::sync_channel::<()>(1);

                    let tx_clone = tx.clone();
                    let sink: pylon_realtime::SnapshotSink =
                        Box::new(move |tick: u64, bytes: &[u8]| {
                            // Format as SSE with an id: line carrying the tick
                            // number so clients can resume with Last-Event-ID.
                            let mut frame = format!("id: {tick}\ndata: ").into_bytes();
                            frame.extend_from_slice(bytes);
                            frame.extend_from_slice(b"\n\n");
                            match tx_clone.try_send(frame) {
                                Ok(()) => {}
                                Err(std::sync::mpsc::TrySendError::Full(_)) => {
                                    // Never block a shard tick on a slow HTTP
                                    // client. The cleanup thread removes this
                                    // subscriber and closes the stream.
                                    let _ = disconnect_tx.try_send(());
                                }
                                Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                                    let _ = disconnect_tx.try_send(());
                                }
                            }
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
                            // A full snapshot queue means the client is too
                            // slow to receive current state. Disconnect it so
                            // memory stays bounded and the client can resume.
                            loop {
                                match disconnect_rx
                                    .recv_timeout(std::time::Duration::from_secs(30))
                                {
                                    Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                                        shard_cleanup.remove_subscriber(&sub_id_cleanup);
                                        return;
                                    }
                                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                                        match tx_liveness.try_send(b": heartbeat\n\n".to_vec()) {
                                            Ok(()) => {}
                                            Err(std::sync::mpsc::TrySendError::Full(_))
                                            | Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                                                shard_cleanup.remove_subscriber(&sub_id_cleanup);
                                                return;
                                            }
                                        }
                                        if !shard_cleanup.is_running() {
                                            return;
                                        }
                                    }
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
                    // Hand the request + stream over to a dedicated stream
                    // thread holding a StreamLimiter slot — the worker
                    // returns immediately so its dispatch slot frees for
                    // the next short request. Streams have their own cap
                    // (PYLON_STREAM_INFLIGHT_MAX + PYLON_STREAM_PER_IP_MAX)
                    // so a flood of streams can't starve normal traffic.
                    if let Err(request) = spawn_streaming_response(
                        request,
                        response,
                        &stream_limiter,
                        dispatch_peer_ip,
                        Arc::clone(&mt),
                        "GET".to_string(),
                        200,
                    ) {
                        let body = json_error(
                            "STREAM_OVERLOADED",
                            "Server is at its streaming concurrency cap. Retry shortly.",
                        );
                        let resp = with_security_headers(
                            Response::from_string(&body)
                                .with_status_code(503u16)
                                .with_header(
                                    Header::from_bytes("Content-Type", "application/json").unwrap(),
                                )
                                .with_header(Header::from_bytes("Retry-After", "1").unwrap()),
                        );
                        let _ = request.respond(resp);
                        mt.record_request("GET", 503);
                    }
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

                let (tx, streaming_body) =
                    bounded_stream(FINITE_STREAM_BUFFER_CAPACITY);

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
                if let Err(request) = spawn_streaming_response(
                    request,
                    response,
                    &stream_limiter,
                    dispatch_peer_ip,
                    Arc::clone(&mt),
                    "POST".to_string(),
                    200,
                ) {
                    let body = json_error(
                        "STREAM_OVERLOADED",
                        "Server is at its streaming concurrency cap. Retry shortly.",
                    );
                    let resp = with_security_headers(
                        Response::from_string(&body)
                            .with_status_code(503u16)
                            .with_header(
                                Header::from_bytes("Content-Type", "application/json").unwrap(),
                            )
                            .with_header(Header::from_bytes("Retry-After", "1").unwrap()),
                    );
                    let _ = request.respond(resp);
                    mt.record_request("POST", 503);
                }
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
            let (tx, streaming_body) = bounded_stream(FINITE_STREAM_BUFFER_CAPACITY);

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
            if let Err(request) = spawn_streaming_response(
                request,
                response,
                &stream_limiter,
                dispatch_peer_ip,
                Arc::clone(&mt),
                "POST".to_string(),
                200,
            ) {
                let body = json_error(
                    "STREAM_OVERLOADED",
                    "Server is at its streaming concurrency cap. Retry shortly.",
                );
                let resp = with_security_headers(
                    Response::from_string(&body)
                        .with_status_code(503u16)
                        .with_header(
                            Header::from_bytes("Content-Type", "application/json").unwrap(),
                        )
                        .with_header(Header::from_bytes("Retry-After", "1").unwrap()),
                );
                let _ = request.respond(resp);
                mt.record_request("POST", 503);
            }
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
                // Auth routes (magic codes / reset / invites) use the auth
                // email channel: PYLON_AUTH_EMAIL_* if set, else PYLON_EMAIL_*.
                // Kept distinct from the app's ctx.email (fn_email_adapter,
                // PYLON_EMAIL_* only) so a shared platform auth key can't be
                // reached by app code to send arbitrary mail.
                let email_adapter = EmailAdapter::for_auth();
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
        //
        // CDN safety: a shared cache (e.g. Cloudflare in front) must NEVER
        // store a personalized or dynamic response — caching one user's
        // session/personalized body and replaying it to another is a
        // cross-user leak. A `Set-Cookie` response is inherently per-user, and
        // every `/api/*` response is dynamic, so default those to `no-store`
        // UNLESS the handler set its own Cache-Control. (Frontend static assets
        // + SSR HTML go through `try_handle`, which sets its own caching.)
        let sets_cookie = extra_headers
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case("set-cookie"));
        let handler_set_cache = extra_headers
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case("cache-control"));
        for (name, value) in extra_headers {
            if let Ok(h) = Header::from_bytes(name.as_bytes(), value.as_bytes().to_vec()) {
                response = response.with_header(h);
            }
        }
        if !handler_set_cache && (sets_cookie || url.starts_with("/api/")) {
            response = response
                .with_header(Header::from_bytes("Cache-Control", b"no-store".to_vec()).unwrap());
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

    // Wait for in-flight HTTP workers AND background jobs to drain. The
    // HTTP workers are the new piece: pre-refactor the dispatch loop
    // was single-threaded so shutdown blocked recv() implicitly; now
    // each request runs on its own worker thread and we need to wait
    // for the active set to clear before we close the runtime. Stream
    // workers are intentionally not waited on — they're long-lived by
    // design and would always hit the drain_timeout. If they outlive
    // the cap, they get killed alongside the process; that's
    // acceptable for a graceful-shutdown signal vs. SIGKILL.
    while start.elapsed() < drain_timeout {
        let pending_jobs = job_queue.stats().pending;
        let in_flight_http = dispatch_limiter.in_flight();
        if pending_jobs == 0 && in_flight_http == 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    let elapsed = start.elapsed();
    let final_in_flight = dispatch_limiter.in_flight();
    let final_pending = job_queue.stats().pending;
    let final_streams = stream_limiter
        .in_flight
        .load(std::sync::atomic::Ordering::Acquire);
    tracing::warn!(
        "Drain complete in {:.1}s (timeout {}s, in-flight http={}, pending jobs={}, streams={})",
        elapsed.as_secs_f32(),
        drain_timeout.as_secs(),
        final_in_flight,
        final_pending,
        final_streams,
    );

    // Flush the change-log persister BEFORE the WAL checkpoint. The
    // persister batches appends on a background thread (off the
    // mutation hot path), so at shutdown the last few events may still
    // be sitting in its channel. Draining them here is what makes a
    // clean redeploy durable — without it, the final messages before a
    // restart die in the channel and the next boot's hydration misses
    // them ("send a message, restart, refresh, it's gone", narrowed to
    // the shutdown window). Bounded by the same drain timeout; a
    // timeout just means we exit with a few events un-persisted, same
    // as a hard kill.
    if change_log.flush(drain_timeout) {
        tracing::info!("Change-log persister flushed");
    } else {
        tracing::warn!(
            "Change-log persister flush timed out after {}s; some events may not have persisted",
            drain_timeout.as_secs()
        );
    }

    // Force-checkpoint the WAL so the next boot doesn't have to recover
    // any uncommitted journal pages. The wedge we hit on dev.db.jobs.db
    // came from skipping this step on abrupt termination — for graceful
    // shutdown we leave the DB in a clean state. Logged-not-fatal: if
    // checkpoint fails (e.g., another conn still holds a lock) we want
    // to exit anyway. busy_timeout=5000 means the operator-visible cap
    // is bounded.
    match runtime.checkpoint_wal() {
        Ok(()) => tracing::info!("WAL checkpoint (TRUNCATE) complete"),
        Err(e) => tracing::warn!(
            "WAL checkpoint failed (next boot will recover): {} {}",
            e.code,
            e.message
        ),
    }
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
/// Whether dev mode should leave the `/admin/*` + `/metrics` endpoints open
/// (no auth). True ONLY in dev AND when no operator token is configured —
/// an empty-string token counts as unset. In production a token is always
/// set, so an accidental `PYLON_DEV_MODE=true` there still enforces auth
/// instead of silently exposing admin/metrics data.
fn dev_admin_endpoints_open(
    is_dev: bool,
    admin_token: Option<&str>,
    metrics_token: Option<&str>,
) -> bool {
    let unset = |t: Option<&str>| t.map(|s| s.trim().is_empty()).unwrap_or(true);
    is_dev && unset(admin_token) && unset(metrics_token)
}

#[cfg(test)]
mod dev_admin_gate_tests {
    use super::dev_admin_endpoints_open;

    #[test]
    fn dev_admin_open_only_when_dev_and_no_token() {
        // Prod: always enforce (never open), token or not.
        assert!(!dev_admin_endpoints_open(false, None, None));
        assert!(!dev_admin_endpoints_open(false, Some("tok"), None));
        // Dev, no token configured → open (local convenience).
        assert!(dev_admin_endpoints_open(true, None, None));
        assert!(dev_admin_endpoints_open(true, Some("  "), Some("")));
        // Dev BUT a token is configured (the accidental-prod case) → enforce.
        assert!(!dev_admin_endpoints_open(true, Some("admintok"), None));
        assert!(!dev_admin_endpoints_open(true, None, Some("metricstok")));
        assert!(!dev_admin_endpoints_open(true, Some("a"), Some("m")));
    }
}

/// Whether to warn at boot that the dedicated SSE port serves UNAUTHENTICATED
/// clients. The port authenticates every connection by default (401 for anon
/// in prod) and per-client policy-filters every event — see
/// `sse::handle_sse_connection`. The ONLY way it serves anonymous clients is
/// when the operator sets `PYLON_SSE_PORT_ACKNOWLEDGE_UNAUTH=1`, which DISABLES
/// that auth gate. So the warning must fire on that opt-in, NOT on the safe
/// default — warning on the safe default would push operators to "silence" it
/// by setting the very flag that opens the leak.
fn sse_unauth_warning_warranted(in_prod: bool, acknowledged_unauth: bool) -> bool {
    in_prod && acknowledged_unauth
}

#[cfg(test)]
mod sse_unauth_warning_tests {
    use super::sse_unauth_warning_warranted;

    #[test]
    fn warns_only_when_the_unauth_flag_is_actually_set() {
        // Safe default (prod, flag unset): the port requires auth → NO warning.
        // Pre-fix this case warned falsely AND advised the leak-opening flag.
        assert!(!sse_unauth_warning_warranted(true, false));
        // Operator opted into anonymous SSE (flag set) in prod → WARN loudly.
        assert!(sse_unauth_warning_warranted(true, true));
        // Dev: anonymous SSE is expected locally → no prod-leak warning either way.
        assert!(!sse_unauth_warning_warranted(false, false));
        assert!(!sse_unauth_warning_warranted(false, true));
    }
}

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
        (true, _) => Ok(in_memory_auth_stores(session_lifetime)),
        (false, Some(path)) => build_sqlite_auth_stores(&path, session_lifetime),
        // No persistent backend configured. In dev this is fine — ephemeral
        // sessions match the ephemeral in-memory datastore. In a production
        // boot it's a silent footgun: sessions evaporate on every restart and
        // are NOT shared across replicas, so users get logged out at random
        // and multi-machine auth breaks — but it looks fine in a single-box
        // demo. Refuse to boot rather than degrade silently (mirrors the
        // SQLite/Postgres fail-fast). Operators who genuinely want ephemeral
        // sessions in prod opt in explicitly with PYLON_SESSION_IN_MEMORY=1.
        (false, None) => {
            ephemeral_sessions_boot_check(force_in_memory, crate::frontend::is_dev_mode())?;
            Ok(in_memory_auth_stores(session_lifetime))
        }
    }
}

/// Boot policy for when NO persistent session backend is configured (no
/// Postgres `DATABASE_URL`, no SQLite path). In-memory sessions are only
/// acceptable in dev, or with an explicit opt-in: a production boot must fail
/// fast rather than silently losing sessions on every restart and failing to
/// share them across replicas (users randomly logged out; multi-machine auth
/// broken — but it looks fine in a single-box demo). Returns `Ok(())` to allow
/// the ephemeral fallback or `Err(msg)` to refuse the boot.
fn ephemeral_sessions_boot_check(force_in_memory: bool, is_dev: bool) -> Result<(), String> {
    if force_in_memory || is_dev {
        Ok(())
    } else {
        Err(
            "[pylon] Refusing to boot with in-memory sessions outside dev: \
             sessions would be lost on every restart and not shared across replicas \
             (users randomly logged out; multi-machine auth broken). Configure a \
             database — DATABASE_URL=postgres://… or PYLON_SESSION_DB=/path/to/sessions.db \
             — or set PYLON_SESSION_IN_MEMORY=1 to explicitly accept ephemeral sessions."
                .to_string(),
        )
    }
}

/// Decide whether a `local-put` byte write must be REFUSED because the asset is
/// already owned by another user or tenant. Fails CLOSED: an ownership-
/// lookup error refuses the write. An unowned asset (`Ok(None)`) is allowed —
/// the caller claims it on write. Admins bypass the owner match.
fn local_put_owned_by_other(
    prior_owner: &Result<
        Option<pylon_storage::files::FileOwner>,
        pylon_storage::files::FileStorageError,
    >,
    caller: &str,
    caller_tenant_id: Option<&str>,
    is_admin: bool,
) -> bool {
    match prior_owner {
        Ok(Some(owner)) => !is_admin && !file_owner_matches(owner, Some(caller), caller_tenant_id),
        Ok(None) => false,
        Err(_) => true,
    }
}

fn in_memory_auth_stores(session_lifetime: u64) -> AuthStores {
    AuthStores {
        session_store: Arc::new(SessionStore::new().with_lifetime(session_lifetime)),
        magic_codes: Arc::new(pylon_auth::MagicCodeStore::new()),
        oauth_state: Arc::new(pylon_auth::OAuthStateStore::new()),
        account_store: Arc::new(pylon_auth::AccountStore::new()),
        api_keys: Arc::new(pylon_auth::api_key::ApiKeyStore::new()),
        siwe: pylon_auth::siwe::NonceStore::new(),
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
        siwe: pylon_auth::siwe::NonceStore::new(),
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
    // Never log/return the raw DSN — it carries the DB password. Redact
    // once and use the masked form in both the error string and the
    // boot log below.
    let url_safe = pylon_kernel::util::redact_dsn(url);
    let map_err = |what: &str, e: String| {
        format!(
            "[pylon] {what} Postgres backend at {url_safe}: {e}. \
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
    tracing::info!("[pylon] Auth state (Postgres): {url_safe}");
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
        siwe: pylon_auth::siwe::NonceStore::new(),
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
/// Persistent by default: without it, tokens in browser localStorage resolve
/// to anonymous after a restart — pulls come back empty under policy and
/// mutations 400 with UNAUTHENTICATED.
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

#[cfg(test)]
mod http_listener_rebuild_tests {
    use super::build_http_server;
    use std::io::{Read, Write};
    use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
    use std::time::Duration;
    use tiny_http::Response;

    /// #346: when tiny_http gives up its accept loop under a connection-reset
    /// storm and drops its listener, the recv loop rebuilds a fresh `Server` on
    /// the same port and keeps serving instead of silently going dark. The
    /// recv-loop rebuild rests on one property: that `build_http_server` can
    /// re-bind the same port after the old listener is gone and serve a real
    /// request over it. This proves exactly that. (tiny_http's internal
    /// 64-EINVAL give-up can't be forced deterministically, so we stand in for
    /// it by dropping the first listener explicitly.)
    #[test]
    fn rebuilt_listener_binds_same_port_and_serves() {
        // Bind an ephemeral port, learn it, then drop the server to free the
        // port — standing in for tiny_http dropping its listener on give-up.
        let first = build_http_server(0).expect("initial bind");
        let port = first.server_addr().to_ip().expect("ip addr").port();
        drop(first);

        // Rebuild on the SAME port. A freshly-vacated port can briefly linger
        // in TIME_WAIT, so retry with a short backoff — the same shape as
        // `rebuild_with_retry`, but bounded so the test can never hang.
        let mut rebuilt = None;
        for _ in 0..100 {
            match build_http_server(port) {
                Ok(s) => {
                    rebuilt = Some(s);
                    break;
                }
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        let server = rebuilt.expect("rebuild on same port within budget");
        let bound = server.server_addr().to_ip().expect("rebuilt addr");

        // Serve exactly one request on a worker thread, then exit.
        let worker = std::thread::spawn(move || {
            if let Ok(req) = server.recv() {
                let _ = req.respond(Response::from_string("pong"));
            }
        });

        // Connect to the rebuilt listener over the loopback of the bound
        // family (`[::1]` for a dual-stack `[::]` bind, `127.0.0.1` for the
        // v4-only fallback) so the test holds on both macOS and Linux.
        let connect_addr: SocketAddr = if bound.is_ipv6() {
            (Ipv6Addr::LOCALHOST, port).into()
        } else {
            (Ipv4Addr::LOCALHOST, port).into()
        };
        let mut stream = None;
        for _ in 0..100 {
            match TcpStream::connect(connect_addr) {
                Ok(s) => {
                    stream = Some(s);
                    break;
                }
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        let mut stream = stream.expect("connect to rebuilt listener");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        // HTTP/1.0 (no keep-alive) so the server closes after one response and
        // `read_to_string` returns rather than blocking on a kept-alive socket.
        stream
            .write_all(b"GET / HTTP/1.0\r\nHost: localhost\r\n\r\n")
            .expect("write request");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read response");

        assert!(
            response.contains("200 OK"),
            "rebuilt listener should serve 200; got: {response}"
        );
        assert!(
            response.contains("pong"),
            "rebuilt listener should serve the body; got: {response}"
        );

        worker.join().expect("worker thread joins");
    }

    /// #361: the main HTTP server runs on tiny_http, whose internal (unnamed)
    /// accept thread used std `TcpListener::accept()` — which PANICS on a
    /// truncated macOS dual-stack `[::]` sockaddr (a peer that RSTs mid-accept).
    /// #345 fixed our ws/sse/shard accept loops via `accept_tcp` but NOT
    /// tiny_http's, so a fresh `pylon dev` on macOS still crashed
    /// (`assertion failed: len >= size_of::<sockaddr_in6>()`). The vendored
    /// `Listener::accept()` is now panic-safe (libc::accept with a null addr +
    /// a strict-length getpeername). Boot the REAL server (`build_http_server`,
    /// the production dual-stack bind), hammer it with connect-then-abort, then
    /// confirm a normal request is STILL served — proving the accept thread
    /// survived. On macOS this drives the truncation path directly; on Linux
    /// it's a survives-aborts smoke test (Linux accept never truncates).
    #[test]
    fn server_accept_survives_a_burst_of_aborted_connections() {
        let server = build_http_server(0).expect("bind");
        let bound = server.server_addr().to_ip().expect("addr");
        let port = bound.port();
        let connect_addr: SocketAddr = if bound.is_ipv6() {
            (Ipv6Addr::LOCALHOST, port).into()
        } else {
            (Ipv4Addr::LOCALHOST, port).into()
        };

        // Serve exactly one REAL request. Aborted connections never reach
        // recv() — tiny_http discards them inside its accept thread, the very
        // place that used to panic.
        let worker = std::thread::spawn(move || {
            if let Ok(req) = server.recv() {
                let _ = req.respond(Response::from_string("pong"));
            }
        });

        // Hammer: open then immediately drop a burst of connections to hit the
        // accept-time truncation window. A rapid open/close burst (the shape of
        // a browser / HMR client reconnecting) is exactly what crashed the
        // original `pylon dev` on macOS ::1.
        for _ in 0..300 {
            if let Ok(s) = TcpStream::connect(connect_addr) {
                drop(s);
            }
        }

        // A normal request must still be accepted + answered. If the accept
        // thread had panicked, recv() would never fire and this read would
        // block until the timeout, failing the test.
        let mut stream = None;
        for _ in 0..100 {
            match TcpStream::connect(connect_addr) {
                Ok(s) => {
                    stream = Some(s);
                    break;
                }
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        let mut stream = stream.expect("connect a real request after the abort burst");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .write_all(b"GET / HTTP/1.0\r\nHost: localhost\r\n\r\n")
            .expect("write request");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read response");
        assert!(
            response.contains("200 OK") && response.contains("pong"),
            "server did not serve after the abort burst (accept thread died?); got: {response}"
        );
        worker.join().expect("worker thread joins");
    }
}
