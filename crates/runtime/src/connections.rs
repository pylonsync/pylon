//! OAuth and API-key connection registry.
//!
//! A connection belongs to one signed-in user (`owner: "user"`, the
//! default) or to the whole app (`owner: "app"`). An app connection is
//! the business's own account (its Square, its Stripe): any server
//! function reads it, including one serving an anonymous visitor, and
//! only an admin connects or removes it. A connection is an OAuth grant
//! (`kind: "oauth"`, the default) or a set of key fields
//! (`kind: "api_key"`).
//!
//! Brokered mode: when `PYLON_CONNECTIONS_BROKER_URL` and
//! `PYLON_CONNECTIONS_BROKER_TOKEN` are set, the platform hosting the
//! app runs the OAuth flow for app connections with its own client
//! credentials and hands the tokens over through the management
//! endpoint (`PYLON_CONNECTIONS_TOKEN`, see server.rs). The app holds no
//! client secret; refreshes go to the broker. Without the broker the app
//! runs the flow itself with `PYLON_OAUTH_<PROVIDER>_CLIENT_ID/SECRET`.
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
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
    #[serde(default)]
    pub owner: ConnectionOwner,
    #[serde(default)]
    pub kind: ConnectionKind,
    /// API-key fields. Empty: the known provider's fields
    /// (`pylon_auth::integrations::api_key_spec`), else `["apiKey"]`.
    #[serde(default)]
    pub fields: Vec<String>,
}

/// Who a connection belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionOwner {
    #[default]
    User,
    App,
}

impl ConnectionOwner {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "" | "user" => Some(Self::User),
            "app" => Some(Self::App),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::App => "app",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionKind {
    #[default]
    Oauth,
    ApiKey,
}

impl ConnectionKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "" | "oauth" => Some(Self::Oauth),
            "api_key" => Some(Self::ApiKey),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Oauth => "oauth",
            Self::ApiKey => "api_key",
        }
    }
}

impl ConnectionDef {
    /// The key fields an API-key connection stores, in form order.
    pub fn credential_fields(&self) -> Vec<String> {
        if !self.fields.is_empty() {
            return self.fields.clone();
        }
        if let Some(spec) = pylon_auth::integrations::api_key_spec(&self.provider) {
            return spec.fields.iter().map(|f| f.to_string()).collect();
        }
        vec!["apiKey".into()]
    }
}

/// The `userId` of an app connection's row. Real user ids never start
/// with `_`.
pub const APP_OWNER: &str = "_app";

// ---------------------------------------------------------------------------
// Stored connection row
// ---------------------------------------------------------------------------

/// Persistent connection row — one per (owner, connection name), where
/// the owner is a user id or [`APP_OWNER`]. Stored in the `_Connection`
/// system entity. `Debug` leaves out the secrets.
#[derive(Clone, Serialize, Deserialize, Default)]
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
    /// The provider's id for the connected account (Square merchant,
    /// Stripe account, Clover merchant).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Non-secret provider data as a JSON object (Mailchimp's `dc`).
    #[serde(default)]
    pub metadata: Option<String>,
    /// API-key connections: every field as a JSON object. Encrypted at
    /// rest like the tokens.
    #[serde(default)]
    pub credentials: Option<String>,
}

impl std::fmt::Debug for StoredConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredConnection")
            .field("id", &self.id)
            .field("user_id", &self.user_id)
            .field("name", &self.name)
            .field("provider", &self.provider)
            .field("expires_at", &self.expires_at)
            .field("account_id", &self.account_id)
            .field("updated_at", &self.updated_at)
            .finish_non_exhaustive()
    }
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
    /// An OAuth operation on an API-key connection, or the reverse.
    WrongKind {
        name: String,
        expected: ConnectionKind,
    },
    /// The broker runs this app connection's OAuth flow.
    Managed {
        name: String,
    },
    /// The caller may not act on this connection.
    Forbidden(String),
    /// The submitted key fields have the wrong shape.
    InvalidCredentials(String),
    /// The provider rejected the key.
    CheckFailed(String),
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
            Self::WrongKind { name, expected } => write!(
                f,
                "Connection \"{name}\" is not an {} connection",
                match expected {
                    ConnectionKind::Oauth => "OAuth",
                    ConnectionKind::ApiKey => "API-key",
                }
            ),
            Self::Managed { name } => write!(
                f,
                "Connection \"{name}\" is connected through the platform that hosts this app"
            ),
            Self::Forbidden(msg) => write!(f, "{msg}"),
            Self::InvalidCredentials(msg) => write!(f, "Invalid credentials: {msg}"),
            Self::CheckFailed(msg) => write!(f, "{msg}"),
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
            Self::WrongKind { .. } => "CONNECTION_WRONG_KIND",
            Self::Managed { .. } => "CONNECTION_MANAGED",
            Self::Forbidden(_) => "CONNECTION_FORBIDDEN",
            Self::InvalidCredentials(_) => "INVALID_CREDENTIALS",
            Self::CheckFailed(_) => "CREDENTIAL_CHECK_FAILED",
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

/// The platform that runs OAuth for app connections (brokered mode).
#[derive(Clone)]
pub struct Broker {
    /// The refresh endpoint. https, or http on a loopback host.
    pub url: String,
    pub token: String,
}

impl std::fmt::Debug for Broker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Broker")
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}

