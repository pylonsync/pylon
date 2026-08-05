//! Per-user OAuth connection registry.
//!
//! Distinct from the framework's "sign in with Google" flow:
//! `ctx.connections.*` lets an authenticated user link an external
//! service account (Google Calendar, GitHub repos, Slack workspace,
//! Notion workspace, etc.) so server-side actions can call those
//! services on their behalf.
//!
//! Flow:
//! 1. App declares `defineConnection({ name, provider, scopes })`
//!    in `app.ts`. Manifest carries the list.
//! 2. User triggers `ctx.connections.authorizeUrl("google")` from
//!    an action — server returns the provider's authorization URL.
//! 3. User completes the consent screen, provider redirects to
//!    `/api/connections/<name>/callback?code=...`.
//! 4. Callback exchanges the code, stores the token pair on the
//!    `_Connection` row keyed by `(userId, name)`.
//! 5. Future action calls `ctx.connections.get("google")` — the
//!    framework returns a fresh access token, refreshing
//!    transparently when `expires_at` is within 60s of now.
//!
//! Storage: the `_Connection` entity is auto-created at boot — apps
//! don't need to declare it. Token fields are AEAD-encrypted at rest
//! when `PYLON_ENCRYPTION_KEY` is set (which is required when
//! manifest.connections has any entries). See
//! `docs/auth/connections.md` for the full operator surface.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::encryption::EncryptionKey;
use pylon_auth::OAuthStateBackend;

// ---------------------------------------------------------------------------
// Config — the developer-facing surface
// ---------------------------------------------------------------------------

/// One connection the app declared via `defineConnection({...})`.
/// Carried inside the manifest so codegen + runtime both see the
/// same shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionDef {
    /// App-facing name (`"google"`, `"github"`, `"slack-team"`).
    /// This is the key in `ctx.connections.get(name)`.
    pub name: String,
    /// Provider identifier the framework's OAuth client understands
    /// (`"google"`, `"github"`, `"slack"`, `"microsoft"`, etc.).
    /// Different from `name` — apps can have multiple connections
    /// pointing at the same provider with different scopes.
    pub provider: String,
    /// Whitespace-separated scopes (e.g. `"email profile"`). When
    /// empty the framework uses the provider's default.
    #[serde(default)]
    pub scopes: String,
}

// ---------------------------------------------------------------------------
// Stored connection row
// ---------------------------------------------------------------------------

/// Persistent connection row — one per (user, connection name).
/// Stored in the `_Connection` system entity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredConnection {
    pub id: String,
    pub user_id: String,
    /// Matches `ConnectionDef.name`.
    pub name: String,
    pub provider: String,
    /// Access token, plaintext in memory only. Encrypted on disk
    /// (see `field.encrypted()` integration in the schema below).
    pub access_token: String,
    /// Refresh token. None when the provider didn't return one
    /// (GitHub classic apps, some Slack flows).
    pub refresh_token: Option<String>,
    /// Unix seconds when the access token expires. None when the
    /// provider didn't return `expires_in`.
    pub expires_at: Option<u64>,
    /// Whitespace-separated scopes the user actually granted (may
    /// differ from what the app requested).
    pub scope: Option<String>,
    /// Unix seconds when this row was last refreshed.
    pub updated_at: u64,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum ConnectionError {
    UnknownConnection {
        name: String,
    },
    ProviderNotConfigured {
        provider: String,
        missing_env: String,
    },
    NotConnected {
        name: String,
    },
    AuthFailed(String),
    RefreshFailed(String),
    StorageFailed(String),
    /// The connection requires encryption but `PYLON_ENCRYPTION_KEY`
    /// is unset. Refresh tokens MUST NOT live in plaintext on disk —
    /// the framework refuses to store / fetch.
    EncryptionRequired,
}

