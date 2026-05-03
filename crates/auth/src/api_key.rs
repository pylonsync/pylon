//! API keys — long-lived bearer tokens for service-to-service or
//! mobile clients that don't fit the cookie-session model.
//!
//! Wire format: `pk_<32-char-base64url>` so they're trivially
//! distinguishable from session tokens (`pylon_…`) at a glance and
//! in log greps. Verification stores the **hash** of the secret —
//! the plaintext is shown to the user exactly once at create time
//! and never again, same pattern as Stripe / GitHub PATs.
//!
//! Key trust model:
//! - Each key belongs to one user (`user_id`).
//! - Optional `name` for the user to identify it ("CI", "iOS app").
//! - Optional `scopes` — comma-separated strings the application
//!   layer interprets. Pylon doesn't enforce them; the host app's
//!   policies do.
//! - Optional `expires_at` — when set, requests with the key are
//!   rejected after this Unix timestamp. `None` means no expiry
//!   (set + forget for trusted CI machines).
//! - Optional `last_used_at` — refreshed on every successful auth
//!   so the user can prune stale keys from a "remove unused for
//!   90 days" sweep.
//!
//! Storage is pluggable via [`ApiKeyBackend`] — the runtime swaps
//! in SQLite/Postgres backends behind the scenes; the in-memory
//! default is fine for tests + ephemeral dev servers.

use std::collections::HashMap;
use std::sync::Mutex;

/// One stored API key. The `secret_hash` is what's persisted; the
/// plaintext secret is returned to the caller exactly once at create
/// time (see [`ApiKeyStore::create`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiKey {
    /// Stable identifier — what the dashboard / management UI lists.
    /// Format: `key_<24-char-base64url>`. Distinct from `prefix` so a
    /// user can revoke by id without seeing the secret prefix.
    pub id: String,
    /// User who owns this key. Auth context resolves to this user_id
    /// when the key authenticates.
    pub user_id: String,
    /// Friendly name set by the owner. Free-form; UI-only.
    pub name: String,
    /// First 16 chars of the FULL plaintext token (`pk.key_<8 id chars>`).
    /// Safe to display in management UIs since this prefix encodes
    /// only the key id, not any of the secret material — the secret
    /// starts AFTER the second `.` separator. Lets the user
    /// distinguish keys by sight without ever exposing the secret.
    pub prefix: String,
    /// HMAC-SHA256 hash of the secret using a server-side pepper
    /// (`PYLON_API_KEY_PEPPER`, or a fixed dev pepper when unset).
    /// Verified at request time via constant-time compare.
    ///
    /// **Why HMAC-SHA256, not Argon2?** Argon2 exists to slow brute
    /// force of LOW-entropy passwords. API key secrets are 32 random
    /// bytes (256 bits) — brute force is computationally infeasible
    /// regardless of hash speed. Using Argon2 here would add ~50ms
    /// of latency per request for zero security benefit. SHA-256
    /// HMAC at ~1µs gives the same effective security plus 50000×
    /// throughput.
    pub secret_hash: String,
    /// Comma-separated scope strings. Application-defined; pylon
    /// stores opaquely.
    pub scopes: Option<String>,
    /// Unix timestamp at which this key stops being valid. None for
    /// no-expiry keys.
    pub expires_at: Option<u64>,
    /// Unix timestamp of the most recent successful auth — refreshed
    /// on every verify. None until the first use.
    pub last_used_at: Option<u64>,
    pub created_at: u64,
}

/// Storage backend for API keys. Same pluggable pattern as sessions
/// + magic codes — in-memory default, runtime injects SQLite/Postgres.
pub trait ApiKeyBackend: Send + Sync {
    fn put(&self, key: &ApiKey);
    fn get(&self, id: &str) -> Option<ApiKey>;
    fn delete(&self, id: &str) -> bool;
    /// All keys for a given user, used by management endpoints.
    fn list_for_user(&self, user_id: &str) -> Vec<ApiKey>;
    /// Update `last_used_at`. Called on every successful auth — must
    /// be cheap. Implementations are free to debounce (write at most
    /// once per minute, etc.) but the in-memory default writes
    /// straight through.
    fn touch(&self, id: &str, now: u64);
}

pub struct InMemoryApiKeyBackend {
    keys: Mutex<HashMap<String, ApiKey>>,
}

