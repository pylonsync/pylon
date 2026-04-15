use std::collections::HashMap;
use std::sync::Mutex;

use crate::Plugin;
use agentdb_auth::AuthContext;
use serde_json::Value;

/// A versioned snapshot of a row.
#[derive(Debug, Clone)]
pub struct RowVersion {
    pub entity: String,
    pub row_id: String,
    pub version: u64,
    pub data: Value,
    pub changed_by: Option<String>,
    pub changed_at: String,
}

/// Versioning plugin. Keeps a history of all row changes for undo/audit.
pub struct VersioningPlugin {
    /// Map of "entity:row_id" -> list of versions.
    history: Mutex<HashMap<String, Vec<RowVersion>>>,
    /// Max versions to keep per row. 0 = unlimited.
    max_versions: usize,
}

impl VersioningPlugin {
    pub fn new(max_versions: usize) -> Self {
        Self {
            history: Mutex::new(HashMap::new()),
            max_versions,
        }
    }

    /// Get version history for a row.
    pub fn get_history(&self, entity: &str, row_id: &str) -> Vec<RowVersion> {
        let key = format!("{entity}:{row_id}");
        self.history
            .lock()
            .unwrap()
            .get(&key)
            .cloned()
            .unwrap_or_default()
    }

    /// Get a specific version of a row.
    pub fn get_version(&self, entity: &str, row_id: &str, version: u64) -> Option<RowVersion> {
        self.get_history(entity, row_id)
            .into_iter()
            .find(|v| v.version == version)
    }

    /// Get the latest version number for a row.
    pub fn latest_version(&self, entity: &str, row_id: &str) -> u64 {
        self.get_history(entity, row_id)
            .last()
            .map(|v| v.version)
            .unwrap_or(0)
    }

    fn record(&self, entity: &str, row_id: &str, data: &Value, auth: &AuthContext) {
        let key = format!("{entity}:{row_id}");
        let mut history = self.history.lock().unwrap();
        let versions = history.entry(key).or_default();

        let version = versions.last().map(|v| v.version + 1).unwrap_or(1);
        versions.push(RowVersion {
            entity: entity.to_string(),
            row_id: row_id.to_string(),
            version,
            data: data.clone(),
            changed_by: auth.user_id.clone(),
            changed_at: now(),
        });

        // Trim if over max.
        if self.max_versions > 0 && versions.len() > self.max_versions {
            let excess = versions.len() - self.max_versions;
            versions.drain(0..excess);
        }
    }
}

impl Plugin for VersioningPlugin {
    fn name(&self) -> &str {
        "versioning"
    }

    fn after_insert(&self, entity: &str, id: &str, data: &Value, auth: &AuthContext) {
        self.record(entity, id, data, auth);
    }

    fn after_update(&self, entity: &str, id: &str, data: &Value, auth: &AuthContext) {
        self.record(entity, id, data, auth);
    }
}

fn now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    format!(
        "{}Z",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_insert() {
        let plugin = VersioningPlugin::new(0);
        let auth = AuthContext::authenticated("user-1".into());
        plugin.after_insert("Todo", "t1", &serde_json::json!({"title": "V1"}), &auth);

        let history = plugin.get_history("Todo", "t1");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].version, 1);
        assert_eq!(history[0].changed_by, Some("user-1".into()));
    }

    #[test]
    fn records_updates() {
        let plugin = VersioningPlugin::new(0);
        let auth = AuthContext::authenticated("user-1".into());
        plugin.after_insert("Todo", "t1", &serde_json::json!({"title": "V1"}), &auth);
        plugin.after_update("Todo", "t1", &serde_json::json!({"title": "V2"}), &auth);
        plugin.after_update("Todo", "t1", &serde_json::json!({"title": "V3"}), &auth);

        let history = plugin.get_history("Todo", "t1");
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].version, 1);
        assert_eq!(history[2].version, 3);
    }

    #[test]
    fn get_specific_version() {
        let plugin = VersioningPlugin::new(0);
        let auth = AuthContext::anonymous();
        plugin.after_insert("Todo", "t1", &serde_json::json!({"title": "V1"}), &auth);
        plugin.after_update("Todo", "t1", &serde_json::json!({"title": "V2"}), &auth);

        let v1 = plugin.get_version("Todo", "t1", 1).unwrap();
        assert_eq!(v1.data["title"], "V1");

        let v2 = plugin.get_version("Todo", "t1", 2).unwrap();
        assert_eq!(v2.data["title"], "V2");

        assert!(plugin.get_version("Todo", "t1", 99).is_none());
    }

    #[test]
    fn latest_version() {
        let plugin = VersioningPlugin::new(0);
        let auth = AuthContext::anonymous();
        assert_eq!(plugin.latest_version("Todo", "t1"), 0);

        plugin.after_insert("Todo", "t1", &serde_json::json!({}), &auth);
        assert_eq!(plugin.latest_version("Todo", "t1"), 1);

        plugin.after_update("Todo", "t1", &serde_json::json!({}), &auth);
        assert_eq!(plugin.latest_version("Todo", "t1"), 2);
    }

    #[test]
    fn max_versions_trims() {
        let plugin = VersioningPlugin::new(2);
        let auth = AuthContext::anonymous();
        plugin.after_insert("Todo", "t1", &serde_json::json!({"v": 1}), &auth);
        plugin.after_update("Todo", "t1", &serde_json::json!({"v": 2}), &auth);
        plugin.after_update("Todo", "t1", &serde_json::json!({"v": 3}), &auth);

        let history = plugin.get_history("Todo", "t1");
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].data["v"], 2); // V1 trimmed
        assert_eq!(history[1].data["v"], 3);
    }

    #[test]
    fn separate_rows_separate_history() {
        let plugin = VersioningPlugin::new(0);
        let auth = AuthContext::anonymous();
        plugin.after_insert("Todo", "t1", &serde_json::json!({"title": "A"}), &auth);
        plugin.after_insert("Todo", "t2", &serde_json::json!({"title": "B"}), &auth);

        assert_eq!(plugin.get_history("Todo", "t1").len(), 1);
        assert_eq!(plugin.get_history("Todo", "t2").len(), 1);
    }
}
