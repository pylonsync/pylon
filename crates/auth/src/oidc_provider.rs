//! OpenID Connect Provider — turn pylon into an IdP that other apps
//! can sign in against. Useful for SSO across a fleet of internal
//! tools when you don't want to depend on Auth0/Okta/Cognito.
//!
//! **Status: full auth-code + PKCE flow, refresh tokens, and token
//! revocation shipped.** Pylon ships `/.well-known/openid-configuration`,
//! `/oidc/jwks`, `/oidc/authorize`, `/oidc/token`, `/oidc/userinfo` and
//! `/oidc/revoke` backed by a real RSA-2048 keystore (auto-generated on
//! first start, persisted at `PYLON_OIDC_KEY_PATH` with 0600 perms).
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
//!   - refresh_token is issued only when the grant includes the
//!     `offline_access` scope. TTL 30 days, renewed on every use: each
//!     refresh returns a new refresh token and marks the presented one
//!     rotated. A rotated token still works for
//!     [`REFRESH_REUSE_GRACE_SECS`] so two concurrent refreshes from one
//!     app both succeed; after that, presenting it is treated as theft and
//!     revokes the whole grant.
//!   - Access and refresh tokens are stored as SHA-256 hashes in the
//!     persistent [`OidcTokenStore`](crate::oidc_tokens::OidcTokenStore)
//!     (SQLite or Postgres), so they survive restarts and are shared by
//!     every replica. Auth codes stay in process memory: they live 10
//!     minutes, are single-use, and a lost code only means the user
//!     signs in again.
//!
//! What pylon implements:
//!   - `/.well-known/openid-configuration` discovery doc
//!   - `/oidc/jwks` — public keys other services use to verify
//!     id_tokens we issue
//!   - `/oidc/authorize` — kicks off an auth-code flow
//!   - `/oidc/token` — authorization_code, refresh_token, and RFC 8693
//!     token exchange grants
//!   - `/oidc/userinfo` — bearer-protected user info endpoint
//!   - `/oidc/revoke` — RFC 7009 revocation; revokes the whole grant
//!
//! Crypto: RS256-signed id_tokens (industry default). Pylon
//! generates a fresh RSA key on first start and stores it on disk
//! (`PYLON_OIDC_KEY_PATH`, defaults to `<sessions.db>.oidc-key.pem`).
//! Same key reused across restarts so issued tokens stay valid.
//!
//! The endpoint wiring lives in `routes/auth.rs` and uses the existing
//! session + scope plumbing.
//!
//! Spec: <https://openid.net/specs/openid-connect-core-1_0.html>

use serde::{Deserialize, Serialize};

use crate::oidc_tokens::{
    hash_token, InMemoryOidcTokenStore, OidcTokenKind, OidcTokenRecord, OidcTokenStore,
};

/// `.well-known/openid-configuration` shape — same fields pylon's
/// OIDC client looks for in a remote IdP.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryDoc {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub userinfo_endpoint: String,
    pub jwks_uri: String,
    pub revocation_endpoint: String,
    pub response_types_supported: Vec<String>,
    pub grant_types_supported: Vec<String>,
    pub subject_types_supported: Vec<String>,
    pub id_token_signing_alg_values_supported: Vec<String>,
    pub scopes_supported: Vec<String>,
    pub token_endpoint_auth_methods_supported: Vec<String>,
    pub revocation_endpoint_auth_methods_supported: Vec<String>,
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
            revocation_endpoint: format!("{issuer}/oidc/revoke"),
            response_types_supported: vec!["code".into()],
            grant_types_supported: vec![
                "authorization_code".into(),
                "refresh_token".into(),
                TOKEN_EXCHANGE_GRANT.into(),
            ],
            subject_types_supported: vec!["public".into()],
            id_token_signing_alg_values_supported: vec!["RS256".into()],
            // "orgs" is a Pylon-convention scope: the id_token/userinfo carry
            // the user's Org memberships ({id, name, role}) so a federated
            // Pylon app can map tenants, not just users. Pylon CLIENTS request
            // it automatically when they see it advertised here; third-party
            // clients ignore it, which is the correct default for a claim
            // that enumerates a user's workspaces.
            //
            // "offline_access" (OIDC Core §11) asks for a refresh token.
            // Pylon clients request it, like "orgs", when a Pylon IdP
            // advertises it (`provider::discovered_scopes`).
            scopes_supported: vec![
                "openid".into(),
                "email".into(),
                "profile".into(),
                "orgs".into(),
                OFFLINE_ACCESS_SCOPE.into(),
            ],
            token_endpoint_auth_methods_supported: client_auth_methods(),
            revocation_endpoint_auth_methods_supported: client_auth_methods(),
            claims_supported: vec![
                "sub".into(),
                "email".into(),
                "email_verified".into(),
                "name".into(),
                "preferred_username".into(),
                "picture".into(),
                "orgs".into(),
            ],
        }
    }
}

