//! Verification of client-supplied OpenID id_tokens (native sign-in).
//!
//! A native app (Sign in with Apple on iOS, Google Sign-In on Android or
//! iOS) receives an id_token from the platform SDK and posts it to the
//! server. Unlike the browser OAuth flow, that token never came through a
//! back-channel exchange, so the server must verify it itself:
//!
//! 1. Parse the compact JWS and read `kid` + `alg` from the header.
//! 2. Fetch the provider's JWKS (cached), find the key by `kid`.
//! 3. Verify the RS256 signature over `<header>.<payload>`.
//! 4. Check `iss`, `aud` (against the app's configured client ids /
//!    bundle ids), and `exp` with a small clock skew.
//! 5. Return the identity claims (`sub`, `email`, `name`).
//!
//! The verification core is a pure function over a JWKS document so it is
//! unit-tested with a locally generated key; only [`fetch_jwks`] talks to
//! the network.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rsa::{BigUint, Pkcs1v15Sign, RsaPublicKey};
use sha2::{Digest, Sha256};

/// Which platform issued the token. Each has a fixed issuer + JWKS URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeProvider {
    Apple,
    Google,
}

impl NativeProvider {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "apple" => Some(Self::Apple),
            "google" => Some(Self::Google),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Apple => "apple",
            Self::Google => "google",
        }
    }

    pub fn jwks_url(self) -> &'static str {
        match self {
            Self::Apple => "https://appleid.apple.com/auth/keys",
            Self::Google => "https://www.googleapis.com/oauth2/v3/certs",
        }
    }

    /// Accepted `iss` values.
    pub fn issuers(self) -> &'static [&'static str] {
        match self {
            Self::Apple => &["https://appleid.apple.com"],
            Self::Google => &["https://accounts.google.com", "accounts.google.com"],
        }
    }

    /// The env vars that list the audiences (client ids / bundle ids)
    /// this server accepts, in priority order. The first set one wins.
    /// A comma-separated list is accepted so one server can serve an
    /// iOS bundle id and an Android client id at once.
    pub fn audience_env_vars(self) -> &'static [&'static str] {
        match self {
            Self::Apple => &[
                "PYLON_APPLE_NATIVE_CLIENT_IDS",
                "PYLON_OAUTH_APPLE_CLIENT_ID",
            ],
            Self::Google => &[
                "PYLON_GOOGLE_NATIVE_CLIENT_IDS",
                "PYLON_OAUTH_GOOGLE_CLIENT_ID",
            ],
        }
    }

    /// Audiences from the environment. Empty when nothing is configured,
    /// which the route turns into a 404 so an unconfigured server never
    /// accepts a token for an arbitrary app.
    pub fn configured_audiences(self) -> Vec<String> {
        for var in self.audience_env_vars() {
            if let Ok(raw) = std::env::var(var) {
                let list: Vec<String> = raw
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                if !list.is_empty() {
                    return list;
                }
            }
        }
        Vec::new()
    }
}

/// Identity claims extracted from a verified id_token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeIdentity {
    pub provider: NativeProvider,
    /// The provider's stable subject id.
    pub sub: String,
    pub email: String,
    pub email_verified: bool,
    /// Google carries `name`; Apple never puts a name in the token.
    pub name: Option<String>,
    pub picture: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeTokenError {
    Malformed(String),
    UnsupportedAlgorithm(String),
    UnknownKey(String),
    BadSignature,
    WrongIssuer(String),
    WrongAudience(String),
    Expired,
    MissingEmail,
    Jwks(String),
}

impl std::fmt::Display for NativeTokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(m) => write!(f, "malformed id_token: {m}"),
            Self::UnsupportedAlgorithm(a) => write!(f, "unsupported alg {a}; expected RS256"),
            Self::UnknownKey(k) => write!(f, "no JWKS key matches kid {k}"),
            Self::BadSignature => write!(f, "id_token signature does not verify"),
            Self::WrongIssuer(i) => write!(f, "id_token issuer {i} is not this provider"),
            Self::WrongAudience(a) => write!(f, "id_token audience {a} is not this app"),
            Self::Expired => write!(f, "id_token has expired"),
            Self::MissingEmail => write!(f, "id_token carries no email"),
            Self::Jwks(m) => write!(f, "could not load provider keys: {m}"),
        }
    }
}

