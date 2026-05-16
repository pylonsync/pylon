//! OpenID Connect Provider — turn pylon into an IdP that other apps
//! can sign in against. Useful for SSO across a fleet of internal
//! tools when you don't want to depend on Auth0/Okta/Cognito.
//!
//! **Status: full auth-code + PKCE flow shipped.** Pylon ships
//! `/.well-known/openid-configuration`, `/oidc/jwks`,
//! `/oidc/authorize`, `/oidc/token`, `/oidc/userinfo` backed by a
//! real RSA-2048 keystore (auto-generated on first start,
//! persisted at `PYLON_OIDC_KEY_PATH` with 0600 perms).
//!
//! Mandatory operator config to enable IdP mode:
//!   - `PYLON_OIDC_ISSUER` — the externally-visible issuer URL
//!     (e.g. `https://auth.example.com`). Stamped into the
//!     discovery doc + id_token `iss` claim.
//!   - `PYLON_OIDC_CLIENTS` — JSON array of registered clients:
//!     `[{"client_id":"...","client_secret":"...","redirect_uris":["..."]}]`.
//!     `client_secret` is optional (public clients use PKCE).
//!
//! Security stance:
//!   - PKCE S256 is REQUIRED for every authorize request — plain
//!     PKCE rejected per OAuth 2.1, no-PKCE rejected outright.
//!   - redirect_uri matched against the client's registered list
//!     by EXACT string compare (no path coercion).
//!   - id_token claims: iss, sub, aud, exp (10 min), iat, nonce
//!     (when supplied at /authorize), email, email_verified, name.
//!   - access_token is opaque random (32-byte b64url), TTL 1h.
//!   - refresh_token NOT issued. Clients re-do the auth code flow
//!     when the access_token expires — keeps the surface small and
//!     drops a class of long-lived bearer leak.
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
            scopes_supported: vec!["openid".into(), "email".into(), "profile".into()],
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

// ---------------------------------------------------------------------------
// RSA keystore — RS256 signing key for id_tokens.
// ---------------------------------------------------------------------------

use rsa::pkcs1v15::SigningKey;
use rsa::pkcs8::{DecodePrivateKey, EncodePrivateKey, LineEnding};
use rsa::signature::{RandomizedSigner, SignatureEncoding};
use rsa::traits::PublicKeyParts;
use rsa::{RsaPrivateKey, RsaPublicKey};
use sha2::Sha256;

/// Process-singleton handle to the OIDC signing key. Lazy-loaded
/// on first call to [`keystore_or_init`], cached for the rest of
/// the process lifetime so the JWKS route doesn't re-read the PEM
/// on every request.
///
/// Tests + dev runs that never set `PYLON_OIDC_ISSUER` skip the
/// load entirely and the JWKS endpoint returns an empty `keys`
/// array — the documented "no asymmetric verification keys
/// published" shape. Apps that want to bootstrap pylon-as-IdP set
/// the env and the keystore materializes lazily on first request.
static KEYSTORE: std::sync::OnceLock<Option<std::sync::Arc<OidcKeyStore>>> =
    std::sync::OnceLock::new();

/// Default disk location for the OIDC signing key when
/// `PYLON_OIDC_KEY_PATH` is unset. Sits next to the SQLite database
/// in the data dir; mode is forced to 0600 by load_or_generate.
fn default_key_path() -> std::path::PathBuf {
    let dir = std::env::var("PYLON_DATA_DIR")
        .or_else(|_| std::env::var("PYLON_DB_PATH"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_default());
    let dir = if dir.is_file() {
        dir.parent().map(|p| p.to_path_buf()).unwrap_or(dir)
    } else {
        dir
    };
    dir.join("oidc-signing-key.pem")
}

/// Resolve the OIDC keystore, lazy-loading on first call. Returns
/// `None` when pylon-as-IdP isn't configured for this deployment
/// (no `PYLON_OIDC_ISSUER` env var set). The route layer treats
/// None as "publish an empty JWKS" so non-IdP deployments don't
/// advertise a key they can't sign with.
///
/// Initialization happens at most once per process — subsequent
/// callers reuse the same `Arc<OidcKeyStore>`. The first caller
/// pays the key-generation cost (~50ms for RSA-2048 on a modern
/// laptop; one-time per host since the key persists to disk).
pub fn keystore_or_init() -> Option<std::sync::Arc<OidcKeyStore>> {
    KEYSTORE
        .get_or_init(|| {
            // Pylon-as-IdP is an opt-in. Without an explicit issuer
            // we refuse to spin up the keystore — operators who
            // didn't intend to run an IdP shouldn't have a 2048-bit
            // RSA key sitting on disk waiting to leak.
            if std::env::var("PYLON_OIDC_ISSUER").is_err() {
                return None;
            }
            let path = std::env::var("PYLON_OIDC_KEY_PATH")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| default_key_path());
            match OidcKeyStore::load_or_generate(&path) {
                Ok(store) => Some(std::sync::Arc::new(store)),
                Err(e) => {
                    tracing::error!("[oidc] failed to load/generate signing key at {path:?}: {e}");
                    None
                }
            }
        })
        .clone()
}