impl std::fmt::Display for ConnectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownConnection { name } => {
                write!(f, "Connection \"{name}\" is not declared in the manifest")
            }
            Self::ProviderNotConfigured { provider, missing_env } => write!(
                f,
                "Provider \"{provider}\" is not configured — set {missing_env}"
            ),
            Self::NotConnected { name } => {
                write!(f, "User has not connected \"{name}\" yet")
            }
            Self::AuthFailed(msg) => write!(f, "OAuth authorization failed: {msg}"),
            Self::RefreshFailed(msg) => write!(f, "Refresh failed: {msg}"),
            Self::StorageFailed(msg) => write!(f, "Storage failed: {msg}"),
            Self::EncryptionRequired => write!(
                f,
                "ctx.connections.* requires PYLON_ENCRYPTION_KEY to be set — refresh tokens are persisted encrypted at rest"
            ),
        }
    }
}

impl std::error::Error for ConnectionError {}

impl ConnectionError {
    /// Code suffix for error response envelopes / `err.code`.
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnknownConnection { .. } => "CONNECTION_UNKNOWN",
            Self::ProviderNotConfigured { .. } => "PROVIDER_NOT_CONFIGURED",
            Self::NotConnected { .. } => "CONNECTION_NOT_LINKED",
            Self::AuthFailed(_) => "AUTH_FAILED",
            Self::RefreshFailed(_) => "REFRESH_FAILED",
            Self::StorageFailed(_) => "STORAGE_FAILED",
            Self::EncryptionRequired => "ENCRYPTION_REQUIRED",
        }
    }
}

// ---------------------------------------------------------------------------
// Provider client cache + redirect builder
// ---------------------------------------------------------------------------

/// Returns the env var pylon reads for the given provider's client
/// id. Operators get a clear "set PYLON_OAUTH_<PROVIDER>_CLIENT_ID"
/// hint when configuration is missing.
pub fn client_id_env(provider: &str) -> String {
    format!("PYLON_OAUTH_{}_CLIENT_ID", provider.to_uppercase())
}

pub fn client_secret_env(provider: &str) -> String {
    format!("PYLON_OAUTH_{}_CLIENT_SECRET", provider.to_uppercase())
}

/// Build the per-deploy callback URL for `<connection_name>`.
/// Derived from `PYLON_PUBLIC_URL` (which already gates the OAuth
/// trusted-origin path for the framework's own auth) or a literal
/// override via `PYLON_CONNECTIONS_CALLBACK_BASE`.
pub fn callback_url(connection_name: &str) -> Result<String, ConnectionError> {
    let base = std::env::var("PYLON_CONNECTIONS_CALLBACK_BASE")
        .ok()
        .or_else(|| std::env::var("PYLON_PUBLIC_URL").ok())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ConnectionError::ProviderNotConfigured {
            provider: connection_name.into(),
            missing_env:
                "PYLON_PUBLIC_URL (or PYLON_CONNECTIONS_CALLBACK_BASE) for the callback URL".into(),
        })?;
    let base = base.trim_end_matches('/');
    Ok(format!("{base}/api/connections/{connection_name}/callback"))
}

// ---------------------------------------------------------------------------
// Manager
// ---------------------------------------------------------------------------

/// Per-process registry of declared connections + state-token store.
/// State-token reuses the same backend the framework's "sign in
/// with X" flow uses — that store is the canonical CSRF defender.
pub struct ConnectionManager {
    defs: HashMap<String, ConnectionDef>,
    encryption_key: Option<EncryptionKey>,
    state_backend: Arc<dyn OAuthStateBackend>,
    /// Process-wide rate limit per (user, connection) to prevent
    /// runaway refresh loops. Codex-style defense — refresh on
    /// every read would burn the provider's rate budget.
    last_refresh: Mutex<HashMap<(String, String), std::time::Instant>>,
    /// Per-(user, connection) lock that serializes the entire
    /// refresh+persist critical section. Two concurrent `get` calls
    /// for the same key serialize on this lock instead of both
    /// running through `exchange_refresh_token` — without it, the
    /// provider's refresh-token rotation invalidates the first
    /// caller's new token between calls. Codex P1.
    refresh_locks: Mutex<HashMap<(String, String), Arc<Mutex<()>>>>,
}

