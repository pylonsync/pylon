use std::collections::HashMap;
use std::sync::Mutex;

use crate::Plugin;
use agentdb_auth::AuthContext;

/// A feature flag rule.
#[derive(Debug, Clone)]
pub enum FlagRule {
    /// Always on or off.
    Boolean(bool),
    /// On for specific user IDs.
    UserList(Vec<String>),
    /// On for a percentage of users (0-100).
    Percentage(u8),
}

/// A feature flag definition.
#[derive(Debug, Clone)]
pub struct FeatureFlag {
    pub name: String,
    pub description: String,
    pub rule: FlagRule,
    pub enabled: bool,
}

/// Feature flags plugin. Toggle features per user/percentage.
pub struct FeatureFlagsPlugin {
    flags: Mutex<HashMap<String, FeatureFlag>>,
}

impl FeatureFlagsPlugin {
    pub fn new() -> Self {
        Self {
            flags: Mutex::new(HashMap::new()),
        }
    }

    /// Define a flag that's globally on or off.
    pub fn add_boolean(&self, name: &str, description: &str, enabled: bool) {
        self.flags.lock().unwrap().insert(
            name.to_string(),
            FeatureFlag {
                name: name.to_string(),
                description: description.to_string(),
                rule: FlagRule::Boolean(enabled),
                enabled,
            },
        );
    }

    /// Define a flag that's on for specific users.
    pub fn add_user_list(&self, name: &str, description: &str, users: Vec<String>) {
        self.flags.lock().unwrap().insert(
            name.to_string(),
            FeatureFlag {
                name: name.to_string(),
                description: description.to_string(),
                rule: FlagRule::UserList(users),
                enabled: true,
            },
        );
    }

    /// Define a flag that's on for a percentage of users.
    pub fn add_percentage(&self, name: &str, description: &str, percent: u8) {
        self.flags.lock().unwrap().insert(
            name.to_string(),
            FeatureFlag {
                name: name.to_string(),
                description: description.to_string(),
                rule: FlagRule::Percentage(percent.min(100)),
                enabled: true,
            },
        );
    }

    /// Check if a flag is enabled for a given auth context.
    pub fn is_enabled(&self, flag_name: &str, auth: &AuthContext) -> bool {
        let flags = self.flags.lock().unwrap();
        let flag = match flags.get(flag_name) {
            Some(f) => f,
            None => return false, // unknown flag = off
        };

        if !flag.enabled {
            return false;
        }

        match &flag.rule {
            FlagRule::Boolean(on) => *on,
            FlagRule::UserList(users) => {
                auth.user_id.as_ref().map(|id| users.contains(id)).unwrap_or(false)
            }
            FlagRule::Percentage(pct) => {
                let hash = auth
                    .user_id
                    .as_ref()
                    .map(|id| {
                        id.bytes().fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64))
                    })
                    .unwrap_or(0);
                (hash % 100) < (*pct as u64)
            }
        }
    }

    /// Toggle a flag on or off.
    pub fn set_enabled(&self, flag_name: &str, enabled: bool) -> bool {
        let mut flags = self.flags.lock().unwrap();
        if let Some(flag) = flags.get_mut(flag_name) {
            flag.enabled = enabled;
            true
        } else {
            false
        }
    }

    /// List all flags.
    pub fn list_flags(&self) -> Vec<FeatureFlag> {
        self.flags.lock().unwrap().values().cloned().collect()
    }

    /// Remove a flag.
    pub fn remove(&self, flag_name: &str) -> bool {
        self.flags.lock().unwrap().remove(flag_name).is_some()
    }
}

impl Plugin for FeatureFlagsPlugin {
    fn name(&self) -> &str {
        "feature-flags"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boolean_flag() {
        let plugin = FeatureFlagsPlugin::new();
        plugin.add_boolean("dark-mode", "Enable dark mode", true);

        assert!(plugin.is_enabled("dark-mode", &AuthContext::anonymous()));

        plugin.set_enabled("dark-mode", false);
        assert!(!plugin.is_enabled("dark-mode", &AuthContext::anonymous()));
    }

    #[test]
    fn user_list_flag() {
        let plugin = FeatureFlagsPlugin::new();
        plugin.add_user_list("beta", "Beta features", vec!["user-1".into(), "user-2".into()]);

        assert!(plugin.is_enabled("beta", &AuthContext::authenticated("user-1".into())));
        assert!(plugin.is_enabled("beta", &AuthContext::authenticated("user-2".into())));
        assert!(!plugin.is_enabled("beta", &AuthContext::authenticated("user-3".into())));
        assert!(!plugin.is_enabled("beta", &AuthContext::anonymous()));
    }

    #[test]
    fn percentage_flag() {
        let plugin = FeatureFlagsPlugin::new();
        plugin.add_percentage("new-ui", "New UI experiment", 50);

        // Check that it's deterministic for the same user.
        let auth = AuthContext::authenticated("test-user".into());
        let result1 = plugin.is_enabled("new-ui", &auth);
        let result2 = plugin.is_enabled("new-ui", &auth);
        assert_eq!(result1, result2);
    }

    #[test]
    fn percentage_zero_always_off() {
        let plugin = FeatureFlagsPlugin::new();
        plugin.add_percentage("disabled", "Always off", 0);

        assert!(!plugin.is_enabled("disabled", &AuthContext::authenticated("user-1".into())));
    }

    #[test]
    fn percentage_100_always_on() {
        let plugin = FeatureFlagsPlugin::new();
        plugin.add_percentage("enabled", "Always on", 100);

        assert!(plugin.is_enabled("enabled", &AuthContext::authenticated("user-1".into())));
        assert!(plugin.is_enabled("enabled", &AuthContext::authenticated("user-2".into())));
    }

    #[test]
    fn unknown_flag_returns_false() {
        let plugin = FeatureFlagsPlugin::new();
        assert!(!plugin.is_enabled("nonexistent", &AuthContext::anonymous()));
    }

    #[test]
    fn remove_flag() {
        let plugin = FeatureFlagsPlugin::new();
        plugin.add_boolean("test", "Test", true);
        assert!(plugin.remove("test"));
        assert!(!plugin.is_enabled("test", &AuthContext::anonymous()));
    }

    #[test]
    fn list_flags() {
        let plugin = FeatureFlagsPlugin::new();
        plugin.add_boolean("a", "Flag A", true);
        plugin.add_boolean("b", "Flag B", false);

        let flags = plugin.list_flags();
        assert_eq!(flags.len(), 2);
    }

    #[test]
    fn disabled_flag_ignores_rules() {
        let plugin = FeatureFlagsPlugin::new();
        plugin.add_user_list("beta", "Beta", vec!["user-1".into()]);
        plugin.set_enabled("beta", false);

        assert!(!plugin.is_enabled("beta", &AuthContext::authenticated("user-1".into())));
    }
}