/// How clients authenticate at `/oidc/token` and `/oidc/revoke`.
fn client_auth_methods() -> Vec<String> {
    vec!["client_secret_post".into(), "client_secret_basic".into()]
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
        pylon_kernel::secret_file::restrict_to_owner(path)
            .map_err(|e| format!("restrict {path:?} to owner: {e}"))?;
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

/// RFC 8693 grant type for trading an IdP session for a client's id_token.
pub const TOKEN_EXCHANGE_GRANT: &str = "urn:ietf:params:oauth:grant-type:token-exchange";
/// RFC 8693 token type of the subject token: a session on this IdP.
pub const ACCESS_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:access_token";
/// The scopes a token exchange may ask for. `openid` is required.
pub const EXCHANGE_SCOPES: &[&str] = &["openid", "email", "profile", "orgs"];

/// The scopes for a token exchange: the requested ones this IdP knows, or
/// all of [`EXCHANGE_SCOPES`] when none were requested. `None` when the
/// request leaves out `openid`.
pub fn exchange_scopes(requested: Option<&str>) -> Option<Vec<String>> {
    let scopes: Vec<String> = match requested.map(str::trim).filter(|s| !s.is_empty()) {
        None => EXCHANGE_SCOPES.iter().map(|s| s.to_string()).collect(),
        Some(list) => {
            let mut out: Vec<String> = Vec::new();
            for s in list.split_whitespace() {
                if EXCHANGE_SCOPES.contains(&s) && !out.iter().any(|o| o == s) {
                    out.push(s.to_string());
                }
            }
            out
        }
    };
    scopes.iter().any(|s| s == "openid").then_some(scopes)
}

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
    /// Allow RFC 8693 token exchange for this client: a caller holding a
    /// session on this IdP (a native app that signed in here with Apple,
    /// Google or an email code) trades it at `/oidc/token` for an id_token
    /// addressed to this client, then signs in to the client app with
    /// `POST /api/auth/federated/<provider>`. No client secret is involved,
    /// because the caller is the user's own device, so turn it on only for
    /// first-party apps. Default off.
    #[serde(default)]
    pub token_exchange: bool,
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
// Token lifetimes + scopes
// ---------------------------------------------------------------------------

/// Access token lifetime.
pub const ACCESS_TOKEN_TTL_SECS: u64 = 3600;
/// Refresh token lifetime. Each refresh issues a new refresh token with a
/// fresh 30 days, so a grant stays alive while the client keeps using it.
pub const REFRESH_TOKEN_TTL_SECS: u64 = 30 * 24 * 3600;
/// How long a rotated refresh token still works. Two requests from one app
/// that refresh at the same moment both present the same token; the
/// second one lands inside this window and succeeds.
pub const REFRESH_REUSE_GRACE_SECS: u64 = 60;
/// The OIDC scope that asks for a refresh token.
pub const OFFLINE_ACCESS_SCOPE: &str = "offline_access";
/// The longest a stale-row sweep is skipped. Sweeps run on token issue.
const SWEEP_INTERVAL_SECS: u64 = 300;

/// Tokens returned from one issue.
#[derive(Debug, Clone)]
pub struct IssuedTokens {
    pub grant_id: String,
    pub access_token: String,
    pub access_expires_in: u64,
    pub refresh_token: Option<String>,
    pub refresh_expires_in: Option<u64>,
}

/// Why a refresh was refused. Every variant except `Storage` is
/// `invalid_grant` on the wire (RFC 6749 §5.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshDenied {
    /// Unknown, expired, revoked, or not a refresh token.
    Invalid,
    /// The token belongs to another client.
    WrongClient,
    /// A rotated token came back after the grace window. The grant is
    /// revoked.
    Reused { grant_id: String, revoked: usize },
    /// The token store failed.
    Storage(String),
}

/// A refresh token that passed every check, with its grant.
#[derive(Debug, Clone)]
pub struct AcceptedRefresh {
    pub grant_id: String,
    pub user_id: String,
    pub client_id: String,
    pub scopes: Vec<String>,
}