impl InMemoryApiKeyBackend {
    pub fn new() -> Self {
        Self {
            keys: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryApiKeyBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl ApiKeyBackend for InMemoryApiKeyBackend {
    fn put(&self, key: &ApiKey) {
        self.keys
            .lock()
            .unwrap()
            .insert(key.id.clone(), key.clone());
    }
    fn get(&self, id: &str) -> Option<ApiKey> {
        self.keys.lock().unwrap().get(id).cloned()
    }
    fn delete(&self, id: &str) -> bool {
        self.keys.lock().unwrap().remove(id).is_some()
    }
    fn list_for_user(&self, user_id: &str) -> Vec<ApiKey> {
        self.keys
            .lock()
            .unwrap()
            .values()
            .filter(|k| k.user_id == user_id)
            .cloned()
            .collect()
    }
    fn touch(&self, id: &str, now: u64) {
        if let Some(k) = self.keys.lock().unwrap().get_mut(id) {
            k.last_used_at = Some(now);
        }
    }
}

pub struct ApiKeyStore {
    backend: Box<dyn ApiKeyBackend>,
}

impl Default for ApiKeyStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Verification result — carries the matched key so the caller can
/// inspect scopes / expiry without a second backend round-trip.
#[derive(Debug, Clone)]
pub enum ApiKeyVerifyError {
    /// Token format is wrong (no `pk_` prefix or wrong length).
    Malformed,
    /// Token format is OK but the embedded id isn't in the store.
    NotFound,
    /// Token + id matched a stored key but the secret didn't verify.
    BadSecret,
    /// `expires_at` has passed.
    Expired,
}

impl std::fmt::Display for ApiKeyVerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed => f.write_str("API key is malformed"),
            Self::NotFound => f.write_str("API key not found"),
            Self::BadSecret => f.write_str("API key secret mismatch"),
            Self::Expired => f.write_str("API key has expired"),
        }
    }
}

impl ApiKeyStore {
    pub fn new() -> Self {
        Self::with_backend(Box::new(InMemoryApiKeyBackend::new()))
    }
    pub fn with_backend(backend: Box<dyn ApiKeyBackend>) -> Self {
        Self { backend }
    }

    /// Mint a new API key. Returns `(plaintext, ApiKey)` — the
    /// plaintext MUST be shown to the user exactly once and never
    /// stored anywhere on the server. The `ApiKey` is what's
    /// persisted (with `secret_hash` not the secret).
    ///
    /// Wire format: `pk.<id>.<secret>` — the id is embedded so
    /// verification is one DB lookup, not a table scan. Hash-only
    /// schemes that store no plaintext id make verification O(N).
    /// `.` separator (not `_`) so it survives the URL-safe base64
    /// alphabet that base64url uses for both id and secret bodies.
    pub fn create(
        &self,
        user_id: String,
        name: String,
        scopes: Option<String>,
        expires_at: Option<u64>,
    ) -> (String, ApiKey) {
        let id = format!("key_{}", random_token(24));
        let secret = random_token(32);
        let plaintext = format!("pk.{id}.{secret}");
        let prefix: String = plaintext.chars().take(16).collect();
        let key = ApiKey {
            id: id.clone(),
            user_id,
            name,
            prefix,
            secret_hash: hash_secret(&secret),
            scopes,
            expires_at,
            last_used_at: None,
            created_at: now_secs(),
        };
        self.backend.put(&key);
        (plaintext, key)
    }

    /// Verify a plaintext token. Touches `last_used_at` on success
    /// so the management UI can show "last used 5m ago".
    ///
    /// `touch` is debounced to once-per-minute per key to avoid a
    /// write storm on hot keys (one DB write per request was a real
    /// contention source under load).
    pub fn verify(&self, token: &str) -> Result<ApiKey, ApiKeyVerifyError> {
        let (id, secret) = parse_token(token).ok_or(ApiKeyVerifyError::Malformed)?;
        let key = self.backend.get(&id).ok_or(ApiKeyVerifyError::NotFound)?;
        if let Some(exp) = key.expires_at {
            if exp <= now_secs() {
                return Err(ApiKeyVerifyError::Expired);
            }
        }
        let expected = hash_secret(&secret);
        if !crate::constant_time_eq(expected.as_bytes(), key.secret_hash.as_bytes()) {
            return Err(ApiKeyVerifyError::BadSecret);
        }
        // Debounced last_used_at update — no point persisting a
        // touch within 60s of the previous one.
        let now = now_secs();
        if key.last_used_at.map(|t| now - t > 60).unwrap_or(true) {
            self.backend.touch(&key.id, now);
        }
        Ok(key)
    }

