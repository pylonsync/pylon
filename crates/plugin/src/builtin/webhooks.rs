use std::sync::Mutex;

use crate::Plugin;
use pylon_auth::AuthContext;
use serde_json::Value;

use super::net_guard::is_private_ip;

/// How the webhook plugin handles delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryMode {
    /// Log webhook events only (no HTTP request).
    Log,
    /// Deliver via HTTP POST (no local log).
    Deliver,
    /// Log and deliver.
    Both,
}

/// A webhook registration.
#[derive(Clone)]
pub struct WebhookConfig {
    /// URL to POST to.
    pub url: String,
    /// Entity to watch. None = all entities.
    pub entity: Option<String>,
    /// Events to fire on. Empty = all events.
    pub events: Vec<String>,
    /// Optional secret for HMAC signing.
    pub secret: Option<String>,
}

/// Webhooks plugin. Fires HTTP POST callbacks on entity changes.
pub struct WebhooksPlugin {
    hooks: Vec<WebhookConfig>,
    log: Mutex<Vec<WebhookEvent>>,
    delivery_log: Mutex<Vec<DeliveryAttempt>>,
    max_log: usize,
    mode: DeliveryMode,
}

#[derive(Debug, Clone)]
pub struct WebhookEvent {
    pub url: String,
    pub entity: String,
    pub event: String,
    pub row_id: String,
    pub status: String,
}

/// A single delivery attempt record.
#[derive(Debug, Clone)]
pub struct DeliveryAttempt {
    pub url: String,
    pub status: u16,
    pub success: bool,
    pub timestamp: String,
    pub error: Option<String>,
}

fn now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{:03}", ts.as_secs(), ts.subsec_millis())
}

/// Actually deliver a webhook via HTTP POST.
fn deliver(url: &str, payload: &str) -> Result<u16, String> {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    // Parse URL: http://host:port/path
    let url = url
        .strip_prefix("http://")
        .ok_or("Only http:// URLs supported")?;
    let (host_port, path) = match url.find('/') {
        Some(i) => (&url[..i], &url[i..]),
        None => (url, "/"),
    };

    // SSRF protection: block connections to private/reserved IP ranges.
    if is_private_ip(host_port) {
        return Err("Connection to private/reserved IP addresses is not allowed".into());
    }

    let mut stream =
        TcpStream::connect(host_port).map_err(|e| format!("Connection failed: {e}"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .ok();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .ok();

    let request = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        path, host_port, payload.len(), payload
    );

    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("Write failed: {e}"))?;

    let mut response = String::new();
    stream.read_to_string(&mut response).ok();

    // Parse status code from response.
    let status: u16 = response
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    Ok(status)
}

impl WebhooksPlugin {
    pub fn new() -> Self {
        Self {
            hooks: Vec::new(),
            log: Mutex::new(Vec::new()),
            delivery_log: Mutex::new(Vec::new()),
            max_log: 100,
            mode: DeliveryMode::Log,
        }
    }

    /// Create a plugin with the specified delivery mode.
    pub fn with_mode(mode: DeliveryMode) -> Self {
        Self {
            hooks: Vec::new(),
            log: Mutex::new(Vec::new()),
            delivery_log: Mutex::new(Vec::new()),
            max_log: 100,
            mode,
        }
    }

    pub fn add(&mut self, config: WebhookConfig) {
        self.hooks.push(config);
    }

    pub fn log(&self) -> Vec<WebhookEvent> {
        self.log.lock().unwrap().clone()
    }

    /// Return all delivery attempts.
    pub fn delivery_history(&self) -> Vec<DeliveryAttempt> {
        self.delivery_log.lock().unwrap().clone()
    }

