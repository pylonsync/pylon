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
//!   PYL007  An entity with a collaborative field (`richtext`,
//!           `.crdt("text")`, `.crdt("prosemirror")`, ...) that also has private fields
//!           (`serverOnly`, `syncOmit`, vectors), or is the account
//!           entity. Its Loro doc keeps every field's history, so Pylon
//!           does not replicate it: live editors subscribe and never
//!           load, with no error. Move the private fields to their own
//!           entity.
//!
//!   PYL008  A route listed in `buildManifest({ offline: { routes } })`
//!           whose page or layouts read `serverData`. Offline there is no
//!           server, so the page must render its data from the local
//!           replica with `db.useQuery`. The runtime also refuses to keep
//!           such a page for offline use; this says so before deploy.
//!
//!   PYL009  An offline route that names no page in the app.
//!
//!   PYL010  A page that exports `navigation = "client"` whose page or
//!           layouts read `serverData`. A client navigation renders it
//!           with no page fetch, so serverData has nothing to return.
//!           Offline routes are PYL008's.
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

        // PYL007 — collaborative fields Pylon will not replicate.
        let collaborative: Vec<&str> = entity
            .fields
            .iter()
            .filter(|f| f.is_collaborative())
            .map(|f| f.name.as_str())
            .collect();
        if entity.crdt && entity.sync && !collaborative.is_empty() {
            let private: Vec<&str> = entity.private_fields().map(|f| f.name.as_str()).collect();
            if name == manifest.auth.user.entity {
                findings.push(Finding {
                    rule: "PYL007",
                    severity: Severity::Warning,
                    entity: Some(name.to_string()),
                    policy: None,
                    message: format!(
                        "Entity \"{name}\" is the account entity and has collaborative fields ({}). Pylon never replicates the account entity's Loro doc, so live editors of them never load.",
                        collaborative.join(", ")
                    ),
                    fix: "Keep collaborative fields on their own entity, keyed by the user id.",
                });
            } else if !private.is_empty() {
                findings.push(Finding {
                    rule: "PYL007",
                    severity: Severity::Warning,
                    entity: Some(name.to_string()),
                    policy: None,
                    message: format!(
                        "Entity \"{name}\" has collaborative fields ({}) and private fields ({}). Its Loro doc keeps every field's history, so Pylon does not replicate it: live editors subscribe and never load.",
                        collaborative.join(", "),
                        private.join(", ")
                    ),
                    fix: "Move the private fields to their own entity (sync: false), keyed by this entity's id.",
                });
            }
        }

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
    let mut findings = lint_manifest(&manifest);
    findings.extend(lint_offline_routes(&manifest, &read_project_module));
    findings.extend(lint_client_navigation(&manifest, &read_project_module));
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

/// PYL008 + PYL009: offline routes. Reads page and layout sources through
/// `read_source` (a project-relative module path without extension →
/// source text) so tests need no files.
pub fn lint_offline_routes(
    manifest: &AppManifest,
    read_source: &dyn Fn(&str) -> Option<String>,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    let Some(offline) = manifest.offline.as_ref() else {
        return findings;
    };
    for pattern in &offline.routes {
        let page = manifest.routes.iter().find(|r| {
            route_pattern_key(&r.path) == route_pattern_key(pattern)
                && r.component.is_some()
                && matches!(r.kind.as_deref(), None | Some("page"))
        });
        let Some(page) = page else {
            findings.push(Finding {
                rule: "PYL009",
                severity: Severity::Warning,
                entity: None,
                policy: None,
                message: format!(
                    "offline.routes has \"{pattern}\", but no page serves that path. It will not open offline."
                ),
                fix: "Use the page's route pattern, such as \"/leads/[id]\" for app/leads/[id]/page.tsx.",
            });
            continue;
        };
        let modules = page
            .layouts
            .iter()
            .chain(page.component.iter())
            .map(String::as_str);
        let readers: Vec<&str> = modules
            .filter(|m| read_source(m).is_some_and(|src| reads_server_data(&src)))
            .collect();
        if !readers.is_empty() {
            findings.push(Finding {
                rule: "PYL008",
                severity: Severity::Warning,
                entity: None,
                policy: None,
                message: format!(
                    "Offline route \"{pattern}\" reads serverData in {}. Offline there is no server, so the page cannot render, and Pylon will not keep it for offline use.",
                    readers.join(", ")
                ),
                fix: "Read the data with db.useQuery (the local replica) on offline routes.",
            });
        }
    }
    findings
}

