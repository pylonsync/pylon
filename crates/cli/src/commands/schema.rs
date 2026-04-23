use std::collections::{BTreeSet, HashMap};

use pylon_kernel::{AppManifest, Diagnostic, ExitCode, Severity};
use serde::Serialize;

use crate::manifest::{load_manifest, validate_all};
use crate::output::{print_diagnostics, print_json};

/// Redact the password out of a Postgres DSN before printing it.
///
/// `postgres://user:secret@host:5432/db` → `postgres://user:***@host:5432/db`.
/// Malformed DSNs return unchanged — better to leak the raw value than to
/// silently hide a configuration problem. Callers must still treat the
/// output as "might include the url shape" and not the credentials.
fn redact_dsn(dsn: &str) -> String {
    // Find the first `@` that separates userinfo from host. A `:` inside
    // the userinfo denotes password.
    let scheme_end = match dsn.find("://") {
        Some(i) => i + 3,
        None => return dsn.to_string(),
    };
    let rest = &dsn[scheme_end..];
    let at = match rest.find('@') {
        Some(i) => i,
        None => return dsn.to_string(),
    };
    let userinfo = &rest[..at];
    let host_and_rest = &rest[at..]; // starts with '@'
    let redacted_userinfo = match userinfo.find(':') {
        Some(i) => format!("{}:***", &userinfo[..i]),
        None => userinfo.to_string(),
    };
    format!(
        "{}{}{}",
        &dsn[..scheme_end],
        redacted_userinfo,
        host_and_rest
    )
}

// ---------------------------------------------------------------------------
// schema check
// ---------------------------------------------------------------------------

pub fn run_check(args: &[String], json_mode: bool) -> ExitCode {
    let path = args
        .iter()
        .filter(|a| !a.starts_with('-') && *a != "schema" && *a != "check")
        .next()
        .map(|s| s.as_str());

    let manifest_path = path.unwrap_or("pylon.manifest.json");

    let manifest = match load_manifest(manifest_path) {
        Ok(m) => m,
        Err(diags) => {
            print_diagnostics(&diags, json_mode);
            return ExitCode::Error;
        }
    };

    let mut diagnostics = validate_all(&manifest);

    if diagnostics.is_empty() {
        diagnostics.push(Diagnostic {
            severity: Severity::Info,
            code: "SCHEMA_OK".into(),
            message: format!(
                "Schema valid: {} entities, {} queries, {} actions, {} policies, {} routes",
                manifest.entities.len(),
                manifest.queries.len(),
                manifest.actions.len(),
                manifest.policies.len(),
                manifest.routes.len(),
            ),
            span: None,
            hint: None,
        });
    }

    let has_errors = diagnostics.iter().any(|d| d.severity == Severity::Error);
    print_diagnostics(&diagnostics, json_mode);
    if has_errors {
        ExitCode::Error
    } else {
        ExitCode::Ok
    }
}

// ---------------------------------------------------------------------------
// schema diff
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct DiffResult {
    changes: Vec<DiffChange>,
    summary: DiffSummary,
}

#[derive(Serialize, Clone)]
struct DiffChange {
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    entity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

#[derive(Serialize)]
struct DiffSummary {
    total: usize,
    added: usize,
    removed: usize,
}

pub fn run_diff(args: &[String], json_mode: bool) -> ExitCode {
    let positional: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with('-') && *a != "schema" && *a != "diff")
        .map(|s| s.as_str())
        .collect();

    if positional.len() < 2 {
        print_diagnostics(
            &[Diagnostic {
                severity: Severity::Error,
                code: "DIFF_MISSING_ARGS".into(),
                message: "Two manifest paths are required".into(),
                span: None,
                hint: Some("Usage: pylon schema diff <old-manifest> <new-manifest>".into()),
            }],
            json_mode,
        );
        return ExitCode::Usage;
    }

    let old_path = positional[0];
    let new_path = positional[1];

    let old_manifest = match load_manifest(old_path) {
        Ok(m) => m,
        Err(diags) => {
            print_diagnostics(&diags, json_mode);
            return ExitCode::Error;
        }
    };

    let new_manifest = match load_manifest(new_path) {
        Ok(m) => m,
        Err(diags) => {
            print_diagnostics(&diags, json_mode);
            return ExitCode::Error;
        }
    };

    let changes = compute_diff(&old_manifest, &new_manifest);
    let added = changes
        .iter()
        .filter(|c| c.kind.ends_with("_added"))
        .count();
    let removed = changes
        .iter()
        .filter(|c| c.kind.ends_with("_removed"))
        .count();

    let result = DiffResult {
        summary: DiffSummary {
            total: changes.len(),
            added,
            removed,
        },
        changes,
    };

