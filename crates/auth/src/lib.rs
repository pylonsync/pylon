pub mod email;

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
    pub user_id: Option<String>,
    /// Whether this is an admin context (bypasses policies).
    pub is_admin: bool,
    /// Roles granted to this user. Empty for anonymous.
    pub roles: Vec<String>,
    /// Active tenant id (for multi-tenant apps). Set when the user has
    /// selected an organization for the current session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
}

impl AuthContext {
    /// Create an anonymous/public auth context.
    pub fn anonymous() -> Self {
        Self {
            user_id: None,
            is_admin: false,
            roles: Vec::new(),
            tenant_id: None,
        }
    }

    /// Create an authenticated auth context.
    pub fn authenticated(user_id: String) -> Self {
        Self {
            user_id: Some(user_id),
            is_admin: false,
            roles: Vec::new(),
            tenant_id: None,
        }
    }

    /// Create a guest auth context with a persistent anonymous ID.
    pub fn guest(guest_id: String) -> Self {
        Self {
            user_id: Some(guest_id),
            is_admin: false,
            roles: Vec::new(),
            tenant_id: None,
        }
    }

    /// Create an admin auth context that bypasses all policies.
    pub fn admin() -> Self {
        Self {
            user_id: Some("__admin__".into()),
            is_admin: true,
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
    pub fn is_authenticated(&self) -> bool {
        self.user_id.is_some()
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
    pub fn is_expired(&self) -> bool {
        self.expires_at != 0 && now_secs() > self.expires_at
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

    /// Exchange an authorization code for an access token.
    ///
    /// Uses the system `curl` binary so the auth crate stays free of HTTP
    /// client dependencies. Returns the provider-specific access token string
    /// (extracted from the JSON response).
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
    /// Returns `(email, display_name)`.
    pub fn fetch_userinfo(&self, access_token: &str) -> Result<(String, Option<String>), String> {
        let out = http_get_bearer(self.userinfo_url(), access_token)?;
        let parsed: serde_json::Value = serde_json::from_str(&out)
            .map_err(|e| format!("userinfo not valid JSON: {e}"))?;
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
                Ok((email, name))
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
                Ok((email, name))
            }
            _ => Err(format!("unknown provider: {}", self.provider)),
        }
    }
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
        Ok(resp) => resp
            .into_string()
            .map_err(|e| format!("read body: {e}")),
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
        Ok(resp) => resp
            .into_string()
            .map_err(|e| format!("read body: {e}")),
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

/// Backing store for OAuth state tokens. Default impl keeps them in memory
/// (fine for tests + dev); the runtime swaps in a SQLite-backed impl so a
/// restart in the middle of an OAuth handshake doesn't leave the user with
/// "invalid state" on the callback. Same pattern as `SessionBackend`.
pub trait OAuthStateBackend: Send + Sync {
    fn put(&self, token: &str, provider: &str, expires_at: u64);
    /// Atomic compare-and-consume: returns the stored provider if the token
    /// exists and hasn't expired, then removes it. Returning `None` means
    /// either the token never existed or it has already been used.
    fn take(&self, token: &str, now_unix_secs: u64) -> Option<String>;
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
    fn put(&self, token: &str, provider: &str, expires_at: u64) {
        self.states.lock().unwrap().insert(
            token.to_string(),
            OAuthState {
                provider: provider.to_string(),
                expires_at,
            },
        );
    }
    fn take(&self, token: &str, now_unix_secs: u64) -> Option<String> {
        let mut s = self.states.lock().unwrap();
        let entry = s.remove(token)?;
        if entry.expires_at <= now_unix_secs {
            return None;
        }
        Some(entry.provider)
    }
}

/// Stores OAuth state parameters to prevent CSRF attacks on the callback.
///
/// State tokens are short-lived (10 minutes) and single-use. Backed by an
/// `OAuthStateBackend`; defaults to in-memory but the runtime persists them
/// to SQLite so they survive a restart that happens mid-OAuth-handshake.
pub struct OAuthStateStore {
    backend: Box<dyn OAuthStateBackend>,
}

pub struct OAuthState {
    pub provider: String,
    pub expires_at: u64,
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

    /// Generate and store a new state parameter. Returns the random state string.
    pub fn create(&self, provider: &str) -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let token = generate_token();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.backend.put(&token, provider, now + 600);
        token
    }

    /// Validate and consume a state parameter. Returns true iff the state
    /// existed, has not expired, and matches `expected_provider`. The token
    /// is removed either way to make replay impossible.
    pub fn validate(&self, state: &str, expected_provider: &str) -> bool {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        match self.backend.take(state, now) {
            Some(provider) => provider == expected_provider,
            None => false,
        }
    }
}

// ---------------------------------------------------------------------------
// Magic code auth — email verification codes
// ---------------------------------------------------------------------------

/// An in-memory magic code store for development.
pub struct MagicCodeStore {
    codes: Mutex<HashMap<String, MagicCode>>,
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

impl MagicCodeStore {
    pub fn new() -> Self {
        Self {
            codes: Mutex::new(HashMap::new()),
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

        let mut codes = self.codes.lock().unwrap();

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
        codes.insert(email.to_string(), mc);
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
    pub fn try_verify(&self, email: &str, code: &str) -> Result<(), MagicCodeError> {
        let now = now_secs();
        let mut codes = self.codes.lock().unwrap();

        let mc = match codes.get_mut(email) {
            Some(m) => m,
            None => return Err(MagicCodeError::NotFound),
        };

        if mc.attempts >= MAX_ATTEMPTS {
            return Err(MagicCodeError::TooManyAttempts);
        }
        if mc.expires_at <= now {
            codes.remove(email);
            return Err(MagicCodeError::Expired);
        }

        let ok = constant_time_eq(mc.code.as_bytes(), code.as_bytes());
        if !ok {
            mc.attempts += 1;
            // Burn the code at MAX_ATTEMPTS so retries can't hit max.
            if mc.attempts >= MAX_ATTEMPTS {
                return Err(MagicCodeError::TooManyAttempts);
            }
            return Err(MagicCodeError::BadCode);
        }

        // Correct code — consume it.
        codes.remove(email);
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
}

impl SessionStore {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            backend: None,
        }
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
        }
    }

    /// Create a session for a user and return it.
    pub fn create(&self, user_id: String) -> Session {
        let session = Session::new(user_id);
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
            .filter_map(|(t, s)| if s.is_expired() { Some(t.clone()) } else { None })
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
        // 256 bits = 64 hex chars + "pylon_" prefix (9 chars)
        assert_eq!(t1.len(), 11 + 64);
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
        assert!(ctx.is_authenticated());
        assert!(!ctx.is_admin);
        assert_eq!(ctx.user_id, Some("guest_123".into()));
    }

    #[test]
    fn oauth_token_urls() {
        let google = OAuthConfig { provider: "google".into(), client_id: "x".into(), client_secret: "x".into(), redirect_uri: "x".into() };
        assert_eq!(google.token_url(), "https://oauth2.googleapis.com/token");
        let github = OAuthConfig { provider: "github".into(), client_id: "x".into(), client_secret: "x".into(), redirect_uri: "x".into() };
        assert_eq!(github.token_url(), "https://github.com/login/oauth/access_token");
        let unknown = OAuthConfig { provider: "unknown".into(), client_id: "x".into(), client_secret: "x".into(), redirect_uri: "x".into() };
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
        let state = store.create("google");
        assert!(store.validate(&state, "google"));
        // Second validation should fail — consumed.
        assert!(!store.validate(&state, "google"));
    }

    #[test]
    fn oauth_state_store_wrong_provider_rejected() {
        let store = OAuthStateStore::new();
        let state = store.create("google");
        assert!(!store.validate(&state, "github"));
    }

    #[test]
    fn oauth_state_store_invalid_state_rejected() {
        let store = OAuthStateStore::new();
        assert!(!store.validate("nonexistent", "google"));
    }
}
