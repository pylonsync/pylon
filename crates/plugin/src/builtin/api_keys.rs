use std::collections::HashMap;
use std::sync::Mutex;

use sha2::{Digest, Sha256};

use crate::{Plugin, PluginError};
use pylon_auth::AuthContext;

/// An API key with scoped permissions.
///
/// The raw key is never stored. Only the hash is retained after creation.
#[derive(Debug, Clone)]
pub struct ApiKey {
    pub key_hash: String,
    pub name: String,
    pub user_id: String,
    pub scopes: Vec<String>,
    pub created_at: String,
}

/// Result returned from key creation. Contains the raw key (shown only once)
/// alongside the stored metadata.
#[derive(Debug, Clone)]
pub struct CreatedApiKey {
    /// The raw API key. This is the only time the caller will see it.
    pub raw_key: String,
    pub name: String,
    pub user_id: String,
    pub scopes: Vec<String>,
    pub created_at: String,
}

/// API Keys plugin. Allows issuing and revoking API keys with scoped permissions.
///
/// Keys are stored as SHA-256 hashes only. The raw key is returned exactly once
/// at creation time and is never persisted.
pub struct ApiKeysPlugin {
    /// Map from `sha256(api_key)` -> `ApiKey` metadata.
    keys: Mutex<HashMap<String, ApiKey>>,
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Produce a SHA-256 hash of an API key string.
///
/// Returns a lowercase hex-encoded 64-character string (256 bits).
fn hash_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    let result = hasher.finalize();
    hex_encode(&result)
}

/// Generate an API key with 192 bits of entropy from a CSPRNG.
fn generate_key() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: [u8; 24] = rng.gen();
    format!("pylon_{}", hex_encode(&bytes))
}

impl ApiKeysPlugin {
    pub fn new() -> Self {
        Self {
            keys: Mutex::new(HashMap::new()),
        }
    }

    /// Create a new API key. Returns a [`CreatedApiKey`] containing the raw key.
    /// The raw key is **not** stored; only its SHA-256 hash is retained.
    pub fn create_key(&self, name: &str, user_id: &str, scopes: Vec<String>) -> CreatedApiKey {
        let raw_key = generate_key();
        let key_hash = hash_key(&raw_key);

        let api_key = ApiKey {
            key_hash: key_hash.clone(),
            name: name.to_string(),
            user_id: user_id.to_string(),
            scopes: scopes.clone(),
            created_at: now(),
        };
        self.keys.lock().unwrap().insert(key_hash, api_key);

        CreatedApiKey {
            raw_key,
            name: name.to_string(),
            user_id: user_id.to_string(),
            scopes,
            created_at: now(),
        }
    }

    /// Resolve an API key to an auth context.
    /// Hashes the provided key and performs an O(1) HashMap lookup.
    ///
    /// The returned `AuthContext` is DETACHED from this store — if the key
    /// is later revoked, callers holding the context won't see the change.
    /// This matters for middleware/session layers that cache the resolved
    /// context across requests. Such callers should also call
    /// [`is_active`] on every request or re-`resolve` to pick up
    /// revocations.
    pub fn resolve(&self, key: &str) -> Option<AuthContext> {
        let h = hash_key(key);
        let keys = self.keys.lock().unwrap();
        keys.get(&h)
            .map(|k| AuthContext::authenticated(k.user_id.clone()))
    }

    /// Returns true if the raw key still exists in the store. Use this to
    /// validate a cached `AuthContext` against the current revocation state
    /// before trusting it on a subsequent request.
    pub fn is_active(&self, key: &str) -> bool {
        let h = hash_key(key);
        self.keys.lock().unwrap().contains_key(&h)
    }

    /// Check if an API key has a specific scope.
    pub fn has_scope(&self, key: &str, scope: &str) -> bool {
        let h = hash_key(key);
        let keys = self.keys.lock().unwrap();
        keys.get(&h)
            .map(|k| k.scopes.is_empty() || k.scopes.iter().any(|s| s == scope || s == "*"))
            .unwrap_or(false)
    }

    /// Revoke an API key. The caller must provide the raw key; it is hashed
    /// internally to locate the stored entry.
    pub fn revoke(&self, key: &str) -> bool {
        let h = hash_key(key);
        self.keys.lock().unwrap().remove(&h).is_some()
    }

    /// List all API keys for a user.
    ///
    /// Returns [`ApiKey`] entries which contain the hash but **not** the raw key.
    pub fn list_keys(&self, user_id: &str) -> Vec<ApiKey> {
        self.keys
            .lock()
            .unwrap()
            .values()
            .filter(|k| k.user_id == user_id)
            .cloned()
            .collect()
    }
}