    if json_mode {
        print_json(&result);
    } else {
        if result.changes.is_empty() {
            println!("No changes.");
        } else {
            for change in &result.changes {
                let detail = match (&change.entity, &change.name) {
                    (Some(entity), Some(name)) => format!("{entity}.{name}"),
                    (Some(entity), None) => entity.clone(),
                    (None, Some(name)) => name.clone(),
                    (None, None) => String::new(),
                };
                if detail.is_empty() {
                    println!("  {}", change.kind);
                } else {
                    println!("  {} {}", change.kind, detail);
                }
            }
            println!();
            println!(
                "{} changes ({} added, {} removed)",
                result.summary.total, result.summary.added, result.summary.removed
            );
        }
    }

    ExitCode::Ok
}

// ---------------------------------------------------------------------------
// schema push
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct PushResult {
    code: &'static str,
    dry_run: bool,
    applied: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    adapter: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    database_path: Option<String>,
    baseline: &'static str,
    manifest_version: u32,
    plan: pylon_storage::SchemaPlan,
    analysis: pylon_storage::PlanAnalysis,
    diagnostics: Vec<Diagnostic>,
}

pub fn run_push(args: &[String], json_mode: bool) -> ExitCode {
    let dry_run = args.iter().any(|a| a == "--dry-run");

    let sqlite_path = args
        .windows(2)
        .find(|w| w[0] == "--sqlite")
        .map(|w| w[1].as_str());

    let postgres_url = args
        .windows(2)
        .find(|w| w[0] == "--postgres")
        .map(|w| w[1].as_str());

    let from_path = args
        .windows(2)
        .find(|w| w[0] == "--from")
        .map(|w| w[1].as_str());

    // Count apply targets.
    let apply_targets = [sqlite_path.is_some(), postgres_url.is_some()]
        .iter()
        .filter(|&&x| x)
        .count();

    // Reject ambiguous flag combinations.
    if dry_run && apply_targets > 0 {
        print_diagnostics(
            &[Diagnostic {
                severity: Severity::Error,
                code: "PUSH_AMBIGUOUS_MODE".into(),
                message: "Cannot use --dry-run with --sqlite or --postgres".into(),
                span: None,
                hint: Some("Use --dry-run to preview, or a target flag to apply".into()),
            }],
            json_mode,
        );
        return ExitCode::Usage;
    }

    if apply_targets > 1 {
        print_diagnostics(
            &[Diagnostic {
                severity: Severity::Error,
                code: "PUSH_AMBIGUOUS_MODE".into(),
                message: "Cannot use both --sqlite and --postgres".into(),
                span: None,
                hint: Some("Choose one target".into()),
            }],
            json_mode,
        );
        return ExitCode::Usage;
    }

    // Require at least one mode.
    if !dry_run && apply_targets == 0 {
        print_diagnostics(
            &[Diagnostic {
                severity: Severity::Error,
                code: "PUSH_NO_TARGET".into(),
                message: "No push target specified".into(),
                span: None,
                hint: Some("Use --dry-run, --sqlite <path>, or --postgres <url>".into()),
            }],
            json_mode,
        );
        return ExitCode::Usage;
    }

    let positional: Vec<&str> = args
        .iter()
        .filter(|a| {
            !a.starts_with('-')
                && *a != "schema"
                && *a != "push"
                && Some(a.as_str()) != from_path
                && Some(a.as_str()) != sqlite_path
                && Some(a.as_str()) != postgres_url
        })
        .map(|s| s.as_str())
        .collect();

    let manifest_path = match positional.first() {
        Some(p) => *p,
        None => {
            print_diagnostics(
                &[Diagnostic {
                    severity: Severity::Error,
                    code: "PUSH_MISSING_MANIFEST".into(),
                    message: "No manifest path provided".into(),
                    span: None,
                    hint: Some(
                        "Usage: pylon schema push <manifest> --dry-run|--sqlite <path> [--from <old>]"
                            .into(),
                    ),
                }],
                json_mode,
            );
            return ExitCode::Usage;
        }
    };

    // Load and validate the target manifest.
    let manifest = match load_manifest(manifest_path) {
        Ok(m) => m,
        Err(diags) => {
            print_diagnostics(&diags, json_mode);
            return ExitCode::Error;
        }
    };

    let validation_diags = validate_all(&manifest);
    let has_errors = validation_diags
        .iter()
        .any(|d| d.severity == Severity::Error);

    if has_errors {
        print_diagnostics(&validation_diags, json_mode);
        return ExitCode::Error;
    }

    // Build the storage plan.
    let plan = match build_plan(&manifest, from_path, json_mode) {
        Some(p) => p,
        None => return ExitCode::Error,
    };

    if dry_run {
        // Dry-run: report plan, do not apply.
        let baseline = if from_path.is_some() {
            "manifest"
        } else {
            "empty"
        };
        let analysis = pylon_storage::analyze_plan(&plan);
        let result = PushResult {
            code: "PUSH_DRY_RUN",
            dry_run: true,
            applied: false,
            adapter: None,
            database_path: None,
            baseline,
            manifest_version: manifest.manifest_version,
            plan: plan.clone(),
            analysis: analysis.clone(),
            diagnostics: validation_diags,
        };

        if json_mode {
            print_json(&result);
        } else {
            println!("schema push --dry-run");
            println!();
            println!("  Manifest:  {manifest_path}");
            println!("  Baseline:  {baseline}");
            println!("  Version:   {}", manifest.manifest_version);
            print_plan_human(&plan);
            print_warnings_human(&analysis);
            println!();
            println!("  Schema validated. Not applied (dry-run only).");
        }

        return ExitCode::Ok;
    }

    if let Some(db_path) = sqlite_path {
        // SQLite apply mode.
        let adapter = match pylon_storage::sqlite::SqliteAdapter::open(db_path) {
            Ok(a) => a,
            Err(e) => {
                print_diagnostics(
                    &[Diagnostic {
                        severity: Severity::Error,
                        code: "SQLITE_OPEN_FAILED".into(),
                        message: format!("Failed to open SQLite database: {e}"),
                        span: None,
                        hint: None,
                    }],
                    json_mode,
                );
                return ExitCode::Error;
            }
        };

        // Plan from live DB state if no --from, otherwise use manifest baseline.
        let (live_plan, baseline) = if from_path.is_some() {
            // Already have a plan from build_plan using --from.
            (plan, "manifest")
        } else {
            // Plan from live DB introspection.
            match adapter.plan_from_live(&manifest) {
                Ok(p) => (p, "live_sqlite"),
                Err(e) => {
                    print_diagnostics(
                        &[Diagnostic {
                            severity: Severity::Error,
                            code: "PUSH_PLAN_FAILED".into(),
                            message: format!("Failed to plan from live DB: {e}"),
                            span: None,
                            hint: None,
                        }],
                        json_mode,
                    );
                    return ExitCode::Error;
                }
            }
        };

        let analysis = pylon_storage::analyze_plan(&live_plan);

        // Block apply if the plan contains unsupported operations.
        if analysis.has_unsupported {
            let result = PushResult {
                code: "PUSH_BLOCKED",
                dry_run: false,
                applied: false,
                adapter: Some("sqlite"),
                database_path: Some(db_path.to_string()),
                baseline,
                manifest_version: manifest.manifest_version,
                plan: live_plan.clone(),
                analysis: analysis.clone(),
                diagnostics: validation_diags,
            };

            if json_mode {
                print_json(&result);
            } else {
                println!("schema push --sqlite (BLOCKED)");
                println!();
                println!("  Manifest:  {manifest_path}");
                println!("  Database:  {db_path}");
                println!("  Baseline:  {baseline}");
                println!("  Version:   {}", manifest.manifest_version);
                print_plan_human(&live_plan);
                print_warnings_human(&analysis);
                println!();
                println!("  Push blocked: plan contains unsupported operations.");
            }

            return ExitCode::Error;
        }

        let push_meta = pylon_storage::sqlite::PushMetadata {
            manifest_version: manifest.manifest_version,
            app_version: &manifest.version,
            baseline,
        };

        match adapter.apply_with_history(&live_plan, &push_meta) {
            Ok(()) => {
                let result = PushResult {
                    code: "PUSH_APPLIED",
                    dry_run: false,
                    applied: true,
                    adapter: Some("sqlite"),
                    database_path: Some(db_path.to_string()),
                    baseline,
                    manifest_version: manifest.manifest_version,
                    plan: live_plan.clone(),
                    analysis,
                    diagnostics: validation_diags,
                };

                if json_mode {
                    print_json(&result);
                } else {
                    println!("schema push --sqlite");
                    println!();
                    println!("  Manifest:  {manifest_path}");
                    println!("  Database:  {db_path}");
                    println!("  Baseline:  {baseline}");
                    println!("  Version:   {}", manifest.manifest_version);
                    print_plan_human(&live_plan);
                    println!();
                    println!("  Schema applied successfully.");
                }

                ExitCode::Ok
            }
            Err(e) => {
                print_diagnostics(
                    &[Diagnostic {
                        severity: Severity::Error,
                        code: e.code.clone(),
                        message: e.message.clone(),
                        span: None,
                        hint: if e.code == "SQLITE_OP_UNSUPPORTED" {
                            Some("This operation is not yet supported by the SQLite adapter".into())
                        } else {
                            None
                        },
                    }],
                    json_mode,
                );
                ExitCode::Error
            }
        }
    } else if let Some(pg_url) = postgres_url {
        // Postgres apply mode.
        let mut adapter = match pylon_storage::postgres::live::LivePostgresAdapter::connect(pg_url)
        {
            Ok(a) => a,
            Err(e) => {
                print_diagnostics(
                    &[Diagnostic {
                        severity: Severity::Error,
                        code: "PG_CONNECT_FAILED".into(),
                        message: format!("Failed to connect to Postgres: {e}"),
                        span: None,
                        hint: None,
                    }],
                    json_mode,
                );
                return ExitCode::Error;
            }
        };

        let (pg_plan, baseline) = if from_path.is_some() {
            (plan, "manifest")
        } else {
            match adapter.plan_from_live(&manifest) {
                Ok(p) => (p, "live_postgres"),
                Err(e) => {
                    print_diagnostics(
                        &[Diagnostic {
                            severity: Severity::Error,
                            code: "PUSH_PLAN_FAILED".into(),
                            message: format!("Failed to plan from live Postgres: {e}"),
                            span: None,
                            hint: None,
                        }],
                        json_mode,
                    );
                    return ExitCode::Error;
                }
            }
        };

        let analysis = pylon_storage::analyze_plan(&pg_plan);

        if analysis.has_unsupported {
            let result = PushResult {
                code: "PUSH_BLOCKED",
                dry_run: false,
                applied: false,
                adapter: Some("postgres"),
                database_path: Some(redact_dsn(pg_url)),
                baseline,
                manifest_version: manifest.manifest_version,
                plan: pg_plan.clone(),
                analysis: analysis.clone(),
                diagnostics: validation_diags,
            };

            if json_mode {
                print_json(&result);
            } else {
                println!("schema push --postgres (BLOCKED)");
                println!();
                print_plan_human(&pg_plan);
                print_warnings_human(&analysis);
                println!();
                println!("  Push blocked: plan contains unsupported operations.");
            }
            return ExitCode::Error;
        }

        match adapter.apply_plan(&pg_plan) {
            Ok(()) => {
                let result = PushResult {
                    code: "PUSH_APPLIED",
                    dry_run: false,
                    applied: true,
                    adapter: Some("postgres"),
                    database_path: Some(redact_dsn(pg_url)),
                    baseline,
                    manifest_version: manifest.manifest_version,
                    plan: pg_plan.clone(),
                    analysis,
                    diagnostics: validation_diags,
                };

                if json_mode {
                    print_json(&result);
                } else {
                    println!("schema push --postgres");
                    println!();
                    println!("  Manifest:  {manifest_path}");
                    println!("  Database:  {}", redact_dsn(pg_url));
                    println!("  Baseline:  {baseline}");
                    println!("  Version:   {}", manifest.manifest_version);
                    print_plan_human(&pg_plan);
                    println!();
                    println!("  Schema applied successfully.");
                }

                ExitCode::Ok
            }
            Err(e) => {
                print_diagnostics(
                    &[Diagnostic {
                        severity: Severity::Error,
                        code: e.code.clone(),
                        message: e.message.clone(),
                        span: None,
                        hint: None,
                    }],
                    json_mode,
                );
                ExitCode::Error
            }
        }
    } else {
        ExitCode::Ok
    }
}