    pub fn revoke(&self, id: &str) -> bool {
        self.backend.delete(id)
    }

    pub fn list_for_user(&self, user_id: &str) -> Vec<ApiKey> {
        self.backend.list_for_user(user_id)
    }
}

/// Split `pk.<id>.<secret>` into `(id, secret)`. Returns `None` if
/// the format doesn't match exactly. `.` separator survives the
/// base64url alphabet — `_` and `-` are valid base64url chars and
/// would create false split points.
///
/// Tightened (codex Wave-2 P3):
///   - rejects extra `.` segments (`pk.id.secret.junk`)
///   - rejects non-base64url chars in id or secret
///   - rejects mismatched lengths (id is `key_` + 32 chars, secret is 43 chars)
fn parse_token(token: &str) -> Option<(String, String)> {
    let rest = token.strip_prefix("pk.")?;
    // Exactly two `.`-separated segments after the `pk.` header.
    let mut parts = rest.split('.');
    let id_part = parts.next()?;
    let secret = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    if !id_part.starts_with("key_") {
        return None;
    }
    let id_body = &id_part[4..]; // strip "key_"
                                 // 24 random bytes → base64url-no-pad → 32 chars.
                                 // 32 random bytes → base64url-no-pad → 43 chars.
    if id_body.len() != 32 || secret.len() != 43 {
        return None;
    }
    if !is_base64url(id_body) || !is_base64url(secret) {
        return None;
    }
    Some((id_part.to_string(), secret.to_string()))
}

