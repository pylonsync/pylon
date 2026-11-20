use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use crate::log_ring::{log_ring, RingEntry};
use crate::tinybird_logger::{iso_now_ms, TinybirdLogger};

/// Rolling per-minute buckets for the request, server error and client
/// error counts.
/// Sized for the Studio Overview's "last 60 minutes" sparkline. Cheap
/// to update (one mutex lock + two array bumps per request) and tiny
/// (~960 bytes). The Mutex-guarded path runs once per request,
/// dwarfed by everything else in the dispatch — not a hot-path
/// concern.
const ROLLUP_MINUTES: usize = 60;

/// How a response status is counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StatusClass {
    /// 1xx, 2xx and 3xx. A `101 Switching Protocols` (every WebSocket
    /// upgrade, including `/shard`) is a success.
    Ok,
    /// 4xx: the server refused the request (not found, unauthorized, rate
    /// limited). Scanners and bad clients make these, so they are not errors.
    ClientError,
    /// 5xx, and a status below 100, which is not valid HTTP.
    ServerError,
}

fn status_class(status: u16) -> StatusClass {
    match status {
        100..=399 => StatusClass::Ok,
        400..=499 => StatusClass::ClientError,
        _ => StatusClass::ServerError,
    }
}

struct RequestBuckets {
    /// Wall-clock minute (epoch seconds / 60) the head bucket
    /// represents. We rotate when the current minute moves past this.
    head_minute: u64,
    /// Counts per minute, ring-buffered. `requests[0]` is the most recent
    /// minute, `requests[ROLLUP_MINUTES-1]` is 59 minutes ago. `errors`
    /// counts server errors and `client_errors` counts 4xx responses.
    requests: [u64; ROLLUP_MINUTES],
    errors: [u64; ROLLUP_MINUTES],
    client_errors: [u64; ROLLUP_MINUTES],
    /// Request durations per minute, as counts per [`LATENCY_BOUNDS_MS`]
    /// bucket. The last slot counts durations above the largest bound.
    latency: [[u32; LATENCY_BUCKETS]; ROLLUP_MINUTES],
}

/// Upper bounds of the latency histogram buckets, in milliseconds.
const LATENCY_BOUNDS_MS: [u32; 12] = [1, 2, 5, 10, 25, 50, 100, 250, 500, 1000, 2500, 5000];
const LATENCY_BUCKETS: usize = LATENCY_BOUNDS_MS.len() + 1;
/// What a percentile reports when it falls in the bucket above the largest bound.
const LATENCY_OVERFLOW_MS: u32 = 10_000;

/// The percentile `p` (0.0 to 1.0) of a latency histogram: the upper bound of
/// the bucket that holds it. `None` when the histogram has no samples.
fn latency_percentile(counts: &[u32; LATENCY_BUCKETS], p: f64) -> Option<u32> {
    let total: u64 = counts.iter().map(|c| u64::from(*c)).sum();
    if total == 0 {
        return None;
    }
    let rank = ((total as f64) * p).ceil().max(1.0) as u64;
    let mut seen = 0u64;
    for (i, count) in counts.iter().enumerate() {
        seen += u64::from(*count);
        if seen >= rank {
            return Some(
                LATENCY_BOUNDS_MS
                    .get(i)
                    .copied()
                    .unwrap_or(LATENCY_OVERFLOW_MS),
            );
        }
    }
    Some(LATENCY_OVERFLOW_MS)
}

impl RequestBuckets {
    fn new() -> Self {
        Self {
            head_minute: 0,
            requests: [0; ROLLUP_MINUTES],
            errors: [0; ROLLUP_MINUTES],
            client_errors: [0; ROLLUP_MINUTES],
            latency: [[0; LATENCY_BUCKETS]; ROLLUP_MINUTES],
        }
    }

