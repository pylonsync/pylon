//! Verification tokens — single-use, email-delivered random tokens
//! that back password reset, email change, and magic-link sign-in.
//!
//! All three flows share the same shape: server mints a long random
//! token, hashes it, emails the plaintext to the user, then consumes
//! the token on the verify endpoint. Same backend pattern as
//! [`crate::api_key`]: HMAC-SHA256 with a server pepper (NOT Argon2 —
//! these are 32-byte random secrets, not low-entropy passwords).
//!
//! `kind` lets the verifier reject cross-purpose replay (a magic-link
//! token can't be used as a password-reset token even if an attacker
//! intercepts both emails).

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TokenKind {
    /// `/api/auth/password/reset/request` → `/complete`. `email`
    /// identifies the account; `user_id` is None at mint time
    /// (the user isn't logged in).
    PasswordReset,
    /// `/api/auth/email/change/request` → `/confirm`. `user_id` is
    /// the currently-logged-in user; `payload` carries the new
    /// email address; `email` is the new email (delivery target).
    EmailChange,
    /// `/api/auth/magic-link/send` → `/verify`. `email` identifies
    /// the account; `user_id` is None until verify creates/looks
    /// up the user.
    MagicLink,
}

impl TokenKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PasswordReset => "password_reset",
            Self::EmailChange => "email_change",
            Self::MagicLink => "magic_link",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationToken {
    /// Stable id for management UIs / audit logs (`vt_<24-base64url>`).
    pub id: String,
    pub kind: TokenKind,
    /// Email this token was minted for. Lowercased before storage.
    pub email: String,
    /// User id when known at mint time (email-change flow). None for
    /// password-reset / magic-link minted before any auth.
    pub user_id: Option<String>,
    /// Arbitrary opaque payload (e.g. email-change carries the
    /// proposed new email here so consume() can apply it without a
    /// second round-trip).
    pub payload: Option<String>,
    /// HMAC-SHA256 of the plaintext + server pepper (hex). Constant-
    /// time-compared at consume time.
    pub token_hash: String,
    /// First 8 chars of the plaintext for index narrowing — same
    /// trick as the org invite path.
    pub token_prefix: String,
    pub created_at: u64,
    pub expires_at: u64,
    /// Stamped on first successful consume so a replay returns the
    /// `AlreadyConsumed` error rather than `NotFound`.
    pub consumed_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationError {
    NotFound,
    Expired,
    AlreadyConsumed,
    /// Token was minted for a different `kind` — defends against an
    /// attacker tricking a victim into clicking a magic-link URL
    /// pointed at the password-reset endpoint.
    KindMismatch,
}

impl std::fmt::Display for VerificationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotFound => "verification token not found",
            Self::Expired => "verification token expired",
            Self::AlreadyConsumed => "verification token already consumed",
            Self::KindMismatch => "verification token is for a different flow",
        })
    }
}

pub trait VerificationBackend: Send + Sync {
    fn put(&self, token: &VerificationToken);
    fn get(&self, id: &str) -> Option<VerificationToken>;
    /// Lookup by `token_prefix` — backends index this column so
    /// `consume_by_plaintext` is one fast SQL hit per attempt.
    fn by_prefix(&self, prefix: &str) -> Vec<VerificationToken>;
    /// Mark as consumed. Implementations MUST be idempotent so
    /// concurrent verify requests can't both succeed.
    fn mark_consumed(&self, id: &str, now: u64) -> bool;
    /// Best-effort sweep of expired-and-consumed rows. Called
    /// opportunistically — never blocking the hot path.
    fn purge_expired(&self, now: u64);
}

pub struct InMemoryVerificationBackend {
    tokens: Mutex<HashMap<String, VerificationToken>>,
}

impl Default for InMemoryVerificationBackend {
    fn default() -> Self {
        Self {
            tokens: Mutex::new(HashMap::new()),
        }
    }
}

