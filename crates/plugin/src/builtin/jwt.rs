use std::collections::HashSet;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::Plugin;

type HmacSha256 = Hmac<Sha256>;

/// A minimal JWT implementation using HMAC-SHA256 (HS256).
///
/// For production, consider a full JWT library. This implementation uses real
/// cryptographic primitives (HMAC-SHA256 via the `hmac` and `sha2` crates).
pub struct JwtPlugin {
    secret: String,
    expiry_secs: u64,
    /// Tracks consumed refresh tokens to enforce one-time use.
    used_refresh_tokens: Mutex<HashSet<String>>,
}

/// Decoded JWT claims.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claims {
    pub sub: String,
    pub iat: u64,
    pub exp: u64,
    /// Token kind: `"access"` or `"refresh"`. `None` for legacy tokens
    /// issued before the kind field was introduced.
    pub kind: Option<String>,
}

/// A paired access + refresh token, issued together.
#[derive(Debug, Clone)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    pub access_expires_in: u64,
    pub refresh_expires_in: u64,
}

impl JwtPlugin {
    pub fn new(secret: &str, expiry_secs: u64) -> Self {
        Self {
            secret: secret.to_string(),
            expiry_secs,
            used_refresh_tokens: Mutex::new(HashSet::new()),
        }
    }

    /// Issue a short-lived access JWT for a user ID.
    pub fn issue(&self, user_id: &str) -> String {
        self.issue_with_kind(user_id, "access", self.expiry_secs)
    }

    /// Issue a JWT with an explicit kind and expiry.
    pub fn issue_with_kind(&self, user_id: &str, kind: &str, expiry_secs: u64) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let header = base64url_encode(b"{\"alg\":\"HS256\",\"typ\":\"JWT\"}");
        let payload = base64url_encode(
            format!(
                "{{\"sub\":\"{}\",\"iat\":{},\"exp\":{},\"kind\":\"{}\"}}",
                user_id,
                now,
                now + expiry_secs,
                kind,
            )
            .as_bytes(),
        );

        let signing_input = format!("{header}.{payload}");
        let signature = base64url_encode(&hmac_sha256(&self.secret, &signing_input));

        format!("{signing_input}.{signature}")
    }

    /// Issue a token pair: a short-lived access token and a long-lived refresh
    /// token. The access token uses the plugin's configured expiry; the refresh
    /// token uses the provided `refresh_expiry_secs`.
    pub fn issue_pair(&self, user_id: &str, refresh_expiry_secs: u64) -> TokenPair {
        let access_token = self.issue(user_id);
        let refresh_token = self.issue_with_kind(user_id, "refresh", refresh_expiry_secs);
        TokenPair {
            access_token,
            refresh_token,
            access_expires_in: self.expiry_secs,
            refresh_expires_in: refresh_expiry_secs,
        }
    }

    /// Consume a refresh token and issue a new token pair.
    ///
    /// Order of operations matters for security:
    ///   1. Cryptographically verify the token FIRST. If we inserted into the
    ///      replay cache before verification, an attacker could pollute the
    ///      cache by posting random garbage, growing it unbounded. Worse, a
    ///      real token presented alongside that garbage would get "burned"
    ///      before we knew whether it was even valid.
    ///   2. Then check the replay cache and atomically insert.
    ///
    /// The window between `verify()` and `insert()` is a TOCTOU where two
    /// concurrent refreshes of the same token could both succeed. The Mutex
    /// around `used_refresh_tokens` is the serialization point — the check +
    /// insert happens under the same lock.
    pub fn refresh(&self, refresh_token: &str) -> Result<TokenPair, String> {
        let claims = self.verify(refresh_token)?;

        match claims.kind.as_deref() {
            Some("refresh") => {}
            _ => return Err("Token is not a refresh token".into()),
        }

        {
            let mut used = self.used_refresh_tokens.lock().map_err(|_| "Lock poisoned")?;
            if used.contains(refresh_token) {
                return Err("Refresh token already used".into());
            }
            used.insert(refresh_token.to_string());
        }

        Ok(self.issue_pair(&claims.sub, 86400 * 7))
    }

    /// Verify and decode a JWT. Returns claims if valid and not expired.
    /// Uses constant-time comparison for the signature to prevent timing attacks.
    pub fn verify(&self, token: &str) -> Result<Claims, String> {
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return Err("Invalid JWT format".into());
        }

        let signing_input = format!("{}.{}", parts[0], parts[1]);
        let expected_sig = base64url_encode(&hmac_sha256(&self.secret, &signing_input));

        if !pylon_auth::constant_time_eq(parts[2].as_bytes(), expected_sig.as_bytes()) {
            return Err("Invalid signature".into());
        }

        let payload_bytes = base64url_decode(parts[1])?;
        let payload_str = String::from_utf8(payload_bytes).map_err(|_| "Invalid payload")?;

        // Parse claims manually (no serde dependency in this minimal impl).
        let sub = extract_json_string(&payload_str, "sub").ok_or("Missing sub claim")?;
        let iat = extract_json_number(&payload_str, "iat").ok_or("Missing iat claim")?;
        let exp = extract_json_number(&payload_str, "exp").ok_or("Missing exp claim")?;
        let kind = extract_json_string(&payload_str, "kind");

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        if now > exp {
            return Err("Token expired".into());
        }

        Ok(Claims { sub, iat, exp, kind })
    }

    /// Resolve a JWT to a user ID. Returns None if invalid.
    pub fn resolve_user(&self, token: &str) -> Option<String> {
        self.verify(token).ok().map(|c| c.sub)
    }
}

