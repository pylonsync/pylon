use crate::Plugin;
use std::sync::Mutex;

use super::net_guard::is_private_ip;

/// Email delivery method.
pub enum EmailTransport {
    /// Log to console (dev mode).
    Log,
    /// Send via SMTP.
    Smtp(SmtpConfig),
}

/// SMTP server configuration.
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub from: String,
}

/// An email to be sent.
pub struct EmailMessage {
    pub to: String,
    pub subject: String,
    pub body: String,
}

/// Record of a sent email.
#[derive(Debug, Clone)]
pub struct SentEmail {
    pub to: String,
    pub subject: String,
    pub timestamp: String,
    pub success: bool,
}

/// Email transport plugin. Sends emails via SMTP or logs them in dev mode.
pub struct EmailPlugin {
    transport: EmailTransport,
    sent: Mutex<Vec<SentEmail>>,
}

fn now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{:03}", ts.as_secs(), ts.subsec_millis())
}

/// Send an email via a minimal SMTP client using raw TCP.
fn smtp_send(config: &SmtpConfig, msg: &EmailMessage) -> Result<(), String> {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    let addr = format!("{}:{}", config.host, config.port);

    // SSRF protection: block connections to private/reserved IP ranges.
    if is_private_ip(&addr) {
        return Err("SMTP connection to private/reserved IP addresses is not allowed".into());
    }

    let stream = TcpStream::connect(&addr).map_err(|e| format!("SMTP connect failed: {e}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(10))).ok();

    let mut reader = BufReader::new(
        stream
            .try_clone()
            .map_err(|e| format!("Stream clone failed: {e}"))?,
    );
    // We need a mutable reference to write; use try_clone so reader owns its own handle.
    let mut writer = stream;

    let mut line = String::new();

    // Helper: read one SMTP response line.
    let read_line = |reader: &mut BufReader<TcpStream>, buf: &mut String| -> Result<(), String> {
        buf.clear();
        reader
            .read_line(buf)
            .map_err(|e| format!("SMTP read failed: {e}"))?;
        Ok(())
    };

    // Read server greeting.
    read_line(&mut reader, &mut line)?;

    // EHLO
    writer
        .write_all(b"EHLO localhost\r\n")
        .map_err(|e| format!("SMTP write failed: {e}"))?;
    // Read EHLO response (may be multi-line; read until we get a line not starting with "250-").
    loop {
        read_line(&mut reader, &mut line)?;
        if !line.starts_with("250-") {
            break;
        }
    }

    // MAIL FROM
    write!(writer, "MAIL FROM:<{}>\r\n", config.from)
        .map_err(|e| format!("SMTP write failed: {e}"))?;
    read_line(&mut reader, &mut line)?;

    // RCPT TO
    write!(writer, "RCPT TO:<{}>\r\n", msg.to).map_err(|e| format!("SMTP write failed: {e}"))?;
    read_line(&mut reader, &mut line)?;

    // DATA
    writer
        .write_all(b"DATA\r\n")
        .map_err(|e| format!("SMTP write failed: {e}"))?;
    read_line(&mut reader, &mut line)?;

    // Message headers + body, terminated by CRLF.CRLF
    write!(
        writer,
        "Subject: {}\r\nFrom: {}\r\nTo: {}\r\n\r\n{}\r\n.\r\n",
        msg.subject, config.from, msg.to, msg.body
    )
    .map_err(|e| format!("SMTP write failed: {e}"))?;
    read_line(&mut reader, &mut line)?;

    // QUIT
    writer
        .write_all(b"QUIT\r\n")
        .map_err(|e| format!("SMTP write failed: {e}"))?;

    Ok(())
}

impl EmailPlugin {
    /// Create a new email plugin with the given transport.
    pub fn new(transport: EmailTransport) -> Self {
        Self {
            transport,
            sent: Mutex::new(Vec::new()),
        }
    }

    /// Create a dev-mode plugin that only logs emails.
    pub fn dev() -> Self {
        Self::new(EmailTransport::Log)
    }

    /// Send an email via the configured transport.
    pub fn send(&self, msg: EmailMessage) -> Result<(), String> {
        let result = match &self.transport {
            EmailTransport::Log => {
                eprintln!(
                    "[email:dev] to={} subject=\"{}\" body_len={}",
                    msg.to,
                    msg.subject,
                    msg.body.len()
                );
                Ok(())
            }
            EmailTransport::Smtp(config) => smtp_send(config, &msg),
        };

        let success = result.is_ok();
        self.sent.lock().unwrap().push(SentEmail {
            to: msg.to,
            subject: msg.subject,
            timestamp: now(),
            success,
        });

        result
    }

    /// Return the history of all emails sent through this plugin.
    pub fn sent_history(&self) -> Vec<SentEmail> {
        self.sent.lock().unwrap().clone()
    }

    /// Convenience: send a magic-code authentication email.
    pub fn send_magic_code(&self, email: &str, code: &str) -> Result<(), String> {
        self.send(EmailMessage {
            to: email.to_string(),
            subject: "Your login code".to_string(),
            body: format!(
                "Your verification code is: {}\n\nThis code expires in 10 minutes.\nIf you did not request this, please ignore this email.",
                code
            ),
        })
    }

    /// Convenience: send a welcome email.
    pub fn send_welcome(&self, email: &str, name: &str) -> Result<(), String> {
        self.send(EmailMessage {
            to: email.to_string(),
            subject: "Welcome!".to_string(),
            body: format!(
                "Hi {},\n\nWelcome! Your account has been created successfully.\n\nBest regards,\nThe Team",
                name
            ),
        })
    }
}

impl Plugin for EmailPlugin {
    fn name(&self) -> &str {
        "email"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_mode_logs_and_records() {
        let plugin = EmailPlugin::dev();
        let result = plugin.send(EmailMessage {
            to: "user@example.com".into(),
            subject: "Test".into(),
            body: "Hello".into(),
        });
        assert!(result.is_ok());
        assert_eq!(plugin.sent_history().len(), 1);
        assert!(plugin.sent_history()[0].success);
    }

    #[test]
    fn send_magic_code_formats_correctly() {
        let plugin = EmailPlugin::dev();
        plugin
            .send_magic_code("user@example.com", "123456")
            .unwrap();

        let history = plugin.sent_history();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].to, "user@example.com");
        assert_eq!(history[0].subject, "Your login code");
        assert!(history[0].success);
    }

    #[test]
    fn send_welcome_formats_correctly() {
        let plugin = EmailPlugin::dev();
        plugin.send_welcome("user@example.com", "Alice").unwrap();

        let history = plugin.sent_history();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].to, "user@example.com");
        assert_eq!(history[0].subject, "Welcome!");
        assert!(history[0].success);
    }

    #[test]
    fn sent_history_tracks_multiple() {
        let plugin = EmailPlugin::dev();
        plugin
            .send(EmailMessage {
                to: "a@example.com".into(),
                subject: "First".into(),
                body: "1".into(),
            })
            .unwrap();
        plugin
            .send(EmailMessage {
                to: "b@example.com".into(),
                subject: "Second".into(),
                body: "2".into(),
            })
            .unwrap();
        plugin
            .send(EmailMessage {
                to: "c@example.com".into(),
                subject: "Third".into(),
                body: "3".into(),
            })
            .unwrap();

        let history = plugin.sent_history();
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].to, "a@example.com");
        assert_eq!(history[1].to, "b@example.com");
        assert_eq!(history[2].to, "c@example.com");
    }

    #[test]
    fn multiple_sends_accumulate() {
        let plugin = EmailPlugin::dev();
        for i in 0..5 {
            plugin
                .send(EmailMessage {
                    to: format!("user{}@example.com", i),
                    subject: format!("Email {}", i),
                    body: "body".into(),
                })
                .unwrap();
        }
        assert_eq!(plugin.sent_history().len(), 5);
    }

    #[test]
    fn plugin_name() {
        let plugin = EmailPlugin::dev();
        assert_eq!(plugin.name(), "email");
    }

    #[test]
    fn smtp_transport_blocks_private_ip() {
        let plugin = EmailPlugin::new(EmailTransport::Smtp(SmtpConfig {
            host: "127.0.0.1".into(),
            port: 19998,
            username: "user".into(),
            password: "pass".into(),
            from: "noreply@example.com".into(),
        }));

        let result = plugin.send(EmailMessage {
            to: "user@example.com".into(),
            subject: "Test".into(),
            body: "Hello".into(),
        });

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("private/reserved"));
        // Even on failure, the attempt is recorded.
        let history = plugin.sent_history();
        assert_eq!(history.len(), 1);
        assert!(!history[0].success);
    }

    #[test]
    fn smtp_blocks_10_network() {
        let plugin = EmailPlugin::new(EmailTransport::Smtp(SmtpConfig {
            host: "10.0.0.1".into(),
            port: 25,
            username: "user".into(),
            password: "pass".into(),
            from: "noreply@example.com".into(),
        }));

        let result = plugin.send(EmailMessage {
            to: "user@example.com".into(),
            subject: "Test".into(),
            body: "Hello".into(),
        });

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("private/reserved"));
    }

    #[test]
    fn smtp_blocks_metadata_endpoint() {
        let plugin = EmailPlugin::new(EmailTransport::Smtp(SmtpConfig {
            host: "169.254.169.254".into(),
            port: 25,
            username: "user".into(),
            password: "pass".into(),
            from: "noreply@example.com".into(),
        }));

        let result = plugin.send(EmailMessage {
            to: "user@example.com".into(),
            subject: "Test".into(),
            body: "Hello".into(),
        });

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("private/reserved"));
    }
}
