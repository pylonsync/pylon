//! Pluggable email transport for auth flows (magic codes, invitations, etc.).

// ---------------------------------------------------------------------------
// Email transport trait
// ---------------------------------------------------------------------------

/// Pluggable email delivery backend.
///
/// Implemented for SMTP, SendGrid, SES, Resend, etc.
/// The `ConsoleTransport` prints to stderr for local development.
pub trait EmailTransport: Send + Sync {
    fn send(&self, to: &str, subject: &str, body: &str) -> Result<(), EmailError>;
}

#[derive(Debug, Clone)]
pub struct EmailError {
    pub message: String,
}

impl std::fmt::Display for EmailError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "EmailError: {}", self.message)
    }
}

impl std::error::Error for EmailError {}

// ---------------------------------------------------------------------------
// Console transport (dev mode)
// ---------------------------------------------------------------------------

/// Prints emails to stderr. Used in development.
pub struct ConsoleTransport;

impl EmailTransport for ConsoleTransport {
    fn send(&self, to: &str, subject: &str, body: &str) -> Result<(), EmailError> {
        eprintln!("[email] To: {to}");
        eprintln!("[email] Subject: {subject}");
        eprintln!("[email] Body: {body}");
        eprintln!("[email] ---");
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// HTTP transport (SendGrid, Resend, generic webhook)
// ---------------------------------------------------------------------------

/// Email delivery via HTTP POST (SendGrid, Resend, or any HTTP endpoint).
pub struct HttpEmailTransport {
    pub endpoint: String,
    pub api_key: String,
    pub from: String,
    pub provider: HttpEmailProvider,
}

#[derive(Debug, Clone, Copy)]
pub enum HttpEmailProvider {
    SendGrid,
    Resend,
    Webhook,
}

impl HttpEmailTransport {
    /// Create from environment variables.
    ///
    /// Reads: PYLON_EMAIL_PROVIDER (sendgrid|resend|webhook),
    /// PYLON_EMAIL_API_KEY, PYLON_EMAIL_FROM, PYLON_EMAIL_ENDPOINT
    pub fn from_env() -> Option<Self> {
        let provider_str = std::env::var("PYLON_EMAIL_PROVIDER").ok()?;
        let provider = match provider_str.as_str() {
            "sendgrid" => HttpEmailProvider::SendGrid,
            "resend" => HttpEmailProvider::Resend,
            "webhook" => HttpEmailProvider::Webhook,
            _ => return None,
        };

        let endpoint = match provider {
            HttpEmailProvider::SendGrid => "https://api.sendgrid.com/v3/mail/send".to_string(),
            HttpEmailProvider::Resend => "https://api.resend.com/emails".to_string(),
            HttpEmailProvider::Webhook => std::env::var("PYLON_EMAIL_ENDPOINT").ok()?,
        };

        Some(Self {
            endpoint,
            api_key: std::env::var("PYLON_EMAIL_API_KEY").ok()?,
            from: std::env::var("PYLON_EMAIL_FROM").unwrap_or_else(|_| "noreply@pylonsync.com".into()),
            provider,
        })
    }

    /// Build the JSON body for the provider's API.
    pub fn build_body(&self, to: &str, subject: &str, body: &str) -> String {
        match self.provider {
            HttpEmailProvider::SendGrid => serde_json::json!({
                "personalizations": [{"to": [{"email": to}]}],
                "from": {"email": self.from},
                "subject": subject,
                "content": [{"type": "text/plain", "value": body}]
            })
            .to_string(),
            HttpEmailProvider::Resend => serde_json::json!({
                "from": self.from,
                "to": [to],
                "subject": subject,
                "text": body
            })
            .to_string(),
            HttpEmailProvider::Webhook => serde_json::json!({
                "to": to,
                "from": self.from,
                "subject": subject,
                "body": body
            })
            .to_string(),
        }
    }
}

impl EmailTransport for HttpEmailTransport {
    fn send(&self, to: &str, subject: &str, body: &str) -> Result<(), EmailError> {
        let body_json = self.build_body(to, subject, body);
        post_json(&self.endpoint, &self.api_key, &body_json)
            .map_err(|message| EmailError { message })
    }
}

/// POST a JSON body with a Bearer token, using ureq with a 10s timeout.
fn post_json(url: &str, api_key: &str, body: &str) -> Result<(), String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout_read(std::time::Duration::from_secs(10))
        .timeout_write(std::time::Duration::from_secs(10))
        .user_agent("pylon/0.1")
        .build();

    match agent
        .post(url)
        .set("Content-Type", "application/json")
        .set("Authorization", &format!("Bearer {api_key}"))
        .send_string(body)
    {
        Ok(_) => Ok(()),
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(format!("HTTP {code}: {body}"))
        }
        Err(e) => Err(format!("HTTP error: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_transport_succeeds() {
        let t = ConsoleTransport;
        assert!(t.send("test@example.com", "Code", "123456").is_ok());
    }

    #[test]
    fn sendgrid_body_format() {
        let t = HttpEmailTransport {
            endpoint: "https://api.sendgrid.com/v3/mail/send".into(),
            api_key: "key".into(),
            from: "noreply@test.com".into(),
            provider: HttpEmailProvider::SendGrid,
        };
        let body = t.build_body("user@test.com", "Your code", "123456");
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(parsed["personalizations"][0]["to"][0]["email"] == "user@test.com");
        assert!(parsed["from"]["email"] == "noreply@test.com");
    }

    #[test]
    fn resend_body_format() {
        let t = HttpEmailTransport {
            endpoint: "https://api.resend.com/emails".into(),
            api_key: "key".into(),
            from: "noreply@test.com".into(),
            provider: HttpEmailProvider::Resend,
        };
        let body = t.build_body("user@test.com", "Your code", "123456");
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(parsed["to"][0] == "user@test.com");
        assert!(parsed["text"] == "123456");
    }
}