impl std::error::Error for NativeTokenError {}

fn b64url_decode(s: &str) -> Result<Vec<u8>, NativeTokenError> {
    URL_SAFE_NO_PAD
        .decode(s)
        .map_err(|e| NativeTokenError::Malformed(format!("base64url: {e}")))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Verify `id_token` against `jwks_json` (the provider's JWKS document)
/// and the expected issuer/audience set. `now` is injectable for tests.
pub fn verify_id_token_with_jwks(
    provider: NativeProvider,
    id_token: &str,
    jwks_json: &str,
    allowed_audiences: &[String],
    now: u64,
) -> Result<NativeIdentity, NativeTokenError> {
    let mut parts = id_token.split('.');
    let (header_b64, payload_b64, sig_b64) = match (parts.next(), parts.next(), parts.next()) {
        (Some(h), Some(p), Some(s)) if parts.next().is_none() => (h, p, s),
        _ => return Err(NativeTokenError::Malformed("expected 3 segments".into())),
    };
    let header: serde_json::Value = serde_json::from_slice(&b64url_decode(header_b64)?)
        .map_err(|e| NativeTokenError::Malformed(format!("header: {e}")))?;
    let alg = header.get("alg").and_then(|v| v.as_str()).unwrap_or("");
    if alg != "RS256" {
        return Err(NativeTokenError::UnsupportedAlgorithm(alg.to_string()));
    }
    let kid = header
        .get("kid")
        .and_then(|v| v.as_str())
        .ok_or_else(|| NativeTokenError::Malformed("header has no kid".into()))?;

    let jwks: serde_json::Value = serde_json::from_str(jwks_json)
        .map_err(|e| NativeTokenError::Jwks(format!("JWKS not JSON: {e}")))?;
    let key = jwks
        .get("keys")
        .and_then(|k| k.as_array())
        .and_then(|keys| {
            keys.iter().find(|k| {
                k.get("kid").and_then(|v| v.as_str()) == Some(kid)
                    && k.get("kty").and_then(|v| v.as_str()) == Some("RSA")
            })
        })
        .ok_or_else(|| NativeTokenError::UnknownKey(kid.to_string()))?;
    let n = b64url_decode(key.get("n").and_then(|v| v.as_str()).unwrap_or(""))?;
    let e = b64url_decode(key.get("e").and_then(|v| v.as_str()).unwrap_or(""))?;
    let public_key = RsaPublicKey::new(BigUint::from_bytes_be(&n), BigUint::from_bytes_be(&e))
        .map_err(|e| NativeTokenError::Jwks(format!("bad RSA key: {e}")))?;

    let signing_input = format!("{header_b64}.{payload_b64}");
    let signature = b64url_decode(sig_b64)?;
    let digest = Sha256::digest(signing_input.as_bytes());
    public_key
        .verify(Pkcs1v15Sign::new::<Sha256>(), &digest, &signature)
        .map_err(|_| NativeTokenError::BadSignature)?;

    let claims: serde_json::Value = serde_json::from_slice(&b64url_decode(payload_b64)?)
        .map_err(|e| NativeTokenError::Malformed(format!("claims: {e}")))?;

    let iss = claims.get("iss").and_then(|v| v.as_str()).unwrap_or("");
    if !provider.issuers().contains(&iss) {
        return Err(NativeTokenError::WrongIssuer(iss.to_string()));
    }
    // `aud` is a string or an array of strings.
    let auds: Vec<String> = match claims.get("aud") {
        Some(serde_json::Value::String(s)) => vec![s.clone()],
        Some(serde_json::Value::Array(a)) => a
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        _ => Vec::new(),
    };
    if !auds
        .iter()
        .any(|a| allowed_audiences.iter().any(|ok| ok == a))
    {
        return Err(NativeTokenError::WrongAudience(auds.join(",")));
    }
    // 60s of skew for a device clock that is slightly behind.
    let exp = claims.get("exp").and_then(|v| v.as_u64()).unwrap_or(0);
    if exp + 60 < now {
        return Err(NativeTokenError::Expired);
    }

    let sub = claims
        .get("sub")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| NativeTokenError::Malformed("claims have no sub".into()))?
        .to_string();
    let email = claims
        .get("email")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or(NativeTokenError::MissingEmail)?
        .to_string();
    // Apple encodes email_verified as the string "true" on some tokens.
    let email_verified = match claims.get("email_verified") {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::String(s)) => s == "true",
        _ => false,
    };
    Ok(NativeIdentity {
        provider,
        sub,
        email,
        email_verified,
        name: claims
            .get("name")
            .and_then(|v| v.as_str())
            .map(String::from),
        picture: claims
            .get("picture")
            .and_then(|v| v.as_str())
            .map(String::from),
    })
}