impl ConnectionManager {
    pub fn new(
        defs: Vec<ConnectionDef>,
        encryption_key: Option<EncryptionKey>,
        state_backend: Arc<dyn OAuthStateBackend>,
    ) -> Self {
        let mut map = HashMap::new();
        for d in defs {
            map.insert(d.name.clone(), d);
        }
        Self {
            defs: map,
            encryption_key,
            state_backend,
            last_refresh: Mutex::new(HashMap::new()),
            refresh_locks: Mutex::new(HashMap::new()),
        }
    }

    /// Get-or-create the per-(user, connection) refresh lock.
    /// Held for the entire exchange+persist sequence so concurrent
    /// `get` calls for the same key serialize.
    fn refresh_lock_for(&self, user_id: &str, connection_name: &str) -> Arc<Mutex<()>> {
        let key = (user_id.to_string(), connection_name.to_string());
        let mut guard = self.refresh_locks.lock().unwrap_or_else(|p| p.into_inner());
        guard
            .entry(key)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }

    pub fn get_def(&self, name: &str) -> Option<&ConnectionDef> {
        self.defs.get(name)
    }

    pub fn names(&self) -> Vec<String> {
        self.defs.keys().cloned().collect()
    }

    pub fn encryption_configured(&self) -> bool {
        self.encryption_key.is_some()
    }

    /// Build the provider's authorization URL for a user. The caller
    /// is expected to redirect the browser to this URL; the user
    /// returns to `/api/connections/<name>/callback` after consent.
    ///
    /// Returns `(auth_url, state_token)`. The state token is also
    /// persisted to the OAuth state backend so the callback can
    /// verify it.
    pub fn build_auth_url(
        &self,
        connection_name: &str,
        user_id: &str,
        post_callback_redirect: Option<&str>,
    ) -> Result<String, ConnectionError> {
        if !self.encryption_configured() {
            // Refresh tokens MUST NOT land in plaintext. Fail BEFORE
            // we send the user through consent so the operator sees
            // the gap immediately, not after a 30-second OAuth dance.
            return Err(ConnectionError::EncryptionRequired);
        }
        // Codex P1: validate post_redirect is a relative path. The
        // callback handler 302s here after success — without
        // validation it's a CWE-601 open redirect (attacker
        // smuggles users to https://evil.com after consent).
        if let Some(r) = post_callback_redirect {
            if !is_safe_relative_redirect(r) {
                return Err(ConnectionError::AuthFailed(format!(
                    "post_redirect must be a relative path starting with `/` (got: {r})"
                )));
            }
        }
        let def =
            self.get_def(connection_name)
                .ok_or_else(|| ConnectionError::UnknownConnection {
                    name: connection_name.into(),
                })?;
        let cfg = self.build_oauth_config(def)?;

        // Mint the CSRF state token, embedding user_id + connection
        // name + post-callback redirect target inside the token's
        // accompanying state row (NOT the URL state param itself,
        // which only needs to be opaque + unguessable). Format:
        // `conn:<random_hex>`. The state backend tracks expiry +
        // single-use semantics; we add a per-user-per-connection
        // payload via a wrapper table maintained alongside.
        let state_token = mint_random_token();
        let callback = callback_url(connection_name)?;
        let post_redirect = post_callback_redirect.unwrap_or("");
        // Stash {user_id, connection_name, post_redirect} on the
        // state row. We use the `callback_url` field for our payload
        // (a `:` separator-encoded triple) because the existing
        // OAuthStateBackend already persists that field for the
        // framework auth flow. The callback handler decodes it.
        let payload = encode_state_payload(user_id, connection_name, post_redirect);
        let expires_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
            + 10 * 60;
        self.state_backend.put(
            &state_token,
            &pylon_auth::OAuthState {
                provider: def.provider.clone(),
                callback_url: payload,
                // error_callback_url field repurposed to carry the
                // resolved redirect URL so the callback handler can
                // reach it without re-parsing. Empty when not set.
                error_callback_url: callback.clone(),
                pkce_verifier: None,
                expires_at,
            },
        );

        // Build the URL with state. Use auth_url_with_state because
        // the upstream OAuth client already URL-encodes the state.
        let mut url = cfg.auth_url_with_state(&state_token);
        // Apply scope override when the manifest declared scopes.
        // OAuthConfig's auth_url consumed `scopes_override` from the
        // cfg we built — already applied via `build_oauth_config`.
        // But if cfg has no scope and the manifest provided none,
        // the provider's default scope kicks in.
        if url.is_empty() {
            // OAuthConfig::auth_url returns "" on unknown provider
            // — surface that as a typed error.
            return Err(ConnectionError::ProviderNotConfigured {
                provider: def.provider.clone(),
                missing_env: format!(
                    "{} (unknown provider \"{}\"; check spelling against pylon-auth providers)",
                    client_id_env(&def.provider),
                    def.provider
                ),
            });
        }
        // Force prompt=consent for Google so refresh tokens are
        // issued on every link. Google omits the refresh token on
        // subsequent authorizations unless the user is explicitly
        // re-prompted. Without this, second-link flows return
        // access-only tokens that can't be refreshed.
        if def.provider == "google" && !url.contains("prompt=") {
            let sep = if url.contains('?') { '&' } else { '?' };
            url.push(sep);
            url.push_str("prompt=consent&access_type=offline");
        }
        Ok(url)
    }