fn build_plan(
    manifest: &AppManifest,
    from_path: Option<&str>,
    json_mode: bool,
) -> Option<pylon_storage::SchemaPlan> {
    if let Some(from) = from_path {
        let old_manifest = match load_manifest(from) {
            Ok(m) => m,
            Err(diags) => {
                print_diagnostics(&diags, json_mode);
                return None;
            }
        };
        let adapter = pylon_storage::DiffAdapter { from: old_manifest };
        match pylon_storage::StorageAdapter::plan_schema(&adapter, manifest) {
            Ok(p) => Some(p),
            Err(e) => {
                print_diagnostics(
                    &[Diagnostic {
                        severity: Severity::Error,
                        code: "PUSH_PLAN_FAILED".into(),
                        message: format!("Failed to plan schema: {e}"),
                        span: None,
                        hint: None,
                    }],
                    json_mode,
                );
                None
            }
        }
    } else {
        let adapter = pylon_storage::DryRunAdapter;
        match pylon_storage::StorageAdapter::plan_schema(&adapter, manifest) {
            Ok(p) => Some(p),
            Err(e) => {
                print_diagnostics(
                    &[Diagnostic {
                        severity: Severity::Error,
                        code: "PUSH_PLAN_FAILED".into(),
                        message: format!("Failed to plan schema: {e}"),
                        span: None,
                        hint: None,
                    }],
                    json_mode,
                );
                None
            }
        }
    }
}

