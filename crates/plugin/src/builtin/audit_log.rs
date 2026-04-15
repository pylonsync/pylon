use std::sync::Mutex;

use crate::Plugin;
use agentdb_auth::AuthContext;
use serde_json::Value;

/// An audit log entry.
#[derive(Debug, Clone)]
pub struct AuditEntry {
    pub timestamp: String,
    pub user_id: Option<String>,
    pub action: String,
    pub entity: String,
    pub row_id: String,
    pub data: Option<Value>,
}

/// Audit log plugin. Records all mutations for compliance/debugging.
pub struct AuditLogPlugin {
    entries: Mutex<Vec<AuditEntry>>,
    max_entries: usize,
}

impl AuditLogPlugin {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
            max_entries,
        }
    }

    pub fn entries(&self) -> Vec<AuditEntry> {
        self.entries.lock().unwrap().clone()
    }

    pub fn len(&self) -> usize {
        self.entries.lock().unwrap().len()
    }

    fn record(&self, action: &str, entity: &str, row_id: &str, data: Option<&Value>, auth: &AuthContext) {
        let entry = AuditEntry {
            timestamp: now(),
            user_id: auth.user_id.clone(),
            action: action.to_string(),
            entity: entity.to_string(),
            row_id: row_id.to_string(),
            data: data.cloned(),
        };

        let mut entries = self.entries.lock().unwrap();
        entries.push(entry);

        // Trim if over max.
        if entries.len() > self.max_entries {
            let excess = entries.len() - self.max_entries;
            entries.drain(0..excess);
        }
    }
}

impl Plugin for AuditLogPlugin {
    fn name(&self) -> &str {
        "audit-log"
    }

    fn after_insert(&self, entity: &str, id: &str, data: &Value, auth: &AuthContext) {
        self.record("insert", entity, id, Some(data), auth);
    }

    fn after_update(&self, entity: &str, id: &str, data: &Value, auth: &AuthContext) {
        self.record("update", entity, id, Some(data), auth);
    }

    fn after_delete(&self, entity: &str, id: &str, auth: &AuthContext) {
        self.record("delete", entity, id, None, auth);
    }
}

fn now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{ts}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_insert() {
        let plugin = AuditLogPlugin::new(100);
        let auth = AuthContext::authenticated("user-1".into());
        let data = serde_json::json!({"title": "Test"});
        plugin.after_insert("Todo", "t1", &data, &auth);

        let entries = plugin.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action, "insert");
        assert_eq!(entries[0].entity, "Todo");
        assert_eq!(entries[0].row_id, "t1");
        assert_eq!(entries[0].user_id, Some("user-1".into()));
    }

    #[test]
    fn records_update_and_delete() {
        let plugin = AuditLogPlugin::new(100);
        let auth = AuthContext::authenticated("user-1".into());
        plugin.after_update("Todo", "t1", &serde_json::json!({"done": true}), &auth);
        plugin.after_delete("Todo", "t1", &auth);

        assert_eq!(plugin.len(), 2);
        let entries = plugin.entries();
        assert_eq!(entries[0].action, "update");
        assert_eq!(entries[1].action, "delete");
        assert!(entries[1].data.is_none());
    }

    #[test]
    fn trims_over_max() {
        let plugin = AuditLogPlugin::new(2);
        let auth = AuthContext::anonymous();
        let data = serde_json::json!({});
        plugin.after_insert("A", "1", &data, &auth);
        plugin.after_insert("A", "2", &data, &auth);
        plugin.after_insert("A", "3", &data, &auth);

        assert_eq!(plugin.len(), 2);
        let entries = plugin.entries();
        assert_eq!(entries[0].row_id, "2"); // oldest trimmed
        assert_eq!(entries[1].row_id, "3");
    }

    #[test]
    fn anonymous_audit() {
        let plugin = AuditLogPlugin::new(100);
        let auth = AuthContext::anonymous();
        plugin.after_insert("Todo", "t1", &serde_json::json!({}), &auth);
        assert_eq!(plugin.entries()[0].user_id, None);
    }
}
