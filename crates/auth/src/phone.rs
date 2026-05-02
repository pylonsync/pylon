//! Phone / SMS magic-code sign-in.
//!
//! Mirror of the email magic-code flow but with phone numbers as
//! the identity. Same code shape (6-digit numeric), same expiry
//! (10 min), same single-use semantics. Pluggable SMS transport
//! lets apps use Twilio / MessageBird / a webhook.
//!
//! Phone numbers are E.164-normalized (`+15551234567`) before any
//! storage / lookup so case + whitespace + formatting differences
//! collapse to one identity.
//!
//! Workflow:
//!   1. POST /api/auth/phone/send-code  { phone }
//!      → SMS arrives with `Your sign-in code is 123456`.
//!   2. POST /api/auth/phone/verify     { phone, code }
//!      → returns the session token, same shape as magic-email.
//!
//! Apps that need full E.164 validation (libphonenumber-style)
//! should plug a custom validator before calling `Phone::normalize`.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// Stored pending code. Same shape as MagicCode but keyed on phone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhoneCode {
    pub phone: String,
    pub code: String,
    pub expires_at: u64,
    pub attempts: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhoneCodeError {
    NotFound,
    Expired,
    BadCode,
    TooManyAttempts,
    Throttled { retry_after_secs: u64 },
    InvalidPhone,
}

impl std::fmt::Display for PhoneCodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => f.write_str("no pending code for this phone"),
            Self::Expired => f.write_str("code expired"),
            Self::BadCode => f.write_str("wrong code"),
            Self::TooManyAttempts => f.write_str("too many failed attempts; request a new code"),
            Self::Throttled { retry_after_secs } => {
                write!(f, "wait {retry_after_secs}s before requesting another code")
            }
            Self::InvalidPhone => f.write_str("phone number not in E.164 format"),
        }
    }
}

pub trait PhoneCodeBackend: Send + Sync {
    fn put(&self, phone: &str, code: &PhoneCode);
    fn get(&self, phone: &str) -> Option<PhoneCode>;
    fn remove(&self, phone: &str);
    fn put_attempts(&self, phone: &str, attempts: u32);
}

pub struct InMemoryPhoneCodeBackend {
    codes: Mutex<HashMap<String, PhoneCode>>,
}

impl Default for InMemoryPhoneCodeBackend {
    fn default() -> Self {
        Self {
            codes: Mutex::new(HashMap::new()),
        }
    }
}

impl PhoneCodeBackend for InMemoryPhoneCodeBackend {
    fn put(&self, phone: &str, code: &PhoneCode) {
        self.codes
            .lock()
            .unwrap()
            .insert(phone.to_string(), code.clone());
    }
    fn get(&self, phone: &str) -> Option<PhoneCode> {
        self.codes.lock().unwrap().get(phone).cloned()
    }
    fn remove(&self, phone: &str) {
        self.codes.lock().unwrap().remove(phone);
    }
    fn put_attempts(&self, phone: &str, attempts: u32) {
        if let Some(c) = self.codes.lock().unwrap().get_mut(phone) {
            c.attempts = attempts;
        }
    }
}

pub struct PhoneCodeStore {
    backend: Box<dyn PhoneCodeBackend>,
}

impl Default for PhoneCodeStore {
    fn default() -> Self {
        Self::new()
    }
}

impl PhoneCodeStore {
    const TTL_SECS: u64 = 10 * 60;
    const RESEND_THROTTLE_SECS: u64 = 30;
    const MAX_ATTEMPTS: u32 = 5;

    pub fn new() -> Self {
        Self::with_backend(Box::new(InMemoryPhoneCodeBackend::default()))
    }
    pub fn with_backend(backend: Box<dyn PhoneCodeBackend>) -> Self {
        Self { backend }
    }

