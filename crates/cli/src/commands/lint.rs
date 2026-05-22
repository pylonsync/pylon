//! `pylon lint` — static-analysis pass over the app's manifest that
//! catches the canonical Pylon-shaped security footguns BEFORE the
//! framework's runtime defenses fire. The runtime is the last line
//! of defense; the linter is the first.
//!
//! Rules (rule ids stable so users can write
//! `// pylon-lint-allow: PYL004` comments later):
//!
//!   PYL001  Policy with `allow: "true"` on an entity carrying
//!           tenant-shaped fields (orgId, userId, email, stripe*).
//!           Always intentional sometimes, almost always not.
//!
//!   PYL002  Entity has zero registered policies. Pylon default-denies,
//!           so this is safe at runtime — but the developer almost
//!           certainly meant to expose the entity and forgot. Flag so
//!           it's visible at boot rather than discovered when the UI
//!           breaks. (Underscore-prefixed framework tables skip this
//!           rule — they're internal and never expose.)
//!
//!   PYL003  Policy referencing `data.X == auth.userId` on update
//!           where the entity has the same field — but no
//!           `existing.X == auth.userId` clause. The update path uses
//!           `data` (the proposed payload) which the attacker can set
//!           freely; the safe shape pins `existing` to the row's
//!           current value. Same pattern bites tenantId updates.
//!
//!   PYL004  Policy declares `allowRead` only, no `allowInsert`,
//!           `allowUpdate`, or `allowDelete`. Default-deny will refuse
//!           writes, but the developer may have forgotten to add the
//!           write rules — flagging makes the gap visible.
//!
//! Runs standalone (`pylon lint`) and also at `pylon dev` startup
//! (via a small invocation in `commands/dev.rs`) so issues land in
//! front of the developer the moment they save app.ts.

use crate::output;
use pylon_kernel::{AppManifest, ExitCode, ManifestEntity, ManifestPolicy};
use serde::Serialize;
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Loud — always shown, exits non-zero on `--strict`.
    Warning,
    /// FYI — shown by default, doesn't fail the run.
    Info,
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub rule: &'static str,
    pub severity: Severity,
    pub entity: Option<String>,
    pub policy: Option<String>,
    pub message: String,
    pub fix: &'static str,
}

/// Field names that signal an entity is tenant-shaped (anything
/// owning user data, billing identity, or session-relevant state).
/// `allow: "true"` on these is almost always a mistake.
///
/// Conservative: false-positives are noisy but harmless (developer
/// adds an inline allow-comment), false-negatives are silent
/// security holes. Bias the list toward false-positives.
const SENSITIVE_FIELDS: &[&str] = &[
    "userId",
    "authorId",
    "ownerId",
    "createdBy",
    "orgId",
    "tenantId",
    "workspaceId",
    "email",
    "phone",
    "stripeCustomerId",
    "stripeSubscriptionId",
    "passwordHash",
    "tokenHash",
    "apiKeyHash",
    "secret",
];

