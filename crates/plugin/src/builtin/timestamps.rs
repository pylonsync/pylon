use crate::{Plugin, PluginError};
use pylon_auth::AuthContext;
use serde_json::Value;

/// Timestamps plugin. Auto-sets `createdAt` on insert and `updatedAt` on update.
pub struct TimestampsPlugin {
    pub created_field: String,
    pub updated_field: String,
}

impl TimestampsPlugin {
    pub fn new() -> Self {
        Self {
            created_field: "createdAt".into(),
            updated_field: "updatedAt".into(),
        }
    }

    pub fn with_fields(created: &str, updated: &str) -> Self {
        Self {
            created_field: created.into(),
            updated_field: updated.into(),
        }
    }
}

fn now_iso() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let secs_per_day: u64 = 86400;
    let days = ts / secs_per_day;
    let rem = ts % secs_per_day;
    let h = rem / 3600;
    let m = (rem % 3600) / 60;
    let s = rem % 60;
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let yr = if mo <= 2 { y + 1 } else { y };
    format!("{yr:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

impl Plugin for TimestampsPlugin {
    fn name(&self) -> &str {
        "timestamps"
    }

    fn before_insert(
        &self,
        _entity: &str,
        data: &mut Value,
        _auth: &AuthContext,
    ) -> Result<(), PluginError> {
        if let Some(obj) = data.as_object_mut() {
            let now = now_iso();
            obj.entry(&self.created_field)
                .or_insert(Value::String(now.clone()));
            obj.entry(&self.updated_field).or_insert(Value::String(now));
        }
        Ok(())
    }

    fn before_update(
        &self,
        _entity: &str,
        _id: &str,
        data: &mut Value,
        _auth: &AuthContext,
    ) -> Result<(), PluginError> {
        if let Some(obj) = data.as_object_mut() {
            obj.insert(self.updated_field.clone(), Value::String(now_iso()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_created_at_on_insert() {
        let plugin = TimestampsPlugin::new();
        let mut data = serde_json::json!({"title": "Test"});
        plugin
            .before_insert("Todo", &mut data, &AuthContext::anonymous())
            .unwrap();
        assert!(data.get("createdAt").is_some());
        assert!(data.get("updatedAt").is_some());
    }

    #[test]
    fn does_not_overwrite_existing_created_at() {
        let plugin = TimestampsPlugin::new();
        let mut data = serde_json::json!({"title": "Test", "createdAt": "2020-01-01"});
        plugin
            .before_insert("Todo", &mut data, &AuthContext::anonymous())
            .unwrap();
        assert_eq!(data["createdAt"], "2020-01-01");
    }

    #[test]
    fn sets_updated_at_on_update() {
        let plugin = TimestampsPlugin::new();
        let mut data = serde_json::json!({"title": "Updated"});
        plugin
            .before_update("Todo", "t1", &mut data, &AuthContext::anonymous())
            .unwrap();
        assert!(data.get("updatedAt").is_some());
    }

    #[test]
    fn custom_field_names() {
        let plugin = TimestampsPlugin::with_fields("created", "modified");
        let mut data = serde_json::json!({"title": "Test"});
        plugin
            .before_insert("Todo", &mut data, &AuthContext::anonymous())
            .unwrap();
        assert!(data.get("created").is_some());
        assert!(data.get("modified").is_some());
    }
}
