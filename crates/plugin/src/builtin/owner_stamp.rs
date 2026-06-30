//! Server-authoritative ownership via automatic owner-id stamping.
//!
//! This is the plugin behind `field.X().owner()`. It exists so that
//! *optimistic, local-first writes can be the default* for owned data
//! without a security hole: a client `db.insert("Offer", { buyerId, … })`
//! paints the row in the local store instantly (no server round-trip),
//! and the server still guarantees `buyerId` is the caller — a forged id
//! is rejected before the row lands.
//!
//! How it works:
//! 1. `from_manifest` scans every entity for fields whose manifest
//!    `default` is the auth sentinel `{"$auth":"userId"}` (emitted by
//!    `field.X().owner()` in @pylonsync/sdk). Those become the entity's
//!    owner fields. Entities with none behave normally.
//! 2. Before insert, for each owner field:
//!    - missing / empty / non-string → stamp `auth.user_id`;
//!    - equal to `auth.user_id` → pass through (this is the common
//!      path: the client supplied its OWN id so the optimistic ghost
//!      and the canonical row share a value, no flash);
//!    - a different non-empty value from a non-admin caller → reject
//!      with `OWNER_MISMATCH` (403). Admin contexts (migrations,
//!      tooling) keep the explicit value.
//!    - no `auth.user_id` at all (truly anonymous) → reject with
//!      `OWNER_REQUIRED` (401): you can't own a row anonymously.
//!
//! Update-side protection is handled separately: `field.X().owner()`
//! also sets `readonly: true`, so the framework's readonly HTTP gate
//! rejects any `PATCH`/`/api/transact` payload that tries to reassign
//! the owner. Together they close the canonical IDOR shape where a
//! policy gates on `data.ownerId == auth.userId` but the attacker sends
//! someone else's id at create OR flips it via a later update.
//!
//! Mirrors [`super::tenant_scope::TenantScopePlugin`] deliberately — the
//! tenant stamp and the owner stamp are the same shape of problem
//! ("stamp an identity column from the session, never trust the
//! client's value") and share its battle-tested override semantics.

use std::collections::HashMap;

use serde_json::Value;

use crate::{Plugin, PluginError};
use pylon_auth::AuthContext;

/// Per-entity owner-field configuration. An entity can in principle
/// declare more than one owner-shaped field, so we carry a list.
#[derive(Debug, Clone, Default)]
pub struct OwnerScopeConfig {
    pub fields: Vec<String>,
}

pub struct OwnerStampPlugin {
    /// entity → which field(s) carry the owner id
    scopes: HashMap<String, OwnerScopeConfig>,
}

impl OwnerStampPlugin {
    pub fn new() -> Self {
        Self {
            scopes: HashMap::new(),
        }
    }

    /// Auto-configure from a manifest: any field whose `default` is the
    /// owner sentinel `{"$auth":"userId"}` marks its entity as
    /// owner-scoped on that field. This is how the server registers the
    /// plugin by default — `field.X().owner()` in the schema is the only
    /// signal needed, no separate config step.
    pub fn from_manifest(manifest: &pylon_kernel::AppManifest) -> Self {
        let mut plugin = Self::new();
        for entity in &manifest.entities {
            let mut fields = Vec::new();
            for field in &entity.fields {
                if field
                    .default
                    .as_ref()
                    .map(is_owner_sentinel)
                    .unwrap_or(false)
                {
                    fields.push(field.name.clone());
                }
            }
            if !fields.is_empty() {
                plugin
                    .scopes
                    .insert(entity.name.clone(), OwnerScopeConfig { fields });
            }
        }
        plugin
    }

    /// Mark `entity`'s `field` as owner-stamped (test/programmatic use).
    pub fn scope(&mut self, entity: impl Into<String>, field: impl Into<String>) -> &mut Self {
        self.scopes
            .entry(entity.into())
            .or_default()
            .fields
            .push(field.into());
        self
    }

    pub fn is_scoped(&self, entity: &str) -> bool {
        self.scopes.contains_key(entity)
    }

    pub fn fields_for(&self, entity: &str) -> Option<&[String]> {
        self.scopes.get(entity).map(|c| c.fields.as_slice())
    }

