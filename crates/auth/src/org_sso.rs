//! Per-organization Single Sign-On (OIDC).
//!
//! An org admin configures their identity provider (Okta, Auth0, Azure
//! AD, Google Workspace, Keycloak — anything OIDC-compliant). Members
//! sign in by going to `/api/auth/orgs/<slug>/sso/start`, which redirects
//! to the org's IdP. On callback, the user is auto-joined to the org
//! with the configured default role.
//!
//! Trust model: org admins explicitly choose their IdP. The framework
//! doesn't validate the IdP itself — admins are responsible for
//! configuring an IdP they trust to vouch for their members. The
//! framework DOES validate the discovered endpoints have HTTPS schemes
//! (per OIDC spec) so a misconfiguration can't accidentally route auth
//! through plaintext HTTP.
//!
//! This module owns the persistence + discovery only — the HTTP route
//! handlers live in `pylon-router`.

use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;

/// One org's SSO configuration. The client_secret is stored encrypted
/// at rest using the same envelope the rest of the auth crate uses for
/// secrets at rest (sealed with PYLON_SSO_ENCRYPTION_KEY when set; in
/// dev mode, plain).
///
/// Discovery endpoints (`authorization_endpoint`, `token_endpoint`,
/// `userinfo_endpoint`, `jwks_uri`) are populated from the IdP's
/// `/.well-known/openid-configuration` document at config-write time
/// and cached. They're refreshed if the operator PUTs the config again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgSsoConfig {
    pub org_id: String,
    /// Base URL of the IdP — e.g. `https://acme.okta.com`. Used to
    /// resolve `<issuer_url>/.well-known/openid-configuration`.
    pub issuer_url: String,
    pub client_id: String,
    /// Sealed (PYLON_SSO_ENCRYPTION_KEY) when set; raw otherwise. The
    /// public-facing config endpoint NEVER returns this field.
    pub client_secret_sealed: String,
    /// Org role granted to a freshly-auto-joined user. Defaults to
    /// "Member"; admins can promote later. Should NOT be "Owner" — the
    /// IdP can't be trusted to decide ownership.
    pub default_role: String,
    /// Optional list of email domains routed to this IdP for the
    /// "domain detection" sign-in path (typed user@acme.com → Acme's
    /// IdP without typing the org slug). Empty means explicit-URL only.
    /// Domains stored lowercase, no leading `@`.
    pub email_domains: Vec<String>,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub userinfo_endpoint: String,
    pub jwks_uri: String,
    pub created_at: u64,
    pub updated_at: u64,
}

impl OrgSsoConfig {
    /// Strip the secret before serializing for the public config
    /// endpoint. `client_secret_sealed` becomes the empty string so the
    /// shape stays the same for clients but the value is absent.
    pub fn redacted(&self) -> Self {
        let mut copy = self.clone();
        copy.client_secret_sealed = String::new();
        copy
    }
}

/// One pending SSO state token. Mirrors the global OAuth state but
/// scoped to the org so a state token issued for one org's flow can't
/// be replayed against another's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrgSsoStateRecord {
    pub state: String,
    pub org_id: String,
    pub pkce_verifier: String,
    /// OIDC `nonce` per Core §3.1.2.1 — bound into the AuthnRequest,
    /// must appear verbatim in the id_token's `nonce` claim. Defeats
    /// id_token replay across distinct sign-in attempts.
    pub nonce: String,
    pub callback_url: String,
    pub error_callback_url: String,
    pub created_at: u64,
}

/// Persistence trait. Implemented in-memory below; SQLite/PG impls live
/// in `pylon-runtime`.
pub trait OrgSsoStore: Send + Sync {
    /// Look up an org's config. Returns None when the org has no SSO
    /// configured (the default for every org).
    fn get(&self, org_id: &str) -> Option<OrgSsoConfig>;

    /// Insert or replace. Atomic w.r.t. domain claims — if any domain
    /// in `config.email_domains` is already claimed by a *different*
    /// org, the upsert MUST be rejected with `Err(_)` and the existing
    /// row left untouched (this is the cross-tenant takeover defense
    /// from review finding "first-come-first-served domain claim").
    fn upsert(&self, config: OrgSsoConfig) -> Result<(), DomainConflictError>;

    /// Remove. Returns true if a row existed.
    fn delete(&self, org_id: &str) -> bool;

    /// Map an email domain back to the org_id with that domain in its
    /// `email_domains` list. None when no org claims the domain.
    fn find_by_email_domain(&self, domain: &str) -> Option<String>;

    /// Persist a fresh SSO state record. Single-use; consumed via
    /// [`take_state`].
    fn save_state(&self, record: OrgSsoStateRecord);

    /// Single-use consumption. Returns the record if it exists AND the
    /// org_id matches the URL the callback hit. Removes the row from
    /// storage to defeat replay.
    fn take_state(&self, state: &str, expected_org_id: &str) -> Option<OrgSsoStateRecord>;
}

/// Returned by [`OrgSsoStore::upsert`] when one of the requested
/// `email_domains` is already claimed by another org. The conflicting
/// (domain, owning_org_id) pair is reported so the operator UI can
/// say "acme.com is already claimed by acme-corp" and not just "fail".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainConflictError {
    pub domain: String,
    pub claimed_by: String,
}

impl DomainConflictError {
    pub fn code(&self) -> &'static str {
        "DOMAIN_ALREADY_CLAIMED"
    }
    pub fn message(&self) -> String {
        format!(
            "domain `{}` is already claimed by org `{}`",
            self.domain, self.claimed_by
        )
    }
}

