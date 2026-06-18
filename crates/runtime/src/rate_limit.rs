use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Number of independent shards the IP map is split across. A power of two so
/// the shard index is a cheap bit-mask of the IP hash. 16 keeps the per-shard
/// lock contention low under high concurrency without wasting memory on empty
/// maps.
const SHARD_COUNT: usize = 16;

type Bucket = HashMap<String, Vec<Instant>>;

/// Per-IP rate limiter using a sliding window.
///
/// Each IP address gets a bucket of timestamps. When a request arrives, expired
/// entries (older than `window`) are pruned, and the remaining count is checked
/// against `max_requests`. If the limit is exceeded, `check()` returns `Err`
/// with the number of seconds the caller should wait before retrying.
///
/// The IP→bucket map is SHARDED across [`SHARD_COUNT`] independently-locked
/// `HashMap`s, keyed by a hash of the IP. A single global `Mutex<HashMap>` was
/// a hard serialization point — every request funneled through one lock
/// regardless of client IP, which shows up immediately under `wrk -c 256`.
/// Sharding spreads that contention so requests from different IPs usually take
/// different locks.
pub struct RateLimiter {
    window: Duration,
    max_requests: u32,
    shards: Box<[Mutex<Bucket>]>,
}

impl RateLimiter {
    /// Create a new rate limiter.
    ///
    /// - `max_requests`: maximum number of requests allowed within the window.
    /// - `window_secs`: sliding window duration in seconds.
    pub fn new(max_requests: u32, window_secs: u64) -> Self {
        let shards = std::iter::repeat_with(|| Mutex::new(HashMap::new()))
            .take(SHARD_COUNT)
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            window: Duration::from_secs(window_secs),
            max_requests,
            shards,
        }
    }

    /// The shard that owns `ip`. `& (SHARD_COUNT - 1)` is a valid mask because
    /// `SHARD_COUNT` is a power of two.
    fn shard_for(&self, ip: &str) -> &Mutex<Bucket> {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        ip.hash(&mut hasher);
        &self.shards[(hasher.finish() as usize) & (SHARD_COUNT - 1)]
    }

    /// Check if a request from this IP is allowed.
    ///
    /// Returns `Ok(())` if the request is within limits, or `Err(retry_after)`
    /// with the number of seconds to wait before the next request will be
    /// accepted.
    pub fn check(&self, ip: &str) -> Result<(), u64> {
        let now = Instant::now();
        let mut buckets = self.shard_for(ip).lock().unwrap_or_else(|e| e.into_inner());

        // Look up by &str first and only allocate an owned key on a genuine
        // miss. The `entry(ip.to_string())` form forced a fresh String on
        // EVERY request, including the common already-present-IP hit.
        if let Some(timestamps) = buckets.get_mut(ip) {
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
            return Ok(());
        }

        // First request from this IP (in this shard) — now we pay for the key.
        buckets.insert(ip.to_string(), vec![now]);
        Ok(())
    }

    /// Remove all expired entries from every bucket.
    ///
    /// Call periodically (e.g., from a background thread) to prevent unbounded
    /// memory growth from IPs that stop sending requests.
    pub fn cleanup(&self) {
        let now = Instant::now();
        for shard in self.shards.iter() {
            let mut buckets = shard.lock().unwrap_or_else(|e| e.into_inner());
            // Remove expired timestamps, then drop empty buckets entirely.
            buckets.retain(|_ip, timestamps| {
                timestamps.retain(|t| now.duration_since(*t) < self.window);
                !timestamps.is_empty()
            });
        }
    }

    /// Get the current request count for an IP within the active window.
    pub fn current_count(&self, ip: &str) -> u32 {
        let now = Instant::now();
        let buckets = self.shard_for(ip).lock().unwrap_or_else(|e| e.into_inner());
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
    fn many_distinct_ips_across_shards_are_independent() {
        // With SHARD_COUNT=16, exercise far more IPs than shards so multiple
        // IPs collide on the same shard — each must still get its own bucket
        // and its own independent limit (no cross-IP bleed from sharing a lock).
        let rl = RateLimiter::new(2, 60);
        for i in 0..200u32 {
            let ip = format!("10.0.{}.{}", i / 256, i % 256);
            assert!(rl.check(&ip).is_ok(), "1st for {ip}");
            assert!(rl.check(&ip).is_ok(), "2nd for {ip}");
            assert!(rl.check(&ip).is_err(), "3rd over limit for {ip}");
            assert_eq!(rl.current_count(&ip), 2, "count for {ip}");
        }
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