    /// Generate + store a 6-digit code, returning it for the caller
    /// to send via SMS. Throttled to one request per 30 seconds per
    /// phone to make SMS-cost-bombing impractical.
    pub fn try_create(&self, phone: &str) -> Result<String, PhoneCodeError> {
        let normalized = normalize(phone).ok_or(PhoneCodeError::InvalidPhone)?;
        let now = now_secs();
        if let Some(existing) = self.backend.get(&normalized) {
            // Throttle: same phone, recent issuance.
            let issued_at = existing.expires_at.saturating_sub(Self::TTL_SECS);
            if now - issued_at < Self::RESEND_THROTTLE_SECS {
                return Err(PhoneCodeError::Throttled {
                    retry_after_secs: Self::RESEND_THROTTLE_SECS - (now - issued_at),
                });
            }
        }
        let code = generate_code();
        let pc = PhoneCode {
            phone: normalized.clone(),
            code: code.clone(),
            expires_at: now + Self::TTL_SECS,
            attempts: 0,
        };
        self.backend.put(&normalized, &pc);
        Ok(code)
    }

    pub fn try_verify(&self, phone: &str, code: &str) -> Result<(), PhoneCodeError> {
        let normalized = normalize(phone).ok_or(PhoneCodeError::InvalidPhone)?;
        let mut entry = self.backend.get(&normalized).ok_or(PhoneCodeError::NotFound)?;
        if entry.expires_at <= now_secs() {
            self.backend.remove(&normalized);
            return Err(PhoneCodeError::Expired);
        }
        if entry.attempts >= Self::MAX_ATTEMPTS {
            self.backend.remove(&normalized);
            return Err(PhoneCodeError::TooManyAttempts);
        }
        let ok = crate::constant_time_eq(entry.code.as_bytes(), code.trim().as_bytes());
        if ok {
            self.backend.remove(&normalized);
            Ok(())
        } else {
            entry.attempts += 1;
            self.backend.put_attempts(&normalized, entry.attempts);
            if entry.attempts >= Self::MAX_ATTEMPTS {
                self.backend.remove(&normalized);
                return Err(PhoneCodeError::TooManyAttempts);
            }
            Err(PhoneCodeError::BadCode)
        }
    }
}

/// Normalize a user-supplied phone number to E.164. Strips spaces,
/// dashes, parens, dots. Leading `+` required. ASCII digits only.
/// Returns None for malformed input.
pub fn normalize(input: &str) -> Option<String> {
    let mut out = String::with_capacity(input.len());
    let mut started = false;
    for ch in input.chars() {
        match ch {
            '+' if !started => {
                out.push('+');
                started = true;
            }
            '0'..='9' => {
                out.push(ch);
                started = true;
            }
            ' ' | '-' | '.' | '(' | ')' | '\t' => continue,
            _ => return None,
        }
    }
    if !out.starts_with('+') || out.len() < 8 || out.len() > 16 {
        return None;
    }
    Some(out)
}

/// Generate a zero-padded 6-digit code.
fn generate_code() -> String {
    use rand::Rng;
    format!("{:06}", rand::thread_rng().gen_range(0..1_000_000))
}

fn now_secs() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ---------------------------------------------------------------------------
// SMS transport — pluggable, same shape as the email transport
// ---------------------------------------------------------------------------

/// SMS sender. Apps register a Twilio/MessageBird transport at
/// startup; tests use [`NullSmsTransport`].
pub trait SmsSender: Send + Sync {
    fn send_sms(&self, phone: &str, body: &str) -> Result<(), String>;
}

/// No-op transport for tests + the in-memory dev runtime.
pub struct NullSmsTransport;

impl SmsSender for NullSmsTransport {
    fn send_sms(&self, _phone: &str, _body: &str) -> Result<(), String> {
        Ok(())
    }
}

/// Twilio REST API transport. Reads PYLON_TWILIO_ACCOUNT_SID +
/// PYLON_TWILIO_AUTH_TOKEN + PYLON_TWILIO_FROM at construction.
/// `from` MUST be a verified Twilio number / messaging service id.
pub struct TwilioSmsTransport {
    account_sid: String,
    auth_token: String,
    from: String,
}

