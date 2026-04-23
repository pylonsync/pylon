use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// Plugin marketplace metadata types
// ---------------------------------------------------------------------------

/// Metadata for a published plugin in the marketplace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginMetadata {
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub license: String,
    pub homepage: Option<String>,
    pub repository: Option<String>,
    pub tags: Vec<String>,
    pub category: PluginCategory,
    /// Semver range for pylon version compatibility.
    pub compatibility: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum PluginCategory {
    Auth,
    Storage,
    Integration,
    Analytics,
    Billing,
    Communication,
    Security,
    DevTools,
    Other,
}

impl PluginCategory {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Auth => "auth",
            Self::Storage => "storage",
            Self::Integration => "integration",
            Self::Analytics => "analytics",
            Self::Billing => "billing",
            Self::Communication => "communication",
            Self::Security => "security",
            Self::DevTools => "devtools",
            Self::Other => "other",
        }
    }

    /// Display-friendly label for CLI output.
    pub fn label(&self) -> &str {
        match self {
            Self::Auth => "Auth",
            Self::Storage => "Storage",
            Self::Integration => "Integration",
            Self::Analytics => "Analytics",
            Self::Billing => "Billing",
            Self::Communication => "Communication",
            Self::Security => "Security",
            Self::DevTools => "Dev Tools",
            Self::Other => "Other",
        }
    }

    /// Ordered list of all categories for consistent display.
    pub fn all_ordered() -> &'static [PluginCategory] {
        &[
            Self::Auth,
            Self::Storage,
            Self::Integration,
            Self::Analytics,
            Self::Security,
            Self::DevTools,
            Self::Billing,
            Self::Communication,
            Self::Other,
        ]
    }
}

// ---------------------------------------------------------------------------
// Plugin marketplace — in-memory registry
// ---------------------------------------------------------------------------

/// A local plugin marketplace registry.
///
/// In production this would talk to a remote API. For now it is an in-memory
/// catalog used for testing, development, and the CLI `plugins` command.
pub struct PluginMarketplace {
    plugins: Mutex<HashMap<String, PluginMetadata>>,
}

impl PluginMarketplace {
    pub fn new() -> Self {
        Self {
            plugins: Mutex::new(HashMap::new()),
        }
    }

    /// Register a plugin in the marketplace.
    ///
    /// Returns `Err` if the name is empty, version is empty, or a plugin with
    /// the same name is already published.
    pub fn publish(&self, metadata: PluginMetadata) -> Result<(), String> {
        if metadata.name.is_empty() {
            return Err("plugin name must not be empty".into());
        }
        if metadata.version.is_empty() {
            return Err("plugin version must not be empty".into());
        }

        let mut plugins = self.plugins.lock().unwrap();
        if plugins.contains_key(&metadata.name) {
            return Err(format!("plugin \"{}\" is already published", metadata.name));
        }
        plugins.insert(metadata.name.clone(), metadata);
        Ok(())
    }

    /// Search plugins by query string.
    ///
    /// Matches against name, description, and tags (case-insensitive).
    pub fn search(&self, query: &str) -> Vec<PluginMetadata> {
        let q = query.to_lowercase();
        let plugins = self.plugins.lock().unwrap();
        plugins
            .values()
            .filter(|p| {
                p.name.to_lowercase().contains(&q)
                    || p.description.to_lowercase().contains(&q)
                    || p.tags.iter().any(|t| t.to_lowercase().contains(&q))
            })
            .cloned()
            .collect()
    }

    /// List plugins by category.
    pub fn by_category(&self, category: PluginCategory) -> Vec<PluginMetadata> {
        let plugins = self.plugins.lock().unwrap();
        plugins
            .values()
            .filter(|p| p.category == category)
            .cloned()
            .collect()
    }

    /// Get a specific plugin by name.
    pub fn get(&self, name: &str) -> Option<PluginMetadata> {
        self.plugins.lock().unwrap().get(name).cloned()
    }

    /// List all available plugins.
    pub fn list_all(&self) -> Vec<PluginMetadata> {
        self.plugins.lock().unwrap().values().cloned().collect()
    }