/// Lint a parsed manifest. Pure function — no I/O — so it's easy to
/// unit-test against fixture manifests.
pub fn lint_manifest(manifest: &AppManifest) -> Vec<Finding> {
    let mut findings = Vec::new();
    let entity_by_name: std::collections::HashMap<&str, &ManifestEntity> = manifest
        .entities
        .iter()
        .map(|e| (e.name.as_str(), e))
        .collect();
    let policies_by_entity: std::collections::HashMap<&str, Vec<&ManifestPolicy>> = {
        let mut map: std::collections::HashMap<&str, Vec<&ManifestPolicy>> =
            std::collections::HashMap::new();
        for p in &manifest.policies {
            if let Some(e) = p.entity.as_deref() {
                map.entry(e).or_default().push(p);
            }
        }
        map
    };

    for entity in &manifest.entities {
        let name = entity.name.as_str();
        // Framework-owned tables (`_PylonJobs`, etc.) bypass user policies
        // by design; skip them.
        if name.starts_with('_') {
            continue;
        }
        let policies = policies_by_entity.get(name);

        // PYL002 — no policy declared.
        if policies.is_none() || policies.is_some_and(|v| v.is_empty()) {
            findings.push(Finding {
                rule: "PYL002",
                severity: Severity::Warning,
                entity: Some(name.to_string()),
                policy: None,
                message: format!(
                    "Entity \"{name}\" has no policy. Pylon default-denies, so reads + writes return POLICY_DENIED at runtime — but if you meant to expose it, declare a policy explicitly so the gap is visible."
                ),
                fix: "policy({ entity: \"<name>\", allowRead: \"...\", allowInsert: \"...\", ... })",
            });
            continue;
        }

        let sensitive_fields: HashSet<&str> = entity
            .fields
            .iter()
            .map(|f| f.name.as_str())
            .filter(|n| SENSITIVE_FIELDS.contains(n))
            .collect();
        let has_sensitive = !sensitive_fields.is_empty();

        for policy in policies.unwrap() {
            check_policy(
                policy,
                entity,
                &sensitive_fields,
                has_sensitive,
                &mut findings,
            );
        }
    }

    // Function-level lint (PYL005-PYL006) lives elsewhere — the
    // manifest doesn't carry function bodies, so those rules require
    // either a Bun handshake or a TS AST pass. Keeping the
    // manifest-only rules in this module so it stays no-IO.

    // Stable order — sort by (entity, rule, policy) so test
    // assertions stay stable across runs.
    findings.sort_by(|a, b| {
        a.entity
            .as_deref()
            .cmp(&b.entity.as_deref())
            .then_with(|| a.rule.cmp(b.rule))
            .then_with(|| a.policy.as_deref().cmp(&b.policy.as_deref()))
    });

    // Silence the unused-binding warning for the rare entity-without-fields case.
    let _ = entity_by_name;

    findings
}

fn check_policy(
    policy: &ManifestPolicy,
    entity: &ManifestEntity,
    sensitive_fields: &HashSet<&str>,
    has_sensitive: bool,
    findings: &mut Vec<Finding>,
) {
    let entity_name = entity.name.as_str();
    let policy_name = policy.name.as_str();

    // Collect every effective allow expression for the policy. Use the
    // same fallback ordering the policy engine does — specific over
    // shared, never the bare `allow` if a per-action override is set.
    let read = effective(&policy.allow_read, None, &policy.allow);
    let insert = effective(
        &policy.allow_insert,
        policy.allow_write.as_ref(),
        &policy.allow,
    );
    let update = effective(
        &policy.allow_update,
        policy.allow_write.as_ref(),
        &policy.allow,
    );
    let delete = effective(
        &policy.allow_delete,
        policy.allow_write.as_ref(),
        &policy.allow,
    );

    // PYL001 — `allow: "true"` on a sensitive entity.
    if has_sensitive {
        for (action_name, expr) in [
            ("read", read.as_deref()),
            ("insert", insert.as_deref()),
            ("update", update.as_deref()),
            ("delete", delete.as_deref()),
        ] {
            if expr_is_unconditional_true(expr) {
                findings.push(Finding {
                    rule: "PYL001",
                    severity: Severity::Warning,
                    entity: Some(entity_name.to_string()),
                    policy: Some(policy_name.to_string()),
                    message: format!(
                        "Policy \"{policy_name}\" allows {action_name} unconditionally (`{action_name} = true`) on entity \"{entity_name}\", which has tenant-shaped fields ({}). This means anyone can {action_name} every row, including across tenants.",
                        sorted_field_list(sensitive_fields)
                    ),
                    fix: "Scope to the caller — e.g. `allowRead: \"auth.tenantId == data.orgId\"` instead of `true`.",
                });
            }
        }
    }

    // PYL003 — `data.X == auth.userId` on update without an `existing.X`
    //          check. Pure heuristic: look for the canonical IDOR shape
    //          on update.
    if let Some(expr) = update.as_deref() {
        for field in ["userId", "authorId", "ownerId", "tenantId", "orgId"] {
            // Entity must actually have this field; otherwise the
            // expression doesn't really matter.
            if !entity.fields.iter().any(|f| f.name == field) {
                continue;
            }
            let data_check = format!("data.{field} == auth.");
            let existing_check = format!("existing.{field}");
            if expr.contains(&data_check) && !expr.contains(&existing_check) {
                findings.push(Finding {
                    rule: "PYL003",
                    severity: Severity::Warning,
                    entity: Some(entity_name.to_string()),
                    policy: Some(policy_name.to_string()),
                    message: format!(
                        "Policy \"{policy_name}\" gates updates on `data.{field}` (the payload) but doesn't pin `existing.{field}` (the row's current value). An attacker can rewrite `{field}` in the update payload to bypass the check.",
                    ),
                    fix: "Add `existing.<field> == auth.<id>` alongside the `data.<field>` clause. The payload-side check confirms the new value; the existing-side check confirms the caller already owned the row.",
                });
            }
        }
    }

    // PYL004 — read-only policy. Default-deny catches the gap at
    //          runtime, but flagging makes it visible.
    let has_read = read.as_deref().is_some_and(|s| !s.is_empty());
    let has_writes = insert.as_deref().is_some_and(|s| !s.is_empty())
        || update.as_deref().is_some_and(|s| !s.is_empty())
        || delete.as_deref().is_some_and(|s| !s.is_empty());
    if has_read && !has_writes {
        findings.push(Finding {
            rule: "PYL004",
            severity: Severity::Info,
            entity: Some(entity_name.to_string()),
            policy: Some(policy_name.to_string()),
            message: format!(
                "Policy \"{policy_name}\" declares only `allowRead` on \"{entity_name}\". Inserts/updates/deletes default-deny — confirm that's intentional.",
            ),
            fix: "If writes should also be allowed, add `allowInsert`/`allowUpdate`/`allowDelete`. If reads should be the only path, ignore.",
        });
    }
}

