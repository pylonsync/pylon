//! Stateless JWT sessions — alternative to opaque session tokens.
//!
//! By default Pylon mints opaque random `pylon_…` tokens that must
//! be looked up in the session store on every request. For deploys
//! that can't tolerate that round-trip (edge runtimes, CDN-backed
//! routes, multi-region read replicas), Pylon can mint **JWT-shaped**
//! sessions instead — verified by the local secret with no DB hit.
//!
//! Trade-offs:
//!   - **Pro**: stateless verification (no DB read on every request)
//!   - **Pro**: clients can decode their own claims (without verifying)
//!     for UI personalization without a `/me` round-trip
//!   - **Con**: revocation requires either a denylist or a short TTL —
//!     a leaked JWT stays valid until its `exp`
//!   - **Con**: secret rotation needs both old + new keys to coexist
//!     for at least one session lifetime
//!
//! Pylon uses HS256 (HMAC-SHA256) — symmetric, no key distribution.
//! Apps that need RS256 / asymmetric verification across services
//! should use the OIDC discovery / JWKS path on Wave 5.
//!
//! Spec: <https://www.rfc-editor.org/rfc/rfc7519> + RFC 7515 (JWS).

use crate::apple_jwt::base64_url;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};

type HmacSha256 = Hmac<Sha256>;

/// Standard claims pylon mints. Apps that want extra claims can
/// extend via the Custom variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JwtClaims {
    /// Subject — the user_id.
    pub sub: String,
    /// Issued at (Unix seconds).
    pub iat: u64,
    /// Expiry (Unix seconds). Pylon defaults to 30d for parity with
    /// opaque sessions; apps can override.
    pub exp: u64,
    /// Issuer — `PYLON_JWT_ISSUER` if set, else `pylon`.
    pub iss: String,
    /// Optional tenant id (Pylon-specific extension claim
    /// `https://pylonsync.com/tenant`).
    pub tenant_id: Option<String>,
    /// Optional roles array.
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JwtError {
    /// Token doesn't have three `.`-separated segments.
    Malformed,
    /// Header / claims base64 decode failed.
    BadEncoding,
    /// Header alg isn't `HS256` (we only mint that).
    UnsupportedAlg,
    /// Signature didn't match the secret.
    BadSignature,
    /// `exp` is in the past.
    Expired,
    /// `iss` doesn't match expected issuer.
    WrongIssuer,
}

impl std::fmt::Display for JwtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Malformed => "JWT malformed",
            Self::BadEncoding => "JWT base64/JSON decode failed",
            Self::UnsupportedAlg => "JWT alg not supported (expected HS256)",
            Self::BadSignature => "JWT signature mismatch",
            Self::Expired => "JWT expired",
            Self::WrongIssuer => "JWT issuer mismatch",
        })
    }
}

/// Mint a JWT-shaped session token. The output is the
/// `header.claims.sig` triplet, ready to be returned in
/// `Authorization: Bearer …` form. Client doesn't need to know the
/// difference from an opaque session token.
pub fn mint(secret: &[u8], claims: &JwtClaims) -> String {
    let header = serde_json::json!({"alg": "HS256", "typ": "JWT"});
    let mut claims_obj = serde_json::Map::new();
    claims_obj.insert("sub".into(), claims.sub.clone().into());
    claims_obj.insert("iat".into(), claims.iat.into());
    claims_obj.insert("exp".into(), claims.exp.into());
    claims_obj.insert("iss".into(), claims.iss.clone().into());
    if let Some(t) = &claims.tenant_id {
        claims_obj.insert("https://pylonsync.com/tenant".into(), t.clone().into());
    }
    if !claims.roles.is_empty() {
        claims_obj.insert(
            "https://pylonsync.com/roles".into(),
            serde_json::Value::Array(
                claims.roles.iter().cloned().map(Into::into).collect(),
            ),
        );
    }
    let header_b64 = base64_url(serde_json::to_vec(&header).unwrap());
    let claims_b64 = base64_url(serde_json::to_vec(&claims_obj).unwrap());
    let signing_input = format!("{header_b64}.{claims_b64}");
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(signing_input.as_bytes());
    let sig = mac.finalize().into_bytes();
    let sig_b64 = base64_url(sig);
    format!("{signing_input}.{sig_b64}")
}