fn print_plan_human(plan: &pylon_storage::SchemaPlan) {
    if plan.is_empty() {
        println!();
        println!("  No operations needed.");
    } else {
        println!();
        println!("  Plan ({} operations):", plan.operations.len());
        for op in &plan.operations {
            println!("    {}", format_operation(op));
        }
    }
}

fn print_warnings_human(analysis: &pylon_storage::PlanAnalysis) {
    if analysis.warnings.is_empty() {
        return;
    }
    println!();
    if analysis.destructive {
        println!("  WARNING: Plan contains destructive operations:");
    } else {
        println!("  WARNING: Plan contains unsupported operations:");
    }
    for w in &analysis.warnings {
        println!("    [{}] {}", w.code, w.message);
    }
}

fn format_operation(op: &pylon_storage::SchemaOperation) -> String {
    use pylon_storage::SchemaOperation::*;
    match op {
        CreateEntity { name, fields } => {
            format!("CREATE entity {} ({} fields)", name, fields.len())
        }
        AddField { entity, field } => {
            format!("ADD field {}.{} ({})", entity, field.name, field.field_type)
        }
        RemoveField { entity, field_name } => {
            format!("REMOVE field {}.{}", entity, field_name)
        }
        RemoveEntity { name } => {
            format!("REMOVE entity {}", name)
        }
        AddIndex {
            entity,
            name,
            fields,
            unique,
        } => {
            let u = if *unique { " UNIQUE" } else { "" };
            format!("ADD{u} index {}.{} [{}]", entity, name, fields.join(", "))
        }
        RemoveIndex { entity, name } => {
            format!("REMOVE index {}.{}", entity, name)
        }
        Noop => "NOOP".into(),
    }
}

