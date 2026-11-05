//! Access and refresh tokens issued by Pylon's OIDC provider
//! ([`crate::oidc_provider`]).
//!
//! Tokens are opaque 256-bit random strings. The store keeps only the
//! SHA-256 hash of each token, so a copy of the table cannot be replayed
//! against `/oidc/userinfo` or `/oidc/token`. The hash is unsalted: the
//! input is 256 bits from the OS CSPRNG, so there is nothing to brute force.
//!
//! Every token belongs to a grant: one sign-in of one user to one client.
//! A refresh issues new tokens under the same `grant_id`, and revocation
//! (RFC 7009, refresh-token theft, user deletion, sign-out everywhere)
//! removes the whole grant.
//!
//! The in-memory store here is for tests and dev runs. The SQLite and
//! Postgres stores live in `pylon-runtime` (`oidc_token_backend.rs`).

use std::collections::HashMap;
use std::sync::Mutex;

/// What a stored token is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OidcTokenKind {
    /// Bearer for `/oidc/userinfo` and `ctx.auth.verifyOidcToken`.
    Access,
    /// Presented at `/oidc/token` with `grant_type=refresh_token`.
    Refresh,
}

impl OidcTokenKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Access => "access",
            Self::Refresh => "refresh",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "access" => Some(Self::Access),
            "refresh" => Some(Self::Refresh),
            _ => None,
        }
    }
}

/// One stored token. Never carries the raw token, only its hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidcTokenRecord {
    /// Lowercase hex SHA-256 of the raw token. Primary key.
    pub token_hash: String,
    pub kind: OidcTokenKind,
    /// Shared by every token of one sign-in, across refreshes.
    pub grant_id: String,
    pub user_id: String,
    pub client_id: String,
    pub scopes: Vec<String>,
    /// Unix seconds.
    pub expires_at: u64,
    /// Unix seconds.
    pub created_at: u64,
    /// Refresh tokens only: when the token was first exchanged for new
    /// tokens. `None` until then. A rotated token stays in the store
    /// until it expires so a late reuse can be detected as theft.
    pub rotated_at: Option<u64>,
}

impl OidcTokenRecord {
    pub fn is_expired(&self, now: u64) -> bool {
        self.expires_at <= now
    }

    /// Scopes as stored: one space-separated string.
    pub fn scopes_joined(&self) -> String {
        self.scopes.join(" ")
    }
}

/// Split a stored space-separated scope string.
pub fn split_scopes(raw: &str) -> Vec<String> {
    raw.split_whitespace().map(String::from).collect()
}

/// Hash a raw token for storage and lookup.
pub fn hash_token(token: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Storage for OIDC tokens. Every method reports storage failures as
/// `Err` so the routes can answer `server_error` instead of treating an
/// outage as "unknown token" (which would make clients drop a good
/// refresh token).
pub trait OidcTokenStore: Send + Sync {
    /// Store a new token. A token whose insert fails must not be handed
    /// to the client.
    fn insert(&self, record: &OidcTokenRecord) -> Result<(), String>;

    /// Find a token by hash. Expired rows are deleted and reported as
    /// `None`.
    fn find(&self, token_hash: &str, now: u64) -> Result<Option<OidcTokenRecord>, String>;

    /// Set `rotated_at = now` on a refresh token that has not been rotated
    /// yet, in one atomic step. Returns the token's `rotated_at` after the
    /// call: `now` for the first caller, the earlier time for later ones,
    /// `None` when the row is gone.
    fn mark_rotated(&self, token_hash: &str, now: u64) -> Result<Option<u64>, String>;

    /// Delete every token of a grant. Returns how many rows were removed.
    fn revoke_grant(&self, grant_id: &str) -> Result<usize, String>;

    /// Delete every token of every grant of a user. Returns how many
    /// grants were revoked.
    fn revoke_user(&self, user_id: &str) -> Result<usize, String>;

    /// Delete expired rows. Returns how many were removed.
    fn sweep_expired(&self, now: u64) -> Result<usize, String>;
}

/// In-memory token store. Lost on restart; tests and dev only.
#[derive(Default)]
pub struct InMemoryOidcTokenStore {
    rows: Mutex<HashMap<String, OidcTokenRecord>>,
}

impl InMemoryOidcTokenStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, OidcTokenRecord>> {
        self.rows.lock().unwrap_or_else(|p| p.into_inner())
    }
}

impl OidcTokenStore for InMemoryOidcTokenStore {
    fn insert(&self, record: &OidcTokenRecord) -> Result<(), String> {
        self.lock()
            .insert(record.token_hash.clone(), record.clone());
        Ok(())
    }

    fn find(&self, token_hash: &str, now: u64) -> Result<Option<OidcTokenRecord>, String> {
        let mut rows = self.lock();
        match rows.get(token_hash) {
            Some(r) if r.is_expired(now) => {
                rows.remove(token_hash);
                Ok(None)
            }
            Some(r) => Ok(Some(r.clone())),
            None => Ok(None),
        }
    }

    fn mark_rotated(&self, token_hash: &str, now: u64) -> Result<Option<u64>, String> {
        let mut rows = self.lock();
        Ok(rows
            .get_mut(token_hash)
            .map(|r| *r.rotated_at.get_or_insert(now)))
    }

    fn revoke_grant(&self, grant_id: &str) -> Result<usize, String> {
        let mut rows = self.lock();
        let before = rows.len();
        rows.retain(|_, r| r.grant_id != grant_id);
        Ok(before - rows.len())
    }

