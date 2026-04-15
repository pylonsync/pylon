use agentdb_auth::AuthContext;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Plugin trait — the core contract
// ---------------------------------------------------------------------------

/// A plugin extends agentdb with custom routes, lifecycle hooks, and entities.
pub trait Plugin: Send + Sync {
    /// Unique name for this plugin.
    fn name(&self) -> &str;

    /// Called once when the plugin is registered.
    fn on_init(&self, _ctx: &PluginContext) {}

    /// Custom API routes this plugin handles.
    fn routes(&self) -> Vec<PluginRoute> {
        vec![]
    }

    /// Called before an entity insert. Return Err to reject.
    fn before_insert(
        &self,
        _entity: &str,
        _data: &mut Value,
        _auth: &AuthContext,
    ) -> Result<(), PluginError> {
        Ok(())
    }

    /// Called after a successful insert.
    fn after_insert(&self, _entity: &str, _id: &str, _data: &Value, _auth: &AuthContext) {}

    /// Called before an entity update. Return Err to reject.
    fn before_update(
        &self,
        _entity: &str,
        _id: &str,
        _data: &mut Value,
        _auth: &AuthContext,
    ) -> Result<(), PluginError> {
        Ok(())
    }

    /// Called after a successful update.
    fn after_update(&self, _entity: &str, _id: &str, _data: &Value, _auth: &AuthContext) {}

    /// Called before an entity delete. Return Err to reject.
    fn before_delete(
        &self,
        _entity: &str,
        _id: &str,
        _auth: &AuthContext,
    ) -> Result<(), PluginError> {
        Ok(())
    }

    /// Called after a successful delete.
    fn after_delete(&self, _entity: &str, _id: &str, _auth: &AuthContext) {}

    /// Called on every incoming request (middleware).
    fn on_request(
        &self,
        _method: &str,
        _path: &str,
        _auth: &AuthContext,
    ) -> Result<(), PluginError> {
        Ok(())
    }

    /// Called when a new session is created.
    fn on_session_create(&self, _user_id: &str, _token: &str) {}

    /// Additional manifest entities this plugin contributes.
    fn entities(&self) -> Vec<agentdb_core::ManifestEntity> {
        vec![]
    }
}

// ---------------------------------------------------------------------------
// Plugin types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct PluginError {
    pub code: String,
    pub message: String,
    pub status: u16,
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

/// A route handler function type.
pub type RouteHandler = Box<dyn Fn(&str, &str, &AuthContext) -> (u16, String) + Send + Sync>;

/// A custom route registered by a plugin.
pub struct PluginRoute {
    pub method: String,
    pub path: String,
    pub handler: RouteHandler,
}

/// Context passed to plugins on init.
pub struct PluginContext {
    pub manifest: agentdb_core::AppManifest,
    pub data: Mutex<HashMap<String, Value>>,
}

impl PluginContext {
    pub fn new(manifest: agentdb_core::AppManifest) -> Self {
        Self {
            manifest,
            data: Mutex::new(HashMap::new()),
        }
    }

    /// Store plugin-specific data.
    pub fn set(&self, key: &str, value: Value) {
        self.data.lock().unwrap().insert(key.to_string(), value);
    }

    /// Retrieve plugin-specific data.
    pub fn get(&self, key: &str) -> Option<Value> {
        self.data.lock().unwrap().get(key).cloned()
    }
}

// ---------------------------------------------------------------------------
// Plugin registry — manages all registered plugins
// ---------------------------------------------------------------------------

pub struct PluginRegistry {
    plugins: Vec<Arc<dyn Plugin>>,
    context: Arc<PluginContext>,
}

impl PluginRegistry {
    pub fn new(manifest: agentdb_core::AppManifest) -> Self {
        Self {
            plugins: Vec::new(),
            context: Arc::new(PluginContext::new(manifest)),
        }
    }

    /// Register a plugin.
    pub fn register(&mut self, plugin: Arc<dyn Plugin>) {
        plugin.on_init(&self.context);
        self.plugins.push(plugin);
    }

    /// Get all registered plugins.
    pub fn plugins(&self) -> &[Arc<dyn Plugin>] {
        &self.plugins
    }

    /// Collect all custom routes from all plugins.
    pub fn all_routes(&self) -> Vec<&PluginRoute> {
        // Can't return references to temporary Vecs, so we need a different approach.
        // For now, routes are checked per-plugin in the request handler.
        vec![]
    }

    /// Run before_insert hooks. Returns first error, or Ok.
    pub fn run_before_insert(
        &self,
        entity: &str,
        data: &mut Value,
        auth: &AuthContext,
    ) -> Result<(), PluginError> {
        for plugin in &self.plugins {
            plugin.before_insert(entity, data, auth)?;
        }
        Ok(())
    }

    /// Run after_insert hooks.
    pub fn run_after_insert(&self, entity: &str, id: &str, data: &Value, auth: &AuthContext) {
        for plugin in &self.plugins {
            plugin.after_insert(entity, id, data, auth);
        }
    }

    /// Run before_update hooks.
    pub fn run_before_update(
        &self,
        entity: &str,
        id: &str,
        data: &mut Value,
        auth: &AuthContext,
    ) -> Result<(), PluginError> {
        for plugin in &self.plugins {
            plugin.before_update(entity, id, data, auth)?;
        }
        Ok(())
    }

    /// Run after_update hooks.
    pub fn run_after_update(&self, entity: &str, id: &str, data: &Value, auth: &AuthContext) {
        for plugin in &self.plugins {
            plugin.after_update(entity, id, data, auth);
        }
    }