/// In-memory store (default).
pub struct InMemoryOrgSsoStore {
    configs: RwLock<HashMap<String, OrgSsoConfig>>,
    domains: RwLock<HashMap<String, String>>,
    states: RwLock<HashMap<String, OrgSsoStateRecord>>,
}

impl Default for InMemoryOrgSsoStore {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryOrgSsoStore {
    pub fn new() -> Self {
        Self {
            configs: RwLock::new(HashMap::new()),
            domains: RwLock::new(HashMap::new()),
            states: RwLock::new(HashMap::new()),
        }
    }
}

impl OrgSsoStore for InMemoryOrgSsoStore {
    fn get(&self, org_id: &str) -> Option<OrgSsoConfig> {
        self.configs.read().unwrap().get(org_id).cloned()
    }

    fn upsert(&self, config: OrgSsoConfig) -> Result<(), DomainConflictError> {
        let mut configs = self.configs.write().unwrap();
        let mut domains = self.domains.write().unwrap();
        // Conflict check FIRST — fail without mutating anything if a
        // requested domain is owned by a different org. The org may
        // re-upsert its own existing domains freely.
        for d in &config.email_domains {
            let lower = d.to_ascii_lowercase();
            if let Some(owner) = domains.get(&lower) {
                if owner != &config.org_id {
                    return Err(DomainConflictError {
                        domain: lower,
                        claimed_by: owner.clone(),
                    });
                }
            }
        }
        // Drop the previous row's domain claims so removed-from-list
        // domains don't keep routing here.
        if let Some(prev) = configs.get(&config.org_id) {
            for d in &prev.email_domains {
                if domains.get(d).map(|v| v == &config.org_id).unwrap_or(false) {
                    domains.remove(d);
                }
            }
        }
        for d in &config.email_domains {
            domains.insert(d.to_ascii_lowercase(), config.org_id.clone());
        }
        configs.insert(config.org_id.clone(), config);
        Ok(())
    }

    fn delete(&self, org_id: &str) -> bool {
        let removed = self.configs.write().unwrap().remove(org_id);
        if let Some(cfg) = &removed {
            let mut domains = self.domains.write().unwrap();
            for d in &cfg.email_domains {
                if domains.get(d).map(|v| v == &cfg.org_id).unwrap_or(false) {
                    domains.remove(d);
                }
            }
        }
        removed.is_some()
    }

    fn find_by_email_domain(&self, domain: &str) -> Option<String> {
        self.domains
            .read()
            .unwrap()
            .get(&domain.to_ascii_lowercase())
            .cloned()
    }

    fn save_state(&self, record: OrgSsoStateRecord) {
        // Lazy GC of old states — anything older than 10 minutes is dead.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut states = self.states.write().unwrap();
        states.retain(|_, v| now.saturating_sub(v.created_at) < STATE_TTL_SECS);
        states.insert(record.state.clone(), record);
    }

