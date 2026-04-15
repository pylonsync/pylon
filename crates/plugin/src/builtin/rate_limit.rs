use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::{Plugin, PluginError};
use agentdb_auth::AuthContext;

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
        // Key by user_id if authenticated, otherwise by a generic "anon" key.
        let key = auth.user_id.as_deref().unwrap_or("__anon__").to_string();
        self.check(&key)
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
