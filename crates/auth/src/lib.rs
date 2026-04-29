pub mod cookie;
pub mod email;
pub mod password;

pub use cookie::{extract_token as extract_session_cookie, CookieConfig, SameSite};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Auth context — the identity available to runtime operations
// ---------------------------------------------------------------------------

/// The auth context for a request. Represents who is making the request.
///
/// **Do NOT derive `Deserialize` on this type.** If the server ever parses an
/// `AuthContext` from client-supplied JSON, a client can set `is_admin=true`
/// or add roles and bypass every policy. Identity must come from
/// server-minted sessions (`Session::to_auth_context`) or explicit
/// constructors, never from deserialization.
///
/// `Serialize` is safe because sending the resolved context BACK to the
/// client exposes nothing the server didn't already decide.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuthContext {
    /// The authenticated user ID, or None for public/anonymous access.
    /// For guest contexts this is `Some(guest_id)` — a stable
    /// anonymous identifier, NOT a real user.
    pub user_id: Option<String>,
    /// Whether this is an admin context (bypasses policies).
    pub is_admin: bool,
    /// True for `AuthContext::guest()` — anonymous-with-stable-id, used
    /// for cart state and similar pre-login persistence. Routes guarded
    /// by `AuthMode::User` reject guests; only `is_authenticated()` ==
    /// "real signed-in user" should pass auth-required gates.
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_guest: bool,
    /// Roles granted to this user. Empty for anonymous.
    pub roles: Vec<String>,
    /// Active tenant id (for multi-tenant apps). Set when the user has
    /// selected an organization for the current session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
}

fn is_false(b: &bool) -> bool {
    !b
}

impl AuthContext {
    /// Create an anonymous/public auth context.
    pub fn anonymous() -> Self {
        Self {
            user_id: None,
            is_admin: false,
            is_guest: false,
            roles: Vec::new(),
            tenant_id: None,
        }
    }

    /// Create an authenticated auth context.
    pub fn authenticated(user_id: String) -> Self {
        Self {
            user_id: Some(user_id),
            is_admin: false,
            is_guest: false,
            roles: Vec::new(),
            tenant_id: None,
        }
    }

    /// Create a guest auth context with a persistent anonymous ID.
    /// Guests carry an opaque stable id (cart/session continuity) but
    /// are NOT considered authenticated — `is_authenticated()` returns
    /// false and `AuthMode::User` rejects them.
    pub fn guest(guest_id: String) -> Self {
        Self {
            user_id: Some(guest_id),
            is_admin: false,
            is_guest: true,
            roles: Vec::new(),
            tenant_id: None,
        }
    }

    /// Create an admin auth context that bypasses all policies.
    pub fn admin() -> Self {
        Self {
            user_id: Some("__admin__".into()),
            is_admin: true,
            is_guest: false,
            roles: vec!["admin".into()],
            tenant_id: None,
        }
    }

    /// Convenience: build a user context from a user id.
    pub fn user(user_id: String) -> Self {
        Self::authenticated(user_id)
    }

    /// Active tenant id (None when the user hasn't selected an org).
    pub fn tenant_id(&self) -> Option<&str> {
        self.tenant_id.as_deref()
    }

    /// Attach a tenant id to the context (chainable).
    pub fn with_tenant(mut self, tenant_id: String) -> Self {
        self.tenant_id = Some(tenant_id);
        self
    }

    /// Check if this context represents an authenticated user.
    /// Guests intentionally return `false` — they have a stable anonymous
    /// id but never gain user-level access.
    pub fn is_authenticated(&self) -> bool {
        self.user_id.is_some() && !self.is_guest
    }

    /// Check if the user has a specific role. Admins have every role implicitly.
    pub fn has_role(&self, role: &str) -> bool {
        self.is_admin || self.roles.iter().any(|r| r == role)
    }

    /// Check if the user has ANY of the given roles.
    pub fn has_any_role(&self, roles: &[&str]) -> bool {
        self.is_admin || roles.iter().any(|r| self.has_role(r))
    }

    /// Attach roles to the context (chainable).
    pub fn with_roles(mut self, roles: Vec<String>) -> Self {
        self.roles = roles;
        self
    }
}

// ---------------------------------------------------------------------------
// Constant-time comparison
// ---------------------------------------------------------------------------

/// Constant-time byte comparison to prevent timing attacks.
///
/// The length check leaks whether the two slices are the same length, but the
/// content comparison always examines every byte regardless of where (or
/// whether) a mismatch occurs.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut result: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        result |= x ^ y;
    }
    result == 0
}

// ---------------------------------------------------------------------------
// Auth mode — matches the route "auth" field values
// ---------------------------------------------------------------------------

/// The auth mode declared on a route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthMode {
    /// Anyone can access.
    Public,
    /// Only authenticated users can access.
    User,
}

impl AuthMode {
    /// Parse from the manifest auth string.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "public" => Some(AuthMode::Public),
            "user" => Some(AuthMode::User),
            _ => None,
        }
    }

    /// Check if the given auth context satisfies this mode.
    pub fn check(&self, ctx: &AuthContext) -> bool {
        match self {
            AuthMode::Public => true,
            AuthMode::User => ctx.is_authenticated(),
        }
    }
}

// ---------------------------------------------------------------------------
// Session — opaque token session
// ---------------------------------------------------------------------------

/// A session token and its associated user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub token: String,
    pub user_id: String,
    /// Unix epoch seconds at which this session expires. 0 = never.
    #[serde(default)]
    pub expires_at: u64,
    /// Optional user-agent / device tag recorded at session creation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    /// Unix epoch seconds when the session was created.
    #[serde(default)]
    pub created_at: u64,
    /// Active tenant id (selected organization). Set via
    /// `/api/auth/select-org`. Flows into `AuthContext.tenant_id` which
    /// powers row-scoped policies like `data.orgId == auth.tenantId`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
}

impl Session {
    /// Default session lifetime: 30 days.
    pub const DEFAULT_LIFETIME_SECS: u64 = 30 * 24 * 60 * 60;

    /// Create a new session with a generated token and default 30-day expiry.
    pub fn new(user_id: String) -> Self {
        let now = now_secs();
        Self {
            token: generate_token(),
            user_id,
            expires_at: now.saturating_add(Self::DEFAULT_LIFETIME_SECS),
            device: None,
            created_at: now,
            tenant_id: None,
        }
    }

    /// Create a session with a specific lifetime.
    pub fn with_lifetime(user_id: String, lifetime_secs: u64) -> Self {
        let now = now_secs();
        Self {
            token: generate_token(),
            user_id,
            expires_at: if lifetime_secs == 0 {
                0
            } else {
                now.saturating_add(lifetime_secs)
            },
            device: None,
            created_at: now,
            tenant_id: None,
        }
    }

    /// Convert this session to an auth context, carrying the selected
    /// tenant so row-scoped policies see `auth.tenantId`.
    pub fn to_auth_context(&self) -> AuthContext {
        let ctx = AuthContext::authenticated(self.user_id.clone());
        match &self.tenant_id {
            Some(t) => ctx.with_tenant(t.clone()),
            None => ctx,
        }
    }

    /// Returns true if the session has passed its expires_at time.
    /// Boundary is inclusive (`>=`) to match the rest of the codebase
    /// (`magic_codes.expires_at <= now`, `oauth_state.expires_at <= now`).
    pub fn is_expired(&self) -> bool {
        self.expires_at != 0 && now_secs() >= self.expires_at
    }
}

fn now_secs() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ---------------------------------------------------------------------------
// OAuth provider config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthConfig {
    pub provider: String,
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
}