impl Broker {
    /// Read `PYLON_CONNECTIONS_BROKER_URL` + `PYLON_CONNECTIONS_BROKER_TOKEN`.
    /// Both or neither: one without the other is a misconfiguration, and
    /// a plain-http URL to a remote host would send refresh tokens in the
    /// clear. Either is logged and treated as no broker.
    pub fn from_env() -> Option<Self> {
        let url = std::env::var("PYLON_CONNECTIONS_BROKER_URL")
            .ok()
            .filter(|s| !s.is_empty());
        let token = std::env::var("PYLON_CONNECTIONS_BROKER_TOKEN")
            .ok()
            .filter(|s| !s.is_empty());
        match (url, token) {
            (None, None) => None,
            (Some(url), Some(token)) => {
                if !broker_url_is_safe(&url) {
                    tracing::error!(
                        "[connections] PYLON_CONNECTIONS_BROKER_URL must be https (or http on localhost); ignoring the broker"
                    );
                    return None;
                }
                Some(Self { url, token })
            }
            _ => {
                tracing::error!(
                    "[connections] set both PYLON_CONNECTIONS_BROKER_URL and PYLON_CONNECTIONS_BROKER_TOKEN, or neither; ignoring the broker"
                );
                None
            }
        }
    }
}

pub(crate) fn broker_url_is_safe(url: &str) -> bool {
    if let Some(rest) = url.strip_prefix("https://") {
        return !rest.is_empty();
    }
    if let Some(rest) = url.strip_prefix("http://") {
        let host = rest.split(['/', ':']).next().unwrap_or("");
        return host == "localhost" || host == "127.0.0.1" || host == "[::1]";
    }
    false
}

/// Per-process registry of declared connections + state-token store.
/// State-token reuses the same backend the framework's "sign in
/// with X" flow uses — that store is the canonical CSRF defender.
pub struct ConnectionManager {
    defs: HashMap<String, ConnectionDef>,
    encryption_key: Option<EncryptionKey>,
    state_backend: Arc<dyn OAuthStateBackend>,
    broker: Option<Broker>,
    /// Process-wide rate limit per (owner, connection) to prevent
    /// runaway refresh loops. Codex-style defense — refresh on
    /// every read would burn the provider's rate budget.
    last_refresh: Mutex<HashMap<(String, String), std::time::Instant>>,
    /// Per-(owner, connection) lock that serializes the entire
    /// refresh+persist critical section. Two concurrent `get` calls
    /// for the same key serialize on this lock instead of both
    /// running through the refresh — without it, the provider's
    /// refresh-token rotation invalidates the first caller's new
    /// token between calls. Codex P1.
    refresh_locks: Mutex<HashMap<(String, String), Arc<Mutex<()>>>>,
}

impl ConnectionManager {
    pub fn new(
        defs: Vec<ConnectionDef>,
        encryption_key: Option<EncryptionKey>,
        state_backend: Arc<dyn OAuthStateBackend>,
    ) -> Self {
        Self::with_broker(defs, encryption_key, state_backend, Broker::from_env())
    }

    pub fn with_broker(
        defs: Vec<ConnectionDef>,
        encryption_key: Option<EncryptionKey>,
        state_backend: Arc<dyn OAuthStateBackend>,
        broker: Option<Broker>,
    ) -> Self {
        let mut map = HashMap::new();
        for d in defs {
            map.insert(d.name.clone(), d);
        }
        Self {
            defs: map,
            encryption_key,
            state_backend,
            broker,
            last_refresh: Mutex::new(HashMap::new()),
            refresh_locks: Mutex::new(HashMap::new()),
        }
    }

    /// Get-or-create the per-(owner, connection) refresh lock.
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

    /// Delete after any active refresh has persisted its result.
    /// This prevents that refresh from restoring a disconnected connection.
    pub fn disconnect<F>(
        &self,
        user_id: &str,
        connection_name: &str,
        delete_fn: F,
    ) -> Result<(), ConnectionError>
    where
        F: FnOnce() -> Result<(), String>,
    {
        // Undeclared connections cannot refresh. Still delete stale rows for
        // removed definitions without retaining locks for arbitrary names.
        if self.get_def(connection_name).is_none() {
            return delete_fn().map_err(ConnectionError::StorageFailed);
        }
        let key_lock = self.refresh_lock_for(user_id, connection_name);
        let _guard = key_lock.lock().unwrap_or_else(|p| p.into_inner());
        delete_fn().map_err(ConnectionError::StorageFailed)
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }

    pub fn get_def(&self, name: &str) -> Option<&ConnectionDef> {
        self.defs.get(name)
    }

    pub fn defs(&self) -> impl Iterator<Item = &ConnectionDef> {
        self.defs.values()
    }

    pub fn names(&self) -> Vec<String> {
        self.defs.keys().cloned().collect()
    }

    pub fn encryption_configured(&self) -> bool {
        self.encryption_key.is_some()
    }

    pub fn brokered(&self) -> bool {
        self.broker.is_some()
    }

    fn def_or_unknown(&self, name: &str) -> Result<&ConnectionDef, ConnectionError> {
        self.get_def(name)
            .ok_or_else(|| ConnectionError::UnknownConnection { name: name.into() })
    }

    /// The row owner for an operation on `name` by this caller. A user
    /// connection belongs to the signed-in caller. An app connection
    /// belongs to the app; `admin_only` operations on it (connect,
    /// disconnect, set keys) need an admin caller.
    pub fn owner_for(
        &self,
        name: &str,
        caller_user_id: Option<&str>,
        caller_is_admin: bool,
        admin_only: bool,
    ) -> Result<String, ConnectionError> {
        let def = self.def_or_unknown(name)?;
        match def.owner {
            ConnectionOwner::App => {
                if admin_only && !caller_is_admin {
                    return Err(ConnectionError::Forbidden(format!(
                        "Connection \"{name}\" belongs to the app; only an admin can change it (ctx.auth.elevate)"
                    )));
                }
                Ok(APP_OWNER.to_string())
            }
            ConnectionOwner::User => match caller_user_id {
                Some(u) if !u.is_empty() && !u.starts_with('_') => Ok(u.to_string()),
                _ => Err(ConnectionError::Forbidden(format!(
                    "Connection \"{name}\" belongs to a user; the caller must be signed in"
                ))),
            },
        }
    }