// ---------------------------------------------------------------------------
// schema inspect
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// schema history
// ---------------------------------------------------------------------------

pub fn run_history(args: &[String], json_mode: bool) -> ExitCode {
    let sqlite_path = args
        .windows(2)
        .find(|w| w[0] == "--sqlite")
        .map(|w| w[1].as_str());

    let limit_str = args
        .windows(2)
        .find(|w| w[0] == "--limit")
        .map(|w| w[1].as_str());

    let entry_id = args
        .windows(2)
        .find(|w| w[0] == "--id")
        .map(|w| w[1].as_str());

    let db_path = match sqlite_path {
        Some(p) => p,
        None => {
            print_diagnostics(
                &[Diagnostic {
                    severity: Severity::Error,
                    code: "HISTORY_NO_TARGET".into(),
                    message: "No database target specified".into(),
                    span: None,
                    hint: Some("Usage: pylon schema history --sqlite <db-path> [--limit N] [--id <entry-id>]".into()),
                }],
                json_mode,
            );
            return ExitCode::Usage;
        }
    };

    let limit: Option<u32> = match limit_str {
        Some(s) => match s.parse::<u32>() {
            Ok(0) => {
                print_diagnostics(
                    &[Diagnostic {
                        severity: Severity::Error,
                        code: "HISTORY_INVALID_LIMIT".into(),
                        message: "Limit must be a positive integer".into(),
                        span: None,
                        hint: None,
                    }],
                    json_mode,
                );
                return ExitCode::Usage;
            }
            Ok(n) => Some(n),
            Err(_) => {
                print_diagnostics(
                    &[Diagnostic {
                        severity: Severity::Error,
                        code: "HISTORY_INVALID_LIMIT".into(),
                        message: format!("Invalid limit value: \"{s}\""),
                        span: None,
                        hint: Some("Limit must be a positive integer".into()),
                    }],
                    json_mode,
                );
                return ExitCode::Usage;
            }
        },
        None => None,
    };

    let adapter = match pylon_storage::sqlite::SqliteAdapter::open(db_path) {
        Ok(a) => a,
        Err(e) => {
            print_diagnostics(
                &[Diagnostic {
                    severity: Severity::Error,
                    code: "SQLITE_OPEN_FAILED".into(),
                    message: format!("Failed to open SQLite database: {e}"),
                    span: None,
                    hint: None,
                }],
                json_mode,
            );
            return ExitCode::Error;
        }
    };

    // Single-entry mode.
    if let Some(id) = entry_id {
        return run_history_single(&adapter, id, db_path, json_mode);
    }

    // List mode.
    let entries = match adapter.read_history(limit) {
        Ok(e) => e,
        Err(e) => {
            print_diagnostics(
                &[Diagnostic {
                    severity: Severity::Error,
                    code: e.code.clone(),
                    message: e.message.clone(),
                    span: None,
                    hint: None,
                }],
                json_mode,
            );
            return ExitCode::Error;
        }
    };

    if json_mode {
        print_json(&entries);
    } else {
        println!("schema history --sqlite {db_path}");
        println!();
        if entries.is_empty() {
            println!("  No push history found.");
        } else {
            let label = match limit {
                Some(n) => format!("showing {} of newest", entries.len().min(n as usize)),
                None => format!("{} entries (newest first)", entries.len()),
            };
            println!("  {label}:");
            println!();
            for entry in &entries {
                let ops_label = if entry.operation_count == 0 {
                    "noop".to_string()
                } else {
                    format!("{} ops", entry.operation_count)
                };
                println!(
                    "  {}  v{} (manifest v{})  {}  baseline={}  id={}",
                    entry.applied_at,
                    entry.app_version,
                    entry.manifest_version,
                    ops_label,
                    entry.baseline,
                    entry.id,
                );
            }
        }
    }

    ExitCode::Ok
}