    /// Exchange the callback code for a token set. Caller is the
    /// HTTP route handler; it consumes the state from the backend
    /// (single-use), validates expiry, then stores the resulting
    /// `StoredConnection` via the provided store callback.
    ///
    /// `store_fn` writes the row to the `_Connection` entity. The
    /// manager doesn't take a `DataStore` directly to avoid a cycle
    /// with the runtime crate.
    pub fn complete_callback<F>(
        &self,
        code: &str,
        state_token: &str,
        store_fn: F,
    ) -> Result<CallbackOutcome, ConnectionError>
    where
        F: FnOnce(&StoredConnection) -> Result<(), String>,
    {
        // Defense-in-depth (codex P2): even though build_auth_url
        // refused without a key, the env could have been rotated
        // off between the auth-url request and the callback. Fail
        // here too so the OAuth code never gets exchanged into a
        // row we can't encrypt.
        if !self.encryption_configured() {
            return Err(ConnectionError::EncryptionRequired);
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let state = self
            .state_backend
            .take(state_token, now)
            .ok_or_else(|| ConnectionError::AuthFailed("invalid or expired state".into()))?;
        let (user_id, connection_name, post_redirect) =
            decode_state_payload(&state.callback_url)
                .ok_or_else(|| ConnectionError::AuthFailed("state payload malformed".into()))?;
        let def =
            self.get_def(&connection_name)
                .ok_or_else(|| ConnectionError::UnknownConnection {
                    name: connection_name.clone(),
                })?;
        let cfg = self.build_oauth_config(def)?;
        let tokens = cfg
            .exchange_code_full(code)
            .map_err(|e| ConnectionError::AuthFailed(e))?;
        let stored = StoredConnection {
            id: stable_id(&user_id, &connection_name),
            user_id: user_id.clone(),
            name: connection_name.clone(),
            provider: def.provider.clone(),
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token,
            expires_at: tokens.expires_at,
            scope: tokens.scope,
            updated_at: now,
        };
        store_fn(&stored).map_err(ConnectionError::StorageFailed)?;
        Ok(CallbackOutcome {
            user_id,
            connection_name,
            post_redirect,
        })
    }

    /// Fetch a fresh access token for `(user_id, connection_name)`.
    ///
    /// Returns the stored access token when it's not about to expire
    /// (>60s remaining). Otherwise calls `exchange_refresh_token`,
    /// upserts the new token pair via `store_fn`, and returns the new
    /// access token. Refresh-token rotation is respected — the new
    /// refresh token (when the provider sends one) replaces the old.
    ///
    /// `load_fn` reads the current stored row; the manager doesn't
    /// touch DataStore directly to dodge the dep cycle.
    pub fn ensure_fresh_token<L, S>(
        &self,
        user_id: &str,
        connection_name: &str,
        load_fn: L,
        store_fn: S,
    ) -> Result<String, ConnectionError>
    where
        L: Fn() -> Result<Option<StoredConnection>, String>,
        S: FnOnce(&StoredConnection) -> Result<(), String>,
    {
        let def =
            self.get_def(connection_name)
                .ok_or_else(|| ConnectionError::UnknownConnection {
                    name: connection_name.into(),
                })?;
        // Fast path: read first WITHOUT taking the refresh lock.
        // If the token isn't expiring, we don't need to refresh.
        // Avoids serializing every `get` call behind the per-key
        // mutex when the common case is "token is fresh enough".
        let row = load_fn()
            .map_err(ConnectionError::StorageFailed)?
            .ok_or_else(|| ConnectionError::NotConnected {
                name: connection_name.into(),
            })?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let needs_refresh = match row.expires_at {
            Some(exp) => exp.saturating_sub(now) <= 60,
            None => false,
        };
        if !needs_refresh {
            return Ok(row.access_token);
        }

        // Slow path: serialize the entire exchange+persist via the
        // per-(user, connection) lock. Codex P1 fix — without this,
        // two concurrent refreshes both pass the rate-limit gate,
        // both call the provider, and rotation invalidates the
        // first response between calls.
        let key_lock = self.refresh_lock_for(user_id, connection_name);
        let _guard = key_lock.lock().unwrap_or_else(|p| p.into_inner());

        // Re-load INSIDE the lock — another caller may have just
        // refreshed while we were waiting. If the freshly-loaded
        // row no longer needs refreshing, return its access token
        // and skip the exchange.
        let mut row = load_fn()
            .map_err(ConnectionError::StorageFailed)?
            .ok_or_else(|| ConnectionError::NotConnected {
                name: connection_name.into(),
            })?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let still_needs_refresh = match row.expires_at {
            Some(exp) => exp.saturating_sub(now) <= 60,
            None => false,
        };
        if !still_needs_refresh {
            return Ok(row.access_token);
        }
        let refresh_token = row.refresh_token.clone().ok_or_else(|| {
            ConnectionError::RefreshFailed(
                "no refresh token available — the provider didn't return one. User must re-link the connection.".into(),
            )
        })?;

        // Rate-limit (defense-in-depth — the per-key lock already
        // serializes, so this catches the rare case where two
        // refreshes complete back-to-back within 1s and the
        // provider's exchange endpoint is being abused).
        {
            let mut guard = self.last_refresh.lock().unwrap_or_else(|p| p.into_inner());
            let key = (user_id.to_string(), connection_name.to_string());
            let now_instant = std::time::Instant::now();
            if let Some(last) = guard.get(&key) {
                if now_instant.saturating_duration_since(*last) < std::time::Duration::from_secs(1)
                {
                    return Err(ConnectionError::RefreshFailed(
                        "refresh rate-limited (1/sec per user+connection)".into(),
                    ));
                }
            }
            guard.insert(key, now_instant);
            // Evict entries older than 5 minutes to prevent
            // unbounded growth (codex P2). 5 minutes is plenty
            // long past the 1-second gate.
            let cutoff = std::time::Duration::from_secs(300);
            guard.retain(|_, t| now_instant.saturating_duration_since(*t) < cutoff);
        }

        let cfg = self.build_oauth_config(def)?;
        let new_tokens = cfg
            .exchange_refresh_token(&refresh_token)
            .map_err(|e| ConnectionError::RefreshFailed(sanitize_provider_message(&e)))?;
        row.access_token = new_tokens.access_token.clone();
        if let Some(rt) = new_tokens.refresh_token {
            row.refresh_token = Some(rt);
        }
        row.expires_at = new_tokens.expires_at;
        if new_tokens.scope.is_some() {
            row.scope = new_tokens.scope;
        }
        row.updated_at = now;
        // Persist the rotated token pair. Codex P1: if this fails
        // AFTER the provider rotated the refresh token, the user
        // is silently locked out. Log loudly at error level with
        // structured data so an operator can re-issue a re-link
        // prompt; return a distinct error so callers can
        // distinguish "provider rejected" from "we lost the
        // rotated token".
        if let Err(e) = store_fn(&row) {
            tracing::error!(
                user_id = %user_id,
                connection = %connection_name,
                provider = %def.provider,
                error = %e,
                "[connections] REFRESH_PERSISTENCE_LOST: provider rotated refresh token but we failed to persist. User must re-link.",
            );
            return Err(ConnectionError::RefreshFailed(format!(
                "Refresh succeeded but persistence failed; user must re-link \"{connection_name}\""
            )));
        }
        Ok(row.access_token)
    }

    // -----------------------------------------------------------------------
    // Internals
    // -----------------------------------------------------------------------

    fn build_oauth_config(
        &self,
        def: &ConnectionDef,
    ) -> Result<pylon_auth::OAuthConfig, ConnectionError> {
        let client_id = std::env::var(client_id_env(&def.provider))
            .ok()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ConnectionError::ProviderNotConfigured {
                provider: def.provider.clone(),
                missing_env: client_id_env(&def.provider),
            })?;
        let client_secret = std::env::var(client_secret_env(&def.provider))
            .ok()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ConnectionError::ProviderNotConfigured {
                provider: def.provider.clone(),
                missing_env: client_secret_env(&def.provider),
            })?;
        let redirect_uri = callback_url(&def.name)?;
        let scopes_override = if def.scopes.is_empty() {
            None
        } else {
            Some(def.scopes.clone())
        };
        Ok(pylon_auth::OAuthConfig {
            provider: def.provider.clone(),
            client_id,
            client_secret,
            redirect_uri,
            scopes_override,
            tenant: None,
            #[cfg(not(target_arch = "wasm32"))]
            apple: None,
            oidc_issuer: None,
        })
    }
}

