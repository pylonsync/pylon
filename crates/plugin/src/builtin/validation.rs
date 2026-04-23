use std::collections::HashMap;

use crate::{Plugin, PluginError};
use pylon_auth::AuthContext;
use serde_json::Value;

/// A field validation rule.
pub enum FieldRule {
    /// Minimum string length.
    MinLength(usize),
    /// Maximum string length.
    MaxLength(usize),
    /// Must match regex pattern (simple contains check).
    Pattern(String),
    /// Must be a valid email (contains @ and .).
    Email,
    /// Numeric minimum.
    Min(f64),
    /// Numeric maximum.
    Max(f64),
    /// Must not be empty string.
    NotEmpty,
    /// Custom validation function.
    Custom(
        String,
        Box<dyn Fn(&Value) -> Result<(), String> + Send + Sync>,
    ),
}

/// Validation rule for an entity.field.
pub struct EntityRules {
    pub rules: HashMap<String, Vec<FieldRule>>,
}

/// Validation plugin. Validates field values on insert and update.
pub struct ValidationPlugin {
    entity_rules: HashMap<String, EntityRules>,
}

impl ValidationPlugin {
    pub fn new() -> Self {
        Self {
            entity_rules: HashMap::new(),
        }
    }

    /// Add a rule for entity.field.
    pub fn add_rule(&mut self, entity: &str, field: &str, rule: FieldRule) {
        let entity_rules = self
            .entity_rules
            .entry(entity.to_string())
            .or_insert_with(|| EntityRules {
                rules: HashMap::new(),
            });
        entity_rules
            .rules
            .entry(field.to_string())
            .or_default()
            .push(rule);
    }

    fn validate_data(&self, entity: &str, data: &Value) -> Result<(), PluginError> {
        let rules = match self.entity_rules.get(entity) {
            Some(r) => r,
            None => return Ok(()),
        };

        let obj = match data.as_object() {
            Some(o) => o,
            None => return Ok(()),
        };

        for (field_name, field_rules) in &rules.rules {
            if let Some(value) = obj.get(field_name) {
                for rule in field_rules {
                    if let Err(msg) = validate_value(value, rule) {
                        return Err(PluginError {
                            code: "VALIDATION_FAILED".into(),
                            message: format!("{}.{}: {}", entity, field_name, msg),
                            status: 400,
                        });
                    }
                }
            }
        }

        Ok(())
    }
}

fn validate_value(value: &Value, rule: &FieldRule) -> Result<(), String> {
    match rule {
        FieldRule::MinLength(min) => {
            if let Some(s) = value.as_str() {
                if s.len() < *min {
                    return Err(format!("must be at least {} characters", min));
                }
            }
        }
        FieldRule::MaxLength(max) => {
            if let Some(s) = value.as_str() {
                if s.len() > *max {
                    return Err(format!("must be at most {} characters", max));
                }
            }
        }
        FieldRule::Pattern(pattern) => {
            if let Some(s) = value.as_str() {
                if !s.contains(pattern.as_str()) {
                    return Err(format!("must match pattern: {}", pattern));
                }
            }
        }
        FieldRule::Email => {
            if let Some(s) = value.as_str() {
                if !s.contains('@') || !s.contains('.') {
                    return Err("must be a valid email address".into());
                }
            }
        }
        FieldRule::Min(min) => {
            if let Some(n) = value.as_f64() {
                if n < *min {
                    return Err(format!("must be at least {}", min));
                }
            }
        }
        FieldRule::Max(max) => {
            if let Some(n) = value.as_f64() {
                if n > *max {
                    return Err(format!("must be at most {}", max));
                }
            }
        }
        FieldRule::NotEmpty => {
            if let Some(s) = value.as_str() {
                if s.trim().is_empty() {
                    return Err("must not be empty".into());
                }
            }
        }
        FieldRule::Custom(_name, validator) => {
            validator(value)?;
        }
    }
    Ok(())
}

impl Plugin for ValidationPlugin {
    fn name(&self) -> &str {
        "validation"
    }

    fn before_insert(
        &self,
        entity: &str,
        data: &mut Value,
        _auth: &AuthContext,
    ) -> Result<(), PluginError> {
        self.validate_data(entity, data)
    }