    fn take_state(&self, state: &str, expected_org_id: &str) -> Option<OrgSsoStateRecord> {
        let mut states = self.states.write().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let candidate = states.get(state)?.clone();
        if candidate.org_id != expected_org_id {
            // Cross-org replay attempt — leave the record in place so a
            // legitimate parallel flow for the right org still works.
            return None;
        }
        if now.saturating_sub(candidate.created_at) >= STATE_TTL_SECS {
            states.remove(state);
            return None;
        }
        states.remove(state);
        Some(candidate)
    }
}

/// State TTL (10 minutes). Long enough for slow auth flows (re-auth,
/// MFA prompt on the IdP side); short enough that a leaked state isn't
/// usable forever.
pub const STATE_TTL_SECS: u64 = 10 * 60;

/// Determine whether `domain` is allowed to be claimed in
/// `email_domains` by org SSO config. Two layers:
///
/// 1. Hardcoded blocklist of public free-mail providers — a malicious
///    org owner cannot claim `gmail.com` and intercept every Gmail
///    user's sign-in via domain detection. Multi-tenant attack model
///    from the post-Wave-8 review explicitly requires this.
/// 2. Operator allowlist via `PYLON_SSO_ALLOWED_DOMAINS` (comma-
///    separated). When set + non-empty, ONLY listed domains can be
///    claimed.
///
/// The verification layer needed to prove ownership (DNS TXT challenge)
/// is a follow-up; this function locks down the obvious abuse vector.
pub fn validate_claimable_domain(domain: &str) -> Result<(), DomainClaimError> {
    let lower = domain.to_ascii_lowercase();
    if BLOCKLIST_FREEMAIL_DOMAINS
        .iter()
        .any(|d| *d == lower.as_str())
    {
        return Err(DomainClaimError::Blocklisted(lower));
    }
    if let Ok(raw) = std::env::var("PYLON_SSO_ALLOWED_DOMAINS") {
        let allowed: Vec<String> = raw
            .split(',')
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        if !allowed.is_empty() && !allowed.iter().any(|a| a == &lower) {
            return Err(DomainClaimError::NotInAllowlist(lower));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainClaimError {
    Blocklisted(String),
    NotInAllowlist(String),
}

impl DomainClaimError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Blocklisted(_) => "DOMAIN_BLOCKLISTED",
            Self::NotInAllowlist(_) => "DOMAIN_NOT_ALLOWED",
        }
    }
    pub fn message(&self) -> String {
        match self {
            Self::Blocklisted(d) => {
                format!("domain `{d}` is a free-mail provider and cannot be claimed by an org")
            }
            Self::NotInAllowlist(d) => {
                format!("domain `{d}` is not in PYLON_SSO_ALLOWED_DOMAINS")
            }
        }
    }
}

const BLOCKLIST_FREEMAIL_DOMAINS: &[&str] = &[
    "gmail.com",
    "googlemail.com",
    "yahoo.com",
    "yahoo.co.uk",
    "yahoo.co.jp",
    "outlook.com",
    "hotmail.com",
    "live.com",
    "msn.com",
    "icloud.com",
    "me.com",
    "mac.com",
    "aol.com",
    "protonmail.com",
    "proton.me",
    "mail.com",
    "gmx.com",
    "gmx.net",
    "gmx.de",
    "yandex.com",
    "yandex.ru",
    "qq.com",
    "163.com",
    "126.com",
    "fastmail.com",
    "fastmail.fm",
    "zoho.com",
    "tutanota.com",
    "duck.com",
];

/// Discovery: fetch the IdP's `/.well-known/openid-configuration` JSON
/// and pull the four endpoints we need. Caller is responsible for
/// validating `issuer_url` itself (HTTPS scheme, no trailing slash
/// chaos).
///
/// Returns the four endpoint URLs in the order
/// `(authorization_endpoint, token_endpoint, userinfo_endpoint, jwks_uri)`.
/// The IdP's discovery doc is normative — no fallback synthesis from
/// the issuer URL.
pub fn discover_endpoints(issuer_url: &str) -> Result<DiscoveredEndpoints, String> {
    let trimmed = issuer_url.trim_end_matches('/');
    if !trimmed.starts_with("https://") {
        // OIDC Discovery 1.0 §4 mandates HTTPS. Plain-HTTP discovery
        // would let a network attacker swap the IdP's endpoints.
        return Err(format!("issuer URL must use https:// (got `{trimmed}`)"));
    }
    let url = format!("{trimmed}/.well-known/openid-configuration");
    // 5-second timeout — discovery is a blocking call on the config-
    // write path, and IdPs that take longer than that are misconfigured.
    let body = ureq::get(&url)
        .timeout(std::time::Duration::from_secs(5))
        .call()
        .map_err(|e| format!("discovery fetch failed: {e}"))?
        .into_string()
        .map_err(|e| format!("discovery body read failed: {e}"))?;
    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("discovery body is not JSON: {e}"))?;
    let pull = |k: &str| -> Result<String, String> {
        json.get(k)
            .and_then(|v| v.as_str())
            .filter(|s| s.starts_with("https://"))
            .map(String::from)
            .ok_or_else(|| format!("discovery doc missing or non-HTTPS `{k}`"))
    };
    Ok(DiscoveredEndpoints {
        authorization_endpoint: pull("authorization_endpoint")?,
        token_endpoint: pull("token_endpoint")?,
        userinfo_endpoint: pull("userinfo_endpoint")?,
        jwks_uri: pull("jwks_uri")?,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredEndpoints {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub userinfo_endpoint: String,
    pub jwks_uri: String,
}

/// Complete an OIDC code exchange against a configured IdP. Wave 8 +
/// post-review hardening — runs the full OIDC validation chain:
///
/// - Token endpoint exchange (PKCE).
/// - id_token signature verification against the IdP's `jwks_uri`
///   (RS256 / ES256). Without this, a hijacked userinfo TLS cert OR a
///   compromised access_token is enough to spoof identity. The
///   id_token is the only signed identity carrier OIDC offers.
/// - id_token claims validation: `iss == config.issuer_url`,
///   `aud == config.client_id`, `nonce == expected_nonce` (anti-replay
///   binding to the AuthnRequest), `exp` in the future.
/// - `email_verified == true` requirement before treating the email
///   as authoritative for User-row lookup-or-create.
/// - Optional userinfo fetch as a defense-in-depth name source.
///
/// Returns `(email, display_name)` on success.
pub fn complete_oidc_login(
    config: &OrgSsoConfig,
    code: &str,
    pkce_verifier: &str,
    redirect_uri: &str,
    expected_nonce: &str,
) -> Result<(String, Option<String>), OrgSsoLoginError> {
    let secret =
        unseal_secret(&config.client_secret_sealed).map_err(OrgSsoLoginError::SecretUnreadable)?;
    let body = format!(
        "grant_type=authorization_code&code={code}&redirect_uri={ruri}&client_id={cid}&client_secret={secret}&code_verifier={pkce}",
        code = url_form(code),
        ruri = url_form(redirect_uri),
        cid = url_form(&config.client_id),
        secret = url_form(&secret),
        pkce = url_form(pkce_verifier),
    );
    let token_body = ureq::post(&config.token_endpoint)
        .set("Accept", "application/json")
        .set("Content-Type", "application/x-www-form-urlencoded")
        .timeout(std::time::Duration::from_secs(10))
        .send_string(&body)
        .map_err(|e| OrgSsoLoginError::TokenExchangeFailed(format!("{e}")))?
        .into_string()
        .map_err(|e| OrgSsoLoginError::TokenBodyReadFailed(format!("{e}")))?;
    let token_json: serde_json::Value = serde_json::from_str(&token_body).map_err(|_| {
        OrgSsoLoginError::TokenResponseNotJson(token_body[..token_body.len().min(200)].into())
    })?;
    let access_token = token_json
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or(OrgSsoLoginError::NoAccessToken)?
        .to_string();
    let id_token_jwt = token_json
        .get("id_token")
        .and_then(|v| v.as_str())
        .ok_or(OrgSsoLoginError::NoIdToken)?
        .to_string();

    // Verify id_token signature against the IdP's JWKS, then enforce
    // iss / aud / nonce / exp. Failure here blocks the auth flow.
    let claims = verify_id_token(&id_token_jwt, config, expected_nonce)?;

    let email = claims
        .get("email")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .ok_or(OrgSsoLoginError::NoEmailClaim)?;

    // Reject unverified emails — see review finding P1
    // "complete_oidc_login trusts unsigned userinfo.email". Without
    // this an attacker who registers `evil@victim-corp.com` at the
    // IdP without verifying it gets auto-merged into Pylon's account
    // for victim-corp.com. Most IdPs (Google, Okta, Auth0, Azure AD)
    // include `email_verified` in id_token; refuse when absent or
    // when the value is not exactly `true`.
    let email_verified = claims
        .get("email_verified")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !email_verified {
        return Err(OrgSsoLoginError::EmailNotVerified(email));
    }

    // Display name: try id_token first (fewer round-trips), fall back
    // to userinfo if the id_token didn't carry it (some IdPs don't
    // include `name` in the id_token by default).
    let name_from_id_token = claims
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);
    let name = name_from_id_token.or_else(|| {
        ureq::get(&config.userinfo_endpoint)
            .set("Authorization", &format!("Bearer {access_token}"))
            .timeout(std::time::Duration::from_secs(10))
            .call()
            .ok()
            .and_then(|r| r.into_string().ok())
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(String::from))
            .filter(|s| !s.is_empty())
    });

    Ok((email, name))
}

/// Verify an OIDC id_token against the IdP's JWKS + enforce the
/// standard claims. RS256 + ES256 + HS256 covered (HS256 only valid
/// against `client_secret`, which the operator already shared with
/// the IdP). EdDSA + PS256 are common at modern IdPs but require
/// dependency expansion; rejected with a clear error so operators
/// know to either update or open an issue.
fn verify_id_token(
    jwt: &str,
    config: &OrgSsoConfig,
    expected_nonce: &str,
) -> Result<serde_json::Value, OrgSsoLoginError> {
    use base64::Engine;
    let parts: Vec<&str> = jwt.split('.').collect();
    if parts.len() != 3 {
        return Err(OrgSsoLoginError::IdTokenInvalid("not a 3-part JWT".into()));
    }
    let header_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[0])
        .map_err(|e| OrgSsoLoginError::IdTokenInvalid(format!("header b64: {e}")))?;
    let header: serde_json::Value = serde_json::from_slice(&header_bytes)
        .map_err(|e| OrgSsoLoginError::IdTokenInvalid(format!("header json: {e}")))?;
    let alg = header
        .get("alg")
        .and_then(|v| v.as_str())
        .ok_or_else(|| OrgSsoLoginError::IdTokenInvalid("missing alg".into()))?;
    let kid = header.get("kid").and_then(|v| v.as_str());

