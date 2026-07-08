//! Pluggable email transport for auth flows (magic codes, invitations, etc.).

// ---------------------------------------------------------------------------
// Email transport trait
// ---------------------------------------------------------------------------

/// Pluggable email delivery backend.
///
/// Implemented for SMTP, SendGrid, SES, Resend, Stack0, etc.
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
// HTTP transport (SendGrid, Resend, Stack0, generic webhook)
// ---------------------------------------------------------------------------

/// Email delivery via HTTP POST (SendGrid, Resend, Stack0, or any HTTP endpoint).
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
    Stack0,
    Webhook,
}

impl HttpEmailTransport {
    /// App-facing transport, read from `PYLON_EMAIL_*`.
    ///
    /// Backs the `ctx.email.send` primitive available to app code —
    /// arbitrary recipient and body. Because that's a spam/phishing
    /// surface, it must be the customer's OWN provider: on Pylon Cloud
    /// this family is intentionally left UNSET unless the customer
    /// configures one, so arbitrary app email is never sent on a shared
    /// platform key. Auth-flow email is configured separately via
    /// [`Self::from_env_auth`].
    ///
    /// Reads: PYLON_EMAIL_PROVIDER (sendgrid|resend|stack0|webhook),
    /// PYLON_EMAIL_API_KEY, PYLON_EMAIL_FROM, PYLON_EMAIL_ENDPOINT
    pub fn from_env() -> Option<Self> {
        Self::from_env_prefixed("PYLON_EMAIL")
    }

    /// Auth-flow transport, read from `PYLON_AUTH_EMAIL_*` with a
    /// back-compat fallback to `PYLON_EMAIL_*`.
    ///
    /// Auth flows (magic codes, password reset, invitations) only ever
    /// email the address a user just entered into a form — a
    /// self-selected recipient — with a fixed, framework-controlled
    /// template. That makes this the one email channel safe to back with
    /// a shared, locked-down platform key on a dedicated sending
    /// subdomain (e.g. `noreply@apps.pyln.dev`). Pylon Cloud injects
    /// `PYLON_AUTH_EMAIL_*` here WITHOUT setting `PYLON_EMAIL_*`, so the
    /// shared key is unreachable from app code via `ctx.email`.
    ///
    /// Falling back to `PYLON_EMAIL_*` keeps single-provider self-hosters
    /// working unchanged: set one family and both auth + app email use it.
    pub fn from_env_auth() -> Option<Self> {
        Self::from_env_prefixed("PYLON_AUTH_EMAIL")
            .or_else(|| Self::from_env_prefixed("PYLON_EMAIL"))
    }

    /// Read a `{prefix}_PROVIDER` / `{prefix}_API_KEY` / `{prefix}_FROM` /
    /// `{prefix}_ENDPOINT` env family into a transport. `None` when the
    /// provider (or a required field) is unset, so callers fall back to
    /// `ConsoleTransport`.
    fn from_env_prefixed(prefix: &str) -> Option<Self> {
        let provider_str = std::env::var(format!("{prefix}_PROVIDER")).ok()?;
        let provider = match provider_str.as_str() {
            "sendgrid" => HttpEmailProvider::SendGrid,
            "resend" => HttpEmailProvider::Resend,
            "stack0" => HttpEmailProvider::Stack0,
            "webhook" => HttpEmailProvider::Webhook,
            _ => return None,
        };

        let endpoint = match provider {
            HttpEmailProvider::SendGrid => "https://api.sendgrid.com/v3/mail/send".to_string(),
            HttpEmailProvider::Resend => "https://api.resend.com/emails".to_string(),
            HttpEmailProvider::Stack0 => "https://api.stack0.dev/v1/mail/send".to_string(),
            HttpEmailProvider::Webhook => std::env::var(format!("{prefix}_ENDPOINT")).ok()?,
        };

        Some(Self {
            endpoint,
            api_key: std::env::var(format!("{prefix}_API_KEY")).ok()?,
            from: std::env::var(format!("{prefix}_FROM"))
                .unwrap_or_else(|_| "noreply@pylonsync.com".into()),
            provider,
        })
    }

    /// Split the configured `From` into an optional display name and the bare
    /// address. Accepts RFC-5322 `Name <email>` and a plain `email`. Used to
    /// feed providers whose APIs take a structured sender (SendGrid, Stack0)
    /// rather than a `Name <email>` string — those reject the concatenated form.
    fn from_parts(&self) -> (Option<&str>, &str) {
        let s = self.from.trim();
        if let (Some(lt), Some(gt)) = (s.find('<'), s.rfind('>')) {
            if lt < gt {
                let name = s[..lt].trim().trim_matches('"').trim();
                let email = s[lt + 1..gt].trim();
                if !email.is_empty() {
                    return (if name.is_empty() { None } else { Some(name) }, email);
                }
            }
        }
        (None, s)
    }