    /// Remove a plugin from the marketplace. Returns `true` if it existed.
    pub fn unpublish(&self, name: &str) -> bool {
        self.plugins.lock().unwrap().remove(name).is_some()
    }

    /// Get plugin count.
    pub fn count(&self) -> usize {
        self.plugins.lock().unwrap().len()
    }

    /// Seed the marketplace with all built-in plugin metadata.
    pub fn seed_builtins(&self) {
        let builtins = vec![
            // -- Auth --
            PluginMetadata {
                name: "password-auth".into(),
                version: "0.1.0".into(),
                description: "Secure password hashing with salt".into(),
                author: "pylon".into(),
                license: "MIT".into(),
                homepage: None,
                repository: None,
                tags: vec!["auth".into(), "password".into(), "hashing".into()],
                category: PluginCategory::Auth,
                compatibility: ">=0.1.0".into(),
            },
            PluginMetadata {
                name: "session-expiry".into(),
                version: "0.1.0".into(),
                description: "Session lifetime with idle timeout".into(),
                author: "pylon".into(),
                license: "MIT".into(),
                homepage: None,
                repository: None,
                tags: vec!["auth".into(), "session".into(), "expiry".into()],
                category: PluginCategory::Auth,
                compatibility: ">=0.1.0".into(),
            },
            PluginMetadata {
                name: "jwt".into(),
                version: "0.1.0".into(),
                description: "JWT token issuance and verification".into(),
                author: "pylon".into(),
                license: "MIT".into(),
                homepage: None,
                repository: None,
                tags: vec!["auth".into(), "jwt".into(), "token".into()],
                category: PluginCategory::Auth,
                compatibility: ">=0.1.0".into(),
            },
            PluginMetadata {
                name: "totp".into(),
                version: "0.1.0".into(),
                description: "TOTP 2FA (RFC 6238)".into(),
                author: "pylon".into(),
                license: "MIT".into(),
                homepage: None,
                repository: None,
                tags: vec!["auth".into(), "2fa".into(), "totp".into(), "mfa".into()],
                category: PluginCategory::Auth,
                compatibility: ">=0.1.0".into(),
            },
            PluginMetadata {
                name: "organizations".into(),
                version: "0.1.0".into(),
                description: "Multi-tenant team management".into(),
                author: "pylon".into(),
                license: "MIT".into(),
                homepage: None,
                repository: None,
                tags: vec!["auth".into(), "multi-tenant".into(), "teams".into()],
                category: PluginCategory::Auth,
                compatibility: ">=0.1.0".into(),
            },
            PluginMetadata {
                name: "cors".into(),
                version: "0.1.0".into(),
                description: "CORS origin validation".into(),
                author: "pylon".into(),
                license: "MIT".into(),
                homepage: None,
                repository: None,
                tags: vec!["auth".into(), "cors".into(), "security".into()],
                category: PluginCategory::Auth,
                compatibility: ">=0.1.0".into(),
            },
            PluginMetadata {
                name: "csrf".into(),
                version: "0.1.0".into(),
                description: "CSRF protection middleware".into(),
                author: "pylon".into(),
                license: "MIT".into(),
                homepage: None,
                repository: None,
                tags: vec!["auth".into(), "csrf".into(), "security".into()],
                category: PluginCategory::Auth,
                compatibility: ">=0.1.0".into(),
            },
            // -- Storage --
            PluginMetadata {
                name: "file-storage".into(),
                version: "0.1.0".into(),
                description: "File upload/download with storage backends".into(),
                author: "pylon".into(),
                license: "MIT".into(),
                homepage: None,
                repository: None,
                tags: vec!["storage".into(), "files".into(), "upload".into()],
                category: PluginCategory::Storage,
                compatibility: ">=0.1.0".into(),
            },
            PluginMetadata {
                name: "soft-delete".into(),
                version: "0.1.0".into(),
                description: "Mark-as-deleted instead of hard delete".into(),
                author: "pylon".into(),
                license: "MIT".into(),
                homepage: None,
                repository: None,
                tags: vec!["storage".into(), "soft-delete".into(), "archive".into()],
                category: PluginCategory::Storage,
                compatibility: ">=0.1.0".into(),
            },
            PluginMetadata {
                name: "versioning".into(),
                version: "0.1.0".into(),
                description: "Row version tracking for optimistic concurrency".into(),
                author: "pylon".into(),
                license: "MIT".into(),
                homepage: None,
                repository: None,
                tags: vec!["storage".into(), "versioning".into(), "concurrency".into()],
                category: PluginCategory::Storage,
                compatibility: ">=0.1.0".into(),
            },
            PluginMetadata {
                name: "cascade".into(),
                version: "0.1.0".into(),
                description: "Cascading deletes across related entities".into(),
                author: "pylon".into(),
                license: "MIT".into(),
                homepage: None,
                repository: None,
                tags: vec!["storage".into(), "cascade".into(), "relations".into()],
                category: PluginCategory::Storage,
                compatibility: ">=0.1.0".into(),
            },
            // -- Integration --
            PluginMetadata {
                name: "webhooks".into(),
                version: "0.1.0".into(),
                description: "Outbound webhook delivery with retries".into(),
                author: "pylon".into(),
                license: "MIT".into(),
                homepage: None,
                repository: None,
                tags: vec!["integration".into(), "webhooks".into(), "events".into()],
                category: PluginCategory::Integration,
                compatibility: ">=0.1.0".into(),
            },
            PluginMetadata {
                name: "email".into(),
                version: "0.1.0".into(),
                description: "Transactional email sending via SMTP/API".into(),
                author: "pylon".into(),
                license: "MIT".into(),
                homepage: None,
                repository: None,
                tags: vec!["integration".into(), "email".into(), "smtp".into()],
                category: PluginCategory::Integration,
                compatibility: ">=0.1.0".into(),
            },
            PluginMetadata {
                name: "mcp".into(),
                version: "0.1.0".into(),
                description: "Model Context Protocol server for AI agents".into(),
                author: "pylon".into(),
                license: "MIT".into(),
                homepage: None,
                repository: None,
                tags: vec![
                    "integration".into(),
                    "mcp".into(),
                    "ai".into(),
                    "agents".into(),
                ],
                category: PluginCategory::Integration,
                compatibility: ">=0.1.0".into(),
            },
            // -- Analytics --
            PluginMetadata {
                name: "audit-log".into(),
                version: "0.1.0".into(),
                description: "Immutable audit trail for all mutations".into(),
                author: "pylon".into(),
                license: "MIT".into(),
                homepage: None,
                repository: None,
                tags: vec!["analytics".into(), "audit".into(), "logging".into()],
                category: PluginCategory::Analytics,
                compatibility: ">=0.1.0".into(),
            },
            PluginMetadata {
                name: "search".into(),
                version: "0.1.0".into(),
                description: "Full-text search across entities".into(),
                author: "pylon".into(),
                license: "MIT".into(),
                homepage: None,
                repository: None,
                tags: vec!["analytics".into(), "search".into(), "full-text".into()],
                category: PluginCategory::Analytics,
                compatibility: ">=0.1.0".into(),
            },
            // -- Security --
            PluginMetadata {
                name: "rate-limit".into(),
                version: "0.1.0".into(),
                description: "Per-user/IP rate limiting with configurable windows".into(),
                author: "pylon".into(),
                license: "MIT".into(),
                homepage: None,
                repository: None,
                tags: vec!["security".into(), "rate-limiting".into()],
                category: PluginCategory::Security,
                compatibility: ">=0.1.0".into(),
            },
            // -- DevTools --
            PluginMetadata {
                name: "timestamps".into(),
                version: "0.1.0".into(),
                description: "Auto-populate created_at and updated_at fields".into(),
                author: "pylon".into(),
                license: "MIT".into(),
                homepage: None,
                repository: None,
                tags: vec!["devtools".into(), "timestamps".into(), "auto".into()],
                category: PluginCategory::DevTools,
                compatibility: ">=0.1.0".into(),
            },
            PluginMetadata {
                name: "slugify".into(),
                version: "0.1.0".into(),
                description: "Auto-generate URL-safe slugs from fields".into(),
                author: "pylon".into(),
                license: "MIT".into(),
                homepage: None,
                repository: None,
                tags: vec!["devtools".into(), "slug".into(), "url".into()],
                category: PluginCategory::DevTools,
                compatibility: ">=0.1.0".into(),
            },
            PluginMetadata {
                name: "validation".into(),
                version: "0.1.0".into(),
                description: "Schema-level field validation rules".into(),
                author: "pylon".into(),
                license: "MIT".into(),
                homepage: None,
                repository: None,
                tags: vec!["devtools".into(), "validation".into(), "schema".into()],
                category: PluginCategory::DevTools,
                compatibility: ">=0.1.0".into(),
            },
            PluginMetadata {
                name: "computed".into(),
                version: "0.1.0".into(),
                description: "Computed/derived fields from other columns".into(),
                author: "pylon".into(),
                license: "MIT".into(),
                homepage: None,
                repository: None,
                tags: vec!["devtools".into(), "computed".into(), "derived".into()],
                category: PluginCategory::DevTools,
                compatibility: ">=0.1.0".into(),
            },
            PluginMetadata {
                name: "feature-flags".into(),
                version: "0.1.0".into(),
                description: "Runtime feature flags with rollout controls".into(),
                author: "pylon".into(),
                license: "MIT".into(),
                homepage: None,
                repository: None,
                tags: vec!["devtools".into(), "feature-flags".into(), "rollout".into()],
                category: PluginCategory::DevTools,
                compatibility: ">=0.1.0".into(),
            },
            // -- Other --
            PluginMetadata {
                name: "api-keys".into(),
                version: "0.1.0".into(),
                description: "API key generation and authentication".into(),
                author: "pylon".into(),
                license: "MIT".into(),
                homepage: None,
                repository: None,
                tags: vec!["api".into(), "keys".into(), "authentication".into()],
                category: PluginCategory::Other,
                compatibility: ">=0.1.0".into(),
            },
        ];

        for p in builtins {
            let _ = self.publish(p);
        }
    }
}

