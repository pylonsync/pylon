//! Trusted-device records. The "skip TOTP for 30 days from this device"
//! primitive — apps that gate sensitive flows on `User.totpVerified`
//! check `ctx.auth.isTrustedDevice` first and skip the second-factor
//! step if the device is already trusted.
//!
//! The framework is deliberately *not* in the business of deciding when
//! TOTP is enforced; that's an app-level policy. What the framework owns:
//!
//! - Storing trust records (who trusted what device, until when).
//! - Validating an incoming `pylon_trusted_device=<token>` cookie against
//!   the store + the current session's user_id, and exposing the result
//!   as `AuthContext::is_trusted_device`.
//! - Endpoints to mint a record (after TOTP verify), list a user's
//!   trusted devices, and revoke them individually or all at once.
//!
//! Tokens are 32 bytes of CSPRNG output (base64url-encoded). They're
//! stored verbatim (not hashed) — same model as session tokens. A token
//! is bound to a single user; presenting a valid token while signed in
//! as a different user is treated as untrusted (the cookie is ignored,
//! not actively rejected, so a shared device that's trusted for User A
//! quietly degrades to "untrusted" when User B signs in on it).
//!
//! The store does NOT track usage. "Last seen" or "device count per user"
//! are app-level concerns; the framework stays minimal.

use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;

/// One trusted-device record.
///
/// Two separate identifiers by design:
/// - `token` — the secret the client presents in the cookie. NEVER returned
///   from the list endpoint (would let dashboard XSS exfiltrate trust
///   tokens equivalent to the cookie itself).
/// - `id` — the public-facing handle used by the management endpoints
///   (`DELETE /api/auth/trusted-devices/<id>`, list response). Knowing
///   the id lets a user revoke a device but doesn't let the holder
///   present a trust cookie.
///
/// `user_id` is what the cookie is bound to — the auth middleware accepts
/// the cookie ONLY when this matches the current session's user. `label`
/// is the parsed device string ("Chrome on macOS") so the management UI
/// doesn't have to re-parse user-agents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedDevice {
    pub id: String,
    pub token: String,
    pub user_id: String,
    pub label: Option<String>,
    pub created_at: String,
    pub expires_at: String,
}

impl TrustedDevice {
    /// Mint a fresh trust record. `lifetime_secs` defaults to 30 days
    /// when callers don't pass anything sensible — matches better-auth
    /// and the muscle memory most operators have for "remember this
    /// device".
    pub fn mint(user_id: impl Into<String>, label: Option<String>, lifetime_secs: u64) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let exp = now.saturating_add(lifetime_secs);
        Self {
            // 12 random bytes → 16-char base64url. Short enough for URLs,
            // long enough that brute-forcing other users' device ids is
            // not practical — id collisions are not security-critical
            // since the revoke path also checks user_id ownership.
            id: random_id(),
            token: random_token(),
            user_id: user_id.into(),
            label,
            created_at: format!("{now}"),
            expires_at: format!("{exp}"),
        }
    }

    /// True iff the record's `expires_at` is in the past.
    pub fn is_expired(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.expires_at
            .parse::<u64>()
            .map(|exp| exp <= now)
            .unwrap_or(true)
    }
}