impl OAuthConfig {
    /// Generate the authorization URL for the provider.
    ///
    /// Callers MUST append a `&state=<random>` parameter and validate it in the
    /// callback to prevent CSRF attacks. See `OAuthStateStore` for a minimal
    /// implementation.
    pub fn auth_url(&self) -> String {
        match self.provider.as_str() {
            "google" => format!(
                "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&redirect_uri={}&response_type=code&scope=openid%20email%20profile",
                self.client_id, self.redirect_uri
            ),
            "github" => format!(
                "https://github.com/login/oauth/authorize?client_id={}&redirect_uri={}&scope=user:email",
                self.client_id, self.redirect_uri
            ),
            _ => String::new(),
        }
    }

    /// Generate the authorization URL with a CSRF state parameter attached.
    pub fn auth_url_with_state(&self, state: &str) -> String {
        let base = self.auth_url();
        if base.is_empty() {
            return base;
        }
        format!("{}&state={}", base, state)
    }

    /// Generate the token exchange URL.
    pub fn token_url(&self) -> &str {
        match self.provider.as_str() {
            "google" => "https://oauth2.googleapis.com/token",
            "github" => "https://github.com/login/oauth/access_token",
            _ => "",
        }
    }

    /// URL for the userinfo endpoint, which returns the authenticated user's profile.
    pub fn userinfo_url(&self) -> &str {
        match self.provider.as_str() {
            "google" => "https://www.googleapis.com/oauth2/v3/userinfo",
            "github" => "https://api.github.com/user",
            _ => "",
        }
    }

    /// Exchange an authorization code for the full token set
    /// (`access_token`, optional `refresh_token`, optional `id_token`,
    /// `expires_in`, `scope`). The longer struct is what the
    /// account-store needs to persist; the legacy
    /// [`OAuthConfig::exchange_code`] returns just the access token for
    /// callers that don't care.
    pub fn exchange_code_full(&self, code: &str) -> Result<TokenSet, String> {
        let body = match self.provider.as_str() {
            "google" => format!(
                "code={code}&client_id={}&client_secret={}&redirect_uri={}&grant_type=authorization_code",
                url_encode(&self.client_id),
                url_encode(&self.client_secret),
                url_encode(&self.redirect_uri)
            ),
            "github" => format!(
                "code={code}&client_id={}&client_secret={}&redirect_uri={}",
                url_encode(&self.client_id),
                url_encode(&self.client_secret),
                url_encode(&self.redirect_uri)
            ),
            _ => return Err(format!("unknown OAuth provider: {}", self.provider)),
        };

        let out = http_post_form(self.token_url(), &body, self.provider.as_str() == "github")?;
        parse_token_response(&out)
    }

    /// Exchange an authorization code for an access token. Thin wrapper
    /// around [`OAuthConfig::exchange_code_full`] for callers that only
    /// need the access token (existing pre-account-store call sites).
    pub fn exchange_code(&self, code: &str) -> Result<String, String> {
        let body = match self.provider.as_str() {
            "google" => format!(
                "code={code}&client_id={}&client_secret={}&redirect_uri={}&grant_type=authorization_code",
                url_encode(&self.client_id),
                url_encode(&self.client_secret),
                url_encode(&self.redirect_uri)
            ),
            "github" => format!(
                "code={code}&client_id={}&client_secret={}&redirect_uri={}",
                url_encode(&self.client_id),
                url_encode(&self.client_secret),
                url_encode(&self.redirect_uri)
            ),
            _ => return Err(format!("unknown OAuth provider: {}", self.provider)),
        };

        let out = http_post_form(self.token_url(), &body, self.provider.as_str() == "github")?;
        extract_access_token(&out)
    }

    /// Fetch the authenticated user's email + display name using an access token.
    /// Returns `(email, display_name)`. Use [`OAuthConfig::fetch_userinfo_full`]
    /// when you also need the provider-stable account ID for account
    /// linking — the (`provider`, `provider_account_id`) pair is what
    /// keeps a renamed-email user matched to the same row.
    pub fn fetch_userinfo(&self, access_token: &str) -> Result<(String, Option<String>), String> {
        let info = self.fetch_userinfo_full(access_token)?;
        Ok((info.email, info.name))
    }

    /// Fetch the authenticated user's full identity info — email + name +
    /// the provider-stable account ID (Google's `sub`, GitHub's `id`).
    /// `provider_account_id` is what the account-store keys on, NOT the
    /// email; otherwise a user changing their Google address would orphan
    /// their existing pylon account.
    pub fn fetch_userinfo_full(&self, access_token: &str) -> Result<UserInfo, String> {
        let out = http_get_bearer(self.userinfo_url(), access_token)?;
        let parsed: serde_json::Value =
            serde_json::from_str(&out).map_err(|e| format!("userinfo not valid JSON: {e}"))?;
        match self.provider.as_str() {
            "google" => {
                let email = parsed
                    .get("email")
                    .and_then(|v| v.as_str())
                    .ok_or("no email in userinfo")?
                    .to_string();
                let name = parsed
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let provider_account_id = parsed
                    .get("sub")
                    .and_then(|v| v.as_str())
                    .ok_or("no sub in userinfo")?
                    .to_string();
                Ok(UserInfo {
                    provider: self.provider.clone(),
                    provider_account_id,
                    email,
                    name,
                })
            }
            "github" => {
                let name = parsed
                    .get("name")
                    .and_then(|v| v.as_str())
                    .or_else(|| parsed.get("login").and_then(|v| v.as_str()))
                    .map(String::from);
                let email = parsed
                    .get("email")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                // GitHub may return a null email if the user hasn't published one;
                // in that case the caller should hit /user/emails with the same token.
                let email = email
                    .or_else(|| fetch_github_primary_email(access_token).ok())
                    .ok_or("no accessible email on GitHub account")?;
                // GitHub's `id` field is a numeric user ID — the stable
                // account identifier even if the user renames themselves.
                let provider_account_id = parsed
                    .get("id")
                    .map(|v| {
                        v.as_i64()
                            .map(|n| n.to_string())
                            .or_else(|| v.as_str().map(String::from))
                            .unwrap_or_default()
                    })
                    .filter(|s| !s.is_empty())
                    .ok_or("no id in userinfo")?;
                Ok(UserInfo {
                    provider: self.provider.clone(),
                    provider_account_id,
                    email,
                    name,
                })
            }
            _ => Err(format!("unknown provider: {}", self.provider)),
        }
    }
}

/// Resolved identity returned by [`OAuthConfig::fetch_userinfo_full`].
/// `provider_account_id` is the provider-stable subject id (Google `sub`,
/// GitHub numeric `id`) — what the account store keys on so a renamed
/// email doesn't orphan the pylon account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserInfo {
    pub provider: String,
    pub provider_account_id: String,
    pub email: String,
    pub name: Option<String>,
}

/// Token bundle returned by [`OAuthConfig::exchange_code_full`]. Stored
/// on the matching `Account` row so `refresh_token` is available for
/// silent re-auth and `expires_at` is checked before each provider call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenSet {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    /// Unix epoch seconds at which the access token expires. `None` when
    /// the provider didn't return `expires_in` (GitHub's classic OAuth
    /// app tokens are non-expiring).
    pub expires_at: Option<u64>,
    pub scope: Option<String>,
}

fn parse_token_response(body: &str) -> Result<TokenSet, String> {
    // Most providers return JSON; GitHub Classic apps return form-urlencoded
    // unless you ask with Accept: application/json (which we do).
    let json: serde_json::Value = serde_json::from_str(body).unwrap_or_else(|_| {
        // Fall back to form-urlencoded: access_token=...&scope=...&token_type=...
        let mut map = serde_json::Map::new();
        for pair in body.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                map.insert(k.to_string(), serde_json::Value::String(v.to_string()));
            }
        }
        serde_json::Value::Object(map)
    });

    let access_token = json
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("no access_token in token response: {body}"))?
        .to_string();
    let refresh_token = json
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .map(String::from);
    let id_token = json
        .get("id_token")
        .and_then(|v| v.as_str())
        .map(String::from);
    let expires_at = json
        .get("expires_in")
        .and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        })
        .map(|secs| now_secs().saturating_add(secs));
    let scope = json.get("scope").and_then(|v| v.as_str()).map(String::from);
    Ok(TokenSet {
        access_token,
        refresh_token,
        id_token,
        expires_at,
        scope,
    })
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Timeout for OAuth / userinfo HTTP calls. Short enough that a hung
/// provider doesn't block a login indefinitely; long enough to absorb
/// typical internet latency.
const HTTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