/// JWKS documents by URL with a fetch time. Keys rotate rarely; an hour of
/// caching removes the network round-trip from every sign-in while a new
/// `kid` (a rotation) forces a refresh through [`verify_id_token`].
static JWKS_CACHE: Mutex<Option<HashMap<String, (Instant, String)>>> = Mutex::new(None);
const JWKS_TTL: Duration = Duration::from_secs(3600);

fn cached_jwks(url: &str) -> Option<String> {
    let guard = JWKS_CACHE.lock().unwrap_or_else(|p| p.into_inner());
    guard
        .as_ref()
        .and_then(|m| m.get(url))
        .filter(|(at, _)| at.elapsed() < JWKS_TTL)
        .map(|(_, body)| body.clone())
}

/// Seed the JWKS cache for `url`. Public so an integration test can verify a
/// locally signed token through the HTTP route without a network fetch.
pub fn seed_jwks_cache(url: &str, body: &str) {
    let mut guard = JWKS_CACHE.lock().unwrap_or_else(|p| p.into_inner());
    guard
        .get_or_insert_with(HashMap::new)
        .insert(url.to_string(), (Instant::now(), body.to_string()));
}

/// Fetch a JWKS document over HTTPS.
pub fn fetch_jwks(url: &str) -> Result<String, NativeTokenError> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(10))
        .build();
    match agent.get(url).set("Accept", "application/json").call() {
        Ok(resp) => resp
            .into_string()
            .map_err(|e| NativeTokenError::Jwks(format!("read body: {e}"))),
        Err(ureq::Error::Status(code, _)) => Err(NativeTokenError::Jwks(format!("HTTP {code}"))),
        Err(e) => Err(NativeTokenError::Jwks(format!("{e}"))),
    }
}

