//! Signed relay auth blob — the machine-minted credential a sync-relay
//! subscriber presents to the Durable Object.
//!
//! The sync relay (`PylonSync` DO in `pylon-workers`) filters change
//! events per subscriber with `pylon-policy`, which needs the same
//! enriched [`AuthContext`] the machine's WS hub uses. The DO has no
//! database and no session store, so the machine authenticates +
//! enriches ONCE at handshake and hands the client a signed blob:
//!
//! ```text
//! blob = base64url(claims_json) . exp_unix . hex(HMAC_SHA256(secret, exp + "." + claims_json))
//! ```
//!
//! The relay verifies the HMAC with the shared secret
//! (`PYLON_SYNC_RELAY_SECRET` on both sides), checks `exp`, and only
//! then constructs an `AuthContext` from the claims.
//!
//! **Why a separate claims DTO instead of deserializing `AuthContext`:**
//! `AuthContext` bans `Deserialize` on purpose (see the comment on the
//! struct) — parsing one from untrusted JSON is privilege escalation.
//! [`RelayAuthClaims`] is the one sanctioned bridge: it only converts
//! AFTER the HMAC proves the machine minted the payload, so the trust
//! boundary is the signature check, not the parse.
//!
//! **Staleness:** a role revoked mid-connection keeps its old grants
//! until the blob expires. The TTL bounds that window; the relay closes
//! expired connections so clients re-handshake against the machine.

use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::AuthContext;

type HmacSha256 = Hmac<Sha256>;

const APP_SECRET_CONTEXT: &[u8] = b"pylon-relay-app-v1\0";

/// Derive the relay secret for one app from the worker's root secret.
///
/// Pylon Cloud runs one relay worker for many customer apps. Customer
/// machines must never receive the worker root secret because app code can
/// read its own environment. The control plane gives each machine this
/// derived value instead. The worker derives the same value from the routed
/// app id before it verifies a request or auth blob.
pub fn derive_app_secret(root_secret: &str, app: &str) -> String {
    let mut mac =
        HmacSha256::new_from_slice(root_secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(APP_SECRET_CONTEXT);
    mac.update(app.as_bytes());
    let bytes = mac.finalize().into_bytes();
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes.iter() {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Default blob lifetime. Long enough that reconnect churn doesn't
/// hammer the machine, short enough that a revoked role converges
/// within minutes.
pub const DEFAULT_TTL_SECS: u64 = 15 * 60;

/// The signed claims — a mirror of the policy-relevant [`AuthContext`]
/// fields. API-key fields are deliberately absent: the relay is a
/// read-side fan-out and key scopes gate routes, not policies.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RelayAuthClaims {
    /// The relay app id this blob is minted for. The DO MUST reject a
    /// blob whose `app` doesn't match the app it was routed to —
    /// without this, on a shared-secret worker a blob minted by app A's
    /// relay-token route is accepted by app B's DO (cross-app read
    /// leak; an admin blob would bypass app B's policy entirely). The
    /// field lives inside the signed claims, so it's HMAC-covered.
    pub app: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default)]
    pub is_admin: bool,
    #[serde(default)]
    pub is_guest: bool,
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
    /// Unix seconds after which the blob is dead.
    pub exp: u64,
}

impl RelayAuthClaims {
    pub fn from_auth(app: &str, auth: &AuthContext, exp: u64) -> Self {
        Self {
            app: app.to_string(),
            user_id: auth.user_id.clone(),
            is_admin: auth.is_admin,
            is_guest: auth.is_guest,
            roles: auth.roles.clone(),
            tenant_id: auth.tenant_id.clone(),
            exp,
        }
    }

    /// The sanctioned trusted constructor — only call this on claims
    /// that came out of [`verify`].
    pub fn into_auth_context(self) -> AuthContext {
        AuthContext {
            user_id: self.user_id,
            is_admin: self.is_admin,
            is_guest: self.is_guest,
            roles: self.roles,
            tenant_id: self.tenant_id,
            api_key_id: None,
            api_key_scopes: None,
            is_trusted_device: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayBlobError {
    /// Shared secret unset/empty — the relay feature is off.
    SecretNotConfigured,
    /// Blob structure isn't `payload.exp.sig`.
    Malformed,
    /// `exp` is in the past.
    Expired,
    /// HMAC mismatch — tampered payload or wrong secret.
    BadSignature,
    /// The blob's `app` claim doesn't match the app it was presented
    /// to — a cross-app replay on a shared-secret worker.
    WrongApp,
}

impl std::fmt::Display for RelayBlobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::SecretNotConfigured => "relay secret is not configured",
            Self::Malformed => "relay blob is malformed",
            Self::Expired => "relay blob is expired",
            Self::BadSignature => "relay blob signature does not match",
            Self::WrongApp => "relay blob was minted for a different app",
        })
    }
}

