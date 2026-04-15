use std::collections::HashMap;

use crate::Plugin;
use agentdb_auth::AuthContext;
use serde_json::Value;

/// A computed field definition.
pub type ComputeFn = Box<dyn Fn(&Value) -> Value + Send + Sync>;

/// Computed fields plugin. Auto-derives fields on read based on other fields.
/// Example: fullName = firstName + " " + lastName
pub struct ComputedFieldsPlugin {
    /// Map of entity -> field_name -> compute function.
    fields: HashMap<String, Vec<(String, ComputeFn)>>,
}

impl ComputedFieldsPlugin {
    pub fn new() -> Self {
        Self {
            fields: HashMap::new(),
        }
    }

    /// Add a computed field.
    pub fn add<F>(&mut self, entity: &str, field_name: &str, compute: F)
    where
        F: Fn(&Value) -> Value + Send + Sync + 'static,
    {
        self.fields
            .entry(entity.to_string())
            .or_default()
            .push((field_name.to_string(), Box::new(compute)));
    }

    /// Apply computed fields to a row.
    pub fn apply(&self, entity: &str, row: &mut Value) {
        if let Some(fields) = self.fields.get(entity) {
            if let Some(obj) = row.as_object_mut() {
                for (name, compute) in fields {
                    let value = compute(&Value::Object(obj.clone()));
                    obj.insert(name.clone(), value);
                }
            }
        }
    }

    /// Apply computed fields to a list of rows.
    pub fn apply_all(&self, entity: &str, rows: &mut [Value]) {
        for row in rows {
            self.apply(entity, row);
        }
    }
}

impl Plugin for ComputedFieldsPlugin {
    fn name(&self) -> &str {
        "computed-fields"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_computed_field() {
        let mut plugin = ComputedFieldsPlugin::new();
        plugin.add("User", "fullName", |row| {
            let first = row.get("firstName").and_then(|v| v.as_str()).unwrap_or("");
            let last = row.get("lastName").and_then(|v| v.as_str()).unwrap_or("");
            Value::String(format!("{first} {last}").trim().to_string())
        });

        let mut row = serde_json::json!({"firstName": "Alice", "lastName": "Smith"});
        plugin.apply("User", &mut row);
        assert_eq!(row["fullName"], "Alice Smith");
    }

    #[test]
    fn computed_field_from_numeric() {
        let mut plugin = ComputedFieldsPlugin::new();
        plugin.add("Product", "priceFormatted", |row| {
            let price = row.get("price").and_then(|v| v.as_f64()).unwrap_or(0.0);
            Value::String(format!("${:.2}", price))
        });

        let mut row = serde_json::json!({"price": 29.99});
        plugin.apply("Product", &mut row);
        assert_eq!(row["priceFormatted"], "$29.99");
    }

    #[test]
    fn no_config_no_change() {
        let plugin = ComputedFieldsPlugin::new();
        let mut row = serde_json::json!({"name": "Alice"});
        plugin.apply("User", &mut row);
        assert!(row.get("fullName").is_none());
    }

    #[test]
    fn apply_all_rows() {
        let mut plugin = ComputedFieldsPlugin::new();
        plugin.add("User", "upper", |row| {
            let name = row.get("name").and_then(|v| v.as_str()).unwrap_or("");
            Value::String(name.to_uppercase())
        });

        let mut rows = vec![
            serde_json::json!({"name": "alice"}),
            serde_json::json!({"name": "bob"}),
        ];
        plugin.apply_all("User", &mut rows);
        assert_eq!(rows[0]["upper"], "ALICE");
        assert_eq!(rows[1]["upper"], "BOB");
    }

    #[test]
    fn multiple_computed_fields() {
        let mut plugin = ComputedFieldsPlugin::new();
        plugin.add("User", "initials", |row| {
            let first = row.get("firstName").and_then(|v| v.as_str()).unwrap_or("");
            let last = row.get("lastName").and_then(|v| v.as_str()).unwrap_or("");
            let i = format!("{}{}", first.chars().next().unwrap_or(' '), last.chars().next().unwrap_or(' '));
            Value::String(i.trim().to_string())
        });
        plugin.add("User", "emailDomain", |row| {
            let email = row.get("email").and_then(|v| v.as_str()).unwrap_or("");
            let domain = email.split('@').nth(1).unwrap_or("");
            Value::String(domain.to_string())
        });

        let mut row = serde_json::json!({"firstName": "Alice", "lastName": "Smith", "email": "alice@example.com"});
        plugin.apply("User", &mut row);
        assert_eq!(row["initials"], "AS");
        assert_eq!(row["emailDomain"], "example.com");
    }
}
