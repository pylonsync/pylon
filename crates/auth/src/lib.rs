pub mod api_key;
pub mod apple_jwt;
pub mod audit;
pub mod captcha;
pub mod cookie;
pub mod device;
pub mod email;
pub mod email_templates;
pub mod jwt;
pub mod oidc_provider;
pub mod org;
pub mod org_sso;
pub mod password;
pub mod phone;
pub mod provider;
pub mod rate_limit;
pub mod scim;
pub mod siwe;
pub mod stripe;
pub mod totp;
pub mod trusted_device;
pub mod verification;
pub mod webauthn;

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
    /// API key id when the request was authenticated via a `pk.…`
    /// bearer token. Set so policies + management endpoints can
    /// distinguish "user-via-session" from "user-via-key" — e.g.
    /// password change is forbidden via API key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_id: Option<String>,
    /// Comma-separated scope string from the API key. Application
    /// policies decide what scopes mean — pylon only carries them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_scopes: Option<String>,
    /// Wave-7 E. True iff the request carried a `pylon_trusted_device`
    /// cookie that resolved to a non-expired record bound to the same
    /// user_id as the current session. Apps gate their TOTP step on
    /// this — `if user.totpVerified && !ctx.auth.isTrustedDevice` →
    /// require a code; otherwise skip.
    ///
    /// The framework deliberately does NOT auto-skip TOTP: enforcement
    /// is an app policy. We only carry the boolean.
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_trusted_device: bool,
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
            api_key_id: None,
            api_key_scopes: None,
            is_trusted_device: false,
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
            api_key_id: None,
            api_key_scopes: None,
            is_trusted_device: false,
        }
    }

    /// Create an authenticated context backed by an API key. Policies +
    /// auth-management endpoints can detect this via `is_api_key_auth()`.
    pub fn from_api_key(user_id: String, key_id: String, scopes: Option<String>) -> Self {
        Self {
            user_id: Some(user_id),
            is_admin: false,
            is_guest: false,
            roles: Vec::new(),
            tenant_id: None,
            api_key_id: Some(key_id),
            api_key_scopes: scopes,
            is_trusted_device: false,
        }
    }

    /// True iff this request was authenticated by an API key (not a
    /// session cookie / bearer session token).
    pub fn is_api_key_auth(&self) -> bool {
        self.api_key_id.is_some()
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
            api_key_id: None,
            api_key_scopes: None,
            is_trusted_device: false,
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
            api_key_id: None,
            api_key_scopes: None,
            is_trusted_device: false,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OAuthConfig {
    pub provider: String,
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    /// Optional scope override — replaces the spec's default scope
    /// when set. Use cases: requesting `repo` on GitHub for app
    /// installation flows, requesting `https://www.googleapis.com/...`
    /// scopes on Google for app-specific data access.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scopes_override: Option<String>,
    /// Tenant id for Microsoft/Entra. Defaults to `common`. Single-
    /// tenant apps use a directory GUID; multi-tenant work-only apps
    /// use `organizations`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,
    /// Apple-specific extras (team id, key id, ES256 PEM). Required
    /// for Sign in with Apple — ignored for any other provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apple: Option<provider::AppleConfig>,
    /// OIDC issuer URL when this config targets a generic-OIDC
    /// provider (Auth0, Okta, Keycloak, Cognito, etc.). When set,
    /// the runtime fetches `<issuer>/.well-known/openid-configuration`
    /// and synthesizes a [`provider::ProviderSpec`] from the
    /// discovered endpoints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oidc_issuer: Option<String>,
}

impl OAuthConfig {
    /// Resolve the [`provider::ProviderSpec`] backing this config. For
    /// `oidc_issuer`-configured providers, falls through to the OIDC
    /// discovery cache. Errors propagate so misconfigured providers
    /// fail loudly at first use rather than silently 404'ing later.
    fn resolved_spec(&self) -> Result<provider::ResolvedSpec, String> {
        if let Some(issuer) = self.oidc_issuer.as_deref() {
            return provider::oidc_cache::resolve(issuer);
        }
        provider::find_spec(&self.provider)
            .map(provider::ResolvedSpec::Static)
            .ok_or_else(|| format!("unknown OAuth provider: {}", self.provider))
    }

    /// Build a [`provider::ProviderConfig`] view of `self` for the
    /// helpers in [`provider`] that take the runtime config.
    fn provider_cfg(&self) -> provider::ProviderConfig {
        provider::ProviderConfig {
            provider: self.provider.clone(),
            client_id: self.client_id.clone(),
            client_secret: self.client_secret.clone(),
            redirect_uri: self.redirect_uri.clone(),
            scopes_override: self.scopes_override.clone(),
            tenant: self.tenant.clone(),
            apple: self.apple.clone(),
            oidc_issuer: self.oidc_issuer.clone(),
        }
    }

    /// Generate the authorization URL for the provider.
    ///
    /// Callers MUST append a `&state=<random>` parameter and validate it in the
    /// callback to prevent CSRF attacks. See `OAuthStateStore` for a minimal
    /// implementation.
    ///
    /// For PKCE-required providers (Twitter/X, Kick), callers should
    /// prefer [`Self::auth_url_with_pkce`] so the `code_challenge`
    /// pair survives to the callback.
    pub fn auth_url(&self) -> String {
        match self.build_auth_url(None) {
            Ok(u) => u,
            Err(_) => String::new(),
        }
    }

    /// Generate the authorization URL with a CSRF state parameter attached.
    pub fn auth_url_with_state(&self, state: &str) -> String {
        let base = self.auth_url();
        if base.is_empty() {
            return base;
        }
        format!("{}&state={}", base, url_encode(state))
    }

    /// Generate the authorization URL with state + a freshly minted
    /// PKCE pair when the provider requires it. Returns
    /// `(url, code_verifier)` — the verifier MUST be persisted in
    /// the OAuth state record and replayed in the token exchange.
    pub fn auth_url_with_pkce(&self, state: &str) -> Result<(String, Option<String>), String> {
        let spec = self.resolved_spec()?;
        let pkce = if spec.requires_pkce() {
            Some(generate_pkce())
        } else {
            None
        };
        let challenge = pkce.as_ref().map(|p| p.code_challenge.as_str());
        let mut url = self.build_auth_url(challenge)?;
        if !state.is_empty() {
            url.push_str(&format!("&state={}", url_encode(state)));
        }
        Ok((url, pkce.map(|p| p.code_verifier)))
    }

    fn build_auth_url(&self, pkce_challenge: Option<&str>) -> Result<String, String> {
        let spec = self.resolved_spec()?;
        let cfg = self.provider_cfg();
        let auth = provider::resolve_endpoint(spec.auth_url(), &cfg);
        if auth.is_empty() {
            return Err(format!(
                "provider {} has no authorization endpoint",
                self.provider
            ));
        }
        let scopes_default = spec.scopes().to_string();
        let scopes_raw = self.scopes_override.as_deref().unwrap_or(&scopes_default);
        // Re-join scopes with the provider's separator (TikTok uses
        // commas, everyone else uses spaces). Splitting on whitespace
        // first lets developers always specify scopes the human way.
        let scopes_joined = scopes_raw
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(spec.scope_separator());

        let mut url = format!(
            "{auth}?{cid_param}={cid}&redirect_uri={ruri}&response_type=code&scope={scope}",
            cid_param = spec.client_id_param(),
            cid = url_encode(&self.client_id),
            ruri = url_encode(&self.redirect_uri),
            scope = url_encode(&scopes_joined),
        );
        if !spec.auth_query_extra().is_empty() {
            url.push('&');
            url.push_str(spec.auth_query_extra());
        }
        if let Some(challenge) = pkce_challenge {
            url.push_str("&code_challenge=");
            url.push_str(challenge);
            url.push_str("&code_challenge_method=S256");
        }
        Ok(url)
    }

    /// Generate the token exchange URL.
    pub fn token_url(&self) -> String {
        match self.resolved_spec() {
            Ok(spec) => provider::resolve_endpoint(spec.token_url(), &self.provider_cfg()),
            Err(_) => String::new(),
        }
    }

    /// URL for the userinfo endpoint, which returns the authenticated user's profile.
    pub fn userinfo_url(&self) -> String {
        match self.resolved_spec() {
            Ok(spec) => match spec.userinfo_url() {
                Some(u) => provider::resolve_endpoint(u, &self.provider_cfg()),
                None => String::new(),
            },
            Err(_) => String::new(),
        }
    }

    /// Exchange an authorization code for the full token set
    /// (`access_token`, optional `refresh_token`, optional `id_token`,
    /// `expires_in`, `scope`). When the provider uses PKCE,
    /// `code_verifier` MUST be supplied (the value previously returned
    /// from [`Self::auth_url_with_pkce`]).
    pub fn exchange_code_full(&self, code: &str) -> Result<TokenSet, String> {
        self.exchange_code_full_pkce(code, None)
    }

    pub fn exchange_code_full_pkce(
        &self,
        code: &str,
        code_verifier: Option<&str>,
    ) -> Result<TokenSet, String> {
        let spec = self.resolved_spec()?;
        let cfg = self.provider_cfg();
        let token_url = provider::resolve_endpoint(spec.token_url(), &cfg);
        let pkce_field = code_verifier
            .map(|v| format!("&code_verifier={}", url_encode(v)))
            .unwrap_or_default();

        let out = match spec.token_exchange() {
            provider::TokenExchangeShape::Standard => {
                let body = format!(
                    "code={code}&{cid_param}={cid}&client_secret={secret}&redirect_uri={ruri}&grant_type=authorization_code{pkce}",
                    code = url_encode(code),
                    cid_param = spec.client_id_param(),
                    cid = url_encode(&self.client_id),
                    secret = url_encode(&self.client_secret),
                    ruri = url_encode(&self.redirect_uri),
                    pkce = pkce_field,
                );
                http_post_form(&token_url, &body, true).map_err(sanitize_token_error)?
            }
            provider::TokenExchangeShape::AppleJwt => {
                let apple = self.apple.as_ref().ok_or(
                    "apple provider requires `apple` config (team_id, key_id, private_key_pem)",
                )?;
                let signed_secret = apple_jwt::mint_client_secret(apple, &self.client_id)?;
                let body = format!(
                    "code={code}&client_id={cid}&client_secret={secret}&redirect_uri={ruri}&grant_type=authorization_code{pkce}",
                    code = url_encode(code),
                    cid = url_encode(&self.client_id),
                    secret = url_encode(&signed_secret),
                    ruri = url_encode(&self.redirect_uri),
                    pkce = pkce_field,
                );
                http_post_form(&token_url, &body, true).map_err(sanitize_token_error)?
            }
            provider::TokenExchangeShape::BasicAuth => {
                let body = format!(
                    "code={code}&redirect_uri={ruri}&grant_type=authorization_code{pkce}",
                    code = url_encode(code),
                    ruri = url_encode(&self.redirect_uri),
                    pkce = pkce_field,
                );
                http_post_form_basic(&token_url, &body, &self.client_id, &self.client_secret)
                    .map_err(sanitize_token_error)?
            }
            provider::TokenExchangeShape::JsonBody => {
                let mut json = serde_json::Map::new();
                json.insert("grant_type".into(), "authorization_code".into());
                json.insert("code".into(), code.into());
                json.insert("redirect_uri".into(), self.redirect_uri.clone().into());
                json.insert("client_id".into(), self.client_id.clone().into());
                json.insert("client_secret".into(), self.client_secret.clone().into());
                if let Some(v) = code_verifier {
                    json.insert("code_verifier".into(), v.to_string().into());
                }
                let body = serde_json::Value::Object(json).to_string();
                http_post_json(&token_url, &body, None).map_err(sanitize_token_error)?
            }
            provider::TokenExchangeShape::BasicAuthJsonBody => {
                let mut json = serde_json::Map::new();
                json.insert("grant_type".into(), "authorization_code".into());
                json.insert("code".into(), code.into());
                json.insert("redirect_uri".into(), self.redirect_uri.clone().into());
                if let Some(v) = code_verifier {
                    json.insert("code_verifier".into(), v.to_string().into());
                }
                let body = serde_json::Value::Object(json).to_string();
                http_post_json(
                    &token_url,
                    &body,
                    Some((&self.client_id, &self.client_secret)),
                )
                .map_err(sanitize_token_error)?
            }
        };
        parse_token_response(&out)
    }

    /// Exchange an authorization code for an access token. Thin wrapper
    /// around [`OAuthConfig::exchange_code_full`] for callers that only
    /// need the access token.
    pub fn exchange_code(&self, code: &str) -> Result<String, String> {
        Ok(self.exchange_code_full(code)?.access_token)
    }

    /// Exchange a refresh token for a fresh access (and possibly refresh)
    /// token. Wave 8 — implements `grant_type=refresh_token` per RFC
    /// 6749 §6, mirroring the four token-exchange shapes supported by
    /// `exchange_code_full_pkce`.
    ///
    /// Many providers (Google, Auth0, Okta) ROTATE the refresh token on
    /// each successful refresh and invalidate the old one — callers MUST
    /// upsert the returned `TokenSet` into the account store immediately
    /// or the next refresh will fail. The framework's
    /// [`AccountStore::ensure_fresh_access_token`] helper handles this
    /// atomically; downstream code should prefer it over calling this
    /// raw method directly.
    pub fn exchange_refresh_token(&self, refresh_token: &str) -> Result<TokenSet, String> {
        let spec = self.resolved_spec()?;
        let cfg = self.provider_cfg();
        let token_url = provider::resolve_endpoint(spec.token_url(), &cfg);

        let out = match spec.token_exchange() {
            provider::TokenExchangeShape::Standard => {
                let body = format!(
                    "refresh_token={rt}&{cid_param}={cid}&client_secret={secret}&grant_type=refresh_token",
                    rt = url_encode(refresh_token),
                    cid_param = spec.client_id_param(),
                    cid = url_encode(&self.client_id),
                    secret = url_encode(&self.client_secret),
                );
                http_post_form(&token_url, &body, true).map_err(sanitize_token_error)?
            }
            provider::TokenExchangeShape::AppleJwt => {
                // Apple's refresh flow signs a fresh client_secret JWT
                // every time — same as the initial exchange.
                let apple = self.apple.as_ref().ok_or(
                    "apple provider requires `apple` config (team_id, key_id, private_key_pem)",
                )?;
                let signed_secret = apple_jwt::mint_client_secret(apple, &self.client_id)?;
                let body = format!(
                    "refresh_token={rt}&client_id={cid}&client_secret={secret}&grant_type=refresh_token",
                    rt = url_encode(refresh_token),
                    cid = url_encode(&self.client_id),
                    secret = url_encode(&signed_secret),
                );
                http_post_form(&token_url, &body, true).map_err(sanitize_token_error)?
            }
            provider::TokenExchangeShape::BasicAuth => {
                let body = format!(
                    "refresh_token={rt}&grant_type=refresh_token",
                    rt = url_encode(refresh_token),
                );
                http_post_form_basic(&token_url, &body, &self.client_id, &self.client_secret)
                    .map_err(sanitize_token_error)?
            }
            provider::TokenExchangeShape::JsonBody => {
                let mut json = serde_json::Map::new();
                json.insert("grant_type".into(), "refresh_token".into());
                json.insert("refresh_token".into(), refresh_token.into());
                json.insert("client_id".into(), self.client_id.clone().into());
                json.insert("client_secret".into(), self.client_secret.clone().into());
                let body = serde_json::Value::Object(json).to_string();
                http_post_json(&token_url, &body, None).map_err(sanitize_token_error)?
            }
            provider::TokenExchangeShape::BasicAuthJsonBody => {
                let mut json = serde_json::Map::new();
                json.insert("grant_type".into(), "refresh_token".into());
                json.insert("refresh_token".into(), refresh_token.into());
                let body = serde_json::Value::Object(json).to_string();
                http_post_json(
                    &token_url,
                    &body,
                    Some((&self.client_id, &self.client_secret)),
                )
                .map_err(sanitize_token_error)?
            }
        };
        let mut tokens = parse_token_response(&out)?;
        // Token-rotation gotcha: providers that DO NOT rotate (Microsoft,
        // some OIDC servers) omit `refresh_token` from the response. Per
        // RFC 6749 §6 the prior refresh remains valid in that case;
        // copy it forward so the caller doesn't store None and lose the
        // ability to refresh again. Providers that DO rotate (Google,
        // Auth0) include the new value which overrides this default.
        if tokens.refresh_token.is_none() {
            tokens.refresh_token = Some(refresh_token.to_string());
        }
        Ok(tokens)
    }

    /// Fetch the authenticated user's email + display name using an access token.
    pub fn fetch_userinfo(&self, access_token: &str) -> Result<(String, Option<String>), String> {
        let info = self.fetch_userinfo_full(access_token)?;
        Ok((info.email, info.name))
    }

    /// Fetch the authenticated user's full identity info — email + name +
    /// the provider-stable account ID. Uses the spec's
    /// [`provider::UserinfoParser`] so adding a new provider is a
    /// table change, not a new branch.
    pub fn fetch_userinfo_full(&self, access_token: &str) -> Result<UserInfo, String> {
        // The id_token from the token response carries the identity
        // for Apple and similar; route to the dedicated entry point.
        // Apple's userinfo_url is None — this is the supported path.
        self.fetch_userinfo_with_id_token(access_token, None)
    }

    /// Fetch userinfo, falling back to the supplied id_token JWT when
    /// the provider has no userinfo endpoint (Apple). The id_token
    /// is the one returned by [`Self::exchange_code_full`] in
    /// [`TokenSet::id_token`].
    pub fn fetch_userinfo_with_id_token(
        &self,
        access_token: &str,
        id_token: Option<&str>,
    ) -> Result<UserInfo, String> {
        let spec = self.resolved_spec()?;
        let cfg = self.provider_cfg();

        // Apple — identity lives in the id_token, not a userinfo endpoint.
        if matches!(
            spec.userinfo_parser(),
            provider::UserinfoParser::AppleIdToken
        ) {
            let token =
                id_token.ok_or("apple login requires the id_token from the token response")?;
            return parse_apple_id_token(token, &self.provider);
        }

        // Linear is GraphQL — the userinfo "GET" is actually a POST
        // with a fixed query.
        if matches!(
            spec.userinfo_parser(),
            provider::UserinfoParser::LinearGraphql
        ) {
            return fetch_linear_userinfo(&self.provider, access_token);
        }

        let url = match spec.userinfo_url() {
            Some(u) => provider::resolve_endpoint(u, &cfg),
            None => {
                return Err(format!(
                    "provider {} has no userinfo endpoint",
                    self.provider
                ))
            }
        };
        let out = match spec.userinfo_method() {
            provider::UserinfoMethod::Get => http_get_bearer(&url, access_token),
            provider::UserinfoMethod::Post => http_post_bearer(&url, access_token),
        }
        .map_err(sanitize_token_error)?;
        let parsed: serde_json::Value =
            serde_json::from_str(&out).map_err(|e| format!("userinfo not valid JSON: {e}"))?;

        match spec.userinfo_parser() {
            provider::UserinfoParser::Oidc => {
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
            provider::UserinfoParser::GitHub => {
                let name = parsed
                    .get("name")
                    .and_then(|v| v.as_str())
                    .or_else(|| parsed.get("login").and_then(|v| v.as_str()))
                    .map(String::from);
                let email = parsed
                    .get("email")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let email = email
                    .or_else(|| fetch_github_primary_email(access_token).ok())
                    .ok_or("no accessible email on GitHub account")?;
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
            provider::UserinfoParser::Custom {
                id_path,
                email_path,
                name_path,
            } => {
                let provider_account_id = json_pointer_string(&parsed, id_path)
                    .ok_or_else(|| format!("no id at {id_path} in userinfo"))?;
                let raw_email = json_pointer_string(&parsed, email_path)
                    .ok_or_else(|| format!("no email at {email_path} in userinfo"))?;
                // Twitter/Reddit don't expose real emails — they map a
                // username into the email slot. Tag it so account
                // policies can distinguish "real verified email" from
                // "we made this up." `.invalid` is reserved by RFC 6761.
                let email = if !raw_email.contains('@') {
                    let domain = match self.provider.as_str() {
                        "twitter" => "x.invalid",
                        "reddit" => "reddit.invalid",
                        other => return Err(format!(
                            "{other}: userinfo `email` field is not an email address (got {raw_email:?}); refusing to synthesize",
                        )),
                    };
                    format!("{raw_email}@{domain}")
                } else {
                    raw_email
                };
                let name = name_path.and_then(|p| json_pointer_string(&parsed, p));
                Ok(UserInfo {
                    provider: self.provider.clone(),
                    provider_account_id,
                    email,
                    name,
                })
            }
            provider::UserinfoParser::AppleIdToken => unreachable!("handled above"),
            provider::UserinfoParser::LinearGraphql => unreachable!("handled above"),
        }
    }
}

/// PKCE pair — the verifier stays server-side until token exchange,
/// the (S256-hashed) challenge goes on the auth URL.
struct PkcePair {
    code_verifier: String,
    code_challenge: String,
}

/// Generate a PKCE pair: random 43-char verifier + S256 challenge.
/// RFC 7636 §4.1 permits 43–128 chars from `[A-Za-z0-9-._~]`. 32
/// random bytes URL-base64-encoded comes out to exactly 43 chars.
fn generate_pkce() -> PkcePair {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let code_verifier = apple_jwt::base64_url(bytes);
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(code_verifier.as_bytes());
    let code_challenge = apple_jwt::base64_url(hasher.finalize());
    PkcePair {
        code_verifier,
        code_challenge,
    }
}

/// Decode an Apple id_token JWT and pull the identity claims.
///
/// **Trust assumption:** the caller MUST have obtained this token
/// via the back-channel `/auth/token` exchange (mutually authenticated
/// TLS to `appleid.apple.com`). Under that assumption no third party
/// can have substituted a forged JWT, so we skip signature
/// verification.
///
/// **DO NOT call this on a JWT supplied by the client** (e.g. a
/// "post your id_token to me" mobile-SDK flow). For those paths,
/// implement Apple JWKS verification: fetch
/// `https://appleid.apple.com/auth/keys`, verify the RS256
/// signature, then check `iss == "https://appleid.apple.com"`,
/// `aud == client_id`, and `exp > now`. Pylon doesn't ship that
/// verifier yet — apps that need it can compose `crate::jwt::verify`
/// against a JWKS-loaded RSA key.
///
/// This function is private (`fn`, not `pub fn`) precisely so it
/// can't be misused by an external caller. The only call site is
/// [`OAuthConfig::fetch_userinfo_with_id_token`] which is reached
/// only via the OAuth callback handler, which only processes
/// back-channel-exchanged tokens.
fn parse_apple_id_token(id_token: &str, provider: &str) -> Result<UserInfo, String> {
    let mut parts = id_token.split('.');
    let _header = parts.next().ok_or("apple id_token: missing header")?;
    let claims_b64 = parts.next().ok_or("apple id_token: missing claims")?;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    let claims_bytes = URL_SAFE_NO_PAD
        .decode(claims_b64)
        .map_err(|e| format!("apple id_token claims not base64: {e}"))?;
    let claims: serde_json::Value = serde_json::from_slice(&claims_bytes)
        .map_err(|e| format!("apple id_token claims not JSON: {e}"))?;
    let provider_account_id = claims
        .get("sub")
        .and_then(|v| v.as_str())
        .ok_or("apple id_token: missing sub")?
        .to_string();
    let email = claims
        .get("email")
        .and_then(|v| v.as_str())
        .ok_or("apple id_token: missing email (was the `email` scope requested?)")?
        .to_string();
    Ok(UserInfo {
        provider: provider.to_string(),
        provider_account_id,
        email,
        name: None, // Apple sends `name` as a separate form field on FIRST signup only.
    })
}

/// Strip provider error bodies of secrets before they propagate to
/// logs / `oauth_error_message` redirect URLs.
///
/// **Why:** Several token endpoints echo the request body (or pieces
/// of it) on auth failure. Without this, a misconfigured deployment
/// can leak `client_secret`, the Apple JWT, or even the auth `code`
/// into the user's browser history and CDN logs.
///
/// Covers both shapes echoed by real providers:
///   - form / query: `client_secret=sk_…`
///   - JSON: `"client_secret":"sk_…"` (Notion, Atlassian)
fn sanitize_token_error(err: String) -> String {
    const SENSITIVE: &[&str] = &[
        "client_secret",
        "code_verifier",
        "client_assertion",
        "refresh_token",
        "access_token",
        "id_token",
        // The auth `code` itself is single-use but still sensitive
        // until the token endpoint consumes it — and many providers
        // echo it back on a 4xx token-exchange error before the
        // attacker has had a chance to redeem it.
        "code",
    ];
    let mut out = err;
    for key in SENSITIVE {
        out = redact_param_form(&out, key);
        out = redact_param_json(&out, key);
    }
    out
}

/// Replace the value of `key=…` (form/query string) with `***`,
/// terminating at any of `& \n " '`. UTF-8 safe — uses `char_indices`
/// so a stray multibyte character before a sensitive key won't panic.
fn redact_param_form(input: &str, key: &str) -> String {
    let needle = format!("{key}=");
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if input[i..].starts_with(&needle) {
            out.push_str(&needle);
            out.push_str("***");
            i += needle.len();
            // Skip until a terminator. char_indices keeps i aligned
            // to char boundaries.
            while let Some((rel, ch)) = input[i..].char_indices().next() {
                if matches!(ch, '&' | '\n' | '"' | ' ' | '\'') {
                    i += rel;
                    break;
                }
                i += rel + ch.len_utf8();
            }
        } else {
            // Advance by one full char to stay UTF-8 aligned.
            let (_, ch) = input[i..].char_indices().next().expect("non-empty");
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// Replace the value in `"key":"…"` with `***`. Case-sensitive,
/// tolerant of whitespace between `:` and the value (per JSON).
fn redact_param_json(input: &str, key: &str) -> String {
    let needle = format!("\"{key}\"");
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if !input[i..].starts_with(&needle) {
            let (_, ch) = input[i..].char_indices().next().expect("non-empty");
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }
        // Found `"key"`. Walk forward over `:` + optional whitespace,
        // then `"`, then the value, then closing `"`. If anything
        // is off (not actually a string-valued field) bail and
        // copy verbatim.
        let mut j = i + needle.len();
        // optional whitespace
        while let Some((_, ch)) = input[j..].char_indices().next() {
            if !ch.is_whitespace() {
                break;
            }
            j += ch.len_utf8();
        }
        if !input[j..].starts_with(':') {
            // Not a key-value form (could be in an array, etc.).
            out.push_str(&input[i..j]);
            i = j;
            continue;
        }
        j += 1;
        while let Some((_, ch)) = input[j..].char_indices().next() {
            if !ch.is_whitespace() {
                break;
            }
            j += ch.len_utf8();
        }
        if !input[j..].starts_with('"') {
            out.push_str(&input[i..j]);
            i = j;
            continue;
        }
        let value_start = j + 1;
        // Find the closing `"`, honoring `\"` escapes.
        let mut k = value_start;
        let mut prev_backslash = false;
        let mut closing: Option<usize> = None;
        while k < input.len() {
            let (_, ch) = input[k..].char_indices().next().expect("non-empty");
            if ch == '"' && !prev_backslash {
                closing = Some(k);
                break;
            }
            prev_backslash = ch == '\\' && !prev_backslash;
            k += ch.len_utf8();
        }
        match closing {
            Some(end) => {
                out.push_str(&input[i..value_start]);
                out.push_str("***");
                out.push('"');
                i = end + 1;
            }
            None => {
                // Malformed JSON, redact to end of input to be safe.
                out.push_str(&input[i..value_start]);
                out.push_str("***");
                i = input.len();
            }
        }
    }
    out
}

/// Linear's userinfo lives behind a GraphQL endpoint — the bearer
/// token is the same OAuth access token, but the request is a POST
/// with a fixed query. Kept as a separate fn so the main fetcher
/// stays uniform across the other parsers.
fn fetch_linear_userinfo(provider: &str, access_token: &str) -> Result<UserInfo, String> {
    let body = r#"{"query":"query { viewer { id email name } }"}"#;
    let agent = ureq_agent();
    let resp = agent
        .post("https://api.linear.app/graphql")
        .set("Authorization", &format!("Bearer {access_token}"))
        .set("Content-Type", "application/json")
        .set("Accept", "application/json")
        .send_string(body)
        .map_err(|e| format!("linear graphql: {e}"))?;
    let out = resp.into_string().map_err(|e| format!("read body: {e}"))?;
    let parsed: serde_json::Value =
        serde_json::from_str(&out).map_err(|e| format!("linear graphql not JSON: {e}"))?;
    let viewer = parsed
        .pointer("/data/viewer")
        .ok_or("linear graphql: no /data/viewer")?;
    let provider_account_id = viewer
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("linear graphql: no id")?
        .to_string();
    let email = viewer
        .get("email")
        .and_then(|v| v.as_str())
        .ok_or("linear graphql: no email")?
        .to_string();
    let name = viewer
        .get("name")
        .and_then(|v| v.as_str())
        .map(String::from);
    Ok(UserInfo {
        provider: provider.to_string(),
        provider_account_id,
        email,
        name,
    })
}

/// JSON-pointer (RFC 6901) string extraction. Returns `None` for
/// missing paths or non-string values. Numeric ids (Discord's `id`,
/// Roblox's `sub`) are coerced to strings.
fn json_pointer_string(v: &serde_json::Value, path: &str) -> Option<String> {
    let node = v.pointer(path)?;
    if let Some(s) = node.as_str() {
        return Some(s.to_string());
    }
    if let Some(n) = node.as_i64() {
        return Some(n.to_string());
    }
    if let Some(n) = node.as_u64() {
        return Some(n.to_string());
    }
    None
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

/// POST a form body using HTTP Basic auth for the client credentials.
/// Used by Spotify, Reddit, Figma, Zoom, PayPal — providers that
/// mandate Basic auth on the token endpoint.
fn http_post_form_basic(
    url: &str,
    body: &str,
    client_id: &str,
    client_secret: &str,
) -> Result<String, String> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let creds = format!("{client_id}:{client_secret}");
    let basic = STANDARD.encode(creds.as_bytes());
    let agent = ureq_agent();
    match agent
        .post(url)
        .set("Content-Type", "application/x-www-form-urlencoded")
        .set("Accept", "application/json")
        .set("Authorization", &format!("Basic {basic}"))
        .send_string(body)
    {
        Ok(resp) => resp.into_string().map_err(|e| format!("read body: {e}")),
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(format!("HTTP {code}: {body}"))
        }
        Err(e) => Err(format!("HTTP error: {e}")),
    }
}

/// POST a JSON body, optionally with HTTP Basic auth. Used by
/// Notion (Basic + JSON) and Atlassian (JSON only) — both reject
/// form-encoded bodies on their token endpoints.
fn http_post_json(
    url: &str,
    body: &str,
    basic_creds: Option<(&str, &str)>,
) -> Result<String, String> {
    let agent = ureq_agent();
    let mut req = agent
        .post(url)
        .set("Content-Type", "application/json")
        .set("Accept", "application/json");
    if let Some((id, secret)) = basic_creds {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let creds = STANDARD.encode(format!("{id}:{secret}").as_bytes());
        req = req.set("Authorization", &format!("Basic {creds}"));
    }
    // Notion requires the API version header on every call, even the
    // token exchange. Using a recent stable version.
    req = req.set("Notion-Version", "2022-06-28");
    match req.send_string(body) {
        Ok(resp) => resp.into_string().map_err(|e| format!("read body: {e}")),
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(format!("HTTP {code}: {body}"))
        }
        Err(e) => Err(format!("HTTP error: {e}")),
    }
}

/// POST with empty body + bearer auth. Used for Dropbox userinfo
/// (an RPC-style endpoint that requires POST instead of GET).
fn http_post_bearer(url: &str, token: &str) -> Result<String, String> {
    let agent = ureq_agent();
    match agent
        .post(url)
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
    ///
    /// For each builtin provider (and any `oidc_issuer`-configured
    /// IdP), looks for `PYLON_OAUTH_<PROVIDER>_CLIENT_ID` /
    /// `_CLIENT_SECRET` / `_REDIRECT`. Apple additionally requires
    /// `_TEAM_ID`, `_KEY_ID`, `_PRIVATE_KEY` (PEM contents or path).
    /// Microsoft accepts an optional `_TENANT`.
    ///
    /// Generic OIDC: any env var matching
    /// `PYLON_OAUTH_<NAME>_OIDC_ISSUER` registers a provider with id
    /// `<name>` (lowercased) using the discovered endpoints. Useful
    /// for Auth0, Okta, Keycloak, Cognito, Logto, Authentik, etc.
    pub fn from_env() -> Self {
        let mut reg = Self::new();

        for spec in provider::builtin::all() {
            let upper = spec.id.to_ascii_uppercase();
            let prefix = format!("PYLON_OAUTH_{upper}");
            let id = match std::env::var(format!("{prefix}_CLIENT_ID")) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let secret = match std::env::var(format!("{prefix}_CLIENT_SECRET")) {
                Ok(v) => v,
                // Apple's "client_secret" is synthesized — allow blank.
                Err(_) if spec.id == "apple" => String::new(),
                Err(_) => continue,
            };
            let redirect_uri = std::env::var(format!("{prefix}_REDIRECT"))
                .unwrap_or_else(|_| format!("http://localhost:3000/api/auth/callback/{}", spec.id));
            let scopes_override = std::env::var(format!("{prefix}_SCOPES")).ok();
            let tenant = std::env::var(format!("{prefix}_TENANT")).ok();

            let apple = if spec.id == "apple" {
                match (
                    std::env::var(format!("{prefix}_TEAM_ID")),
                    std::env::var(format!("{prefix}_KEY_ID")),
                    std::env::var(format!("{prefix}_PRIVATE_KEY")),
                ) {
                    (Ok(team_id), Ok(key_id), Ok(private_key_pem)) => Some(provider::AppleConfig {
                        team_id,
                        key_id,
                        private_key_pem,
                    }),
                    _ => continue, // Apple requires the JWT material to function.
                }
            } else {
                None
            };

            reg.register(OAuthConfig {
                provider: spec.id.to_string(),
                client_id: id,
                client_secret: secret,
                redirect_uri,
                scopes_override,
                tenant,
                apple,
                oidc_issuer: None,
            });
        }

        // Generic OIDC providers — scan PYLON_OAUTH_<NAME>_OIDC_ISSUER.
        for (key, issuer) in std::env::vars() {
            let Some(rest) = key.strip_prefix("PYLON_OAUTH_") else {
                continue;
            };
            let Some(name_upper) = rest.strip_suffix("_OIDC_ISSUER") else {
                continue;
            };
            let name = name_upper.to_ascii_lowercase();
            if provider::find_spec(&name).is_some() {
                continue; // already handled as a builtin
            }
            let prefix = format!("PYLON_OAUTH_{name_upper}");
            let id = match std::env::var(format!("{prefix}_CLIENT_ID")) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let secret = std::env::var(format!("{prefix}_CLIENT_SECRET")).unwrap_or_default();
            let redirect_uri = std::env::var(format!("{prefix}_REDIRECT"))
                .unwrap_or_else(|_| format!("http://localhost:3000/api/auth/callback/{name}"));
            reg.register(OAuthConfig {
                provider: name,
                client_id: id,
                client_secret: secret,
                redirect_uri,
                scopes_override: std::env::var(format!("{prefix}_SCOPES")).ok(),
                tenant: None,
                apple: None,
                oidc_issuer: Some(issuer),
            });
        }

        reg
    }

    /// Iterate over registered provider ids — used by routes/auth.rs
    /// to expose `/api/auth/providers` and to validate
    /// `/api/auth/login/<id>` paths against the configured set.
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.providers.keys().map(|s| s.as_str())
    }

    /// Process-wide cached registry. Built once on first use from
    /// `from_env`; subsequent calls are zero-cost. Routes use this
    /// to avoid the ~150 syscalls `from_env` does per call.
    ///
    /// **Trade-off:** env changes after server start aren't picked up
    /// without a restart — same as every other Pylon env-var path.
    pub fn shared() -> &'static OAuthRegistry {
        static CELL: std::sync::OnceLock<OAuthRegistry> = std::sync::OnceLock::new();
        CELL.get_or_init(Self::from_env)
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
    /// PKCE code_verifier when the provider requires PKCE. Set by the
    /// `/api/auth/login/<provider>` start route via
    /// [`OAuthConfig::auth_url_with_pkce`]; replayed on token exchange
    /// in the callback. `None` for non-PKCE providers.
    pub pkce_verifier: Option<String>,
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
        self.create_with_pkce(provider, callback_url, error_callback_url, None)
    }

    /// Same as [`Self::create`] but accepts a PKCE verifier to stash
    /// alongside the state record. The callback handler reads it back
    /// out and replays it in the token exchange.
    pub fn create_with_pkce(
        &self,
        provider: &str,
        callback_url: &str,
        error_callback_url: &str,
        pkce_verifier: Option<String>,
    ) -> String {
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
            pkce_verifier,
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
        self.create_with_device(user_id, None)
    }

    /// Create a session with an attached device label. The label is
    /// what `/api/auth/sessions` shows to the user — typically the
    /// parsed User-Agent (see [`crate::device::parse_user_agent`]).
    /// Pass `None` (or use `create()`) for non-browser flows where
    /// no UA is available.
    pub fn create_with_device(&self, user_id: String, device: Option<String>) -> Session {
        let mut session = Session::with_lifetime(user_id, self.default_lifetime_secs);
        session.device = device;
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
        // Use the store's configured lifetime so a manifest-set
        // `auth.session.expires_in` survives session refresh. Previous
        // bug: `Session::new(...)` baked in 30 days regardless of
        // config — apps with a custom lifetime got the right value on
        // first sign-in and lost it on the next refresh.
        let mut new = Session::with_lifetime(old.user_id.clone(), self.default_lifetime_secs);
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

    /// True if the access token will expire within `buffer_secs`. Used
    /// by the auto-refresh path so apps don't pull a token that's about
    /// to expire mid-flight. Non-expiring tokens (GitHub Classic) always
    /// return false — refresh-on-401 is the right pattern there.
    ///
    /// `buffer_secs` is typically 60–300 (1–5 min). The upstream
    /// recommendation is to refresh ~5 minutes before expiry.
    pub fn needs_refresh(&self, buffer_secs: u64) -> bool {
        match self.access_token_expires_at {
            Some(ts) => now_secs().saturating_add(buffer_secs) >= ts,
            None => false,
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
    /// Remove every account link for a user. Used during account
    /// deletion to ensure no OAuth references survive past a user row
    /// delete. Default implementation walks `find_for_user` + `unlink`;
    /// SQL backends can override with a single DELETE.
    fn delete_for_user(&self, user_id: &str) -> usize {
        let accounts = self.find_for_user(user_id);
        let n = accounts.len();
        for a in accounts {
            self.unlink(&a.provider_id, &a.account_id);
        }
        n
    }
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
    pub fn delete_for_user(&self, user_id: &str) -> usize {
        self.backend.delete_for_user(user_id)
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

    /// Wave-8 — auto-refresh helper. Looks up the account by
    /// `(provider_id, account_id)`, returns the existing access token if
    /// it has more than `buffer_secs` of life left, otherwise calls the
    /// provider's `grant_type=refresh_token` endpoint, upserts the new
    /// bundle, and returns the refreshed account.
    ///
    /// Errors:
    /// - `ACCOUNT_NOT_FOUND` — no row for that provider/account pair.
    /// - `NO_REFRESH_TOKEN` — row exists but never stored a refresh
    ///   token (provider didn't issue one, or operator scrubbed it).
    /// - `REFRESH_FAILED` — provider rejected the refresh (revoked,
    ///   expired). Caller should re-prompt the user to OAuth again.
    /// - `PROVIDER_NOT_CONFIGURED` — `OAuthRegistry` has no entry for
    ///   `provider_id` in this process. Operator misconfig.
    ///
    /// Atomic upsert: the new bundle is persisted BEFORE the function
    /// returns, so a caller crashing right after won't have a stale row
    /// in the store. The OAuth-refresh-twice race (two callers refresh
    /// concurrently, one wins) is mitigated by the provider — most
    /// providers either accept both refreshes or reject the second
    /// with INVALID_GRANT. Pylon doesn't add its own lock since the
    /// per-user refresh rate is naturally low (once per ~hour).
    pub fn ensure_fresh_access_token(
        &self,
        provider_id: &str,
        account_id: &str,
        buffer_secs: u64,
    ) -> Result<Account, RefreshError> {
        let account = self
            .find_by_provider(provider_id, account_id)
            .ok_or(RefreshError::AccountNotFound)?;
        if !account.needs_refresh(buffer_secs) {
            return Ok(account);
        }
        let refresh = account
            .refresh_token
            .as_deref()
            .ok_or(RefreshError::NoRefreshToken)?;
        let registry = OAuthRegistry::shared();
        let cfg = registry
            .get(provider_id)
            .cloned()
            .ok_or(RefreshError::ProviderNotConfigured)?;
        let new_tokens = cfg
            .exchange_refresh_token(refresh)
            .map_err(|e| RefreshError::RefreshFailed(e))?;
        // Preserve created_at + user_id; rotate the token bundle.
        let mut updated = account.clone();
        updated.access_token = Some(new_tokens.access_token.clone());
        if let Some(rt) = new_tokens.refresh_token {
            updated.refresh_token = Some(rt);
        }
        if new_tokens.id_token.is_some() {
            updated.id_token = new_tokens.id_token;
        }
        updated.access_token_expires_at = new_tokens.expires_at;
        if let Some(scope) = new_tokens.scope {
            updated.scope = Some(scope);
        }
        updated.updated_at = now_secs();
        self.upsert(&updated);
        Ok(updated)
    }
}

/// Reasons [`AccountStore::ensure_fresh_access_token`] can fail. Codes
/// match the wire codes used by `/api/auth/oauth/refresh/<provider>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshError {
    AccountNotFound,
    NoRefreshToken,
    RefreshFailed(String),
    ProviderNotConfigured,
}

impl RefreshError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::AccountNotFound => "ACCOUNT_NOT_FOUND",
            Self::NoRefreshToken => "NO_REFRESH_TOKEN",
            Self::RefreshFailed(_) => "REFRESH_FAILED",
            Self::ProviderNotConfigured => "PROVIDER_NOT_CONFIGURED",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::AccountNotFound => "no linked account for that provider".into(),
            Self::NoRefreshToken => "account has no stored refresh token".into(),
            Self::RefreshFailed(e) => format!("refresh failed: {e}"),
            Self::ProviderNotConfigured => "OAuth provider not configured on this server".into(),
        }
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
    fn from_api_key_carries_scope_metadata() {
        let ctx =
            AuthContext::from_api_key("user-1".into(), "key_abc".into(), Some("read,write".into()));
        assert!(ctx.is_authenticated());
        assert!(ctx.is_api_key_auth());
        assert_eq!(ctx.user_id.as_deref(), Some("user-1"));
        assert_eq!(ctx.api_key_id.as_deref(), Some("key_abc"));
        assert_eq!(ctx.api_key_scopes.as_deref(), Some("read,write"));
    }

    #[test]
    fn session_auth_is_not_api_key_auth() {
        let ctx = AuthContext::authenticated("user-1".into());
        assert!(!ctx.is_api_key_auth());
        assert!(ctx.api_key_id.is_none());
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
            ..Default::default()
        });
        let config = reg.get("google").unwrap();
        assert_eq!(config.client_id, "test-id");
        assert!(config.auth_url().contains("accounts.google.com"));
    }

    // -- Spec-driven provider routing --

    /// Every builtin provider must produce a non-empty auth_url +
    /// token_url when wired with placeholder credentials. This is the
    /// regression test for the table-driven refactor: a typo in any
    /// `ProviderSpec` field that breaks URL formatting will trip here
    /// before it reaches a user.
    #[test]
    fn every_builtin_provider_routes_through_oauth_config() {
        for spec in provider::builtin::all() {
            let cfg = OAuthConfig {
                provider: spec.id.into(),
                client_id: "cid".into(),
                client_secret: "csecret".into(),
                redirect_uri: "https://app/cb".into(),
                tenant: if spec.id == "microsoft" {
                    Some("contoso".into())
                } else {
                    None
                },
                apple: if spec.id == "apple" {
                    Some(provider::AppleConfig {
                        team_id: "T".into(),
                        key_id: "K".into(),
                        private_key_pem: "no".into(),
                    })
                } else {
                    None
                },
                ..Default::default()
            };
            let auth = cfg.auth_url();
            assert!(!auth.is_empty(), "{}: empty auth_url", spec.id);
            // TikTok uses `client_key`; everyone else uses `client_id`.
            let expected_param = format!("{}=cid", spec.client_id_param);
            assert!(
                auth.contains(&expected_param),
                "{}: missing {}; got auth_url: {}",
                spec.id,
                expected_param,
                auth,
            );
            assert!(!cfg.token_url().is_empty(), "{}: empty token_url", spec.id);
            // Apple requires response_mode=form_post in the auth URL.
            if spec.id == "apple" {
                assert!(
                    auth.contains("response_mode=form_post"),
                    "apple auth_url must include response_mode=form_post; got {auth}"
                );
            }
        }
    }

    /// Microsoft uses `{tenant}` placeholder substitution — the
    /// configured tenant must end up in both auth + token URLs.
    #[test]
    fn microsoft_tenant_placeholder_resolves() {
        let cfg = OAuthConfig {
            provider: "microsoft".into(),
            client_id: "id".into(),
            client_secret: "secret".into(),
            redirect_uri: "https://app/cb".into(),
            tenant: Some("contoso.onmicrosoft.com".into()),
            ..Default::default()
        };
        assert!(cfg.auth_url().contains("/contoso.onmicrosoft.com/"));
        assert!(cfg.token_url().contains("/contoso.onmicrosoft.com/"));
    }

    /// Microsoft without a tenant defaults to `common` (any account).
    #[test]
    fn microsoft_default_tenant_common() {
        let cfg = OAuthConfig {
            provider: "microsoft".into(),
            client_id: "id".into(),
            client_secret: "secret".into(),
            redirect_uri: "https://app/cb".into(),
            ..Default::default()
        };
        assert!(cfg.auth_url().contains("/common/"));
        assert!(cfg.token_url().contains("/common/"));
    }

    /// `scopes_override` replaces the spec default — used for GitHub
    /// `repo` scope or Google calendar scopes.
    #[test]
    fn scopes_override_replaces_spec_default() {
        let cfg = OAuthConfig {
            provider: "github".into(),
            client_id: "id".into(),
            client_secret: "secret".into(),
            redirect_uri: "https://app/cb".into(),
            scopes_override: Some("repo user:email".into()),
            ..Default::default()
        };
        let auth = cfg.auth_url();
        // url-encoded "repo user:email" → "repo%20user%3Aemail"
        assert!(auth.contains("scope=repo%20user%3Aemail"), "got: {auth}");
    }

    /// Apple's `client_secret` is minted as a JWT — passing a bad PEM
    /// must surface the signing error, not silently send the literal
    /// string. The mint path is tested in `apple_jwt::tests`; this
    /// asserts the wiring delegates to it.
    #[test]
    fn apple_exchange_requires_apple_config() {
        let cfg = OAuthConfig {
            provider: "apple".into(),
            client_id: "com.example.app".into(),
            client_secret: String::new(),
            redirect_uri: "https://app/cb".into(),
            apple: None, // missing!
            ..Default::default()
        };
        let err = cfg.exchange_code_full("x").unwrap_err();
        assert!(err.contains("apple provider requires"), "got: {err}");
    }

    /// OIDC discovery cache: priming with a synthetic spec lets us
    /// route an issuer-configured provider without touching the
    /// network. Validates that `oidc_issuer` short-circuits the
    /// builtin lookup.
    #[test]
    fn oidc_issuer_uses_discovered_endpoints() {
        let issuer = "https://acme.test.invalid";
        provider::oidc_cache::insert_for_test(
            issuer,
            provider::DiscoveredSpec {
                auth_url: "https://acme.test.invalid/authorize".into(),
                token_url: "https://acme.test.invalid/oauth/token".into(),
                userinfo_url: Some("https://acme.test.invalid/userinfo".into()),
                scopes: "openid email profile".into(),
                userinfo_parser: provider::UserinfoParser::Oidc,
                token_exchange: provider::TokenExchangeShape::Standard,
            },
        );
        let cfg = OAuthConfig {
            provider: "auth0".into(), // not a builtin id
            client_id: "id".into(),
            client_secret: "secret".into(),
            redirect_uri: "https://app/cb".into(),
            oidc_issuer: Some(issuer.into()),
            ..Default::default()
        };
        assert!(cfg
            .auth_url()
            .starts_with("https://acme.test.invalid/authorize?"));
        assert_eq!(cfg.token_url(), "https://acme.test.invalid/oauth/token");
        assert_eq!(cfg.userinfo_url(), "https://acme.test.invalid/userinfo");
    }

    // -- Codex review regression tests (P1/P2 from Wave 1 review) --

    /// P1: Apple's auth URL MUST include response_mode=form_post when
    /// requesting name/email scopes, otherwise Apple rejects with
    /// "invalid_request".
    #[test]
    fn apple_auth_url_includes_form_post() {
        let cfg = OAuthConfig {
            provider: "apple".into(),
            client_id: "com.example.app".into(),
            client_secret: String::new(),
            redirect_uri: "https://app/cb".into(),
            apple: Some(provider::AppleConfig {
                team_id: "T".into(),
                key_id: "K".into(),
                private_key_pem: "no".into(),
            }),
            ..Default::default()
        };
        let auth = cfg.auth_url();
        assert!(auth.contains("response_mode=form_post"), "got: {auth}");
        // Apple identity comes from id_token, so userinfo_url is empty.
        assert_eq!(cfg.userinfo_url(), "");
    }

    /// P1: Apple identity is extracted from the id_token JWT
    /// (Apple has no userinfo endpoint). `fetch_userinfo_with_id_token`
    /// must decode the claims; `fetch_userinfo_full` (no id_token)
    /// must surface a clear error.
    #[test]
    fn apple_id_token_decode_extracts_identity() {
        // Synthesize an unsigned JWT with realistic Apple claims.
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"{\"alg\":\"none\"}");
        use base64::Engine;
        let claims = serde_json::json!({
            "iss": "https://appleid.apple.com",
            "sub": "001234.abc.def",
            "aud": "com.example.app",
            "email": "user@privaterelay.appleid.com",
            "email_verified": "true",
        });
        let claims_b64 =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string().as_bytes());
        let id_token = format!("{header}.{claims_b64}.signature_ignored");

        let cfg = OAuthConfig {
            provider: "apple".into(),
            client_id: "com.example.app".into(),
            client_secret: String::new(),
            redirect_uri: "https://app/cb".into(),
            apple: Some(provider::AppleConfig {
                team_id: "T".into(),
                key_id: "K".into(),
                private_key_pem: "no".into(),
            }),
            ..Default::default()
        };
        let info = cfg
            .fetch_userinfo_with_id_token("ignored", Some(&id_token))
            .expect("apple id_token decode");
        assert_eq!(info.provider_account_id, "001234.abc.def");
        assert_eq!(info.email, "user@privaterelay.appleid.com");

        // Without an id_token the call must fail loud, not silently
        // try to hit a non-existent userinfo endpoint.
        let err = cfg.fetch_userinfo_full("token").unwrap_err();
        assert!(err.contains("apple login requires"), "got: {err}");
    }

    /// P1: Twitter/X requires PKCE — `auth_url_with_pkce` must mint a
    /// verifier, embed the SHA-256 challenge in the auth URL, and
    /// return the verifier for the callback to replay.
    #[test]
    fn twitter_auth_url_includes_pkce() {
        let cfg = OAuthConfig {
            provider: "twitter".into(),
            client_id: "tw_client".into(),
            client_secret: "tw_secret".into(),
            redirect_uri: "https://app/cb".into(),
            ..Default::default()
        };
        let (url, verifier) = cfg.auth_url_with_pkce("state123").expect("twitter pkce");
        let v = verifier.expect("twitter must produce verifier");
        assert!(v.len() >= 43, "PKCE verifier must be 43+ chars: got {v}");
        assert!(url.contains("code_challenge="), "got: {url}");
        assert!(url.contains("code_challenge_method=S256"), "got: {url}");

        // Non-PKCE provider must NOT add a code_challenge.
        let google = OAuthConfig {
            provider: "google".into(),
            client_id: "g".into(),
            client_secret: "g".into(),
            redirect_uri: "https://app/cb".into(),
            ..Default::default()
        };
        let (gurl, gverifier) = google.auth_url_with_pkce("st").expect("google");
        assert!(gverifier.is_none(), "google should not add PKCE");
        assert!(!gurl.contains("code_challenge"), "got: {gurl}");
    }

    /// P2: TikTok uses `client_key` (not `client_id`) and joins
    /// scopes with commas (not spaces).
    #[test]
    fn tiktok_uses_client_key_and_comma_scopes() {
        let cfg = OAuthConfig {
            provider: "tiktok".into(),
            client_id: "tk_client".into(),
            client_secret: "tk_secret".into(),
            redirect_uri: "https://app/cb".into(),
            scopes_override: Some("user.info.basic video.list".into()),
            ..Default::default()
        };
        let auth = cfg.auth_url();
        assert!(auth.contains("client_key=tk_client"), "got: {auth}");
        // Comma-separated, url-encoded → "user.info.basic%2Cvideo.list"
        assert!(auth.contains("user.info.basic%2Cvideo.list"), "got: {auth}");
        // Should NOT use the standard space separator.
        assert!(
            !auth.contains("user.info.basic%20video.list"),
            "got: {auth}"
        );
    }

    /// P2: `code` MUST be url-encoded in the token-exchange body.
    /// Auth codes can contain reserved characters (`+`, `=`, `/`) that
    /// would otherwise corrupt the form body.
    #[test]
    fn token_exchange_url_encodes_code() {
        // We can't hit the network in a unit test, so this asserts
        // via the `apple_exchange_requires_apple_config` shape — if
        // we DID have a working apple config, encoding would happen
        // before the network call. Instead, verify by calling the
        // helper used internally:
        let raw = "code+with/special=chars";
        let encoded = url_encode(raw);
        assert!(!encoded.contains('+'));
        assert!(!encoded.contains('/'));
        assert!(!encoded.contains('='));
        assert!(encoded.contains("%2B"));
        assert!(encoded.contains("%2F"));
        assert!(encoded.contains("%3D"));
    }

    /// P1: Token-endpoint error bodies must NOT propagate
    /// `client_secret`, `code_verifier`, or other sensitive form
    /// fields that providers sometimes echo back on auth failure.
    #[test]
    fn sanitize_token_error_redacts_secrets() {
        let raw = "HTTP 400: error=invalid_grant&client_secret=sk_real_secret_value&code_verifier=verifierxyz&hint=check%20your%20code";
        let scrubbed = sanitize_token_error(raw.into());
        assert!(!scrubbed.contains("sk_real_secret_value"));
        assert!(!scrubbed.contains("verifierxyz"));
        assert!(scrubbed.contains("client_secret=***"));
        assert!(scrubbed.contains("code_verifier=***"));
        // Non-sensitive context preserved.
        assert!(scrubbed.contains("invalid_grant"));
        assert!(scrubbed.contains("hint=check%20your%20code"));
    }

    /// P1 (codex round-2): JSON-shaped error bodies (Notion,
    /// Atlassian) must also have their secret fields redacted.
    #[test]
    fn sanitize_token_error_redacts_json_secrets() {
        let raw = r#"HTTP 400: {"error":"invalid_grant","client_secret":"sk_jsonleak","refresh_token":"rt_abcxyz","id_token":"ey.payload.sig"}"#;
        let scrubbed = sanitize_token_error(raw.into());
        assert!(!scrubbed.contains("sk_jsonleak"), "got: {scrubbed}");
        assert!(!scrubbed.contains("rt_abcxyz"), "got: {scrubbed}");
        assert!(!scrubbed.contains("ey.payload.sig"), "got: {scrubbed}");
        assert!(
            scrubbed.contains(r#""client_secret":"***""#),
            "got: {scrubbed}"
        );
        assert!(
            scrubbed.contains(r#""refresh_token":"***""#),
            "got: {scrubbed}"
        );
        assert!(scrubbed.contains(r#""id_token":"***""#), "got: {scrubbed}");
        assert!(scrubbed.contains("invalid_grant"));
    }

    /// P2 (codex round-2): redact_param_form must NOT panic on
    /// multibyte chars before the sensitive key. Earlier byte-index
    /// implementation hit `panicked at byte index N is not a char
    /// boundary` on bodies with emoji or non-ASCII text.
    #[test]
    fn sanitize_token_error_handles_utf8() {
        let raw = "HTTP 400: ⚠️ provider says the secret is wrong: client_secret=sk_x";
        let scrubbed = sanitize_token_error(raw.into());
        assert!(
            scrubbed.contains("⚠️"),
            "non-ASCII chars must survive: {scrubbed}"
        );
        assert!(!scrubbed.contains("sk_x"));
        assert!(scrubbed.contains("client_secret=***"));
    }

    /// P2: OIDC discovery must respect
    /// `token_endpoint_auth_methods_supported`. When the IdP
    /// publishes `client_secret_post`, use Standard form bodies.
    /// When omitted (the spec default), use BasicAuth.
    #[test]
    fn oidc_discovery_picks_token_auth_method() {
        let json_post = r#"{
            "issuer": "https://acme.test/",
            "authorization_endpoint": "https://acme.test/auth",
            "token_endpoint": "https://acme.test/token",
            "token_endpoint_auth_methods_supported": ["client_secret_post"]
        }"#;
        let spec = provider::OidcDiscoveryDoc::parse(json_post)
            .unwrap()
            .into_spec();
        assert!(matches!(
            spec.token_exchange,
            provider::TokenExchangeShape::Standard
        ));

        // Default (omitted) → BasicAuth.
        let json_default = r#"{
            "issuer": "https://acme.test/",
            "authorization_endpoint": "https://acme.test/auth",
            "token_endpoint": "https://acme.test/token"
        }"#;
        let spec = provider::OidcDiscoveryDoc::parse(json_default)
            .unwrap()
            .into_spec();
        assert!(matches!(
            spec.token_exchange,
            provider::TokenExchangeShape::BasicAuth
        ));
    }

    /// P2: OIDC discovery missing required endpoints must fail loud,
    /// not silently produce empty URLs that would 404 every login.
    #[test]
    fn oidc_discovery_rejects_incomplete_doc() {
        // Missing token_endpoint.
        let json = r#"{
            "issuer": "https://acme.test/",
            "authorization_endpoint": "https://acme.test/auth"
        }"#;
        let err = provider::OidcDiscoveryDoc::parse(json).unwrap_err();
        assert!(err.contains("token_endpoint"), "got: {err}");
    }

    /// `OAuthRegistry::from_env` must auto-discover every provider
    /// whose env vars are set — not just google/github. Smoke-test
    /// with Discord since it covers the simple-builtin path.
    #[test]
    fn from_env_picks_up_discord() {
        // Use a unique prefix so this doesn't collide with a real
        // dev environment variable. Set+restore in scope.
        let key_id = "PYLON_OAUTH_DISCORD_CLIENT_ID";
        let key_secret = "PYLON_OAUTH_DISCORD_CLIENT_SECRET";
        // SAFETY: tests run single-threaded for env mutation isn't
        // strictly true, but this provider is unique enough that
        // contention is unlikely. Cleanup happens at end.
        std::env::set_var(key_id, "discord-test-id");
        std::env::set_var(key_secret, "discord-test-secret");

        let reg = OAuthRegistry::from_env();
        let discord = reg.get("discord").expect("discord registered");
        assert_eq!(discord.client_id, "discord-test-id");
        assert!(discord.auth_url().contains("discord.com"));

        std::env::remove_var(key_id);
        std::env::remove_var(key_secret);
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
            ..Default::default()
        };
        assert_eq!(google.token_url(), "https://oauth2.googleapis.com/token");
        let github = OAuthConfig {
            provider: "github".into(),
            client_id: "x".into(),
            client_secret: "x".into(),
            redirect_uri: "x".into(),
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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

    // -----------------------------------------------------------------
    // Wave-8: OAuth refresh-token helpers
    // -----------------------------------------------------------------

    fn fresh_account_with_expiry(expires_in_secs: i64) -> Account {
        let now = now_secs();
        let expires_at = if expires_in_secs >= 0 {
            Some(now.saturating_add(expires_in_secs as u64))
        } else {
            Some(now.saturating_sub((-expires_in_secs) as u64))
        };
        Account {
            id: "acc-1".into(),
            user_id: "user-1".into(),
            provider_id: "google".into(),
            account_id: "google-sub-123".into(),
            access_token: Some("at_old".into()),
            refresh_token: Some("rt_old".into()),
            id_token: None,
            access_token_expires_at: expires_at,
            refresh_token_expires_at: None,
            scope: Some("openid email".into()),
            password: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn needs_refresh_true_when_within_buffer() {
        let acc = fresh_account_with_expiry(30); // expires in 30s
        assert!(acc.needs_refresh(60));
    }

    #[test]
    fn needs_refresh_false_when_well_outside_buffer() {
        let acc = fresh_account_with_expiry(3600); // 1h to live
        assert!(!acc.needs_refresh(60));
    }

    #[test]
    fn needs_refresh_true_when_already_expired() {
        let acc = fresh_account_with_expiry(-30);
        assert!(acc.needs_refresh(60));
    }

    #[test]
    fn needs_refresh_false_when_no_expiry_set() {
        // GitHub-Classic-style non-expiring tokens. Refresh-on-401 is
        // the right pattern there; we shouldn't proactively refresh.
        let mut acc = fresh_account_with_expiry(0);
        acc.access_token_expires_at = None;
        assert!(!acc.needs_refresh(60));
    }

    #[test]
    fn ensure_fresh_returns_existing_when_not_due() {
        let store = AccountStore::new();
        let acc = fresh_account_with_expiry(3600);
        store.upsert(&acc);
        let got = store
            .ensure_fresh_access_token(&acc.provider_id, &acc.account_id, 60)
            .expect("non-expired account should not trigger refresh");
        // Same access token — no provider call was made.
        assert_eq!(got.access_token.as_deref(), Some("at_old"));
        assert_eq!(got.updated_at, acc.updated_at);
    }

    #[test]
    fn ensure_fresh_errors_when_no_account() {
        let store = AccountStore::new();
        let err = store
            .ensure_fresh_access_token("google", "ghost", 60)
            .unwrap_err();
        assert_eq!(err.code(), "ACCOUNT_NOT_FOUND");
    }

    #[test]
    fn ensure_fresh_errors_when_no_refresh_token_stored() {
        let store = AccountStore::new();
        let mut acc = fresh_account_with_expiry(10);
        acc.refresh_token = None;
        store.upsert(&acc);
        let err = store
            .ensure_fresh_access_token(&acc.provider_id, &acc.account_id, 60)
            .unwrap_err();
        assert_eq!(err.code(), "NO_REFRESH_TOKEN");
    }

    #[test]
    fn refresh_error_codes_map_to_documented_strings() {
        assert_eq!(RefreshError::AccountNotFound.code(), "ACCOUNT_NOT_FOUND");
        assert_eq!(RefreshError::NoRefreshToken.code(), "NO_REFRESH_TOKEN");
        assert_eq!(
            RefreshError::ProviderNotConfigured.code(),
            "PROVIDER_NOT_CONFIGURED"
        );
        assert_eq!(
            RefreshError::RefreshFailed("boom".into()).code(),
            "REFRESH_FAILED"
        );
    }
}
