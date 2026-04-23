use std::collections::HashMap;
use std::sync::Mutex;

use crate::Plugin;
use pylon_auth::AuthContext;
use serde_json::Value;

/// A search index entry.
#[derive(Debug, Clone)]
struct IndexEntry {
    entity: String,
    row_id: String,
    text: String,
    data: Value,
}

/// Search configuration for an entity.
pub struct SearchConfig {
    /// Fields to index for full-text search.
    pub fields: Vec<String>,
}

/// Full-text search plugin. Maintains an in-memory inverted index.
pub struct SearchPlugin {
    configs: HashMap<String, SearchConfig>,
    index: Mutex<Vec<IndexEntry>>,
}

impl SearchPlugin {
    pub fn new() -> Self {
        Self {
            configs: HashMap::new(),
            index: Mutex::new(Vec::new()),
        }
    }

    /// Register an entity and its searchable fields.
    pub fn add(&mut self, entity: &str, fields: Vec<String>) {
        self.configs
            .insert(entity.to_string(), SearchConfig { fields });
    }

    /// Search across all indexed entities. Returns matching rows.
    pub fn search(&self, query: &str) -> Vec<SearchResult> {
        let query_lower = query.to_lowercase();
        let terms: Vec<&str> = query_lower.split_whitespace().collect();

        let index = self.index.lock().unwrap();
        let results: Vec<SearchResult> = index
            .iter()
            .filter(|entry| {
                let text_lower = entry.text.to_lowercase();
                terms.iter().all(|term| text_lower.contains(term))
            })
            .map(|entry| SearchResult {
                entity: entry.entity.clone(),
                row_id: entry.row_id.clone(),
                data: entry.data.clone(),
            })
            .collect();

        results
    }

    fn index_row(&self, entity: &str, row_id: &str, data: &Value) {
        if let Some(config) = self.configs.get(entity) {
            let mut text_parts = Vec::new();
            if let Some(obj) = data.as_object() {
                for field in &config.fields {
                    if let Some(val) = obj.get(field).and_then(|v| v.as_str()) {
                        text_parts.push(val.to_string());
                    }
                }
            }

            if !text_parts.is_empty() {
                let mut index = self.index.lock().unwrap();
                // Remove existing entry for this row.
                index.retain(|e| !(e.entity == entity && e.row_id == row_id));
                // Add new entry.
                index.push(IndexEntry {
                    entity: entity.to_string(),
                    row_id: row_id.to_string(),
                    text: text_parts.join(" "),
                    data: data.clone(),
                });
            }
        }
    }

    fn remove_from_index(&self, entity: &str, row_id: &str) {
        let mut index = self.index.lock().unwrap();
        index.retain(|e| !(e.entity == entity && e.row_id == row_id));
    }
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub entity: String,
    pub row_id: String,
    pub data: Value,
}

impl Plugin for SearchPlugin {
    fn name(&self) -> &str {
        "search"
    }

    fn after_insert(&self, entity: &str, id: &str, data: &Value, _auth: &AuthContext) {
        self.index_row(entity, id, data);
    }

    fn after_update(&self, entity: &str, id: &str, data: &Value, _auth: &AuthContext) {
        self.index_row(entity, id, data);
    }

    fn after_delete(&self, entity: &str, id: &str, _auth: &AuthContext) {
        self.remove_from_index(entity, id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_finds_matching_text() {
        let mut plugin = SearchPlugin::new();
        plugin.add("Todo", vec!["title".into()]);

        plugin.after_insert(
            "Todo",
            "t1",
            &serde_json::json!({"title": "Buy milk"}),
            &AuthContext::anonymous(),
        );
        plugin.after_insert(
            "Todo",
            "t2",
            &serde_json::json!({"title": "Buy bread"}),
            &AuthContext::anonymous(),
        );
        plugin.after_insert(
            "Todo",
            "t3",
            &serde_json::json!({"title": "Walk the dog"}),
            &AuthContext::anonymous(),
        );

        let results = plugin.search("buy");
        assert_eq!(results.len(), 2);

        let results = plugin.search("milk");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].row_id, "t1");
    }

    #[test]
    fn search_multiple_terms() {
        let mut plugin = SearchPlugin::new();
        plugin.add("Todo", vec!["title".into()]);

        plugin.after_insert(
            "Todo",
            "t1",
            &serde_json::json!({"title": "Buy organic milk"}),
            &AuthContext::anonymous(),
        );
        plugin.after_insert(
            "Todo",
            "t2",
            &serde_json::json!({"title": "Buy regular milk"}),
            &AuthContext::anonymous(),
        );

        let results = plugin.search("organic milk");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].row_id, "t1");
    }

    #[test]
    fn search_case_insensitive() {
        let mut plugin = SearchPlugin::new();
        plugin.add("Todo", vec!["title".into()]);

        plugin.after_insert(
            "Todo",
            "t1",
            &serde_json::json!({"title": "IMPORTANT TASK"}),
            &AuthContext::anonymous(),
        );

        let results = plugin.search("important");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn search_updates_index_on_update() {
        let mut plugin = SearchPlugin::new();
        plugin.add("Todo", vec!["title".into()]);

        plugin.after_insert(
            "Todo",
            "t1",
            &serde_json::json!({"title": "Old title"}),
            &AuthContext::anonymous(),
        );
        plugin.after_update(
            "Todo",
            "t1",
            &serde_json::json!({"title": "New title"}),
            &AuthContext::anonymous(),
        );

        let results = plugin.search("old");
        assert_eq!(results.len(), 0);

        let results = plugin.search("new");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn search_removes_on_delete() {
        let mut plugin = SearchPlugin::new();
        plugin.add("Todo", vec!["title".into()]);

        plugin.after_insert(
            "Todo",
            "t1",
            &serde_json::json!({"title": "Deletable"}),
            &AuthContext::anonymous(),
        );
        plugin.after_delete("Todo", "t1", &AuthContext::anonymous());

        let results = plugin.search("deletable");
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn search_multiple_fields() {
        let mut plugin = SearchPlugin::new();
        plugin.add("User", vec!["displayName".into(), "email".into()]);

        plugin.after_insert(
            "User",
            "u1",
            &serde_json::json!({"displayName": "Alice", "email": "alice@test.com"}),
            &AuthContext::anonymous(),
        );

        let results = plugin.search("alice");
        assert_eq!(results.len(), 1);

        let results = plugin.search("test.com");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn search_no_config_no_index() {
        let plugin = SearchPlugin::new();
        plugin.after_insert(
            "Todo",
            "t1",
            &serde_json::json!({"title": "Test"}),
            &AuthContext::anonymous(),
        );
        let results = plugin.search("test");
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn search_empty_query() {
        let mut plugin = SearchPlugin::new();
        plugin.add("Todo", vec!["title".into()]);
        plugin.after_insert(
            "Todo",
            "t1",
            &serde_json::json!({"title": "Test"}),
            &AuthContext::anonymous(),
        );

        let results = plugin.search("");
        assert_eq!(results.len(), 1); // empty query matches all
    }
}