    /// Bump the current-minute bucket. Rotates the ring when the wall
    /// clock crosses a minute boundary. Drops samples if we go more
    /// than ROLLUP_MINUTES minutes idle (sparkline shows zeros for
    /// missing slots — accurate).
    ///
    /// `duration_ms` is the wall time of the request. `None` for a request
    /// the server does not time (`/health`, `/metrics`).
    fn record(&mut self, status: u16, duration_ms: Option<u32>) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() / 60)
            .unwrap_or(0);
        if self.head_minute == 0 {
            self.head_minute = now;
        }
        let advance = now.saturating_sub(self.head_minute) as usize;
        if advance > 0 {
            let shift = advance.min(ROLLUP_MINUTES);
            // Shift ring right by `shift` slots, zeroing the freed head.
            self.requests.rotate_right(shift);
            self.errors.rotate_right(shift);
            self.client_errors.rotate_right(shift);
            self.latency.rotate_right(shift);
            for i in 0..shift {
                self.requests[i] = 0;
                self.errors[i] = 0;
                self.client_errors[i] = 0;
                self.latency[i] = [0; LATENCY_BUCKETS];
            }
            self.head_minute = now;
        }
        self.requests[0] = self.requests[0].saturating_add(1);
        match status_class(status) {
            StatusClass::Ok => {}
            StatusClass::ClientError => {
                self.client_errors[0] = self.client_errors[0].saturating_add(1);
            }
            StatusClass::ServerError => self.errors[0] = self.errors[0].saturating_add(1),
        }
        if let Some(ms) = duration_ms {
            let bucket = LATENCY_BOUNDS_MS
                .iter()
                .position(|bound| ms <= *bound)
                .unwrap_or(LATENCY_BUCKETS - 1);
            self.latency[0][bucket] = self.latency[0][bucket].saturating_add(1);
        }
    }

    fn snapshot(&self) -> (Vec<u64>, Vec<u64>, Vec<u64>) {
        (
            self.requests.to_vec(),
            self.errors.to_vec(),
            self.client_errors.to_vec(),
        )
    }

    /// Latency percentiles for the whole window and for each minute.
    /// A minute with no timed request reports `null`.
    fn latency_snapshot(&self) -> serde_json::Value {
        let mut window = [0u32; LATENCY_BUCKETS];
        for minute in &self.latency {
            for (total, count) in window.iter_mut().zip(minute) {
                *total = total.saturating_add(*count);
            }
        }
        let per_minute = |p: f64| -> Vec<Option<u32>> {
            self.latency
                .iter()
                .map(|minute| latency_percentile(minute, p))
                .collect()
        };
        serde_json::json!({
            "p50": latency_percentile(&window, 0.50),
            "p95": latency_percentile(&window, 0.95),
            "p99": latency_percentile(&window, 0.99),
            "p50_per_minute": per_minute(0.50),
            "p95_per_minute": per_minute(0.95),
            "p99_per_minute": per_minute(0.99),
            "bucket_bounds_ms": LATENCY_BOUNDS_MS,
        })
    }
}

/// Per-HTTP-method request counters.
pub struct MethodCounters {
    pub get: AtomicU64,
    pub post: AtomicU64,
    pub patch: AtomicU64,
    pub delete: AtomicU64,
    pub options: AtomicU64,
}

impl MethodCounters {
    fn new() -> Self {
        Self {
            get: AtomicU64::new(0),
            post: AtomicU64::new(0),
            patch: AtomicU64::new(0),
            delete: AtomicU64::new(0),
            options: AtomicU64::new(0),
        }
    }

    fn increment(&self, method: &str) {
        match method {
            "GET" => self.get.fetch_add(1, Ordering::Relaxed),
            "POST" => self.post.fetch_add(1, Ordering::Relaxed),
            "PATCH" => self.patch.fetch_add(1, Ordering::Relaxed),
            "DELETE" => self.delete.fetch_add(1, Ordering::Relaxed),
            "OPTIONS" => self.options.fetch_add(1, Ordering::Relaxed),
            _ => 0,
        };
    }
}

/// Lightweight, lock-free request metrics.
///
/// All counters use relaxed atomic ordering — sufficient for monitoring
/// where exact cross-thread consistency is not required.
pub struct Metrics {
    pub requests_total: AtomicU64,
    pub requests_ok: AtomicU64,
    /// Responses with a 5xx status.
    pub requests_err: AtomicU64,
    /// Responses with a 4xx status.
    pub requests_client_err: AtomicU64,
    pub requests_by_method: MethodCounters,
    /// Rolling 60-minute window for the Studio Overview sparkline.
    /// Mutex-guarded because we rotate the ring on minute boundaries.
    request_buckets: Mutex<RequestBuckets>,
    start_time: Instant,
}

