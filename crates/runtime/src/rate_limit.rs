use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Per-IP rate limiter using a sliding window.
///
/// Each IP address gets a bucket of timestamps. When a request arrives, expired
/// entries (older than `window`) are pruned, and the remaining count is checked
/// against `max_requests`. If the limit is exceeded, `check()` returns `Err`
/// with the number of seconds the caller should wait before retrying.
pub struct RateLimiter {
    window: Duration,
    max_requests: u32,
    buckets: Mutex<HashMap<String, Vec<Instant>>>,
}

impl RateLimiter {
    /// Create a new rate limiter.
    ///
    /// - `max_requests`: maximum number of requests allowed within the window.
    /// - `window_secs`: sliding window duration in seconds.
    pub fn new(max_requests: u32, window_secs: u64) -> Self {
        Self {
            window: Duration::from_secs(window_secs),
            max_requests,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Check if a request from this IP is allowed.
    ///
    /// Returns `Ok(())` if the request is within limits, or `Err(retry_after)`
    /// with the number of seconds to wait before the next request will be
    /// accepted.
    pub fn check(&self, ip: &str) -> Result<(), u64> {
        let now = Instant::now();
        let mut buckets = self.buckets.lock().unwrap();
        let timestamps = buckets.entry(ip.to_string()).or_default();

        // Remove entries outside the sliding window.
        timestamps.retain(|t| now.duration_since(*t) < self.window);

        if timestamps.len() as u32 >= self.max_requests {
            let oldest = timestamps.first().unwrap();
            let elapsed = now.duration_since(*oldest).as_secs();
            let retry_after = self.window.as_secs().saturating_sub(elapsed);
            // Ensure we always return at least 1 second.
            return Err(retry_after.max(1));
        }

        timestamps.push(now);
        Ok(())
    }

    /// Remove all expired entries from every bucket.
    ///
    /// Call periodically (e.g., from a background thread) to prevent unbounded
    /// memory growth from IPs that stop sending requests.
    pub fn cleanup(&self) {
        let now = Instant::now();
        let mut buckets = self.buckets.lock().unwrap();

        // Remove expired timestamps, then drop empty buckets entirely.
        buckets.retain(|_ip, timestamps| {
            timestamps.retain(|t| now.duration_since(*t) < self.window);
            !timestamps.is_empty()
        });
    }

    /// Get the current request count for an IP within the active window.
    pub fn current_count(&self, ip: &str) -> u32 {
        let now = Instant::now();
        let buckets = self.buckets.lock().unwrap();
        match buckets.get(ip) {
            Some(timestamps) => timestamps
                .iter()
                .filter(|t| now.duration_since(**t) < self.window)
                .count() as u32,
            None => 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn under_limit_passes() {
        let rl = RateLimiter::new(5, 60);
        for _ in 0..5 {
            assert!(rl.check("10.0.0.1").is_ok());
        }
    }

    #[test]
    fn over_limit_rejected() {
        let rl = RateLimiter::new(3, 60);
        for _ in 0..3 {
            assert!(rl.check("10.0.0.1").is_ok());
        }
        let err = rl.check("10.0.0.1").unwrap_err();
        assert!(err >= 1, "retry_after should be at least 1 second");
    }

    #[test]
    fn window_expiry_allows_new_requests() {
        // Use a very short window so the test finishes quickly.
        let rl = RateLimiter::new(2, 1);
        assert!(rl.check("10.0.0.1").is_ok());
        assert!(rl.check("10.0.0.1").is_ok());
        assert!(rl.check("10.0.0.1").is_err());

        // Wait for the window to expire.
        thread::sleep(Duration::from_millis(1100));

        // Should be allowed again.
        assert!(rl.check("10.0.0.1").is_ok());
    }

    #[test]
    fn different_ips_are_independent() {
        let rl = RateLimiter::new(2, 60);
        assert!(rl.check("10.0.0.1").is_ok());
        assert!(rl.check("10.0.0.1").is_ok());
        assert!(rl.check("10.0.0.1").is_err());

        // Different IP should still be allowed.
        assert!(rl.check("10.0.0.2").is_ok());
        assert!(rl.check("10.0.0.2").is_ok());
    }

    #[test]
    fn cleanup_removes_expired_buckets() {
        let rl = RateLimiter::new(10, 1);
        assert!(rl.check("10.0.0.1").is_ok());
        assert!(rl.check("10.0.0.2").is_ok());

        // Wait for expiry.
        thread::sleep(Duration::from_millis(1100));

        rl.cleanup();

        // After cleanup, counts should be zero (expired entries removed).
        assert_eq!(rl.current_count("10.0.0.1"), 0);
        assert_eq!(rl.current_count("10.0.0.2"), 0);
    }

    #[test]
    fn current_count_reflects_active_requests() {
        let rl = RateLimiter::new(10, 60);
        assert_eq!(rl.current_count("10.0.0.1"), 0);

        rl.check("10.0.0.1").unwrap();
        assert_eq!(rl.current_count("10.0.0.1"), 1);

        rl.check("10.0.0.1").unwrap();
        rl.check("10.0.0.1").unwrap();
        assert_eq!(rl.current_count("10.0.0.1"), 3);
    }
}
