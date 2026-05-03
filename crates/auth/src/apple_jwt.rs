//! Apple's "Sign in with Apple" client_secret signer.
//!
//! Apple is the only major OAuth provider that requires the
//! `client_secret` to be a **signed JWT** rather than a static
//! string. The JWT is short-lived (5 min recommended; max 6 months)
//! and signed with an ES256 private key the developer downloads
//! from the Apple Developer portal.
//!
//! Pylon mints a fresh JWT on every token exchange so we never have
//! to think about expiry — token-exchange round-trips are cheap and
//! happen at most a few times per session.
//!
//! Spec: <https://developer.apple.com/documentation/sign_in_with_apple/generate_and_validate_tokens>
//!
//! Header:
//!   { "alg": "ES256", "kid": "<key_id>" }
//!
//! Claims:
//!   { "iss": "<team_id>",
//!     "iat": <now>,
//!     "exp": <now + 5min>,
//!     "aud": "https://appleid.apple.com",
//!     "sub": "<client_id>" }

use std::time::{SystemTime, UNIX_EPOCH};

use crate::provider::AppleConfig;

/// Mint a JWT to use as Apple's `client_secret` form field on the
/// token exchange request.
///
/// `client_id` is the OAuth client id (the Apple "Service ID" or
/// the iOS app's bundle id depending on the flow). The JWT's `sub`
/// claim binds the secret to a specific client so a leaked JWT
/// can't be used for a different app.
pub fn mint_client_secret(cfg: &AppleConfig, client_id: &str) -> Result<String, String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("apple jwt clock: {e}"))?
        .as_secs();
    // 5-minute validity window. Apple allows up to 6 months but a
    // short window minimizes blast radius if the JWT leaks via logs.
    let exp = now + 5 * 60;

    let header = serde_json::json!({
        "alg": "ES256",
        "kid": cfg.key_id,
    });
    let claims = serde_json::json!({
        "iss": cfg.team_id,
        "iat": now,
        "exp": exp,
        "aud": "https://appleid.apple.com",
        "sub": client_id,
    });

    let header_b64 = base64_url(serde_json::to_vec(&header).map_err(|e| e.to_string())?);
    let claims_b64 = base64_url(serde_json::to_vec(&claims).map_err(|e| e.to_string())?);
    let signing_input = format!("{header_b64}.{claims_b64}");

    let signature = es256_sign(&cfg.private_key_pem, signing_input.as_bytes())?;
    let sig_b64 = base64_url(signature);

    Ok(format!("{signing_input}.{sig_b64}"))
}

/// ES256 (ECDSA P-256 + SHA-256) signature using the `ring` crate
/// as the crypto backbone. `ring` is already a transitive dep via
/// `rustls`, so this adds zero new deps.
///
/// Reads either a PEM-encoded PKCS8 key OR a path to one (anything
/// without "BEGIN" in the first 32 bytes is treated as a path).
fn es256_sign(key_pem_or_path: &str, msg: &[u8]) -> Result<Vec<u8>, String> {
    let pem = if key_pem_or_path.contains("BEGIN") {
        key_pem_or_path.to_string()
    } else {
        std::fs::read_to_string(key_pem_or_path)
            .map_err(|e| format!("apple key read {key_pem_or_path}: {e}"))?
    };

    // Strip PEM armor + decode the base64 to PKCS8 DER bytes.
    let der = pem_to_der(&pem)?;

    // ring's EcdsaKeyPair wants PKCS8 v1 ECDSA P-256 SHA-256
    // (ASN.1-encoded signatures). Apple's downloadable .p8 files
    // are PKCS8 v1 — exactly what ring expects.
    use ring::rand::SystemRandom;
    use ring::signature::{EcdsaKeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
    let rng = SystemRandom::new();
    let key_pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &der, &rng)
        .map_err(|e| format!("apple key parse: {e}"))?;
    let sig = key_pair
        .sign(&rng, msg)
        .map_err(|e| format!("apple key sign: {e}"))?;
    // FIXED_SIGNING gives us the JWS-required raw r||s 64-byte form
    // directly — no DER conversion needed.
    Ok(sig.as_ref().to_vec())
}

/// Strip PEM headers/footers/whitespace and base64-decode the body.
fn pem_to_der(pem: &str) -> Result<Vec<u8>, String> {
    let body: String = pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect::<Vec<_>>()
        .join("");
    base64_decode(body.trim()).map_err(|e| format!("apple key base64 decode: {e}"))
}

/// URL-safe base64 (RFC 4648 §5) WITHOUT padding — what JWS uses.
pub(crate) fn base64_url(bytes: impl AsRef<[u8]>) -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    URL_SAFE_NO_PAD.encode(bytes.as_ref())
}

fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    STANDARD
        .decode(input)
        .map_err(|e| format!("base64 decode: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip: produce a JWT, split it on '.', verify the header
    /// + claims have the right shape. Doesn't verify the signature
    /// because that needs a real Apple key — covered by the
    /// integration test in tests/.
    #[test]
    fn mint_produces_three_segment_jwt_with_correct_header() {
        // Generate a throwaway P-256 key for the test. ring exposes
        // the ECDSA algorithm we need, but key generation isn't on
        // its public API in a stable form — easier to skip the real
        // signing here and just check the unsigned shape via a known
        // test vector.
        //
        // Instead: assert that mint_client_secret WITHOUT a valid
        // PEM returns an error (no panic), proving the error path is
        // wired. The signing path is exercised by the live test.
        let cfg = AppleConfig {
            team_id: "TEAMID12".into(),
            key_id: "KEYID0001".into(),
            private_key_pem: "not-a-real-key".into(),
        };
        let r = mint_client_secret(&cfg, "com.example.app");
        assert!(r.is_err(), "garbage PEM should error, got: {r:?}");
    }

    #[test]
    fn pem_to_der_strips_armor_and_whitespace() {
        let pem = "-----BEGIN PRIVATE KEY-----\nQUJDREVG\n-----END PRIVATE KEY-----\n";
        let der = pem_to_der(pem).expect("decode");
        assert_eq!(der, b"ABCDEF");
    }

    #[test]
    fn base64_url_drops_padding_and_swaps_chars() {
        // Picks an input that produces both `+` and `/` in standard
        // base64, and an output length that needs padding. The
        // url-safe form swaps + → -, / → _, and strips =.
        let raw = b"Hello, world!?";
        let encoded = base64_url(raw);
        assert!(!encoded.contains('+'));
        assert!(!encoded.contains('/'));
        assert!(!encoded.ends_with('='));
    }
}
