//! In-process token-bucket rate limiter for auth endpoints.
//!
//! Sized for the auth surface specifically: small key space (per-IP
//! and per-account, not per-(IP, route)), short windows, fixed
//! defaults that match Better-Auth's posture. Apps that need
//! cluster-wide rate limits across multiple replicas should put a
//! reverse proxy in front (Cloudflare / Caddy / nginx limit_req).
//!
//! Two scopes:
//!   - **per-IP**: blanket cap on auth attempts from a single client.
//!     Stops trivial credential-stuffing from one box.
//!   - **per-account**: caps attempts against a single
//!     email/user_id/phone — slower than per-IP but harder to bypass
//!     (an attacker who rotates IPs still hits the per-account cap).
//!
//! Limits are tuned to be invisible to humans (1 retry/s leaves you
//! plenty of headroom) but make brute force impractical.

use std::collections::HashMap;
use std::sync::Mutex;

/// Auth endpoint families with distinct rate limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuthBucket {
    /// `/api/auth/password/login`, `/api/auth/totp/verify` — credential
    /// guesses. Strictest cap.
    Login,
    /// `/api/auth/password/register`, `/api/auth/magic-link/send`,
    /// `/api/auth/magic/send`, `/api/auth/password/reset/request`,
    /// `/api/auth/phone/send-code` — sends an email/SMS or creates a
    /// user. Caps email-bombing + signup spam.
    Send,
    /// `/api/auth/passkey/login/finish`, `/api/auth/siwe/verify` —
    /// public verify endpoints with cryptographic gates. Caps the
    /// signature-fuzzing class.
    Verify,
}

impl AuthBucket {
    /// `(per_ip_limit_per_min, per_account_limit_per_hour)`.
    fn caps(&self) -> (u32, u32) {
        match self {
            // 5 logins/min/IP, 30/hr/account — Better-Auth-equivalent.
            Self::Login => (5, 30),
            // 3 sends/min/IP, 10/hr/email — protects SMS/email spend.
            Self::Send => (3, 10),
            // 30/min/IP — generous because legitimate flows can retry.
            Self::Verify => (30, 100),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateLimitDecision {
    Allow,
    /// Caller exceeded the cap. `retry_after_secs` is a hint for the
    /// 429 `Retry-After` header.
    Deny {
        retry_after_secs: u64,
    },
}

/// Token-bucket counter — for each `(bucket, key)` we track the
/// epoch-second window start + count. When the window rolls over,
/// the count resets. Cheap O(1) per check.
#[derive(Debug, Clone, Copy)]
struct Counter {
    window_start: u64,
    count: u32,
}

pub struct AuthRateLimiter {
    per_ip: Mutex<HashMap<(AuthBucket, String), Counter>>,
    per_account: Mutex<HashMap<(AuthBucket, String), Counter>>,
}

impl Default for AuthRateLimiter {
    fn default() -> Self {
        Self {
            per_ip: Mutex::new(HashMap::new()),
            per_account: Mutex::new(HashMap::new()),
        }
    }
}

impl AuthRateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Process-wide singleton. Auth endpoints use this so per-IP
    /// counters survive across requests without plumbing a store
    /// through every call site.
    pub fn shared() -> &'static AuthRateLimiter {
        static CELL: std::sync::OnceLock<AuthRateLimiter> = std::sync::OnceLock::new();
        CELL.get_or_init(AuthRateLimiter::default)
    }

    /// Check + bump. `account_key` is the email/user_id/phone — pass
    /// `None` for endpoints with no pre-auth account binding (e.g.
    /// passkey/login/begin).
    pub fn check(
        &self,
        bucket: AuthBucket,
        ip: &str,
        account_key: Option<&str>,
    ) -> RateLimitDecision {
        let (ip_cap, acct_cap) = bucket.caps();
        let now = now_secs();
        // 1-minute window for IP, 1-hour window for account.
        if let Some(retry) = bump(&self.per_ip, (bucket, ip.to_string()), 60, ip_cap, now) {
            return RateLimitDecision::Deny {
                retry_after_secs: retry,
            };
        }
        if let Some(key) = account_key {
            if let Some(retry) = bump(
                &self.per_account,
                (bucket, key.to_ascii_lowercase()),
                3600,
                acct_cap,
                now,
            ) {
                return RateLimitDecision::Deny {
                    retry_after_secs: retry,
                };
            }
        }
        RateLimitDecision::Allow
    }
}

fn bump(
    map: &Mutex<HashMap<(AuthBucket, String), Counter>>,
    key: (AuthBucket, String),
    window_secs: u64,
    cap: u32,
    now: u64,
) -> Option<u64> {
    let mut g = map.lock().unwrap();
    let entry = g.entry(key).or_insert(Counter {
        window_start: now,
        count: 0,
    });
    if now >= entry.window_start + window_secs {
        entry.window_start = now;
        entry.count = 0;
    }
    if entry.count >= cap {
        return Some(entry.window_start + window_secs - now);
    }
    entry.count += 1;
    None
}

fn now_secs() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_within_cap() {
        let rl = AuthRateLimiter::new();
        for _ in 0..5 {
            assert_eq!(
                rl.check(AuthBucket::Login, "1.2.3.4", Some("a@b.com")),
                RateLimitDecision::Allow
            );
        }
    }

    #[test]
    fn denies_after_per_ip_cap() {
        let rl = AuthRateLimiter::new();
        let bucket = AuthBucket::Login;
        let (ip_cap, _) = bucket.caps();
        for _ in 0..ip_cap {
            assert_eq!(rl.check(bucket, "1.2.3.4", None), RateLimitDecision::Allow);
        }
        match rl.check(bucket, "1.2.3.4", None) {
            RateLimitDecision::Deny { retry_after_secs } => assert!(retry_after_secs <= 60),
            _ => panic!("expected Deny"),
        }
    }

    #[test]
    fn per_account_cap_independent_of_ip() {
        let rl = AuthRateLimiter::new();
        let bucket = AuthBucket::Send;
        let (_, acct_cap) = bucket.caps();
        // Rotate IPs to exhaust per-account before per-IP.
        for i in 0..acct_cap {
            let ip = format!("10.0.0.{i}");
            assert_eq!(
                rl.check(bucket, &ip, Some("victim@x.com")),
                RateLimitDecision::Allow
            );
        }
        let result = rl.check(bucket, "10.0.0.99", Some("victim@x.com"));
        assert!(matches!(result, RateLimitDecision::Deny { .. }));
    }

    #[test]
    fn account_key_lowercased() {
        let rl = AuthRateLimiter::new();
        let bucket = AuthBucket::Send;
        let (_, acct_cap) = bucket.caps();
        // Rotate IPs so we exhaust the per-account counter before
        // any single IP hits its own per-minute cap.
        for i in 0..acct_cap {
            let ip = format!("10.0.0.{i}");
            let _ = rl.check(bucket, &ip, Some("a@b.com"));
        }
        // Capitalized variant of the same email must hit the same
        // (now-exhausted) per-account bucket from a fresh IP.
        let result = rl.check(bucket, "172.16.0.1", Some("A@B.COM"));
        assert!(matches!(result, RateLimitDecision::Deny { .. }));
    }
}
