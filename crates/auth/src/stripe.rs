//! Stripe billing — minimal surface area focused on what auth /
//! orgs actually need:
//!   - Create / retrieve a Stripe customer for a user (or org)
//!   - Create a Checkout Session (subscription or one-time)
//!   - Verify webhook signatures (avoid trusting unauthenticated POSTs)
//!   - Map webhook events to a `BillingEvent` enum the host app can match on
//!
//! Out of scope (apps can call the Stripe API directly):
//!   - Invoice / refund / payment-intent management
//!   - Customer portal (one-line redirect; app handles)
//!   - Discounts / promo codes (set in Checkout config)
//!
//! Stripe API docs: <https://docs.stripe.com/api>
//! Webhook signing: <https://docs.stripe.com/webhooks/signatures>

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone)]
pub struct StripeConfig {
    /// Server-side secret key (`sk_live_…` or `sk_test_…`).
    pub api_key: String,
    /// Webhook signing secret (`whsec_…`) for the configured endpoint.
    /// Apps with multiple webhooks should run a separate verifier per
    /// endpoint with each endpoint's own secret.
    pub webhook_secret: Option<String>,
}

impl StripeConfig {
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("PYLON_STRIPE_API_KEY").ok()?;
        let webhook_secret = std::env::var("PYLON_STRIPE_WEBHOOK_SECRET").ok();
        Some(Self {
            api_key,
            webhook_secret,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StripeCustomer {
    pub id: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckoutSession {
    pub id: String,
    /// Hosted checkout URL — what you 302 the user to.
    pub url: String,
    #[serde(default)]
    pub customer: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckoutMode {
    /// Subscription billing — recurring price.
    Subscription,
    /// One-time payment.
    Payment,
}

impl CheckoutMode {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Subscription => "subscription",
            Self::Payment => "payment",
        }
    }
}

impl StripeConfig {
    /// Create or retrieve a Stripe customer for the given email. If
    /// you already store `stripeCustomerId` on the user/org row, pass
    /// it instead — `retrieve_or_create` does the lookup-or-create
    /// dance based on which field is populated.
    pub fn create_customer(&self, email: &str, name: Option<&str>) -> Result<StripeCustomer, String> {
        let mut body = format!("email={}", url_encode(email));
        if let Some(n) = name {
            body.push_str("&name=");
            body.push_str(&url_encode(n));
        }
        self.post("https://api.stripe.com/v1/customers", &body)
    }

    /// Create a Checkout Session — the standard hosted-payment flow.
    /// `price_ids` are the Stripe Price ids the customer is buying
    /// (1 for subscriptions, N for cart-style one-time payments).
    pub fn create_checkout(
        &self,
        customer_id: Option<&str>,
        price_ids: &[&str],
        mode: CheckoutMode,
        success_url: &str,
        cancel_url: &str,
    ) -> Result<CheckoutSession, String> {
        let mut body = format!(
            "mode={}&success_url={}&cancel_url={}",
            mode.as_str(),
            url_encode(success_url),
            url_encode(cancel_url),
        );
        if let Some(cid) = customer_id {
            body.push_str("&customer=");
            body.push_str(&url_encode(cid));
        }
        for (i, pid) in price_ids.iter().enumerate() {
            body.push_str(&format!(
                "&line_items[{i}][price]={}&line_items[{i}][quantity]=1",
                url_encode(pid)
            ));
        }
        self.post("https://api.stripe.com/v1/checkout/sessions", &body)
    }

    fn post<T: for<'de> Deserialize<'de>>(&self, url: &str, body: &str) -> Result<T, String> {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(10))
            .timeout_read(std::time::Duration::from_secs(10))
            .user_agent("pylon-auth/0.1")
            .build();
        let resp = agent
            .post(url)
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .set("Content-Type", "application/x-www-form-urlencoded")
            .send_string(body)
            .map_err(|e| match e {
                ureq::Error::Status(code, r) => {
                    let body = r.into_string().unwrap_or_default();
                    format!("stripe HTTP {code}: {body}")
                }
                e => format!("stripe network: {e}"),
            })?;
        let txt = resp
            .into_string()
            .map_err(|e| format!("stripe body: {e}"))?;
        serde_json::from_str(&txt).map_err(|e| format!("stripe JSON: {e}"))
    }
}

// ---------------------------------------------------------------------------
// Webhook signature verification + event parsing
// ---------------------------------------------------------------------------

/// Subset of Stripe webhook events pylon directly supports. Apps
/// receiving any other event get the raw `event_type` string in
/// [`BillingEvent::Other`] and can match on it themselves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BillingEvent {
    /// `checkout.session.completed` — customer finished checkout.
    /// Attach `customer_id` + `subscription_id` to your user/org row.
    CheckoutCompleted {
        customer_id: Option<String>,
        subscription_id: Option<String>,
        client_reference_id: Option<String>,
    },
    /// `customer.subscription.updated` / `created` — subscription
    /// state changed (renewed, plan changed, paused). Map `status`
    /// to your app's "is this org allowed to use the paid feature"
    /// gate.
    SubscriptionChanged {
        subscription_id: String,
        customer_id: String,
        status: String,
        current_period_end: u64,
    },
    /// `customer.subscription.deleted` — subscription canceled or
    /// ended. Revoke paid access.
    SubscriptionDeleted {
        subscription_id: String,
        customer_id: String,
    },
    /// `invoice.payment_failed` — billing problem; usually pylon
    /// surfaces this to the org's billing email.
    PaymentFailed {
        customer_id: String,
        invoice_id: String,
    },
    /// Any event pylon doesn't model. Carries the raw event type
    /// + the full JSON body for the app to parse.
    Other {
        event_type: String,
        body: serde_json::Value,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebhookError {
    /// `Stripe-Signature` header missing or malformed.
    MissingSignature,
    /// Timestamp older than 5 min or newer than 5 min — replay
    /// protection per Stripe's docs.
    StaleTimestamp,
    /// HMAC-SHA256 mismatch — payload was tampered with or the
    /// secret is wrong.
    BadSignature,
    /// Body wasn't valid JSON.
    BadJson,
}

impl std::fmt::Display for WebhookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::MissingSignature => "Stripe-Signature header missing",
            Self::StaleTimestamp => "webhook timestamp outside ±5min tolerance",
            Self::BadSignature => "webhook signature mismatch",
            Self::BadJson => "webhook body not valid JSON",
        })
    }
}