// ---------------------------------------------------------------------------
// Process-singleton: provider state (keystore + clients + code/token stores)
// ---------------------------------------------------------------------------

/// All the state a pylon-as-IdP needs that the HTTP routes share
/// across requests. Lazy-initialized on first /oidc/* request that
/// requires it.
///
/// Wraps the keystore + the client registry + the auth code store
/// + the token store in one Arc-able bundle so the route layer does
/// ONE lookup per request.
pub struct OidcProvider {
    pub keystore: std::sync::Arc<OidcKeyStore>,
    pub clients: Vec<OidcClient>,
    pub auth_codes: AuthCodeStore,
    pub tokens: std::sync::Arc<dyn OidcTokenStore>,
    last_sweep: std::sync::atomic::AtomicU64,
}

impl OidcProvider {
    pub fn new(
        keystore: std::sync::Arc<OidcKeyStore>,
        clients: Vec<OidcClient>,
        tokens: std::sync::Arc<dyn OidcTokenStore>,
    ) -> Self {
        Self {
            keystore,
            clients,
            auth_codes: AuthCodeStore::new(),
            tokens,
            last_sweep: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub fn find_client(&self, client_id: &str) -> Option<&OidcClient> {
        self.clients.iter().find(|c| c.client_id == client_id)
    }

    /// Issue an access token, plus a refresh token when `refresh_scopes`
    /// is given and includes `offline_access`. Pass the grant id of the
    /// refresh token being exchanged, or `None` to start a new grant.
    ///
    /// `access_scopes` may be narrower than `refresh_scopes` (a refresh
    /// that asked for less); the refresh token keeps the grant's full
    /// scopes per RFC 6749 §6.
    pub fn issue_tokens(
        &self,
        grant_id: Option<&str>,
        user_id: &str,
        client_id: &str,
        access_scopes: &[String],
        refresh_scopes: Option<&[String]>,
    ) -> Result<IssuedTokens, String> {
        let now = now_secs();
        self.maybe_sweep(now);
        let grant_id = grant_id.map(String::from).unwrap_or_else(mint_random_token);
        let record =
            |token: &str, kind: OidcTokenKind, scopes: &[String], ttl: u64| OidcTokenRecord {
                token_hash: hash_token(token),
                kind,
                grant_id: grant_id.clone(),
                user_id: user_id.to_string(),
                client_id: client_id.to_string(),
                scopes: scopes.to_vec(),
                expires_at: now + ttl,
                created_at: now,
                rotated_at: None,
            };
        let access_token = mint_random_token();
        self.tokens.insert(&record(
            &access_token,
            OidcTokenKind::Access,
            access_scopes,
            ACCESS_TOKEN_TTL_SECS,
        ))?;
        let refresh_scopes = refresh_scopes.filter(|s| s.iter().any(|x| x == OFFLINE_ACCESS_SCOPE));
        let refresh_token = match refresh_scopes {
            Some(scopes) => {
                let token = mint_random_token();
                self.tokens.insert(&record(
                    &token,
                    OidcTokenKind::Refresh,
                    scopes,
                    REFRESH_TOKEN_TTL_SECS,
                ))?;
                Some(token)
            }
            None => None,
        };
        Ok(IssuedTokens {
            grant_id,
            access_token,
            access_expires_in: ACCESS_TOKEN_TTL_SECS,
            refresh_expires_in: refresh_token.as_ref().map(|_| REFRESH_TOKEN_TTL_SECS),
            refresh_token,
        })
    }

    /// Resolve a live access token. `Ok(None)` for unknown, expired,
    /// revoked, and refresh tokens.
    pub fn lookup_access_token(&self, token: &str) -> Result<Option<OidcTokenRecord>, String> {
        let found = self.tokens.find(&hash_token(token), now_secs())?;
        Ok(found.filter(|r| r.kind == OidcTokenKind::Access))
    }

    /// Check a refresh token presented by `client_id` and rotate it.
    /// On success the caller issues new tokens under the returned grant.
    pub fn accept_refresh_token(
        &self,
        client_id: &str,
        refresh_token: &str,
    ) -> Result<AcceptedRefresh, RefreshDenied> {
        let now = now_secs();
        let hash = hash_token(refresh_token);
        let record = match self.tokens.find(&hash, now) {
            Ok(Some(r)) if r.kind == OidcTokenKind::Refresh => r,
            Ok(_) => return Err(RefreshDenied::Invalid),
            Err(e) => return Err(RefreshDenied::Storage(e)),
        };
        // A token issued to another client is refused without touching it:
        // the caller does not hold that client's credentials, so it must
        // not be able to burn the real client's token either.
        if record.client_id != client_id {
            return Err(RefreshDenied::WrongClient);
        }
        let rotated_at = match self.tokens.mark_rotated(&hash, now) {
            Ok(Some(t)) => t,
            // Revoked between the find and the update.
            Ok(None) => return Err(RefreshDenied::Invalid),
            Err(e) => return Err(RefreshDenied::Storage(e)),
        };
        if now > rotated_at + REFRESH_REUSE_GRACE_SECS {
            let revoked = self
                .tokens
                .revoke_grant(&record.grant_id)
                .map_err(RefreshDenied::Storage)?;
            return Err(RefreshDenied::Reused {
                grant_id: record.grant_id,
                revoked,
            });
        }
        Ok(AcceptedRefresh {
            grant_id: record.grant_id,
            user_id: record.user_id,
            client_id: record.client_id,
            scopes: record.scopes,
        })
    }

    /// RFC 7009: revoke the grant of `token` (access or refresh) when it
    /// was issued to `client_id`. Returns the number of tokens removed;
    /// 0 for unknown tokens and tokens of other clients.
    pub fn revoke_token(&self, client_id: &str, token: &str) -> Result<usize, String> {
        match self.tokens.find(&hash_token(token), now_secs())? {
            Some(r) if r.client_id == client_id => self.tokens.revoke_grant(&r.grant_id),
            _ => Ok(0),
        }
    }

    /// Revoke every grant of a user. Returns the number of grants.
    pub fn revoke_user(&self, user_id: &str) -> Result<usize, String> {
        self.tokens.revoke_user(user_id)
    }

    /// Remove expired rows at most once per [`SWEEP_INTERVAL_SECS`]. Lookups
    /// already delete the expired rows they meet; this catches the rows
    /// nobody looks up again.
    fn maybe_sweep(&self, now: u64) {
        use std::sync::atomic::Ordering;
        let last = self.last_sweep.load(Ordering::Relaxed);
        if now < last + SWEEP_INTERVAL_SECS {
            return;
        }
        if self
            .last_sweep
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {
            return; // another request is sweeping
        }
        match self.tokens.sweep_expired(now) {
            Ok(0) => {}
            Ok(n) => tracing::debug!("[oidc] swept {n} expired token rows"),
            Err(e) => tracing::warn!("[oidc] expired-token sweep failed: {e}"),
        }
    }
}

static PROVIDER: std::sync::OnceLock<Option<std::sync::Arc<OidcProvider>>> =
    std::sync::OnceLock::new();

/// The token store the provider singleton uses.
///
/// The server installs its SQLite or Postgres store here at boot
/// ([`install_token_store`]), before it accepts requests, instead of the
/// store being a field threaded through every `RouterContext`: the
/// provider is already a process singleton built from env on first use,
/// and every route and function host that reaches it goes through
/// [`provider_or_init`]. If nothing was installed by first use (unit
/// tests, tools that never boot a server) the provider gets an in-memory
/// store.
static TOKEN_STORE: std::sync::OnceLock<std::sync::Arc<dyn OidcTokenStore>> =
    std::sync::OnceLock::new();

/// Give the provider its persistent token store. Call once at boot before
/// serving requests. Returns `false` when a store was already in place
/// (a second server in the same process, as in tests); the first stays.
pub fn install_token_store(store: std::sync::Arc<dyn OidcTokenStore>) -> bool {
    TOKEN_STORE.set(store).is_ok()
}

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
            let tokens = TOKEN_STORE
                .get_or_init(|| {
                    tracing::warn!(
                        "[oidc] no persistent token store installed; OIDC tokens live in \
                         memory and are lost on restart"
                    );
                    std::sync::Arc::new(InMemoryOidcTokenStore::new())
                })
                .clone();
            Some(std::sync::Arc::new(OidcProvider::new(
                keystore, clients, tokens,
            )))
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
    fn exchange_scopes_default_to_all_and_require_openid() {
        assert_eq!(
            exchange_scopes(None).unwrap(),
            vec!["openid", "email", "profile", "orgs"]
        );
        assert_eq!(
            exchange_scopes(Some("openid orgs admin orgs")).unwrap(),
            vec!["openid", "orgs"]
        );
        assert!(exchange_scopes(Some("email orgs")).is_none());
    }

    #[test]
    fn token_exchange_is_off_unless_the_client_turns_it_on() {
        let clients: Vec<OidcClient> = serde_json::from_str(
            r#"[{"client_id":"a","redirect_uris":[]},{"client_id":"b","redirect_uris":[],"token_exchange":true}]"#,
        )
        .unwrap();
        assert!(!clients[0].token_exchange);
        assert!(clients[1].token_exchange);
    }

    #[test]
    fn oidc_client_public_client_authenticates_without_secret() {
        let c = OidcClient {
            client_id: "c1".into(),
            client_secret: None,
            redirect_uris: vec!["https://app/cb".into()],
            token_exchange: false,
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
            token_exchange: false,
        };
        assert!(c.authenticate(Some("shh")));
        assert!(!c.authenticate(Some("wrong")));
        assert!(!c.authenticate(None));
    }

    fn test_provider(tokens: std::sync::Arc<dyn OidcTokenStore>) -> OidcProvider {
        // One RSA key per test binary: keygen is slow and these tests only
        // exercise tokens.
        static KEY: std::sync::OnceLock<std::sync::Arc<OidcKeyStore>> = std::sync::OnceLock::new();
        let keystore = KEY
            .get_or_init(|| {
                std::sync::Arc::new(OidcKeyStore::load_or_generate(&tmp_key_path()).unwrap())
            })
            .clone();
        OidcProvider::new(keystore, vec![], tokens)
    }

    fn scopes(list: &str) -> Vec<String> {
        list.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn access_token_resolves_and_refresh_token_is_not_an_access_token() {
        let p = test_provider(std::sync::Arc::new(InMemoryOidcTokenStore::new()));
        let s = scopes("openid email offline_access");
        let issued = p.issue_tokens(None, "u1", "c1", &s, Some(&s)).unwrap();
        let access = p
            .lookup_access_token(&issued.access_token)
            .unwrap()
            .expect("access token resolves");
        assert_eq!(access.user_id, "u1");
        assert_eq!(access.client_id, "c1");
        assert_eq!(access.scopes, s);
        assert_eq!(access.grant_id, issued.grant_id);
        let refresh = issued.refresh_token.expect("offline_access issues one");
        assert!(p.lookup_access_token(&refresh).unwrap().is_none());
        assert!(p.lookup_access_token("not-a-token").unwrap().is_none());
    }

    #[test]
    fn only_raw_token_hashes_are_stored() {
        let store = std::sync::Arc::new(InMemoryOidcTokenStore::new());
        let p = test_provider(store.clone());
        let s = scopes("openid offline_access");
        let issued = p.issue_tokens(None, "u1", "c1", &s, Some(&s)).unwrap();
        let now = now_secs();
        assert!(store.find(&issued.access_token, now).unwrap().is_none());
        assert!(store
            .find(&hash_token(&issued.access_token), now)
            .unwrap()
            .is_some());
    }

    #[test]
    fn no_offline_access_means_no_refresh_token() {
        let p = test_provider(std::sync::Arc::new(InMemoryOidcTokenStore::new()));
        let s = scopes("openid email");
        let issued = p.issue_tokens(None, "u1", "c1", &s, Some(&s)).unwrap();
        assert!(issued.refresh_token.is_none());
        assert!(issued.refresh_expires_in.is_none());
    }

    #[test]
    fn refresh_rotates_within_grace_and_keeps_the_grant() {
        let p = test_provider(std::sync::Arc::new(InMemoryOidcTokenStore::new()));
        let s = scopes("openid offline_access");
        let first = p.issue_tokens(None, "u1", "c1", &s, Some(&s)).unwrap();
        let rt = first.refresh_token.unwrap();
        let accepted = p.accept_refresh_token("c1", &rt).unwrap();
        assert_eq!(accepted.grant_id, first.grant_id);
        assert_eq!(accepted.user_id, "u1");
        assert_eq!(accepted.scopes, s);
        // The same token again, inside the grace window: still accepted.
        let again = p.accept_refresh_token("c1", &rt).unwrap();
        assert_eq!(again.grant_id, first.grant_id);
        let second = p
            .issue_tokens(Some(&accepted.grant_id), "u1", "c1", &s, Some(&s))
            .unwrap();
        assert_eq!(second.grant_id, first.grant_id);
        assert_ne!(second.refresh_token.as_deref(), Some(rt.as_str()));
    }

    #[test]
    fn late_reuse_of_a_rotated_refresh_token_revokes_the_grant() {
        let store = std::sync::Arc::new(InMemoryOidcTokenStore::new());
        let p = test_provider(store.clone());
        let s = scopes("openid offline_access");
        let first = p.issue_tokens(None, "u1", "c1", &s, Some(&s)).unwrap();
        let rt = first.refresh_token.unwrap();
        let next = p
            .issue_tokens(Some(&first.grant_id), "u1", "c1", &s, Some(&s))
            .unwrap();
        // Rotated two minutes ago: past the grace window.
        store
            .mark_rotated(&hash_token(&rt), now_secs() - 2 * REFRESH_REUSE_GRACE_SECS)
            .unwrap();
        match p.accept_refresh_token("c1", &rt) {
            Err(RefreshDenied::Reused { grant_id, revoked }) => {
                assert_eq!(grant_id, first.grant_id);
                assert_eq!(revoked, 4, "two access and two refresh tokens");
            }
            other => panic!("expected Reused, got {other:?}"),
        }
        assert!(p.lookup_access_token(&next.access_token).unwrap().is_none());
        assert_eq!(
            p.accept_refresh_token("c1", next.refresh_token.as_deref().unwrap())
                .unwrap_err(),
            RefreshDenied::Invalid
        );
    }

    #[test]
    fn refresh_token_of_another_client_is_refused_and_left_alone() {
        let store = std::sync::Arc::new(InMemoryOidcTokenStore::new());
        let p = test_provider(store.clone());
        let s = scopes("openid offline_access");
        let rt = p
            .issue_tokens(None, "u1", "c1", &s, Some(&s))
            .unwrap()
            .refresh_token
            .unwrap();
        assert_eq!(
            p.accept_refresh_token("c2", &rt).unwrap_err(),
            RefreshDenied::WrongClient
        );
        let row = store.find(&hash_token(&rt), now_secs()).unwrap().unwrap();
        assert_eq!(row.rotated_at, None, "a wrong client must not rotate it");
        assert!(p.accept_refresh_token("c1", &rt).is_ok());
    }

    #[test]
    fn an_access_token_is_not_a_refresh_token() {
        let p = test_provider(std::sync::Arc::new(InMemoryOidcTokenStore::new()));
        let s = scopes("openid offline_access");
        let issued = p.issue_tokens(None, "u1", "c1", &s, Some(&s)).unwrap();
        assert_eq!(
            p.accept_refresh_token("c1", &issued.access_token)
                .unwrap_err(),
            RefreshDenied::Invalid
        );
    }

    #[test]
    fn revoke_token_removes_the_grant_only_for_its_client() {
        let p = test_provider(std::sync::Arc::new(InMemoryOidcTokenStore::new()));
        let s = scopes("openid offline_access");
        let issued = p.issue_tokens(None, "u1", "c1", &s, Some(&s)).unwrap();
        let other = p.issue_tokens(None, "u1", "c1", &s, Some(&s)).unwrap();
        assert_eq!(p.revoke_token("c2", &issued.access_token).unwrap(), 0);
        assert_eq!(p.revoke_token("c1", "unknown").unwrap(), 0);
        assert_eq!(p.revoke_token("c1", &issued.access_token).unwrap(), 2);
        assert!(p
            .lookup_access_token(&issued.access_token)
            .unwrap()
            .is_none());
        assert!(p
            .lookup_access_token(&other.access_token)
            .unwrap()
            .is_some());
        assert_eq!(p.revoke_user("u1").unwrap(), 1);
        assert!(p
            .lookup_access_token(&other.access_token)
            .unwrap()
            .is_none());
    }

    #[test]
    fn discovery_advertises_refresh_and_revocation() {
        let doc = DiscoveryDoc::for_issuer("https://auth.example.com");
        assert!(doc.scopes_supported.iter().any(|s| s == "offline_access"));
        assert!(doc
            .grant_types_supported
            .iter()
            .any(|g| g == "refresh_token"));
        assert_eq!(
            doc.revocation_endpoint,
            "https://auth.example.com/oidc/revoke"
        );
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
            token_exchange: false,
        };
        assert!(c.is_registered_redirect("https://app.example.com/cb"));
        // Path-suffix variations rejected — open-redirect prevention.
        assert!(!c.is_registered_redirect("https://app.example.com/cb/extra"));
        assert!(!c.is_registered_redirect("https://app.example.com/cb?next=evil"));
        assert!(!c.is_registered_redirect("https://app.example.com.evil.com/cb"));
        assert!(!c.is_registered_redirect("http://app.example.com/cb"));
    }
}