    /// Run before_delete hooks.
    pub fn run_before_delete(
        &self,
        entity: &str,
        id: &str,
        auth: &AuthContext,
    ) -> Result<(), PluginError> {
        for plugin in &self.plugins {
            plugin.before_delete(entity, id, auth)?;
        }
        Ok(())
    }

    /// Run after_delete hooks.
    pub fn run_after_delete(&self, entity: &str, id: &str, auth: &AuthContext) {
        for plugin in &self.plugins {
            plugin.after_delete(entity, id, auth);
        }
    }

    /// Run on_request middleware. Returns first error, or Ok.
    pub fn run_on_request(
        &self,
        method: &str,
        path: &str,
        auth: &AuthContext,
    ) -> Result<(), PluginError> {
        for plugin in &self.plugins {
            plugin.on_request(method, path, auth)?;
        }
        Ok(())
    }

    /// Try to handle a request with plugin routes.
    pub fn try_handle_route(
        &self,
        method: &str,
        path: &str,
        body: &str,
        auth: &AuthContext,
    ) -> Option<(u16, String)> {
        for plugin in &self.plugins {
            for route in plugin.routes() {
                if route.method == method && path.starts_with(&route.path) {
                    return Some((route.handler)(body, path, auth));
                }
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Built-in plugins
// ---------------------------------------------------------------------------

pub mod builtin;

// ---------------------------------------------------------------------------
// Plugin marketplace — discovery and metadata registry
// ---------------------------------------------------------------------------

pub mod registry;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    struct TestPlugin {
        insert_count: Mutex<u32>,
    }

    impl TestPlugin {
        fn new() -> Self {
            Self {
                insert_count: Mutex::new(0),
            }
        }
        fn count(&self) -> u32 {
            *self.insert_count.lock().unwrap()
        }
    }

    impl Plugin for TestPlugin {
        fn name(&self) -> &str {
            "test"
        }

        fn after_insert(&self, _entity: &str, _id: &str, _data: &Value, _auth: &AuthContext) {
            *self.insert_count.lock().unwrap() += 1;
        }

        fn before_insert(
            &self,
            entity: &str,
            _data: &mut Value,
            _auth: &AuthContext,
        ) -> Result<(), PluginError> {
            if entity == "Blocked" {
                return Err(PluginError {
                    code: "BLOCKED".into(),
                    message: "Inserts to Blocked are not allowed".into(),
                    status: 403,
                });
            }
            Ok(())
        }
    }

    fn test_manifest() -> agentdb_core::AppManifest {
        agentdb_core::AppManifest {
            manifest_version: agentdb_core::MANIFEST_VERSION,
            name: "test".into(),
            version: "0.1.0".into(),
            entities: vec![],
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![],
        }
    }

    #[test]
    fn register_plugin() {
        let mut registry = PluginRegistry::new(test_manifest());
        let plugin = Arc::new(TestPlugin::new());
        registry.register(plugin.clone());
        assert_eq!(registry.plugins().len(), 1);
        assert_eq!(registry.plugins()[0].name(), "test");
    }

    #[test]
    fn before_insert_hook_allows() {
        let mut registry = PluginRegistry::new(test_manifest());
        registry.register(Arc::new(TestPlugin::new()));

        let mut data = serde_json::json!({"title": "test"});
        let auth = AuthContext::anonymous();
        let result = registry.run_before_insert("Todo", &mut data, &auth);
        assert!(result.is_ok());
    }

    #[test]
    fn before_insert_hook_rejects() {
        let mut registry = PluginRegistry::new(test_manifest());
        registry.register(Arc::new(TestPlugin::new()));

        let mut data = serde_json::json!({"title": "test"});
        let auth = AuthContext::anonymous();
        let result = registry.run_before_insert("Blocked", &mut data, &auth);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, "BLOCKED");
    }

    #[test]
    fn after_insert_hook_fires() {
        let mut registry = PluginRegistry::new(test_manifest());
        let plugin = Arc::new(TestPlugin::new());
        registry.register(plugin.clone());

        let data = serde_json::json!({"title": "test"});
        let auth = AuthContext::anonymous();
        registry.run_after_insert("Todo", "1", &data, &auth);
        assert_eq!(plugin.count(), 1);

        registry.run_after_insert("Todo", "2", &data, &auth);
        assert_eq!(plugin.count(), 2);
    }

    #[test]
    fn on_request_middleware() {
        struct BlockAdmin;
        impl Plugin for BlockAdmin {
            fn name(&self) -> &str { "block-admin" }
            fn on_request(&self, _method: &str, path: &str, _auth: &AuthContext) -> Result<(), PluginError> {
                if path.starts_with("/api/admin") {
                    Err(PluginError { code: "FORBIDDEN".into(), message: "Admin access denied".into(), status: 403 })
                } else {
                    Ok(())
                }
            }
        }

        let mut registry = PluginRegistry::new(test_manifest());
        registry.register(Arc::new(BlockAdmin));

        let auth = AuthContext::anonymous();
        assert!(registry.run_on_request("GET", "/api/entities/Todo", &auth).is_ok());
        assert!(registry.run_on_request("GET", "/api/admin/users", &auth).is_err());
    }

    #[test]
    fn plugin_context_data() {
        let ctx = PluginContext::new(test_manifest());
        ctx.set("key", serde_json::json!("value"));
        assert_eq!(ctx.get("key"), Some(serde_json::json!("value")));
        assert_eq!(ctx.get("missing"), None);
    }

    #[test]
    fn plugin_error_display() {
        let err = PluginError { code: "TEST".into(), message: "msg".into(), status: 400 };
        assert_eq!(format!("{err}"), "[TEST] msg");
    }
}