/// Per-request context tracked across the dispatch path so the
/// `record_request` access log can emit URL + duration without every
/// response site threading them in. Set at request receive in
/// `server.rs`, consumed on `record_request`.
struct CurrentRequest {
    path: String,
    started: std::time::Instant,
    /// The signed-in caller, stamped by `set_current_request_user` once the
    /// server resolves the session.
    user: Option<String>,
    /// The request is one of Studio's own: a Studio page, the admin API, or
    /// an API call that Studio made.
    studio: bool,
    /// Request body size in bytes — stamped by `set_current_request_bytes`
    /// after the body read in the central dispatch loop. Pre-fix the
    /// shipper hardcoded `bytes_in=0` for every event.
    request_bytes: u32,
    /// Response body size in bytes — stamped by `set_current_response_bytes`
    /// at send time. Pre-fix the shipper hardcoded `bytes_out=0`, so
    /// every project's "Egress" tile on the usage page was 0 regardless
    /// of actual traffic.
    response_bytes: u32,
}

thread_local! {
    static CURRENT_REQUEST: std::cell::Cell<Option<CurrentRequest>> = const { std::cell::Cell::new(None) };
}

thread_local! {
    static RESPONSE_STATUS: std::cell::Cell<Option<u16>> = const { std::cell::Cell::new(None) };
}

/// Store the status of the response this thread sent. The frontend module
/// sends many kinds of response (pages, files, redirects, not-found pages,
/// render errors), and the server reads the status back with
/// [`take_response_status`] to count and log the request.
pub(crate) fn note_response_status(status: u16) {
    RESPONSE_STATUS.with(|cell| cell.set(Some(status)));
}

/// Return the status that [`note_response_status`] stored, and clear it.
pub(crate) fn take_response_status() -> Option<u16> {
    RESPONSE_STATUS.with(|cell| cell.take())
}

/// Remove query strings and fragments, which can contain credentials.
pub(crate) fn request_log_path(url: &str) -> &str {
    url.split(['?', '#']).next().unwrap_or(url)
}

/// Store the request path and start time for the response log.
/// Discard query strings before storing the context.
pub fn set_current_request(url: &str, started: std::time::Instant) {
    CURRENT_REQUEST.with(|cell| {
        cell.set(Some(CurrentRequest {
            path: request_log_path(url).to_string(),
            started,
            user: None,
            studio: false,
            request_bytes: 0,
            response_bytes: 0,
        }))
    });
}

/// Mark the in-flight request as one of Studio's own, before the caller is
/// known. No-op when no request context exists.
pub fn set_current_request_studio(studio: bool) {
    CURRENT_REQUEST.with(|cell| {
        if let Some(mut ctx) = cell.take() {
            ctx.studio = studio;
            cell.set(Some(ctx));
        }
    });
}

/// Record who makes the in-flight request, for the request log: the id of the
/// signed-in caller, and whether the request is one of Studio's own. No-op
/// when no request context exists.
pub fn set_current_request_caller(user_id: Option<&str>, studio: bool) {
    CURRENT_REQUEST.with(|cell| {
        if let Some(mut ctx) = cell.take() {
            ctx.user = user_id.map(str::to_string);
            ctx.studio = studio;
            cell.set(Some(ctx));
        }
    });
}

/// Stamp the request body length after the central dispatch loop
/// reads it. Used by the shipper's `bytes_in` field. No-op when no
/// request context exists (test paths, /health, etc).
pub fn set_current_request_bytes(bytes: usize) {
    CURRENT_REQUEST.with(|cell| {
        if let Some(mut ctx) = cell.take() {
            ctx.request_bytes = u32::try_from(bytes).unwrap_or(u32::MAX);
            cell.set(Some(ctx));
        }
    });
}

/// Update the in-flight request's response body size. Called from the
/// response-send sites in `server.rs` once the body is known. No-op
/// if no request context was set (test harnesses, paths that skipped
/// `set_current_request`). Idempotent — last call wins.
pub fn set_current_response_bytes(bytes: usize) {
    CURRENT_REQUEST.with(|cell| {
        if let Some(mut ctx) = cell.take() {
            ctx.response_bytes = u32::try_from(bytes).unwrap_or(u32::MAX);
            cell.set(Some(ctx));
        }
    });
}

