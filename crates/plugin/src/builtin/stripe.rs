//! Stripe billing primitives.
//!
//! This plugin is intentionally small. It does NOT make HTTP calls to Stripe
//! — apps drive the API from TypeScript functions where they already have a
//! networking story. What this module *does* provide is the security-critical
//! and easy-to-mess-up bits:
//!
//! - **Webhook signature verification** (`verify_signature`) — Stripe rejects
//!   a webhook if you don't validate the `Stripe-Signature` header against
//!   your endpoint secret. Getting the HMAC + timestamp comparison right is
//!   subtle (constant-time compare, replay window). This implementation
//!   matches Stripe's published reference algorithm.
//! - **Event payload typing** (`StripeEvent`) — a tiny shape over what arrives
//!   from a webhook, so app code can match on `event.type` without re-parsing
//!   raw JSON.
//! - **Customer lookup state** (`StripeCustomerStore`) — an optional in-memory
//!   map from app user id → Stripe customer id, useful in dev. Production
//!   apps store this in their own user table.
//!
//! See https://stripe.com/docs/webhooks/signatures for the algorithm spec.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::Plugin;

/// Default Stripe replay window: 5 minutes.
const DEFAULT_TOLERANCE_SECS: u64 = 300;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignatureError {
    MissingTimestamp,
    MissingSignature,
    Replayed,
    InvalidSignature,
    BadHeaderFormat,
}

/// Verify a Stripe webhook signature.
///
/// `header` is the raw `Stripe-Signature` header value. `payload` is the raw
/// request body bytes (NOT a re-serialized JSON value — Stripe signs the
/// exact bytes they sent). `secret` is the endpoint signing secret from your
/// Stripe dashboard (`whsec_...`).
///
/// `now_unix_secs` is injected so tests can pin the clock; production callers
/// pass `current_unix_secs()`.
pub fn verify_signature(
    header: &str,
    payload: &[u8],
    secret: &str,
    now_unix_secs: u64,
    tolerance_secs: u64,
) -> Result<(), SignatureError> {
    let mut timestamp: Option<u64> = None;
    let mut sigs: Vec<&str> = Vec::new();

    for part in header.split(',') {
        let mut kv = part.splitn(2, '=');
        let key = kv.next().unwrap_or("").trim();
        let val = kv.next().ok_or(SignatureError::BadHeaderFormat)?.trim();
        match key {
            "t" => timestamp = val.parse().ok(),
            "v1" => sigs.push(val),
            _ => {} // older / future schemes ignored
        }
    }

    let ts = timestamp.ok_or(SignatureError::MissingTimestamp)?;
    if sigs.is_empty() {
        return Err(SignatureError::MissingSignature);
    }
    if now_unix_secs.saturating_sub(ts) > tolerance_secs {
        return Err(SignatureError::Replayed);
    }

    let signed_payload = format!("{ts}.");
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .map_err(|_| SignatureError::InvalidSignature)?;
    mac.update(signed_payload.as_bytes());
    mac.update(payload);
    let expected = mac.finalize().into_bytes();
    let expected_hex = hex_encode(&expected);

    // Constant-time compare against any matching v1 signature.
    if sigs
        .iter()
        .any(|s| ct_eq(s.as_bytes(), expected_hex.as_bytes()))
    {
        Ok(())
    } else {
        Err(SignatureError::InvalidSignature)
    }
}

pub fn current_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Minimal projection of a Stripe webhook event.
///
/// Apps usually want `event_type` to dispatch on, then `data` for the actual
/// object payload. Skipping the full Stripe schema keeps this plugin compile-
/// independent of any one Stripe API version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StripeEvent {
    pub id: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub created: u64,
    pub data: serde_json::Value,
}

impl StripeEvent {
    pub fn from_payload(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }

    pub fn object_id(&self) -> Option<&str> {
        self.data
            .get("object")
            .and_then(|o| o.get("id"))
            .and_then(|v| v.as_str())
    }
}

/// In-memory mapping from app user id → Stripe customer id.
///
/// For dev / tests / single-process deployments. Production apps store this
/// on the User entity directly so it survives restarts.
pub struct StripeCustomerStore {
    map: Mutex<HashMap<String, String>>,
}

impl StripeCustomerStore {
    pub fn new() -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
        }
    }

    pub fn link(&self, user_id: &str, stripe_customer_id: &str) {
        self.map
            .lock()
            .unwrap()
            .insert(user_id.into(), stripe_customer_id.into());
    }

    pub fn lookup(&self, user_id: &str) -> Option<String> {
        self.map.lock().unwrap().get(user_id).cloned()
    }

    pub fn unlink(&self, user_id: &str) -> Option<String> {
        self.map.lock().unwrap().remove(user_id)
    }
}

