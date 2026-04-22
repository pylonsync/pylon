use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::Plugin;

/// Session with expiry tracking.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct TrackedSession {
    token: String,
    user_id: String,
    created_at: u64,
    last_active: u64,
    expires_at: u64,
}

/// Session expiry plugin. Tracks session lifetimes and rejects expired sessions.
pub struct SessionExpiryPlugin {
    /// Maximum session lifetime in seconds (absolute expiry).
    max_lifetime: u64,
    /// Idle timeout in seconds (expires if no activity).
    idle_timeout: u64,
    sessions: Mutex<HashMap<String, TrackedSession>>,
}

impl SessionExpiryPlugin {
    /// Create with default settings: 24h max lifetime, 2h idle timeout.
    pub fn new() -> Self {
        Self {
            max_lifetime: 86400,  // 24 hours
            idle_timeout: 7200,   // 2 hours
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// Create with custom timeouts.
    pub fn with_timeouts(max_lifetime: Duration, idle_timeout: Duration) -> Self {
        Self {
            max_lifetime: max_lifetime.as_secs(),
            idle_timeout: idle_timeout.as_secs(),
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// Register a session for tracking.
    pub fn track(&self, token: &str, user_id: &str) {
        let now = now_secs();
        self.sessions.lock().unwrap().insert(
            token.to_string(),
            TrackedSession {
                token: token.to_string(),
                user_id: user_id.to_string(),
                created_at: now,
                last_active: now,
                expires_at: now + self.max_lifetime,
            },
        );
    }

    /// Check if a session is still valid. Updates last_active if valid.
    pub fn check(&self, token: &str) -> Result<String, String> {
        let now = now_secs();
        let mut sessions = self.sessions.lock().unwrap();

        let session = sessions.get_mut(token).ok_or("Session not found")?;

        // Check absolute expiry.
        if now > session.expires_at {
            sessions.remove(token);
            return Err("Session expired".into());
        }

        // Check idle timeout.
        if now - session.last_active > self.idle_timeout {
            sessions.remove(token);
            return Err("Session timed out due to inactivity".into());
        }

        // Session is valid — update last_active.
        session.last_active = now;
        Ok(session.user_id.clone())
    }

    /// Explicitly expire a session.
    pub fn expire(&self, token: &str) -> bool {
        self.sessions.lock().unwrap().remove(token).is_some()
    }

    /// Clean up all expired sessions.
    pub fn cleanup(&self) -> usize {
        let now = now_secs();
        let mut sessions = self.sessions.lock().unwrap();
        let before = sessions.len();
        sessions.retain(|_, s| {
            s.expires_at > now && (now - s.last_active) <= self.idle_timeout
        });
        before - sessions.len()
    }

    /// Get the number of active sessions.
    pub fn active_count(&self) -> usize {
        self.sessions.lock().unwrap().len()
    }

    /// Refresh a session's expiry (extend the lifetime).
    ///
    /// The new `expires_at` is capped at `created_at + max_lifetime`, so a
    /// session can be kept alive by activity but will still be forced to
    /// re-authenticate when its absolute lifetime is up. Previously this
    /// method set `expires_at = now + max_lifetime` unconditionally, which
    /// meant a busy user could renew their session indefinitely — defeating
    /// the whole point of an "absolute" lifetime cap.
    pub fn refresh(&self, token: &str) -> bool {
        let now = now_secs();
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(session) = sessions.get_mut(token) {
            let hard_cap = session.created_at.saturating_add(self.max_lifetime);
            if now >= hard_cap {
                // Session is past its absolute lifetime — refusing to renew.
                sessions.remove(token);
                return false;
            }
            session.last_active = now;
            let proposed = now.saturating_add(self.max_lifetime);
            session.expires_at = proposed.min(hard_cap);
            true
        } else {
            false
        }
    }
}

impl Plugin for SessionExpiryPlugin {
    fn name(&self) -> &str {
        "session-expiry"
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_and_check() {
        let plugin = SessionExpiryPlugin::new();
        plugin.track("token-1", "user-1");

        let user_id = plugin.check("token-1").unwrap();
        assert_eq!(user_id, "user-1");
    }

    #[test]
    fn unknown_token_fails() {
        let plugin = SessionExpiryPlugin::new();
        assert!(plugin.check("unknown").is_err());
    }

    #[test]
    fn expire_session() {
        let plugin = SessionExpiryPlugin::new();
        plugin.track("token-1", "user-1");

        assert!(plugin.expire("token-1"));
        assert!(plugin.check("token-1").is_err());
    }

    #[test]
    fn active_count() {
        let plugin = SessionExpiryPlugin::new();
        assert_eq!(plugin.active_count(), 0);
        plugin.track("t1", "u1");
        plugin.track("t2", "u2");
        assert_eq!(plugin.active_count(), 2);
    }

    #[test]
    fn refresh_extends_lifetime() {
        let plugin = SessionExpiryPlugin::new();
        plugin.track("t1", "u1");
        assert!(plugin.refresh("t1"));
        assert!(plugin.check("t1").is_ok());
    }

    #[test]
    fn refresh_unknown_returns_false() {
        let plugin = SessionExpiryPlugin::new();
        assert!(!plugin.refresh("unknown"));
    }

    #[test]
    fn cleanup_removes_expired() {
        let plugin = SessionExpiryPlugin::with_timeouts(
            Duration::from_secs(86400),
            Duration::from_secs(86400),
        );
        plugin.track("t1", "u1");
        // Not expired yet, cleanup should remove 0.
        let removed = plugin.cleanup();
        assert_eq!(removed, 0);
        assert_eq!(plugin.active_count(), 1);
    }

    #[test]
    fn custom_timeouts() {
        let plugin = SessionExpiryPlugin::with_timeouts(
            Duration::from_secs(3600),
            Duration::from_secs(600),
        );
        plugin.track("t1", "u1");
        assert!(plugin.check("t1").is_ok());
    }
}