    /// Build the provider's authorization URL for a user, or for the
    /// app when `owner_id` is [`APP_OWNER`]. The caller is expected to
    /// redirect the browser to this URL; the browser returns to
    /// `/api/connections/<name>/callback` after consent.
    ///
    /// The state token is persisted to the OAuth state backend so the
    /// callback can verify it.
    pub fn build_auth_url(
        &self,
        connection_name: &str,
        owner_id: &str,
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
        let def = self.def_or_unknown(connection_name)?;
        if def.kind != ConnectionKind::Oauth {
            return Err(ConnectionError::WrongKind {
                name: def.name.clone(),
                expected: ConnectionKind::Oauth,
            });
        }
        if def.owner == ConnectionOwner::App && self.broker.is_some() {
            return Err(ConnectionError::Managed {
                name: def.name.clone(),
            });
        }

        // Mint the CSRF state token, embedding owner + connection
        // name + post-callback redirect target inside the token's
        // accompanying state row (NOT the URL state param itself,
        // which only needs to be opaque + unguessable). The state
        // backend tracks expiry + single-use semantics.
        let state_token = mint_random_token();
        let callback = callback_url(connection_name)?;
        let post_redirect = post_callback_redirect.unwrap_or("");
        let payload = encode_state_payload(owner_id, connection_name, post_redirect);
        let expires_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
            + 10 * 60;

        let (url, pkce_verifier) = if let Some(spec) = pylon_auth::integrations::find(&def.provider)
        {
            let (client_id, client_secret) = client_credentials(&def.provider)?;
            let pkce = spec.pkce.then(pylon_auth::generate_pkce);
            let client = pylon_auth::integrations::Client {
                client_id: &client_id,
                client_secret: &client_secret,
                redirect_uri: &callback,
            };
            let url = pylon_auth::integrations::authorize_url(
                spec,
                &client,
                &def.scopes,
                &state_token,
                pkce.as_ref().map(|p| p.code_challenge.as_str()),
            );
            (url, pkce.map(|p| p.code_verifier))
        } else {
            let cfg = self.build_oauth_config(def)?;
            let mut url = cfg.auth_url_with_state(&state_token);
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
            (url, None)
        };

        // The payload rides in the state row's `callback_url` slot (the
        // OAuthStateBackend already persists that field for the
        // framework auth flow); `error_callback_url` carries the
        // provider redirect URI. The callback handler decodes both.
        self.state_backend.put(
            &state_token,
            &pylon_auth::OAuthState {
                provider: def.provider.clone(),
                callback_url: payload,
                error_callback_url: callback,
                pkce_verifier,
                handoff_binding: None,
                guest_binding: None,
                expires_at,
            },
        );
        Ok(url)
    }

    /// Exchange the callback code for a token set. Caller is the
    /// HTTP route handler; it consumes the state from the backend
    /// (single-use), validates expiry, then stores the resulting
    /// `StoredConnection` via the provided store callback.
    /// `callback_params` is the callback URL's whole query (Clover
    /// puts the merchant id there).
    ///
    /// `store_fn` writes the row to the `_Connection` entity. The
    /// manager doesn't take a `DataStore` directly to avoid a cycle
    /// with the runtime crate.
    pub fn complete_callback<F>(
        &self,
        code: &str,
        state_token: &str,
        callback_params: &HashMap<String, String>,
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
        let now = now_secs();
        let state = self
            .state_backend
            .take(state_token, now)
            .ok_or_else(|| ConnectionError::AuthFailed("invalid or expired state".into()))?;
        let (user_id, connection_name, post_redirect) =
            decode_state_payload(&state.callback_url)
                .ok_or_else(|| ConnectionError::AuthFailed("state payload malformed".into()))?;
        let def = self.def_or_unknown(&connection_name)?;
        // The owner in the state must still match the declaration: a
        // definition switched from user to app (or back) between the
        // two halves of the flow must not file a user's token as the
        // app's.
        let owner_matches = match def.owner {
            ConnectionOwner::App => user_id == APP_OWNER,
            ConnectionOwner::User => user_id != APP_OWNER,
        };
        if !owner_matches || def.kind != ConnectionKind::Oauth {
            return Err(ConnectionError::AuthFailed(
                "connection changed during the flow".into(),
            ));
        }
        let mut stored = StoredConnection {
            id: stable_id(&user_id, &connection_name),
            user_id: user_id.clone(),
            name: connection_name.clone(),
            provider: def.provider.clone(),
            updated_at: now,
            ..Default::default()
        };
        if let Some(spec) = pylon_auth::integrations::find(&def.provider) {
            let (client_id, client_secret) = client_credentials(&def.provider)?;
            let client = pylon_auth::integrations::Client {
                client_id: &client_id,
                client_secret: &client_secret,
                redirect_uri: &state.error_callback_url,
            };
            let tokens = pylon_auth::integrations::exchange_code(
                spec,
                &client,
                code,
                state.pkce_verifier.as_deref(),
                callback_params,
            )
            .map_err(ConnectionError::AuthFailed)?;
            stored.access_token = tokens.access_token;
            stored.refresh_token = tokens.refresh_token;
            stored.expires_at = tokens.expires_at;
            stored.scope = tokens.scope;
            stored.account_id = tokens.account_id;
            stored.metadata = tokens.metadata.map(|m| m.to_string());
        } else {
            let cfg = self.build_oauth_config(def)?;
            let tokens = cfg
                .exchange_code_full(code)
                .map_err(ConnectionError::AuthFailed)?;
            stored.access_token = tokens.access_token;
            stored.refresh_token = tokens.refresh_token;
            stored.expires_at = tokens.expires_at;
            stored.scope = tokens.scope;
        }
        store_fn(&stored).map_err(ConnectionError::StorageFailed)?;
        Ok(CallbackOutcome {
            user_id,
            connection_name,
            post_redirect,
        })
    }