impl VerificationBackend for InMemoryVerificationBackend {
    fn put(&self, token: &VerificationToken) {
        self.tokens
            .lock()
            .unwrap()
            .insert(token.id.clone(), token.clone());
    }
    fn get(&self, id: &str) -> Option<VerificationToken> {
        self.tokens.lock().unwrap().get(id).cloned()
    }
    fn by_prefix(&self, prefix: &str) -> Vec<VerificationToken> {
        self.tokens
            .lock()
            .unwrap()
            .values()
            .filter(|t| t.token_prefix == prefix)
            .cloned()
            .collect()
    }
    fn mark_consumed(&self, id: &str, now: u64) -> bool {
        let mut map = self.tokens.lock().unwrap();
        let Some(t) = map.get_mut(id) else {
            return false;
        };
        if t.consumed_at.is_some() {
            return false;
        }
        t.consumed_at = Some(now);
        true
    }
    fn purge_expired(&self, now: u64) {
        let mut map = self.tokens.lock().unwrap();
        map.retain(|_, t| t.expires_at > now || t.consumed_at.is_none());
    }
}

pub struct VerificationStore {
    backend: Box<dyn VerificationBackend>,
}

impl Default for VerificationStore {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct MintedToken {
    pub token: VerificationToken,
    /// Plaintext shown to the user EXACTLY once (in the email body).
    /// Never persisted server-side after this struct is dropped.
    pub plaintext: String,
}

impl VerificationStore {
    /// Default expiry windows. Password reset is short to limit
    /// blast radius if an inbox is compromised post-issue. Magic
    /// links match the existing magic-code TTL. Email change is
    /// longer because the user might be away from their old inbox.
    const PASSWORD_RESET_TTL_SECS: u64 = 30 * 60; // 30 min
    const MAGIC_LINK_TTL_SECS: u64 = 15 * 60; // 15 min
    const EMAIL_CHANGE_TTL_SECS: u64 = 24 * 60 * 60; // 24 hours

    pub fn new() -> Self {
        Self::with_backend(Box::new(InMemoryVerificationBackend::default()))
    }

    pub fn with_backend(backend: Box<dyn VerificationBackend>) -> Self {
        Self { backend }
    }

    /// Mint + store a token. Returns the plaintext (caller emails it
    /// to the user) alongside the persisted record (whose
    /// `token_hash` is what's saved).
    pub fn mint(
        &self,
        kind: TokenKind,
        email: &str,
        user_id: Option<String>,
        payload: Option<String>,
    ) -> MintedToken {
        let id = format!("vt_{}", random_token(20));
        let plaintext = random_token(32);
        let prefix: String = plaintext.chars().take(8).collect();
        let token_hash = hash_token(&plaintext);
        let now = now_secs();
        let ttl = match kind {
            TokenKind::PasswordReset => Self::PASSWORD_RESET_TTL_SECS,
            TokenKind::MagicLink => Self::MAGIC_LINK_TTL_SECS,
            TokenKind::EmailChange => Self::EMAIL_CHANGE_TTL_SECS,
        };
        let token = VerificationToken {
            id,
            kind,
            email: email.to_lowercase(),
            user_id,
            payload,
            token_hash,
            token_prefix: prefix,
            created_at: now,
            expires_at: now + ttl,
            consumed_at: None,
        };
        self.backend.put(&token);
        MintedToken { token, plaintext }
    }

    /// Look up + consume a plaintext token. Returns the matching
    /// record on success — the caller then applies the side effect
    /// (set new password, swap email, mint session). Idempotent: a
    /// second call with the same token returns `AlreadyConsumed`.
    pub fn consume(
        &self,
        plaintext: &str,
        expected_kind: TokenKind,
    ) -> Result<VerificationToken, VerificationError> {
        let prefix: String = plaintext.chars().take(8).collect();
        // Constant-set HMAC prevents timing distinction from the
        // narrow prefix lookup. Per-row hash compare is also
        // constant-time via constant_time_eq.
        let expected_hash = hash_token(plaintext);
        let candidates = self.backend.by_prefix(&prefix);
        let now = now_secs();
        for t in candidates {
            if !crate::constant_time_eq(t.token_hash.as_bytes(), expected_hash.as_bytes()) {
                continue;
            }
            // Hash matched — now run the lifecycle checks.
            if t.kind != expected_kind {
                return Err(VerificationError::KindMismatch);
            }
            if t.consumed_at.is_some() {
                return Err(VerificationError::AlreadyConsumed);
            }
            if t.expires_at <= now {
                return Err(VerificationError::Expired);
            }
            // Atomic mark-consumed; loses the race if another
            // concurrent verify already did it.
            if !self.backend.mark_consumed(&t.id, now) {
                return Err(VerificationError::AlreadyConsumed);
            }
            return Ok(t);
        }
        Err(VerificationError::NotFound)
    }

