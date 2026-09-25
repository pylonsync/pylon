//! One-time codes that move a new session onto a platform (tenant) host.
//!
//! An app that serves its customers on their own domains (`ctx.domains`)
//! answers on hosts like `feedback.acme.com`, but an OAuth provider only
//! knows the app's one registered callback URL, on the app's own host. The
//! provider therefore returns the browser to the app's host, where a session
//! cookie would be useless: the browser keeps it for that host, not for
//! `feedback.acme.com`.
//!
//! The callback instead stores the signed-in user under a random one-time
//! code, bound to the tenant host, and redirects the browser to
//! `https://<tenant host>/api/auth/handoff?code=…`. That request redeems the
//! code, mints a session, and sets the cookie on the tenant host.
//!
//! Security properties:
//!   - The code is 32 random bytes. Only its SHA-256 is stored, so a read of
//!     the store cannot replay a pending handoff.
//!   - Single use: redeeming removes the record, whether or not the rest of
//!     the checks pass, so a leaked code is burned by its first use.
//!   - Short-lived: 120 seconds, enough for one redirect.
//!   - Host-bound: a code minted for `feedback.acme.com` is refused on any
//!     other host, so it cannot be replayed against a different tenant.
//!   - No session token is stored: the record holds the user id, and the
//!     session is minted at redemption, on the tenant host.
//!   - Browser-bound: the OAuth start, which runs on the tenant host, sets a
//!     random host-only cookie there and keeps its hash in the OAuth state.
//!     The handoff carries that hash and only redeems in a browser that
//!     presents the cookie, so a code sent to someone else cannot sign them
//!     in to the sender's account (login CSRF).

use std::collections::HashMap;
use std::sync::Mutex;

/// A pending handoff: who signed in, which host may redeem it, and where to
/// send the browser after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionHandoff {
    pub user_id: String,
    /// Lowercased host, no port. Must equal the redeeming request's host.
    pub target_host: String,
    /// Absolute URL on `target_host` to redirect to after redemption.
    pub redirect_url: String,
    /// SHA-256 (hex) of the browser-binding cookie set at the OAuth start.
    pub binding: Option<String>,
    pub expires_at: u64,
}

/// Storage for pending handoffs, keyed by the SHA-256 (hex) of the code. The
/// runtime supplies SQLite and Postgres backends so a handoff minted on one
/// machine can be redeemed on another.
pub trait SessionHandoffBackend: Send + Sync {
    fn put(&self, key: &str, handoff: &SessionHandoff);
    /// Atomic compare-and-consume: the record when it exists and has not
    /// expired, removing it in either case.
    fn take(&self, key: &str, now_unix_secs: u64) -> Option<SessionHandoff>;
}

/// In-memory backend: tests, dev, and single-process deployments.
#[derive(Default)]
pub struct InMemorySessionHandoffBackend {
    records: Mutex<HashMap<String, SessionHandoff>>,
}

impl InMemorySessionHandoffBackend {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SessionHandoffBackend for InMemorySessionHandoffBackend {
    fn put(&self, key: &str, handoff: &SessionHandoff) {
        self.records
            .lock()
            .unwrap()
            .insert(key.to_string(), handoff.clone());
    }

    fn take(&self, key: &str, now_unix_secs: u64) -> Option<SessionHandoff> {
        let record = self.records.lock().unwrap().remove(key)?;
        (record.expires_at > now_unix_secs).then_some(record)
    }
}

/// Why a redemption was refused. The code is consumed in every case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffError {
    /// Unknown, already used, or expired.
    Invalid,
    /// Minted for a different host than the one redeeming it.
    WrongHost,
    /// The browser does not hold the cookie the sign-in started with.
    OtherBrowser,
}

/// Name of the browser-binding cookie, next to the session cookie.
pub fn binding_cookie_name(session_cookie_name: &str) -> String {
    format!("{session_cookie_name}_handoff")
}

/// How long the binding cookie lives: the OAuth state's lifetime.
pub const BINDING_COOKIE_MAX_AGE_SECS: u64 = 600;

/// A new binding: the cookie value, and the hash to keep in the OAuth state.
pub fn new_binding() -> (String, String) {
    let value = new_code();
    let hash = key_for(&value);
    (value, hash)
}

pub struct SessionHandoffStore {
    backend: Box<dyn SessionHandoffBackend>,
}

impl Default for SessionHandoffStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionHandoffStore {
    /// How long a code stays redeemable.
    pub const TTL_SECS: u64 = 120;

    pub fn new() -> Self {
        Self::with_backend(Box::new(InMemorySessionHandoffBackend::new()))
    }

    pub fn with_backend(backend: Box<dyn SessionHandoffBackend>) -> Self {
        Self { backend }
    }

    /// Store a handoff for `user_id` to `target_host` and return the code to
    /// put in the redirect. The caller has checked that `target_host` is a
    /// trusted platform host and that `redirect_url` is on it.
    /// `binding` is the hash from the OAuth state (see [`new_binding`]).
    pub fn create(
        &self,
        user_id: &str,
        target_host: &str,
        redirect_url: &str,
        binding: Option<&str>,
    ) -> String {
        let code = new_code();
        self.backend.put(
            &key_for(&code),
            &SessionHandoff {
                user_id: user_id.to_string(),
                target_host: normalize_host(target_host),
                redirect_url: redirect_url.to_string(),
                binding: binding.map(str::to_string),
                expires_at: now() + Self::TTL_SECS,
            },
        );
        code
    }