/// Verify a webhook payload + parse it into a `BillingEvent`.
///
/// `signature_header` is the raw `Stripe-Signature` header value,
/// shaped `t=<unix_ts>,v1=<hex_sig>[,v0=<old>]`. We accept any v1
/// matching the configured secret; v0 is the deprecated scheme and
/// we ignore it.
pub fn verify_webhook(
    secret: &str,
    body: &[u8],
    signature_header: &str,
    now_secs: u64,
) -> Result<BillingEvent, WebhookError> {
    let mut t: Option<u64> = None;
    let mut v1_sigs: Vec<&str> = Vec::new();
    for kv in signature_header.split(',') {
        let kv = kv.trim();
        if let Some(v) = kv.strip_prefix("t=") {
            t = v.parse().ok();
        } else if let Some(v) = kv.strip_prefix("v1=") {
            v1_sigs.push(v);
        }
    }
    let ts = t.ok_or(WebhookError::MissingSignature)?;
    if v1_sigs.is_empty() {
        return Err(WebhookError::MissingSignature);
    }
    // ±5min tolerance, Stripe's documented default.
    let diff = if now_secs > ts { now_secs - ts } else { ts - now_secs };
    if diff > 5 * 60 {
        return Err(WebhookError::StaleTimestamp);
    }

    // Signed payload = "<ts>." + body
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .expect("HMAC accepts any key length");
    mac.update(format!("{ts}.").as_bytes());
    mac.update(body);
    let expected = mac.finalize().into_bytes();
    let expected_hex = bytes_to_hex(&expected);

    let any_match = v1_sigs
        .iter()
        .any(|s| crate::constant_time_eq(s.as_bytes(), expected_hex.as_bytes()));
    if !any_match {
        return Err(WebhookError::BadSignature);
    }

    let body_json: serde_json::Value =
        serde_json::from_slice(body).map_err(|_| WebhookError::BadJson)?;
    Ok(parse_event(body_json))
}