impl Plugin for ApiKeysPlugin {
    fn name(&self) -> &str {
        "api-keys"
    }

    fn on_request(
        &self,
        _method: &str,
        _path: &str,
        _auth: &AuthContext,
    ) -> Result<(), PluginError> {
        // API keys are resolved at the auth layer, not here.
        // This hook could be used for scope checking per-route if needed.
        Ok(())
    }
}

fn now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    format!(
        "{}Z",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_resolve() {
        let plugin = ApiKeysPlugin::new();
        let created = plugin.create_key("test-key", "user-1", vec!["read".into(), "write".into()]);
        assert!(created.raw_key.starts_with("pylon_"));

        let ctx = plugin.resolve(&created.raw_key).unwrap();
        assert_eq!(ctx.user_id, Some("user-1".into()));
    }

    #[test]
    fn raw_key_not_stored() {
        let plugin = ApiKeysPlugin::new();
        let created = plugin.create_key("test-key", "user-1", vec![]);
        let keys = plugin.keys.lock().unwrap();
        // The HashMap key should be a hash, not the raw key.
        assert!(!keys.contains_key(&created.raw_key));
        // The stored ApiKey should not contain the raw key in any field.
        let stored = keys.values().next().unwrap();
        assert_ne!(stored.key_hash, created.raw_key);
        assert_eq!(stored.key_hash, hash_key(&created.raw_key));
    }

    #[test]
    fn resolve_invalid_key() {
        let plugin = ApiKeysPlugin::new();
        assert!(plugin.resolve("invalid").is_none());
    }

    #[test]
    fn scope_checking() {
        let plugin = ApiKeysPlugin::new();
        let created = plugin.create_key("test", "user-1", vec!["read".into()]);

        assert!(plugin.has_scope(&created.raw_key, "read"));
        assert!(!plugin.has_scope(&created.raw_key, "write"));
    }

    #[test]
    fn wildcard_scope() {
        let plugin = ApiKeysPlugin::new();
        let created = plugin.create_key("admin", "user-1", vec!["*".into()]);

        assert!(plugin.has_scope(&created.raw_key, "read"));
        assert!(plugin.has_scope(&created.raw_key, "write"));
        assert!(plugin.has_scope(&created.raw_key, "anything"));
    }

    #[test]
    fn empty_scopes_allows_all() {
        let plugin = ApiKeysPlugin::new();
        let created = plugin.create_key("full-access", "user-1", vec![]);

        assert!(plugin.has_scope(&created.raw_key, "read"));
        assert!(plugin.has_scope(&created.raw_key, "write"));
    }

    #[test]
    fn revoke_key() {
        let plugin = ApiKeysPlugin::new();
        let created = plugin.create_key("test", "user-1", vec![]);

        assert!(plugin.revoke(&created.raw_key));
        assert!(plugin.resolve(&created.raw_key).is_none());
        assert!(!plugin.revoke(&created.raw_key)); // already revoked
    }

    #[test]
    fn is_active_tracks_revocation() {
        let plugin = ApiKeysPlugin::new();
        let created = plugin.create_key("test", "user-1", vec![]);
        assert!(plugin.is_active(&created.raw_key));
        plugin.revoke(&created.raw_key);
        assert!(!plugin.is_active(&created.raw_key));
    }

    #[test]
    fn list_keys_by_user() {
        let plugin = ApiKeysPlugin::new();
        plugin.create_key("key1", "user-1", vec![]);
        plugin.create_key("key2", "user-1", vec![]);
        plugin.create_key("key3", "user-2", vec![]);

        let keys = plugin.list_keys("user-1");
        assert_eq!(keys.len(), 2);

        let keys = plugin.list_keys("user-2");
        assert_eq!(keys.len(), 1);
    }

    #[test]
    fn generated_keys_are_unique() {
        let k1 = generate_key();
        let k2 = generate_key();
        assert_ne!(k1, k2);
        assert!(k1.starts_with("pylon_"));
        // "pylon_" (11) + 48 hex chars (24 bytes) = 59
        assert_eq!(k1.len(), 59);
    }

    #[test]
    fn hash_key_is_deterministic() {
        let h1 = hash_key("test_key_123");
        let h2 = hash_key("test_key_123");
        assert_eq!(h1, h2);
    }

    #[test]
    fn hash_key_differs_for_different_inputs() {
        let h1 = hash_key("key_a");
        let h2 = hash_key("key_b");
        assert_ne!(h1, h2);
    }

    #[test]
    fn hash_key_is_sha256_length() {
        let h = hash_key("test");
        // SHA-256 = 32 bytes = 64 hex chars
        assert_eq!(h.len(), 64);
    }
}