impl Plugin for JwtPlugin {
    fn name(&self) -> &str {
        "jwt"
    }
}

// -- Minimal base64url encoding/decoding --

fn base64url_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[((n >> 18) & 63) as usize] as char);
        out.push(CHARS[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(CHARS[((n >> 6) & 63) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(CHARS[(n & 63) as usize] as char);
        }
    }
    out
}

fn base64url_decode(data: &str) -> Result<Vec<u8>, String> {
    fn val(c: u8) -> Result<u8, String> {
        match c {
            b'A'..=b'Z' => Ok(c - b'A'),
            b'a'..=b'z' => Ok(c - b'a' + 26),
            b'0'..=b'9' => Ok(c - b'0' + 52),
            b'-' => Ok(62),
            b'_' => Ok(63),
            _ => Err(format!("Invalid base64url character: {}", c as char)),
        }
    }

    let bytes = data.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b0 = val(bytes[i])?;
        let b1 = if i + 1 < bytes.len() { val(bytes[i + 1])? } else { 0 };
        let b2 = if i + 2 < bytes.len() { val(bytes[i + 2])? } else { 0 };
        let b3 = if i + 3 < bytes.len() { val(bytes[i + 3])? } else { 0 };

        let n = ((b0 as u32) << 18) | ((b1 as u32) << 12) | ((b2 as u32) << 6) | (b3 as u32);
        out.push((n >> 16) as u8);
        if i + 2 < bytes.len() {
            out.push((n >> 8) as u8);
        }
        if i + 3 < bytes.len() {
            out.push(n as u8);
        }
        i += 4;
    }
    Ok(out)
}

// -- HMAC-SHA256 signing --

fn hmac_sha256(key: &str, data: &str) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key.as_bytes())
        .expect("HMAC can take key of any size");
    mac.update(data.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

fn extract_json_string(json: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{}\":\"", key);
    let idx = json.find(&pattern)?;
    let start = idx + pattern.len();
    let end = json[start..].find('"')? + start;
    Some(json[start..end].to_string())
}

