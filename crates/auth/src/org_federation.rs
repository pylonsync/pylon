//! Federated org mirroring — the relying-app half of Pylon-to-Pylon
//! tenant federation.
//!
//! A Pylon IdP puts the user's org memberships in the `orgs` claim
//! (`[{ id, name, slug?, role }]`, see [`crate::ExternalOrg`]). On every
//! login through the configured provider the relying app reconciles its
//! local Org rows and memberships against that claim:
//!
//!   - an org in the claim with no local mirror is created (keyed by the
//!     external-id field) and the user joins it with the mapped role;
//!   - an org in the claim with a local mirror: the user joins it, or
//!     their role is updated to the mapped role;
//!   - a local mirror whose name (or, with `slug_field`, slug) differs
//!     from the claim is updated to match, so upstream renames land on
//!     the next login;
//!   - with `remove_missing`, a mirrored org the user belongs to locally
//!     but that is absent from the claim loses the membership. The Org
//!     row is never deleted — other members may still hold it.
//!
//! A `None` claim (provider sent nothing) is a no-op. An empty claim is
//! a statement — the user belongs to no upstream org — and with
//! `remove_missing` it removes every mirrored membership.
//!
//! The decisions are pure ([`plan_mirror`]) so they are unit-testable;
//! [`mirror_external_orgs`] applies them through an [`OrgStore`].

use crate::org::{OrgRole, OrgStore};
use crate::ExternalOrg;
use pylon_kernel::ManifestAuthOrgFederation;

/// What a mirror pass did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MirrorReport {
    pub created: usize,
    pub joined: usize,
    pub role_changed: usize,
    pub removed: usize,
    /// Mirrors whose name or slug was updated from the claim.
    pub refreshed: usize,
    /// Claim orgs that could not be created or refreshed (store error,
    /// e.g. a slug another local org still holds). Logged, not fatal.
    pub failed: usize,
}

/// A local org row that mirrors an upstream org, whether or not the user
/// is a member of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalOrg {
    pub external_id: String,
    pub org_id: String,
    pub name: String,
    /// The value of `slug_field`, when the config sets one.
    pub slug: Option<String>,
}

/// One membership the relying app currently holds for the user, in a
/// mirrored org.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalMirror {
    pub org_id: String,
    pub external_id: String,
    pub role: OrgRole,
}

/// A step the mirror pass will take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirrorStep {
    Create {
        external: ExternalOrg,
        role: OrgRole,
    },
    /// Bring a mirror's metadata in line with the claim. Only the fields
    /// that changed are `Some`.
    Refresh {
        org_id: String,
        name: Option<String>,
        slug: Option<String>,
    },
    Join {
        org_id: String,
        role: OrgRole,
    },
    SetRole {
        org_id: String,
        role: OrgRole,
    },
    Remove {
        org_id: String,
    },
}

/// Map a claim role through the config: explicit `role_map` entry, else
/// the role itself, else `member`. `declared_roles` are the app's custom
/// roles (beyond owner/admin/member).
pub fn map_role(
    cfg: &ManifestAuthOrgFederation,
    claim_role: &str,
    declared_roles: &[String],
) -> OrgRole {
    let mapped = cfg
        .role_map
        .get(claim_role)
        .map(String::as_str)
        .unwrap_or(claim_role);
    OrgRole::from_declared(mapped, declared_roles).unwrap_or(OrgRole::Member)
}

/// Decide the steps for one user. `existing` holds the local mirror row
/// for each claim org that has one, whether or not the user is a member;
/// `memberships` are the user's current memberships in mirrored orgs.
pub fn plan_mirror(
    cfg: &ManifestAuthOrgFederation,
    declared_roles: &[String],
    claim: &[ExternalOrg],
    existing: &[LocalOrg],
    memberships: &[LocalMirror],
) -> Vec<MirrorStep> {
    let mut steps = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for ext in claim {
        if !seen.insert(ext.id.clone()) {
            continue; // duplicate entry in the claim
        }
        let role = map_role(cfg, &ext.role, declared_roles);
        let Some(local) = existing.iter().find(|l| l.external_id == ext.id) else {
            steps.push(MirrorStep::Create {
                external: ext.clone(),
                role,
            });
            continue;
        };
        if let Some(step) = plan_refresh(cfg, ext, local) {
            steps.push(step);
        }
        let org_id = local.org_id.clone();
        match memberships.iter().find(|m| m.org_id == org_id) {
            None => steps.push(MirrorStep::Join { org_id, role }),
            Some(m) if m.role != role => steps.push(MirrorStep::SetRole { org_id, role }),
            Some(_) => {}
        }
    }
    if cfg.remove_missing {
        for m in memberships {
            if !seen.contains(&m.external_id) {
                steps.push(MirrorStep::Remove {
                    org_id: m.org_id.clone(),
                });
            }
        }
    }
    steps
}