fn ureq_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(HTTP_TIMEOUT)
        .timeout_read(HTTP_TIMEOUT)
        .timeout_write(HTTP_TIMEOUT)
        .user_agent("pylon/0.1")
        .build()
}

fn http_post_form(url: &str, body: &str, accept_json: bool) -> Result<String, String> {
    let agent = ureq_agent();
    let mut req = agent
        .post(url)
        .set("Content-Type", "application/x-www-form-urlencoded");
    if accept_json {
        req = req.set("Accept", "application/json");
    }
    match req.send_string(body) {
        Ok(resp) => resp.into_string().map_err(|e| format!("read body: {e}")),
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(format!("HTTP {code}: {body}"))
        }
        Err(e) => Err(format!("HTTP error: {e}")),
    }
}

fn http_get_bearer(url: &str, token: &str) -> Result<String, String> {
    let agent = ureq_agent();
    match agent
        .get(url)
        .set("Authorization", &format!("Bearer {token}"))
        .set("Accept", "application/json")
        .call()
    {
        Ok(resp) => resp.into_string().map_err(|e| format!("read body: {e}")),
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(format!("HTTP {code}: {body}"))
        }
        Err(e) => Err(format!("HTTP error: {e}")),
    }
}

fn fetch_github_primary_email(token: &str) -> Result<String, String> {
    let out = http_get_bearer("https://api.github.com/user/emails", token)?;
    let emails: serde_json::Value =
        serde_json::from_str(&out).map_err(|e| format!("emails not JSON: {e}"))?;
    emails
        .as_array()
        .and_then(|arr| {
            arr.iter()
                .find(|e| {
                    e.get("primary").and_then(|v| v.as_bool()).unwrap_or(false)
                        && e.get("verified").and_then(|v| v.as_bool()).unwrap_or(false)
                })
                .and_then(|e| e.get("email").and_then(|v| v.as_str()).map(String::from))
        })
        .ok_or_else(|| "no primary verified email on GitHub".into())
}

fn extract_access_token(body: &str) -> Result<String, String> {
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(t) = json.get("access_token").and_then(|v| v.as_str()) {
            return Ok(t.to_string());
        }
    }
    // GitHub can return url-encoded: access_token=...&scope=...&token_type=bearer
    for pair in body.split('&') {
        if let Some(val) = pair.strip_prefix("access_token=") {
            return Ok(val.to_string());
        }
    }
    Err(format!("no access_token in token response: {body}"))
}

/// OAuth provider registry.
pub struct OAuthRegistry {
    providers: std::collections::HashMap<String, OAuthConfig>,
}

impl Default for OAuthRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl OAuthRegistry {
    pub fn new() -> Self {
        Self {
            providers: std::collections::HashMap::new(),
        }
    }

    pub fn register(&mut self, config: OAuthConfig) {
        self.providers.insert(config.provider.clone(), config);
    }

    pub fn get(&self, provider: &str) -> Option<&OAuthConfig> {
        self.providers.get(provider)
    }

    /// Build from environment variables.
    /// Looks for PYLON_OAUTH_GOOGLE_CLIENT_ID, etc.
    pub fn from_env() -> Self {
        let mut reg = Self::new();

        // Google
        if let (Ok(id), Ok(secret)) = (
            std::env::var("PYLON_OAUTH_GOOGLE_CLIENT_ID"),
            std::env::var("PYLON_OAUTH_GOOGLE_CLIENT_SECRET"),
        ) {
            reg.register(OAuthConfig {
                provider: "google".into(),
                client_id: id,
                client_secret: secret,
                redirect_uri: std::env::var("PYLON_OAUTH_GOOGLE_REDIRECT")
                    .unwrap_or_else(|_| "http://localhost:3000/api/auth/callback/google".into()),
            });
        }

        // GitHub
        if let (Ok(id), Ok(secret)) = (
            std::env::var("PYLON_OAUTH_GITHUB_CLIENT_ID"),
            std::env::var("PYLON_OAUTH_GITHUB_CLIENT_SECRET"),
        ) {
            reg.register(OAuthConfig {
                provider: "github".into(),
                client_id: id,
                client_secret: secret,
                redirect_uri: std::env::var("PYLON_OAUTH_GITHUB_REDIRECT")
                    .unwrap_or_else(|_| "http://localhost:3000/api/auth/callback/github".into()),
            });
        }

        reg
    }
}

// ---------------------------------------------------------------------------
// OAuth state store — CSRF protection for OAuth flows
// ---------------------------------------------------------------------------

/// One stored OAuth state record. Carries the post-callback redirect
/// URLs alongside the provider so the callback handler doesn't need to
/// consult an env var to know where to send the user. Both URLs are
/// validated against `PYLON_TRUSTED_ORIGINS` at create time, so the
/// callback can trust them without re-checking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthState {
    pub provider: String,
    /// URL the callback redirects to on success. The frontend supplies
    /// this via `?callback=` on the start request.
    pub callback_url: String,
    /// URL the callback redirects to on failure. Defaults to
    /// `callback_url` when the frontend doesn't pass an explicit
    /// `?error_callback=`. The error code + message ride along as
    /// query params (`?oauth_error=X&oauth_error_message=Y`).
    pub error_callback_url: String,
    pub expires_at: u64,
}

/// Backing store for OAuth state records. Default impl keeps them in
/// memory (fine for tests + dev); the runtime swaps in a SQLite or
/// Postgres backend so a restart in the middle of an OAuth handshake
/// doesn't leave the user with "invalid state" on the callback.
pub trait OAuthStateBackend: Send + Sync {
    /// Persist a state record under `token`.
    fn put(&self, token: &str, state: &OAuthState);
    /// Atomic compare-and-consume: returns the stored record if the
    /// token exists and hasn't expired, then removes it. Returning
    /// `None` means either the token never existed or it has already
    /// been used / expired.
    fn take(&self, token: &str, now_unix_secs: u64) -> Option<OAuthState>;
}

/// In-memory backend (default). Lost on restart.
pub struct InMemoryOAuthBackend {
    states: Mutex<HashMap<String, OAuthState>>,
}

