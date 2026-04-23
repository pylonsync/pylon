//! Row-level multi-tenancy via automatic `tenantId` injection.
//!
//! Pairs with `OrganizationsPlugin`: orgs answer "who belongs to what",
//! `TenantScopePlugin` answers "which rows belong to what" by stamping every
//! insert with the active tenant id.
//!
//! How it works:
//! 1. Configure which entities are tenant-scoped, plus the column name
//!    (default `tenantId`). Untouched entities behave normally.
//! 2. Before insert, the plugin sets `data.tenantId = auth.tenant_id` if
//!    the field is missing or empty.
//! 3. Before update/delete, the plugin checks the existing row's tenant
//!    matches the caller's tenant — cross-tenant writes are rejected by
//!    returning an `Err` from the hook (the runtime translates this to a
//!    403 response).
//!
//! This plugin does NOT enforce reads. Use `pylon-policy` expressions for
//! that — they have access to `auth.tenantId` and can scope `query` and
//! `lookup` calls. The asymmetry is intentional: writes need the tenant id
//! anyway (to stamp the row), so enforcing them here is free; reads need
//! the user-defined policy expression engine because filtering rules can
//! get arbitrarily complex.

use std::collections::HashMap;

use serde_json::Value;

use crate::Plugin;
use pylon_auth::AuthContext;

/// Per-entity tenant scoping configuration.
#[derive(Debug, Clone)]
pub struct TenantScopeConfig {
    pub field: String,
}

impl Default for TenantScopeConfig {
    fn default() -> Self {
        Self {
            field: "tenantId".into(),
        }
    }
}

pub struct TenantScopePlugin {
    /// entity → which field carries the tenant id
    scopes: HashMap<String, TenantScopeConfig>,
}

impl TenantScopePlugin {
    pub fn new() -> Self {
        Self {
            scopes: HashMap::new(),
        }
    }

    /// Auto-configure from a manifest: any entity that declares a `tenantId`
    /// (or `tenant_id`) field is marked tenant-scoped using that field.
    ///
    /// This is how the server registers the plugin by default. Adding a
    /// `tenantId` field to an entity is the signal — no separate config
    /// step required. Apps that use a different field name (e.g. `orgId`)
    /// can still call [`scope_with_field`] after this to customize.
    pub fn from_manifest(manifest: &pylon_kernel::AppManifest) -> Self {
        let mut plugin = Self::new();
        for entity in &manifest.entities {
            for field in &entity.fields {
                if field.name == "tenantId" || field.name == "tenant_id" {
                    plugin.scopes.insert(
                        entity.name.clone(),
                        TenantScopeConfig {
                            field: field.name.clone(),
                        },
                    );
                    break;
                }
            }
        }
        plugin
    }

    /// Mark `entity` as tenant-scoped using the default `tenantId` field.
    pub fn scope(&mut self, entity: impl Into<String>) -> &mut Self {
        self.scopes.insert(entity.into(), TenantScopeConfig::default());
        self
    }

    /// Mark `entity` as tenant-scoped using a custom field name.
    pub fn scope_with_field(
        &mut self,
        entity: impl Into<String>,
        field: impl Into<String>,
    ) -> &mut Self {
        self.scopes.insert(
            entity.into(),
            TenantScopeConfig {
                field: field.into(),
            },
        );
        self
    }

    pub fn is_scoped(&self, entity: &str) -> bool {
        self.scopes.contains_key(entity)
    }

    pub fn field_for(&self, entity: &str) -> Option<&str> {
        self.scopes.get(entity).map(|c| c.field.as_str())
    }

    /// Stamp `tenantId` onto a row that's about to be inserted.
    /// Returns `Err` if the entity is scoped but the caller has no tenant.
    pub fn stamp_insert(
        &self,
        entity: &str,
        data: &mut Value,
        auth: &AuthContext,
    ) -> Result<(), TenantError> {
        let Some(field) = self.field_for(entity) else {
            return Ok(());
        };
        let tenant_id = match auth_tenant_id(auth) {
            Some(t) => t,
            None => return Err(TenantError::MissingTenant),
        };
        if let Some(obj) = data.as_object_mut() {
            // Only inject when caller didn't provide one — let admin tooling
            // override by being explicit.
            let needs_set = obj
                .get(field)
                .map(|v| v.is_null() || v.as_str().map_or(false, str::is_empty))
                .unwrap_or(true);
            if needs_set {
                obj.insert(field.into(), Value::String(tenant_id.into()));
            }
        }
        Ok(())
    }