// Process-global Tinybird shipper. Initialized once at server startup
// if PYLON_TINYBIRD_TOKEN is set; otherwise stays None and the shipper
// hot path is a single `is_none()` check. OnceLock avoids the lazy-init
// allocation on every request.
static TINYBIRD_LOGGER: OnceLock<Option<Arc<TinybirdLogger>>> = OnceLock::new();

/// Initialize the Tinybird request-log shipper from env. Idempotent:
/// subsequent calls are no-ops (the first call wins). Call once during
/// server boot, before requests start arriving.
pub fn init_tinybird_logger() {
    TINYBIRD_LOGGER.get_or_init(TinybirdLogger::from_env);
}

fn tinybird_logger() -> Option<&'static Arc<TinybirdLogger>> {
    TINYBIRD_LOGGER.get().and_then(|opt| opt.as_ref())
}

impl Metrics {
    /// Create a new metrics instance. The uptime clock starts immediately.
    pub fn new() -> Self {
        Self {
            requests_total: AtomicU64::new(0),
            requests_ok: AtomicU64::new(0),
            requests_err: AtomicU64::new(0),
            requests_client_err: AtomicU64::new(0),
            requests_by_method: MethodCounters::new(),
            request_buckets: Mutex::new(RequestBuckets::new()),
            start_time: Instant::now(),
        }
    }

    /// Record a completed request. A 1xx-3xx status is a success, a 4xx
    /// status is a client error, and anything else is a server error.
    ///
    /// Also emits an access log line via tracing if a thread-local
    /// request context was set (see [`set_current_request`]). The
    /// thread-local trick keeps the existing 30+ call sites of this
    /// method untouched while still giving us Next.js-style
    /// `GET /foo 200 in 27ms` output for free. Server `recv()` is
    /// single-threaded per request, so the thread-local matches the
    /// in-flight request without cross-talk.
    pub fn record_request(&self, method: &str, status: u16) {
        self.requests_total.fetch_add(1, Ordering::Relaxed);
        let counter = match status_class(status) {
            StatusClass::Ok => &self.requests_ok,
            StatusClass::ClientError => &self.requests_client_err,
            StatusClass::ServerError => &self.requests_err,
        };
        counter.fetch_add(1, Ordering::Relaxed);
        self.requests_by_method.increment(method);

        // Pull the per-request context if set. We log even without it
        // (just method + status) so callers that haven't been wired
        // through still get partial visibility. /health and /metrics
        // are noisy and intentionally skipped at the call site of
        // `set_current_request` — they don't set the thread-local, so
        // the URL ends up "?".
        let ctx = CURRENT_REQUEST.take();
        let duration_ms = ctx
            .as_ref()
            .map(|c| u32::try_from(c.started.elapsed().as_millis()).unwrap_or(u32::MAX));
        // Rolling 60-minute buckets. Lock for the rotation + bump;
        // contention is fine — `record_request` already serializes
        // through one tracing::info! per call.
        if let Ok(mut buckets) = self.request_buckets.lock() {
            buckets.record(status, duration_ms);
        }
        match ctx {
            Some(c) => {
                let cpu_ms = duration_ms.unwrap_or(0);
                tracing::info!("← {} {} {} in {}ms", method, c.path, status, cpu_ms);
                log_row(
                    method,
                    &c.path,
                    status,
                    cpu_ms,
                    c.request_bytes,
                    c.response_bytes,
                    None,
                    c.user.as_deref(),
                    c.studio,
                );
            }
            None => {
                tracing::debug!("← {} {} (no per-request ctx)", method, status);
            }
        }
    }

    /// Seconds elapsed since this `Metrics` instance was created.
    pub fn uptime_secs(&self) -> u64 {
        self.start_time.elapsed().as_secs()
    }