impl InMemoryOAuthBackend {
    pub fn new() -> Self {
        Self {
            states: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryOAuthBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl OAuthStateBackend for InMemoryOAuthBackend {
    fn put(&self, token: &str, state: &OAuthState) {
        self.states
            .lock()
            .unwrap()
            .insert(token.to_string(), state.clone());
    }
    fn take(&self, token: &str, now_unix_secs: u64) -> Option<OAuthState> {
        let mut s = self.states.lock().unwrap();
        let entry = s.remove(token)?;
        if entry.expires_at <= now_unix_secs {
            return None;
        }
        Some(entry)
    }
}

/// Stores OAuth state parameters to prevent CSRF attacks on the callback.
///
/// State tokens are short-lived (10 minutes) and single-use. Backed by an
/// `OAuthStateBackend`; defaults to in-memory but the runtime persists them
/// to SQLite (or Postgres when `DATABASE_URL` is set) so they survive a
/// restart that happens mid-OAuth-handshake.
pub struct OAuthStateStore {
    backend: Box<dyn OAuthStateBackend>,
}

impl Default for OAuthStateStore {
    fn default() -> Self {
        Self::new()
    }
}

impl OAuthStateStore {
    pub fn new() -> Self {
        Self {
            backend: Box::new(InMemoryOAuthBackend::new()),
        }
    }

    pub fn with_backend(backend: Box<dyn OAuthStateBackend>) -> Self {
        Self { backend }
    }

    /// Generate and store a new state record. Returns the random
    /// state token (the value the OAuth provider echoes back as
    /// `?state=…` on the callback).
    ///
    /// Caller is responsible for validating `callback_url` and
    /// `error_callback_url` against the trusted-origins allowlist
    /// BEFORE calling this — the store trusts what it's given.
    pub fn create(&self, provider: &str, callback_url: &str, error_callback_url: &str) -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let token = generate_token();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let state = OAuthState {
            provider: provider.to_string(),
            callback_url: callback_url.to_string(),
            error_callback_url: error_callback_url.to_string(),
            expires_at: now + 600,
        };
        self.backend.put(&token, &state);
        token
    }

    /// Validate and consume a state token. Returns the stored record
    /// iff the token existed, has not expired, AND matches
    /// `expected_provider`. The token is removed either way to make
    /// replay impossible.
    pub fn validate(&self, state: &str, expected_provider: &str) -> Option<OAuthState> {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let entry = self.backend.take(state, now)?;
        if entry.provider != expected_provider {
            return None;
        }
        Some(entry)
    }
}

/// Validate that `url` has an origin (scheme://host[:port]) listed in
/// `trusted_origins`. Returns `Ok(url)` when trusted (echoes input for
/// chaining), `Err` with a code/message when not. Used by the OAuth
/// start endpoint to gate `?callback=` + `?error_callback=` values
/// before storing them in the state record.
///
/// `trusted_origins` entries are origin strings like
/// `"https://app.example.com"` or `"http://localhost:3000"` — no
/// trailing slash, no path. A `url` like
/// `"http://localhost:3000/dashboard?x=1"` matches the
/// `"http://localhost:3000"` entry.
///
/// Borrowed wholesale from better-auth's `trustedOrigins` model:
/// explicit allowlist, no implicit "same-origin trust," no env-var
/// magic. An open-redirect via OAuth is one of the easier auth bugs
/// to ship by accident.
pub fn validate_trusted_redirect(
    url: &str,
    trusted_origins: &[String],
) -> Result<(), TrustedOriginError> {
    if url.is_empty() {
        return Err(TrustedOriginError::Empty);
    }
    // Must be absolute http(s) URL — no relative paths, no schemes
    // like javascript:, file:, data:.
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(TrustedOriginError::NotHttp);
    }
    let url_origin = origin_of(url);
    if trusted_origins.iter().any(|t| t == &url_origin) {
        Ok(())
    } else {
        Err(TrustedOriginError::NotTrusted { origin: url_origin })
    }
}

/// Reasons a redirect URL might be rejected by [`validate_trusted_redirect`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustedOriginError {
    Empty,
    NotHttp,
    NotTrusted { origin: String },
}

impl std::fmt::Display for TrustedOriginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrustedOriginError::Empty => write!(f, "redirect URL is empty"),
            TrustedOriginError::NotHttp => {
                write!(f, "redirect URL must use http:// or https:// scheme")
            }
            TrustedOriginError::NotTrusted { origin } => write!(
                f,
                "redirect origin {origin:?} is not in PYLON_TRUSTED_ORIGINS"
            ),
        }
    }
}

/// Extract the origin (`scheme://host[:port]`) from a URL string,
/// stripping any path/query/fragment. Best-effort string slicing —
/// no full URL parser dep. Public so router crates can reuse the same
/// logic when comparing redirect URLs against the trusted-origins list.
pub fn origin_of(url: &str) -> String {
    let after_scheme = match url.find("://") {
        Some(i) => i + 3,
        None => return url.trim_end_matches('/').to_string(),
    };
    let rest = &url[after_scheme..];
    let cut = rest
        .find(|c: char| c == '/' || c == '?' || c == '#')
        .unwrap_or(rest.len());
    url[..after_scheme + cut].to_string()
}

// ---------------------------------------------------------------------------
// Magic code auth — email verification codes
// ---------------------------------------------------------------------------

/// Pluggable storage for magic-code records. In-memory is the default
/// (fine for dev); persistent backends (SQLite, Postgres) live in
/// `pylon-runtime` so a server restart between "send code" and "verify
/// code" doesn't invalidate the user's pending login.
///
/// All methods are infallible from the caller's perspective — durability
/// is best-effort. A backend that fails to write should log; the
/// in-memory cache remains authoritative for the current process.
pub trait MagicCodeBackend: Send + Sync {
    /// Replace any existing code for `email` with `code`.
    fn put(&self, email: &str, code: &MagicCode);
    /// Look up the current code for `email`. Returns `None` if absent.
    fn get(&self, email: &str) -> Option<MagicCode>;
    /// Remove the code for `email` (called on successful verify or
    /// expiry). Idempotent — missing key is not an error.
    fn remove(&self, email: &str);
    /// Persist an attempts++ on the existing record without touching
    /// other fields. Used by the verify-failed path to enforce
    /// `MAX_ATTEMPTS` across restarts.
    fn bump_attempts(&self, email: &str);
    /// Load all live records on construction. Lets `MagicCodeStore::with_backend`
    /// hydrate the in-memory cache from durable storage on startup.
    fn load_all(&self) -> Vec<MagicCode>;
}

/// In-memory backend for magic codes. The default — also used as the
/// authoritative cache by `MagicCodeStore`.
pub struct InMemoryMagicCodeBackend {
    codes: Mutex<HashMap<String, MagicCode>>,
}