/// Returned by [`ConnectionManager::complete_callback`] so the
/// HTTP handler can issue the post-link redirect.
#[derive(Debug, Clone)]
pub struct CallbackOutcome {
    pub user_id: String,
    pub connection_name: String,
    /// App-supplied destination for the browser after a successful
    /// link. May be empty when the caller of `authorizeUrl` didn't
    /// supply one — handler should fall back to a sane default.
    pub post_redirect: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Deterministic id derived from (user_id, connection_name) so
/// upsert is "delete + insert" or "update" without needing a
/// separate uniqueness index. SHA-256 hash truncated to 40 hex
/// chars to fit Pylon's 40-char id constraint.
pub fn stable_id(user_id: &str, connection_name: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(user_id.as_bytes());
    h.update(b"\0");
    h.update(connection_name.as_bytes());
    let digest = h.finalize();
    let mut out = String::with_capacity(40);
    for byte in &digest[..20] {
        use std::fmt::Write as _;
        write!(out, "{byte:02x}").unwrap();
    }
    out
}

/// Strip anything that looks like a key from a provider error
/// message before surfacing to the caller. Codex P2: provider
/// bodies sometimes echo client_id / scope mistakes — keep the
/// shape "useful for ops" but drop key prefixes.
fn sanitize_provider_message(raw: &str) -> String {
    let mut out = raw.to_string();
    for prefix in ["sk-", "ya29.", "ghp_", "ghs_", "tgp_v1_", "gsk_", "pplx-"] {
        while let Some(start) = out.find(prefix) {
            let after = &out[start..];
            let end_rel = after
                .find(|c: char| c.is_whitespace() || c == '"' || c == ',' || c == '}')
                .unwrap_or(after.len());
            out.replace_range(start..start + end_rel, "<redacted>");
        }
    }
    if out.len() > 512 {
        let mut cut = 512;
        while cut > 0 && !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
        out.push_str("...");
    }
    out
}

/// Reject anything that could escape the app's origin.
/// Allowlist: starts with `/` AND doesn't start with `//` (protocol-
/// relative URL that browsers resolve to a foreign origin) or `/\`.
pub(crate) fn is_safe_relative_redirect(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let bytes = s.as_bytes();
    if bytes[0] != b'/' {
        return false;
    }
    if bytes.len() >= 2 && (bytes[1] == b'/' || bytes[1] == b'\\') {
        return false;
    }
    // Reject control characters that could confuse browsers
    // resolving the Location header.
    if s.chars().any(|c| c.is_control()) {
        return false;
    }
    true
}

fn mint_random_token() -> String {
    use ring::rand::{SecureRandom, SystemRandom};
    let mut bytes = [0u8; 24];
    SystemRandom::new().fill(&mut bytes).unwrap_or(());
    let mut out = String::with_capacity(48);
    for byte in &bytes {
        use std::fmt::Write as _;
        write!(out, "{byte:02x}").unwrap();
    }
    out
}

/// Encode (user_id, connection_name, post_redirect) into the
/// state's `callback_url` slot. Format:
/// `pylon-conn-v1:<user_id_b64>:<conn_b64>:<redirect_b64>`. URL-safe
/// base64 so we don't accidentally have `:` collide with the
/// separator inside the data.
fn encode_state_payload(user_id: &str, connection_name: &str, post_redirect: &str) -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    let a = URL_SAFE_NO_PAD.encode(user_id.as_bytes());
    let b = URL_SAFE_NO_PAD.encode(connection_name.as_bytes());
    let c = URL_SAFE_NO_PAD.encode(post_redirect.as_bytes());
    format!("pylon-conn-v1:{a}:{b}:{c}")
}