fn run_history_single(
    adapter: &pylon_storage::sqlite::SqliteAdapter,
    entry_id: &str,
    db_path: &str,
    json_mode: bool,
) -> ExitCode {
    let entry = match adapter.read_history_entry(entry_id) {
        Ok(Some(e)) => e,
        Ok(None) => {
            print_diagnostics(
                &[Diagnostic {
                    severity: Severity::Error,
                    code: "HISTORY_ENTRY_NOT_FOUND".into(),
                    message: format!("No history entry with id \"{entry_id}\""),
                    span: None,
                    hint: Some("Use 'schema history --sqlite <path>' to list entries".into()),
                }],
                json_mode,
            );
            return ExitCode::Error;
        }
        Err(e) => {
            print_diagnostics(
                &[Diagnostic {
                    severity: Severity::Error,
                    code: e.code.clone(),
                    message: e.message.clone(),
                    span: None,
                    hint: None,
                }],
                json_mode,
            );
            return ExitCode::Error;
        }
    };

    if json_mode {
        print_json(&entry);
    } else {
        println!("schema history --sqlite {db_path} --id {entry_id}");
        println!();
        println!("  ID:         {}", entry.id);
        println!("  Applied:    {}", entry.applied_at);
        println!("  App:        v{}", entry.app_version);
        println!("  Manifest:   v{}", entry.manifest_version);
        println!("  Baseline:   {}", entry.baseline);
        println!("  Operations: {}", entry.operation_count);

        if let Some(ref plan) = entry.plan {
            if !plan.is_empty() {
                println!();
                println!("  Plan:");
                for op in &plan.operations {
                    println!("    {}", format_operation(op));
                }
            }
        }
    }

    ExitCode::Ok
}

pub fn run_inspect(args: &[String], json_mode: bool) -> ExitCode {
    let sqlite_path = args
        .windows(2)
        .find(|w| w[0] == "--sqlite")
        .map(|w| w[1].as_str());

    let postgres_url = args
        .windows(2)
        .find(|w| w[0] == "--postgres")
        .map(|w| w[1].as_str());

    if sqlite_path.is_none() && postgres_url.is_none() {
        print_diagnostics(
            &[Diagnostic {
                severity: Severity::Error,
                code: "INSPECT_NO_TARGET".into(),
                message: "No database target specified".into(),
                span: None,
                hint: Some(
                    "Usage: pylon schema inspect --sqlite <path> or --postgres <url>".into(),
                ),
            }],
            json_mode,
        );
        return ExitCode::Usage;
    }

    let (snapshot, target_label) = if let Some(db_path) = sqlite_path {
        let adapter = match pylon_storage::sqlite::SqliteAdapter::open(db_path) {
            Ok(a) => a,
            Err(e) => {
                print_diagnostics(
                    &[Diagnostic {
                        severity: Severity::Error,
                        code: "SQLITE_OPEN_FAILED".into(),
                        message: format!("Failed to open SQLite database: {e}"),
                        span: None,
                        hint: None,
                    }],
                    json_mode,
                );
                return ExitCode::Error;
            }
        };
        match adapter.read_schema() {
            Ok(s) => (s, format!("--sqlite {db_path}")),
            Err(e) => {
                print_diagnostics(
                    &[Diagnostic {
                        severity: Severity::Error,
                        code: e.code.clone(),
                        message: e.message.clone(),
                        span: None,
                        hint: None,
                    }],
                    json_mode,
                );
                return ExitCode::Error;
            }
        }
    } else if let Some(pg_url) = postgres_url {
        let mut adapter = match pylon_storage::postgres::live::LivePostgresAdapter::connect(pg_url)
        {
            Ok(a) => a,
            Err(e) => {
                print_diagnostics(
                    &[Diagnostic {
                        severity: Severity::Error,
                        code: "PG_CONNECT_FAILED".into(),
                        message: format!("Failed to connect to Postgres: {e}"),
                        span: None,
                        hint: None,
                    }],
                    json_mode,
                );
                return ExitCode::Error;
            }
        };
        match adapter.read_schema() {
            Ok(s) => (s, format!("--postgres {}", redact_dsn(pg_url))),
            Err(e) => {
                print_diagnostics(
                    &[Diagnostic {
                        severity: Severity::Error,
                        code: e.code.clone(),
                        message: e.message.clone(),
                        span: None,
                        hint: None,
                    }],
                    json_mode,
                );
                return ExitCode::Error;
            }
        }
    } else {
        unreachable!()
    };

    let _db_path = sqlite_path.or(postgres_url).unwrap_or("unknown");

    if json_mode {
        print_json(&snapshot);
    } else {
        println!("schema inspect {target_label}");
        println!();
        if snapshot.tables.is_empty() {
            println!("  No tables found.");
        } else {
            println!("  {} tables:", snapshot.tables.len());
            for table in &snapshot.tables {
                println!();
                println!("  {}", table.name);
                for col in &table.columns {
                    let mut flags = Vec::new();
                    if col.primary_key {
                        flags.push("PK");
                    }
                    if col.notnull {
                        flags.push("NOT NULL");
                    }
                    let flag_str = if flags.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", flags.join(", "))
                    };
                    println!("    {}: {}{}", col.name, col.column_type, flag_str);
                }
                for idx in &table.indexes {
                    let unique_str = if idx.unique { "UNIQUE " } else { "" };
                    println!(
                        "    {}index {}: [{}]",
                        unique_str,
                        idx.name,
                        idx.columns.join(", ")
                    );
                }
            }
        }
    }

    ExitCode::Ok
}

