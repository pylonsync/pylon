//! Pluggable email transport for auth flows (magic codes, invitations, etc.)
//! and the app-facing `ctx.email.send` channel.

use pylon_kernel::EmailMessage;

// ---------------------------------------------------------------------------
// Email transport trait
// ---------------------------------------------------------------------------

/// Pluggable email delivery backend.
///
/// Implemented for SendGrid, Resend, Stack0, and a generic webhook
/// endpoint. The `ConsoleTransport` prints to stderr for local
/// development. Messages carry optional HTML and base64 attachments;
/// plain-text senders build one with [`EmailMessage::plain`].
pub trait EmailTransport: Send + Sync {
    fn send(&self, msg: &EmailMessage) -> Result<(), EmailError>;
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
    fn send(&self, msg: &EmailMessage) -> Result<(), EmailError> {
        eprintln!("[email] To: {}", msg.to);
        eprintln!("[email] Subject: {}", msg.subject);
        eprintln!("[email] Body: {}", msg.text);
        if msg.html.is_some() {
            eprintln!("[email] (html body present)");
        }
        for a in &msg.attachments {
            eprintln!(
                "[email] Attachment: {} ({}, {} base64 bytes)",
                a.filename,
                a.content_type,
                a.content.len()
            );
        }
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
    ///
    /// Attachment `content_type` values pass through VERBATIM in every
    /// arm — parameterized types like `text/calendar; method=REQUEST`
    /// are what make Gmail/Outlook render an RSVP-able invite instead
    /// of a plain file download, so no normalization is allowed here.
    pub fn build_body(&self, msg: &EmailMessage) -> String {
        let (from_name, from_email) = self.from_parts();
        match self.provider {
            HttpEmailProvider::SendGrid => {
                let mut from = serde_json::json!({ "email": from_email });
                if let Some(name) = from_name {
                    from["name"] = serde_json::Value::String(name.to_string());
                }
                // SendGrid requires content parts ordered text/plain
                // first, then text/html.
                let mut content = vec![serde_json::json!({
                    "type": "text/plain", "value": msg.text
                })];
                if let Some(html) = &msg.html {
                    content.push(serde_json::json!({
                        "type": "text/html", "value": html
                    }));
                }
                let mut body = serde_json::json!({
                    "personalizations": [{"to": [{"email": msg.to}]}],
                    "from": from,
                    "subject": msg.subject,
                    "content": content
                });
                if !msg.attachments.is_empty() {
                    body["attachments"] = msg
                        .attachments
                        .iter()
                        .map(|a| {
                            serde_json::json!({
                                "content": a.content,
                                "filename": a.filename,
                                "type": a.content_type,
                                "disposition": "attachment"
                            })
                        })
                        .collect();
                }
                body.to_string()
            }
            // Resend accepts the `Name <email>` string form directly.
            HttpEmailProvider::Resend => {
                let mut body = serde_json::json!({
                    "from": self.from,
                    "to": [msg.to],
                    "subject": msg.subject,
                    "text": msg.text
                });
                if let Some(html) = &msg.html {
                    body["html"] = serde_json::Value::String(html.clone());
                }
                if !msg.attachments.is_empty() {
                    body["attachments"] = msg
                        .attachments
                        .iter()
                        .map(|a| {
                            serde_json::json!({
                                "filename": a.filename,
                                "content": a.content,
                                "content_type": a.content_type
                            })
                        })
                        .collect();
                }
                body.to_string()
            }
            // Stack0's mail/send validates `from` as a bare email string and
            // rejects `Name <email>`; a display name must go through the
            // structured `{email, name}` form. The html/attachments shape
            // mirrors Resend's.
            HttpEmailProvider::Stack0 => {
                let from = match from_name {
                    Some(name) => serde_json::json!({ "email": from_email, "name": name }),
                    None => serde_json::json!(from_email),
                };
                let mut body = serde_json::json!({
                    "from": from,
                    "to": [msg.to],
                    "subject": msg.subject,
                    "text": msg.text
                });
                if let Some(html) = &msg.html {
                    body["html"] = serde_json::Value::String(html.clone());
                }
                if !msg.attachments.is_empty() {
                    body["attachments"] = msg
                        .attachments
                        .iter()
                        .map(|a| {
                            serde_json::json!({
                                "filename": a.filename,
                                "content": a.content,
                                "content_type": a.content_type
                            })
                        })
                        .collect();
                }
                body.to_string()
            }
            HttpEmailProvider::Webhook => {
                // `body` (the legacy key) stays so existing receiver
                // endpoints keep working; `text`/`html`/`attachments`
                // are additive.
                let mut body = serde_json::json!({
                    "to": msg.to,
                    "from": self.from,
                    "subject": msg.subject,
                    "body": msg.text,
                    "text": msg.text
                });
                if let Some(html) = &msg.html {
                    body["html"] = serde_json::Value::String(html.clone());
                }
                if !msg.attachments.is_empty() {
                    body["attachments"] = serde_json::to_value(&msg.attachments)
                        .unwrap_or(serde_json::Value::Null);
                }
                body.to_string()
            }
        }
    }
}

impl EmailTransport for HttpEmailTransport {
    fn send(&self, msg: &EmailMessage) -> Result<(), EmailError> {
        let body_json = self.build_body(msg);
        // A multi-megabyte base64 attachment can't finish inside the
        // default 10s write window; scale the HTTP timeouts when
        // attachments are present. The TS-side call deadline (60s)
        // remains the upper bound.
        let timeout_secs = if msg.attachments.is_empty() { 10 } else { 60 };
        post_json(&self.endpoint, &self.api_key, &body_json, timeout_secs)
            .map_err(|message| EmailError { message })
    }
}

/// POST a JSON body with a Bearer token, using ureq. `timeout_secs`
/// bounds read + write (connect stays at 10s).
fn post_json(url: &str, api_key: &str, body: &str, timeout_secs: u64) -> Result<(), String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout_read(std::time::Duration::from_secs(timeout_secs))
        .timeout_write(std::time::Duration::from_secs(timeout_secs))
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