fn parse_event(body: serde_json::Value) -> BillingEvent {
    let event_type = body
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let object = body.pointer("/data/object").cloned().unwrap_or_default();
    match event_type.as_str() {
        "checkout.session.completed" => BillingEvent::CheckoutCompleted {
            customer_id: object
                .get("customer")
                .and_then(|v| v.as_str())
                .map(String::from),
            subscription_id: object
                .get("subscription")
                .and_then(|v| v.as_str())
                .map(String::from),
            client_reference_id: object
                .get("client_reference_id")
                .and_then(|v| v.as_str())
                .map(String::from),
        },
        "customer.subscription.updated" | "customer.subscription.created" => {
            BillingEvent::SubscriptionChanged {
                subscription_id: object
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                customer_id: object
                    .get("customer")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                status: object
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                current_period_end: object
                    .get("current_period_end")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
            }
        }
        "customer.subscription.deleted" => BillingEvent::SubscriptionDeleted {
            subscription_id: object
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            customer_id: object
                .get("customer")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        },
        "invoice.payment_failed" => BillingEvent::PaymentFailed {
            customer_id: object
                .get("customer")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            invoice_id: object
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        },
        _ => BillingEvent::Other {
            event_type,
            body,
        },
    }
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Sha256;

    fn sign(secret: &str, ts: u64, body: &[u8]) -> String {
        let mut mac =
            Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
        mac.update(format!("{ts}.").as_bytes());
        mac.update(body);
        bytes_to_hex(&mac.finalize().into_bytes())
    }

    #[test]
    fn verify_webhook_round_trip_checkout_completed() {
        let secret = "whsec_test_secret";
        let body = br#"{
            "type": "checkout.session.completed",
            "data": { "object": {
                "customer": "cus_xyz",
                "subscription": "sub_abc",
                "client_reference_id": "user_123"
            }}
        }"#;
        let ts = 1_700_000_000;
        let sig = sign(secret, ts, body);
        let header = format!("t={ts},v1={sig}");
        let event = verify_webhook(secret, body, &header, ts).unwrap();
        match event {
            BillingEvent::CheckoutCompleted {
                customer_id,
                subscription_id,
                client_reference_id,
            } => {
                assert_eq!(customer_id.as_deref(), Some("cus_xyz"));
                assert_eq!(subscription_id.as_deref(), Some("sub_abc"));
                assert_eq!(client_reference_id.as_deref(), Some("user_123"));
            }
            other => panic!("expected CheckoutCompleted, got {other:?}"),
        }
    }

    #[test]
    fn verify_webhook_rejects_bad_signature() {
        let body = b"{}";
        let ts = 1_700_000_000;
        let header = format!("t={ts},v1=deadbeefdeadbeef");
        assert_eq!(
            verify_webhook("secret", body, &header, ts),
            Err(WebhookError::BadSignature)
        );
    }

    #[test]
    fn verify_webhook_rejects_stale_timestamp() {
        let secret = "s";
        let body = b"{}";
        let ts = 1_700_000_000;
        let sig = sign(secret, ts, body);
        let header = format!("t={ts},v1={sig}");
        // 6 minutes later — outside Stripe's ±5min tolerance.
        let now = ts + 6 * 60;
        assert_eq!(
            verify_webhook(secret, body, &header, now),
            Err(WebhookError::StaleTimestamp)
        );
    }

    #[test]
    fn verify_webhook_missing_signature_header() {
        let body = b"{}";
        assert_eq!(
            verify_webhook("s", body, "", 0),
            Err(WebhookError::MissingSignature)
        );
        // Has timestamp but no v1 sig.
        assert_eq!(
            verify_webhook("s", body, "t=100", 100),
            Err(WebhookError::MissingSignature)
        );
    }

    #[test]
    fn parse_subscription_changed() {
        let body = serde_json::json!({
            "type": "customer.subscription.updated",
            "data": { "object": {
                "id": "sub_xyz",
                "customer": "cus_abc",
                "status": "active",
                "current_period_end": 9_999_999_999u64
            }}
        });
        match parse_event(body) {
            BillingEvent::SubscriptionChanged {
                subscription_id,
                customer_id,
                status,
                current_period_end,
            } => {
                assert_eq!(subscription_id, "sub_xyz");
                assert_eq!(customer_id, "cus_abc");
                assert_eq!(status, "active");
                assert_eq!(current_period_end, 9_999_999_999);
            }
            other => panic!("expected SubscriptionChanged, got {other:?}"),
        }
    }

    #[test]
    fn unknown_event_falls_through_to_other() {
        let body = serde_json::json!({"type": "some.weird.event", "data": {}});
        match parse_event(body) {
            BillingEvent::Other { event_type, .. } => {
                assert_eq!(event_type, "some.weird.event");
            }
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[test]
    fn webhook_accepts_multiple_v1_sigs() {
        // Stripe rotates webhook secrets via "endpoint signing secret
        // rotation"; during the rotation window, a single Stripe-Signature
        // header carries v1 sigs for both old + new secrets. Verifier
        // must accept any matching one.
        let secret = "new_secret";
        let body = br#"{"type":"x"}"#;
        let ts = 1_700_000_000;
        let sig_new = sign(secret, ts, body);
        let header = format!("t={ts},v1=deadbeef,v1={sig_new}");
        assert!(verify_webhook(secret, body, &header, ts).is_ok());
    }
}