    /// Return a JSON snapshot of all current metrics, including the
    /// rolling 60-minute requests + errors series used by the Studio
    /// Overview sparkline. Index 0 is the current minute (partial),
    /// index 59 is 59 minutes ago.
    pub fn snapshot(&self) -> serde_json::Value {
        let ((requests_per_min, errors_per_min, client_errors_per_min), latency) = self
            .request_buckets
            .lock()
            .map(|b| (b.snapshot(), b.latency_snapshot()))
            .unwrap_or((
                (
                    vec![0; ROLLUP_MINUTES],
                    vec![0; ROLLUP_MINUTES],
                    vec![0; ROLLUP_MINUTES],
                ),
                serde_json::Value::Null,
            ));
        serde_json::json!({
            "uptime_secs": self.uptime_secs(),
            "requests": {
                "total": self.requests_total.load(Ordering::Relaxed),
                "ok": self.requests_ok.load(Ordering::Relaxed),
                "error": self.requests_err.load(Ordering::Relaxed),
                "client_error": self.requests_client_err.load(Ordering::Relaxed),
                "per_minute": requests_per_min,
                "errors_per_minute": errors_per_min,
                "client_errors_per_minute": client_errors_per_min,
            },
            "latency_ms": latency,
            "ssr_cache": crate::ssr_cache::stats::json(),
            "methods": {
                "GET": self.requests_by_method.get.load(Ordering::Relaxed),
                "POST": self.requests_by_method.post.load(Ordering::Relaxed),
                "PATCH": self.requests_by_method.patch.load(Ordering::Relaxed),
                "DELETE": self.requests_by_method.delete.load(Ordering::Relaxed),
            }
        })
    }