/// PYL010: pages that export `navigation = "client"` and read `serverData`.
/// Routes listed in `offline.routes` are skipped: PYL008 reports those.
pub fn lint_client_navigation(
    manifest: &AppManifest,
    read_source: &dyn Fn(&str) -> Option<String>,
) -> Vec<Finding> {
    let offline: HashSet<&str> = manifest
        .offline
        .as_ref()
        .map(|o| o.routes.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let mut findings = Vec::new();
    for route in &manifest.routes {
        let Some(component) = route.component.as_deref() else {
            continue;
        };
        if !matches!(route.kind.as_deref(), None | Some("page"))
            || offline.contains(route.path.as_str())
        {
            continue;
        }
        if !read_source(component).is_some_and(|src| declares_client_navigation(&src)) {
            continue;
        }
        let readers: Vec<&str> = route
            .layouts
            .iter()
            .map(String::as_str)
            .chain(std::iter::once(component))
            .filter(|m| read_source(m).is_some_and(|src| reads_server_data(&src)))
            .collect();
        if !readers.is_empty() {
            findings.push(Finding {
                rule: "PYL010",
                severity: Severity::Warning,
                entity: None,
                policy: None,
                message: format!(
                    "\"{}\" exports navigation = \"client\" but reads serverData in {}. A client navigation renders it with no page fetch, so serverData returns nothing.",
                    route.path,
                    readers.join(", ")
                ),
                fix: "Read the data with db.useQuery (the local replica), or remove the navigation export.",
            });
        }
    }
    findings
}

/// A route pattern in one spelling. `buildManifest({ offline: { routes } })`
/// takes file-system patterns (`/leads/[id]`, the form the client route
/// matcher uses), while the app manifest's routes carry the server's tokens
/// (`/leads/:id`, `*slug`, `*?slug`). Both become the server form.
fn route_pattern_key(pattern: &str) -> String {
    pattern
        .split('/')
        .map(|seg| {
            if let Some(name) = seg.strip_prefix("[[...").and_then(|s| s.strip_suffix("]]")) {
                format!("*?{name}")
            } else if let Some(name) = seg.strip_prefix("[...").and_then(|s| s.strip_suffix(']')) {
                format!("*{name}")
            } else if let Some(name) = seg.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                format!(":{name}")
            } else {
                seg.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// `src` with comments blanked out, and quoted strings too when
/// `blank_strings`, so the checks below see code only:
/// `// no serverData here` is not a read, and
/// `// export const navigation = "client"` is not a declaration. Template
/// literals keep their `${...}` expressions, which are code. Lengths and
/// line breaks are kept.
fn code_only(src: &str, blank_strings: bool) -> String {
    let b = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    // Template literal nesting: each entry is the brace depth at which a
    // `${` opened, so its `}` returns to the template.
    let mut template_braces: Vec<usize> = Vec::new();
    let mut in_template = false;
    let mut depth = 0usize;
    let blank_comment = |out: &mut String, c: char| out.push(if c == '\n' { '\n' } else { ' ' });
    let blank = |out: &mut String, c: char| {
        if blank_strings {
            out.push(if c == '\n' { '\n' } else { ' ' });
        } else {
            out.push(c);
        }
    };
    while i < b.len() {
        let c = src[i..].chars().next().unwrap();
        let len = c.len_utf8();
        if in_template {
            if c == '\\' && i + 1 < b.len() {
                blank(&mut out, c);
                i += 1;
                let n = src[i..].chars().next().unwrap();
                blank(&mut out, n);
                i += n.len_utf8();
                continue;
            }
            if c == '`' {
                out.push('`');
                in_template = false;
                i += 1;
                continue;
            }
            if c == '$' && b.get(i + 1) == Some(&b'{') {
                out.push_str("${");
                template_braces.push(depth);
                depth += 1;
                in_template = false;
                i += 2;
                continue;
            }
            blank(&mut out, c);
            i += len;
            continue;
        }
        match c {
            '/' if b.get(i + 1) == Some(&b'/') => {
                let end = src[i..].find('\n').map_or(b.len(), |e| i + e);
                for _ in src[i..end].chars() {
                    out.push(' ');
                }
                i = end;
            }
            '/' if b.get(i + 1) == Some(&b'*') => {
                let end = src[i + 2..].find("*/").map_or(b.len(), |e| i + 2 + e + 2);
                for ch in src[i..end].chars() {
                    blank_comment(&mut out, ch);
                }
                i = end;
            }
            '"' | '\'' => {
                out.push(c);
                i += 1;
                while i < b.len() {
                    let ch = src[i..].chars().next().unwrap();
                    if ch == '\\' && i + 1 < b.len() {
                        blank(&mut out, ch);
                        i += 1;
                        let n = src[i..].chars().next().unwrap();
                        blank(&mut out, n);
                        i += n.len_utf8();
                        continue;
                    }
                    if ch == c || ch == '\n' {
                        out.push(ch);
                        i += ch.len_utf8();
                        break;
                    }
                    blank(&mut out, ch);
                    i += ch.len_utf8();
                }
            }
            '`' => {
                out.push('`');
                in_template = true;
                i += 1;
            }
            '{' => {
                depth += 1;
                out.push('{');
                i += 1;
            }
            '}' => {
                depth = depth.saturating_sub(1);
                out.push('}');
                i += 1;
                if template_braces.last() == Some(&depth) {
                    template_braces.pop();
                    in_template = true;
                }
            }
            _ => {
                out.push(c);
                i += len;
            }
        }
    }
    out
}

/// Whether page source declares `export const navigation = "client"`. The
/// same rule as `declaresClientNavigation` in
/// packages/functions/src/ssr-client-nav.ts: single, double or backtick
/// quotes, with an optional type annotation.
fn declares_client_navigation(src: &str) -> bool {
    let code = code_only(src, false);
    let src = code.as_str();
    let mut from = 0;
    while let Some(pos) = src[from..].find("export") {
        let start = from + pos;
        from = start + "export".len();
        let rest = &src[from..];
        let Some(rest) = strip_keyword(rest, "const") else {
            continue;
        };
        let Some(rest) = strip_keyword(rest, "navigation") else {
            continue;
        };
        // An optional type annotation, up to the `=`.
        let rest = rest.trim_start();
        let rest = match rest.strip_prefix(':') {
            Some(annotated) => match annotated.find('=') {
                Some(eq) => &annotated[eq..],
                None => continue,
            },
            None => rest,
        };
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let rest = rest.trim_start();
        for quote in ['"', '\'', '`'] {
            if let Some(value) = rest.strip_prefix(quote) {
                if value.starts_with("client") && value["client".len()..].starts_with(quote) {
                    return true;
                }
            }
        }
    }
    false
}

/// `src` after whitespace and then `keyword` as a whole word, or None.
fn strip_keyword<'a>(src: &'a str, keyword: &str) -> Option<&'a str> {
    let trimmed = src.trim_start();
    if trimmed.len() == src.len() {
        // A keyword must follow whitespace.
        return None;
    }
    let rest = trimmed.strip_prefix(keyword)?;
    let next = rest.chars().next();
    if next.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$') {
        return None;
    }
    Some(rest)
}

/// Whether module source uses `serverData` as an identifier (not as part of
/// a longer name).
fn reads_server_data(src: &str) -> bool {
    // Code only: a comment or a string that names serverData is not a read.
    let code = code_only(src, true);
    let src = code.as_str();
    let bytes = src.as_bytes();
    let needle = b"serverData";
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$';
    let mut from = 0;
    while let Some(pos) = src[from..].find("serverData") {
        let start = from + pos;
        let end = start + needle.len();
        let before_ok = start == 0 || !is_ident(bytes[start - 1]);
        let after_ok = end >= bytes.len() || !is_ident(bytes[end]);
        if before_ok && after_ok {
            return true;
        }
        from = end;
    }
    false
}

/// Read a project module (`app/leads/page`) by trying the source
/// extensions the SSR discoverer accepts.
pub fn read_project_module(module: &str) -> Option<String> {
    ["tsx", "ts", "jsx", "js"]
        .iter()
        .find_map(|ext| std::fs::read_to_string(format!("{module}.{ext}")).ok())
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
            encrypted: false,
            sync_omit: false,
            max_length: None,
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
            sync: true,
            ..Default::default()
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
            llm: Default::default(),
            connections: vec![],
            crons: vec![],
            fonts: vec![],
            required_env: vec![],
            build: Default::default(),
            shards: Vec::new(),
            offline: None,
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
    fn default_scaffold_manifest_is_lint_clean() {
        // The `create-pylon` scaffold (templates/basic/app.ts) ships a `Post`
        // entity WITH a `post_access` policy (public read, authed write). It
        // must lint clean — a newcomer running `pylon dev` should never hit a
        // PYL002 (no-policy → 403) cliff or a PYL001/PYL004 warning out of the
        // box. If the default template drifts to something lint-dirty, this
        // fails. Mirrors the scaffold exactly.
        let post = entity_with(
            "Post",
            vec![
                field("title"),
                field("slug"),
                field("body"),
                field("publishedAt"),
            ],
        );
        let post_policy = policy_with(
            "post_access",
            "Post",
            &[
                ("read", "true"),
                ("insert", "auth.userId != null"),
                ("update", "auth.userId != null"),
                ("delete", "auth.userId != null"),
            ],
        );
        let findings = lint_manifest(&manifest(vec![post], vec![post_policy]));
        assert!(
            findings.is_empty(),
            "default scaffold must lint clean, got: {findings:?}"
        );
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

    fn collaborative(name: &str) -> ManifestField {
        ManifestField {
            crdt: Some(pylon_kernel::CrdtAnnotation::ProseMirror),
            ..field(name)
        }
    }

    fn crdt_entity(fields: Vec<ManifestField>) -> ManifestEntity {
        entity_with("Doc", fields)
    }

    #[test]
    fn pyl007_flags_a_crdt_entity_with_private_fields() {
        let mut secret = field("linkToken");
        secret.server_only = true;
        let mut heavy = field("plan");
        heavy.sync_omit = true;
        let m = manifest(
            vec![crdt_entity(vec![collaborative("body"), secret, heavy])],
            vec![policy_with(
                "doc",
                "Doc",
                &[("read", "true"), ("update", "true")],
            )],
        );
        let pyl007: Vec<_> = lint_manifest(&m)
            .into_iter()
            .filter(|f| f.rule == "PYL007")
            .collect();
        assert_eq!(pyl007.len(), 1, "{pyl007:?}");
        assert!(
            pyl007[0].message.contains("(body)"),
            "{}",
            pyl007[0].message
        );
        assert!(
            pyl007[0].message.contains("(linkToken, plan)"),
            "{}",
            pyl007[0].message
        );
    }

    #[test]
    fn pyl007_skips_public_collaborative_entities_and_private_plain_ones() {
        // Every entity is a CRDT entity by default; only one with a
        // collaborative field needs its Loro doc replicated.
        let mut secret = field("tokenHash");
        secret.server_only = true;
        let plain_with_secret = entity_with("Link", vec![field("documentId"), secret]);
        let m = manifest(
            vec![crdt_entity(vec![collaborative("body")]), plain_with_secret],
            vec![
                policy_with("doc", "Doc", &[("read", "true"), ("update", "true")]),
                policy_with("link", "Link", &[("read", "false"), ("update", "false")]),
            ],
        );
        assert!(lint_manifest(&m).iter().all(|f| f.rule != "PYL007"));
    }

    #[test]
    fn pyl007_flags_a_crdt_account_entity() {
        let mut account = crdt_entity(vec![field("email"), collaborative("bio")]);
        account.name = "User".into();
        let mut m = manifest(
            vec![account],
            vec![policy_with(
                "user",
                "User",
                &[("read", "true"), ("update", "true")],
            )],
        );
        m.auth.user.entity = "User".into();
        assert!(lint_manifest(&m).iter().any(|f| f.rule == "PYL007"));
    }

    fn offline_app(routes: &[&str]) -> AppManifest {
        let mut m = manifest(vec![], vec![]);
        let page = |path: &str, component: &str| pylon_kernel::ManifestRoute {
            path: path.into(),
            mode: "ssr".into(),
            component: Some(component.into()),
            layouts: vec!["app/layout".into()],
            ..Default::default()
        };
        m.routes = vec![
            page("/leads", "app/leads/page"),
            page("/leads/[id]", "app/leads/[id]/page"),
            page("/reports", "app/reports/page"),
        ];
        m.offline = Some(pylon_kernel::ManifestOffline {
            name: "Field Sales".into(),
            short_name: None,
            description: None,
            icons: vec![],
            theme_color: None,
            background_color: None,
            start_url: "/leads".into(),
            display: "standalone".into(),
            routes: routes.iter().map(|r| r.to_string()).collect(),
        });
        m
    }

    fn sources(module: &str) -> Option<String> {
        Some(
            match module {
                "app/layout" => "export default function L({ children }) { return children; }",
                "app/leads/page" => "const rows = db.useQuery(\"Lead\"); // not myserverDataCache",
                "app/leads/[id]/page" => "const lead = use(props.serverData.get(\"Lead\", id));",
                "app/reports/page" => "const { serverData } = props;",
                _ => return None,
            }
            .to_string(),
        )
    }

    #[test]
    fn pyl008_flags_offline_routes_that_read_server_data() {
        let findings = lint_offline_routes(
            &offline_app(&["/leads", "/leads/[id]", "/reports"]),
            &sources,
        );
        let flagged: Vec<&str> = findings
            .iter()
            .filter(|f| f.rule == "PYL008")
            .map(|f| f.message.as_str())
            .collect();
        assert_eq!(flagged.len(), 2, "{findings:?}");
        assert!(
            flagged[0].contains("\"/leads/[id]\"") && flagged[0].contains("app/leads/[id]/page")
        );
        assert!(flagged[1].contains("\"/reports\""));
    }

    #[test]
    fn pyl008_ignores_routes_not_listed_and_apps_without_offline() {
        assert!(lint_offline_routes(&offline_app(&["/leads"]), &sources).is_empty());
        let mut plain = offline_app(&["/reports"]);
        plain.offline = None;
        assert!(lint_offline_routes(&plain, &sources).is_empty());
    }

    #[test]
    fn pyl009_flags_an_offline_route_with_no_page() {
        let findings = lint_offline_routes(&offline_app(&["/leads", "/lead/[id]"]), &sources);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "PYL009");
        assert!(findings[0].message.contains("/lead/[id]"));
    }

    #[test]
    fn pyl010_flags_a_client_navigation_page_that_reads_server_data() {
        let mut m = offline_app(&["/leads/[id]"]);
        let src = |module: &str| -> Option<String> {
            Some(
                match module {
                    "app/layout" => "export default function L({ children }) { return children; }",
                    // Opted in, reads the replica: fine.
                    "app/leads/page" => "export const navigation = \"client\";\nconst rows = db.useQuery(\"Lead\");",
                    // Opted in, reads serverData: flagged.
                    "app/reports/page" => "export const navigation: \"client\" = 'client';\nconst { serverData } = props;",
                    // An offline route: PYL008's, not PYL010's.
                    "app/leads/[id]/page" => "export const navigation = \"client\"; use(props.serverData.get(\"Lead\", id));",
                    _ => return None,
                }
                .to_string(),
            )
        };
        let findings = lint_client_navigation(&m, &src);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].rule, "PYL010");
        assert!(findings[0].message.contains("\"/reports\""));
        assert!(findings[0].message.contains("app/reports/page"));

        // Without the export, reading serverData is the normal fetched path.
        let plain = |module: &str| -> Option<String> {
            src(module).map(|s| s.replace("export const navigation", "const navigationMode"))
        };
        assert!(lint_client_navigation(&m, &plain).is_empty());
        m.offline = None;
        assert_eq!(lint_client_navigation(&m, &src).len(), 2);
    }

    #[test]
    fn comments_and_strings_are_not_reads_or_declarations() {
        assert!(!reads_server_data(
            "// this page reads no serverData\nconst x = 1;"
        ));
        assert!(!reads_server_data(
            "/* serverData is not used */ const x = 1;"
        ));
        assert!(!reads_server_data("const label = \"serverData\";"));
        assert!(!reads_server_data("const t = `no serverData here`;"));
        assert!(reads_server_data(
            "const t = `${props.serverData.get(\"Lead\", id)}`;"
        ));
        assert!(reads_server_data(
            "// a comment\nconst rows = use(props.serverData.list(\"Lead\"));"
        ));
        assert!(reads_server_data(
            "const url = \"http://x\"; const { serverData } = props;"
        ));
        assert!(!declares_client_navigation(
            "// export const navigation = \"client\";"
        ));
        assert!(!declares_client_navigation(
            "/* export const navigation = 'client' */"
        ));
        assert!(declares_client_navigation(
            "// navigation runs on the client\nexport const navigation = \"client\";"
        ));
    }

    #[test]
    fn offline_routes_match_dynamic_pages_in_either_spelling() {
        assert_eq!(route_pattern_key("/jobs/[id]"), "/jobs/:id");
        assert_eq!(route_pattern_key("/docs/[...slug]"), "/docs/*slug");
        assert_eq!(route_pattern_key("/docs/[[...slug]]"), "/docs/*?slug");
        assert_eq!(route_pattern_key("/jobs/:id"), "/jobs/:id");
        // The app manifest writes `:id`; offline.routes uses `[id]`.
        let mut m = offline_app(&["/leads", "/leads/[id]"]);
        m.routes = vec![
            pylon_kernel::ManifestRoute {
                path: "/leads".into(),
                mode: "ssr".into(),
                component: Some("app/leads/page".into()),
                layouts: vec!["app/layout".into()],
                ..Default::default()
            },
            pylon_kernel::ManifestRoute {
                path: "/leads/:id".into(),
                mode: "ssr".into(),
                component: Some("app/leads/[id]/page".into()),
                layouts: vec!["app/layout".into()],
                ..Default::default()
            },
        ];
        let findings = lint_offline_routes(&m, &sources);
        assert!(findings.iter().all(|f| f.rule != "PYL009"), "{findings:?}");
        // The dynamic page reads serverData, so PYL008 now reaches it.
        assert!(findings
            .iter()
            .any(|f| f.rule == "PYL008" && f.message.contains("[id]")));
    }

    #[test]
    fn client_navigation_export_detection() {
        for yes in [
            "export const navigation = \"client\";",
            "export const navigation='client'",
            "export const navigation = `client`",
            "export  const navigation: \"client\" | \"server\" = \"client\" as const;",
        ] {
            assert!(declares_client_navigation(yes), "{yes}");
        }
        for no in [
            "export const navigation = \"server\";",
            "const navigation = \"client\";",
            "export const navigationMode = \"client\";",
            "export const navigation = \"clientele\";",
            "",
        ] {
            assert!(!declares_client_navigation(no), "{no}");
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