/// Base64url alphabet check — `[A-Za-z0-9_-]` per RFC 4648 §5.
fn is_base64url(s: &str) -> bool {
    s.bytes().all(|b| {
        matches!(b,
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_')
    })
}

fn random_token(n_bytes: usize) -> String {
    use rand::RngCore;
    let mut bytes = vec![0u8; n_bytes];
    rand::thread_rng().fill_bytes(&mut bytes);
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    URL_SAFE_NO_PAD.encode(bytes)
}

/// HMAC-SHA256 the secret with a server-side pepper. Returns hex.
/// The pepper is read from `PYLON_API_KEY_PEPPER` (set this in
/// production — apps that don't risk the pepper being a known
/// constant). For dev convenience an unset pepper yields a fixed
/// dev value so testing works without env setup.
fn hash_secret(secret: &str) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;
    // OnceLock would be nicer but std env::var per call is fine:
    // we already trade-off env reads vs cache complexity elsewhere.
    let pepper = std::env::var("PYLON_API_KEY_PEPPER")
        .unwrap_or_else(|_| "pylon-dev-api-key-pepper-not-for-production".into());
    let mut mac =
        HmacSha256::new_from_slice(pepper.as_bytes()).expect("HMAC accepts any key length");
    mac.update(secret.as_bytes());
    let out = mac.finalize().into_bytes();
    use std::fmt::Write;
    let mut s = String::with_capacity(64);
    for b in out {
        let _ = write!(s, "{b:02x}");
    }
    s
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
    fn create_and_verify_roundtrip() {
        let store = ApiKeyStore::new();
        let (plaintext, key) = store.create(
            "user_1".into(),
            "test".into(),
            Some("read,write".into()),
            None,
        );
        assert!(plaintext.starts_with("pk.key_"));
        let verified = store.verify(&plaintext).expect("verify");
        assert_eq!(verified.id, key.id);
        assert_eq!(verified.user_id, "user_1");
        assert_eq!(verified.scopes.as_deref(), Some("read,write"));
    }

    #[test]
    fn malformed_token_rejected() {
        let store = ApiKeyStore::new();
        let err = store.verify("not_a_real_key").unwrap_err();
        assert!(matches!(err, ApiKeyVerifyError::Malformed));
    }

    #[test]
    fn unknown_id_returns_not_found() {
        let store = ApiKeyStore::new();
        // Well-formed token shape but unknown id → NotFound, not Malformed.
        let token = format!("pk.key_{}.{}", "z".repeat(32), "y".repeat(43));
        let err = store.verify(&token).unwrap_err();
        assert!(matches!(err, ApiKeyVerifyError::NotFound), "got: {err}");
    }

    #[test]
    fn wrong_secret_rejected() {
        let store = ApiKeyStore::new();
        let (plaintext, key) = store.create("u".into(), "n".into(), None, None);
        let mut bad = plaintext;
        bad.pop();
        bad.push('X');
        let err = store.verify(&bad).unwrap_err();
        assert!(matches!(err, ApiKeyVerifyError::BadSecret), "got: {err}");
        // The id should still resolve, so the error path is BadSecret
        // not NotFound — confirms we don't accidentally truncate the id.
        let _ = key.id;
    }

    #[test]
    fn expired_key_rejected() {
        let store = ApiKeyStore::new();
        let (plaintext, _) = store.create("u".into(), "n".into(), None, Some(now_secs() - 1));
        let err = store.verify(&plaintext).unwrap_err();
        assert!(matches!(err, ApiKeyVerifyError::Expired));
    }

    #[test]
    fn revoke_removes_key() {
        let store = ApiKeyStore::new();
        let (plaintext, key) = store.create("u".into(), "n".into(), None, None);
        assert!(store.revoke(&key.id));
        let err = store.verify(&plaintext).unwrap_err();
        assert!(matches!(err, ApiKeyVerifyError::NotFound));
    }

    #[test]
    fn touch_updates_last_used_at() {
        let store = ApiKeyStore::new();
        let (plaintext, key) = store.create("u".into(), "n".into(), None, None);
        assert!(key.last_used_at.is_none());
        let _ = store.verify(&plaintext);
        let after = store.list_for_user("u")[0].clone();
        assert!(after.last_used_at.is_some(), "touch should refresh");
    }

    #[test]
    fn list_for_user_only_returns_owned() {
        let store = ApiKeyStore::new();
        let _ = store.create("alice".into(), "k1".into(), None, None);
        let _ = store.create("alice".into(), "k2".into(), None, None);
        let _ = store.create("bob".into(), "k3".into(), None, None);
        assert_eq!(store.list_for_user("alice").len(), 2);
        assert_eq!(store.list_for_user("bob").len(), 1);
    }

    #[test]
    fn parse_token_accepts_well_formed() {
        // Real-shape token: id is "key_" + 32 base64url chars,
        // secret is 43 base64url chars.
        let id_body = "a".repeat(32);
        let secret = "b".repeat(43);
        let token = format!("pk.key_{id_body}.{secret}");
        let parsed = parse_token(&token).unwrap();
        assert_eq!(parsed.0, format!("key_{id_body}"));
        assert_eq!(parsed.1, secret);
    }

    #[test]
    fn parse_token_rejects_malformed() {
        // empty parts
        assert!(parse_token("pk.key_abc.").is_none());
        assert!(parse_token("pk.key_abc").is_none());
        // missing key_ prefix
        assert!(parse_token(&format!("pk.abc.{}", "b".repeat(43))).is_none());
        // wrong outer prefix
        assert!(parse_token(&format!("xy.key_{}.{}", "a".repeat(32), "b".repeat(43))).is_none());
        // wrong id length
        assert!(parse_token(&format!("pk.key_{}.{}", "a".repeat(31), "b".repeat(43))).is_none());
        // wrong secret length
        assert!(parse_token(&format!("pk.key_{}.{}", "a".repeat(32), "b".repeat(42))).is_none());
        // non-base64url chars
        assert!(parse_token(&format!("pk.key_{}.{}", "@".repeat(32), "b".repeat(43))).is_none());
        // extra dots / segments (codex P3)
        assert!(parse_token(&format!(
            "pk.key_{}.{}.junk",
            "a".repeat(32),
            "b".repeat(43)
        ))
        .is_none());
    }

    /// Regression: id and secret are base64url which contains `_` and
    /// `-`. Previous wire format used `_` as separator, which split
    /// the id at the wrong place when it contained an underscore.
    /// `.` separator avoids that class of bug.
    #[test]
    fn random_keys_with_underscores_round_trip() {
        let store = ApiKeyStore::new();
        // Run a handful of times to defeat lucky-RNG flakes.
        for _ in 0..20 {
            let (plaintext, key) = store.create("u".into(), "n".into(), None, None);
            let verified = store
                .verify(&plaintext)
                .expect("base64url body must verify");
            assert_eq!(verified.id, key.id);
        }
    }
}