    fn revoke_user(&self, user_id: &str) -> Result<usize, String> {
        let mut rows = self.lock();
        let grants: std::collections::HashSet<String> = rows
            .values()
            .filter(|r| r.user_id == user_id)
            .map(|r| r.grant_id.clone())
            .collect();
        rows.retain(|_, r| r.user_id != user_id);
        Ok(grants.len())
    }

    fn sweep_expired(&self, now: u64) -> Result<usize, String> {
        let mut rows = self.lock();
        let before = rows.len();
        rows.retain(|_, r| !r.is_expired(now));
        Ok(before - rows.len())
    }
}

/// Shared behavior tests every [`OidcTokenStore`] must pass. The SQLite
/// and Postgres stores in `pylon-runtime` run the same suite.
#[doc(hidden)]
pub mod conformance {
    use super::*;

    fn record(
        hash: &str,
        kind: OidcTokenKind,
        grant: &str,
        user: &str,
        exp: u64,
    ) -> OidcTokenRecord {
        OidcTokenRecord {
            token_hash: hash.into(),
            kind,
            grant_id: grant.into(),
            user_id: user.into(),
            client_id: "client-a".into(),
            scopes: vec!["openid".into(), "offline_access".into()],
            expires_at: exp,
            created_at: 100,
            rotated_at: None,
        }
    }

    /// Run every check against `store`. Hashes are prefixed with `tag` so
    /// a shared database (Postgres in CI) can run the suite more than once.
    pub fn run(store: &dyn OidcTokenStore, tag: &str) {
        let now = 1_000;
        let h = |s: &str| format!("{tag}-{s}");
        let g = |s: &str| format!("{tag}-grant-{s}");
        let u = |s: &str| format!("{tag}-user-{s}");

        // Round trip keeps every field.
        let access = record(&h("a1"), OidcTokenKind::Access, &g("1"), &u("1"), now + 60);
        store.insert(&access).unwrap();
        assert_eq!(store.find(&h("a1"), now).unwrap(), Some(access.clone()));
        assert_eq!(store.find(&h("missing"), now).unwrap(), None);

        // Expired rows read as missing and are deleted on lookup.
        let old = record(&h("old"), OidcTokenKind::Access, &g("1"), &u("1"), now);
        store.insert(&old).unwrap();
        assert_eq!(store.find(&h("old"), now).unwrap(), None);
        assert_eq!(
            store.find(&h("old"), 0).unwrap(),
            None,
            "lookup deleted the row"
        );

        // Rotation is set once; later calls see the first time.
        let refresh = record(
            &h("r1"),
            OidcTokenKind::Refresh,
            &g("1"),
            &u("1"),
            now + 600,
        );
        store.insert(&refresh).unwrap();
        assert_eq!(store.mark_rotated(&h("r1"), now).unwrap(), Some(now));
        assert_eq!(store.mark_rotated(&h("r1"), now + 30).unwrap(), Some(now));
        assert_eq!(
            store.find(&h("r1"), now).unwrap().unwrap().rotated_at,
            Some(now)
        );
        assert_eq!(store.mark_rotated(&h("missing"), now).unwrap(), None);

        // Revoking a grant removes its tokens and nothing else.
        let other = record(&h("a2"), OidcTokenKind::Access, &g("2"), &u("1"), now + 60);
        store.insert(&other).unwrap();
        assert_eq!(store.revoke_grant(&g("1")).unwrap(), 2);
        assert_eq!(store.find(&h("a1"), now).unwrap(), None);
        assert_eq!(store.find(&h("r1"), now).unwrap(), None);
        assert!(store.find(&h("a2"), now).unwrap().is_some());

        // Revoking a user counts grants and leaves other users alone.
        store
            .insert(&record(
                &h("a3"),
                OidcTokenKind::Access,
                &g("3"),
                &u("1"),
                now + 60,
            ))
            .unwrap();
        store
            .insert(&record(
                &h("r3"),
                OidcTokenKind::Refresh,
                &g("3"),
                &u("1"),
                now + 60,
            ))
            .unwrap();
        store
            .insert(&record(
                &h("a4"),
                OidcTokenKind::Access,
                &g("4"),
                &u("2"),
                now + 60,
            ))
            .unwrap();
        assert_eq!(store.revoke_user(&u("1")).unwrap(), 2);
        assert_eq!(store.find(&h("a2"), now).unwrap(), None);
        assert_eq!(store.find(&h("r3"), now).unwrap(), None);
        assert!(store.find(&h("a4"), now).unwrap().is_some());

        // Sweep removes expired rows only.
        store
            .insert(&record(
                &h("stale"),
                OidcTokenKind::Refresh,
                &g("5"),
                &u("2"),
                now - 1,
            ))
            .unwrap();
        assert!(store.sweep_expired(now).unwrap() >= 1);
        assert!(store.find(&h("a4"), now).unwrap().is_some());
        assert_eq!(store.find(&h("stale"), 0).unwrap(), None);
        store.revoke_user(&u("2")).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_store_passes_conformance() {
        conformance::run(&InMemoryOidcTokenStore::new(), "mem");
    }

    #[test]
    fn hash_is_hex_sha256_and_not_the_token() {
        let h = hash_token("abc");
        assert_eq!(
            h,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_ne!(hash_token("abd"), h);
    }

    #[test]
    fn kind_round_trips() {
        for k in [OidcTokenKind::Access, OidcTokenKind::Refresh] {
            assert_eq!(OidcTokenKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(OidcTokenKind::parse("code"), None);
    }
}