    let signing_input = format!("{}.{}", parts[0], parts[1]);
    let signature = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[2])
        .map_err(|e| OrgSsoLoginError::IdTokenInvalid(format!("sig b64: {e}")))?;

    match alg {
        "HS256" => {
            // Symmetric — verify with client_secret. Per OIDC §10.1,
            // symmetric algs are valid only when client_secret is the
            // shared key.
            let secret = unseal_secret(&config.client_secret_sealed)
                .map_err(OrgSsoLoginError::SecretUnreadable)?;
            use hmac::{Hmac, Mac};
            use sha2::Sha256;
            let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
                .map_err(|e| OrgSsoLoginError::IdTokenInvalid(format!("hmac init: {e}")))?;
            mac.update(signing_input.as_bytes());
            mac.verify_slice(&signature)
                .map_err(|_| OrgSsoLoginError::IdTokenSignatureInvalid)?;
        }
        "RS256" | "ES256" => {
            verify_with_jwks(
                &config.jwks_uri,
                alg,
                kid,
                signing_input.as_bytes(),
                &signature,
            )?;
        }
        other => {
            return Err(OrgSsoLoginError::IdTokenAlgorithmUnsupported(other.into()));
        }
    }

    let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .map_err(|e| OrgSsoLoginError::IdTokenInvalid(format!("payload b64: {e}")))?;
    let claims: serde_json::Value = serde_json::from_slice(&payload_bytes)
        .map_err(|e| OrgSsoLoginError::IdTokenInvalid(format!("payload json: {e}")))?;

    // iss check — anchored to the configured issuer URL. Trim trailing
    // slash on both sides since OIDC discovery and id_tokens disagree.
    let iss = claims
        .get("iss")
        .and_then(|v| v.as_str())
        .ok_or_else(|| OrgSsoLoginError::IdTokenInvalid("missing iss".into()))?;
    if iss.trim_end_matches('/') != config.issuer_url.trim_end_matches('/') {
        return Err(OrgSsoLoginError::IdTokenIssuerMismatch {
            got: iss.into(),
            expected: config.issuer_url.clone(),
        });
    }

    // aud check — must include our client_id. `aud` may be a string
    // OR an array of strings per RFC 7519 §4.1.3.
    let aud = claims
        .get("aud")
        .ok_or_else(|| OrgSsoLoginError::IdTokenInvalid("missing aud".into()))?;
    let aud_match = match aud {
        serde_json::Value::String(s) => s == &config.client_id,
        serde_json::Value::Array(arr) => arr
            .iter()
            .any(|v| v.as_str() == Some(config.client_id.as_str())),
        _ => false,
    };
    if !aud_match {
        return Err(OrgSsoLoginError::IdTokenAudienceMismatch);
    }

    // exp check — claims expiry in seconds since epoch. Accept up to
    // 60s of clock skew.
    let exp = claims
        .get("exp")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| OrgSsoLoginError::IdTokenInvalid("missing exp".into()))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if exp.saturating_add(60) < now {
        return Err(OrgSsoLoginError::IdTokenExpired);
    }

    // nonce check — anti-replay binding to the AuthnRequest we minted.
    // Per OIDC Core §3.1.2.1, when nonce was sent in the AuthnRequest
    // (we always do), the id_token MUST carry the same value.
    let nonce = claims.get("nonce").and_then(|v| v.as_str()).unwrap_or("");
    if nonce != expected_nonce {
        return Err(OrgSsoLoginError::IdTokenNonceMismatch);
    }

    Ok(claims)
}