    /// Stamp the owner id onto a row about to be inserted.
    ///
    /// Returns `Err` if the entity is owner-scoped but the caller is
    /// anonymous (no `user_id`), or if a non-admin caller supplied a
    /// different owner id than their own.
    pub fn stamp_insert(
        &self,
        entity: &str,
        data: &mut Value,
        auth: &AuthContext,
    ) -> Result<(), OwnerError> {
        let Some(fields) = self.fields_for(entity) else {
            return Ok(());
        };
        // A row that's owner-scoped on ANY field needs an identity.
        // Guests count — they carry a stable `user_id` — only truly
        // anonymous (no session) callers are turned away.
        let owner_id = match auth.user_id.as_deref().filter(|s| !s.is_empty()) {
            Some(id) => id,
            None => return Err(OwnerError::MissingOwner),
        };
        let Some(obj) = data.as_object_mut() else {
            return Ok(());
        };
        for field in fields {
            let provided = obj.get(field);
            let provided_str = provided.and_then(|v| v.as_str()).filter(|s| !s.is_empty());
            match provided_str {
                None => {
                    // Absent / empty / non-string → stamp from the session.
                    obj.insert(field.clone(), Value::String(owner_id.to_string()));
                }
                Some(supplied) if supplied == owner_id => {
                    // Client supplied its OWN id (the optimistic-ghost
                    // path). Keep it — the ghost and canonical row match.
                }
                Some(_other) => {
                    if !auth.is_admin {
                        return Err(OwnerError::WrongOwner);
                    }
                    // Admin context: keep the explicit value (impersonation
                    // for migrations / tooling).
                }
            }
        }
        Ok(())
    }
}

impl Default for OwnerStampPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for OwnerStampPlugin {
    fn name(&self) -> &str {
        "owner_stamp"
    }