/// The metadata update a mirror needs to match the claim, if any.
///
/// The name is taken from the claim unless the claim sent none (the
/// parser then falls back to the id, which is not a name worth writing).
/// The slug is written only when the config names a `slug_field` and the
/// claim carries a non-empty slug; a claim without one leaves the mirror
/// as it is.
fn plan_refresh(
    cfg: &ManifestAuthOrgFederation,
    ext: &ExternalOrg,
    local: &LocalOrg,
) -> Option<MirrorStep> {
    let name = (ext.name != ext.id && !ext.name.is_empty() && ext.name != local.name)
        .then(|| ext.name.clone());
    let slug = cfg
        .slug_field
        .as_ref()
        .and(ext.slug.as_deref())
        .filter(|s| !s.is_empty() && local.slug.as_deref() != Some(*s))
        .map(String::from);
    if name.is_none() && slug.is_none() {
        return None;
    }
    Some(MirrorStep::Refresh {
        org_id: local.org_id.clone(),
        name,
        slug,
    })
}

/// Reconcile the user's local memberships against the IdP's claim.
pub fn mirror_external_orgs(
    orgs: &OrgStore,
    cfg: &ManifestAuthOrgFederation,
    declared_roles: &[String],
    user_id: &str,
    claim: Option<&[ExternalOrg]>,
) -> MirrorReport {
    let mut report = MirrorReport::default();
    let Some(claim) = claim else {
        return report;
    };
    let field = cfg.external_id_field.as_str();
    let memberships: Vec<LocalMirror> = orgs
        .list_external_for_user(field, user_id)
        .into_iter()
        .map(|(org, external_id, role)| LocalMirror {
            org_id: org.id,
            external_id,
            role,
        })
        .collect();
    let slug_field = cfg.slug_field.as_deref();
    let existing: Vec<LocalOrg> = claim
        .iter()
        .filter_map(|ext| {
            let org = orgs.find_by_external_id(field, &ext.id)?;
            let slug = slug_field.and_then(|f| orgs.field_of(&org.id, f));
            Some(LocalOrg {
                external_id: ext.id.clone(),
                org_id: org.id,
                name: org.name,
                slug,
            })
        })
        .collect();
    for step in plan_mirror(cfg, declared_roles, claim, &existing, &memberships) {
        match step {
            MirrorStep::Create { external, role } => {
                let slug = slug_field.zip(external.slug.as_deref().filter(|s| !s.is_empty()));
                match orgs.create_external(&external.name, field, &external.id, slug, user_id) {
                    Some(org) => {
                        orgs.add_member(&org.id, user_id, role);
                        report.created += 1;
                        report.joined += 1;
                    }
                    None => report.failed += 1,
                }
            }
            MirrorStep::Refresh { org_id, name, slug } => {
                let slug = slug_field.zip(slug.as_deref());
                if orgs.update_mirror(&org_id, name.as_deref(), slug) {
                    report.refreshed += 1;
                } else {
                    report.failed += 1;
                }
            }
            MirrorStep::Join { org_id, role } => {
                orgs.add_member(&org_id, user_id, role);
                report.joined += 1;
            }
            MirrorStep::SetRole { org_id, role } => {
                if orgs.set_role(&org_id, user_id, role) {
                    report.role_changed += 1;
                }
            }
            MirrorStep::Remove { org_id } => {
                if orgs.remove_member(&org_id, user_id) {
                    report.removed += 1;
                }
            }
        }
    }
    if report != MirrorReport::default() {
        tracing::info!(
            "[org] federation mirror for user={user_id}: created={} joined={} role_changed={} removed={} refreshed={} failed={}",
            report.created, report.joined, report.role_changed, report.removed, report.refreshed, report.failed
        );
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(remove_missing: bool) -> ManifestAuthOrgFederation {
        ManifestAuthOrgFederation {
            provider: "stack0".into(),
            external_id_field: "externalId".into(),
            remove_missing,
            role_map: [("owner".to_string(), "admin".to_string())]
                .into_iter()
                .collect(),
            disable_local_create: true,
            slug_field: None,
        }
    }
    fn local(ext: &str, org: &str) -> LocalOrg {
        LocalOrg {
            external_id: ext.into(),
            org_id: org.into(),
            name: format!("Org {ext}"),
            slug: None,
        }
    }
    fn ext(id: &str, role: &str) -> ExternalOrg {
        ExternalOrg {
            id: id.into(),
            name: format!("Org {id}"),
            slug: None,
            role: role.into(),
        }
    }
    fn mem(org: &str, ext: &str, role: OrgRole) -> LocalMirror {
        LocalMirror {
            org_id: org.into(),
            external_id: ext.into(),
            role,
        }
    }

    #[test]
    fn creates_joins_and_updates_roles() {
        let steps = plan_mirror(
            &cfg(true),
            &[],
            &[ext("a", "member"), ext("b", "admin"), ext("c", "owner")],
            &[local("b", "org-b"), local("c", "org-c")],
            &[mem("org-c", "c", OrgRole::Member)],
        );
        assert_eq!(
            steps,
            vec![
                MirrorStep::Create {
                    external: ext("a", "member"),
                    role: OrgRole::Member
                },
                MirrorStep::Join {
                    org_id: "org-b".into(),
                    role: OrgRole::Admin
                },
                // owner → admin through the role map
                MirrorStep::SetRole {
                    org_id: "org-c".into(),
                    role: OrgRole::Admin
                },
            ]
        );
    }

    #[test]
    fn removes_missing_only_when_configured() {
        let memberships = [mem("org-old", "old", OrgRole::Member)];
        let on = plan_mirror(&cfg(true), &[], &[], &[], &memberships);
        assert_eq!(
            on,
            vec![MirrorStep::Remove {
                org_id: "org-old".into()
            }]
        );
        let off = plan_mirror(&cfg(false), &[], &[], &[], &memberships);
        assert!(off.is_empty());
    }

    #[test]
    fn a_matching_membership_is_a_no_op_and_duplicates_collapse() {
        let steps = plan_mirror(
            &cfg(true),
            &[],
            &[ext("a", "member"), ext("a", "member")],
            &[local("a", "org-a")],
            &[mem("org-a", "a", OrgRole::Member)],
        );
        assert!(steps.is_empty());
    }

    #[test]
    fn a_renamed_org_is_refreshed_before_the_membership_step() {
        let mut claim = ext("a", "member");
        claim.name = "Acme Inc".into();
        let steps = plan_mirror(&cfg(true), &[], &[claim], &[local("a", "org-a")], &[]);
        assert_eq!(
            steps,
            vec![
                MirrorStep::Refresh {
                    org_id: "org-a".into(),
                    name: Some("Acme Inc".into()),
                    slug: None,
                },
                MirrorStep::Join {
                    org_id: "org-a".into(),
                    role: OrgRole::Member
                },
            ]
        );
    }

    #[test]
    fn slugs_mirror_only_with_a_slug_field() {
        let mut claim = ext("a", "member");
        claim.slug = Some("acme".into());
        let member = [mem("org-a", "a", OrgRole::Member)];

        let off = plan_mirror(
            &cfg(true),
            &[],
            &[claim.clone()],
            &[local("a", "org-a")],
            &member,
        );
        assert!(off.is_empty(), "no slug_field: slug ignored, {off:?}");

        let mut with_slug = cfg(true);
        with_slug.slug_field = Some("slug".into());
        let on = plan_mirror(
            &with_slug,
            &[],
            &[claim.clone()],
            &[local("a", "org-a")],
            &member,
        );
        assert_eq!(
            on,
            vec![MirrorStep::Refresh {
                org_id: "org-a".into(),
                name: None,
                slug: Some("acme".into()),
            }]
        );

        let mut current = local("a", "org-a");
        current.slug = Some("acme".into());
        assert!(plan_mirror(
            &with_slug,
            &[],
            &[claim.clone()],
            &[current.clone()],
            &member
        )
        .is_empty());

        // A claim without a slug leaves the stored one alone.
        claim.slug = None;
        assert!(plan_mirror(&with_slug, &[], &[claim], &[current], &member).is_empty());
    }

    #[test]
    fn a_claim_without_a_name_does_not_rename() {
        let claim = crate::parse_external_orgs(Some(&serde_json::json!([
            { "id": "a", "role": "member" }
        ])))
        .unwrap();
        let member = [mem("org-a", "a", OrgRole::Member)];
        assert!(plan_mirror(&cfg(true), &[], &claim, &[local("a", "org-a")], &member).is_empty());
    }

    #[test]
    fn unknown_roles_fall_back_to_member() {
        assert_eq!(map_role(&cfg(true), "wizard", &[]), OrgRole::Member);
        assert_eq!(
            map_role(&cfg(true), "billing", &["billing".to_string()]),
            OrgRole::from_declared("billing", &["billing".to_string()]).unwrap()
        );
    }

    #[test]
    fn the_claim_parser_keeps_ids_and_roles_and_drops_junk() {
        let v = serde_json::json!([
            { "id": "o1", "name": "Acme", "slug": "acme", "role": "owner" },
            { "id": "o2", "role": "member" },
            { "name": "no id", "role": "member" },
            { "id": "", "role": "member" },
        ]);
        let parsed = crate::parse_external_orgs(Some(&v)).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].slug.as_deref(), Some("acme"));
        assert_eq!(parsed[1].name, "o2"); // name falls back to the id
        assert!(crate::parse_external_orgs(None).is_none());
        assert!(crate::parse_external_orgs(Some(&serde_json::json!("nope"))).is_none());
        assert_eq!(
            crate::parse_external_orgs(Some(&serde_json::json!([]))),
            Some(vec![])
        );
    }
}