impl Default for PluginMarketplace {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_plugin(name: &str, category: PluginCategory) -> PluginMetadata {
        PluginMetadata {
            name: name.into(),
            version: "1.0.0".into(),
            description: format!("A {name} plugin"),
            author: "test".into(),
            license: "MIT".into(),
            homepage: None,
            repository: None,
            tags: vec!["test".into(), name.into()],
            category,
            compatibility: ">=0.1.0".into(),
        }
    }

    #[test]
    fn publish_and_get() {
        let mp = PluginMarketplace::new();
        let plugin = make_plugin("my-plugin", PluginCategory::Auth);
        assert!(mp.publish(plugin).is_ok());
        assert_eq!(mp.count(), 1);

        let got = mp.get("my-plugin").unwrap();
        assert_eq!(got.name, "my-plugin");
        assert_eq!(got.version, "1.0.0");
    }

    #[test]
    fn duplicate_rejected() {
        let mp = PluginMarketplace::new();
        let p1 = make_plugin("dup", PluginCategory::Auth);
        let p2 = make_plugin("dup", PluginCategory::Storage);
        assert!(mp.publish(p1).is_ok());

        let err = mp.publish(p2).unwrap_err();
        assert!(err.contains("already published"));
    }

    #[test]
    fn empty_name_rejected() {
        let mp = PluginMarketplace::new();
        let mut p = make_plugin("x", PluginCategory::Auth);
        p.name = String::new();
        let err = mp.publish(p).unwrap_err();
        assert!(err.contains("name must not be empty"));
    }