impl InMemoryMagicCodeBackend {
    pub fn new() -> Self {
        Self {
            codes: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryMagicCodeBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl MagicCodeBackend for InMemoryMagicCodeBackend {
    fn put(&self, email: &str, code: &MagicCode) {
        self.codes
            .lock()
            .unwrap()
            .insert(email.to_string(), code.clone());
    }
    fn get(&self, email: &str) -> Option<MagicCode> {
        self.codes.lock().unwrap().get(email).cloned()
    }
    fn remove(&self, email: &str) {
        self.codes.lock().unwrap().remove(email);
    }
    fn bump_attempts(&self, email: &str) {
        if let Some(c) = self.codes.lock().unwrap().get_mut(email) {
            c.attempts = c.attempts.saturating_add(1);
        }
    }
    fn load_all(&self) -> Vec<MagicCode> {
        self.codes.lock().unwrap().values().cloned().collect()
    }
}

/// A magic-code store. Wraps a `MagicCodeBackend` (in-memory by default)
/// and applies the verify/cooldown semantics. Hydrates the in-memory
/// cache from the backend on construction so durable backends survive
/// restart without losing in-flight codes.
pub struct MagicCodeStore {
    cache: Mutex<HashMap<String, MagicCode>>,
    backend: Box<dyn MagicCodeBackend>,
}

#[derive(Debug, Clone)]
pub struct MagicCode {
    pub email: String,
    pub code: String,
    pub expires_at: u64,
    /// Failed verify attempts against this code. Once it reaches
    /// `MAX_ATTEMPTS` the code is invalidated.
    pub attempts: u32,
}

/// Maximum verify attempts per code before it's burned. 5 is a common bound —
/// lets the user fix typos without enabling realistic brute-force against a
/// 6-digit code space.
const MAX_ATTEMPTS: u32 = 5;

/// Minimum seconds between successive `create()` calls for the same email.
/// Throttles magic-code spam (user can't be flooded with login codes).
const CREATE_COOLDOWN_SECS: u64 = 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MagicCodeError {
    /// There is no active code for this email, or it expired.
    NotFound,
    /// The code is present but `MAX_ATTEMPTS` failed verifies have occurred.
    TooManyAttempts,
    /// The code did not match.
    BadCode,
    /// The code expired since it was created.
    Expired,
    /// Another code was requested too recently. Wait and try again.
    Throttled { retry_after_secs: u64 },
}

impl Default for MagicCodeStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MagicCodeStore {
    pub fn new() -> Self {
        Self::with_backend(Box::new(InMemoryMagicCodeBackend::new()))
    }

    /// Build a magic-code store backed by a persistent backend. Existing
    /// live codes are hydrated into the in-memory cache on construction
    /// so a server restart between "send" and "verify" doesn't kill the
    /// user's pending login.
    pub fn with_backend(backend: Box<dyn MagicCodeBackend>) -> Self {
        let now = now_secs();
        let mut cache = HashMap::new();
        for c in backend.load_all() {
            if c.expires_at > now {
                cache.insert(c.email.clone(), c);
            }
        }
        Self {
            cache: Mutex::new(cache),
            backend,
        }
    }

    /// Generate a 6-digit code for an email and return it. Subject to a
    /// per-email cooldown — returns the error-shape via `try_create`.
    pub fn create(&self, email: &str) -> String {
        // Back-compat wrapper: same signature as before, but we still burn
        // the cooldown if one is active. Use `try_create` for a Result shape.
        self.try_create(email).unwrap_or_else(|_| String::new())
    }

    /// Create a magic code, enforcing per-email cooldown. Returns the code
    /// or an error describing why one couldn't be issued.
    pub fn try_create(&self, email: &str) -> Result<String, MagicCodeError> {
        let now = now_secs();

        let mut codes = self.cache.lock().unwrap();

        // Cooldown check: if a live code exists and was created less than
        // CREATE_COOLDOWN_SECS ago, throttle. The age-of-code is
        // `expires_at - 600 + cooldown` since expires_at is create_time + 600.
        if let Some(existing) = codes.get(email) {
            if existing.expires_at > now {
                let created_at = existing.expires_at.saturating_sub(600);
                let age = now.saturating_sub(created_at);
                if age < CREATE_COOLDOWN_SECS {
                    return Err(MagicCodeError::Throttled {
                        retry_after_secs: CREATE_COOLDOWN_SECS - age,
                    });
                }
            }
        }

        let code = generate_magic_code();
        let mc = MagicCode {
            email: email.to_string(),
            code: code.clone(),
            expires_at: now + 600, // 10 minutes
            attempts: 0,
        };
        codes.insert(email.to_string(), mc.clone());
        // Persist after the cache mutation lands. Backend write is
        // best-effort — if it fails the code still works for this
        // process; only a restart in the next 10 minutes would lose it.
        self.backend.put(email, &mc);
        Ok(code)
    }

    /// Verify a code for an email. Returns true if valid and not expired.
    /// Uses constant-time comparison to prevent timing attacks.
    /// Back-compat wrapper around [`try_verify`].
    pub fn verify(&self, email: &str, code: &str) -> bool {
        matches!(self.try_verify(email, code), Ok(()))
    }

    /// Verify a code. Returns a typed error so callers can surface specific
    /// messages. On the MAX_ATTEMPTS-th failure, the code is burned — even
    /// correct subsequent attempts return `TooManyAttempts`.
    /// Every magic code currently in the cache. Powers the Studio
    /// "Auth tables" view; not for app use. Includes expired codes —
    /// the cache only drops them on next verify attempt for that email.
    pub fn list_all_unfiltered(&self) -> Vec<MagicCode> {
        self.cache
            .lock()
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default()
    }

    pub fn try_verify(&self, email: &str, code: &str) -> Result<(), MagicCodeError> {
        let now = now_secs();
        let mut codes = self.cache.lock().unwrap();

        let mc = match codes.get_mut(email) {
            Some(m) => m,
            None => return Err(MagicCodeError::NotFound),
        };

        if mc.attempts >= MAX_ATTEMPTS {
            return Err(MagicCodeError::TooManyAttempts);
        }
        if mc.expires_at <= now {
            codes.remove(email);
            self.backend.remove(email);
            return Err(MagicCodeError::Expired);
        }

        let ok = constant_time_eq(mc.code.as_bytes(), code.as_bytes());
        if !ok {
            mc.attempts += 1;
            self.backend.bump_attempts(email);
            // Burn the code at MAX_ATTEMPTS so retries can't hit max.
            if mc.attempts >= MAX_ATTEMPTS {
                return Err(MagicCodeError::TooManyAttempts);
            }
            return Err(MagicCodeError::BadCode);
        }

        // Correct code — consume it.
        codes.remove(email);
        self.backend.remove(email);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Cryptographic helpers — CSPRNG-based token and code generation
// ---------------------------------------------------------------------------

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Generate a 6-digit magic code using a CSPRNG.
fn generate_magic_code() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let code: u32 = rng.gen_range(0..1_000_000);
    format!("{:06}", code)
}

/// Generate a session token with 256 bits of entropy from a CSPRNG.
fn generate_token() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: [u8; 32] = rng.gen();
    format!("pylon_{}", hex_encode(&bytes))
}

// ---------------------------------------------------------------------------
// Session store — in-memory for dev
// ---------------------------------------------------------------------------

use std::collections::HashMap;
use std::sync::Mutex;

/// Pluggable storage backend for sessions. The default is in-memory; apps
/// deploying for real should supply a persistent backend (e.g. SQLite or
/// Redis) so users don't log out on server restart.
pub trait SessionBackend: Send + Sync {
    fn load_all(&self) -> Vec<Session>;
    fn save(&self, session: &Session);
    fn remove(&self, token: &str);
}

/// A session store. In-memory by default; optionally backed by a
/// persistent [`SessionBackend`].
///
/// The in-memory map is always authoritative — reads don't touch the
/// backend. The backend receives every `save`/`remove`, making it a
/// write-through cache. On construction via [`SessionStore::with_backend`],
/// the store hydrates from the backend so sessions survive restart.
pub struct SessionStore {
    sessions: Mutex<HashMap<String, Session>>,
    backend: Option<Box<dyn SessionBackend>>,
    /// Default lifetime for new sessions (seconds). Sourced from the
    /// manifest's `auth.session.expires_in` config at server boot;
    /// falls back to `Session::DEFAULT_LIFETIME_SECS` (30 days).
    default_lifetime_secs: u64,
}

impl Default for SessionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionStore {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            backend: None,
            default_lifetime_secs: Session::DEFAULT_LIFETIME_SECS,
        }
    }

    /// Override the default session lifetime. Used by `pylon-runtime`'s
    /// server bootstrap to apply the manifest's `auth.session.expires_in`.
    pub fn with_lifetime(mut self, lifetime_secs: u64) -> Self {
        self.default_lifetime_secs = lifetime_secs;
        self
    }

    /// Build a session store backed by a persistent store. Existing sessions
    /// are loaded from the backend on construction; every future mutation
    /// writes through.
    pub fn with_backend(backend: Box<dyn SessionBackend>) -> Self {
        let mut map = HashMap::new();
        for s in backend.load_all() {
            if !s.is_expired() {
                map.insert(s.token.clone(), s);
            }
        }
        Self {
            sessions: Mutex::new(map),
            backend: Some(backend),
            default_lifetime_secs: Session::DEFAULT_LIFETIME_SECS,
        }
    }

    /// Create a session for a user and return it. Uses the store's
    /// configured `default_lifetime_secs` (from the manifest's
    /// `auth.session.expires_in`, default 30 days).
    pub fn create(&self, user_id: String) -> Session {
        let session = Session::with_lifetime(user_id, self.default_lifetime_secs);
        let mut sessions = self.sessions.lock().unwrap();
        sessions.insert(session.token.clone(), session.clone());
        if let Some(b) = &self.backend {
            b.save(&session);
        }
        session
    }

    /// Look up a session by token. Returns None if the session is expired.
    pub fn get(&self, token: &str) -> Option<Session> {
        let mut sessions = self.sessions.lock().unwrap();
        match sessions.get(token) {
            Some(s) if s.is_expired() => {
                sessions.remove(token);
                None
            }
            Some(s) => Some(s.clone()),
            None => None,
        }
    }

    /// Resolve a token to an auth context.
    /// Returns anonymous context if the token is invalid, missing, or expired.
    pub fn resolve(&self, token: Option<&str>) -> AuthContext {
        match token {
            Some(t) => match self.get(t) {
                Some(session) => session.to_auth_context(),
                None => AuthContext::anonymous(),
            },
            None => AuthContext::anonymous(),
        }
    }

    /// Refresh a session — issues a new token, copies user/device, extends expiry.
    /// The old token is revoked. Returns the new session or None if the old
    /// token is missing/expired.
    pub fn refresh(&self, old_token: &str) -> Option<Session> {
        let mut sessions = self.sessions.lock().unwrap();
        let old = sessions.remove(old_token)?;
        if let Some(b) = &self.backend {
            b.remove(old_token);
        }
        if old.is_expired() {
            return None;
        }
        let mut new = Session::new(old.user_id.clone());
        new.device = old.device.clone();
        sessions.insert(new.token.clone(), new.clone());
        if let Some(b) = &self.backend {
            b.save(&new);
        }
        Some(new)
    }

    /// Every session in the store, including expired ones, with no
    /// filtering. Powers the Studio "Auth tables" view so operators
    /// can see orphaned sessions / debug stuck logins. Don't use for
    /// app code — `list_for_user` is the right surface there.
    pub fn list_all_unfiltered(&self) -> Vec<Session> {
        self.sessions
            .lock()
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default()
    }

    /// List all active sessions for a user.
    pub fn list_for_user(&self, user_id: &str) -> Vec<Session> {
        let sessions = self.sessions.lock().unwrap();
        sessions
            .values()
            .filter(|s| s.user_id == user_id && !s.is_expired())
            .cloned()
            .collect()
    }

    /// Revoke all sessions for a user. Returns the count removed.
    pub fn revoke_all_for_user(&self, user_id: &str) -> usize {
        let mut sessions = self.sessions.lock().unwrap();
        let tokens: Vec<String> = sessions
            .iter()
            .filter_map(|(t, s)| {
                if s.user_id == user_id {
                    Some(t.clone())
                } else {
                    None
                }
            })
            .collect();
        let n = tokens.len();
        for t in &tokens {
            sessions.remove(t);
            if let Some(b) = &self.backend {
                b.remove(t);
            }
        }
        n
    }

    /// Sweep expired sessions. Returns the count removed.
    pub fn sweep_expired(&self) -> usize {
        let mut sessions = self.sessions.lock().unwrap();
        let expired: Vec<String> = sessions
            .iter()
            .filter_map(|(t, s)| {
                if s.is_expired() {
                    Some(t.clone())
                } else {
                    None
                }
            })
            .collect();
        let n = expired.len();
        for t in &expired {
            sessions.remove(t);
            if let Some(b) = &self.backend {
                b.remove(t);
            }
        }
        n
    }

    /// Attach a device label to a session (typically on login from a browser).
    pub fn set_device(&self, token: &str, device: String) -> bool {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(s) = sessions.get_mut(token) {
            s.device = Some(device);
            if let Some(b) = &self.backend {
                b.save(s);
            }
            true
        } else {
            false
        }
    }

    /// Create a guest session with a generated anonymous ID.
    pub fn create_guest(&self) -> Session {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        let bytes: [u8; 16] = rng.gen();
        let guest_id = format!("guest_{}", hex_encode(&bytes));
        self.create(guest_id)
    }

    /// Upgrade a guest session to a real user. Replaces the user_id.
    pub fn upgrade(&self, token: &str, real_user_id: String) -> bool {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(session) = sessions.get_mut(token) {
            session.user_id = real_user_id;
            if let Some(b) = &self.backend {
                b.save(session);
            }
            true
        } else {
            false
        }
    }

    /// Switch the session's active tenant (organization). `None` clears it.
    /// Callers should verify the user actually has membership in the target
    /// tenant BEFORE invoking this — the session store takes the value on
    /// trust. Returns true if the session exists, false otherwise.
    pub fn set_tenant(&self, token: &str, tenant_id: Option<String>) -> bool {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(session) = sessions.get_mut(token) {
            session.tenant_id = tenant_id;
            if let Some(b) = &self.backend {
                b.save(session);
            }
            true
        } else {
            false
        }
    }

    /// Remove a session.
    pub fn revoke(&self, token: &str) -> bool {
        let mut sessions = self.sessions.lock().unwrap();
        let removed = sessions.remove(token).is_some();
        if removed {
            if let Some(b) = &self.backend {
                b.remove(token);
            }
        }
        removed
    }
}

// ---------------------------------------------------------------------------
// OAuth account links — better-auth's `account` table equivalent
// ---------------------------------------------------------------------------

/// A persisted account link. Schema-aligned with better-auth's `account`
/// table (verified against https://www.better-auth.com/docs/concepts/database
/// at the time of writing) so users migrating from better-auth see the
/// same field names + meanings:
///
/// - `provider_id` — the provider's name (`"google"`, `"github"`, plus
///   `"credential"` once email/password auth lands). Matches
///   better-auth's `providerId`.
/// - `account_id` — the PROVIDER'S ID for the user (Google `sub`,
///   GitHub numeric `id`, or for email/password the user's own id).
///   Matches better-auth's `accountId`. NOT the row PK.
/// - `id` — the row PK, generated. Lets the row be referenced
///   independently of the (provider_id, account_id) natural key.
/// - `password` — bcrypt/argon2 hash for `provider_id="credential"`
///   rows; `None` for OAuth links. Reserves the column so adding
///   email/password auth doesn't need a schema migration.
///
/// Account vs. user: a single User row can have many Account rows
/// (Google + GitHub + a password — all linked to one pylon user).
/// Provider lookup is by `(provider_id, account_id)` — NOT email — so
/// a user changing their Google address keeps the same pylon account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub id: String,
    pub user_id: String,
    /// Provider name — `"google"`, `"github"`, `"credential"`, etc.
    /// (better-auth: `providerId`)
    pub provider_id: String,
    /// Provider's id for the user — Google `sub`, GitHub numeric `id`,
    /// or for `provider_id="credential"` the user's own id. (better-auth: `accountId`)
    pub account_id: String,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    /// Unix epoch seconds at which `access_token` expires. `None` for
    /// non-expiring tokens (GitHub Classic apps) or for password rows.
    pub access_token_expires_at: Option<u64>,
    /// Unix epoch seconds at which `refresh_token` expires. `None` when
    /// the provider doesn't expire refresh tokens (most don't, but
    /// Microsoft Identity Platform does after 90 days of inactivity).
    pub refresh_token_expires_at: Option<u64>,
    pub scope: Option<String>,
    /// Bcrypt/argon2 hash for email/password rows. `None` for OAuth.
    /// Always `None` today — present so adding password auth later
    /// doesn't require a schema migration.
    pub password: Option<String>,
    /// Unix epoch seconds when this account was first linked.
    pub created_at: u64,
    /// Unix epoch seconds when the token bundle was last refreshed.
    pub updated_at: u64,
}

