//! OpenID Connect Provider — turn pylon into an IdP that other apps
//! can sign in against. Useful for SSO across a fleet of internal
//! tools when you don't want to depend on Auth0/Okta/Cognito.
//!
//! **Status: library only — HTTP endpoints not yet wired.**
//! Discovery doc / JWKS / AuthCode types ship today so apps that
//! want to roll their own OIDC routes can compose them. The
//! pylon-shipped `/.well-known/openid-configuration` + `/oidc/*`
//! routes are queued for the next wave (need RSA key generation
//! + on-disk persistence first). Until then, do NOT advertise a
//! pylon instance as an OIDC provider in production.
//!
//! What pylon implements:
//!   - `/.well-known/openid-configuration` discovery doc
//!   - `/oidc/jwks` — public keys other services use to verify
//!     id_tokens we issue
//!   - `/oidc/authorize` — kicks off an auth-code flow
//!   - `/oidc/token` — exchange code for `id_token` + `access_token`
//!   - `/oidc/userinfo` — bearer-protected user info endpoint
//!
//! Crypto: RS256-signed id_tokens (industry default). Pylon
//! generates a fresh RSA key on first start and stores it on disk
//! (`PYLON_OIDC_KEY_PATH`, defaults to `<sessions.db>.oidc-key.pem`).
//! Same key reused across restarts so issued tokens stay valid.
//!
//! For Wave-5 we ship the discovery + jwks + verify primitives.
//! The `/authorize` + `/token` + `/userinfo` endpoint wiring lives
//! in `routes/auth.rs` and uses the existing session + scope plumbing.
//!
//! Spec: <https://openid.net/specs/openid-connect-core-1_0.html>

use serde::{Deserialize, Serialize};

/// `.well-known/openid-configuration` shape — same fields pylon's
/// OIDC client looks for in a remote IdP.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryDoc {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub userinfo_endpoint: String,
    pub jwks_uri: String,
    pub response_types_supported: Vec<String>,
    pub subject_types_supported: Vec<String>,
    pub id_token_signing_alg_values_supported: Vec<String>,
    pub scopes_supported: Vec<String>,
    pub token_endpoint_auth_methods_supported: Vec<String>,
    pub claims_supported: Vec<String>,
}

impl DiscoveryDoc {
    /// Build the discovery doc for an instance whose external
    /// address is `issuer` (e.g. `https://auth.example.com`).
    pub fn for_issuer(issuer: &str) -> Self {
        let issuer = issuer.trim_end_matches('/').to_string();
        Self {
            issuer: issuer.clone(),
            authorization_endpoint: format!("{issuer}/oidc/authorize"),
            token_endpoint: format!("{issuer}/oidc/token"),
            userinfo_endpoint: format!("{issuer}/oidc/userinfo"),
            jwks_uri: format!("{issuer}/oidc/jwks"),
            response_types_supported: vec!["code".into()],
            subject_types_supported: vec!["public".into()],
            id_token_signing_alg_values_supported: vec!["RS256".into()],
            scopes_supported: vec![
                "openid".into(),
                "email".into(),
                "profile".into(),
            ],
            token_endpoint_auth_methods_supported: vec![
                "client_secret_post".into(),
                "client_secret_basic".into(),
            ],
            claims_supported: vec![
                "sub".into(),
                "email".into(),
                "email_verified".into(),
                "name".into(),
                "preferred_username".into(),
                "picture".into(),
            ],
        }
    }
}

/// Single JWK entry for the JWKS doc. Pylon currently only emits
/// one RSA key at a time but the JWKS array shape lets you rotate
/// (publish old + new together for one signing-window) without
/// breaking in-flight tokens.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Jwk {
    pub kty: String,
    pub alg: String,
    #[serde(rename = "use")]
    pub use_: String,
    pub kid: String,
    /// Modulus, base64url-no-pad. RSA-only.
    pub n: String,
    /// Exponent, base64url-no-pad. RSA-only.
    pub e: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Jwks {
    pub keys: Vec<Jwk>,
}

impl Jwks {
    pub fn one(key: Jwk) -> Self {
        Self { keys: vec![key] }
    }
}