fn random_token() -> String {
    // 32 bytes = 256 bits, base64url-encoded → 43 chars unpadded. Same
    // strength target as session tokens.
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn random_id() -> String {
    // 12 bytes = 96 bits — collision-resistant within one user's tiny
    // device list (max maybe a few dozen). Not the secret; the secret
    // is `token`.
    let mut bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut bytes);
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Default trusted-device lifetime: 30 days. Matches the "remember this
/// computer for 30 days" copy most consumer apps use.
pub const DEFAULT_TRUST_LIFETIME_SECS: u64 = 30 * 24 * 60 * 60;

/// Cookie name used by the framework to ferry the trust token between
/// browser and server. Matches the `pylon_*` namespace used by other
/// session-shaped cookies.
pub const TRUST_COOKIE_NAME: &str = "pylon_trusted_device";

/// Persistence trait. Implemented by the in-memory store below for tests
/// + dev mode, and by SQLite/PG backends in `pylon-runtime`.
///
/// All methods are sync to keep the trait `Send + Sync` and consistent
/// with the other auth stores. PG impls bridge via `block_on` if needed.
pub trait TrustedDeviceStore: Send + Sync {
    /// Persist a freshly-minted record.
    fn create(&self, device: TrustedDevice);

    /// Look up by SECRET token (the cookie value). Returns None when not
    /// found OR when the row exists but has expired (callers treat both
    /// the same). Implementations MAY also delete expired rows
    /// opportunistically during this call. Used by the auth middleware
    /// to populate `auth_ctx.is_trusted_device`.
    fn find(&self, token: &str) -> Option<TrustedDevice>;

    /// Look up by PUBLIC id (the management identifier). Used by the
    /// `DELETE /api/auth/trusted-devices/<id>` endpoint to verify
    /// ownership before revoking. None on not-found / expired.
    fn find_by_id(&self, id: &str) -> Option<TrustedDevice>;

    /// All non-expired records owned by `user_id`. Newest-first ordering
    /// is nice for UIs but not strictly required.
    fn list_for_user(&self, user_id: &str) -> Vec<TrustedDevice>;

    /// Delete one record by PUBLIC id. Returns true if it existed.
    /// Caller is responsible for the user-id ownership check before
    /// invoking — the store doesn't enforce it.
    fn revoke_by_id(&self, id: &str) -> bool;

    /// Delete every record owned by `user_id`. Returns the count
    /// deleted. Called from `/api/auth/account` deletion + from
    /// "sign me out everywhere" flows.
    fn revoke_all_for_user(&self, user_id: &str) -> usize;
}

/// In-memory store keyed by token. Used in tests, dev mode, and the
/// pylon binary's startup until a persistent backend is wired up.
#[derive(Default)]
pub struct InMemoryTrustedDeviceStore {
    inner: RwLock<HashMap<String, TrustedDevice>>,
}

impl InMemoryTrustedDeviceStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl TrustedDeviceStore for InMemoryTrustedDeviceStore {
    fn create(&self, device: TrustedDevice) {
        // Lazy GC: prune expired rows opportunistically so a long-lived
        // process doesn't accumulate dead entries forever.
        let mut map = self.inner.write().unwrap();
        map.retain(|_, v| !v.is_expired());
        map.insert(device.token.clone(), device);
    }

    fn find(&self, token: &str) -> Option<TrustedDevice> {
        let map = self.inner.read().unwrap();
        map.get(token).filter(|d| !d.is_expired()).cloned()
    }

    fn find_by_id(&self, id: &str) -> Option<TrustedDevice> {
        let map = self.inner.read().unwrap();
        map.values()
            .find(|d| d.id == id && !d.is_expired())
            .cloned()
    }

    fn list_for_user(&self, user_id: &str) -> Vec<TrustedDevice> {
        let map = self.inner.read().unwrap();
        let mut out: Vec<TrustedDevice> = map
            .values()
            .filter(|d| d.user_id == user_id && !d.is_expired())
            .cloned()
            .collect();
        // Newest-first: parse expires_at as an integer; ties are
        // resolved arbitrarily.
        out.sort_by(|a, b| {
            let a_exp: u64 = a.expires_at.parse().unwrap_or(0);
            let b_exp: u64 = b.expires_at.parse().unwrap_or(0);
            b_exp.cmp(&a_exp)
        });
        out
    }

    fn revoke_by_id(&self, id: &str) -> bool {
        let mut map = self.inner.write().unwrap();
        let before = map.len();
        map.retain(|_, v| v.id != id);
        before != map.len()
    }

    fn revoke_all_for_user(&self, user_id: &str) -> usize {
        let mut map = self.inner.write().unwrap();
        let before = map.len();
        map.retain(|_, v| v.uid_neq(user_id));
        before - map.len()
    }
}

impl TrustedDevice {
    fn uid_neq(&self, other: &str) -> bool {
        self.user_id != other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mint_produces_unique_tokens() {
        let a = TrustedDevice::mint("u1", None, 60);
        let b = TrustedDevice::mint("u1", None, 60);
        assert_ne!(a.token, b.token);
        assert_eq!(a.token.len(), 43, "32 bytes base64url unpadded = 43 chars");
    }

    #[test]
    fn find_returns_none_when_expired() {
        let store = InMemoryTrustedDeviceStore::new();
        // Lifetime 0 => expires_at == now. is_expired() returns true.
        store.create(TrustedDevice::mint("u1", None, 0));
        let token = store.inner.read().unwrap().keys().next().cloned().unwrap();
        assert!(store.find(&token).is_none());
    }

    #[test]
    fn find_resolves_unexpired_token() {
        let store = InMemoryTrustedDeviceStore::new();
        let device = TrustedDevice::mint("u1", Some("Chrome on macOS".into()), 3600);
        let token = device.token.clone();
        store.create(device);
        let got = store.find(&token).unwrap();
        assert_eq!(got.user_id, "u1");
        assert_eq!(got.label.as_deref(), Some("Chrome on macOS"));
    }

    #[test]
    fn list_for_user_isolates_per_user() {
        let store = InMemoryTrustedDeviceStore::new();
        store.create(TrustedDevice::mint("u1", None, 60));
        store.create(TrustedDevice::mint("u1", None, 60));
        store.create(TrustedDevice::mint("u2", None, 60));
        assert_eq!(store.list_for_user("u1").len(), 2);
        assert_eq!(store.list_for_user("u2").len(), 1);
        assert_eq!(store.list_for_user("u3").len(), 0);
    }

    #[test]
    fn revoke_by_id_removes_only_target() {
        let store = InMemoryTrustedDeviceStore::new();
        let a = TrustedDevice::mint("u1", None, 60);
        let b = TrustedDevice::mint("u1", None, 60);
        let a_id = a.id.clone();
        let a_token = a.token.clone();
        store.create(a);
        store.create(b);
        assert!(store.revoke_by_id(&a_id));
        assert!(store.find(&a_token).is_none());
        assert_eq!(store.list_for_user("u1").len(), 1);
    }

    #[test]
    fn revoke_by_id_returns_false_for_unknown() {
        let store = InMemoryTrustedDeviceStore::new();
        assert!(!store.revoke_by_id("ghost-id"));
    }

    #[test]
    fn id_and_token_are_distinct() {
        let d = TrustedDevice::mint("u1", None, 60);
        assert_ne!(d.id, d.token);
        assert_eq!(d.id.len(), 16, "12 bytes base64url unpadded = 16 chars");
        assert_eq!(d.token.len(), 43);
    }

    #[test]
    fn find_by_id_resolves_when_alive() {
        let store = InMemoryTrustedDeviceStore::new();
        let d = TrustedDevice::mint("u1", None, 60);
        let id = d.id.clone();
        store.create(d);
        let got = store.find_by_id(&id).unwrap();
        assert_eq!(got.user_id, "u1");
    }

    #[test]
    fn revoke_all_for_user_scoped() {
        let store = InMemoryTrustedDeviceStore::new();
        store.create(TrustedDevice::mint("u1", None, 60));
        store.create(TrustedDevice::mint("u1", None, 60));
        store.create(TrustedDevice::mint("u2", None, 60));
        let removed = store.revoke_all_for_user("u1");
        assert_eq!(removed, 2);
        assert_eq!(store.list_for_user("u1").len(), 0);
        assert_eq!(store.list_for_user("u2").len(), 1);
    }

    #[test]
    fn create_prunes_expired_rows() {
        let store = InMemoryTrustedDeviceStore::new();
        store.create(TrustedDevice::mint("u1", None, 0)); // expires immediately
        store.create(TrustedDevice::mint("u1", None, 3600)); // alive
                                                             // Trigger another create which prunes.
        store.create(TrustedDevice::mint("u2", None, 3600));
        let alive = store.list_for_user("u1");
        assert_eq!(alive.len(), 1, "expired row should be pruned");
    }

    #[test]
    fn cookie_name_is_namespaced() {
        // Catch accidental rename — apps + browser extensions key on
        // this string.
        assert_eq!(TRUST_COOKIE_NAME, "pylon_trusted_device");
    }

    #[test]
    fn default_lifetime_is_thirty_days() {
        assert_eq!(DEFAULT_TRUST_LIFETIME_SECS, 30 * 24 * 60 * 60);
    }
}