/// Mirror `PolicyEngine::expr_for` — pick the most-specific allow
/// expression for an action, falling back to `allow_write` (writes
/// only) then `allow`.
fn effective(
    specific: &Option<String>,
    write_fallback: Option<&String>,
    universal: &str,
) -> Option<String> {
    if let Some(s) = specific.as_deref() {
        if !s.is_empty() {
            return Some(s.to_string());
        }
    }
    if let Some(s) = write_fallback {
        if !s.is_empty() {
            return Some(s.clone());
        }
    }
    if !universal.is_empty() {
        return Some(universal.to_string());
    }
    None
}

/// `true` for `"true"`, `"True"`, `"TRUE"`, with surrounding
/// whitespace tolerated. Doesn't try to evaluate the expression —
/// just the literal-`true` shape, which is the actual footgun.
fn expr_is_unconditional_true(expr: Option<&str>) -> bool {
    expr.map(|s| s.trim().eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn sorted_field_list(fields: &HashSet<&str>) -> String {
    let mut v: Vec<&str> = fields.iter().copied().collect();
    v.sort_unstable();
    v.join(", ")
}

// ---------------------------------------------------------------------------
// CLI entry — `pylon lint`
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct JsonReport<'a> {
    findings: &'a [Finding],
    warnings: usize,
    infos: usize,
}

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let strict = args.iter().any(|a| a == "--strict");

    let manifest = match load_manifest() {
        Ok(m) => m,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Usage;
        }
    };
    let findings = lint_manifest(&manifest);
    let warnings = findings
        .iter()
        .filter(|f| f.severity == Severity::Warning)
        .count();
    let infos = findings
        .iter()
        .filter(|f| f.severity == Severity::Info)
        .count();

    if json_mode {
        output::print_json(&JsonReport {
            findings: &findings,
            warnings,
            infos,
        });
    } else {
        print_pretty(&findings, warnings, infos);
    }

    if strict && warnings > 0 {
        return ExitCode::Error;
    }
    ExitCode::Ok
}

