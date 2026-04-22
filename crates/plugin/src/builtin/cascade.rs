use std::sync::Mutex;

use crate::Plugin;
use statecraft_auth::AuthContext;

/// A cascade rule: when parent is deleted, delete children.
#[derive(Debug, Clone)]
pub struct CascadeRule {
    /// Parent entity name.
    pub parent: String,
    /// Child entity name.
    pub child: String,
    /// Foreign key field on the child that references the parent.
    pub foreign_key: String,
}

/// Cascade delete plugin. Automatically deletes child rows when a parent is deleted.
/// Queues deletions to be executed by the runtime.
pub struct CascadePlugin {
    rules: Vec<CascadeRule>,
    pending_deletes: Mutex<Vec<(String, String)>>, // (entity, id) pairs to delete
}

impl CascadePlugin {
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            pending_deletes: Mutex::new(Vec::new()),
        }
    }

    /// Add a cascade rule.
    pub fn add_rule(&mut self, parent: &str, child: &str, foreign_key: &str) {
        self.rules.push(CascadeRule {
            parent: parent.to_string(),
            child: child.to_string(),
            foreign_key: foreign_key.to_string(),
        });
    }

    /// Get pending cascade deletions (the runtime should execute these).
    pub fn take_pending(&self) -> Vec<(String, String)> {
        let mut pending = self.pending_deletes.lock().unwrap();
        let items = pending.clone();
        pending.clear();
        items
    }

    /// Get cascade rules for an entity.
    pub fn rules_for(&self, parent: &str) -> Vec<&CascadeRule> {
        self.rules.iter().filter(|r| r.parent == parent).collect()
    }
}

impl Plugin for CascadePlugin {
    fn name(&self) -> &str {
        "cascade-delete"
    }

    fn after_delete(&self, entity: &str, id: &str, _auth: &AuthContext) {
        // When a parent is deleted, queue child deletions.
        let rules = self.rules_for(entity);
        if !rules.is_empty() {
            let mut pending = self.pending_deletes.lock().unwrap();
            for rule in rules {
                // Queue a "find and delete children" marker.
                // The runtime needs to: SELECT id FROM child WHERE foreign_key = parent_id, then DELETE each.
                pending.push((rule.child.clone(), format!("__cascade__{}={}", rule.foreign_key, id)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queues_cascade_on_delete() {
        let mut plugin = CascadePlugin::new();
        plugin.add_rule("User", "Todo", "authorId");

        plugin.after_delete("User", "u1", &AuthContext::anonymous());

        let pending = plugin.take_pending();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].0, "Todo");
        assert!(pending[0].1.contains("authorId=u1"));
    }

    #[test]
    fn no_rules_no_cascade() {
        let plugin = CascadePlugin::new();
        plugin.after_delete("User", "u1", &AuthContext::anonymous());
        assert!(plugin.take_pending().is_empty());
    }

    #[test]
    fn multiple_children() {
        let mut plugin = CascadePlugin::new();
        plugin.add_rule("User", "Todo", "authorId");
        plugin.add_rule("User", "Comment", "userId");

        plugin.after_delete("User", "u1", &AuthContext::anonymous());

        let pending = plugin.take_pending();
        assert_eq!(pending.len(), 2);
    }

    #[test]
    fn take_clears_pending() {
        let mut plugin = CascadePlugin::new();
        plugin.add_rule("User", "Todo", "authorId");

        plugin.after_delete("User", "u1", &AuthContext::anonymous());
        let first = plugin.take_pending();
        assert_eq!(first.len(), 1);

        let second = plugin.take_pending();
        assert!(second.is_empty());
    }

    #[test]
    fn unrelated_entity_no_cascade() {
        let mut plugin = CascadePlugin::new();
        plugin.add_rule("User", "Todo", "authorId");

        plugin.after_delete("Post", "p1", &AuthContext::anonymous());
        assert!(plugin.take_pending().is_empty());
    }
}