fn decode_state_payload(payload: &str) -> Option<(String, String, String)> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    let rest = payload.strip_prefix("pylon-conn-v1:")?;
    let mut parts = rest.split(':');
    let a = parts.next()?;
    let b = parts.next()?;
    let c = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let user_id = String::from_utf8(URL_SAFE_NO_PAD.decode(a).ok()?).ok()?;
    let name = String::from_utf8(URL_SAFE_NO_PAD.decode(b).ok()?).ok()?;
    let redirect = String::from_utf8(URL_SAFE_NO_PAD.decode(c).ok()?).ok()?;
    Some((user_id, name, redirect))
}

// ---------------------------------------------------------------------------
// Auto-created _Connection entity
// ---------------------------------------------------------------------------

/// Returns the manifest entity definition for `_Connection`. The
/// framework injects this at boot so apps don't have to declare it
/// manually. Token fields are marked `serverOnly + encrypted` so
/// the wire never sees plaintext (and rests-at-rest stays
/// ciphertext given `PYLON_ENCRYPTION_KEY`).
pub fn connection_entity() -> pylon_kernel::ManifestEntity {
    use pylon_kernel::{ManifestField, ManifestIndex};
    let field = |name: &str, ft: &str, opts: FieldOpts| ManifestField {
        name: name.into(),
        field_type: ft.into(),
        optional: opts.optional,
        unique: false,
        crdt: None,
        server_only: opts.server_only,
        readonly: false,
        default: None,
        enum_values: None,
        encrypted: opts.encrypted,
        sync_omit: false,
    };
    pylon_kernel::ManifestEntity {
        name: "_Connection".into(),
        fields: vec![
            // `id` is auto-created by the storage layer (CREATE TABLE
            // always includes an id PK); declaring it here would
            // collide with a "duplicate column name: id" error.
            field("userId", "id(User)", FieldOpts::default()),
            field("name", "string", FieldOpts::default()),
            field("provider", "string", FieldOpts::default()),
            field(
                "accessToken",
                "string",
                FieldOpts {
                    server_only: true,
                    encrypted: true,
                    optional: false,
                },
            ),
            field(
                "refreshToken",
                "string",
                FieldOpts {
                    server_only: true,
                    encrypted: true,
                    optional: true,
                },
            ),
            field(
                "expiresAt",
                "int",
                FieldOpts {
                    optional: true,
                    ..FieldOpts::default()
                },
            ),
            field(
                "scope",
                "string",
                FieldOpts {
                    server_only: true,
                    optional: true,
                    ..FieldOpts::default()
                },
            ),
            field("updatedAt", "int", FieldOpts::default()),
        ],
        indexes: vec![ManifestIndex {
            name: "_Connection_user_name".into(),
            fields: vec!["userId".into(), "name".into()],
            unique: true,
            where_clause: None,
        }],
        relations: vec![],
        search: None,
        crdt: false,
        sync: true,
        ..Default::default()
    }
}