impl TwilioSmsTransport {
    pub fn from_env() -> Option<Self> {
        Some(Self {
            account_sid: std::env::var("PYLON_TWILIO_ACCOUNT_SID").ok()?,
            auth_token: std::env::var("PYLON_TWILIO_AUTH_TOKEN").ok()?,
            from: std::env::var("PYLON_TWILIO_FROM").ok()?,
        })
    }
}

impl SmsSender for TwilioSmsTransport {
    fn send_sms(&self, phone: &str, body: &str) -> Result<(), String> {
        let url = format!(
            "https://api.twilio.com/2010-04-01/Accounts/{}/Messages.json",
            self.account_sid
        );
        let form = format!(
            "From={}&To={}&Body={}",
            url_encode(&self.from),
            url_encode(phone),
            url_encode(body),
        );
        use base64::{engine::general_purpose::STANDARD, Engine};
        let basic = STANDARD.encode(format!("{}:{}", self.account_sid, self.auth_token).as_bytes());
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(10))
            .timeout_read(std::time::Duration::from_secs(10))
            .build();
        match agent
            .post(&url)
            .set("Content-Type", "application/x-www-form-urlencoded")
            .set("Authorization", &format!("Basic {basic}"))
            .send_string(&form)
        {
            Ok(_) => Ok(()),
            Err(ureq::Error::Status(code, r)) => {
                let body = r.into_string().unwrap_or_default();
                Err(format!("twilio HTTP {code}: {body}"))
            }
            Err(e) => Err(format!("twilio: {e}")),
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_formatting() {
        assert_eq!(normalize("+1 (555) 123-4567"), Some("+15551234567".into()));
        assert_eq!(normalize("+44 20 7946 0958"), Some("+442079460958".into()));
        assert_eq!(normalize("+1.555.123.4567"), Some("+15551234567".into()));
    }

    #[test]
    fn normalize_rejects_no_plus() {
        assert!(normalize("5551234567").is_none());
    }

    #[test]
    fn normalize_rejects_letters() {
        assert!(normalize("+1-555-CALL-NOW").is_none());
    }

    #[test]
    fn normalize_length_bounds() {
        assert!(normalize("+1234").is_none()); // too short
        assert!(normalize("+12345678901234567").is_none()); // too long
    }

    #[test]
    fn create_and_verify_round_trip() {
        let store = PhoneCodeStore::new();
        let code = store.try_create("+15551234567").unwrap();
        assert_eq!(code.len(), 6);
        assert!(store.try_verify("+15551234567", &code).is_ok());
        // Single-use.
        assert_eq!(
            store.try_verify("+15551234567", &code).unwrap_err(),
            PhoneCodeError::NotFound
        );
    }

    #[test]
    fn verify_rejects_wrong_code() {
        let store = PhoneCodeStore::new();
        let _ = store.try_create("+15551234567").unwrap();
        assert_eq!(
            store.try_verify("+15551234567", "000000").unwrap_err(),
            PhoneCodeError::BadCode
        );
    }

    #[test]
    fn too_many_attempts_locks() {
        let store = PhoneCodeStore::new();
        let _ = store.try_create("+15551234567").unwrap();
        for _ in 0..PhoneCodeStore::MAX_ATTEMPTS - 1 {
            let _ = store.try_verify("+15551234567", "000000");
        }
        // Last failure flips to TooManyAttempts.
        assert_eq!(
            store.try_verify("+15551234567", "000000").unwrap_err(),
            PhoneCodeError::TooManyAttempts
        );
    }

    #[test]
    fn invalid_phone_rejected() {
        let store = PhoneCodeStore::new();
        assert_eq!(
            store.try_create("not-a-number").unwrap_err(),
            PhoneCodeError::InvalidPhone
        );
    }

    #[test]
    fn normalization_collapses_formatting_at_send() {
        // Different formatted inputs map to the same store key so a
        // resend uses the same throttle bucket.
        let store = PhoneCodeStore::new();
        let _ = store.try_create("+1 555 123 4567").unwrap();
        // Same phone, different formatting → throttled.
        let err = store.try_create("+15551234567").unwrap_err();
        assert!(matches!(err, PhoneCodeError::Throttled { .. }));
    }
}
