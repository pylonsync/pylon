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

    /// Insert or replace. Used by the configuration endpoint after a
    /// successful discovery round-trip.
    fn upsert(&self, config: OrgSsoConfig);

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

    fn upsert(&self, config: OrgSsoConfig) {
        let mut configs = self.configs.write().unwrap();
        // Remove the existing → its old domain claims (so a removed
        // domain doesn't keep routing to this org).
        if let Some(prev) = configs.get(&config.org_id) {
            let mut domains = self.domains.write().unwrap();
            for d in &prev.email_domains {
                if domains.get(d).map(|v| v == &config.org_id).unwrap_or(false) {
                    domains.remove(d);
                }
            }
        }
        let mut domains = self.domains.write().unwrap();
        for d in &config.email_domains {
            domains.insert(d.to_ascii_lowercase(), config.org_id.clone());
        }
        configs.insert(config.org_id.clone(), config);
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

/// Complete an OIDC code exchange + userinfo fetch against a configured
/// IdP. Wave 8 — handles the per-org SSO callback HTTP calls that
/// pylon-router can't do directly (router has no http client).
///
/// Returns the resolved (email, display_name) on success. Errors carry
/// a stable code for the route handler to surface in the redirect URL.
pub fn complete_oidc_login(
    config: &OrgSsoConfig,
    code: &str,
    pkce_verifier: &str,
    redirect_uri: &str,
) -> Result<(String, Option<String>), OrgSsoLoginError> {
    let secret = unseal_secret(&config.client_secret_sealed)
        .map_err(|e| OrgSsoLoginError::SecretUnreadable(e))?;
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
    let userinfo_body = ureq::get(&config.userinfo_endpoint)
        .set("Authorization", &format!("Bearer {access_token}"))
        .timeout(std::time::Duration::from_secs(10))
        .call()
        .map_err(|e| OrgSsoLoginError::UserinfoFetchFailed(format!("{e}")))?
        .into_string()
        .map_err(|e| OrgSsoLoginError::UserinfoBodyReadFailed(format!("{e}")))?;
    let userinfo: serde_json::Value =
        serde_json::from_str(&userinfo_body).unwrap_or(serde_json::Value::Null);
    let email = userinfo
        .get("email")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .ok_or(OrgSsoLoginError::NoEmailClaim)?;
    let name = userinfo
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);
    Ok((email, name))
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
    UserinfoFetchFailed(String),
    UserinfoBodyReadFailed(String),
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
            Self::UserinfoFetchFailed(_) => "USERINFO_FETCH_FAILED",
            Self::UserinfoBodyReadFailed(_) => "USERINFO_BODY_READ_FAILED",
            Self::NoEmailClaim => "NO_EMAIL_CLAIM",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::SecretUnreadable(e) => format!("could not read SSO secret: {e}"),
            Self::TokenExchangeFailed(e) => format!("token exchange failed: {e}"),
            Self::TokenBodyReadFailed(e) => format!("could not read token response: {e}"),
            Self::TokenResponseNotJson(s) => format!("token response was not JSON: {s}"),
            Self::NoAccessToken => "IdP did not return an access_token".into(),
            Self::UserinfoFetchFailed(e) => format!("userinfo fetch failed: {e}"),
            Self::UserinfoBodyReadFailed(e) => format!("could not read userinfo response: {e}"),
            Self::NoEmailClaim => "IdP userinfo response missing `email`".into(),
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
        store.upsert(config("acme", vec!["acme.com"]));
        let got = store.get("acme").unwrap();
        assert_eq!(got.client_id, "client_abc");
        assert_eq!(got.email_domains, vec!["acme.com".to_string()]);
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
        store.upsert(config("acme", vec!["acme.com"]));
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
        store.upsert(config("acme", vec!["old.com"]));
        store.upsert(config("acme", vec!["new.com"]));
        assert_eq!(store.find_by_email_domain("old.com"), None);
        assert_eq!(
            store.find_by_email_domain("new.com").as_deref(),
            Some("acme")
        );
    }

    #[test]
    fn delete_clears_domain_index_too() {
        let store = InMemoryOrgSsoStore::new();
        store.upsert(config("acme", vec!["acme.com"]));
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