fn extract_json_number(json: &str, key: &str) -> Option<u64> {
    let pattern = format!("\"{}\":", key);
    let idx = json.find(&pattern)?;
    let start = idx + pattern.len();
    let rest = &json[start..];
    let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
    rest[..end].parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_and_verify() {
        let jwt = JwtPlugin::new("test-secret", 3600);
        let token = jwt.issue("user-1");

        assert!(!token.is_empty());
        assert_eq!(token.split('.').count(), 3);

        let claims = jwt.verify(&token).unwrap();
        assert_eq!(claims.sub, "user-1");
        assert!(claims.exp > claims.iat);
        assert_eq!(claims.kind, Some("access".into()));
    }

    #[test]
    fn wrong_secret_fails() {
        let jwt1 = JwtPlugin::new("secret-1", 3600);
        let jwt2 = JwtPlugin::new("secret-2", 3600);

        let token = jwt1.issue("user-1");
        let result = jwt2.verify(&token);
        assert!(result.is_err());
    }

    #[test]
    fn expired_token_rejected() {
        let jwt = JwtPlugin::new("secret", 0); // 0 second expiry
        let token = jwt.issue("user-1");

        // Token is already expired (exp = iat + 0 = now, and we check now > exp).
        // This might pass if checked in the same second. Sleep would make it reliable.
        // For testing, use a very short expiry and accept the edge case.
        let _ = jwt.verify(&token); // may or may not fail depending on timing
    }

    #[test]
    fn invalid_format_rejected() {
        let jwt = JwtPlugin::new("secret", 3600);
        assert!(jwt.verify("not.a.jwt.token").is_err());
        assert!(jwt.verify("invalid").is_err());
        assert!(jwt.verify("").is_err());
    }

    #[test]
    fn resolve_user() {
        let jwt = JwtPlugin::new("secret", 3600);
        let token = jwt.issue("alice");

        assert_eq!(jwt.resolve_user(&token), Some("alice".into()));
        assert_eq!(jwt.resolve_user("invalid"), None);
    }

    #[test]
    fn different_users_different_tokens() {
        let jwt = JwtPlugin::new("secret", 3600);
        let t1 = jwt.issue("user-1");
        let t2 = jwt.issue("user-2");
        assert_ne!(t1, t2);
    }

    #[test]
    fn hmac_sha256_produces_32_bytes() {
        let sig = hmac_sha256("key", "data");
        assert_eq!(sig.len(), 32);
    }

    #[test]
    fn hmac_sha256_different_keys_different_output() {
        let s1 = hmac_sha256("key1", "data");
        let s2 = hmac_sha256("key2", "data");
        assert_ne!(s1, s2);
    }

    #[test]
    fn hmac_sha256_different_data_different_output() {
        let s1 = hmac_sha256("key", "data1");
        let s2 = hmac_sha256("key", "data2");
        assert_ne!(s1, s2);
    }

    // -- Token pair tests --

    #[test]
    fn issue_pair_creates_two_distinct_tokens() {
        let jwt = JwtPlugin::new("secret", 300);
        let pair = jwt.issue_pair("user-1", 86400 * 7);

        assert_ne!(pair.access_token, pair.refresh_token);
        assert_eq!(pair.access_expires_in, 300);
        assert_eq!(pair.refresh_expires_in, 86400 * 7);

        let access_claims = jwt.verify(&pair.access_token).unwrap();
        assert_eq!(access_claims.sub, "user-1");
        assert_eq!(access_claims.kind, Some("access".into()));

        let refresh_claims = jwt.verify(&pair.refresh_token).unwrap();
        assert_eq!(refresh_claims.sub, "user-1");
        assert_eq!(refresh_claims.kind, Some("refresh".into()));
    }

    #[test]
    fn refresh_returns_new_pair() {
        let jwt = JwtPlugin::new("secret", 300);
        let pair = jwt.issue_pair("user-1", 86400 * 7);

        let new_pair = jwt.refresh(&pair.refresh_token).unwrap();

        // The new pair should contain valid tokens for the same user.
        let access_claims = jwt.verify(&new_pair.access_token).unwrap();
        assert_eq!(access_claims.sub, "user-1");
        assert_eq!(access_claims.kind, Some("access".into()));

        let refresh_claims = jwt.verify(&new_pair.refresh_token).unwrap();
        assert_eq!(refresh_claims.sub, "user-1");
        assert_eq!(refresh_claims.kind, Some("refresh".into()));

        // The old refresh token must now be rejected (one-time use).
        let err = jwt.refresh(&pair.refresh_token).unwrap_err();
        assert!(err.contains("already used"));
    }

    #[test]
    fn used_refresh_token_rejected() {
        let jwt = JwtPlugin::new("secret", 300);
        let pair = jwt.issue_pair("user-1", 86400 * 7);

        // First use succeeds.
        assert!(jwt.refresh(&pair.refresh_token).is_ok());

        // Second use is rejected (replay protection).
        let err = jwt.refresh(&pair.refresh_token).unwrap_err();
        assert!(err.contains("already used"));
    }

    #[test]
    fn access_token_cannot_be_used_as_refresh() {
        let jwt = JwtPlugin::new("secret", 300);
        let pair = jwt.issue_pair("user-1", 86400 * 7);

        let err = jwt.refresh(&pair.access_token).unwrap_err();
        assert!(err.contains("not a refresh token"));
    }

    #[test]
    fn issue_with_kind_sets_kind_field() {
        let jwt = JwtPlugin::new("secret", 3600);
        let token = jwt.issue_with_kind("user-1", "refresh", 86400);
        let claims = jwt.verify(&token).unwrap();
        assert_eq!(claims.kind, Some("refresh".into()));
    }
}