/// Persistent RSA signing key for the OIDC provider. Generates a
/// fresh 2048-bit key on first start, persists as PKCS#8 PEM at the
/// configured path. Same key reused across restarts so issued
/// id_tokens stay verifiable.
///
/// On-disk format: PKCS#8 PEM (`-----BEGIN PRIVATE KEY-----`). File
/// mode is set to 0600 on Unix so the signing key isn't world-
/// readable — without this, a webserver running as a low-priv user
/// could share the key with any other process on the same host.
pub struct OidcKeyStore {
    private: RsaPrivateKey,
    public: RsaPublicKey,
    /// Stable key id for the JWKS `kid` field. Pylon derives this
    /// from the SHA-256 fingerprint of the modulus so a rotation
    /// (delete the file, restart) produces a fresh `kid` that
    /// clients can disambiguate from the old key during the
    /// publish-both-keys-overlap window.
    kid: String,
}

impl OidcKeyStore {
    /// Load the signing key from `path`, generating + persisting a
    /// fresh one if the file doesn't exist. Caller is expected to
    /// hold an Arc<OidcKeyStore> for the lifetime of the runtime
    /// (cached at startup, never re-read).
    pub fn load_or_generate(path: &std::path::Path) -> Result<Self, String> {
        if path.exists() {
            let pem = std::fs::read_to_string(path)
                .map_err(|e| format!("read OIDC key {path:?}: {e}"))?;
            let private = RsaPrivateKey::from_pkcs8_pem(&pem)
                .map_err(|e| format!("parse OIDC key {path:?}: {e}"))?;
            let public = RsaPublicKey::from(&private);
            let kid = derive_kid(&public);
            return Ok(Self {
                private,
                public,
                kid,
            });
        }
        // First start — mint a fresh RSA-2048 key, persist with
        // owner-only perms, return.
        let mut rng = rand::thread_rng();
        let private =
            RsaPrivateKey::new(&mut rng, 2048).map_err(|e| format!("generate OIDC key: {e}"))?;
        let pem = private
            .to_pkcs8_pem(LineEnding::LF)
            .map_err(|e| format!("encode OIDC key: {e}"))?;
        // Ensure parent dir exists — `<data_dir>` may be the SQLite
        // db dir which is already there, but a freshly-configured
        // PYLON_OIDC_KEY_PATH could point at a nested location.
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create {parent:?}: {e}"))?;
        }
        std::fs::write(path, pem.as_str()).map_err(|e| format!("write OIDC key {path:?}: {e}"))?;
        // Restrict to owner read/write only. Without this, anyone
        // on the same host with read access to the data dir could
        // exfiltrate the signing key and mint forged id_tokens.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(path, perms)
                .map_err(|e| format!("chmod 0600 {path:?}: {e}"))?;
        }
        let public = RsaPublicKey::from(&private);
        let kid = derive_kid(&public);
        Ok(Self {
            private,
            public,
            kid,
        })
    }

    /// Construct a JWKS doc containing this signer's public key.
    /// The `n` + `e` fields are base64url-no-pad-encoded big-endian
    /// integers per RFC 7517 §4.
    pub fn jwks(&self) -> Jwks {
        let n_bytes = self.public.n().to_bytes_be();
        let e_bytes = self.public.e().to_bytes_be();
        Jwks::one(Jwk {
            kty: "RSA".into(),
            alg: "RS256".into(),
            use_: "sig".into(),
            kid: self.kid.clone(),
            n: b64url(&n_bytes),
            e: b64url(&e_bytes),
        })
    }

    /// Sign a set of JWT claims with this key, producing the full
    /// `header.payload.signature` compact-JWS string. `claims` must
    /// serialize to a JSON object — anything else is a programmer
    /// error so we surface as a typed Err rather than panicking.
    ///
    /// Header is built here (not taken from the caller) so the
    /// `alg` and `kid` always match the actual signing key —
    /// callers can't accidentally claim a different alg.
    pub fn sign_jwt(&self, claims: &serde_json::Value) -> Result<String, String> {
        if !claims.is_object() {
            return Err("JWT claims must be a JSON object".into());
        }
        let header = serde_json::json!({
            "alg": "RS256",
            "typ": "JWT",
            "kid": self.kid,
        });
        let header_b64 = b64url(
            serde_json::to_string(&header)
                .map_err(|e| format!("encode header: {e}"))?
                .as_bytes(),
        );
        let claims_b64 = b64url(
            serde_json::to_string(claims)
                .map_err(|e| format!("encode claims: {e}"))?
                .as_bytes(),
        );
        let signing_input = format!("{header_b64}.{claims_b64}");
        let signing_key = SigningKey::<Sha256>::new(self.private.clone());
        let mut rng = rand::thread_rng();
        let signature = signing_key.sign_with_rng(&mut rng, signing_input.as_bytes());
        let sig_b64 = b64url(signature.to_bytes().as_ref());
        Ok(format!("{signing_input}.{sig_b64}"))
    }

    pub fn kid(&self) -> &str {
        &self.kid
    }
}