    /// Build the JSON body for the provider's API.
    pub fn build_body(&self, to: &str, subject: &str, body: &str) -> String {
        let (from_name, from_email) = self.from_parts();
        match self.provider {
            HttpEmailProvider::SendGrid => {
                let mut from = serde_json::json!({ "email": from_email });
                if let Some(name) = from_name {
                    from["name"] = serde_json::Value::String(name.to_string());
                }
                serde_json::json!({
                    "personalizations": [{"to": [{"email": to}]}],
                    "from": from,
                    "subject": subject,
                    "content": [{"type": "text/plain", "value": body}]
                })
                .to_string()
            }
            // Resend accepts the `Name <email>` string form directly.
            HttpEmailProvider::Resend => serde_json::json!({
                "from": self.from,
                "to": [to],
                "subject": subject,
                "text": body
            })
            .to_string(),
            // Stack0's mail/send validates `from` as a bare email string and
            // rejects `Name <email>`; a display name must go through the
            // structured `{email, name}` form.
            HttpEmailProvider::Stack0 => {
                let from = match from_name {
                    Some(name) => serde_json::json!({ "email": from_email, "name": name }),
                    None => serde_json::json!(from_email),
                };
                serde_json::json!({
                    "from": from,
                    "to": [to],
                    "subject": subject,
                    "text": body
                })
                .to_string()
            }
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

    #[test]
    fn stack0_body_format() {
        let t = HttpEmailTransport {
            endpoint: "https://api.stack0.dev/mail/send".into(),
            api_key: "key".into(),
            from: "noreply@test.com".into(),
            provider: HttpEmailProvider::Stack0,
        };
        let body = t.build_body("user@test.com", "Your code", "123456");
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["from"], "noreply@test.com");
        assert_eq!(parsed["to"][0], "user@test.com");
        assert_eq!(parsed["subject"], "Your code");
        assert_eq!(parsed["text"], "123456");
    }

    #[test]
    fn display_name_sent_as_structured_from() {
        // Stack0 + SendGrid reject a `Name <email>` string, so a display name
        // must go through the structured {email, name} form. A bare From (the
        // test above) stays a plain string.
        let stack0 = HttpEmailTransport {
            endpoint: "https://api.stack0.dev/v1/mail/send".into(),
            api_key: "key".into(),
            from: "Pylon Cloud <noreply@mail.pylonsync.com>".into(),
            provider: HttpEmailProvider::Stack0,
        };
        let p: serde_json::Value =
            serde_json::from_str(&stack0.build_body("u@test.com", "s", "b")).unwrap();
        assert_eq!(p["from"]["email"], "noreply@mail.pylonsync.com");
        assert_eq!(p["from"]["name"], "Pylon Cloud");

        let sendgrid = HttpEmailTransport {
            endpoint: "https://api.sendgrid.com/v3/mail/send".into(),
            api_key: "key".into(),
            from: "Pylon Cloud <noreply@mail.pylonsync.com>".into(),
            provider: HttpEmailProvider::SendGrid,
        };
        let ps: serde_json::Value =
            serde_json::from_str(&sendgrid.build_body("u@test.com", "s", "b")).unwrap();
        assert_eq!(ps["from"]["email"], "noreply@mail.pylonsync.com");
        assert_eq!(ps["from"]["name"], "Pylon Cloud");
    }

    #[test]
    fn stack0_from_env_picks_correct_endpoint() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // Snapshot + clear env vars we touch so this test is hermetic.
        let prev_provider = std::env::var("PYLON_EMAIL_PROVIDER").ok();
        let prev_key = std::env::var("PYLON_EMAIL_API_KEY").ok();
        let prev_from = std::env::var("PYLON_EMAIL_FROM").ok();

        std::env::set_var("PYLON_EMAIL_PROVIDER", "stack0");
        std::env::set_var("PYLON_EMAIL_API_KEY", "sk_test_abc");
        std::env::set_var("PYLON_EMAIL_FROM", "noreply@example.com");

        let t = HttpEmailTransport::from_env().expect("should construct");
        assert_eq!(t.endpoint, "https://api.stack0.dev/v1/mail/send");
        assert_eq!(t.from, "noreply@example.com");
        assert!(matches!(t.provider, HttpEmailProvider::Stack0));

        // Restore.
        match prev_provider {
            Some(v) => std::env::set_var("PYLON_EMAIL_PROVIDER", v),
            None => std::env::remove_var("PYLON_EMAIL_PROVIDER"),
        }
        match prev_key {
            Some(v) => std::env::set_var("PYLON_EMAIL_API_KEY", v),
            None => std::env::remove_var("PYLON_EMAIL_API_KEY"),
        }
        match prev_from {
            Some(v) => std::env::set_var("PYLON_EMAIL_FROM", v),
            None => std::env::remove_var("PYLON_EMAIL_FROM"),
        }
    }

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// RAII: clears every email-related env var on construction and
    /// restores the prior values on drop (panic-safe, so a failing
    /// assertion can't leak env into another test). Hold ENV_LOCK for
    /// the duration alongside this guard.
    struct EmailEnvGuard {
        saved: Vec<(&'static str, Option<String>)>,
    }