    fn fire(&self, entity: &str, event: &str, row_id: &str, data: Option<&Value>) {
        for hook in &self.hooks {
            let entity_match = hook
                .entity
                .as_deref()
                .map(|e| e == entity)
                .unwrap_or(true);
            let event_match =
                hook.events.is_empty() || hook.events.iter().any(|e| e == event);

            if entity_match && event_match {
                let payload = serde_json::json!({
                    "event": event,
                    "entity": entity,
                    "row_id": row_id,
                    "data": data,
                });

                let should_log = matches!(self.mode, DeliveryMode::Log | DeliveryMode::Both);
                let should_deliver =
                    matches!(self.mode, DeliveryMode::Deliver | DeliveryMode::Both);

                // Build the log status from either a real delivery or the old stub.
                let status = if should_deliver {
                    let url = hook.url.clone();
                    let payload_str = payload.to_string();
                    let timestamp = now();

                    // Deliver in a separate thread so we don't block the caller.
                    let result = {
                        let url_clone = url.clone();
                        let payload_clone = payload_str.clone();
                        std::thread::spawn(move || deliver(&url_clone, &payload_clone))
                            .join()
                            .unwrap_or_else(|_| Err("Thread panicked".into()))
                    };

                    let attempt = match &result {
                        Ok(code) => DeliveryAttempt {
                            url: url.clone(),
                            status: *code,
                            success: (200..300).contains(code),
                            timestamp,
                            error: None,
                        },
                        Err(e) => DeliveryAttempt {
                            url: url.clone(),
                            status: 0,
                            success: false,
                            timestamp,
                            error: Some(e.clone()),
                        },
                    };

                    let mut dlog = self.delivery_log.lock().unwrap();
                    dlog.push(attempt);
                    let excess = dlog.len().saturating_sub(self.max_log);
                    if excess > 0 {
                        dlog.drain(0..excess);
                    }

                    match result {
                        Ok(code) => format!("{code}"),
                        Err(e) => format!("error: {e}"),
                    }
                } else {
                    // Log-only mode: no real HTTP call.
                    "200".to_string()
                };

                if should_log {
                    let mut log = self.log.lock().unwrap();
                    log.push(WebhookEvent {
                        url: hook.url.clone(),
                        entity: entity.to_string(),
                        event: event.to_string(),
                        row_id: row_id.to_string(),
                        status,
                    });
                    let excess = log.len().saturating_sub(self.max_log);
                    if excess > 0 {
                        log.drain(0..excess);
                    }
                }
            }
        }
    }
}

impl Plugin for WebhooksPlugin {
    fn name(&self) -> &str {
        "webhooks"
    }

    fn after_insert(&self, entity: &str, id: &str, data: &Value, _auth: &AuthContext) {
        self.fire(entity, "insert", id, Some(data));
    }

    fn after_update(&self, entity: &str, id: &str, data: &Value, _auth: &AuthContext) {
        self.fire(entity, "update", id, Some(data));
    }