/// Verify a native id_token against the provider's live JWKS. On an
/// unknown `kid` with a cached document, refetch once — the provider may
/// have rotated keys since the cache filled.
pub fn verify_id_token(
    provider: NativeProvider,
    id_token: &str,
    allowed_audiences: &[String],
) -> Result<NativeIdentity, NativeTokenError> {
    let url = provider.jwks_url();
    let now = now_secs();
    if let Some(cached) = cached_jwks(url) {
        match verify_id_token_with_jwks(provider, id_token, &cached, allowed_audiences, now) {
            Err(NativeTokenError::UnknownKey(_)) => {}
            other => return other,
        }
    }
    let fresh = fetch_jwks(url)?;
    seed_jwks_cache(url, &fresh);
    verify_id_token_with_jwks(provider, id_token, &fresh, allowed_audiences, now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::traits::PublicKeyParts;
    use rsa::RsaPrivateKey;

    fn b64url(bytes: &[u8]) -> String {
        URL_SAFE_NO_PAD.encode(bytes)
    }

    struct Signer {
        key: RsaPrivateKey,
        kid: String,
    }

    impl Signer {
        fn new(kid: &str) -> Self {
            let key = RsaPrivateKey::new(&mut rand::thread_rng(), 2048).expect("keygen");
            Self {
                key,
                kid: kid.into(),
            }
        }

        fn jwks(&self) -> String {
            let public = self.key.to_public_key();
            serde_json::json!({
                "keys": [{
                    "kty": "RSA",
                    "kid": self.kid,
                    "alg": "RS256",
                    "use": "sig",
                    "n": b64url(&public.n().to_bytes_be()),
                    "e": b64url(&public.e().to_bytes_be()),
                }]
            })
            .to_string()
        }

        fn token(&self, claims: serde_json::Value) -> String {
            self.token_with_header(serde_json::json!({"alg": "RS256", "kid": self.kid}), claims)
        }

        fn token_with_header(
            &self,
            header: serde_json::Value,
            claims: serde_json::Value,
        ) -> String {
            let h = b64url(header.to_string().as_bytes());
            let p = b64url(claims.to_string().as_bytes());
            let input = format!("{h}.{p}");
            let digest = Sha256::digest(input.as_bytes());
            let sig = self
                .key
                .sign(Pkcs1v15Sign::new::<Sha256>(), &digest)
                .expect("sign");
            format!("{input}.{}", b64url(&sig))
        }
    }

    const NOW: u64 = 1_800_000_000;

    fn apple_claims() -> serde_json::Value {
        serde_json::json!({
            "iss": "https://appleid.apple.com",
            "aud": "com.example.app",
            "exp": NOW + 600,
            "iat": NOW - 10,
            "sub": "001234.abcdef",
            "email": "jane@privaterelay.appleid.com",
            "email_verified": "true",
        })
    }

    #[test]
    fn a_valid_apple_token_yields_the_identity() {
        let signer = Signer::new("k1");
        let token = signer.token(apple_claims());
        let id = verify_id_token_with_jwks(
            NativeProvider::Apple,
            &token,
            &signer.jwks(),
            &["com.example.app".to_string()],
            NOW,
        )
        .expect("verifies");
        assert_eq!(id.sub, "001234.abcdef");
        assert_eq!(id.email, "jane@privaterelay.appleid.com");
        assert!(
            id.email_verified,
            "Apple's string \"true\" counts as verified"
        );
        assert_eq!(id.name, None);
    }

    #[test]
    fn a_google_token_with_array_audience_and_name_verifies() {
        let signer = Signer::new("g1");
        let token = signer.token(serde_json::json!({
            "iss": "accounts.google.com",
            "aud": ["ios-client.apps.googleusercontent.com"],
            "exp": NOW + 600,
            "sub": "1234567890",
            "email": "jane@gmail.com",
            "email_verified": true,
            "name": "Jane Doe",
            "picture": "https://lh3.example/p.jpg",
        }));
        let id = verify_id_token_with_jwks(
            NativeProvider::Google,
            &token,
            &signer.jwks(),
            &[
                "android-client.apps.googleusercontent.com".to_string(),
                "ios-client.apps.googleusercontent.com".to_string(),
            ],
            NOW,
        )
        .expect("verifies");
        assert_eq!(id.name.as_deref(), Some("Jane Doe"));
        assert!(id.picture.is_some());
    }

    #[test]
    fn a_token_signed_by_another_key_is_rejected() {
        let real = Signer::new("k1");
        let mut forger = Signer::new("k1"); // same kid, different key
        forger.kid = "k1".into();
        let token = forger.token(apple_claims());
        let err = verify_id_token_with_jwks(
            NativeProvider::Apple,
            &token,
            &real.jwks(),
            &["com.example.app".to_string()],
            NOW,
        )
        .unwrap_err();
        assert_eq!(err, NativeTokenError::BadSignature);
    }

    #[test]
    fn a_tampered_payload_is_rejected() {
        let signer = Signer::new("k1");
        let token = signer.token(apple_claims());
        let mut parts: Vec<&str> = token.split('.').collect();
        let mut claims = apple_claims();
        claims["sub"] = serde_json::json!("someone-else");
        let forged = b64url(claims.to_string().as_bytes());
        parts[1] = &forged;
        let tampered = parts.join(".");
        let err = verify_id_token_with_jwks(
            NativeProvider::Apple,
            &tampered,
            &signer.jwks(),
            &["com.example.app".to_string()],
            NOW,
        )
        .unwrap_err();
        assert_eq!(err, NativeTokenError::BadSignature);
    }

    #[test]
    fn wrong_audience_issuer_expiry_and_alg_are_rejected() {
        let signer = Signer::new("k1");
        let ok_aud = ["com.example.app".to_string()];

        let token = signer.token(apple_claims());
        let err = verify_id_token_with_jwks(
            NativeProvider::Apple,
            &token,
            &signer.jwks(),
            &["com.other.app".to_string()],
            NOW,
        )
        .unwrap_err();
        assert!(matches!(err, NativeTokenError::WrongAudience(_)));

        let mut c = apple_claims();
        c["iss"] = serde_json::json!("https://accounts.google.com");
        let err = verify_id_token_with_jwks(
            NativeProvider::Apple,
            &signer.token(c),
            &signer.jwks(),
            &ok_aud,
            NOW,
        )
        .unwrap_err();
        assert!(matches!(err, NativeTokenError::WrongIssuer(_)));

        let mut c = apple_claims();
        c["exp"] = serde_json::json!(NOW - 120);
        let err = verify_id_token_with_jwks(
            NativeProvider::Apple,
            &signer.token(c),
            &signer.jwks(),
            &ok_aud,
            NOW,
        )
        .unwrap_err();
        assert_eq!(err, NativeTokenError::Expired);

        // Within the 60s skew still passes.
        let mut c = apple_claims();
        c["exp"] = serde_json::json!(NOW - 30);
        assert!(verify_id_token_with_jwks(
            NativeProvider::Apple,
            &signer.token(c),
            &signer.jwks(),
            &ok_aud,
            NOW,
        )
        .is_ok());

        // alg=none must never verify, even with a valid-looking body.
        let none = signer.token_with_header(
            serde_json::json!({"alg": "none", "kid": "k1"}),
            apple_claims(),
        );
        let err =
            verify_id_token_with_jwks(NativeProvider::Apple, &none, &signer.jwks(), &ok_aud, NOW)
                .unwrap_err();
        assert!(matches!(err, NativeTokenError::UnsupportedAlgorithm(_)));

        // HS256 with the JWKS bytes as the secret (the classic confusion
        // attack) is rejected at the alg check before any key use.
        let hs = signer.token_with_header(
            serde_json::json!({"alg": "HS256", "kid": "k1"}),
            apple_claims(),
        );
        assert!(matches!(
            verify_id_token_with_jwks(NativeProvider::Apple, &hs, &signer.jwks(), &ok_aud, NOW)
                .unwrap_err(),
            NativeTokenError::UnsupportedAlgorithm(_)
        ));
    }

    #[test]
    fn an_unknown_kid_is_reported_so_the_caller_refetches() {
        let signer = Signer::new("rotated");
        let stale = Signer::new("old").jwks();
        let err = verify_id_token_with_jwks(
            NativeProvider::Apple,
            &signer.token(apple_claims()),
            &stale,
            &["com.example.app".to_string()],
            NOW,
        )
        .unwrap_err();
        assert_eq!(err, NativeTokenError::UnknownKey("rotated".into()));
    }

    #[test]
    fn configured_audiences_split_a_comma_list() {
        // Serial: env is process-global.
        std::env::set_var(
            "PYLON_APPLE_NATIVE_CLIENT_IDS",
            " com.a.app, com.a.app.dev ,",
        );
        assert_eq!(
            NativeProvider::Apple.configured_audiences(),
            vec!["com.a.app".to_string(), "com.a.app.dev".to_string()]
        );
        std::env::remove_var("PYLON_APPLE_NATIVE_CLIENT_IDS");
    }
}