fn compute_diff(old: &AppManifest, new: &AppManifest) -> Vec<DiffChange> {
    let mut changes = Vec::new();

    // Manifest version
    if old.manifest_version != new.manifest_version {
        changes.push(DiffChange {
            kind: "manifest_version_changed".into(),
            entity: None,
            name: Some(format!(
                "{} -> {}",
                old.manifest_version, new.manifest_version
            )),
        });
    }

    // Entities
    let old_entities: HashMap<&str, &pylon_kernel::ManifestEntity> =
        old.entities.iter().map(|e| (e.name.as_str(), e)).collect();
    let new_entities: HashMap<&str, &pylon_kernel::ManifestEntity> =
        new.entities.iter().map(|e| (e.name.as_str(), e)).collect();

    let old_entity_names: BTreeSet<&str> = old_entities.keys().copied().collect();
    let new_entity_names: BTreeSet<&str> = new_entities.keys().copied().collect();

    for name in new_entity_names.difference(&old_entity_names) {
        changes.push(DiffChange {
            kind: "entity_added".into(),
            entity: None,
            name: Some(name.to_string()),
        });
    }
    for name in old_entity_names.difference(&new_entity_names) {
        changes.push(DiffChange {
            kind: "entity_removed".into(),
            entity: None,
            name: Some(name.to_string()),
        });
    }

    // Fields per shared entity
    for name in old_entity_names.intersection(&new_entity_names) {
        let old_fields: BTreeSet<&str> = old_entities[name]
            .fields
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        let new_fields: BTreeSet<&str> = new_entities[name]
            .fields
            .iter()
            .map(|f| f.name.as_str())
            .collect();

        for field in new_fields.difference(&old_fields) {
            changes.push(DiffChange {
                kind: "field_added".into(),
                entity: Some(name.to_string()),
                name: Some(field.to_string()),
            });
        }
        for field in old_fields.difference(&new_fields) {
            changes.push(DiffChange {
                kind: "field_removed".into(),
                entity: Some(name.to_string()),
                name: Some(field.to_string()),
            });
        }
    }

    // Routes
    diff_by_key(
        &old.routes.iter().map(|r| r.path.as_str()).collect(),
        &new.routes.iter().map(|r| r.path.as_str()).collect(),
        "route_added",
        "route_removed",
        &mut changes,
    );

    // Queries
    diff_by_key(
        &old.queries.iter().map(|q| q.name.as_str()).collect(),
        &new.queries.iter().map(|q| q.name.as_str()).collect(),
        "query_added",
        "query_removed",
        &mut changes,
    );

    // Actions
    diff_by_key(
        &old.actions.iter().map(|a| a.name.as_str()).collect(),
        &new.actions.iter().map(|a| a.name.as_str()).collect(),
        "action_added",
        "action_removed",
        &mut changes,
    );

    // Policies
    diff_by_key(
        &old.policies.iter().map(|p| p.name.as_str()).collect(),
        &new.policies.iter().map(|p| p.name.as_str()).collect(),
        "policy_added",
        "policy_removed",
        &mut changes,
    );

    changes
}