#[derive(Default, Clone, Copy)]
struct FieldOpts {
    optional: bool,
    server_only: bool,
    encrypted: bool,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_id_is_deterministic_per_user_connection() {
        let a = stable_id("user-1", "google");
        let b = stable_id("user-1", "google");
        assert_eq!(a, b);
        assert_eq!(a.len(), 40);
        let c = stable_id("user-1", "github");
        assert_ne!(a, c);
        let d = stable_id("user-2", "google");
        assert_ne!(a, d);
    }

    #[test]
    fn state_payload_round_trips() {
        let p = encode_state_payload("user-1", "google", "/dashboard?ok=1");
        assert!(p.starts_with("pylon-conn-v1:"));
        let (u, n, r) = decode_state_payload(&p).unwrap();
        assert_eq!(u, "user-1");
        assert_eq!(n, "google");
        assert_eq!(r, "/dashboard?ok=1");
    }

    #[test]
    fn state_payload_rejects_extra_segments() {
        // Adversarial: caller supplies a payload with too many
        // segments (attacker tries to wedge extra fields). Reject.
        let bad = "pylon-conn-v1:dXNlcg:Z29vZ2xl:Lw:extra";
        assert!(decode_state_payload(bad).is_none());
    }

    #[test]
    fn state_payload_rejects_wrong_prefix() {
        let bad = encode_state_payload("u", "g", "/").replace("pylon-conn-v1:", "wrong:");
        assert!(decode_state_payload(&bad).is_none());
    }