    impl EmailEnvGuard {
        const KEYS: [&'static str; 8] = [
            "PYLON_EMAIL_PROVIDER",
            "PYLON_EMAIL_API_KEY",
            "PYLON_EMAIL_FROM",
            "PYLON_EMAIL_ENDPOINT",
            "PYLON_AUTH_EMAIL_PROVIDER",
            "PYLON_AUTH_EMAIL_API_KEY",
            "PYLON_AUTH_EMAIL_FROM",
            "PYLON_AUTH_EMAIL_ENDPOINT",
        ];

        fn clean() -> Self {
            let saved = Self::KEYS
                .iter()
                .map(|k| (*k, std::env::var(k).ok()))
                .collect();
            for k in Self::KEYS {
                std::env::remove_var(k);
            }
            Self { saved }
        }
    }

    impl Drop for EmailEnvGuard {
        fn drop(&mut self) {
            for (k, v) in &self.saved {
                match v {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
        }
    }

    /// The abuse fence: the app-facing `ctx.email` transport
    /// (`from_env`) must NEVER read `PYLON_AUTH_EMAIL_*`. Otherwise a
    /// shared platform auth key injected on Pylon Cloud would be
    /// reachable by app code to send arbitrary mail.
    #[test]
    fn auth_email_key_is_not_reachable_by_app_ctx_email() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _g = EmailEnvGuard::clean();

        // Platform configures ONLY the auth channel (shared, locked-down
        // key on a dedicated sending subdomain).
        std::env::set_var("PYLON_AUTH_EMAIL_PROVIDER", "stack0");
        std::env::set_var("PYLON_AUTH_EMAIL_API_KEY", "sk_shared_auth");
        std::env::set_var("PYLON_AUTH_EMAIL_FROM", "noreply@apps.pyln.dev");

        // Auth flows pick it up...
        let auth = HttpEmailTransport::from_env_auth().expect("auth transport");
        assert_eq!(auth.endpoint, "https://api.stack0.dev/v1/mail/send");
        assert_eq!(auth.from, "noreply@apps.pyln.dev");
        assert!(matches!(auth.provider, HttpEmailProvider::Stack0));

        // ...but the app-facing ctx.email transport sees NOTHING, so it
        // falls back to ConsoleTransport at the adapter — arbitrary app
        // mail is never sent on the shared key.
        assert!(
            HttpEmailTransport::from_env().is_none(),
            "ctx.email must not read PYLON_AUTH_EMAIL_*"
        );
    }

    /// Back-compat: a single-provider self-hoster sets only
    /// `PYLON_EMAIL_*`, and auth email falls back to it.
    #[test]
    fn auth_email_falls_back_to_app_env() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _g = EmailEnvGuard::clean();

        std::env::set_var("PYLON_EMAIL_PROVIDER", "resend");
        std::env::set_var("PYLON_EMAIL_API_KEY", "sk_self_hosted");
        std::env::set_var("PYLON_EMAIL_FROM", "noreply@example.com");

        let auth = HttpEmailTransport::from_env_auth().expect("falls back to PYLON_EMAIL_*");
        assert_eq!(auth.endpoint, "https://api.resend.com/emails");
        assert_eq!(auth.from, "noreply@example.com");
        assert!(matches!(auth.provider, HttpEmailProvider::Resend));
    }

    /// When both families are set, auth email prefers its own
    /// (`PYLON_AUTH_EMAIL_*`) over the app fallback.
    #[test]
    fn auth_email_prefers_auth_env_over_app() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _g = EmailEnvGuard::clean();

        std::env::set_var("PYLON_EMAIL_PROVIDER", "resend");
        std::env::set_var("PYLON_EMAIL_API_KEY", "sk_app");
        std::env::set_var("PYLON_EMAIL_FROM", "app@example.com");
        std::env::set_var("PYLON_AUTH_EMAIL_PROVIDER", "stack0");
        std::env::set_var("PYLON_AUTH_EMAIL_API_KEY", "sk_auth");
        std::env::set_var("PYLON_AUTH_EMAIL_FROM", "noreply@apps.pyln.dev");

        let auth = HttpEmailTransport::from_env_auth().expect("auth transport");
        assert!(matches!(auth.provider, HttpEmailProvider::Stack0));
        assert_eq!(auth.from, "noreply@apps.pyln.dev");
        assert_eq!(auth.api_key, "sk_auth");
    }
}