    fn before_update(
        &self,
        entity: &str,
        _id: &str,
        data: &mut Value,
        _auth: &AuthContext,
    ) -> Result<(), PluginError> {
        self.validate_data(entity, data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn min_length() {
        let mut plugin = ValidationPlugin::new();
        plugin.add_rule("User", "displayName", FieldRule::MinLength(3));

        let mut data = serde_json::json!({"displayName": "AB"});
        let result = plugin.before_insert("User", &mut data, &AuthContext::anonymous());
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("at least 3"));

        let mut data = serde_json::json!({"displayName": "Alice"});
        assert!(plugin
            .before_insert("User", &mut data, &AuthContext::anonymous())
            .is_ok());
    }

    #[test]
    fn max_length() {
        let mut plugin = ValidationPlugin::new();
        plugin.add_rule("User", "email", FieldRule::MaxLength(50));

        let mut data = serde_json::json!({"email": "a".repeat(51)});
        assert!(plugin
            .before_insert("User", &mut data, &AuthContext::anonymous())
            .is_err());
    }

    #[test]
    fn email_validation() {
        let mut plugin = ValidationPlugin::new();
        plugin.add_rule("User", "email", FieldRule::Email);

        let mut bad = serde_json::json!({"email": "notanemail"});
        assert!(plugin
            .before_insert("User", &mut bad, &AuthContext::anonymous())
            .is_err());

        let mut good = serde_json::json!({"email": "alice@example.com"});
        assert!(plugin
            .before_insert("User", &mut good, &AuthContext::anonymous())
            .is_ok());
    }

    #[test]
    fn not_empty() {
        let mut plugin = ValidationPlugin::new();
        plugin.add_rule("Todo", "title", FieldRule::NotEmpty);

        let mut empty = serde_json::json!({"title": "  "});
        assert!(plugin
            .before_insert("Todo", &mut empty, &AuthContext::anonymous())
            .is_err());

        let mut valid = serde_json::json!({"title": "Buy milk"});
        assert!(plugin
            .before_insert("Todo", &mut valid, &AuthContext::anonymous())
            .is_ok());
    }

    #[test]
    fn numeric_min_max() {
        let mut plugin = ValidationPlugin::new();
        plugin.add_rule("Product", "price", FieldRule::Min(0.0));
        plugin.add_rule("Product", "price", FieldRule::Max(10000.0));

        let mut negative = serde_json::json!({"price": -5});
        assert!(plugin
            .before_insert("Product", &mut negative, &AuthContext::anonymous())
            .is_err());

        let mut too_high = serde_json::json!({"price": 99999});
        assert!(plugin
            .before_insert("Product", &mut too_high, &AuthContext::anonymous())
            .is_err());

        let mut valid = serde_json::json!({"price": 29.99});
        assert!(plugin
            .before_insert("Product", &mut valid, &AuthContext::anonymous())
            .is_ok());
    }

    #[test]
    fn pattern_match() {
        let mut plugin = ValidationPlugin::new();
        plugin.add_rule("User", "website", FieldRule::Pattern("https://".into()));

        let mut bad = serde_json::json!({"website": "http://example.com"});
        assert!(plugin
            .before_insert("User", &mut bad, &AuthContext::anonymous())
            .is_err());

        let mut good = serde_json::json!({"website": "https://example.com"});
        assert!(plugin
            .before_insert("User", &mut good, &AuthContext::anonymous())
            .is_ok());
    }

    #[test]
    fn no_rules_for_entity_passes() {
        let plugin = ValidationPlugin::new();
        let mut data = serde_json::json!({"anything": "goes"});
        assert!(plugin
            .before_insert("Unknown", &mut data, &AuthContext::anonymous())
            .is_ok());
    }

    #[test]
    fn validates_on_update_too() {
        let mut plugin = ValidationPlugin::new();
        plugin.add_rule("Todo", "title", FieldRule::NotEmpty);

        let mut data = serde_json::json!({"title": ""});
        assert!(plugin
            .before_update("Todo", "t1", &mut data, &AuthContext::anonymous())
            .is_err());
    }

    #[test]
    fn multiple_rules_on_same_field() {
        let mut plugin = ValidationPlugin::new();
        plugin.add_rule("User", "displayName", FieldRule::NotEmpty);
        plugin.add_rule("User", "displayName", FieldRule::MinLength(2));
        plugin.add_rule("User", "displayName", FieldRule::MaxLength(50));

        let mut empty = serde_json::json!({"displayName": ""});
        assert!(plugin
            .before_insert("User", &mut empty, &AuthContext::anonymous())
            .is_err());

        let mut short = serde_json::json!({"displayName": "A"});
        assert!(plugin
            .before_insert("User", &mut short, &AuthContext::anonymous())
            .is_err());

        let mut valid = serde_json::json!({"displayName": "Alice"});
        assert!(plugin
            .before_insert("User", &mut valid, &AuthContext::anonymous())
            .is_ok());
    }
}
