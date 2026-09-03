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
    /// Claim orgs that could not be created (store error). Logged, not fatal.
    pub failed: usize,
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

/// Decide the steps for one user. `existing` maps external id → the
/// local org (id) when a mirror row exists, whether or not the user is
/// a member; `memberships` are the user's current memberships in mirrored
/// orgs.
pub fn plan_mirror(
    cfg: &ManifestAuthOrgFederation,
    declared_roles: &[String],
    claim: &[ExternalOrg],
    existing: &[(String, String)], // (external_id, local org_id)
    memberships: &[LocalMirror],
) -> Vec<MirrorStep> {
    let mut steps = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for ext in claim {
        if !seen.insert(ext.id.clone()) {
            continue; // duplicate entry in the claim
        }
        let role = map_role(cfg, &ext.role, declared_roles);
        let local = existing
            .iter()
            .find(|(e, _)| e == &ext.id)
            .map(|(_, id)| id.clone());
        match local {
            None => steps.push(MirrorStep::Create {
                external: ext.clone(),
                role,
            }),
            Some(org_id) => match memberships.iter().find(|m| m.org_id == org_id) {
                None => steps.push(MirrorStep::Join { org_id, role }),
                Some(m) if m.role != role => steps.push(MirrorStep::SetRole { org_id, role }),
                Some(_) => {}
            },
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
    let mut existing: Vec<(String, String)> = memberships
        .iter()
        .map(|m| (m.external_id.clone(), m.org_id.clone()))
        .collect();
    for ext in claim {
        if existing.iter().any(|(e, _)| e == &ext.id) {
            continue;
        }
        if let Some(org) = orgs.find_by_external_id(field, &ext.id) {
            existing.push((ext.id.clone(), org.id));
        }
    }
    for step in plan_mirror(cfg, declared_roles, claim, &existing, &memberships) {
        match step {
            MirrorStep::Create { external, role } => {
                match orgs.create_external(&external.name, field, &external.id, user_id) {
                    Some(org) => {
                        orgs.add_member(&org.id, user_id, role);
                        report.created += 1;
                        report.joined += 1;
                    }
                    None => report.failed += 1,
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
            "[org] federation mirror for user={user_id}: created={} joined={} role_changed={} removed={} failed={}",
            report.created, report.joined, report.role_changed, report.removed, report.failed
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
            &[("b".into(), "org-b".into()), ("c".into(), "org-c".into())],
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
            &[("a".into(), "org-a".into())],
            &[mem("org-a", "a", OrgRole::Member)],
        );
        assert!(steps.is_empty());
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