impl Default for StripeCustomerStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Aggregate plugin so apps can register the whole Stripe surface at once.
pub struct StripePlugin {
    pub customers: StripeCustomerStore,
    pub webhook_secret: String,
    pub tolerance_secs: u64,
}

impl StripePlugin {
    pub fn new(webhook_secret: impl Into<String>) -> Self {
        Self {
            customers: StripeCustomerStore::new(),
            webhook_secret: webhook_secret.into(),
            tolerance_secs: DEFAULT_TOLERANCE_SECS,
        }
    }

    pub fn verify_webhook(
        &self,
        header: &str,
        payload: &[u8],
    ) -> Result<StripeEvent, SignatureError> {
        verify_signature(
            header,
            payload,
            &self.webhook_secret,
            current_unix_secs(),
            self.tolerance_secs,
        )?;
        StripeEvent::from_payload(payload).map_err(|_| SignatureError::InvalidSignature)
    }
}

impl Plugin for StripePlugin {
    fn name(&self) -> &str {
        "stripe"
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xF) as usize] as char);
    }
    out
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed_header(ts: u64, payload: &[u8], secret: &str) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(format!("{ts}.").as_bytes());
        mac.update(payload);
        let sig = hex_encode(&mac.finalize().into_bytes());
        format!("t={ts},v1={sig}")
    }

    #[test]
    fn verifies_valid_signature() {
        let payload = br#"{"id":"evt_1","type":"checkout.session.completed","created":1,"data":{"object":{"id":"cs_1"}}}"#;
        let secret = "whsec_test";
        let ts = 1_700_000_000;
        let header = signed_header(ts, payload, secret);
        verify_signature(&header, payload, secret, ts + 5, 300).unwrap();
    }

    #[test]
    fn rejects_tampered_payload() {
        let secret = "whsec_test";
        let ts = 1_700_000_000;
        let header = signed_header(ts, b"original", secret);
        let err = verify_signature(&header, b"tampered", secret, ts, 300).unwrap_err();
        assert_eq!(err, SignatureError::InvalidSignature);
    }

    #[test]
    fn rejects_wrong_secret() {
        let payload = b"hi";
        let header = signed_header(100, payload, "whsec_a");
        let err = verify_signature(&header, payload, "whsec_b", 100, 300).unwrap_err();
        assert_eq!(err, SignatureError::InvalidSignature);
    }

    #[test]
    fn rejects_replay_outside_tolerance() {
        let payload = b"hi";
        let secret = "whsec";
        let ts = 1_000;
        let header = signed_header(ts, payload, secret);
        let err = verify_signature(&header, payload, secret, ts + 1000, 300).unwrap_err();
        assert_eq!(err, SignatureError::Replayed);
    }

    #[test]
    fn rejects_missing_timestamp() {
        let err = verify_signature("v1=abc", b"hi", "secret", 0, 300).unwrap_err();
        assert_eq!(err, SignatureError::MissingTimestamp);
    }

    #[test]
    fn rejects_missing_signature() {
        let err = verify_signature("t=100", b"hi", "secret", 100, 300).unwrap_err();
        assert_eq!(err, SignatureError::MissingSignature);
    }

    #[test]
    fn accepts_one_of_multiple_v1_signatures() {
        let payload = b"hi";
        let secret = "whsec";
        let ts = 100;
        let valid = signed_header(ts, payload, secret);
        // Pull the v1 portion off and reuse with an extra bogus v1 in front.
        let v1 = valid.split(',').find(|p| p.starts_with("v1=")).unwrap();
        let header = format!("t={ts},v1=deadbeef,{v1}");
        verify_signature(&header, payload, secret, ts, 300).unwrap();
    }

    #[test]
    fn parses_event_payload() {
        let bytes = br#"{"id":"evt_X","type":"customer.created","created":42,"data":{"object":{"id":"cus_1"}}}"#;
        let ev = StripeEvent::from_payload(bytes).unwrap();
        assert_eq!(ev.id, "evt_X");
        assert_eq!(ev.event_type, "customer.created");
        assert_eq!(ev.created, 42);
        assert_eq!(ev.object_id(), Some("cus_1"));
    }

    #[test]
    fn customer_store_round_trip() {
        let s = StripeCustomerStore::new();
        s.link("user_1", "cus_abc");
        assert_eq!(s.lookup("user_1").as_deref(), Some("cus_abc"));
        assert_eq!(s.unlink("user_1").as_deref(), Some("cus_abc"));
        assert_eq!(s.lookup("user_1"), None);
    }

    #[test]
    fn plugin_verify_webhook_end_to_end() {
        let secret = "whsec_E2E";
        let payload = br#"{"id":"evt_1","type":"x","created":1,"data":{}}"#;
        let plugin = StripePlugin::new(secret);
        let ts = current_unix_secs();
        let header = signed_header(ts, payload, secret);
        let ev = plugin.verify_webhook(&header, payload).unwrap();
        assert_eq!(ev.event_type, "x");
    }
}