fn base64_url<T: AsRef<[u8]>>(input: T) -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    URL_SAFE_NO_PAD.encode(input)
}

fn base64_url_decode(input: &str) -> Option<Vec<u8>> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    URL_SAFE_NO_PAD.decode(input).ok()
}

fn hmac_hex(secret: &str, exp: u64, claims_json: &[u8]) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(format!("{exp}.").as_bytes());
    mac.update(claims_json);
    let bytes = mac.finalize().into_bytes();
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes.iter() {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Mint a blob for an already-authenticated, already-enriched context.
/// `exp` must be absolute unix seconds (caller adds the TTL — the
/// relay has no shared clock source with the machine beyond unix time).
pub fn mint(
    secret: &str,
    app: &str,
    auth: &AuthContext,
    exp: u64,
) -> Result<String, RelayBlobError> {
    if secret.is_empty() {
        return Err(RelayBlobError::SecretNotConfigured);
    }
    let claims = RelayAuthClaims::from_auth(app, auth, exp);
    let claims_json = serde_json::to_vec(&claims).expect("claims serialize");
    let sig = hmac_hex(secret, exp, &claims_json);
    Ok(format!("{}.{exp}.{sig}", base64_url(&claims_json)))
}

/// Verify a blob for a specific app and return its claims. Constant-time
/// on the signature comparison; the expiry check runs BEFORE the HMAC so
/// a dead blob costs nothing, and its error is distinguishable so clients
/// know to re-handshake rather than treat it as an attack.
///
/// `expected_app` MUST be the app the caller routed the blob to (the
/// DO's `?app=` param). A blob whose signed `app` claim differs is
/// rejected — that's what stops a cross-app replay on a shared secret.
pub fn verify(
    secret: &str,
    expected_app: &str,
    blob: &str,
    now_secs: u64,
) -> Result<RelayAuthClaims, RelayBlobError> {
    if secret.is_empty() {
        return Err(RelayBlobError::SecretNotConfigured);
    }
    let mut parts = blob.rsplitn(3, '.');
    let sig = parts.next().ok_or(RelayBlobError::Malformed)?;
    let exp_str = parts.next().ok_or(RelayBlobError::Malformed)?;
    let payload_b64 = parts.next().ok_or(RelayBlobError::Malformed)?;
    if payload_b64.is_empty() || sig.is_empty() {
        return Err(RelayBlobError::Malformed);
    }
    let exp: u64 = exp_str.parse().map_err(|_| RelayBlobError::Malformed)?;
    if exp <= now_secs {
        return Err(RelayBlobError::Expired);
    }
    let claims_json = base64_url_decode(payload_b64).ok_or(RelayBlobError::Malformed)?;
    let expected = hmac_hex(secret, exp, &claims_json);
    if !crate::constant_time_eq(expected.as_bytes(), sig.as_bytes()) {
        return Err(RelayBlobError::BadSignature);
    }
    let claims: RelayAuthClaims =
        serde_json::from_slice(&claims_json).map_err(|_| RelayBlobError::Malformed)?;
    // The exp inside the signed payload is authoritative; the plaintext
    // copy exists so verify can early-exit. A mismatch means tampering.
    if claims.exp != exp {
        return Err(RelayBlobError::BadSignature);
    }
    // App binding — checked AFTER the signature so a wrong-app blob is
    // only reported once we know it's authentically signed.
    if claims.app != expected_app {
        return Err(RelayBlobError::WrongApp);
    }
    Ok(claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> AuthContext {
        let mut c = AuthContext::authenticated("u1".into());
        c.roles = vec!["member".into(), "admin:org_1".into()];
        c.tenant_id = Some("org_1".into());
        c
    }

    #[test]
    fn app_secrets_are_stable_and_isolated() {
        let a1 = derive_app_secret("root-secret", "project-a");
        let a2 = derive_app_secret("root-secret", "project-a");
        let b = derive_app_secret("root-secret", "project-b");
        assert_eq!(a1, a2);
        assert_eq!(a1.len(), 64);
        assert_ne!(a1, b);
        assert_ne!(a1, derive_app_secret("other-root", "project-a"));
        assert_eq!(
            a1,
            "eaee1121c4e3190c850c809f87fbf72999a16f6efcbfc364daac6f29134d1bc1"
        );
    }

    #[test]
    fn roundtrip_preserves_policy_fields() {
        let blob = mint("s3cret", "appA", &ctx(), 2_000).unwrap();
        let claims = verify("s3cret", "appA", &blob, 1_000).unwrap();
        assert_eq!(claims.app, "appA");
        let auth = claims.into_auth_context();
        assert_eq!(auth.user_id.as_deref(), Some("u1"));
        assert_eq!(auth.roles, vec!["member", "admin:org_1"]);
        assert_eq!(auth.tenant_id.as_deref(), Some("org_1"));
        assert!(!auth.is_admin);
        // Never resurrected from a blob:
        assert!(auth.api_key_id.is_none());
        assert!(!auth.is_trusted_device);
    }

    #[test]
    fn a_blob_minted_for_one_app_is_rejected_by_another() {
        // The cross-app leak class: on a shared-secret worker, app A's
        // blob must NOT verify against app B's DO even though the
        // signature is authentic — the app claim is HMAC-covered and
        // checked against the routed app.
        let mut admin = AuthContext::anonymous();
        admin.is_admin = true; // worst case: an unscoped-admin blob
        let blob = mint("shared", "appA", &admin, 2_000).unwrap();
        assert_eq!(
            verify("shared", "appB", &blob, 1_000),
            Err(RelayBlobError::WrongApp)
        );
        // Same app still works.
        assert!(verify("shared", "appA", &blob, 1_000).is_ok());

        // The app is signed: swapping the claim to appB invalidates the
        // HMAC rather than passing as appB.
        let parts: Vec<&str> = {
            let mut p: Vec<&str> = blob.rsplitn(3, '.').collect();
            p.reverse();
            p
        };
        let mut claims: RelayAuthClaims =
            serde_json::from_slice(&base64_url_decode(parts[0]).unwrap()).unwrap();
        claims.app = "appB".into();
        let forged = format!(
            "{}.{}.{}",
            base64_url(serde_json::to_vec(&claims).unwrap()),
            parts[1],
            parts[2]
        );
        assert_eq!(
            verify("shared", "appB", &forged, 1_000),
            Err(RelayBlobError::BadSignature)
        );
    }

    #[test]
    fn expired_blob_is_rejected() {
        let blob = mint("s", "app", &ctx(), 500).unwrap();
        assert_eq!(verify("s", "app", &blob, 500), Err(RelayBlobError::Expired));
        assert_eq!(
            verify("s", "app", &blob, 9_999),
            Err(RelayBlobError::Expired)
        );
    }

    #[test]
    fn wrong_secret_and_tampering_are_rejected() {
        let blob = mint("right", "app", &ctx(), 2_000).unwrap();
        assert_eq!(
            verify("wrong", "app", &blob, 1_000),
            Err(RelayBlobError::BadSignature)
        );

        // Tamper with the payload (grant is_admin) keeping the sig.
        let mut parts: Vec<&str> = blob.rsplitn(3, '.').collect();
        parts.reverse(); // [payload, exp, sig]
        let mut claims: RelayAuthClaims =
            serde_json::from_slice(&base64_url_decode(parts[0]).unwrap()).unwrap();
        claims.is_admin = true;
        let forged_payload = base64_url(serde_json::to_vec(&claims).unwrap());
        let forged = format!("{forged_payload}.{}.{}", parts[1], parts[2]);
        assert_eq!(
            verify("right", "app", &forged, 1_000),
            Err(RelayBlobError::BadSignature)
        );

        // Tamper with the plaintext exp (extend lifetime) keeping the sig.
        let extended = format!("{}.{}.{}", parts[0], 999_999, parts[2]);
        assert_eq!(
            verify("right", "app", &extended, 1_000),
            Err(RelayBlobError::BadSignature)
        );
    }

    #[test]
    fn malformed_blobs_are_rejected_not_panicked() {
        for junk in ["", "a", "a.b", "a.b.c", "!!!.12.beef", "cGF5.notanum.beef"] {
            let err = verify("s", "app", junk, 1).unwrap_err();
            assert!(
                matches!(
                    err,
                    RelayBlobError::Malformed | RelayBlobError::BadSignature
                ),
                "{junk} -> {err}"
            );
        }
    }

    #[test]
    fn empty_secret_means_feature_off() {
        assert_eq!(
            mint("", "app", &ctx(), 2_000),
            Err(RelayBlobError::SecretNotConfigured)
        );
        assert_eq!(
            verify("", "app", "a.1.b", 0),
            Err(RelayBlobError::SecretNotConfigured)
        );
    }
}