/// Verify + decode a JWT. Checks signature, alg, expiry, and issuer
/// (when supplied). Returns the parsed claims or a structured error.
pub fn verify(token: &str, secret: &[u8], expected_issuer: Option<&str>) -> Result<JwtClaims, JwtError> {
    let mut parts = token.split('.');
    let header_b64 = parts.next().ok_or(JwtError::Malformed)?;
    let claims_b64 = parts.next().ok_or(JwtError::Malformed)?;
    let sig_b64 = parts.next().ok_or(JwtError::Malformed)?;
    if parts.next().is_some() {
        return Err(JwtError::Malformed);
    }

    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    let header_bytes = URL_SAFE_NO_PAD
        .decode(header_b64)
        .map_err(|_| JwtError::BadEncoding)?;
    let header: serde_json::Value =
        serde_json::from_slice(&header_bytes).map_err(|_| JwtError::BadEncoding)?;
    if header.get("alg").and_then(|v| v.as_str()) != Some("HS256") {
        return Err(JwtError::UnsupportedAlg);
    }

    let signing_input = format!("{header_b64}.{claims_b64}");
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(signing_input.as_bytes());
    let expected_sig = mac.finalize().into_bytes();
    let provided_sig = URL_SAFE_NO_PAD
        .decode(sig_b64)
        .map_err(|_| JwtError::BadEncoding)?;
    if !crate::constant_time_eq(&expected_sig, &provided_sig) {
        return Err(JwtError::BadSignature);
    }

    let claims_bytes = URL_SAFE_NO_PAD
        .decode(claims_b64)
        .map_err(|_| JwtError::BadEncoding)?;
    let claims: serde_json::Value =
        serde_json::from_slice(&claims_bytes).map_err(|_| JwtError::BadEncoding)?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let exp = claims.get("exp").and_then(|v| v.as_u64()).unwrap_or(0);
    if exp <= now {
        return Err(JwtError::Expired);
    }
    let iss = claims
        .get("iss")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    if let Some(want) = expected_issuer {
        if iss != want {
            return Err(JwtError::WrongIssuer);
        }
    }

    let sub = claims
        .get("sub")
        .and_then(|v| v.as_str())
        .ok_or(JwtError::BadEncoding)?
        .to_string();
    let iat = claims.get("iat").and_then(|v| v.as_u64()).unwrap_or(0);
    let tenant_id = claims
        .get("https://pylonsync.com/tenant")
        .and_then(|v| v.as_str())
        .map(String::from);
    let roles = claims
        .get("https://pylonsync.com/roles")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    Ok(JwtClaims {
        sub,
        iat,
        exp,
        iss,
        tenant_id,
        roles,
    })
}

/// Convenience: detect whether a bearer token looks like a JWT
/// (three `.`-separated base64url segments) so the dispatcher can
/// route between session store and JWT verifier without trying both.
pub fn looks_like_jwt(token: &str) -> bool {
    let mut parts = token.split('.');
    let a = parts.next();
    let b = parts.next();
    let c = parts.next();
    let extra = parts.next();
    matches!((a, b, c, extra), (Some(a), Some(b), Some(c), None) if !a.is_empty() && !b.is_empty() && !c.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_claims(exp_secs_from_now: i64) -> JwtClaims {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        JwtClaims {
            sub: "user-1".into(),
            iat: now,
            exp: (now as i64 + exp_secs_from_now) as u64,
            iss: "pylon-test".into(),
            tenant_id: None,
            roles: vec![],
        }
    }

    #[test]
    fn round_trip_minimal_claims() {
        let secret = b"super-secret-pylon-key";
        let claims = fixture_claims(3600);
        let token = mint(secret, &claims);
        let decoded = verify(&token, secret, Some("pylon-test")).unwrap();
        assert_eq!(decoded.sub, "user-1");
        assert_eq!(decoded.iss, "pylon-test");
    }

    #[test]
    fn round_trip_with_tenant_and_roles() {
        let secret = b"k";
        let mut claims = fixture_claims(3600);
        claims.tenant_id = Some("acme".into());
        claims.roles = vec!["admin".into(), "billing".into()];
        let token = mint(secret, &claims);
        let decoded = verify(&token, secret, None).unwrap();
        assert_eq!(decoded.tenant_id.as_deref(), Some("acme"));
        assert_eq!(decoded.roles, vec!["admin", "billing"]);
    }

    #[test]
    fn expired_token_rejected() {
        let secret = b"k";
        let claims = fixture_claims(-60); // expired 1 min ago
        let token = mint(secret, &claims);
        assert_eq!(verify(&token, secret, None), Err(JwtError::Expired));
    }

    #[test]
    fn wrong_secret_rejected() {
        let secret = b"k";
        let claims = fixture_claims(3600);
        let token = mint(secret, &claims);
        assert_eq!(
            verify(&token, b"different-secret", None),
            Err(JwtError::BadSignature)
        );
    }

    #[test]
    fn wrong_issuer_rejected() {
        let secret = b"k";
        let claims = fixture_claims(3600);
        let token = mint(secret, &claims);
        assert_eq!(
            verify(&token, secret, Some("different-issuer")),
            Err(JwtError::WrongIssuer)
        );
    }

    #[test]
    fn alg_none_rejected() {
        // Critical security check — RFC 7519 famously had the "alg:none"
        // bypass class. Hand-craft a token with `alg: none` and assert
        // verify rejects it.
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none","typ":"JWT"}"#);
        let claims = URL_SAFE_NO_PAD.encode(br#"{"sub":"attacker","exp":99999999999}"#);
        let token = format!("{header}.{claims}.");
        let result = verify(&token, b"any-secret", None);
        assert_eq!(result, Err(JwtError::UnsupportedAlg));
    }

    #[test]
    fn malformed_token_rejected() {
        assert_eq!(verify("not.a.jwt.too-many-parts", b"k", None), Err(JwtError::Malformed));
        assert_eq!(verify("only-one-part", b"k", None), Err(JwtError::Malformed));
        assert_eq!(verify("", b"k", None), Err(JwtError::Malformed));
    }

    #[test]
    fn looks_like_jwt_classifies() {
        assert!(looks_like_jwt("aaa.bbb.ccc"));
        assert!(!looks_like_jwt("pylon_abcdef"));
        assert!(!looks_like_jwt("aaa.bbb"));
        assert!(!looks_like_jwt(""));
        assert!(!looks_like_jwt("aaa..ccc"));
        // NOTE: `pk.key_abc.secret` has three nonempty segments and
        // would superficially look like a JWT — that's why the
        // dispatcher in server.rs MUST check the `pk.` prefix
        // BEFORE looks_like_jwt. Documented for whoever changes that
        // dispatcher next.
        assert!(looks_like_jwt("pk.key_abc.secret"));
    }
}