    /// Build the row for an app connection's tokens handed over by the
    /// broker (the management endpoint's PUT). Only for OAuth app
    /// connections.
    pub fn row_from_broker(
        &self,
        connection_name: &str,
        handed: HandedTokens,
    ) -> Result<StoredConnection, ConnectionError> {
        let def = self.def_or_unknown(connection_name)?;
        if def.owner != ConnectionOwner::App {
            return Err(ConnectionError::Forbidden(format!(
                "Connection \"{connection_name}\" belongs to a user, not the app"
            )));
        }
        if def.kind != ConnectionKind::Oauth {
            return Err(ConnectionError::WrongKind {
                name: def.name.clone(),
                expected: ConnectionKind::Oauth,
            });
        }
        if handed.access_token.is_empty() {
            return Err(ConnectionError::InvalidCredentials(
                "accessToken is required".into(),
            ));
        }
        let metadata = match handed.metadata {
            None | Some(serde_json::Value::Null) => None,
            Some(v @ serde_json::Value::Object(_)) => Some(v.to_string()),
            Some(_) => {
                return Err(ConnectionError::InvalidCredentials(
                    "metadata must be an object".into(),
                ))
            }
        };
        Ok(StoredConnection {
            id: stable_id(APP_OWNER, connection_name),
            user_id: APP_OWNER.into(),
            name: connection_name.into(),
            provider: def.provider.clone(),
            access_token: handed.access_token,
            refresh_token: handed.refresh_token.filter(|s| !s.is_empty()),
            expires_at: handed.expires_at,
            scope: handed.scope.filter(|s| !s.is_empty()),
            updated_at: now_secs(),
            account_id: handed.account_id.filter(|s| !s.is_empty()),
            metadata,
            credentials: None,
        })
    }