    #[test]
    fn callback_url_requires_public_url_env() {
        std::env::remove_var("PYLON_PUBLIC_URL");
        std::env::remove_var("PYLON_CONNECTIONS_CALLBACK_BASE");
        let err = callback_url("google").unwrap_err();
        match err {
            ConnectionError::ProviderNotConfigured { .. } => {}
            other => panic!("expected ProviderNotConfigured, got {other:?}"),
        }
    }

    #[test]
    fn connection_entity_has_encrypted_serverOnly_tokens() {
        let e = connection_entity();
        let access = e.fields.iter().find(|f| f.name == "accessToken").unwrap();
        assert!(access.encrypted);
        assert!(access.server_only);
        let refresh = e.fields.iter().find(|f| f.name == "refreshToken").unwrap();
        assert!(refresh.encrypted);
        assert!(refresh.server_only);
        // userId is queryable — NOT encrypted (we need to find rows
        // by user).
        let uid = e.fields.iter().find(|f| f.name == "userId").unwrap();
        assert!(!uid.encrypted);
    }

    #[test]
    fn unique_index_on_user_and_name() {
        let e = connection_entity();
        let idx = e
            .indexes
            .iter()
            .find(|i| i.name == "_Connection_user_name")
            .unwrap();
        assert!(idx.unique);
        assert_eq!(idx.fields, vec!["userId".to_string(), "name".to_string()]);
    }

    #[test]
    fn safe_redirect_accepts_relative_paths() {
        assert!(is_safe_relative_redirect("/"));
        assert!(is_safe_relative_redirect("/dashboard"));
        assert!(is_safe_relative_redirect("/dashboard?connected=google"));
        assert!(is_safe_relative_redirect("/users/123#profile"));
    }

    #[test]
    fn safe_redirect_rejects_external_origins() {
        // Codex P1: open-redirect surface.
        assert!(!is_safe_relative_redirect(""));
        assert!(!is_safe_relative_redirect("https://evil.com"));
        assert!(!is_safe_relative_redirect("http://evil.com"));
        assert!(!is_safe_relative_redirect("//evil.com")); // protocol-relative
        assert!(!is_safe_relative_redirect("/\\evil.com")); // backslash trick
        assert!(!is_safe_relative_redirect("javascript:alert(1)"));
        assert!(!is_safe_relative_redirect("data:text/html,<script>"));
        assert!(!is_safe_relative_redirect("file:///etc/passwd"));
        // Header injection via CR/LF.
        assert!(!is_safe_relative_redirect("/ok\r\nLocation: https://evil"));
        assert!(!is_safe_relative_redirect("/ok\nX: y"));
    }

    #[test]
    fn mint_random_token_is_distinct_per_call() {
        let a = mint_random_token();
        let b = mint_random_token();
        assert_eq!(a.len(), 48);
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