/// Fetch the IdP's JWKS, find the key matching `kid` (or fall back to
/// any key when `kid` is absent), and verify the signature.
fn verify_with_jwks(
    jwks_uri: &str,
    alg: &str,
    kid: Option<&str>,
    signing_input: &[u8],
    signature: &[u8],
) -> Result<(), OrgSsoLoginError> {
    let body = ureq::get(jwks_uri)
        .timeout(std::time::Duration::from_secs(5))
        .call()
        .map_err(|e| OrgSsoLoginError::JwksFetchFailed(format!("{e}")))?
        .into_string()
        .map_err(|e| OrgSsoLoginError::JwksFetchFailed(format!("body: {e}")))?;
    let jwks: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| OrgSsoLoginError::JwksFetchFailed(format!("json: {e}")))?;
    let keys = jwks
        .get("keys")
        .and_then(|v| v.as_array())
        .ok_or_else(|| OrgSsoLoginError::JwksFetchFailed("no keys array".into()))?;

    // Match by kid when the JWT has one; otherwise try every key
    // matching the alg's family. Most IdPs publish 1-2 keys at a time.
    let candidates: Vec<&serde_json::Value> = keys
        .iter()
        .filter(|k| match kid {
            Some(want) => k.get("kid").and_then(|v| v.as_str()) == Some(want),
            None => true,
        })
        .collect();
    if candidates.is_empty() {
        return Err(OrgSsoLoginError::IdTokenSignatureInvalid);
    }

    for key in candidates {
        let kty = key.get("kty").and_then(|v| v.as_str()).unwrap_or("");
        let result = match (alg, kty) {
            ("RS256", "RSA") => verify_rs256(key, signing_input, signature),
            ("ES256", "EC") => verify_es256(key, signing_input, signature),
            _ => continue,
        };
        if result.is_ok() {
            return Ok(());
        }
    }
    Err(OrgSsoLoginError::IdTokenSignatureInvalid)
}

fn verify_rs256(key: &serde_json::Value, signing_input: &[u8], signature: &[u8]) -> Result<(), ()> {
    use base64::Engine;
    let n_b64 = key.get("n").and_then(|v| v.as_str()).ok_or(())?;
    let e_b64 = key.get("e").and_then(|v| v.as_str()).ok_or(())?;
    let n_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(n_b64)
        .map_err(|_| ())?;
    let e_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(e_b64)
        .map_err(|_| ())?;
    use ring::signature;
    let public_key = signature::RsaPublicKeyComponents {
        n: &n_bytes,
        e: &e_bytes,
    };
    public_key
        .verify(
            &signature::RSA_PKCS1_2048_8192_SHA256,
            signing_input,
            signature,
        )
        .map_err(|_| ())
}

fn verify_es256(key: &serde_json::Value, signing_input: &[u8], signature: &[u8]) -> Result<(), ()> {
    use base64::Engine;
    let x_b64 = key.get("x").and_then(|v| v.as_str()).ok_or(())?;
    let y_b64 = key.get("y").and_then(|v| v.as_str()).ok_or(())?;
    let x = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(x_b64)
        .map_err(|_| ())?;
    let y = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(y_b64)
        .map_err(|_| ())?;
    if x.len() != 32 || y.len() != 32 {
        return Err(());
    }
    // Uncompressed SEC1 point: 0x04 || X || Y.
    let mut sec1 = Vec::with_capacity(65);
    sec1.push(0x04);
    sec1.extend_from_slice(&x);
    sec1.extend_from_slice(&y);
    use ring::signature;
    let key = signature::UnparsedPublicKey::new(&signature::ECDSA_P256_SHA256_FIXED, &sec1);
    key.verify(signing_input, signature).map_err(|_| ())
}

/// Stable codes for [`complete_oidc_login`] failures. Surfaced via the
/// SSO callback's error-redirect URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrgSsoLoginError {
    SecretUnreadable(String),
    TokenExchangeFailed(String),
    TokenBodyReadFailed(String),
    TokenResponseNotJson(String),
    NoAccessToken,
    NoIdToken,
    IdTokenInvalid(String),
    IdTokenSignatureInvalid,
    IdTokenAlgorithmUnsupported(String),
    IdTokenIssuerMismatch { got: String, expected: String },
    IdTokenAudienceMismatch,
    IdTokenExpired,
    IdTokenNonceMismatch,
    JwksFetchFailed(String),
    EmailNotVerified(String),
    NoEmailClaim,
}