fn diff_by_key(
    old_keys: &BTreeSet<&str>,
    new_keys: &BTreeSet<&str>,
    added_kind: &str,
    removed_kind: &str,
    changes: &mut Vec<DiffChange>,
) {
    for key in new_keys.difference(old_keys) {
        changes.push(DiffChange {
            kind: added_kind.into(),
            entity: None,
            name: Some(key.to_string()),
        });
    }
    for key in old_keys.difference(new_keys) {
        changes.push(DiffChange {
            kind: removed_kind.into(),
            entity: None,
            name: Some(key.to_string()),
        });
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_kernel::*;

    #[test]
    fn redact_dsn_hides_password() {
        let dsn = "postgres://alice:s3cret@db.example.com:5432/app";
        let red = redact_dsn(dsn);
        assert!(!red.contains("s3cret"));
        assert!(red.contains("alice"));
        assert!(red.contains("db.example.com"));
        assert!(red.contains("***"));
    }

    #[test]
    fn redact_dsn_passes_through_no_password() {
        let dsn = "postgres://alice@db.example.com/app";
        assert_eq!(redact_dsn(dsn), dsn);
    }

    #[test]
    fn redact_dsn_passes_through_malformed() {
        let dsn = "not a url";
        assert_eq!(redact_dsn(dsn), dsn);
    }

    fn minimal_manifest() -> AppManifest {
        AppManifest {
            manifest_version: MANIFEST_VERSION,
            name: "test".into(),
            version: "0.1.0".into(),
            entities: vec![ManifestEntity {
                name: "User".into(),
                fields: vec![ManifestField {
                    name: "email".into(),
                    field_type: "string".into(),
                    optional: false,
                    unique: true,
                }],
                indexes: vec![],
                relations: vec![],
            }],
            routes: vec![ManifestRoute {
                path: "/".into(),
                mode: "server".into(),
                query: None,
                auth: None,
            }],
            queries: vec![],
            actions: vec![],
            policies: vec![],
        }
    }

    #[test]
    fn no_changes() {
        let m = minimal_manifest();
        let changes = compute_diff(&m, &m);
        assert!(changes.is_empty());
    }

    #[test]
    fn entity_added() {
        let old = minimal_manifest();
        let mut new = minimal_manifest();
        new.entities.push(ManifestEntity {
            name: "Post".into(),
            fields: vec![],
            indexes: vec![],
            relations: vec![],
        });
        let changes = compute_diff(&old, &new);
        assert!(changes
            .iter()
            .any(|c| c.kind == "entity_added" && c.name.as_deref() == Some("Post")));
    }

    #[test]
    fn entity_removed() {
        let old = minimal_manifest();
        let mut new = minimal_manifest();
        new.entities.clear();
        let changes = compute_diff(&old, &new);
        assert!(changes
            .iter()
            .any(|c| c.kind == "entity_removed" && c.name.as_deref() == Some("User")));
    }

    #[test]
    fn field_added_to_entity() {
        let old = minimal_manifest();
        let mut new = minimal_manifest();
        new.entities[0].fields.push(ManifestField {
            name: "name".into(),
            field_type: "string".into(),
            optional: false,
            unique: false,
        });
        let changes = compute_diff(&old, &new);
        assert!(changes.iter().any(|c| c.kind == "field_added"
            && c.entity.as_deref() == Some("User")
            && c.name.as_deref() == Some("name")));
    }

    #[test]
    fn field_removed_from_entity() {
        let old = minimal_manifest();
        let mut new = minimal_manifest();
        new.entities[0].fields.clear();
        let changes = compute_diff(&old, &new);
        assert!(changes.iter().any(|c| c.kind == "field_removed"
            && c.entity.as_deref() == Some("User")
            && c.name.as_deref() == Some("email")));
    }

    #[test]
    fn route_added() {
        let old = minimal_manifest();
        let mut new = minimal_manifest();
        new.routes.push(ManifestRoute {
            path: "/about".into(),
            mode: "static".into(),
            query: None,
            auth: None,
        });
        let changes = compute_diff(&old, &new);
        assert!(changes
            .iter()
            .any(|c| c.kind == "route_added" && c.name.as_deref() == Some("/about")));
    }

    #[test]
    fn query_and_action_changes() {
        let old = minimal_manifest();
        let mut new = minimal_manifest();
        new.queries.push(ManifestQuery {
            name: "getUser".into(),
            input: vec![],
        });
        new.actions.push(ManifestAction {
            name: "createUser".into(),
            input: vec![],
        });
        let changes = compute_diff(&old, &new);
        assert!(changes
            .iter()
            .any(|c| c.kind == "query_added" && c.name.as_deref() == Some("getUser")));
        assert!(changes
            .iter()
            .any(|c| c.kind == "action_added" && c.name.as_deref() == Some("createUser")));
    }

    #[test]
    fn policy_removed() {
        let mut old = minimal_manifest();
        old.policies.push(ManifestPolicy {
            name: "p1".into(),
            entity: Some("User".into()),
            action: None,
            allow: "true".into(),
            allow_read: None,
            allow_insert: None,
            allow_update: None,
            allow_delete: None,
            allow_write: None,
        });
        let new = minimal_manifest();
        let changes = compute_diff(&old, &new);
        assert!(changes
            .iter()
            .any(|c| c.kind == "policy_removed" && c.name.as_deref() == Some("p1")));
    }

    #[test]
    fn manifest_version_changed() {
        let old = minimal_manifest();
        let mut new = minimal_manifest();
        new.manifest_version = 99;
        let changes = compute_diff(&old, &new);
        assert!(changes.iter().any(|c| c.kind == "manifest_version_changed"));
    }
}