impl Account {
    /// Build a new account link from a freshly-completed OAuth handshake.
    /// Generates a fresh row id; the `(provider_id, account_id)` pair is
    /// what later lookups key on.
    pub fn new(user_id: String, info: &UserInfo, tokens: &TokenSet) -> Self {
        let now = now_secs();
        Self {
            id: generate_token(),
            user_id,
            provider_id: info.provider.clone(),
            account_id: info.provider_account_id.clone(),
            access_token: Some(tokens.access_token.clone()),
            refresh_token: tokens.refresh_token.clone(),
            id_token: tokens.id_token.clone(),
            access_token_expires_at: tokens.expires_at,
            refresh_token_expires_at: None,
            scope: tokens.scope.clone(),
            password: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// True if `access_token_expires_at` is set and has passed.
    /// Non-expiring tokens (GitHub Classic) report `false` — caller
    /// should treat them as "valid until proven otherwise" and refresh
    /// on 401.
    pub fn access_token_expired(&self) -> bool {
        match self.access_token_expires_at {
            Some(ts) => now_secs() >= ts,
            None => false,
        }
    }
}

/// Pluggable storage for account links. In-memory default ships with
/// the crate; SQLite + Postgres impls live in `pylon-runtime`.
pub trait AccountBackend: Send + Sync {
    /// Insert or refresh an account link. The `(provider_id, account_id)`
    /// pair is the natural key — repeated calls for the same pair
    /// update the token bundle and `updated_at` on the existing row.
    fn upsert(&self, account: &Account);
    /// Find an account by provider identity. Returns `None` if the user
    /// hasn't linked this provider yet.
    fn find_by_provider(&self, provider_id: &str, account_id: &str) -> Option<Account>;
    /// Every account linked to a user. The `/api/auth/me` endpoint uses
    /// this to render "you're connected via Google + GitHub" in the UI
    /// and to gate "unlink" affordances behind "user has another way to
    /// sign in" checks.
    fn find_for_user(&self, user_id: &str) -> Vec<Account>;
    /// Remove a single provider link. Returns `true` if a row was removed.
    fn unlink(&self, provider_id: &str, account_id: &str) -> bool;
    /// Every account in the store. Used by `AccountStore::list_all_unfiltered`
    /// to power the Studio admin inspector. Backends that can stream
    /// (SQLite, Postgres) just `SELECT *`; the in-memory backend
    /// returns its full map.
    fn list_all(&self) -> Vec<Account>;
}

/// In-memory account backend (default). Lost on restart — production
/// deployments should swap in a persistent backend so refresh tokens
/// survive a redeploy.
pub struct InMemoryAccountBackend {
    /// Keyed by `(provider_id, account_id)`. A separate map keyed on
    /// user_id would speed up `find_for_user` but at framework scale
    /// the linear scan of (typically ≤ 5) accounts per user is fine.
    accounts: Mutex<HashMap<(String, String), Account>>,
}

impl InMemoryAccountBackend {
    pub fn new() -> Self {
        Self {
            accounts: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryAccountBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl AccountBackend for InMemoryAccountBackend {
    fn upsert(&self, account: &Account) {
        let key = (account.provider_id.clone(), account.account_id.clone());
        self.accounts.lock().unwrap().insert(key, account.clone());
    }
    fn find_by_provider(&self, provider_id: &str, account_id: &str) -> Option<Account> {
        self.accounts
            .lock()
            .unwrap()
            .get(&(provider_id.to_string(), account_id.to_string()))
            .cloned()
    }
    fn find_for_user(&self, user_id: &str) -> Vec<Account> {
        self.accounts
            .lock()
            .unwrap()
            .values()
            .filter(|a| a.user_id == user_id)
            .cloned()
            .collect()
    }
    fn unlink(&self, provider_id: &str, account_id: &str) -> bool {
        self.accounts
            .lock()
            .unwrap()
            .remove(&(provider_id.to_string(), account_id.to_string()))
            .is_some()
    }
    fn list_all(&self) -> Vec<Account> {
        self.accounts.lock().unwrap().values().cloned().collect()
    }
}

/// Account store. Wraps an `AccountBackend` and provides the methods the
/// OAuth callback / API endpoints actually call.
pub struct AccountStore {
    backend: Box<dyn AccountBackend>,
}

impl Default for AccountStore {
    fn default() -> Self {
        Self::new()
    }
}

impl AccountStore {
    pub fn new() -> Self {
        Self {
            backend: Box::new(InMemoryAccountBackend::new()),
        }
    }
    pub fn with_backend(backend: Box<dyn AccountBackend>) -> Self {
        Self { backend }
    }
    pub fn upsert(&self, account: &Account) {
        self.backend.upsert(account);
    }
    pub fn find_by_provider(&self, provider_id: &str, account_id: &str) -> Option<Account> {
        self.backend.find_by_provider(provider_id, account_id)
    }
    pub fn find_for_user(&self, user_id: &str) -> Vec<Account> {
        self.backend.find_for_user(user_id)
    }
    pub fn unlink(&self, provider_id: &str, account_id: &str) -> bool {
        self.backend.unlink(provider_id, account_id)
    }

    /// Every account in the store. Powers the Studio "Auth tables"
    /// view; not for app use. Implemented by walking the backend's
    /// per-user index — doable because account counts per user are
    /// small (typically ≤ 5) and total account count tracks user
    /// count.
    ///
    /// We don't add a `list_all` method to the `AccountBackend` trait
    /// because the in-memory + sqlite + postgres impls would each
    /// need a separate implementation, and the operational use case
    /// (Studio inspector) is narrow enough to live behind a wrapper
    /// that walks the underlying store directly. For PG/SQLite that
    /// means a `SELECT * FROM _pylon_accounts` — which the backends
    /// can grow if we ever need this at scale.
    pub fn list_all_unfiltered(&self) -> Vec<Account> {
        self.backend.list_all()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anonymous_context() {
        let ctx = AuthContext::anonymous();
        assert!(!ctx.is_authenticated());
        assert!(ctx.user_id.is_none());
    }

    #[test]
    fn authenticated_context() {
        let ctx = AuthContext::authenticated("user-1".into());
        assert!(ctx.is_authenticated());
        assert_eq!(ctx.user_id, Some("user-1".into()));
    }

    #[test]
    fn auth_mode_public_allows_anonymous() {
        let mode = AuthMode::Public;
        assert!(mode.check(&AuthContext::anonymous()));
        assert!(mode.check(&AuthContext::authenticated("user-1".into())));
    }

    #[test]
    fn auth_mode_user_requires_authenticated() {
        let mode = AuthMode::User;
        assert!(!mode.check(&AuthContext::anonymous()));
        assert!(mode.check(&AuthContext::authenticated("user-1".into())));
    }

    #[test]
    fn auth_mode_from_str() {
        assert_eq!(AuthMode::from_str("public"), Some(AuthMode::Public));
        assert_eq!(AuthMode::from_str("user"), Some(AuthMode::User));
        assert_eq!(AuthMode::from_str("admin"), None);
    }

    #[test]
    fn session_store_create_and_get() {
        let store = SessionStore::new();
        let session = store.create("user-1".into());
        assert!(!session.token.is_empty());
        assert!(session.token.starts_with("pylon_"));

        let retrieved = store.get(&session.token).unwrap();
        assert_eq!(retrieved.user_id, "user-1");
    }

    #[test]
    fn session_store_resolve() {
        let store = SessionStore::new();
        let session = store.create("user-1".into());

        let ctx = store.resolve(Some(&session.token));
        assert!(ctx.is_authenticated());
        assert_eq!(ctx.user_id, Some("user-1".into()));

        let anon = store.resolve(None);
        assert!(!anon.is_authenticated());

        let bad = store.resolve(Some("invalid-token"));
        assert!(!bad.is_authenticated());
    }

    #[test]
    fn session_store_revoke() {
        let store = SessionStore::new();
        let session = store.create("user-1".into());

        assert!(store.revoke(&session.token));
        assert!(store.get(&session.token).is_none());
        assert!(!store.revoke(&session.token)); // already revoked
    }

    #[test]
    fn session_to_auth_context() {
        let session = Session::new("user-42".into());
        let ctx = session.to_auth_context();
        assert_eq!(ctx.user_id, Some("user-42".into()));
    }

    // -- Admin context --

    #[test]
    fn admin_context() {
        let ctx = AuthContext::admin();
        assert!(ctx.is_admin);
        assert!(ctx.is_authenticated());
    }

    #[test]
    fn anonymous_not_admin() {
        let ctx = AuthContext::anonymous();
        assert!(!ctx.is_admin);
    }

    #[test]
    fn authenticated_not_admin() {
        let ctx = AuthContext::authenticated("user-1".into());
        assert!(!ctx.is_admin);
    }

    // -- Magic codes --

    #[test]
    fn magic_code_create_and_verify() {
        let store = MagicCodeStore::new();
        let code = store.create("test@example.com");
        assert_eq!(code.len(), 6);
        assert!(store.verify("test@example.com", &code));
    }

    #[test]
    fn magic_code_wrong_code_rejected() {
        let store = MagicCodeStore::new();
        store.create("test@example.com");
        assert!(!store.verify("test@example.com", "000000"));
    }

    #[test]
    fn magic_code_wrong_email_rejected() {
        let store = MagicCodeStore::new();
        let code = store.create("test@example.com");
        assert!(!store.verify("other@example.com", &code));
    }

    #[test]
    fn magic_code_consumed_after_verify() {
        let store = MagicCodeStore::new();
        let code = store.create("test@example.com");
        assert!(store.verify("test@example.com", &code));
        // Second verify should fail — code consumed.
        assert!(!store.verify("test@example.com", &code));
    }

    #[test]
    fn magic_code_different_emails_independent() {
        let store = MagicCodeStore::new();
        let code1 = store.create("alice@example.com");
        let code2 = store.create("bob@example.com");
        // Each email has its own code.
        assert!(store.verify("alice@example.com", &code1));
        assert!(store.verify("bob@example.com", &code2));
    }

    // -- Constant-time comparison --

    #[test]
    fn constant_time_eq_equal() {
        assert!(constant_time_eq(b"hello", b"hello"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn constant_time_eq_not_equal() {
        assert!(!constant_time_eq(b"hello", b"world"));
        assert!(!constant_time_eq(b"hello", b"hell"));
        assert!(!constant_time_eq(b"a", b"b"));
    }

    // -- Token generation --

    #[test]
    fn generated_tokens_are_unique() {
        let t1 = generate_token();
        let t2 = generate_token();
        assert_ne!(t1, t2);
        assert!(t1.starts_with("pylon_"));
        assert!(t2.starts_with("pylon_"));
        // 256 bits = 64 hex chars + "pylon_" prefix (6 chars)
        assert_eq!(t1.len(), 6 + 64);
    }

    // -- OAuth registry --

    #[test]
    fn oauth_registry_empty() {
        let reg = OAuthRegistry::new();
        assert!(reg.get("google").is_none());
    }

    #[test]
    fn oauth_registry_register_and_get() {
        let mut reg = OAuthRegistry::new();
        reg.register(OAuthConfig {
            provider: "google".into(),
            client_id: "test-id".into(),
            client_secret: "test-secret".into(),
            redirect_uri: "http://localhost/callback".into(),
        });
        let config = reg.get("google").unwrap();
        assert_eq!(config.client_id, "test-id");
        assert!(config.auth_url().contains("accounts.google.com"));
    }

    // -- Guest auth --

    #[test]
    fn guest_session() {
        let store = SessionStore::new();
        let session = store.create_guest();
        assert!(session.user_id.starts_with("guest_"));
        assert!(!session.token.is_empty());

        let ctx = store.resolve(Some(&session.token));
        assert!(ctx.is_authenticated());
        assert!(ctx.user_id.unwrap().starts_with("guest_"));
    }

    #[test]
    fn upgrade_guest_to_real_user() {
        let store = SessionStore::new();
        let session = store.create_guest();
        assert!(session.user_id.starts_with("guest_"));

        let upgraded = store.upgrade(&session.token, "real-user-123".into());
        assert!(upgraded);

        let ctx = store.resolve(Some(&session.token));
        assert_eq!(ctx.user_id, Some("real-user-123".into()));
    }

    #[test]
    fn upgrade_invalid_token_fails() {
        let store = SessionStore::new();
        let upgraded = store.upgrade("nonexistent-token", "user".into());
        assert!(!upgraded);
    }

    #[test]
    fn guest_context() {
        let ctx = AuthContext::guest("guest_123".into());
        // Guests carry a stable id but are NOT authenticated — routes
        // guarded by AuthMode::User must reject them.
        assert!(!ctx.is_authenticated());
        assert!(ctx.is_guest);
        assert!(!ctx.is_admin);
        assert_eq!(ctx.user_id, Some("guest_123".into()));
        assert!(!AuthMode::User.check(&ctx));
        assert!(AuthMode::Public.check(&ctx));
    }

    #[test]
    fn oauth_token_urls() {
        let google = OAuthConfig {
            provider: "google".into(),
            client_id: "x".into(),
            client_secret: "x".into(),
            redirect_uri: "x".into(),
        };
        assert_eq!(google.token_url(), "https://oauth2.googleapis.com/token");
        let github = OAuthConfig {
            provider: "github".into(),
            client_id: "x".into(),
            client_secret: "x".into(),
            redirect_uri: "x".into(),
        };
        assert_eq!(
            github.token_url(),
            "https://github.com/login/oauth/access_token"
        );
        let unknown = OAuthConfig {
            provider: "unknown".into(),
            client_id: "x".into(),
            client_secret: "x".into(),
            redirect_uri: "x".into(),
        };
        assert_eq!(unknown.token_url(), "");
        assert!(unknown.auth_url().is_empty());
    }

    #[test]
    fn oauth_auth_url_github() {
        let config = OAuthConfig {
            provider: "github".into(),
            client_id: "gh-id".into(),
            client_secret: "gh-secret".into(),
            redirect_uri: "http://localhost/cb".into(),
        };
        assert!(config.auth_url().contains("github.com"));
        assert!(config.auth_url().contains("gh-id"));
    }

    #[test]
    fn oauth_auth_url_with_state() {
        let config = OAuthConfig {
            provider: "google".into(),
            client_id: "test-id".into(),
            client_secret: "test-secret".into(),
            redirect_uri: "http://localhost/cb".into(),
        };
        let url = config.auth_url_with_state("random_state_123");
        assert!(url.contains("&state=random_state_123"));
    }

    #[test]
    fn oauth_state_store_create_and_validate() {
        let store = OAuthStateStore::new();
        let token = store.create("google", "https://app/cb", "https://app/login");
        let rec = store.validate(&token, "google").expect("valid first time");
        assert_eq!(rec.callback_url, "https://app/cb");
        assert_eq!(rec.error_callback_url, "https://app/login");
        // Second validation should fail — single-use.
        assert!(store.validate(&token, "google").is_none());
    }

    #[test]
    fn oauth_state_store_wrong_provider_rejected() {
        let store = OAuthStateStore::new();
        let token = store.create("google", "https://app/cb", "https://app/cb");
        assert!(store.validate(&token, "github").is_none());
    }

    #[test]
    fn oauth_state_store_invalid_state_rejected() {
        let store = OAuthStateStore::new();
        assert!(store.validate("nonexistent", "google").is_none());
    }

    #[test]
    fn validate_trusted_redirect_basics() {
        let trusted = vec!["http://localhost:3000".to_string()];
        assert!(validate_trusted_redirect("http://localhost:3000/dashboard", &trusted).is_ok());
        assert!(validate_trusted_redirect("http://localhost:3000", &trusted).is_ok());
        assert!(validate_trusted_redirect("http://localhost:3000/x?y=1", &trusted).is_ok());

        // Wrong port → wrong origin.
        assert!(matches!(
            validate_trusted_redirect("http://localhost:4321/dashboard", &trusted),
            Err(TrustedOriginError::NotTrusted { .. })
        ));
        // Non-http scheme rejected even before trusted check (defense
        // against javascript:, file:, data:).
        assert!(matches!(
            validate_trusted_redirect("javascript:alert(1)", &trusted),
            Err(TrustedOriginError::NotHttp)
        ));
        assert!(matches!(
            validate_trusted_redirect("", &trusted),
            Err(TrustedOriginError::Empty)
        ));
    }
}