impl OrgSsoLoginError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::SecretUnreadable(_) => "SSO_SECRET_UNREADABLE",
            Self::TokenExchangeFailed(_) => "TOKEN_EXCHANGE_FAILED",
            Self::TokenBodyReadFailed(_) => "TOKEN_BODY_READ_FAILED",
            Self::TokenResponseNotJson(_) => "TOKEN_RESPONSE_NOT_JSON",
            Self::NoAccessToken => "NO_ACCESS_TOKEN",
            Self::NoIdToken => "NO_ID_TOKEN",
            Self::IdTokenInvalid(_) => "ID_TOKEN_INVALID",
            Self::IdTokenSignatureInvalid => "ID_TOKEN_SIGNATURE_INVALID",
            Self::IdTokenAlgorithmUnsupported(_) => "ID_TOKEN_ALG_UNSUPPORTED",
            Self::IdTokenIssuerMismatch { .. } => "ID_TOKEN_ISS_MISMATCH",
            Self::IdTokenAudienceMismatch => "ID_TOKEN_AUD_MISMATCH",
            Self::IdTokenExpired => "ID_TOKEN_EXPIRED",
            Self::IdTokenNonceMismatch => "ID_TOKEN_NONCE_MISMATCH",
            Self::JwksFetchFailed(_) => "JWKS_FETCH_FAILED",
            Self::EmailNotVerified(_) => "EMAIL_NOT_VERIFIED",
            Self::NoEmailClaim => "NO_EMAIL_CLAIM",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::SecretUnreadable(e) => format!("could not read SSO secret: {e}"),
            // Wave-8 P2 fix: provider-side error responses sometimes
            // echo the request back including the auth `code` /
            // `client_secret` / `refresh_token`. Run through the
            // central sanitize_token_error redactor before surfacing
            // — without this the SSO error redirect can leak those
            // values into the caller's error page URL bar and referrer
            // logs.
            Self::TokenExchangeFailed(e) => format!(
                "token exchange failed: {}",
                crate::sanitize_token_error(e.clone())
            ),
            Self::TokenBodyReadFailed(e) => format!(
                "could not read token response: {}",
                crate::sanitize_token_error(e.clone())
            ),
            Self::TokenResponseNotJson(s) => format!(
                "token response was not JSON: {}",
                crate::sanitize_token_error(s.clone())
            ),
            Self::NoAccessToken => "IdP did not return an access_token".into(),
            Self::NoIdToken => "IdP did not return an id_token (request `openid` scope)".into(),
            Self::IdTokenInvalid(e) => format!("id_token malformed: {e}"),
            Self::IdTokenSignatureInvalid => {
                "id_token signature did not verify against the IdP's JWKS".into()
            }
            Self::IdTokenAlgorithmUnsupported(alg) => {
                format!("id_token signing alg `{alg}` is not supported (RS256, ES256, HS256 only)")
            }
            Self::IdTokenIssuerMismatch { got, expected } => {
                format!("id_token iss `{got}` does not match configured issuer `{expected}`")
            }
            Self::IdTokenAudienceMismatch => "id_token aud does not include our client_id".into(),
            Self::IdTokenExpired => "id_token has expired".into(),
            Self::IdTokenNonceMismatch => "id_token nonce does not match the AuthnRequest".into(),
            Self::JwksFetchFailed(e) => format!("could not load IdP JWKS: {e}"),
            Self::EmailNotVerified(email) => {
                format!("IdP reports `email_verified=false` for `{email}`")
            }
            Self::NoEmailClaim => "id_token missing `email` claim".into(),
        }
    }
}

/// Minimal application/x-www-form-urlencoded encoder for the token-
/// exchange body. The auth crate has its own `url_encode` that's
/// slightly more permissive (URI query syntax); for form bodies we
/// want the strict-form encoding (space → `+`, etc.). Tiny inline
/// helper to avoid exposing a sibling-namespaced encoder from the
/// general OAuth path.
fn url_form(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Mint a fresh SSO state token. 32 bytes of CSPRNG, base64url
/// unpadded. Same strength as the global OAuth state.
pub fn random_state() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Encrypt-at-rest envelope for client secrets. Uses
/// PYLON_SSO_ENCRYPTION_KEY (32-byte hex/base64). When unset, returns
/// the secret verbatim with a `plain:` prefix so the round-trip works
/// in dev — operators are warned at server-boot time when a secret
/// would be persisted plain.
///
/// Symmetric placeholder: production SHOULD set the key. The format is
/// `enc:<nonce_hex>:<ciphertext_b64>` so a future migration can detect
/// + upgrade old `plain:` rows.
pub fn seal_secret(secret: &str) -> String {
    if let Some(key) = sso_encryption_key() {
        // ChaCha20-Poly1305 sealed envelope. Re-uses the totp module's
        // approach to keep one crypto primitive in the auth crate.
        match encrypt_chacha(secret.as_bytes(), &key) {
            Ok(b) => format!("enc:{b}"),
            Err(_) => format!("plain:{secret}"),
        }
    } else {
        format!("plain:{secret}")
    }
}

/// Inverse of [`seal_secret`].
pub fn unseal_secret(blob: &str) -> Result<String, String> {
    if let Some(rest) = blob.strip_prefix("plain:") {
        return Ok(rest.to_string());
    }
    if let Some(rest) = blob.strip_prefix("enc:") {
        let key = sso_encryption_key().ok_or_else(|| {
            "PYLON_SSO_ENCRYPTION_KEY required to read sealed SSO secret".to_string()
        })?;
        return decrypt_chacha(rest, &key);
    }
    // Legacy unprefixed value: assume plain. Operators won't see this
    // unless they hand-edited rows.
    Ok(blob.to_string())
}

fn sso_encryption_key() -> Option<[u8; 32]> {
    let raw = std::env::var("PYLON_SSO_ENCRYPTION_KEY").ok()?;
    parse_key32(&raw)
}

fn parse_key32(raw: &str) -> Option<[u8; 32]> {
    use base64::Engine;
    // hex first
    if raw.len() == 64 && raw.chars().all(|c| c.is_ascii_hexdigit()) {
        let mut out = [0u8; 32];
        for i in 0..32 {
            out[i] = u8::from_str_radix(&raw[i * 2..i * 2 + 2], 16).ok()?;
        }
        return Some(out);
    }
    // base64
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(raw)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(raw))
        .ok()?;
    if bytes.len() == 32 {
        let mut out = [0u8; 32];
        out.copy_from_slice(&bytes);
        Some(out)
    } else {
        None
    }
}

