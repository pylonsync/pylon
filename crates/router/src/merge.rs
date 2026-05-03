//! Anonymous → authenticated user data merge.
//!
//! When a guest accumulates state (cart items, drafts, settings) under an
//! anonymous session, then signs in via magic-code or OAuth, the natural
//! expectation is that their work moves with them. Without this, the
//! authenticated session sees an empty cart and the guest user_id ends
//! up with orphan rows that no one can reach.
//!
//! This module exposes a single helper, [`transfer_user_ownership`], that
//! walks the manifest and rewrites every `id(<user_entity>)` field on
//! every entity from `from_user_id` → `to_user_id`. Cart-level granularity
//! (i.e. "move only carts, not audit rows") would require app-level config
//! that's not in the manifest today; the universal default is correct for
//! the vast majority of apps because anonymous sessions don't generate
//! audit rows in the first place.
//!
//! Called from `/api/auth/magic/verify` and the OAuth callback when the
//! request arrives carrying a guest session and lands on a real user_id.
//!
//! Performance: walks N entities × M rows-per-entity worst case. Anonymous
//! merges happen at most once per user lifetime and the row counts are
//! tiny (a guest who put 200 things in their cart is not realistic), so
//! the per-row update loop is fine. A bulk SQL `UPDATE … WHERE userId = ?`
//! would be faster but would force every backend (`Runtime`, `D1DataStore`,
//! future Workers backends) to implement a new trait method just for this
//! cold path.

use pylon_http::DataStore;
use pylon_kernel::AppManifest;
use serde_json::json;

/// Outcome of a single merge run. Surfaced in the audit log so an operator
/// can answer "did the user's cart actually move?" without spelunking
/// through SQL.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergeResult {
    /// Total rows whose owner field was rewritten across all entities.
    pub rows_updated: usize,
    /// Per-entity breakdown: `(entity_name, field_name, rows_updated)`.
    /// Empty when no entity references the user entity.
    pub touched: Vec<(String, String, usize)>,
}

impl MergeResult {
    /// Compact CSV summary for audit-log metadata. Format:
    /// `Cart.userId:3,Order.buyerId:1`. Empty string when nothing moved.
    pub fn entities_csv(&self) -> String {
        self.touched
            .iter()
            .map(|(entity, field, rows)| format!("{entity}.{field}:{rows}"))
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// Find all `(entity, field)` pairs in `manifest` where the field's type
/// is exactly `id(<user_entity>)`. Pure — no I/O. Exposed for tests and
/// for the audit metadata so an operator can see what surface the merge
/// will touch before triggering it.
pub fn find_user_referencing_fields(
    manifest: &AppManifest,
    user_entity: &str,
) -> Vec<(String, String)> {
    let needle = format!("id({user_entity})");
    let mut out = Vec::new();
    for entity in &manifest.entities {
        // Skip the user entity itself — a User row pointing at another
        // User is "manager" / "invitedBy" / similar relational data, not
        // ownership. Rewriting those would corrupt the org chart on
        // every login.
        if entity.name == user_entity {
            continue;
        }
        for field in &entity.fields {
            if field.field_type == needle {
                out.push((entity.name.clone(), field.name.clone()));
            }
        }
    }
    out
}

/// Rewrite every `id(<user_entity>)` field whose value equals `from_user_id`
/// to `to_user_id`. Runs each entity sequentially via `query_filtered` +
/// per-row `update`.
///
/// Errors from individual entity updates are swallowed and counted as
/// "0 rows updated for that entity" — a partial merge is preferable to
/// rolling back the magic-link sign-in itself, since the user is now
/// authenticated and re-running the merge later (via a separate hook) is
/// safer than blocking the login.
///
/// Returns a [`MergeResult`] describing what moved. Caller is responsible
/// for the audit-log entry and for deciding whether to delete the (now
/// empty) guest user row.
pub fn transfer_user_ownership(
    store: &dyn DataStore,
    manifest: &AppManifest,
    from_user_id: &str,
    to_user_id: &str,
    user_entity: &str,
) -> MergeResult {
    if from_user_id == to_user_id {
        return MergeResult::default();
    }
    let mut result = MergeResult::default();
    for (entity, field) in find_user_referencing_fields(manifest, user_entity) {
        let filter = json!({ &field: from_user_id });
        let rows = match store.query_filtered(&entity, &filter) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let mut updated_here = 0usize;
        for row in rows {
            let id = match row.get("id").and_then(|v| v.as_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };
            let patch = json!({ &field: to_user_id });
            if store.update(&entity, &id, &patch).is_ok() {
                updated_here += 1;
            }
        }
        if updated_here > 0 {
            result.rows_updated += updated_here;
            result.touched.push((entity, field, updated_here));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_kernel::{
        AppManifest, ManifestAuthConfig, ManifestAuthUserConfig, ManifestEntity, ManifestField,
    };

    fn field(name: &str, ty: &str) -> ManifestField {
        ManifestField {
            name: name.into(),
            field_type: ty.into(),
            optional: false,
            unique: false,
            crdt: None,
        }
    }

    fn entity(name: &str, fields: Vec<ManifestField>) -> ManifestEntity {
        ManifestEntity {
            name: name.into(),
            fields,
            indexes: vec![],
            relations: vec![],
            search: None,
            crdt: true,
        }
    }

    fn manifest(entities: Vec<ManifestEntity>) -> AppManifest {
        AppManifest {
            manifest_version: 1,
            name: "test".into(),
            version: "0".into(),
            entities,
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            auth: ManifestAuthConfig {
                user: ManifestAuthUserConfig {
                    entity: "User".into(),
                    expose: vec![],
                    hide: vec![],
                },
                ..Default::default()
            },
        }
    }

    #[test]
    fn finds_id_user_fields_across_entities() {
        let m = manifest(vec![
            entity(
                "Cart",
                vec![field("userId", "id(User)"), field("createdAt", "datetime")],
            ),
            entity(
                "Order",
                vec![field("buyerId", "id(User)"), field("total", "int")],
            ),
            entity("Product", vec![field("name", "string")]),
        ]);
        let mut found = find_user_referencing_fields(&m, "User");
        found.sort();
        assert_eq!(
            found,
            vec![
                ("Cart".to_string(), "userId".to_string()),
                ("Order".to_string(), "buyerId".to_string()),
            ]
        );
    }

    #[test]
    fn skips_self_reference_on_user_entity() {
        // A User.invitedBy field referencing another User is not an
        // ownership relationship — rewriting it on anonymous merge would
        // corrupt invite chains.
        let m = manifest(vec![entity(
            "User",
            vec![field("invitedBy", "id(User)"), field("email", "string")],
        )]);
        assert!(find_user_referencing_fields(&m, "User").is_empty());
    }

    #[test]
    fn ignores_unrelated_id_references() {
        let m = manifest(vec![entity(
            "Comment",
            vec![field("postId", "id(Post)"), field("author", "id(User)")],
        )]);
        let found = find_user_referencing_fields(&m, "User");
        assert_eq!(found, vec![("Comment".to_string(), "author".to_string())]);
    }

    #[test]
    fn respects_user_entity_override() {
        // App configured `auth.user.entity = "Account"` — the User name
        // is the manifest's call.
        let m = manifest(vec![
            entity("Cart", vec![field("ownerId", "id(Account)")]),
            entity("Decoy", vec![field("uid", "id(User)")]),
        ]);
        let found = find_user_referencing_fields(&m, "Account");
        assert_eq!(found, vec![("Cart".to_string(), "ownerId".to_string())]);
    }
}