    fn before_insert(
        &self,
        entity: &str,
        data: &mut Value,
        auth: &AuthContext,
    ) -> Result<(), PluginError> {
        match self.stamp_insert(entity, data, auth) {
            Ok(()) => Ok(()),
            Err(OwnerError::MissingOwner) => Err(PluginError {
                code: "OWNER_REQUIRED".into(),
                message: format!(
                    "Entity \"{entity}\" has an owner field (field.owner()); the caller is \
                     anonymous. Sign in (or establish a guest session) before creating one."
                ),
                status: 401,
            }),
            Err(OwnerError::WrongOwner) => Err(PluginError {
                code: "OWNER_MISMATCH".into(),
                message: format!(
                    "Refused insert into \"{entity}\": an owner field was set to a different \
                     user than the caller. The framework stamps owner fields from the session — \
                     don't send someone else's id."
                ),
                status: 403,
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnerError {
    MissingOwner,
    WrongOwner,
}

/// `true` iff a manifest field `default` is the owner sentinel
/// `{"$auth":"userId"}` emitted by `field.X().owner()`.
fn is_owner_sentinel(default: &Value) -> bool {
    default
        .as_object()
        .and_then(|o| o.get("$auth"))
        .and_then(|v| v.as_str())
        == Some("userId")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn manifest_with_owner(entity: &str, field: &str) -> pylon_kernel::AppManifest {
        let mut m = pylon_kernel::AppManifest::default();
        m.entities.push(pylon_kernel::ManifestEntity {
            name: entity.into(),
            fields: vec![pylon_kernel::ManifestField {
                name: field.into(),
                field_type: "string".into(),
                optional: false,
                unique: false,
                crdt: None,
                server_only: false,
                readonly: true,
                default: Some(json!({ "$auth": "userId" })),
                enum_values: None,
                encrypted: false,
            }],
            indexes: vec![],
            relations: vec![],
            search: None,
            crdt: true,
            sync: true,
        });
        m
    }

    #[test]
    fn from_manifest_detects_owner_fields() {
        let m = manifest_with_owner("Listing", "sellerId");
        let p = OwnerStampPlugin::from_manifest(&m);
        assert!(p.is_scoped("Listing"));
        assert_eq!(
            p.fields_for("Listing"),
            Some(["sellerId".to_string()].as_slice())
        );
    }

    #[test]
    fn from_manifest_ignores_non_owner_defaults() {
        let mut m = manifest_with_owner("Listing", "sellerId");
        // A plain literal default + a "now" default must NOT be treated
        // as owner fields.
        m.entities[0].fields.push(pylon_kernel::ManifestField {
            name: "status".into(),
            field_type: "string".into(),
            optional: false,
            unique: false,
            crdt: None,
            server_only: false,
            readonly: false,
            default: Some(json!("active")),
            enum_values: None,
            encrypted: false,
        });
        m.entities[0].fields.push(pylon_kernel::ManifestField {
            name: "createdAt".into(),
            field_type: "datetime".into(),
            optional: false,
            unique: false,
            crdt: None,
            server_only: false,
            readonly: false,
            default: Some(json!("now")),
            enum_values: None,
            encrypted: false,
        });
        let p = OwnerStampPlugin::from_manifest(&m);
        assert_eq!(
            p.fields_for("Listing"),
            Some(["sellerId".to_string()].as_slice())
        );
    }

    #[test]
    fn unscoped_entity_passes_through() {
        let p = OwnerStampPlugin::new();
        let mut data = json!({ "title": "x" });
        p.stamp_insert("Doc", &mut data, &AuthContext::user("u1".into()))
            .unwrap();
        assert_eq!(data, json!({ "title": "x" }));
    }

    #[test]
    fn stamps_owner_when_absent() {
        let p = OwnerStampPlugin::from_manifest(&manifest_with_owner("Listing", "sellerId"));
        let mut data = json!({ "title": "Aeron" });
        p.stamp_insert("Listing", &mut data, &AuthContext::user("u1".into()))
            .unwrap();
        assert_eq!(data["sellerId"], "u1");
    }

    #[test]
    fn empty_owner_string_is_stamped() {
        let p = OwnerStampPlugin::from_manifest(&manifest_with_owner("Listing", "sellerId"));
        let mut data = json!({ "sellerId": "" });
        p.stamp_insert("Listing", &mut data, &AuthContext::user("u1".into()))
            .unwrap();
        assert_eq!(data["sellerId"], "u1");
    }

    #[test]
    fn own_id_passes_through_for_the_optimistic_ghost() {
        // The whole point of field.owner(): the client supplies its own
        // id so the optimistic ghost matches the canonical row. That must
        // be allowed (not flagged as a spoof).
        let p = OwnerStampPlugin::from_manifest(&manifest_with_owner("Listing", "sellerId"));
        let mut data = json!({ "sellerId": "u1", "title": "Aeron" });
        p.stamp_insert("Listing", &mut data, &AuthContext::user("u1".into()))
            .unwrap();
        assert_eq!(data["sellerId"], "u1");
    }

    /// The core security property: a non-admin caller cannot plant a row
    /// owned by someone else. Without the stamp, a policy of
    /// `data.sellerId == auth.userId` is trivially satisfied by lying.
    #[test]
    fn spoofed_owner_is_rejected_for_non_admin() {
        let p = OwnerStampPlugin::from_manifest(&manifest_with_owner("Listing", "sellerId"));
        let mut data = json!({ "sellerId": "victim", "title": "Aeron" });
        let err = p
            .stamp_insert("Listing", &mut data, &AuthContext::user("attacker".into()))
            .unwrap_err();
        assert_eq!(err, OwnerError::WrongOwner);
    }

    #[test]
    fn admin_may_set_an_explicit_owner() {
        let p = OwnerStampPlugin::from_manifest(&manifest_with_owner("Listing", "sellerId"));
        let mut data = json!({ "sellerId": "some-user", "title": "Aeron" });
        p.stamp_insert("Listing", &mut data, &AuthContext::admin())
            .unwrap();
        assert_eq!(data["sellerId"], "some-user");
    }

    #[test]
    fn guest_id_is_stamped() {
        // Guests carry a stable user_id and legitimately own their rows
        // (the marketplace is guest-driven). Stamp it like any other.
        let p = OwnerStampPlugin::from_manifest(&manifest_with_owner("Listing", "sellerId"));
        let mut data = json!({ "title": "Aeron" });
        p.stamp_insert(
            "Listing",
            &mut data,
            &AuthContext::guest("guest_abc".into()),
        )
        .unwrap();
        assert_eq!(data["sellerId"], "guest_abc");
    }

    #[test]
    fn anonymous_insert_is_rejected() {
        let p = OwnerStampPlugin::from_manifest(&manifest_with_owner("Listing", "sellerId"));
        let mut data = json!({ "title": "Aeron" });
        let err = p
            .stamp_insert("Listing", &mut data, &AuthContext::anonymous())
            .unwrap_err();
        assert_eq!(err, OwnerError::MissingOwner);
    }

    #[test]
    fn before_insert_maps_errors_to_http_status() {
        let p = OwnerStampPlugin::from_manifest(&manifest_with_owner("Listing", "sellerId"));

        let mut spoof = json!({ "sellerId": "victim" });
        let err = p
            .before_insert("Listing", &mut spoof, &AuthContext::user("attacker".into()))
            .unwrap_err();
        assert_eq!(err.code, "OWNER_MISMATCH");
        assert_eq!(err.status, 403);

        let mut anon = json!({});
        let err = p
            .before_insert("Listing", &mut anon, &AuthContext::anonymous())
            .unwrap_err();
        assert_eq!(err.code, "OWNER_REQUIRED");
        assert_eq!(err.status, 401);
    }
}