    /// Verify that an existing row belongs to the caller's tenant before
    /// allowing a mutation.
    pub fn check_write(
        &self,
        entity: &str,
        existing: &Value,
        auth: &AuthContext,
    ) -> Result<(), TenantError> {
        let Some(field) = self.field_for(entity) else {
            return Ok(());
        };
        let tenant = auth_tenant_id(auth).ok_or(TenantError::MissingTenant)?;
        let row_tenant = existing
            .as_object()
            .and_then(|o| o.get(field))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if row_tenant.is_empty() {
            // Unscoped legacy row — refuse to mutate from a tenant context to
            // avoid accidentally claiming someone else's data.
            return Err(TenantError::WrongTenant);
        }
        if row_tenant != tenant {
            return Err(TenantError::WrongTenant);
        }
        Ok(())
    }
}

impl Default for TenantScopePlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for TenantScopePlugin {
    fn name(&self) -> &str {
        "tenant_scope"
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TenantError {
    MissingTenant,
    WrongTenant,
}

/// Extract the active tenant id from auth context.
///
/// Looks for a session attribute called `tenant_id` (set by the app when the
/// user picks an org) — falling back to None. Apps that don't use the
/// attribute can subclass this resolution by setting `auth.tenant_id` via
/// AuthContext::with_tenant.
fn auth_tenant_id(auth: &AuthContext) -> Option<&str> {
    auth.tenant_id()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn auth_with_tenant(user: &str, tenant: &str) -> AuthContext {
        AuthContext::user(user.into()).with_tenant(tenant.into())
    }

    #[test]
    fn unscoped_entity_passes_through() {
        let p = TenantScopePlugin::new();
        let mut data = json!({"name": "x"});
        p.stamp_insert("Doc", &mut data, &auth_with_tenant("u1", "t1")).unwrap();
        assert_eq!(data["name"], "x");
        assert!(data.get("tenantId").is_none());
    }

    #[test]
    fn stamps_tenant_on_scoped_insert() {
        let mut p = TenantScopePlugin::new();
        p.scope("Doc");
        let mut data = json!({"title": "hi"});
        p.stamp_insert("Doc", &mut data, &auth_with_tenant("u1", "tA")).unwrap();
        assert_eq!(data["tenantId"], "tA");
    }

    #[test]
    fn does_not_overwrite_provided_tenant() {
        let mut p = TenantScopePlugin::new();
        p.scope("Doc");
        let mut data = json!({"tenantId": "explicit"});
        p.stamp_insert("Doc", &mut data, &auth_with_tenant("u1", "tA")).unwrap();
        assert_eq!(data["tenantId"], "explicit");
    }

    #[test]
    fn rejects_insert_without_tenant() {
        let mut p = TenantScopePlugin::new();
        p.scope("Doc");
        let mut data = json!({});
        let err = p
            .stamp_insert("Doc", &mut data, &AuthContext::user("u1".into()))
            .unwrap_err();
        assert_eq!(err, TenantError::MissingTenant);
    }

    #[test]
    fn allows_write_to_own_tenant_row() {
        let mut p = TenantScopePlugin::new();
        p.scope("Doc");
        let row = json!({"tenantId": "tA", "title": "x"});
        p.check_write("Doc", &row, &auth_with_tenant("u1", "tA")).unwrap();
    }

    #[test]
    fn rejects_write_to_other_tenant_row() {
        let mut p = TenantScopePlugin::new();
        p.scope("Doc");
        let row = json!({"tenantId": "tB", "title": "x"});
        let err = p
            .check_write("Doc", &row, &auth_with_tenant("u1", "tA"))
            .unwrap_err();
        assert_eq!(err, TenantError::WrongTenant);
    }

    #[test]
    fn rejects_write_to_legacy_unscoped_row() {
        let mut p = TenantScopePlugin::new();
        p.scope("Doc");
        let row = json!({"title": "x"}); // no tenantId
        let err = p
            .check_write("Doc", &row, &auth_with_tenant("u1", "tA"))
            .unwrap_err();
        assert_eq!(err, TenantError::WrongTenant);
    }

    #[test]
    fn custom_field_name_used() {
        let mut p = TenantScopePlugin::new();
        p.scope_with_field("Doc", "orgId");
        let mut data = json!({});
        p.stamp_insert("Doc", &mut data, &auth_with_tenant("u1", "tA")).unwrap();
        assert_eq!(data["orgId"], "tA");
        assert!(data.get("tenantId").is_none());
    }
}