fn encrypt_chacha(plaintext: &[u8], key: &[u8; 32]) -> Result<String, String> {
    use ring::aead;
    use ring::rand::{SecureRandom, SystemRandom};
    let unbound =
        aead::UnboundKey::new(&aead::CHACHA20_POLY1305, key).map_err(|_| "key init".to_string())?;
    let sealing = aead::LessSafeKey::new(unbound);
    let mut nonce_bytes = [0u8; 12];
    SystemRandom::new()
        .fill(&mut nonce_bytes)
        .map_err(|_| "nonce gen".to_string())?;
    let nonce = aead::Nonce::assume_unique_for_key(nonce_bytes);
    let mut buf = plaintext.to_vec();
    sealing
        .seal_in_place_append_tag(nonce, aead::Aad::empty(), &mut buf)
        .map_err(|_| "seal".to_string())?;
    use base64::Engine;
    let nonce_hex: String = nonce_bytes.iter().map(|b| format!("{b:02x}")).collect();
    let ct = base64::engine::general_purpose::STANDARD.encode(&buf);
    Ok(format!("{nonce_hex}:{ct}"))
}

fn decrypt_chacha(envelope: &str, key: &[u8; 32]) -> Result<String, String> {
    use ring::aead;
    let (nonce_hex, ct_b64) = envelope
        .split_once(':')
        .ok_or_else(|| "bad envelope".to_string())?;
    if nonce_hex.len() != 24 {
        return Err("bad nonce length".to_string());
    }
    let mut nonce_bytes = [0u8; 12];
    for i in 0..12 {
        nonce_bytes[i] =
            u8::from_str_radix(&nonce_hex[i * 2..i * 2 + 2], 16).map_err(|_| "bad nonce hex")?;
    }
    use base64::Engine;
    let mut ct = base64::engine::general_purpose::STANDARD
        .decode(ct_b64)
        .map_err(|_| "bad ciphertext base64")?;
    let unbound = aead::UnboundKey::new(&aead::CHACHA20_POLY1305, key).map_err(|_| "key init")?;
    let opening = aead::LessSafeKey::new(unbound);
    let nonce = aead::Nonce::assume_unique_for_key(nonce_bytes);
    let pt = opening
        .open_in_place(nonce, aead::Aad::empty(), &mut ct)
        .map_err(|_| "decrypt failed")?;
    String::from_utf8(pt.to_vec()).map_err(|_| "decrypted bytes are not utf-8".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(org: &str, domains: Vec<&str>) -> OrgSsoConfig {
        OrgSsoConfig {
            org_id: org.into(),
            issuer_url: "https://acme.okta.com".into(),
            client_id: "client_abc".into(),
            client_secret_sealed: "plain:shh".into(),
            default_role: "Member".into(),
            email_domains: domains.into_iter().map(String::from).collect(),
            authorization_endpoint: "https://acme.okta.com/oauth2/v1/authorize".into(),
            token_endpoint: "https://acme.okta.com/oauth2/v1/token".into(),
            userinfo_endpoint: "https://acme.okta.com/oauth2/v1/userinfo".into(),
            jwks_uri: "https://acme.okta.com/oauth2/v1/keys".into(),
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn upsert_and_get_round_trip() {
        let store = InMemoryOrgSsoStore::new();
        store.upsert(config("acme", vec!["acme.com"])).unwrap();
        let got = store.get("acme").unwrap();
        assert_eq!(got.client_id, "client_abc");
        assert_eq!(got.email_domains, vec!["acme.com".to_string()]);
    }

    #[test]
    fn validate_claimable_domain_blocks_freemail() {
        for d in &["gmail.com", "GMAIL.COM", "outlook.com", "icloud.com"] {
            let err = validate_claimable_domain(d).unwrap_err();
            assert_eq!(err.code(), "DOMAIN_BLOCKLISTED", "{d} should be blocked");
        }
    }

    #[test]
    fn validate_claimable_domain_allows_corporate_domains() {
        assert!(validate_claimable_domain("acme-corp.com").is_ok());
        assert!(validate_claimable_domain("internal.eng.example.com").is_ok());
    }

    #[test]
    fn upsert_rejects_domain_already_claimed_by_another_org() {
        let store = InMemoryOrgSsoStore::new();
        store.upsert(config("acme", vec!["shared.com"])).unwrap();
        let err = store
            .upsert(config("globex", vec!["shared.com"]))
            .unwrap_err();
        assert_eq!(err.code(), "DOMAIN_ALREADY_CLAIMED");
        assert_eq!(err.domain, "shared.com");
        assert_eq!(err.claimed_by, "acme");
        // The conflicting upsert must NOT have mutated state.
        assert!(store.get("globex").is_none());
        assert_eq!(
            store.find_by_email_domain("shared.com").as_deref(),
            Some("acme"),
        );
    }

    #[test]
    fn upsert_allows_same_org_to_re_upsert_its_own_domains() {
        let store = InMemoryOrgSsoStore::new();
        store.upsert(config("acme", vec!["acme.com"])).unwrap();
        let mut updated = config("acme", vec!["acme.com", "acme.io"]);
        updated.client_id = "client_rotated".into();
        store.upsert(updated).unwrap();
        assert_eq!(store.get("acme").unwrap().client_id, "client_rotated");
        assert_eq!(
            store.find_by_email_domain("acme.io").as_deref(),
            Some("acme"),
        );
    }

    #[test]
    fn redacted_strips_secret() {
        let cfg = config("acme", vec![]);
        assert!(!cfg.client_secret_sealed.is_empty());
        assert!(cfg.redacted().client_secret_sealed.is_empty());
        assert_eq!(cfg.redacted().client_id, "client_abc");
    }

    #[test]
    fn find_by_email_domain_is_case_insensitive() {
        let store = InMemoryOrgSsoStore::new();
        store.upsert(config("acme", vec!["acme.com"])).unwrap();
        assert_eq!(
            store.find_by_email_domain("ACME.COM").as_deref(),
            Some("acme")
        );
        assert_eq!(
            store.find_by_email_domain("acme.com").as_deref(),
            Some("acme")
        );
        assert_eq!(store.find_by_email_domain("other.com"), None);
    }

    #[test]
    fn upsert_replaces_domain_index_on_change() {
        let store = InMemoryOrgSsoStore::new();
        store.upsert(config("acme", vec!["old.com"])).unwrap();
        store.upsert(config("acme", vec!["new.com"])).unwrap();
        assert_eq!(store.find_by_email_domain("old.com"), None);
        assert_eq!(
            store.find_by_email_domain("new.com").as_deref(),
            Some("acme")
        );
    }

    #[test]
    fn delete_clears_domain_index_too() {
        let store = InMemoryOrgSsoStore::new();
        store.upsert(config("acme", vec!["acme.com"])).unwrap();
        assert!(store.delete("acme"));
        assert!(store.get("acme").is_none());
        assert_eq!(store.find_by_email_domain("acme.com"), None);
    }

    #[test]
    fn delete_returns_false_for_unknown() {
        let store = InMemoryOrgSsoStore::new();
        assert!(!store.delete("ghost"));
    }

    #[test]
    fn save_then_take_state_returns_record_and_consumes_it() {
        let store = InMemoryOrgSsoStore::new();
        let record = OrgSsoStateRecord {
            state: random_state(),
            org_id: "acme".into(),
            pkce_verifier: "v_xyz".into(),
            nonce: "n_test".into(),
            callback_url: "https://app/callback".into(),
            error_callback_url: "https://app/err".into(),
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        };
        let token = record.state.clone();
        store.save_state(record);
        let got = store.take_state(&token, "acme").expect("must resolve");
        assert_eq!(got.pkce_verifier, "v_xyz");
        // Replay: state was consumed.
        assert!(store.take_state(&token, "acme").is_none());
    }

    #[test]
    fn take_state_rejects_wrong_org() {
        let store = InMemoryOrgSsoStore::new();
        let record = OrgSsoStateRecord {
            state: "s_1".into(),
            org_id: "acme".into(),
            pkce_verifier: "v".into(),
            nonce: "n_test".into(),
            callback_url: "u".into(),
            error_callback_url: "u".into(),
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        };
        store.save_state(record);
        // Cross-org replay attempt — record stays in place.
        assert!(store.take_state("s_1", "evil-corp").is_none());
        // Legitimate parallel flow for the right org still works.
        assert!(store.take_state("s_1", "acme").is_some());
    }

    #[test]
    fn discover_rejects_plaintext_http() {
        let err = discover_endpoints("http://insecure.example").unwrap_err();
        assert!(err.contains("https"));
    }

    #[test]
    fn random_state_is_unique() {
        let a = random_state();
        let b = random_state();
        assert_ne!(a, b);
        assert_eq!(a.len(), 43, "32 bytes base64url unpadded = 43 chars");
    }

    #[test]
    fn seal_unseal_round_trip_plain_mode() {
        // No PYLON_SSO_ENCRYPTION_KEY set in tests by default.
        let sealed = seal_secret("topsecret");
        assert!(sealed.starts_with("plain:") || sealed.starts_with("enc:"));
        let unsealed = unseal_secret(&sealed).unwrap();
        assert_eq!(unsealed, "topsecret");
    }

    #[test]
    fn unseal_handles_legacy_unprefixed() {
        // Rows written before the prefix scheme existed.
        assert_eq!(unseal_secret("legacy_value").unwrap(), "legacy_value");
    }
}
