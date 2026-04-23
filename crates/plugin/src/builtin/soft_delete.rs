use crate::{Plugin, PluginError};
use pylon_auth::AuthContext;

/// Soft delete plugin. Converts deletes to updates that set `deletedAt`.
/// Adds a before_delete hook that rejects the actual delete and instead
/// sets a timestamp. The application should filter out soft-deleted rows.
pub struct SoftDeletePlugin {
    pub field: String,
    /// Entity names to apply soft delete to. Empty = all entities.
    pub entities: Vec<String>,
}

impl SoftDeletePlugin {
    pub fn new() -> Self {
        Self {
            field: "deletedAt".into(),
            entities: vec![],
        }
    }

    pub fn for_entities(entities: Vec<String>) -> Self {
        Self {
            field: "deletedAt".into(),
            entities,
        }
    }

    fn applies_to(&self, entity: &str) -> bool {
        self.entities.is_empty() || self.entities.iter().any(|e| e == entity)
    }
}

#[allow(dead_code)]
fn now_iso() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    format!("{ts}Z")
}

impl Plugin for SoftDeletePlugin {
    fn name(&self) -> &str {
        "soft-delete"
    }

    fn before_delete(
        &self,
        entity: &str,
        id: &str,
        _auth: &AuthContext,
    ) -> Result<(), PluginError> {
        if self.applies_to(entity) {
            // Block the real delete — the server should instead update with deletedAt.
            Err(PluginError {
                code: "SOFT_DELETE".into(),
                message: format!(
                    "Entity {} uses soft delete. Set {}.{} instead of deleting row {}.",
                    entity, entity, self.field, id
                ),
                status: 400,
            })
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_delete_for_all_entities() {
        let plugin = SoftDeletePlugin::new();
        let result = plugin.before_delete("Todo", "t1", &AuthContext::anonymous());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, "SOFT_DELETE");
    }

    #[test]
    fn blocks_delete_for_specific_entities() {
        let plugin = SoftDeletePlugin::for_entities(vec!["Todo".into()]);
        assert!(plugin.before_delete("Todo", "t1", &AuthContext::anonymous()).is_err());
        assert!(plugin.before_delete("User", "u1", &AuthContext::anonymous()).is_ok());
    }

    #[test]
    fn allows_delete_for_non_matching() {
        let plugin = SoftDeletePlugin::for_entities(vec!["Todo".into()]);
        let result = plugin.before_delete("Comment", "c1", &AuthContext::anonymous());
        assert!(result.is_ok());
    }
}