    /// Opportunistic sweep — call from a background tick.
    pub fn purge_expired(&self) {
        self.backend.purge_expired(now_secs());
    }
}

fn random_token(n_bytes: usize) -> String {
    use rand::RngCore;
    let mut bytes = vec![0u8; n_bytes];
    rand::thread_rng().fill_bytes(&mut bytes);
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    URL_SAFE_NO_PAD.encode(bytes)
}

fn hash_token(plaintext: &str) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;
    // Same pepper as API keys — both classes are server-side random
    // 32-byte secrets. Sharing the pepper is fine because we never
    // accept one token's hash as proof for the other (the consume
    // path narrows by `prefix` then by `token_hash` then by `kind`).
    let pepper = std::env::var("PYLON_API_KEY_PEPPER")
        .unwrap_or_else(|_| "pylon-dev-api-key-pepper-not-for-production".into());
    let mut mac =
        HmacSha256::new_from_slice(pepper.as_bytes()).expect("HMAC accepts any key length");
    mac.update(plaintext.as_bytes());
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
    fn mint_and_consume_round_trip() {
        let store = VerificationStore::new();
        let minted = store.mint(TokenKind::PasswordReset, "alice@example.com", None, None);
        let consumed = store
            .consume(&minted.plaintext, TokenKind::PasswordReset)
            .expect("consume");
        assert_eq!(consumed.id, minted.token.id);
        assert_eq!(consumed.email, "alice@example.com");
    }

    #[test]
    fn consume_is_single_use() {
        let store = VerificationStore::new();
        let minted = store.mint(TokenKind::MagicLink, "a@b.com", None, None);
        store
            .consume(&minted.plaintext, TokenKind::MagicLink)
            .unwrap();
        let err = store
            .consume(&minted.plaintext, TokenKind::MagicLink)
            .unwrap_err();
        assert_eq!(err, VerificationError::AlreadyConsumed);
    }

    #[test]
    fn cross_kind_replay_rejected() {
        // Critical safety check: a token minted as a magic-link
        // must NOT be accepted as a password-reset token even
        // though both share the same hash + expiry shape.
        let store = VerificationStore::new();
        let minted = store.mint(TokenKind::MagicLink, "a@b.com", None, None);
        let err = store
            .consume(&minted.plaintext, TokenKind::PasswordReset)
            .unwrap_err();
        assert_eq!(err, VerificationError::KindMismatch);
    }

    #[test]
    fn unknown_token_returns_not_found() {
        let store = VerificationStore::new();
        let err = store
            .consume(
                "nonexistent_plaintext_xxxxxxxxxxxxxxxxxxxx",
                TokenKind::PasswordReset,
            )
            .unwrap_err();
        assert_eq!(err, VerificationError::NotFound);
    }

    #[test]
    fn email_lowercased_at_mint() {
        let store = VerificationStore::new();
        let minted = store.mint(TokenKind::MagicLink, "MIXED@CASE.com", None, None);
        assert_eq!(minted.token.email, "mixed@case.com");
    }

    #[test]
    fn payload_round_trips() {
        // Email-change flow stuffs the proposed new email into payload.
        let store = VerificationStore::new();
        let minted = store.mint(
            TokenKind::EmailChange,
            "new@example.com",
            Some("user-1".into()),
            Some("new@example.com".into()),
        );
        let consumed = store
            .consume(&minted.plaintext, TokenKind::EmailChange)
            .unwrap();
        assert_eq!(consumed.payload.as_deref(), Some("new@example.com"));
        assert_eq!(consumed.user_id.as_deref(), Some("user-1"));
    }

    #[test]
    fn expired_token_rejected() {
        let store = VerificationStore::new();
        let minted = store.mint(TokenKind::MagicLink, "a@b.com", None, None);
        // Force expiry by mutating the backend directly.
        let backend = InMemoryVerificationBackend::default();
        let mut expired = minted.token.clone();
        expired.expires_at = 1;
        backend.put(&expired);
        let store2 = VerificationStore::with_backend(Box::new(backend));
        let err = store2
            .consume(&minted.plaintext, TokenKind::MagicLink)
            .unwrap_err();
        assert_eq!(err, VerificationError::Expired);
    }
}