    /// Return metrics in Prometheus text exposition format.
    ///
    /// Supports scraping by Prometheus, Grafana Agent, OTel collector, etc.
    pub fn prometheus(&self) -> String {
        let total = self.requests_total.load(Ordering::Relaxed);
        let ok = self.requests_ok.load(Ordering::Relaxed);
        let err = self.requests_err.load(Ordering::Relaxed);
        let client_err = self.requests_client_err.load(Ordering::Relaxed);
        let uptime = self.uptime_secs();
        let get = self.requests_by_method.get.load(Ordering::Relaxed);
        let post = self.requests_by_method.post.load(Ordering::Relaxed);
        let patch = self.requests_by_method.patch.load(Ordering::Relaxed);
        let delete = self.requests_by_method.delete.load(Ordering::Relaxed);
        let options = self.requests_by_method.options.load(Ordering::Relaxed);

        format!(
            "# HELP pylon_uptime_seconds Server uptime in seconds.\n\
             # TYPE pylon_uptime_seconds gauge\n\
             pylon_uptime_seconds {uptime}\n\
             # HELP pylon_http_requests_total HTTP requests total.\n\
             # TYPE pylon_http_requests_total counter\n\
             pylon_http_requests_total {total}\n\
             # HELP pylon_http_requests_ok_total HTTP requests with 1xx/2xx/3xx status.\n\
             # TYPE pylon_http_requests_ok_total counter\n\
             pylon_http_requests_ok_total {ok}\n\
             # HELP pylon_http_requests_errors_total HTTP requests with 5xx status.\n\
             # TYPE pylon_http_requests_errors_total counter\n\
             pylon_http_requests_errors_total {err}\n\
             # HELP pylon_http_requests_client_errors_total HTTP requests with 4xx status.\n\
             # TYPE pylon_http_requests_client_errors_total counter\n\
             pylon_http_requests_client_errors_total {client_err}\n\
             # HELP pylon_http_requests_by_method HTTP requests by method.\n\
             # TYPE pylon_http_requests_by_method counter\n\
             pylon_http_requests_by_method{{method=\"GET\"}} {get}\n\
             pylon_http_requests_by_method{{method=\"POST\"}} {post}\n\
             pylon_http_requests_by_method{{method=\"PATCH\"}} {patch}\n\
             pylon_http_requests_by_method{{method=\"DELETE\"}} {delete}\n\
             pylon_http_requests_by_method{{method=\"OPTIONS\"}} {options}\n"
        )
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Write one row to the request log: the Tinybird (or Pylon Cloud) shipper
/// and the in-process ring that `pylon logs` tails. It does not change the
/// request counters, so a caller can log an event that is not a new request,
/// such as the end of a WebSocket connection.
///
/// `cpu_ms` is the wall time of the request or connection. `error` is why it
/// failed; `None` when it did not.
pub fn record_log_row(
    method: &str,
    path: &str,
    status: u16,
    cpu_ms: u32,
    bytes_in: u32,
    bytes_out: u32,
    error: Option<&str>,
) {
    log_row(
        method, path, status, cpu_ms, bytes_in, bytes_out, error, None, false,
    );
}

/// [`record_log_row`] with the id of the caller, which only the request path
/// knows.
#[allow(clippy::too_many_arguments)]
fn log_row(
    method: &str,
    path: &str,
    status: u16,
    cpu_ms: u32,
    bytes_in: u32,
    bytes_out: u32,
    error: Option<&str>,
    user: Option<&str>,
    studio: bool,
) {
    let path = request_log_path(path);
    // Fire-and-forget into the shipper's background channel. `record()`
    // returns at once even on backpressure (it drops the event), so the cost
    // on the hot path is one channel try_send.
    if let Some(logger) = tinybird_logger() {
        logger.record(
            method,
            path,
            status,
            cpu_ms,
            bytes_in,
            bytes_out,
            error.unwrap_or(""),
        );
    }
    // The ring is what the dashboard and `pylon logs` tail poll, so they do
    // not spend the Tinybird quota on every refresh.
    if let Some(ring) = log_ring() {
        ring.push(RingEntry {
            timestamp: iso_now_ms(),
            method: method.to_string(),
            path: path.to_string(),
            status,
            cpu_ms,
            bytes_in,
            bytes_out,
            user: user.map(str::to_string),
            studio,
            error: error.map(str::to_string),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latency_percentile_is_none_with_no_samples() {
        assert_eq!(latency_percentile(&[0; LATENCY_BUCKETS], 0.5), None);
    }

    #[test]
    fn latency_percentiles_report_the_bucket_bound() {
        let mut b = RequestBuckets::new();
        for _ in 0..90 {
            b.record(200, Some(3)); // bucket <= 5 ms
        }
        for _ in 0..9 {
            b.record(200, Some(80)); // bucket <= 100 ms
        }
        b.record(500, Some(60_000)); // above the largest bound
        b.record(200, None); // not timed: counted as a request only
        let snap = b.latency_snapshot();
        assert_eq!(snap["p50"], 5);
        assert_eq!(snap["p95"], 100);
        assert_eq!(snap["p99"], 100);
        assert_eq!(snap["p50_per_minute"][0], 5);
        assert!(snap["p50_per_minute"][1].is_null());
        assert_eq!(b.requests[0], 101);
        assert_eq!(
            latency_percentile(&b.latency[0], 1.0),
            Some(LATENCY_OVERFLOW_MS)
        );
    }

    #[test]
    fn a_studio_request_is_marked_before_the_caller_is_known() {
        crate::log_ring::init_log_ring();
        set_current_request("/admin/fn/traces?limit=50", std::time::Instant::now());
        // The traces endpoint answers before the server resolves the caller.
        set_current_request_studio(true);
        Metrics::new().record_request("GET", 299);
        let rows = log_ring().unwrap().tail_since(None);
        let row = rows
            .iter()
            .rev()
            .find(|r| r.path == "/admin/fn/traces" && r.status == 299)
            .expect("the row is in the ring");
        assert!(row.studio);
    }

    #[test]
    fn the_request_log_row_carries_the_caller_and_sizes() {
        crate::log_ring::init_log_ring();
        set_current_request("/api/entities/Deal?token=secret", std::time::Instant::now());
        set_current_request_caller(Some("user_42"), true);
        set_current_request_bytes(12);
        set_current_response_bytes(345);
        Metrics::new().record_request("POST", 201);
        let rows = log_ring().unwrap().tail_since(None);
        let row = rows
            .iter()
            .find(|r| r.user.as_deref() == Some("user_42"))
            .expect("the row is in the ring");
        assert_eq!(row.path, "/api/entities/Deal");
        assert_eq!((row.bytes_in, row.bytes_out, row.status), (12, 345, 201));
        assert!(row.studio);
    }

    #[test]
    fn request_context_discards_query_credentials() {
        for (url, expected) in [
            (
                "/auth/callback?code=synthetic-secret&state=private",
                "/auth/callback",
            ),
            ("/login#access_token=synthetic-secret", "/login"),
            ("/api/items?filter%5Bkey%5D=secret#fragment", "/api/items"),
            ("/api/items", "/api/items"),
        ] {
            assert_eq!(request_log_path(url), expected);
            set_current_request(url, Instant::now());
            let context = CURRENT_REQUEST.take().unwrap();
            assert_eq!(context.path, expected);
        }
    }

    #[test]
    fn new_metrics_are_zero() {
        let m = Metrics::new();
        assert_eq!(m.requests_total.load(Ordering::Relaxed), 0);
        assert_eq!(m.requests_ok.load(Ordering::Relaxed), 0);
        assert_eq!(m.requests_err.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn record_ok_request() {
        let m = Metrics::new();
        m.record_request("GET", 200);
        assert_eq!(m.requests_total.load(Ordering::Relaxed), 1);
        assert_eq!(m.requests_ok.load(Ordering::Relaxed), 1);
        assert_eq!(m.requests_err.load(Ordering::Relaxed), 0);
        assert_eq!(m.requests_by_method.get.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn record_error_request() {
        let m = Metrics::new();
        m.record_request("POST", 500);
        assert_eq!(m.requests_total.load(Ordering::Relaxed), 1);
        assert_eq!(m.requests_ok.load(Ordering::Relaxed), 0);
        assert_eq!(m.requests_err.load(Ordering::Relaxed), 1);
        assert_eq!(m.requests_by_method.post.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn websocket_upgrade_is_not_an_error() {
        let m = Metrics::new();
        m.record_request("GET", 101);
        assert_eq!(m.requests_ok.load(Ordering::Relaxed), 1);
        assert_eq!(m.requests_err.load(Ordering::Relaxed), 0);
        let snap = m.snapshot();
        assert_eq!(snap["requests"]["errors_per_minute"][0], 0);
    }

    #[test]
    fn method_counters_increment_independently() {
        let m = Metrics::new();
        m.record_request("GET", 200);
        m.record_request("GET", 200);
        m.record_request("POST", 201);
        m.record_request("DELETE", 204);
        m.record_request("PATCH", 200);
        m.record_request("OPTIONS", 204);

        assert_eq!(m.requests_by_method.get.load(Ordering::Relaxed), 2);
        assert_eq!(m.requests_by_method.post.load(Ordering::Relaxed), 1);
        assert_eq!(m.requests_by_method.delete.load(Ordering::Relaxed), 1);
        assert_eq!(m.requests_by_method.patch.load(Ordering::Relaxed), 1);
        assert_eq!(m.requests_by_method.options.load(Ordering::Relaxed), 1);
        assert_eq!(m.requests_total.load(Ordering::Relaxed), 6);
    }

    #[test]
    fn snapshot_returns_valid_json() {
        let m = Metrics::new();
        m.record_request("GET", 200);
        m.record_request("POST", 400);

        m.record_request("GET", 503);

        let snap = m.snapshot();
        assert_eq!(snap["requests"]["total"], 3);
        assert_eq!(snap["requests"]["ok"], 1);
        assert_eq!(snap["requests"]["error"], 1);
        assert_eq!(snap["requests"]["client_error"], 1);
        assert_eq!(snap["requests"]["errors_per_minute"][0], 1);
        assert_eq!(snap["requests"]["client_errors_per_minute"][0], 1);
        assert_eq!(snap["methods"]["GET"], 2);
        assert_eq!(snap["methods"]["POST"], 1);
        assert_eq!(snap["methods"]["PATCH"], 0);
        assert_eq!(snap["methods"]["DELETE"], 0);
        assert!(snap["uptime_secs"].as_u64().is_some());
    }

    #[test]
    fn uptime_is_non_negative() {
        let m = Metrics::new();
        assert!(m.uptime_secs() < 2); // should be ~0 immediately after creation
    }

    #[test]
    fn status_boundary_classification() {
        let m = Metrics::new();
        // 2xx = ok
        m.record_request("GET", 200);
        m.record_request("GET", 204);
        m.record_request("GET", 299);
        // 3xx = ok (redirects)
        m.record_request("GET", 301);
        m.record_request("GET", 399);
        // 4xx = client error, not an error
        m.record_request("GET", 400);
        m.record_request("GET", 404);
        m.record_request("GET", 429);
        m.record_request("GET", 499);
        // 5xx and invalid statuses = error
        m.record_request("GET", 500);
        m.record_request("GET", 599);
        m.record_request("GET", 0);

        assert_eq!(m.requests_ok.load(Ordering::Relaxed), 5);
        assert_eq!(m.requests_client_err.load(Ordering::Relaxed), 4);
        assert_eq!(m.requests_err.load(Ordering::Relaxed), 3);
        assert!(m
            .prometheus()
            .contains("pylon_http_requests_client_errors_total 4\n"));
    }
}
