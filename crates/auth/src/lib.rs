use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Auth context — the identity available to runtime operations
// ---------------------------------------------------------------------------

/// The auth context for a request. Represents who is making the request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthContext {
    /// The authenticated user ID, or None for public/anonymous access.
    pub user_id: Option<String>,
    /// Whether this is an admin context (bypasses policies).
    #[serde(default)]
    pub is_admin: bool,
}

impl AuthContext {
    /// Create an anonymous/public auth context.
    pub fn anonymous() -> Self {
        Self { user_id: None, is_admin: false }
    }

    /// Create an authenticated auth context.
    pub fn authenticated(user_id: String) -> Self {
        Self {
            user_id: Some(user_id),
            is_admin: false,
        }
    }

    /// Create a guest auth context with a persistent anonymous ID.
    pub fn guest(guest_id: String) -> Self {
        Self {
            user_id: Some(guest_id),
            is_admin: false,
        }
    }

    /// Create an admin auth context that bypasses all policies.
    pub fn admin() -> Self {
        Self {
            user_id: Some("__admin__".into()),
            is_admin: true,
        }
    }

    /// Check if this context represents an authenticated user.
    pub fn is_authenticated(&self) -> bool {
        self.user_id.is_some()
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
}

impl Session {
    /// Create a new session with a generated token.
    pub fn new(user_id: String) -> Self {
        Self {
            token: generate_token(),
            user_id,
        }
    }

    /// Convert this session to an auth context.
    pub fn to_auth_context(&self) -> AuthContext {
        AuthContext::authenticated(self.user_id.clone())
    }
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
    /// Looks for AGENTDB_OAUTH_GOOGLE_CLIENT_ID, etc.
    pub fn from_env() -> Self {
        let mut reg = Self::new();

        // Google
        if let (Ok(id), Ok(secret)) = (
            std::env::var("AGENTDB_OAUTH_GOOGLE_CLIENT_ID"),
            std::env::var("AGENTDB_OAUTH_GOOGLE_CLIENT_SECRET"),
        ) {
            reg.register(OAuthConfig {
                provider: "google".into(),
                client_id: id,
                client_secret: secret,
                redirect_uri: std::env::var("AGENTDB_OAUTH_GOOGLE_REDIRECT")
                    .unwrap_or_else(|_| "http://localhost:3000/api/auth/callback/google".into()),
            });
        }

        // GitHub
        if let (Ok(id), Ok(secret)) = (
            std::env::var("AGENTDB_OAUTH_GITHUB_CLIENT_ID"),
            std::env::var("AGENTDB_OAUTH_GITHUB_CLIENT_SECRET"),
        ) {
            reg.register(OAuthConfig {
                provider: "github".into(),
                client_id: id,
                client_secret: secret,
                redirect_uri: std::env::var("AGENTDB_OAUTH_GITHUB_REDIRECT")
                    .unwrap_or_else(|_| "http://localhost:3000/api/auth/callback/github".into()),
            });
        }

        reg
    }
}

// ---------------------------------------------------------------------------
// OAuth state store — CSRF protection for OAuth flows
// ---------------------------------------------------------------------------

/// Stores OAuth state parameters to prevent CSRF attacks on the callback.
///
/// TODO: For production, state tokens should be short-lived and bound to the
/// user's session. This in-memory store is sufficient for development.
pub struct OAuthStateStore {
    states: Mutex<HashMap<String, OAuthState>>,
}

struct OAuthState {
    provider: String,
    expires_at: u64,
}

impl OAuthStateStore {
    pub fn new() -> Self {
        Self {
            states: Mutex::new(HashMap::new()),
        }
    }

    /// Generate and store a new state parameter. Returns the random state string.
    pub fn create(&self, provider: &str) -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let token = generate_token();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.states.lock().unwrap().insert(
            token.clone(),
            OAuthState {
                provider: provider.to_string(),
                expires_at: now + 600, // 10 minutes
            },
        );
        token
    }

    /// Validate and consume a state parameter. Returns the provider if valid.
    pub fn validate(&self, state: &str, expected_provider: &str) -> bool {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut states = self.states.lock().unwrap();
        if let Some(entry) = states.remove(state) {
            return entry.provider == expected_provider && entry.expires_at > now;
        }
        false
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
}

impl MagicCodeStore {
    pub fn new() -> Self {
        Self {
            codes: Mutex::new(HashMap::new()),
        }
    }

    /// Generate a 6-digit code for an email. Returns the code.
    /// In production this would send an email. In dev it just stores and returns.
    pub fn create(&self, email: &str) -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let code = generate_magic_code();

        let mc = MagicCode {
            email: email.to_string(),
            code: code.clone(),
            expires_at: ts + 600, // 10 minutes
        };

        self.codes.lock().unwrap().insert(email.to_string(), mc);
        code
    }

    /// Verify a code for an email. Returns true if valid and not expired.
    /// Uses constant-time comparison to prevent timing attacks.
    pub fn verify(&self, email: &str, code: &str) -> bool {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut codes = self.codes.lock().unwrap();
        if let Some(mc) = codes.get(email) {
            if constant_time_eq(mc.code.as_bytes(), code.as_bytes()) && mc.expires_at > now {
                codes.remove(email);
                return true;
            }
        }
        false
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
    format!("agentdb_{}", hex_encode(&bytes))
}

// ---------------------------------------------------------------------------
// Session store — in-memory for dev
// ---------------------------------------------------------------------------

use std::collections::HashMap;
use std::sync::Mutex;

/// A simple in-memory session store for development.
pub struct SessionStore {
    sessions: Mutex<HashMap<String, Session>>,
}

impl SessionStore {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// Create a session for a user and return it.
    pub fn create(&self, user_id: String) -> Session {
        let session = Session::new(user_id);
        let mut sessions = self.sessions.lock().unwrap();
        sessions.insert(session.token.clone(), session.clone());
        session
    }

    /// Look up a session by token.
    pub fn get(&self, token: &str) -> Option<Session> {
        let sessions = self.sessions.lock().unwrap();
        sessions.get(token).cloned()
    }

    /// Resolve a token to an auth context.
    /// Returns anonymous context if the token is invalid or missing.
    pub fn resolve(&self, token: Option<&str>) -> AuthContext {
        match token {
            Some(t) => match self.get(t) {
                Some(session) => session.to_auth_context(),
                None => AuthContext::anonymous(),
            },
            None => AuthContext::anonymous(),
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
            true
        } else {
            false
        }
    }

    /// Remove a session.
    pub fn revoke(&self, token: &str) -> bool {
        let mut sessions = self.sessions.lock().unwrap();
        sessions.remove(token).is_some()
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
        assert!(session.token.starts_with("agentdb_"));

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
        assert!(t1.starts_with("agentdb_"));
        assert!(t2.starts_with("agentdb_"));
        // 256 bits = 64 hex chars + "agentdb_" prefix
        assert_eq!(t1.len(), 8 + 64);
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
