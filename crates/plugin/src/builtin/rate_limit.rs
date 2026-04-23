use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::{Plugin, PluginError, RequestMeta};
use pylon_auth::AuthContext;

/// Rate limiting plugin. Limits requests per IP/user within a time window.
pub struct RateLimitPlugin {
    max_requests: u32,
    window: Duration,
    counters: Mutex<HashMap<String, (u32, Instant)>>,
}

impl RateLimitPlugin {
    pub fn new(max_requests: u32, window: Duration) -> Self {
        Self {
            max_requests,
            window,
            counters: Mutex::new(HashMap::new()),
        }
    }

    fn check(&self, key: &str) -> Result<(), PluginError> {
        let mut counters = self.counters.lock().unwrap();
        let now = Instant::now();

        let entry = counters.entry(key.to_string()).or_insert((0, now));

        // Reset if window expired.
        if now.duration_since(entry.1) > self.window {
            *entry = (0, now);
        }

        entry.0 += 1;

        if entry.0 > self.max_requests {
            Err(PluginError {
                code: "RATE_LIMITED".into(),
                message: format!("Too many requests. Limit: {} per {:?}", self.max_requests, self.window),
                status: 429,
            })
        } else {
            Ok(())
        }
    }

    /// Rate-limit by user id when present, otherwise by peer IP. Prefer
    /// this over the Plugin trait's `on_request` hook: that hook has no
    /// access to peer IP and collapses every unauthenticated caller into
    /// a single `__anon__` bucket, which means one attacker can DoS the
    /// entire anonymous client population.
    ///
    /// Call from the HTTP layer where peer IP is available. Pass `""` for
    /// `peer_ip` if unknown — the fallback is the same shared `__anon__`
    /// bucket as before (not worse than the old behavior).
    pub fn check_request(
        &self,
        user_id: Option<&str>,
        peer_ip: &str,
    ) -> Result<(), PluginError> {
        let key = match user_id {
            Some(u) if !u.is_empty() => format!("user:{u}"),
            _ if !peer_ip.is_empty() => format!("ip:{peer_ip}"),
            _ => "__anon__".to_string(),
        };
        self.check(&key)
    }
}

impl Plugin for RateLimitPlugin {
    fn name(&self) -> &str {
        "rate-limit"
    }

    fn on_request(
        &self,
        _method: &str,
        _path: &str,
        auth: &AuthContext,
    ) -> Result<(), PluginError> {
        // Legacy path (no peer_ip available). Keys by user_id or a shared
        // `__anon__` bucket — kept for callers that still invoke
        // `on_request` directly. New callers should prefer
        // `on_request_with_meta` so the IP dimension takes effect.
        let key = auth.user_id.as_deref().unwrap_or("__anon__").to_string();
        self.check(&key)
    }

    fn on_request_with_meta(
        &self,
        _method: &str,
        _path: &str,
        auth: &AuthContext,
        meta: &RequestMeta<'_>,
    ) -> Result<(), PluginError> {
        // Per-IP bucket for anonymous traffic fixes the "one attacker
        // DoSes every anon user" collapse we used to have when all anon
        // callers shared a single `__anon__` bucket.
        self.check_request(auth.user_id.as_deref(), meta.peer_ip)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_under_limit() {
        let plugin = RateLimitPlugin::new(3, Duration::from_secs(60));
        let auth = AuthContext::anonymous();
        assert!(plugin.on_request("GET", "/api/test", &auth).is_ok());
        assert!(plugin.on_request("GET", "/api/test", &auth).is_ok());
        assert!(plugin.on_request("GET", "/api/test", &auth).is_ok());
    }

    #[test]
    fn different_ips_use_different_buckets() {
        let plugin = RateLimitPlugin::new(2, Duration::from_secs(60));
        // Two anonymous clients from different IPs should each get their
        // own bucket under check_request — previously both collapsed into
        // `__anon__` and one could burn the other's quota.
        assert!(plugin.check_request(None, "1.1.1.1").is_ok());
        assert!(plugin.check_request(None, "1.1.1.1").is_ok());
        assert!(plugin.check_request(None, "1.1.1.1").is_err());
        // Second IP is untouched.
        assert!(plugin.check_request(None, "2.2.2.2").is_ok());
        assert!(plugin.check_request(None, "2.2.2.2").is_ok());
    }

    #[test]
    fn user_id_preferred_over_ip() {
        let plugin = RateLimitPlugin::new(2, Duration::from_secs(60));
        // Same user id from different IPs uses one bucket.
        assert!(plugin.check_request(Some("alice"), "1.1.1.1").is_ok());
        assert!(plugin.check_request(Some("alice"), "2.2.2.2").is_ok());
        assert!(plugin.check_request(Some("alice"), "3.3.3.3").is_err());
    }

    #[test]
    fn blocks_over_limit() {
        let plugin = RateLimitPlugin::new(2, Duration::from_secs(60));
        let auth = AuthContext::anonymous();
        assert!(plugin.on_request("GET", "/", &auth).is_ok());
        assert!(plugin.on_request("GET", "/", &auth).is_ok());
        let result = plugin.on_request("GET", "/", &auth);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, "RATE_LIMITED");
    }

    #[test]
    fn separate_users_separate_limits() {
        let plugin = RateLimitPlugin::new(1, Duration::from_secs(60));
        let alice = AuthContext::authenticated("alice".into());
        let bob = AuthContext::authenticated("bob".into());
        assert!(plugin.on_request("GET", "/", &alice).is_ok());
        assert!(plugin.on_request("GET", "/", &bob).is_ok());
        // Alice is now rate limited, Bob is not.
        assert!(plugin.on_request("GET", "/", &alice).is_err());
        assert!(plugin.on_request("GET", "/", &bob).is_err());
    }
}