    /// Check an API-key connection's fields and build its row. For a
    /// known provider the provider's check call runs first
    /// (`check: false` skips it, for tests).
    pub fn row_from_credentials(
        &self,
        connection_name: &str,
        owner_id: &str,
        credentials: &HashMap<String, String>,
        check: bool,
    ) -> Result<StoredConnection, ConnectionError> {
        if !self.encryption_configured() {
            return Err(ConnectionError::EncryptionRequired);
        }
        let def = self.def_or_unknown(connection_name)?;
        if def.kind != ConnectionKind::ApiKey {
            return Err(ConnectionError::WrongKind {
                name: def.name.clone(),
                expected: ConnectionKind::ApiKey,
            });
        }
        let fields = def.credential_fields();
        let field_refs: Vec<&str> = fields.iter().map(String::as_str).collect();
        pylon_auth::integrations::validate_credential_shape(&field_refs, credentials)
            .map_err(ConnectionError::InvalidCredentials)?;
        let mut account_id = None;
        if let Some(spec) = pylon_auth::integrations::api_key_spec(&def.provider) {
            // A custom field list replaces the provider's, so the
            // provider's check no longer applies.
            if def.fields.is_empty() {
                pylon_auth::integrations::check_known_prefix(spec, credentials)
                    .map_err(ConnectionError::InvalidCredentials)?;
                if check {
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        account_id = pylon_auth::integrations::check_api_key(spec, credentials)
                            .map_err(ConnectionError::CheckFailed)?;
                    }
                }
            }
        }
        let primary = credentials.get(&fields[0]).cloned().unwrap_or_default();
        let mut stored = serde_json::Map::new();
        for f in &fields {
            stored.insert(f.clone(), serde_json::Value::String(credentials[f].clone()));
        }
        Ok(StoredConnection {
            id: stable_id(owner_id, connection_name),
            user_id: owner_id.into(),
            name: connection_name.into(),
            provider: def.provider.clone(),
            access_token: primary,
            refresh_token: None,
            expires_at: None,
            scope: None,
            updated_at: now_secs(),
            account_id,
            metadata: None,
            credentials: Some(serde_json::Value::Object(stored).to_string()),
        })
    }

    /// Fetch a fresh access token for `(owner, connection_name)`.
    ///
    /// Returns the stored access token when it's not about to expire
    /// (>60s remaining). Otherwise refreshes — through the broker for a
    /// brokered app connection, else with the app's own client
    /// credentials — upserts the new token pair via `store_fn`, and
    /// returns the stored row. Refresh-token rotation is respected — the
    /// new refresh token (when the provider sends one) replaces the old.
    ///
    /// `load_fn` reads the current stored row; the manager doesn't
    /// touch DataStore directly to dodge the dep cycle.
    pub fn ensure_fresh_token<L, S>(
        &self,
        user_id: &str,
        connection_name: &str,
        load_fn: L,
        store_fn: S,
    ) -> Result<StoredConnection, ConnectionError>
    where
        L: Fn() -> Result<Option<StoredConnection>, String>,
        S: FnOnce(&StoredConnection) -> Result<(), String>,
    {
        let def = self.def_or_unknown(connection_name)?;
        // Fast path: read first WITHOUT taking the refresh lock.
        // If the token isn't expiring, we don't need to refresh.
        // Avoids serializing every `get` call behind the per-key
        // mutex when the common case is "token is fresh enough".
        let row = load_fn()
            .map_err(ConnectionError::StorageFailed)?
            .ok_or_else(|| ConnectionError::NotConnected {
                name: connection_name.into(),
            })?;
        if !needs_refresh(&row, now_secs()) {
            return Ok(row);
        }

        // Slow path: serialize the entire exchange+persist via the
        // per-(owner, connection) lock. Codex P1 fix — without this,
        // two concurrent refreshes both pass the rate-limit gate,
        // both call the provider, and rotation invalidates the
        // first response between calls.
        let key_lock = self.refresh_lock_for(user_id, connection_name);
        let _guard = key_lock.lock().unwrap_or_else(|p| p.into_inner());

        // Re-load INSIDE the lock — another caller may have just
        // refreshed while we were waiting. If the freshly-loaded
        // row no longer needs refreshing, return it and skip the
        // exchange.
        let mut row = load_fn()
            .map_err(ConnectionError::StorageFailed)?
            .ok_or_else(|| ConnectionError::NotConnected {
                name: connection_name.into(),
            })?;
        let now = now_secs();
        if !needs_refresh(&row, now) {
            return Ok(row);
        }
        let refresh_token = row.refresh_token.clone().ok_or_else(|| {
            ConnectionError::RefreshFailed(
                "no refresh token available — the provider didn't return one. The connection must be linked again.".into(),
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
                        "refresh rate-limited (1/sec per owner+connection)".into(),
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

        let fresh = if def.owner == ConnectionOwner::App && self.broker.is_some() {
            self.broker_refresh(def, &row, &refresh_token)?
        } else if let Some(spec) = pylon_auth::integrations::find(&def.provider) {
            let (client_id, client_secret) = client_credentials(&def.provider)?;
            let redirect = callback_url(&def.name)?;
            let client = pylon_auth::integrations::Client {
                client_id: &client_id,
                client_secret: &client_secret,
                redirect_uri: &redirect,
            };
            let t = pylon_auth::integrations::refresh(spec, &client, &refresh_token)
                .map_err(|e| ConnectionError::RefreshFailed(sanitize_provider_message(&e)))?;
            HandedTokens {
                access_token: t.access_token,
                refresh_token: t.refresh_token,
                expires_at: t.expires_at,
                scope: t.scope,
                account_id: t.account_id,
                metadata: None,
            }
        } else {
            let cfg = self.build_oauth_config(def)?;
            let t = cfg
                .exchange_refresh_token(&refresh_token)
                .map_err(|e| ConnectionError::RefreshFailed(sanitize_provider_message(&e)))?;
            HandedTokens {
                access_token: t.access_token,
                refresh_token: t.refresh_token,
                expires_at: t.expires_at,
                scope: t.scope,
                account_id: None,
                metadata: None,
            }
        };
        row.access_token = fresh.access_token;
        if let Some(rt) = fresh.refresh_token.filter(|s| !s.is_empty()) {
            row.refresh_token = Some(rt);
        }
        row.expires_at = fresh.expires_at;
        if fresh.scope.is_some() {
            row.scope = fresh.scope;
        }
        if fresh.account_id.is_some() {
            row.account_id = fresh.account_id;
        }
        row.updated_at = now;
        // Persist the rotated token pair. Codex P1: if this fails
        // AFTER the provider rotated the refresh token, the owner
        // is silently locked out. Log loudly at error level with
        // structured data so an operator can ask for a re-link;
        // return a distinct error so callers can distinguish
        // "provider rejected" from "we lost the rotated token".
        if let Err(e) = store_fn(&row) {
            tracing::error!(
                user_id = %user_id,
                connection = %connection_name,
                provider = %def.provider,
                error = %e,
                "[connections] REFRESH_PERSISTENCE_LOST: provider rotated refresh token but we failed to persist. The connection must be linked again.",
            );
            return Err(ConnectionError::RefreshFailed(format!(
                "Refresh succeeded but persistence failed; \"{connection_name}\" must be linked again"
            )));
        }
        Ok(row)
    }

    /// Ask the broker for a fresh token pair. The broker checks that the
    /// refresh token is the one it handed this app.
    fn broker_refresh(
        &self,
        def: &ConnectionDef,
        row: &StoredConnection,
        refresh_token: &str,
    ) -> Result<HandedTokens, ConnectionError> {
        let broker = self.broker.as_ref().expect("checked by caller");
        let body = serde_json::json!({
            "connection": def.name,
            "provider": def.provider,
            "accountId": row.account_id,
            "refreshToken": refresh_token,
        })
        .to_string();
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(10))
            .timeout_read(std::time::Duration::from_secs(20))
            .redirects(0)
            .build();
        let result = agent
            .post(&broker.url)
            .set("Authorization", &format!("Bearer {}", broker.token))
            .set("Content-Type", "application/json")
            .send_string(&body);
        let text = match result {
            Ok(resp) => resp
                .into_string()
                .map_err(|e| ConnectionError::RefreshFailed(format!("broker: read body: {e}")))?,
            Err(ureq::Error::Status(code, resp)) => {
                let detail = resp
                    .into_string()
                    .ok()
                    .and_then(|b| serde_json::from_str::<serde_json::Value>(&b).ok())
                    .and_then(|v| {
                        v.pointer("/error/message")
                            .or_else(|| v.get("message"))
                            .and_then(|m| m.as_str())
                            .map(String::from)
                    })
                    .unwrap_or_default();
                return Err(ConnectionError::RefreshFailed(sanitize_provider_message(
                    &format!("broker refused the refresh (HTTP {code}) {detail}"),
                )));
            }
            Err(e) => return Err(ConnectionError::RefreshFailed(format!("broker: {e}"))),
        };
        let handed: HandedTokens = serde_json::from_str(&text).map_err(|_| {
            ConnectionError::RefreshFailed("broker returned an unreadable response".into())
        })?;
        if handed.access_token.is_empty() {
            return Err(ConnectionError::RefreshFailed(
                "broker returned no access token".into(),
            ));
        }
        Ok(handed)
    }

    // -----------------------------------------------------------------------
    // Internals
    // -----------------------------------------------------------------------

    fn build_oauth_config(
        &self,
        def: &ConnectionDef,
    ) -> Result<pylon_auth::OAuthConfig, ConnectionError> {
        let (client_id, client_secret) = client_credentials(&def.provider)?;
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

/// Tokens handed to the app by the broker: the management endpoint's
/// PUT body for an OAuth app connection, and the broker's refresh
/// response. `Debug` leaves out the tokens.
#[derive(Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandedTokens {
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_at: Option<u64>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

impl std::fmt::Debug for HandedTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HandedTokens")
            .field("expires_at", &self.expires_at)
            .field("account_id", &self.account_id)
            .finish_non_exhaustive()
    }
}

fn client_credentials(provider: &str) -> Result<(String, String), ConnectionError> {
    let client_id = std::env::var(client_id_env(provider))
        .ok()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ConnectionError::ProviderNotConfigured {
            provider: provider.into(),
            missing_env: client_id_env(provider),
        })?;
    let client_secret = std::env::var(client_secret_env(provider))
        .ok()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ConnectionError::ProviderNotConfigured {
            provider: provider.into(),
            missing_env: client_secret_env(provider),
        })?;
    Ok((client_id, client_secret))
}