fn print_pretty(findings: &[Finding], warnings: usize, infos: usize) {
    if findings.is_empty() {
        println!();
        println!("\u{2713}  No findings — manifest looks clean.");
        return;
    }
    println!();
    println!("pylon lint — {} warning(s), {} info", warnings, infos);
    println!();
    for f in findings {
        let marker = match f.severity {
            Severity::Warning => "\u{26A0}", // ⚠
            Severity::Info => "\u{2139}",    // ℹ
        };
        let location = match (&f.entity, &f.policy) {
            (Some(e), Some(p)) => format!(" [{}.{}]", e, p),
            (Some(e), None) => format!(" [{}]", e),
            (None, Some(p)) => format!(" [{}]", p),
            (None, None) => String::new(),
        };
        println!("{} {}{}: {}", marker, f.rule, location, f.message);
        println!("   fix: {}", f.fix);
        println!();
    }
}

/// Read the serialized manifest from disk. Looks at the same
/// candidate paths as `pylon doctor` plus the freshly-written
/// `pylon.manifest.json` that the runtime emits at boot.
fn load_manifest() -> Result<AppManifest, String> {
    const CANDIDATES: &[&str] = &[
        "pylon.manifest.json",
        "apps/api/pylon.manifest.json",
        ".pylon/manifest.json",
    ];
    for path in CANDIDATES {
        if !Path::new(path).exists() {
            continue;
        }
        let raw =
            std::fs::read_to_string(path).map_err(|e| format!("Could not read {path}: {e}"))?;
        let manifest: AppManifest =
            serde_json::from_str(&raw).map_err(|e| format!("Could not parse {path}: {e}"))?;
        return Ok(manifest);
    }
    Err(format!(
        "No manifest found (checked: {}). Run `pylon dev` once to emit pylon.manifest.json, then re-run lint.",
        CANDIDATES.join(", ")
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_kernel::{ManifestField, MANIFEST_VERSION};

    fn field(name: &str) -> ManifestField {
        ManifestField {
            name: name.to_string(),
            field_type: "string".to_string(),
            optional: false,
            unique: false,
            crdt: None,
            server_only: false,
            readonly: false,
            default: None,
            enum_values: None,
        }
    }

    fn entity_with(name: &str, fields: Vec<ManifestField>) -> ManifestEntity {
        ManifestEntity {
            name: name.to_string(),
            fields,
            indexes: vec![],
            relations: vec![],
            search: None,
            crdt: true,
        }
    }

    fn manifest(entities: Vec<ManifestEntity>, policies: Vec<ManifestPolicy>) -> AppManifest {
        AppManifest {
            manifest_version: MANIFEST_VERSION,
            name: "test".into(),
            version: "0.0.1".into(),
            entities,
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies,
            auth: Default::default(),
        }
    }

    fn policy_with(name: &str, entity: &str, all: &[(&str, &str)]) -> ManifestPolicy {
        let mut p = ManifestPolicy {
            name: name.to_string(),
            entity: Some(entity.to_string()),
            action: None,
            allow: String::new(),
            allow_read: None,
            allow_insert: None,
            allow_update: None,
            allow_delete: None,
            allow_write: None,
        };
        for (action, expr) in all {
            match *action {
                "read" => p.allow_read = Some(expr.to_string()),
                "insert" => p.allow_insert = Some(expr.to_string()),
                "update" => p.allow_update = Some(expr.to_string()),
                "delete" => p.allow_delete = Some(expr.to_string()),
                "write" => p.allow_write = Some(expr.to_string()),
                "allow" => p.allow = expr.to_string(),
                _ => unreachable!(),
            }
        }
        p
    }

    #[test]
    fn pyl001_flags_allow_true_on_sensitive_entity() {
        // Stripe-customer-shaped entity with a wide-open read.
        let m = manifest(
            vec![entity_with(
                "Org",
                vec![field("name"), field("stripeCustomerId")],
            )],
            vec![policy_with("public", "Org", &[("read", "true")])],
        );
        let findings = lint_manifest(&m);
        let pyl001: Vec<_> = findings.iter().filter(|f| f.rule == "PYL001").collect();
        assert_eq!(pyl001.len(), 1, "expected one PYL001: {findings:?}");
        assert!(pyl001[0].message.contains("stripeCustomerId"));
    }

    #[test]
    fn pyl001_skips_non_sensitive_entity() {
        // PublicPost-shaped — no PII / tenant fields. `allow: "true"`
        // is fine here; the lint must NOT warn.
        let m = manifest(
            vec![entity_with(
                "PublicPost",
                vec![field("title"), field("body")],
            )],
            vec![policy_with("p", "PublicPost", &[("read", "true")])],
        );
        let findings = lint_manifest(&m);
        // Read-only policy → still triggers PYL004, but NOT PYL001.
        assert!(findings.iter().all(|f| f.rule != "PYL001"));
    }

    #[test]
    fn pyl002_flags_entity_without_policy() {
        let m = manifest(vec![entity_with("Note", vec![field("title")])], vec![]);
        let findings = lint_manifest(&m);
        assert!(findings.iter().any(|f| f.rule == "PYL002"));
    }

    #[test]
    fn pyl002_skips_underscore_entities() {
        // Framework-owned tables bypass the rule.
        let m = manifest(
            vec![entity_with("_PylonJobs", vec![field("queue")])],
            vec![],
        );
        let findings = lint_manifest(&m);
        assert!(findings.iter().all(|f| f.rule != "PYL002"));
    }

    #[test]
    fn pyl003_flags_data_check_without_existing() {
        // Canonical IDOR shape — payload-only ownership check.
        let m = manifest(
            vec![entity_with("Note", vec![field("authorId"), field("body")])],
            vec![policy_with(
                "note_author",
                "Note",
                &[
                    ("read", "data.authorId == auth.userId"),
                    ("insert", "data.authorId == auth.userId"),
                    ("update", "data.authorId == auth.userId"),
                ],
            )],
        );
        let findings = lint_manifest(&m);
        assert!(
            findings.iter().any(|f| f.rule == "PYL003"),
            "expected PYL003: {findings:?}"
        );
    }

    #[test]
    fn pyl003_skips_when_existing_is_pinned() {
        // The safe shape — payload AND existing-row check.
        let m = manifest(
            vec![entity_with("Note", vec![field("authorId")])],
            vec![policy_with(
                "note_author",
                "Note",
                &[(
                    "update",
                    "existing.authorId == auth.userId and data.authorId == auth.userId",
                )],
            )],
        );
        let findings = lint_manifest(&m);
        assert!(findings.iter().all(|f| f.rule != "PYL003"));
    }

    #[test]
    fn pyl004_flags_read_only_policy() {
        let m = manifest(
            vec![entity_with("PublicPost", vec![field("title")])],
            vec![policy_with("p", "PublicPost", &[("read", "true")])],
        );
        let findings = lint_manifest(&m);
        assert!(findings.iter().any(|f| f.rule == "PYL004"));
    }

    #[test]
    fn pyl001_unconditional_true_check_tolerates_whitespace_and_case() {
        for expr in [" true ", "TRUE", "True"] {
            let m = manifest(
                vec![entity_with("Org", vec![field("email")])],
                vec![policy_with("p", "Org", &[("read", expr)])],
            );
            assert!(
                lint_manifest(&m).iter().any(|f| f.rule == "PYL001"),
                "expected PYL001 for expression {expr:?}",
            );
        }
    }

    #[test]
    fn clean_manifest_produces_no_findings() {
        let m = manifest(
            vec![entity_with("Note", vec![field("authorId"), field("body")])],
            vec![policy_with(
                "note_author",
                "Note",
                &[
                    ("read", "data.authorId == auth.userId"),
                    ("insert", "data.authorId == auth.userId"),
                    (
                        "update",
                        "existing.authorId == auth.userId and data.authorId == auth.userId",
                    ),
                    ("delete", "existing.authorId == auth.userId"),
                ],
            )],
        );
        let findings = lint_manifest(&m);
        assert!(findings.is_empty(), "expected clean manifest: {findings:?}");
    }
}