    /// Redeem `code` on a request to `request_host` (the `Host` header),
    /// from a browser presenting `binding_cookie` (the binding cookie's
    /// value, if any).
    pub fn redeem(
        &self,
        code: &str,
        request_host: &str,
        binding_cookie: Option<&str>,
    ) -> Result<SessionHandoff, HandoffError> {
        let record = self
            .backend
            .take(&key_for(code), now())
            .ok_or(HandoffError::Invalid)?;
        if record.target_host != normalize_host(request_host) {
            return Err(HandoffError::WrongHost);
        }
        if let Some(expected) = &record.binding {
            let presented = binding_cookie.map(key_for).unwrap_or_default();
            if !constant_time_eq(presented.as_bytes(), expected.as_bytes()) {
                return Err(HandoffError::OtherBrowser);
            }
        }
        Ok(record)
    }
}

/// Lowercase, trailing dot and port removed. `Feedback.Acme.com:443` →
/// `feedback.acme.com`.
pub fn normalize_host(host: &str) -> String {
    let host = host.trim();
    let host = match host.rfind(':') {
        Some(i) if !host.ends_with(']') && !host[..i].contains(':') => &host[..i],
        _ => host,
    };
    host.trim_end_matches('.').to_ascii_lowercase()
}

fn new_code() -> String {
    use rand::Rng;
    let bytes: [u8; 32] = rand::thread_rng().gen();
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("hoff_{hex}")
}

fn key_for(code: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(code.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redeems_once_on_the_bound_host() {
        let store = SessionHandoffStore::new();
        let code = store.create(
            "u_1",
            "Feedback.Acme.com",
            "https://feedback.acme.com/p",
            None,
        );
        assert!(code.starts_with("hoff_"));
        let got = store.redeem(&code, "feedback.acme.com:443", None).unwrap();
        assert_eq!(got.user_id, "u_1");
        assert_eq!(got.target_host, "feedback.acme.com");
        assert_eq!(got.redirect_url, "https://feedback.acme.com/p");
        // Single use.
        assert_eq!(
            store.redeem(&code, "feedback.acme.com", None),
            Err(HandoffError::Invalid)
        );
    }

    #[test]
    fn a_bound_code_redeems_only_with_the_cookie() {
        let store = SessionHandoffStore::new();
        let (cookie, hash) = new_binding();
        let code = store.create("u_1", "a.com", "https://a.com/", Some(&hash));
        assert_eq!(
            store.redeem(&code, "a.com", None),
            Err(HandoffError::OtherBrowser)
        );
        // Burned by the failed attempt.
        assert_eq!(
            store.redeem(&code, "a.com", Some(&cookie)),
            Err(HandoffError::Invalid)
        );

        let code = store.create("u_1", "a.com", "https://a.com/", Some(&hash));
        assert_eq!(
            store.redeem(&code, "a.com", Some("hoff_someone_else")),
            Err(HandoffError::OtherBrowser)
        );
        let code = store.create("u_1", "a.com", "https://a.com/", Some(&hash));
        assert_eq!(
            store.redeem(&code, "a.com", Some(&cookie)).unwrap().user_id,
            "u_1"
        );
    }

    #[test]
    fn a_wrong_host_is_refused_and_burns_the_code() {
        let store = SessionHandoffStore::new();
        let code = store.create(
            "u_1",
            "feedback.acme.com",
            "https://feedback.acme.com/",
            None,
        );
        assert_eq!(
            store.redeem(&code, "feedback.other.com", None),
            Err(HandoffError::WrongHost)
        );
        assert_eq!(
            store.redeem(&code, "feedback.acme.com", None),
            Err(HandoffError::Invalid)
        );
    }

    #[test]
    fn unknown_and_expired_codes_are_invalid() {
        let backend = InMemorySessionHandoffBackend::new();
        backend.put(
            &key_for("hoff_old"),
            &SessionHandoff {
                user_id: "u".into(),
                target_host: "a.com".into(),
                redirect_url: "https://a.com/".into(),
                binding: None,
                expires_at: now() - 1,
            },
        );
        let store = SessionHandoffStore::with_backend(Box::new(backend));
        assert_eq!(
            store.redeem("hoff_old", "a.com", None),
            Err(HandoffError::Invalid)
        );
        assert_eq!(
            store.redeem("hoff_never", "a.com", None),
            Err(HandoffError::Invalid)
        );
    }

    #[test]
    fn only_the_hash_of_the_code_is_stored() {
        struct Spy(Mutex<Vec<String>>);
        impl SessionHandoffBackend for Spy {
            fn put(&self, key: &str, _h: &SessionHandoff) {
                self.0.lock().unwrap().push(key.to_string());
            }
            fn take(&self, _key: &str, _now: u64) -> Option<SessionHandoff> {
                None
            }
        }
        let spy = std::sync::Arc::new(Spy(Mutex::new(Vec::new())));
        struct Shared(std::sync::Arc<Spy>);
        impl SessionHandoffBackend for Shared {
            fn put(&self, key: &str, h: &SessionHandoff) {
                self.0.put(key, h)
            }
            fn take(&self, key: &str, now: u64) -> Option<SessionHandoff> {
                self.0.take(key, now)
            }
        }
        let store = SessionHandoffStore::with_backend(Box::new(Shared(spy.clone())));
        let code = store.create("u", "a.com", "https://a.com/", None);
        let keys = spy.0.lock().unwrap();
        assert_eq!(keys.len(), 1);
        assert_ne!(keys[0], code);
        assert_eq!(keys[0].len(), 64);
        assert!(!keys[0].contains("hoff_"));
    }

    #[test]
    fn normalize_host_strips_port_case_and_trailing_dot() {
        assert_eq!(normalize_host("Feedback.Acme.COM."), "feedback.acme.com");
        assert_eq!(normalize_host("localhost:4321"), "localhost");
        assert_eq!(normalize_host("[::1]"), "[::1]");
    }
}