fn needs_refresh(row: &StoredConnection, now: u64) -> bool {
    match row.expires_at {
        Some(exp) => exp.saturating_sub(now) <= 60,
        None => false,
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
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
        max_length: None,
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
            field(
                "accountId",
                "string",
                FieldOpts {
                    server_only: true,
                    optional: true,
                    ..FieldOpts::default()
                },
            ),
            field(
                "metadata",
                "string",
                FieldOpts {
                    server_only: true,
                    optional: true,
                    ..FieldOpts::default()
                },
            ),
            field(
                "credentials",
                "string",
                FieldOpts {
                    server_only: true,
                    encrypted: true,
                    optional: true,
                },
            ),
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

    /// Tests that set process env hold this so they do not race.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn disconnect_waits_for_refresh_and_keeps_other_users_independent() {
        use std::sync::mpsc;
        use std::time::Duration;
        let manager = ConnectionManager::new(
            vec![ConnectionDef {
                name: "google".into(),
                provider: "google".into(),
                scopes: String::new(),
                owner: ConnectionOwner::User,
                kind: ConnectionKind::Oauth,
                fields: vec![],
            }],
            None,
            Arc::new(pylon_auth::InMemoryOAuthBackend::new()),
        );
        let row = Mutex::new(Some("old token"));
        let refresh_lock = manager.refresh_lock_for("user-1", "google");
        let refreshing = refresh_lock.lock().unwrap();
        std::thread::scope(|scope| {
            let (started_tx, started_rx) = mpsc::channel();
            let (deleted_tx, deleted_rx) = mpsc::channel();
            let manager = &manager;
            let row = &row;
            let disconnect = scope.spawn(move || {
                started_tx.send(()).unwrap();
                manager
                    .disconnect("user-1", "google", || {
                        *row.lock().unwrap() = None;
                        deleted_tx.send(()).unwrap();
                        Ok(())
                    })
                    .unwrap();
            });
            started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            // A different user's operation does not wait for this refresh.
            manager.disconnect("user-2", "google", || Ok(())).unwrap();
            assert!(deleted_rx.recv_timeout(Duration::from_millis(50)).is_err());
            *row.lock().unwrap() = Some("refreshed token");
            drop(refreshing);
            disconnect.join().unwrap();
            deleted_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        });
        assert!(row.lock().unwrap().is_none());
        // A refresh started after disconnect cannot recreate the row.
        let result = manager.ensure_fresh_token(
            "user-1",
            "google",
            || Ok(None),
            |_| panic!("deleted row must not be saved"),
        );
        assert!(matches!(result, Err(ConnectionError::NotConnected { .. })));
        assert!(matches!(
            manager.disconnect("user-1", "google", || Err("storage failed".into())),
            Err(ConnectionError::StorageFailed(_))
        ));
    }

    #[test]
    fn unknown_disconnects_delete_stale_rows_without_retaining_locks() {
        let manager = ConnectionManager::new(
            vec![],
            None,
            Arc::new(pylon_auth::InMemoryOAuthBackend::new()),
        );
        let mut deleted = 0;
        for i in 0..1000 {
            manager
                .disconnect("user", &format!("removed-{i}"), || {
                    deleted += 1;
                    Ok(())
                })
                .unwrap();
        }
        assert_eq!(deleted, 1000);
        assert!(manager.refresh_locks.lock().unwrap().is_empty());
    }

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
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
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
        let creds = e.fields.iter().find(|f| f.name == "credentials").unwrap();
        assert!(creds.encrypted && creds.server_only && creds.optional);
        // Columns added after the first release must be optional, or the
        // column upgrade on an existing table fails.
        for name in ["accountId", "metadata", "credentials"] {
            assert!(
                e.fields.iter().find(|f| f.name == name).unwrap().optional,
                "{name}"
            );
        }
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

    fn def(
        name: &str,
        provider: &str,
        owner: ConnectionOwner,
        kind: ConnectionKind,
    ) -> ConnectionDef {
        ConnectionDef {
            name: name.into(),
            provider: provider.into(),
            scopes: String::new(),
            owner,
            kind,
            fields: vec![],
        }
    }

    fn key() -> Option<EncryptionKey> {
        Some(
            EncryptionKey::from_raw(
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            )
            .unwrap(),
        )
    }

    fn manager(defs: Vec<ConnectionDef>, broker: Option<Broker>) -> ConnectionManager {
        ConnectionManager::with_broker(
            defs,
            key(),
            Arc::new(pylon_auth::InMemoryOAuthBackend::new()),
            broker,
        )
    }

    #[test]
    fn app_connections_read_by_anyone_changed_by_admins() {
        let m = manager(
            vec![
                def(
                    "square",
                    "square",
                    ConnectionOwner::App,
                    ConnectionKind::Oauth,
                ),
                def(
                    "google",
                    "google",
                    ConnectionOwner::User,
                    ConnectionKind::Oauth,
                ),
            ],
            None,
        );
        // Any caller reads an app connection, signed in or not.
        assert_eq!(
            m.owner_for("square", None, false, false).unwrap(),
            APP_OWNER
        );
        assert_eq!(
            m.owner_for("square", Some("u1"), false, false).unwrap(),
            APP_OWNER
        );
        // Only an admin changes it.
        assert!(matches!(
            m.owner_for("square", Some("u1"), false, true),
            Err(ConnectionError::Forbidden(_))
        ));
        assert_eq!(m.owner_for("square", None, true, true).unwrap(), APP_OWNER);
        // A user connection is the caller's own, and needs a caller.
        assert_eq!(
            m.owner_for("google", Some("u1"), false, true).unwrap(),
            "u1"
        );
        assert!(m.owner_for("google", None, true, false).is_err());
        // A caller id shaped like the app owner never reaches app rows.
        assert!(m
            .owner_for("google", Some(APP_OWNER), false, false)
            .is_err());
        assert!(matches!(
            m.owner_for("nope", Some("u1"), true, true),
            Err(ConnectionError::UnknownConnection { .. })
        ));
    }

    #[test]
    fn credentials_are_checked_before_they_are_stored() {
        let mut custom = def("pos", "toast", ConnectionOwner::App, ConnectionKind::ApiKey);
        custom.fields = vec!["token".into(), "site".into()];
        let m = manager(
            vec![
                def(
                    "stripe-key",
                    "stripe",
                    ConnectionOwner::App,
                    ConnectionKind::ApiKey,
                ),
                def(
                    "mystery",
                    "acme",
                    ConnectionOwner::User,
                    ConnectionKind::ApiKey,
                ),
                def(
                    "square",
                    "square",
                    ConnectionOwner::App,
                    ConnectionKind::Oauth,
                ),
                custom,
            ],
            None,
        );
        let creds = |pairs: &[(&str, &str)]| -> HashMap<String, String> {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        // A known provider's prefix rule runs before any network call.
        assert!(matches!(
            m.row_from_credentials(
                "stripe-key",
                APP_OWNER,
                &creds(&[("secretKey", "pk_live_x")]),
                false
            ),
            Err(ConnectionError::InvalidCredentials(_))
        ));
        let row = m
            .row_from_credentials(
                "stripe-key",
                APP_OWNER,
                &creds(&[("secretKey", "rk_live_x")]),
                false,
            )
            .unwrap();
        assert_eq!(row.access_token, "rk_live_x");
        assert_eq!(row.id, stable_id(APP_OWNER, "stripe-key"));
        // An unknown provider gets the single apiKey field.
        let row = m
            .row_from_credentials("mystery", "u1", &creds(&[("apiKey", "k")]), false)
            .unwrap();
        assert_eq!(row.credentials.as_deref(), Some(r#"{"apiKey":"k"}"#));
        // Declared fields replace the provider's, and the first declared
        // one is the access token.
        let row = m
            .row_from_credentials(
                "pos",
                APP_OWNER,
                &creds(&[("site", "s1"), ("token", "t1")]),
                false,
            )
            .unwrap();
        assert_eq!(row.access_token, "t1");
        let stored: serde_json::Value =
            serde_json::from_str(row.credentials.as_deref().unwrap()).unwrap();
        assert_eq!(stored, serde_json::json!({"token": "t1", "site": "s1"}));
        assert!(m
            .row_from_credentials("pos", APP_OWNER, &creds(&[("token", "t1")]), false)
            .is_err());
        // OAuth connections take no keys.
        assert!(matches!(
            m.row_from_credentials("square", APP_OWNER, &creds(&[("apiKey", "k")]), false),
            Err(ConnectionError::WrongKind { .. })
        ));
    }

    #[test]
    fn broker_tokens_only_for_app_oauth_connections() {
        let m = manager(
            vec![
                def(
                    "square",
                    "square",
                    ConnectionOwner::App,
                    ConnectionKind::Oauth,
                ),
                def(
                    "google",
                    "google",
                    ConnectionOwner::User,
                    ConnectionKind::Oauth,
                ),
                def(
                    "toast",
                    "toast",
                    ConnectionOwner::App,
                    ConnectionKind::ApiKey,
                ),
            ],
            None,
        );
        let handed = |token: &str| HandedTokens {
            access_token: token.into(),
            refresh_token: Some("r".into()),
            expires_at: Some(10),
            account_id: Some("M1".into()),
            metadata: Some(serde_json::json!({"dc": "us1"})),
            ..Default::default()
        };
        let row = m.row_from_broker("square", handed("a")).unwrap();
        assert_eq!(row.user_id, APP_OWNER);
        assert_eq!(row.account_id.as_deref(), Some("M1"));
        assert_eq!(row.metadata.as_deref(), Some(r#"{"dc":"us1"}"#));
        assert!(matches!(
            m.row_from_broker("google", handed("a")),
            Err(ConnectionError::Forbidden(_))
        ));
        assert!(matches!(
            m.row_from_broker("toast", handed("a")),
            Err(ConnectionError::WrongKind { .. })
        ));
        assert!(m.row_from_broker("square", handed("")).is_err());
        let mut bad = handed("a");
        bad.metadata = Some(serde_json::json!(["x"]));
        assert!(m.row_from_broker("square", bad).is_err());
    }

    #[test]
    fn brokered_app_connections_do_not_start_a_local_flow() {
        let broker = Broker {
            url: "https://hub.example/refresh".into(),
            token: "t".into(),
        };
        let m = manager(
            vec![
                def(
                    "square",
                    "square",
                    ConnectionOwner::App,
                    ConnectionKind::Oauth,
                ),
                def(
                    "toast",
                    "toast",
                    ConnectionOwner::App,
                    ConnectionKind::ApiKey,
                ),
            ],
            Some(broker),
        );
        assert!(matches!(
            m.build_auth_url("square", APP_OWNER, None),
            Err(ConnectionError::Managed { .. })
        ));
        assert!(matches!(
            m.build_auth_url("toast", APP_OWNER, None),
            Err(ConnectionError::WrongKind { .. })
        ));
    }

    #[test]
    fn local_app_flow_uses_the_integration_spec_and_binds_the_app_owner() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        std::env::set_var("PYLON_PUBLIC_URL", "https://shop.example");
        std::env::set_var("PYLON_OAUTH_CALENDLY_CLIENT_ID", "cal-id");
        std::env::set_var("PYLON_OAUTH_CALENDLY_CLIENT_SECRET", "cal-secret");
        let backend = Arc::new(pylon_auth::InMemoryOAuthBackend::new());
        let m = ConnectionManager::with_broker(
            vec![def(
                "calendly",
                "calendly",
                ConnectionOwner::App,
                ConnectionKind::Oauth,
            )],
            key(),
            backend.clone(),
            None,
        );
        let url = m
            .build_auth_url("calendly", APP_OWNER, Some("/admin"))
            .unwrap();
        std::env::remove_var("PYLON_OAUTH_CALENDLY_CLIENT_ID");
        std::env::remove_var("PYLON_OAUTH_CALENDLY_CLIENT_SECRET");
        assert!(url.starts_with("https://auth.calendly.com/oauth/authorize?"));
        assert!(url.contains(
            "redirect_uri=https%3A%2F%2Fshop.example%2Fapi%2Fconnections%2Fcalendly%2Fcallback"
        ));
        assert!(url.contains("code_challenge="));
        assert!(!url.contains("cal-secret"));
        let state = url
            .split('&')
            .find_map(|p| p.strip_prefix("state="))
            .unwrap();
        let row = pylon_auth::OAuthStateBackend::take(backend.as_ref(), state, 0).unwrap();
        let (owner, name, redirect) = decode_state_payload(&row.callback_url).unwrap();
        assert_eq!(
            (owner.as_str(), name.as_str(), redirect.as_str()),
            (APP_OWNER, "calendly", "/admin")
        );
        assert!(
            row.pkce_verifier.is_some(),
            "the PKCE verifier waits in the state row"
        );
    }

    #[test]
    fn refresh_goes_to_the_broker_for_app_connections() {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut headers = Vec::new();
            let mut len = 0usize;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap();
                }
                headers.push(line);
            }
            let mut body = vec![0u8; len];
            reader.read_exact(&mut body).unwrap();
            let reply = r#"{"accessToken":"new-access","refreshToken":"new-refresh","expiresAt":4000000000}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                reply.len(),
                reply
            )
            .unwrap();
            (headers, String::from_utf8(body).unwrap())
        });
        let broker = Broker {
            url: format!("http://127.0.0.1:{port}/refresh"),
            token: "broker-token".into(),
        };
        let m = manager(
            vec![def(
                "square",
                "square",
                ConnectionOwner::App,
                ConnectionKind::Oauth,
            )],
            Some(broker),
        );
        let stored = Mutex::new(StoredConnection {
            id: stable_id(APP_OWNER, "square"),
            user_id: APP_OWNER.into(),
            name: "square".into(),
            provider: "square".into(),
            access_token: "old-access".into(),
            refresh_token: Some("old-refresh".into()),
            expires_at: Some(1),
            account_id: Some("M1".into()),
            ..Default::default()
        });
        let row = m
            .ensure_fresh_token(
                APP_OWNER,
                "square",
                || Ok(Some(stored.lock().unwrap().clone())),
                |r| {
                    *stored.lock().unwrap() = r.clone();
                    Ok(())
                },
            )
            .unwrap();
        let (headers, body) = server.join().unwrap();
        assert!(headers
            .iter()
            .any(|h| h.trim() == "Authorization: Bearer broker-token"));
        let sent: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(sent["refreshToken"], "old-refresh");
        assert_eq!(sent["accountId"], "M1");
        assert_eq!(row.access_token, "new-access");
        assert_eq!(
            stored.lock().unwrap().refresh_token.as_deref(),
            Some("new-refresh")
        );
        assert_eq!(stored.lock().unwrap().account_id.as_deref(), Some("M1"));
    }

    #[test]
    fn broker_url_must_be_https_or_loopback() {
        assert!(broker_url_is_safe("https://cloud.example/api/fn/x"));
        assert!(broker_url_is_safe("http://127.0.0.1:4321/x"));
        assert!(broker_url_is_safe("http://localhost/x"));
        assert!(!broker_url_is_safe("http://cloud.example/x"));
        assert!(!broker_url_is_safe("http://localhost.evil.example/x"));
        assert!(!broker_url_is_safe("ftp://x"));
        assert!(!broker_url_is_safe("https://"));
    }

    #[test]
    fn stored_connection_debug_hides_secrets() {
        let row = StoredConnection {
            access_token: "very-secret".into(),
            refresh_token: Some("also-secret".into()),
            credentials: Some("{\"k\":\"key-secret\"}".into()),
            ..Default::default()
        };
        let dbg = format!("{row:?}");
        assert!(!dbg.contains("secret"));
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