/// `kid` = first 16 hex chars of SHA-256(modulus). Stable across
/// restarts (same key → same kid) but changes on rotation. We
/// hand-roll the hex encode rather than pulling in the `hex` crate
/// for one call.
fn derive_kid(public: &RsaPublicKey) -> String {
    use sha2::Digest;
    let modulus = public.n().to_bytes_be();
    let hash = Sha256::digest(&modulus);
    let mut out = String::with_capacity(16);
    for b in &hash[..8] {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn b64url(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

// ---------------------------------------------------------------------------
// Registered clients + PKCE
// ---------------------------------------------------------------------------

/// One OIDC client registered with this pylon instance. The pylon
/// manifest's `auth.oidc.clients` array deserializes into a Vec of
/// these. Confidential clients have a `client_secret`; public
/// clients (SPAs, native apps) leave it None and MUST use PKCE.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct OidcClient {
    pub client_id: String,
    /// SHA-256 fingerprint of the secret would be preferable here
    /// to keep secrets out of the manifest, but most apps register
    /// clients ahead of time and check in their manifest; the
    /// pragmatic shape is a plain shared secret loaded from the
    /// app config the same way the rest of the server's secrets
    /// are. Comparisons go through `constant_time_eq`.
    #[serde(default)]
    pub client_secret: Option<String>,
    pub redirect_uris: Vec<String>,
}

impl OidcClient {
    /// Authenticate a /token request that presented `client_id` +
    /// optional `client_secret`. Public clients (no secret in
    /// registration) ignore the presented secret. Confidential
    /// clients require the secret to match (constant-time).
    pub fn authenticate(&self, presented_secret: Option<&str>) -> bool {
        match (self.client_secret.as_deref(), presented_secret) {
            (None, _) => true, // public client
            (Some(stored), Some(presented)) => {
                crate::constant_time_eq(stored.as_bytes(), presented.as_bytes())
            }
            (Some(_), None) => false,
        }
    }

    /// Check whether `redirect_uri` matches an entry in this
    /// client's allowlist. EXACT string match per OAuth 2.1 §3.1.2
    /// — no path/query coercion, no suffix matching. An open
    /// redirect here is the textbook OIDC-IdP foot-gun.
    pub fn is_registered_redirect(&self, redirect_uri: &str) -> bool {
        self.redirect_uris.iter().any(|u| u == redirect_uri)
    }
}

// ---------------------------------------------------------------------------
// AccessTokenStore — opaque bearer tokens for the userinfo endpoint
// ---------------------------------------------------------------------------

/// One issued access_token. Pylon's tokens are opaque (32 random
/// bytes base64url-encoded) rather than self-contained JWTs — the
/// userinfo endpoint resolves the bearer to a user_id via this
/// store, then projects the User row's safe-for-client fields.
/// Opaque tokens make revocation O(1) (just remove the row) which
/// the JWT-shaped alternative makes painful.
#[derive(Debug, Clone)]
pub struct AccessToken {
    pub token: String,
    pub user_id: String,
    pub client_id: String,
    pub scopes: Vec<String>,
    pub expires_at: u64,
}

pub struct AccessTokenStore {
    tokens: std::sync::Mutex<std::collections::HashMap<String, AccessToken>>,
}

impl Default for AccessTokenStore {
    fn default() -> Self {
        Self {
            tokens: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }
}

impl AccessTokenStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put(&self, t: AccessToken) {
        self.tokens.lock().unwrap().insert(t.token.clone(), t);
    }

    /// Look up a token by string. Returns None for unknown or
    /// expired tokens; an expired token is also evicted as a
    /// best-effort cleanup so the map doesn't grow without bound.
    pub fn get(&self, token: &str) -> Option<AccessToken> {
        let mut map = self.tokens.lock().unwrap();
        let entry = map.get(token).cloned()?;
        if entry.expires_at <= now_secs() {
            map.remove(token);
            return None;
        }
        Some(entry)
    }
}

// ---------------------------------------------------------------------------
// Process-singleton: provider state (keystore + clients + code/token stores)
// ---------------------------------------------------------------------------

/// All the state a pylon-as-IdP needs that the HTTP routes share
/// across requests. Lazy-initialized on first /oidc/* request that
/// requires it.
///
/// Wraps the keystore + the client registry + the auth code store
/// + the access token store in one Arc-able bundle so the route
/// layer does ONE lookup per request.
pub struct OidcProvider {
    pub keystore: std::sync::Arc<OidcKeyStore>,
    pub clients: Vec<OidcClient>,
    pub auth_codes: AuthCodeStore,
    pub access_tokens: AccessTokenStore,
}

impl OidcProvider {
    pub fn find_client(&self, client_id: &str) -> Option<&OidcClient> {
        self.clients.iter().find(|c| c.client_id == client_id)
    }
}

static PROVIDER: std::sync::OnceLock<Option<std::sync::Arc<OidcProvider>>> =
    std::sync::OnceLock::new();

/// Resolve the process-wide OIDC provider state, lazy-loading on
/// first call. Returns None when pylon-as-IdP isn't configured
/// (no `PYLON_OIDC_ISSUER`). Routes treat None as "this isn't an
/// IdP" and return 404.
pub fn provider_or_init() -> Option<std::sync::Arc<OidcProvider>> {
    PROVIDER
        .get_or_init(|| {
            if std::env::var("PYLON_OIDC_ISSUER").is_err() {
                return None;
            }
            // Keystore lazy-init reuses the same opt-in env, so this
            // never produces a key without an issuer set.
            let keystore = keystore_or_init()?;
            let clients = load_clients_from_env();
            if clients.is_empty() {
                tracing::warn!(
                    "[oidc] PYLON_OIDC_ISSUER set but PYLON_OIDC_CLIENTS missing/empty — \
                     no registered clients, /oidc/authorize will reject every request"
                );
            }
            Some(std::sync::Arc::new(OidcProvider {
                keystore,
                clients,
                auth_codes: AuthCodeStore::new(),
                access_tokens: AccessTokenStore::new(),
            }))
        })
        .clone()
}

/// Parse `PYLON_OIDC_CLIENTS` env var as a JSON array of
/// OidcClient. Bad JSON or wrong shape produces an empty list +
/// a tracing::error — the route layer then refuses every authorize
/// request, surfacing the misconfiguration loudly instead of
/// silently allowing whatever client_id arrives.
fn load_clients_from_env() -> Vec<OidcClient> {
    let raw = match std::env::var("PYLON_OIDC_CLIENTS") {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    match serde_json::from_str::<Vec<OidcClient>>(&raw) {
        Ok(clients) => clients,
        Err(e) => {
            tracing::error!(
                "[oidc] PYLON_OIDC_CLIENTS invalid JSON ({e}) — refusing all authorize requests"
            );
            Vec::new()
        }
    }
}

/// Generate a fresh URL-safe 32-byte token (auth code or access
/// token). Uses the rand crate; 256 bits of entropy from the OS
/// CSPRNG is overkill but adequate.
pub fn mint_random_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    b64url(&bytes)
}

/// Verify a PKCE challenge per RFC 7636 §4.6.
///
/// - `method` = "S256" (REQUIRED — `plain` is deprecated by
///   OAuth 2.1; pylon refuses it outright)
/// - `code_verifier` is the original random string the client kept
/// - `stored_challenge` is what the client sent at /authorize
///
/// Hash: SHA-256(code_verifier), base64url-no-pad → must equal
/// stored_challenge.
pub fn verify_pkce(method: &str, code_verifier: &str, stored_challenge: &str) -> bool {
    if method != "S256" {
        return false;
    }
    // RFC 7636 §4.1: verifier is 43..=128 chars from the unreserved
    // set. We check length but not character set — a malformed
    // verifier just fails the hash compare anyway, and the
    // additional check would only ever produce identical wire
    // behavior (false). Caller-supplied verifier reaching this
    // path is post-input-validation, so we trust the bytes.
    if code_verifier.len() < 43 || code_verifier.len() > 128 {
        return false;
    }
    use sha2::Digest;
    let hash = Sha256::digest(code_verifier.as_bytes());
    let computed = b64url(&hash);
    crate::constant_time_eq(computed.as_bytes(), stored_challenge.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_doc_uses_issuer_for_endpoints() {
        let doc = DiscoveryDoc::for_issuer("https://auth.example.com");
        assert_eq!(doc.issuer, "https://auth.example.com");
        assert_eq!(
            doc.authorization_endpoint,
            "https://auth.example.com/oidc/authorize"
        );
        assert_eq!(doc.token_endpoint, "https://auth.example.com/oidc/token");
        assert_eq!(doc.jwks_uri, "https://auth.example.com/oidc/jwks");
        assert!(doc
            .id_token_signing_alg_values_supported
            .contains(&"RS256".to_string()));
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

    fn tmp_key_path() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pylon-oidc-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("oidc-signing-key.pem")
    }

    #[test]
    fn keystore_generates_and_persists_key() {
        let path = tmp_key_path();
        let store1 = OidcKeyStore::load_or_generate(&path).expect("generate");
        assert!(path.exists());
        // Second load reuses the persisted key — same kid.
        let store2 = OidcKeyStore::load_or_generate(&path).expect("reload");
        assert_eq!(store1.kid(), store2.kid());
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(path.parent().unwrap());
    }

    #[test]
    #[cfg(unix)]
    fn keystore_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let path = tmp_key_path();
        OidcKeyStore::load_or_generate(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "OIDC key file must be owner-only, got 0o{mode:o}"
        );
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(path.parent().unwrap());
    }

    #[test]
    fn sign_jwt_produces_three_part_compact_jws() {
        let path = tmp_key_path();
        let store = OidcKeyStore::load_or_generate(&path).unwrap();
        let claims = serde_json::json!({
            "iss": "https://auth.example.com",
            "sub": "user_1",
            "aud": "client_1",
            "exp": now_secs() + 3600,
            "iat": now_secs(),
        });
        let token = store.sign_jwt(&claims).expect("sign");
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3, "JWT must have header.payload.signature");
        // Header must decode + carry alg=RS256 + the keystore's kid.
        use base64::Engine;
        let header_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[0])
            .unwrap();
        let header: serde_json::Value = serde_json::from_slice(&header_bytes).unwrap();
        assert_eq!(header["alg"], "RS256");
        assert_eq!(header["kid"], store.kid());
        // Payload round-trips.
        let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[1])
            .unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&payload_bytes).unwrap();
        assert_eq!(payload["sub"], "user_1");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(path.parent().unwrap());
    }

    #[test]
    fn sign_jwt_rejects_non_object_claims() {
        let path = tmp_key_path();
        let store = OidcKeyStore::load_or_generate(&path).unwrap();
        let err = store
            .sign_jwt(&serde_json::json!("not an object"))
            .unwrap_err();
        assert!(err.contains("JSON object"));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(path.parent().unwrap());
    }

    #[test]
    fn jwks_exposes_real_modulus() {
        let path = tmp_key_path();
        let store = OidcKeyStore::load_or_generate(&path).unwrap();
        let jwks = store.jwks();
        assert_eq!(jwks.keys.len(), 1);
        let jwk = &jwks.keys[0];
        assert_eq!(jwk.kty, "RSA");
        assert_eq!(jwk.alg, "RS256");
        assert_eq!(jwk.use_, "sig");
        assert_eq!(jwk.e, "AQAB"); // 65537, the standard RSA exponent
                                   // n is base64url-no-pad; for a 2048-bit key the decoded
                                   // length should be ~256 bytes.
        use base64::Engine;
        let n_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(&jwk.n)
            .unwrap();
        assert!(n_bytes.len() >= 250 && n_bytes.len() <= 257);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(path.parent().unwrap());
    }

    #[test]
    fn pkce_s256_round_trip() {
        // From RFC 7636 §4.6 test vectors:
        //   verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
        //   challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert!(verify_pkce("S256", verifier, challenge));
        // Mismatched verifier rejected.
        assert!(!verify_pkce(
            "S256",
            "wrong-verifier-but-right-length-string-padding-aa",
            challenge
        ));
    }

    #[test]
    fn pkce_rejects_plain_method() {
        // `plain` is the other PKCE method but OAuth 2.1 deprecates
        // it. Pylon refuses to accept it — only S256.
        assert!(!verify_pkce("plain", "verifier", "verifier"));
    }

    #[test]
    fn pkce_verifier_length_bounds_enforced() {
        // RFC 7636 §4.1: verifier MUST be 43..=128 chars.
        let short = "a".repeat(42);
        let long = "a".repeat(129);
        assert!(!verify_pkce("S256", &short, "ignored"));
        assert!(!verify_pkce("S256", &long, "ignored"));
    }

    #[test]
    fn oidc_client_public_client_authenticates_without_secret() {
        let c = OidcClient {
            client_id: "c1".into(),
            client_secret: None,
            redirect_uris: vec!["https://app/cb".into()],
        };
        assert!(c.authenticate(None));
        assert!(c.authenticate(Some("anything")));
    }

    #[test]
    fn oidc_client_confidential_requires_matching_secret() {
        let c = OidcClient {
            client_id: "c1".into(),
            client_secret: Some("shh".into()),
            redirect_uris: vec![],
        };
        assert!(c.authenticate(Some("shh")));
        assert!(!c.authenticate(Some("wrong")));
        assert!(!c.authenticate(None));
    }

    #[test]
    fn access_token_round_trip_and_expiry() {
        let store = AccessTokenStore::new();
        store.put(AccessToken {
            token: "tok-1".into(),
            user_id: "u-1".into(),
            client_id: "c-1".into(),
            scopes: vec!["openid".into(), "email".into()],
            expires_at: now_secs() + 60,
        });
        let got = store.get("tok-1").expect("active token resolves");
        assert_eq!(got.user_id, "u-1");
        assert_eq!(got.scopes, vec!["openid".to_string(), "email".to_string()]);
        // Expired token disappears + returns None.
        store.put(AccessToken {
            token: "tok-2".into(),
            user_id: "u-2".into(),
            client_id: "c-1".into(),
            scopes: vec![],
            expires_at: 1, // ancient
        });
        assert!(store.get("tok-2").is_none());
        // Best-effort eviction means a second get also gets None.
        assert!(store.get("tok-2").is_none());
    }

    #[test]
    fn mint_random_token_is_base64url_no_pad() {
        let t = mint_random_token();
        // 32 bytes base64url no-pad = 43 chars.
        assert_eq!(t.len(), 43);
        assert!(!t.contains('='));
        assert!(!t.contains('+'));
        assert!(!t.contains('/'));
        assert!(!t.contains('\n'));
        // Two consecutive mints differ.
        assert_ne!(t, mint_random_token());
    }

    #[test]
    fn redirect_uri_must_match_exactly() {
        let c = OidcClient {
            client_id: "c1".into(),
            client_secret: None,
            redirect_uris: vec!["https://app.example.com/cb".into()],
        };
        assert!(c.is_registered_redirect("https://app.example.com/cb"));
        // Path-suffix variations rejected — open-redirect prevention.
        assert!(!c.is_registered_redirect("https://app.example.com/cb/extra"));
        assert!(!c.is_registered_redirect("https://app.example.com/cb?next=evil"));
        assert!(!c.is_registered_redirect("https://app.example.com.evil.com/cb"));
        assert!(!c.is_registered_redirect("http://app.example.com/cb"));
    }
}