    fn plain(to: &str, subject: &str, body: &str) -> EmailMessage {
        EmailMessage::plain(to, subject, body)
    }

    #[test]
    fn console_transport_succeeds() {
        let t = ConsoleTransport;
        assert!(t.send(&plain("test@example.com", "Code", "123456")).is_ok());
    }

    #[test]
    fn sendgrid_body_format() {
        let t = HttpEmailTransport {
            endpoint: "https://api.sendgrid.com/v3/mail/send".into(),
            api_key: "key".into(),
            from: "noreply@test.com".into(),
            provider: HttpEmailProvider::SendGrid,
        };
        let body = t.build_body(&plain("user@test.com", "Your code", "123456"));
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
        let body = t.build_body(&plain("user@test.com", "Your code", "123456"));
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
        let body = t.build_body(&plain("user@test.com", "Your code", "123456"));
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["from"], "noreply@test.com");
        assert_eq!(parsed["to"][0], "user@test.com");
        assert_eq!(parsed["subject"], "Your code");
        assert_eq!(parsed["text"], "123456");
    }

    fn with_extras(html: Option<&str>, ics: bool) -> EmailMessage {
        let mut m = plain("user@test.com", "Invite", "You're invited");
        m.html = html.map(String::from);
        if ics {
            m.attachments.push(pylon_kernel::EmailAttachment {
                filename: "invite.ics".into(),
                content_type: "text/calendar; method=REQUEST".into(),
                content: "QkVHSU46VkNBTEVOREFS".into(),
            });
        }
        m
    }

    #[test]
    fn sendgrid_html_and_attachments() {
        let t = HttpEmailTransport {
            endpoint: "https://api.sendgrid.com/v3/mail/send".into(),
            api_key: "key".into(),
            from: "noreply@test.com".into(),
            provider: HttpEmailProvider::SendGrid,
        };
        // html-only: text/plain part FIRST, text/html second (SendGrid
        // rejects other orderings), no attachments key.
        let p: serde_json::Value = serde_json::from_str(
            &t.build_body(&with_extras(Some("<p>hi</p>"), false)),
        )
        .unwrap();
        assert_eq!(p["content"][0]["type"], "text/plain");
        assert_eq!(p["content"][1]["type"], "text/html");
        assert_eq!(p["content"][1]["value"], "<p>hi</p>");
        assert!(p.get("attachments").is_none());

        // attachments-only: the parameterized calendar content type must
        // survive VERBATIM — it's what makes the invite RSVP-able.
        let p: serde_json::Value =
            serde_json::from_str(&t.build_body(&with_extras(None, true))).unwrap();
        assert_eq!(p["content"].as_array().unwrap().len(), 1);
        assert_eq!(p["attachments"][0]["type"], "text/calendar; method=REQUEST");
        assert_eq!(p["attachments"][0]["filename"], "invite.ics");
        assert_eq!(p["attachments"][0]["disposition"], "attachment");

        // both together
        let p: serde_json::Value = serde_json::from_str(
            &t.build_body(&with_extras(Some("<p>hi</p>"), true)),
        )
        .unwrap();
        assert_eq!(p["content"][1]["type"], "text/html");
        assert_eq!(p["attachments"][0]["content"], "QkVHSU46VkNBTEVOREFS");
    }

    #[test]
    fn resend_html_and_attachments() {
        let t = HttpEmailTransport {
            endpoint: "https://api.resend.com/emails".into(),
            api_key: "key".into(),
            from: "noreply@test.com".into(),
            provider: HttpEmailProvider::Resend,
        };
        let p: serde_json::Value = serde_json::from_str(
            &t.build_body(&with_extras(Some("<p>hi</p>"), true)),
        )
        .unwrap();
        assert_eq!(p["html"], "<p>hi</p>");
        assert_eq!(p["text"], "You're invited");
        assert_eq!(
            p["attachments"][0]["content_type"],
            "text/calendar; method=REQUEST"
        );
        // Plain send omits the optional keys entirely.
        let p: serde_json::Value =
            serde_json::from_str(&t.build_body(&plain("u@t.com", "s", "b"))).unwrap();
        assert!(p.get("html").is_none());
        assert!(p.get("attachments").is_none());
    }

    #[test]
    fn stack0_html_and_attachments() {
        let t = HttpEmailTransport {
            endpoint: "https://api.stack0.dev/v1/mail/send".into(),
            api_key: "key".into(),
            from: "noreply@test.com".into(),
            provider: HttpEmailProvider::Stack0,
        };
        let p: serde_json::Value = serde_json::from_str(
            &t.build_body(&with_extras(Some("<p>hi</p>"), true)),
        )
        .unwrap();
        assert_eq!(p["html"], "<p>hi</p>");
        assert_eq!(
            p["attachments"][0]["content_type"],
            "text/calendar; method=REQUEST"
        );
    }

    #[test]
    fn webhook_keeps_legacy_body_key() {
        let t = HttpEmailTransport {
            endpoint: "https://hooks.example.com/mail".into(),
            api_key: "key".into(),
            from: "noreply@test.com".into(),
            provider: HttpEmailProvider::Webhook,
        };
        let p: serde_json::Value = serde_json::from_str(
            &t.build_body(&with_extras(Some("<p>hi</p>"), true)),
        )
        .unwrap();
        // `body` is the legacy key existing receiver endpoints parse;
        // text/html/attachments are additive.
        assert_eq!(p["body"], "You're invited");
        assert_eq!(p["text"], "You're invited");
        assert_eq!(p["html"], "<p>hi</p>");
        assert_eq!(
            p["attachments"][0]["content_type"],
            "text/calendar; method=REQUEST"
        );
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
            serde_json::from_str(&stack0.build_body(&plain("u@test.com", "s", "b"))).unwrap();
        assert_eq!(p["from"]["email"], "noreply@mail.pylonsync.com");
        assert_eq!(p["from"]["name"], "Pylon Cloud");

        let sendgrid = HttpEmailTransport {
            endpoint: "https://api.sendgrid.com/v3/mail/send".into(),
            api_key: "key".into(),
            from: "Pylon Cloud <noreply@mail.pylonsync.com>".into(),
            provider: HttpEmailProvider::SendGrid,
        };
        let ps: serde_json::Value =
            serde_json::from_str(&sendgrid.build_body(&plain("u@test.com", "s", "b"))).unwrap();
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
