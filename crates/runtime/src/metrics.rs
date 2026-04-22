use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

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
    pub requests_err: AtomicU64,
    pub requests_by_method: MethodCounters,
    start_time: Instant,
}

impl Metrics {
    /// Create a new metrics instance. The uptime clock starts immediately.
    pub fn new() -> Self {
        Self {
            requests_total: AtomicU64::new(0),
            requests_ok: AtomicU64::new(0),
            requests_err: AtomicU64::new(0),
            requests_by_method: MethodCounters::new(),
            start_time: Instant::now(),
        }
    }

    /// Record a completed request. A status code in the 200-399 range is
    /// counted as successful; everything else counts as an error.
    pub fn record_request(&self, method: &str, status: u16) {
        self.requests_total.fetch_add(1, Ordering::Relaxed);
        if (200..400).contains(&status) {
            self.requests_ok.fetch_add(1, Ordering::Relaxed);
        } else {
            self.requests_err.fetch_add(1, Ordering::Relaxed);
        }
        self.requests_by_method.increment(method);
    }

    /// Seconds elapsed since this `Metrics` instance was created.
    pub fn uptime_secs(&self) -> u64 {
        self.start_time.elapsed().as_secs()
    }

    /// Return a JSON snapshot of all current metrics.
    pub fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "uptime_secs": self.uptime_secs(),
            "requests": {
                "total": self.requests_total.load(Ordering::Relaxed),
                "ok": self.requests_ok.load(Ordering::Relaxed),
                "error": self.requests_err.load(Ordering::Relaxed),
            },
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
        let uptime = self.uptime_secs();
        let get = self.requests_by_method.get.load(Ordering::Relaxed);
        let post = self.requests_by_method.post.load(Ordering::Relaxed);
        let patch = self.requests_by_method.patch.load(Ordering::Relaxed);
        let delete = self.requests_by_method.delete.load(Ordering::Relaxed);
        let options = self.requests_by_method.options.load(Ordering::Relaxed);

        format!(
            "# HELP statecraft_uptime_seconds Server uptime in seconds.\n\
             # TYPE statecraft_uptime_seconds gauge\n\
             statecraft_uptime_seconds {uptime}\n\
             # HELP statecraft_http_requests_total HTTP requests total.\n\
             # TYPE statecraft_http_requests_total counter\n\
             statecraft_http_requests_total {total}\n\
             # HELP statecraft_http_requests_ok_total HTTP requests with 2xx/3xx status.\n\
             # TYPE statecraft_http_requests_ok_total counter\n\
             statecraft_http_requests_ok_total {ok}\n\
             # HELP statecraft_http_requests_errors_total HTTP requests with 4xx/5xx status.\n\
             # TYPE statecraft_http_requests_errors_total counter\n\
             statecraft_http_requests_errors_total {err}\n\
             # HELP statecraft_http_requests_by_method HTTP requests by method.\n\
             # TYPE statecraft_http_requests_by_method counter\n\
             statecraft_http_requests_by_method{{method=\"GET\"}} {get}\n\
             statecraft_http_requests_by_method{{method=\"POST\"}} {post}\n\
             statecraft_http_requests_by_method{{method=\"PATCH\"}} {patch}\n\
             statecraft_http_requests_by_method{{method=\"DELETE\"}} {delete}\n\
             statecraft_http_requests_by_method{{method=\"OPTIONS\"}} {options}\n"
        )
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let snap = m.snapshot();
        assert_eq!(snap["requests"]["total"], 2);
        assert_eq!(snap["requests"]["ok"], 1);
        assert_eq!(snap["requests"]["error"], 1);
        assert_eq!(snap["methods"]["GET"], 1);
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
        // 4xx = error
        m.record_request("GET", 400);
        m.record_request("GET", 404);
        // 5xx = error
        m.record_request("GET", 500);

        assert_eq!(m.requests_ok.load(Ordering::Relaxed), 5);
        assert_eq!(m.requests_err.load(Ordering::Relaxed), 3);
    }
}