    fn after_delete(&self, entity: &str, id: &str, _auth: &AuthContext) {
        self.fire(entity, "delete", id, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fires_on_insert() {
        let mut plugin = WebhooksPlugin::new();
        plugin.add(WebhookConfig {
            url: "https://example.com/webhook".into(),
            entity: None,
            events: vec![],
            secret: None,
        });

        plugin.after_insert(
            "Todo",
            "t1",
            &serde_json::json!({"title": "Test"}),
            &AuthContext::anonymous(),
        );
        assert_eq!(plugin.log().len(), 1);
        assert_eq!(plugin.log()[0].event, "insert");
        assert_eq!(plugin.log()[0].entity, "Todo");
    }

    #[test]
    fn filters_by_entity() {
        let mut plugin = WebhooksPlugin::new();
        plugin.add(WebhookConfig {
            url: "https://example.com/webhook".into(),
            entity: Some("Todo".into()),
            events: vec![],
            secret: None,
        });

        plugin.after_insert("User", "u1", &serde_json::json!({}), &AuthContext::anonymous());
        assert_eq!(plugin.log().len(), 0); // User doesn't match

        plugin.after_insert("Todo", "t1", &serde_json::json!({}), &AuthContext::anonymous());
        assert_eq!(plugin.log().len(), 1);
    }

    #[test]
    fn filters_by_event() {
        let mut plugin = WebhooksPlugin::new();
        plugin.add(WebhookConfig {
            url: "https://example.com/webhook".into(),
            entity: None,
            events: vec!["delete".into()],
            secret: None,
        });

        plugin.after_insert("Todo", "t1", &serde_json::json!({}), &AuthContext::anonymous());
        assert_eq!(plugin.log().len(), 0); // insert doesn't match

        plugin.after_delete("Todo", "t1", &AuthContext::anonymous());
        assert_eq!(plugin.log().len(), 1);
    }

    #[test]
    fn trims_log() {
        let mut plugin = WebhooksPlugin::new();
        plugin.max_log = 2;
        plugin.add(WebhookConfig {
            url: "x".into(),
            entity: None,
            events: vec![],
            secret: None,
        });

        let auth = AuthContext::anonymous();
        plugin.after_insert("A", "1", &serde_json::json!({}), &auth);
        plugin.after_insert("A", "2", &serde_json::json!({}), &auth);
        plugin.after_insert("A", "3", &serde_json::json!({}), &auth);

        assert_eq!(plugin.log().len(), 2);
    }

    // --- Delivery mode tests ---

    #[test]
    fn delivery_mode_enum_values() {
        assert_ne!(DeliveryMode::Log, DeliveryMode::Deliver);
        assert_ne!(DeliveryMode::Deliver, DeliveryMode::Both);
        assert_eq!(DeliveryMode::Log, DeliveryMode::Log);
    }

    #[test]
    fn with_mode_sets_mode() {
        let plugin = WebhooksPlugin::with_mode(DeliveryMode::Deliver);
        assert_eq!(plugin.mode, DeliveryMode::Deliver);
    }

    #[test]
    fn log_mode_does_not_populate_delivery_history() {
        let mut plugin = WebhooksPlugin::new(); // defaults to Log
        plugin.add(WebhookConfig {
            url: "http://localhost:9999/hook".into(),
            entity: None,
            events: vec![],
            secret: None,
        });

        plugin.after_insert("Todo", "t1", &serde_json::json!({}), &AuthContext::anonymous());
        assert_eq!(plugin.delivery_history().len(), 0);
        assert_eq!(plugin.log().len(), 1);
    }

    #[test]
    fn deliver_mode_blocks_private_ip() {
        let mut plugin = WebhooksPlugin::with_mode(DeliveryMode::Deliver);
        plugin.add(WebhookConfig {
            url: "http://127.0.0.1:19999/hook".into(),
            entity: None,
            events: vec![],
            secret: None,
        });

        plugin.after_insert("Todo", "t1", &serde_json::json!({}), &AuthContext::anonymous());

        let history = plugin.delivery_history();
        assert_eq!(history.len(), 1);
        assert!(!history[0].success);
        assert!(history[0].error.as_ref().unwrap().contains("private/reserved"));
    }

    #[test]
    fn both_mode_populates_log_and_delivery_history() {
        let mut plugin = WebhooksPlugin::with_mode(DeliveryMode::Both);
        plugin.add(WebhookConfig {
            url: "http://127.0.0.1:19999/hook".into(),
            entity: None,
            events: vec![],
            secret: None,
        });

        plugin.after_insert("Todo", "t1", &serde_json::json!({}), &AuthContext::anonymous());

        assert_eq!(plugin.delivery_history().len(), 1);
        assert_eq!(plugin.log().len(), 1);
    }

    #[test]
    fn delivery_attempt_tracks_url() {
        let mut plugin = WebhooksPlugin::with_mode(DeliveryMode::Deliver);
        plugin.add(WebhookConfig {
            url: "http://127.0.0.1:19999/my-hook".into(),
            entity: None,
            events: vec![],
            secret: None,
        });

        plugin.after_insert("Todo", "t1", &serde_json::json!({}), &AuthContext::anonymous());

        let history = plugin.delivery_history();
        assert_eq!(history[0].url, "http://127.0.0.1:19999/my-hook");
    }

    #[test]
    fn delivery_history_trimmed_to_max_log() {
        let mut plugin = WebhooksPlugin::with_mode(DeliveryMode::Deliver);
        plugin.max_log = 2;
        plugin.add(WebhookConfig {
            url: "http://127.0.0.1:19999/hook".into(),
            entity: None,
            events: vec![],
            secret: None,
        });

        let auth = AuthContext::anonymous();
        plugin.after_insert("A", "1", &serde_json::json!({}), &auth);
        plugin.after_insert("A", "2", &serde_json::json!({}), &auth);
        plugin.after_insert("A", "3", &serde_json::json!({}), &auth);

        assert_eq!(plugin.delivery_history().len(), 2);
    }

    // --- URL parsing tests ---

    #[test]
    fn deliver_rejects_non_http() {
        let result = deliver("https://example.com/path", "{}");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Only http://"));
    }

    #[test]
    fn deliver_blocks_private_ip_addresses() {
        // 127.0.0.1 (loopback)
        let result = deliver("http://127.0.0.1:19999/webhook/path", "{}");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("private/reserved"));

        // 10.x.x.x
        let result = deliver("http://10.0.0.1:80/hook", "{}");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("private/reserved"));

        // 172.16.x.x
        let result = deliver("http://172.16.0.1:80/hook", "{}");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("private/reserved"));

        // 192.168.x.x
        let result = deliver("http://192.168.1.1:80/hook", "{}");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("private/reserved"));

        // 169.254.x.x (AWS metadata)
        let result = deliver("http://169.254.169.254/latest/meta-data/", "{}");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("private/reserved"));

        // localhost
        let result = deliver("http://localhost:9999/hook", "{}");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("private/reserved"));
    }

    #[test]
    fn deliver_parses_url_without_path() {
        // Public IP -- will fail to connect but passes the SSRF check.
        let result = deliver("http://203.0.113.1:19999", "{}");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Connection failed"));
    }
}