/// Minimal pending-authcode store. Pylon-issued auth codes are
/// random 32-byte tokens, single-use, 10-minute expiry. The stored
/// value carries the `(user_id, client_id, redirect_uri, scopes,
/// nonce, code_challenge?)` tuple so /token can re-bind the
/// originating /authorize request.
#[derive(Debug, Clone)]
pub struct AuthCode {
    pub code: String,
    pub user_id: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
    pub nonce: Option<String>,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
    pub expires_at: u64,
}

pub struct AuthCodeStore {
    codes: std::sync::Mutex<std::collections::HashMap<String, AuthCode>>,
}

impl Default for AuthCodeStore {
    fn default() -> Self {
        Self {
            codes: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }
}

impl AuthCodeStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put(&self, code: AuthCode) {
        self.codes.lock().unwrap().insert(code.code.clone(), code);
    }

    /// Atomically take a code (single-use). Returns `None` for
    /// unknown / expired codes.
    pub fn take(&self, code: &str) -> Option<AuthCode> {
        let mut map = self.codes.lock().unwrap();
        let entry = map.remove(code)?;
        if entry.expires_at <= now_secs() {
            return None;
        }
        Some(entry)
    }
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
    fn discovery_doc_uses_issuer_for_endpoints() {
        let doc = DiscoveryDoc::for_issuer("https://auth.example.com");
        assert_eq!(doc.issuer, "https://auth.example.com");
        assert_eq!(doc.authorization_endpoint, "https://auth.example.com/oidc/authorize");
        assert_eq!(doc.token_endpoint, "https://auth.example.com/oidc/token");
        assert_eq!(doc.jwks_uri, "https://auth.example.com/oidc/jwks");
        assert!(doc.id_token_signing_alg_values_supported.contains(&"RS256".to_string()));
    }

    #[test]
    fn discovery_doc_strips_trailing_slash() {
        let doc = DiscoveryDoc::for_issuer("https://auth.example.com/");
        assert_eq!(doc.issuer, "https://auth.example.com");
        assert!(doc.token_endpoint.ends_with("/oidc/token"));
        assert!(!doc.token_endpoint.contains("//oidc"));
    }

    #[test]
    fn discovery_doc_serializes_to_json() {
        let doc = DiscoveryDoc::for_issuer("https://auth.example.com");
        let json = serde_json::to_string(&doc).unwrap();
        assert!(json.contains("\"issuer\""));
        assert!(json.contains("\"jwks_uri\""));
        assert!(json.contains("\"response_types_supported\""));
    }

    #[test]
    fn jwks_serializes_canonical_shape() {
        let jwks = Jwks::one(Jwk {
            kty: "RSA".into(),
            alg: "RS256".into(),
            use_: "sig".into(),
            kid: "key-1".into(),
            n: "modulus_b64url".into(),
            e: "AQAB".into(),
        });
        let json = serde_json::to_string(&jwks).unwrap();
        // `use` is a reserved keyword — verify the rename worked.
        assert!(json.contains("\"use\":\"sig\""));
        assert!(json.contains("\"kty\":\"RSA\""));
        assert!(json.contains("\"alg\":\"RS256\""));
    }

    #[test]
    fn auth_code_store_round_trip() {
        let store = AuthCodeStore::new();
        let code = AuthCode {
            code: "tok123".into(),
            user_id: "u1".into(),
            client_id: "c1".into(),
            redirect_uri: "https://app/cb".into(),
            scopes: vec!["openid".into()],
            nonce: Some("n".into()),
            code_challenge: None,
            code_challenge_method: None,
            expires_at: 9_999_999_999,
        };
        store.put(code.clone());
        let taken = store.take("tok123").unwrap();
        assert_eq!(taken.user_id, "u1");
        // Single-use.
        assert!(store.take("tok123").is_none());
    }

    #[test]
    fn auth_code_expired_rejected() {
        let store = AuthCodeStore::new();
        store.put(AuthCode {
            code: "old".into(),
            user_id: "u1".into(),
            client_id: "c1".into(),
            redirect_uri: "x".into(),
            scopes: vec![],
            nonce: None,
            code_challenge: None,
            code_challenge_method: None,
            expires_at: 1, // ancient
        });
        assert!(store.take("old").is_none());
    }
}
