//! CAPTCHA token verification for hCaptcha, Cloudflare Turnstile,
//! Google reCAPTCHA v3.
//!
//! Apps wire CAPTCHA into endpoints that bots love (magic-code send,
//! password register, account creation) by:
//!   1. Setting `PYLON_CAPTCHA_PROVIDER` (`hcaptcha` | `turnstile` |
//!      `recaptcha`) and `PYLON_CAPTCHA_SECRET` (server-side secret
//!      from the provider).
//!   2. Frontend includes the CAPTCHA widget; user-supplied token
//!      arrives in the `captchaToken` JSON field on the request.
//!   3. Pylon endpoints check `CaptchaConfig::from_env()` and call
//!      `verify()` before processing — failure returns 400.
//!
//! All three providers expose a similar shape: POST a token to a
//! verify endpoint, get back `{"success": bool, …}`. We collapse them
//! to one trait + one config so the host app decides "do I have CAPTCHA
//! enabled?" not "which CAPTCHA?"

use serde::Deserialize;

/// Which CAPTCHA service the app uses. Reads `PYLON_CAPTCHA_PROVIDER`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptchaProvider {
    HCaptcha,
    Turnstile,
    ReCaptcha,
}

impl CaptchaProvider {
    fn endpoint(&self) -> &'static str {
        match self {
            // hCaptcha: https://docs.hcaptcha.com/#verify-the-user-response-server-side
            Self::HCaptcha => "https://api.hcaptcha.com/siteverify",
            // Turnstile: https://developers.cloudflare.com/turnstile/get-started/server-side-validation/
            Self::Turnstile => "https://challenges.cloudflare.com/turnstile/v0/siteverify",
            // reCAPTCHA: https://developers.google.com/recaptcha/docs/verify
            Self::ReCaptcha => "https://www.google.com/recaptcha/api/siteverify",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "hcaptcha" => Some(Self::HCaptcha),
            "turnstile" | "cloudflare" => Some(Self::Turnstile),
            "recaptcha" | "google" => Some(Self::ReCaptcha),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CaptchaConfig {
    pub provider: CaptchaProvider,
    pub secret: String,
    /// Minimum score for reCAPTCHA v3 (range 0.0..=1.0). Other
    /// providers ignore this. Default 0.5 — Google's recommended
    /// threshold for "probably human."
    pub min_score: f64,
}

impl CaptchaConfig {
    /// Pull config from `PYLON_CAPTCHA_PROVIDER` + `PYLON_CAPTCHA_SECRET`.
    /// Returns `None` when CAPTCHA is not configured (apps treat
    /// `None` as "skip CAPTCHA check entirely").
    pub fn from_env() -> Option<Self> {
        let provider = CaptchaProvider::from_str(&std::env::var("PYLON_CAPTCHA_PROVIDER").ok()?)?;
        let secret = std::env::var("PYLON_CAPTCHA_SECRET").ok()?;
        let min_score = std::env::var("PYLON_CAPTCHA_MIN_SCORE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.5);
        Some(Self {
            provider,
            secret,
            min_score,
        })
    }

    /// Verify a user-supplied CAPTCHA token. Returns `Ok(())` on
    /// success; `Err(reason)` on any failure (reason is safe to
    /// surface as a generic message — don't leak it to clients).
    pub fn verify(&self, token: &str, remote_ip: Option<&str>) -> Result<(), String> {
        if token.is_empty() {
            return Err("CAPTCHA token is empty".into());
        }
        let mut body = format!(
            "secret={}&response={}",
            url_encode(&self.secret),
            url_encode(token)
        );
        if let Some(ip) = remote_ip {
            body.push_str("&remoteip=");
            body.push_str(&url_encode(ip));
        }
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(5))
            .timeout_read(std::time::Duration::from_secs(5))
            .build();
        let resp = agent
            .post(self.provider.endpoint())
            .set("Content-Type", "application/x-www-form-urlencoded")
            .send_string(&body)
            .map_err(|e| format!("captcha network: {e}"))?
            .into_string()
            .map_err(|e| format!("captcha body: {e}"))?;
        let parsed: SiteVerifyResponse =
            serde_json::from_str(&resp).map_err(|e| format!("captcha bad JSON: {e}"))?;
        if !parsed.success {
            return Err(format!(
                "captcha rejected: {}",
                parsed.error_codes.unwrap_or_default().join(",")
            ));
        }
        if let CaptchaProvider::ReCaptcha = self.provider {
            // reCAPTCHA v3 returns a score; v2 doesn't include the
            // field at all. None → treat as v2 (any success passes).
            if let Some(score) = parsed.score {
                if score < self.min_score {
                    return Err(format!(
                        "captcha score {score:.2} below threshold {:.2}",
                        self.min_score
                    ));
                }
            }
        }
        // P3-8 (codex Wave-3 review): reject stale tokens to limit
        // captured-token replay. 2-minute window is conservative;
        // fresh sign-in flows complete in seconds.
        if let Some(ts) = parsed.challenge_ts.as_deref() {
            if let Ok(parsed_ts) = chrono::DateTime::parse_from_rfc3339(ts) {
                let age_secs = chrono::Utc::now()
                    .signed_duration_since(parsed_ts.with_timezone(&chrono::Utc))
                    .num_seconds();
                if age_secs > 120 {
                    return Err(format!("captcha token stale ({age_secs}s old)"));
                }
            }
        }
        Ok(())
    }
}

/// Common subset of fields all three providers return on success.
#[derive(Debug, Deserialize)]
struct SiteVerifyResponse {
    success: bool,
    /// reCAPTCHA v3 only.
    #[serde(default)]
    score: Option<f64>,
    /// Issued-at timestamp from the provider — present on hCaptcha
    /// + Turnstile + reCAPTCHA. ISO-8601. Used to defeat replay
    /// (an attacker who captures a token gets ~2 min to use it
    /// before pylon rejects it as stale).
    #[serde(default, rename = "challenge_ts")]
    challenge_ts: Option<String>,
    /// Provider-specific error codes — left opaque since the host
    /// app shouldn't surface them to the caller.
    #[serde(default, rename = "error-codes")]
    error_codes: Option<Vec<String>>,
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
    fn provider_from_str_recognizes_aliases() {
        assert_eq!(
            CaptchaProvider::from_str("hcaptcha"),
            Some(CaptchaProvider::HCaptcha)
        );
        assert_eq!(
            CaptchaProvider::from_str("HCAPTCHA"),
            Some(CaptchaProvider::HCaptcha)
        );
        assert_eq!(
            CaptchaProvider::from_str("turnstile"),
            Some(CaptchaProvider::Turnstile)
        );
        assert_eq!(
            CaptchaProvider::from_str("cloudflare"),
            Some(CaptchaProvider::Turnstile)
        );
        assert_eq!(
            CaptchaProvider::from_str("recaptcha"),
            Some(CaptchaProvider::ReCaptcha)
        );
        assert_eq!(
            CaptchaProvider::from_str("google"),
            Some(CaptchaProvider::ReCaptcha)
        );
        assert_eq!(CaptchaProvider::from_str("nope"), None);
    }

    #[test]
    fn endpoints_are_https() {
        for p in [
            CaptchaProvider::HCaptcha,
            CaptchaProvider::Turnstile,
            CaptchaProvider::ReCaptcha,
        ] {
            assert!(
                p.endpoint().starts_with("https://"),
                "endpoint must be https"
            );
        }
    }

    #[test]
    fn empty_token_rejected_without_network() {
        let cfg = CaptchaConfig {
            provider: CaptchaProvider::HCaptcha,
            secret: "test".into(),
            min_score: 0.5,
        };
        assert!(cfg.verify("", None).is_err());
    }
}
