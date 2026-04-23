use std::collections::HashMap;

use crate::{Plugin, PluginError};
use pylon_auth::AuthContext;
use serde_json::Value;

/// Slugify configuration for one entity.
pub struct SlugConfig {
    /// Source field to generate slug from.
    pub source: String,
    /// Target field to write the slug to.
    pub target: String,
}

/// Slugify plugin. Auto-generates URL-safe slugs from a source field.
pub struct SlugifyPlugin {
    configs: HashMap<String, SlugConfig>,
}

impl SlugifyPlugin {
    pub fn new() -> Self {
        Self {
            configs: HashMap::new(),
        }
    }

    /// Add slug generation for an entity.
    pub fn add(&mut self, entity: &str, source: &str, target: &str) {
        self.configs.insert(
            entity.to_string(),
            SlugConfig {
                source: source.to_string(),
                target: target.to_string(),
            },
        );
    }
}

/// Convert a string to a URL-safe slug.
pub fn slugify(input: &str) -> String {
    let mut slug = String::new();
    let mut last_was_dash = false;

    for ch in input.chars() {
        if ch.is_alphanumeric() {
            slug.push(ch.to_lowercase().next().unwrap_or(ch));
            last_was_dash = false;
        } else if ch == ' ' || ch == '-' || ch == '_' {
            if !last_was_dash && !slug.is_empty() {
                slug.push('-');
                last_was_dash = true;
            }
        }
        // Skip other characters.
    }

    // Trim trailing dash.
    if slug.ends_with('-') {
        slug.pop();
    }

    slug
}

impl Plugin for SlugifyPlugin {
    fn name(&self) -> &str {
        "slugify"
    }

    fn before_insert(
        &self,
        entity: &str,
        data: &mut Value,
        _auth: &AuthContext,
    ) -> Result<(), PluginError> {
        if let Some(config) = self.configs.get(entity) {
            if let Some(obj) = data.as_object_mut() {
                // Only generate if target field is not already set.
                let target_exists = obj
                    .get(&config.target)
                    .map(|v| v.as_str().map(|s| !s.is_empty()).unwrap_or(false))
                    .unwrap_or(false);

                if !target_exists {
                    if let Some(source_val) = obj.get(&config.source).and_then(|v| v.as_str()) {
                        let slug = slugify(source_val);
                        obj.insert(config.target.clone(), Value::String(slug));
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_slugify() {
        assert_eq!(slugify("Hello World"), "hello-world");
        assert_eq!(slugify("My Blog Post!"), "my-blog-post");
        assert_eq!(slugify("  Spaces  Everywhere  "), "spaces-everywhere");
        assert_eq!(slugify("UPPER CASE"), "upper-case");
        assert_eq!(slugify("special@#$chars"), "specialchars");
        assert_eq!(slugify("already-slugged"), "already-slugged");
        assert_eq!(slugify(""), "");
    }

    #[test]
    fn unicode_slugify() {
        assert_eq!(slugify("Café Résumé"), "café-résumé");
        assert_eq!(slugify("日本語"), "日本語");
    }

    #[test]
    fn plugin_generates_slug_on_insert() {
        let mut plugin = SlugifyPlugin::new();
        plugin.add("Post", "title", "slug");

        let mut data = serde_json::json!({"title": "My First Blog Post"});
        plugin
            .before_insert("Post", &mut data, &AuthContext::anonymous())
            .unwrap();
        assert_eq!(data["slug"], "my-first-blog-post");
    }

    #[test]
    fn does_not_overwrite_existing_slug() {
        let mut plugin = SlugifyPlugin::new();
        plugin.add("Post", "title", "slug");

        let mut data = serde_json::json!({"title": "My Post", "slug": "custom-slug"});
        plugin
            .before_insert("Post", &mut data, &AuthContext::anonymous())
            .unwrap();
        assert_eq!(data["slug"], "custom-slug");
    }

    #[test]
    fn no_config_for_entity_passes() {
        let plugin = SlugifyPlugin::new();
        let mut data = serde_json::json!({"title": "Test"});
        plugin
            .before_insert("Unknown", &mut data, &AuthContext::anonymous())
            .unwrap();
        assert!(data.get("slug").is_none());
    }

    #[test]
    fn missing_source_field_no_error() {
        let mut plugin = SlugifyPlugin::new();
        plugin.add("Post", "title", "slug");

        let mut data = serde_json::json!({"body": "no title here"});
        plugin
            .before_insert("Post", &mut data, &AuthContext::anonymous())
            .unwrap();
        assert!(data.get("slug").is_none());
    }
}