    #[test]
    fn empty_version_rejected() {
        let mp = PluginMarketplace::new();
        let mut p = make_plugin("x", PluginCategory::Auth);
        p.version = String::new();
        let err = mp.publish(p).unwrap_err();
        assert!(err.contains("version must not be empty"));
    }

    #[test]
    fn search_by_name() {
        let mp = PluginMarketplace::new();
        mp.publish(make_plugin("rate-limiter", PluginCategory::Security))
            .unwrap();
        mp.publish(make_plugin("auth-basic", PluginCategory::Auth))
            .unwrap();

        let results = mp.search("rate");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "rate-limiter");
    }

    #[test]
    fn search_by_description() {
        let mp = PluginMarketplace::new();
        mp.publish(make_plugin("foo", PluginCategory::Other))
            .unwrap();
        // description is "A foo plugin"
        let results = mp.search("foo plugin");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn search_by_tag() {
        let mp = PluginMarketplace::new();
        let mut p = make_plugin("widget", PluginCategory::Other);
        p.tags = vec!["special-tag".into()];
        mp.publish(p).unwrap();

        let results = mp.search("special-tag");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "widget");
    }

    #[test]
    fn search_case_insensitive() {
        let mp = PluginMarketplace::new();
        mp.publish(make_plugin("MyPlugin", PluginCategory::Auth))
            .unwrap();

        assert_eq!(mp.search("myplugin").len(), 1);
        assert_eq!(mp.search("MYPLUGIN").len(), 1);
    }

    #[test]
    fn by_category() {
        let mp = PluginMarketplace::new();
        mp.publish(make_plugin("a", PluginCategory::Auth)).unwrap();
        mp.publish(make_plugin("b", PluginCategory::Auth)).unwrap();
        mp.publish(make_plugin("c", PluginCategory::Storage))
            .unwrap();

        let auth = mp.by_category(PluginCategory::Auth);
        assert_eq!(auth.len(), 2);

        let storage = mp.by_category(PluginCategory::Storage);
        assert_eq!(storage.len(), 1);

        let billing = mp.by_category(PluginCategory::Billing);
        assert!(billing.is_empty());
    }

    #[test]
    fn unpublish() {
        let mp = PluginMarketplace::new();
        mp.publish(make_plugin("rm-me", PluginCategory::Other))
            .unwrap();
        assert_eq!(mp.count(), 1);

        assert!(mp.unpublish("rm-me"));
        assert_eq!(mp.count(), 0);
        assert!(mp.get("rm-me").is_none());
    }

    #[test]
    fn unpublish_nonexistent_returns_false() {
        let mp = PluginMarketplace::new();
        assert!(!mp.unpublish("ghost"));
    }

    #[test]
    fn list_all() {
        let mp = PluginMarketplace::new();
        mp.publish(make_plugin("a", PluginCategory::Auth)).unwrap();
        mp.publish(make_plugin("b", PluginCategory::Storage))
            .unwrap();

        let all = mp.list_all();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn seed_builtins_populates_all() {
        let mp = PluginMarketplace::new();
        mp.seed_builtins();

        // 23 built-in plugins total
        assert_eq!(mp.count(), 23);

        // Spot-check a few from different categories
        assert!(mp.get("password-auth").is_some());
        assert!(mp.get("jwt").is_some());
        assert!(mp.get("file-storage").is_some());
        assert!(mp.get("webhooks").is_some());
        assert!(mp.get("audit-log").is_some());
        assert!(mp.get("rate-limit").is_some());
        assert!(mp.get("timestamps").is_some());
        assert!(mp.get("api-keys").is_some());
        assert!(mp.get("mcp").is_some());
    }

    #[test]
    fn seed_builtins_categories_correct() {
        let mp = PluginMarketplace::new();
        mp.seed_builtins();

        assert_eq!(mp.by_category(PluginCategory::Auth).len(), 7);
        assert_eq!(mp.by_category(PluginCategory::Storage).len(), 4);
        assert_eq!(mp.by_category(PluginCategory::Integration).len(), 3);
        assert_eq!(mp.by_category(PluginCategory::Analytics).len(), 2);
        assert_eq!(mp.by_category(PluginCategory::Security).len(), 1);
        assert_eq!(mp.by_category(PluginCategory::DevTools).len(), 5);
        assert_eq!(mp.by_category(PluginCategory::Other).len(), 1);
    }

    #[test]
    fn category_as_str() {
        assert_eq!(PluginCategory::Auth.as_str(), "auth");
        assert_eq!(PluginCategory::DevTools.as_str(), "devtools");
        assert_eq!(PluginCategory::Other.as_str(), "other");
    }

    #[test]
    fn plugin_metadata_serializes() {
        let p = make_plugin("test", PluginCategory::Auth);
        let json = serde_json::to_string(&p).unwrap();
        let deserialized: PluginMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "test");
        assert_eq!(deserialized.category, PluginCategory::Auth);
    }
}
