//! `pylon projects` — list / create / use / current.
//!
//! `pylon projects list` — every project the user can see, grouped by
//!   org. Output as a numbered table or JSON.
//! `pylon projects create <slug>` — create + provision a project on
//!   Pylon Cloud, wait for it to come up, and pin it as the local
//!   context. The whole agent path (`pylon login --code` → create →
//!   deploy) runs without touching the dashboard.
//! `pylon projects use <slug>` — pin a slug to `.pylon/project` so
//!   subsequent commands (secrets, logs, deploy, ...) auto-target it.
//! `pylon projects current` — print the currently-set slug + how it
//!   was resolved (file / env / flag).

use std::time::{Duration, Instant};

use pylon_kernel::ExitCode;
use serde::Deserialize;

use crate::cloud_client::{post_json, require_credentials, set_default_project};
use crate::output;
use crate::project_context::{clear_context_file, write_context_file};

/// System hostname suffix for projects on the hosted Smallware service.
const HOSTED_APP_DOMAIN: &str = "smallware.run";

fn hosted_project_url(slug: &str) -> String {
    format!("https://{slug}.{HOSTED_APP_DOMAIN}")
}

#[derive(Deserialize)]
struct ProjectRow {
    slug: String,
    name: String,
    #[serde(rename = "orgSlug")]
    org_slug: Option<String>,
}

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let positional: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with('-') && *a != "projects")
        .map(|s| s.as_str())
        .collect();

    match positional.first().copied() {
        Some("list") | None => run_list(json_mode),
        Some("create") => run_create(args, json_mode),
        Some("use") => run_use(positional.get(1).copied(), json_mode),
        Some("current") => run_current(json_mode),
        // Hidden alias so the projects subcommand tree still has a
        // link verb for users who muscle-memory it there. Delegates
        // to the top-level `pylon link` orchestrator (not advertised
        // in `pylon projects` help — the canonical surface is `pylon
        // link` and the docs point there).
        Some("link") => crate::commands::link::run(args, json_mode),
        Some(sub) => {
            output::print_error(&format!("unknown subcommand: \"{sub}\""));
            eprintln!("Usage: pylon projects [list|create <slug>|use <slug>|current]");
            ExitCode::Usage
        }
    }
}

fn run_list(json_mode: bool) -> ExitCode {
    let creds = match require_credentials() {
        Ok(c) => c,
        Err(e) => {
            output::print_error(&e);
            eprintln!("  Run: pylon login");
            return ExitCode::Usage;
        }
    };
    let projects: Vec<ProjectRow> = match post_json(&creds, "/api/fn/listMyProjectsForCli", &()) {
        Ok(p) => p,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Error;
        }
    };
    if json_mode {
        let out = serde_json::json!({
            "projects": projects.iter().map(|p| serde_json::json!({
                "slug": p.slug,
                "name": p.name,
                "orgSlug": p.org_slug,
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string(&out).unwrap_or_default());
        return ExitCode::Ok;
    }
    if projects.is_empty() {
        println!("No projects yet.");
        println!("  Create one: pylon projects create <slug>");
        return ExitCode::Ok;
    }
    // Column widths sized to the longest entry. Single pass so we
    // don't reformat per-project on rapid CLI calls.
    let org_w = projects
        .iter()
        .map(|p| p.org_slug.as_deref().unwrap_or("?").len())
        .max()
        .unwrap_or(3)
        .max(3);
    let slug_w = projects
        .iter()
        .map(|p| p.slug.len())
        .max()
        .unwrap_or(4)
        .max(4);
    println!("{:<o$}  {:<s$}  NAME", "ORG", "SLUG", o = org_w, s = slug_w);
    for p in &projects {
        let org = p.org_slug.as_deref().unwrap_or("?");
        println!(
            "{:<o$}  {:<s$}  {}",
            org,
            p.slug,
            p.name,
            o = org_w,
            s = slug_w
        );
    }
    ExitCode::Ok
}

/// Arguments to `pylon projects create`, parsed by hand because the
/// shared `collect_positional` filter in `run()` doesn't know about
/// this subcommand's value flags (`--name "My App"` would leak
/// "My App" in as a positional).
#[derive(Debug, PartialEq)]
struct CreateOpts {
    slug: String,
    name: Option<String>,
    org: Option<String>,
    region: Option<String>,
    database: Option<String>,
    wait: bool,
}

fn parse_create_opts(args: &[String]) -> Result<CreateOpts, String> {
    const VALUE_FLAGS: &[&str] = &["--name", "--org", "--region", "--db", "--database"];

    let mut slug: Option<String> = None;
    let mut name = None;
    let mut org = None;
    let mut region = None;
    let mut database = None;
    let mut wait = true;
    // Skip the `projects` + `create` tokens once each; a bare token
    // after that is the slug.
    let mut skipped_projects = false;
    let mut skipped_create = false;

    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if !skipped_projects && a == "projects" {
            skipped_projects = true;
            i += 1;
            continue;
        }
        if !skipped_create && a == "create" {
            skipped_create = true;
            i += 1;
            continue;
        }
        if let Some((flag, value)) = a.split_once('=') {
            if VALUE_FLAGS.contains(&flag) {
                assign_create_flag(
                    flag,
                    value.to_string(),
                    &mut name,
                    &mut org,
                    &mut region,
                    &mut database,
                )?;
                i += 1;
                continue;
            }
        }
        if VALUE_FLAGS.contains(&a) {
            let Some(value) = args.get(i + 1) else {
                return Err(format!("{a} requires a value"));
            };
            assign_create_flag(
                a,
                value.clone(),
                &mut name,
                &mut org,
                &mut region,
                &mut database,
            )?;
            i += 2;
            continue;
        }
        match a {
            "--no-wait" => wait = false,
            "--wait" => wait = true,
            // Global flags handled elsewhere in the CLI.
            "--json" | "-j" => {}
            _ if a.starts_with('-') => {
                return Err(format!("unknown flag: {a}"));
            }
            _ => {
                if slug.is_some() {
                    return Err(format!("unexpected argument: \"{a}\""));
                }
                slug = Some(a.to_string());
            }
        }
        i += 1;
    }

    let Some(slug) = slug else {
        return Err("missing project slug".to_string());
    };
    let slug = slug.trim().to_lowercase();
    if !valid_slug(&slug) {
        return Err(format!(
            "invalid slug \"{slug}\" — lowercase letters/numbers/dashes, 2–40 chars, must start with a letter or number"
        ));
    }
    if let Some(db) = &database {
        if db != "sqlite" && db != "postgres" {
            return Err(format!(
                "--db must be \"sqlite\" or \"postgres\", got \"{db}\""
            ));
        }
    }

    Ok(CreateOpts {
        slug,
        name,
        org,
        region,
        database,
        wait,
    })
}

fn assign_create_flag(
    flag: &str,
    value: String,
    name: &mut Option<String>,
    org: &mut Option<String>,
    region: &mut Option<String>,
    database: &mut Option<String>,
) -> Result<(), String> {
    let slot = match flag {
        "--name" => name,
        "--org" => org,
        "--region" => region,
        "--db" | "--database" => database,
        _ => return Err(format!("unknown flag: {flag}")),
    };
    if slot.is_some() {
        return Err(format!("{flag} passed twice"));
    }
    *slot = Some(value);
    Ok(())
}

/// Mirror of the server-side slug rule in createProject
/// (`^[a-z0-9][a-z0-9-]{1,39}$`) so typos fail fast with a local
/// message instead of a request round-trip.
fn valid_slug(slug: &str) -> bool {
    let bytes = slug.as_bytes();
    if bytes.len() < 2 || bytes.len() > 40 {
        return false;
    }
    if !bytes[0].is_ascii_lowercase() && !bytes[0].is_ascii_digit() {
        return false;
    }
    bytes[1..]
        .iter()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

#[derive(Deserialize)]
struct OrgRow {
    id: String,
    slug: String,
    name: String,
    #[allow(dead_code)]
    role: String,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct CreatedProject {
    id: String,
    slug: String,
}

#[derive(Deserialize)]
struct ProjectStatusRow {
    status: String,
    #[serde(rename = "errorMessage")]
    error_message: Option<String>,
}

fn run_create(args: &[String], json_mode: bool) -> ExitCode {
    let opts = match parse_create_opts(args) {
        Ok(o) => o,
        Err(e) => {
            output::print_error(&e);
            eprintln!(
                "Usage: pylon projects create <slug> [--name <name>] [--org <org-slug>] [--region <region>] [--db sqlite|postgres] [--no-wait]"
            );
            return ExitCode::Usage;
        }
    };
    let creds = match require_credentials() {
        Ok(c) => c,
        Err(e) => {
            output::print_error(&e);
            eprintln!("  Run: pylon login");
            return ExitCode::Usage;
        }
    };

    // Resolve the target org. The server enforces membership again on
    // create; this lookup exists so slugs stay the user-facing
    // currency and single-org users never have to think about orgs.
    let orgs: Vec<OrgRow> = match post_json(&creds, "/api/fn/listMyOrgsForCli", &()) {
        Ok(o) => o,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Error;
        }
    };
    if orgs.is_empty() {
        output::print_error("Your account has no organizations.");
        eprintln!(
            "  Finish signup at: {}/dashboard",
            crate::cloud_client::dashboard_url()
        );
        return ExitCode::Error;
    }
    let org = match &opts.org {
        Some(wanted) => match orgs.iter().find(|o| &o.slug == wanted) {
            Some(o) => o,
            None => {
                output::print_error(&format!(
                    "You're not a member of an org with slug \"{wanted}\"."
                ));
                eprintln!("  Your orgs:");
                for o in &orgs {
                    eprintln!("    {}  ({})", o.slug, o.name);
                }
                return ExitCode::Usage;
            }
        },
        None => {
            if orgs.len() == 1 {
                &orgs[0]
            } else {
                output::print_error("You belong to multiple orgs — pass --org <slug>.");
                eprintln!("  Your orgs:");
                for o in &orgs {
                    eprintln!("    {}  ({})", o.slug, o.name);
                }
                return ExitCode::Usage;
            }
        }
    };

    #[derive(serde::Serialize)]
    struct CreateArgs<'a> {
        #[serde(rename = "orgId")]
        org_id: &'a str,
        name: &'a str,
        slug: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        region: Option<&'a str>,
        #[serde(rename = "databaseKind", skip_serializing_if = "Option::is_none")]
        database_kind: Option<&'a str>,
    }
    let name = opts.name.clone().unwrap_or_else(|| opts.slug.clone());
    let created: CreatedProject = match post_json(
        &creds,
        "/api/fn/createProject",
        &CreateArgs {
            org_id: &org.id,
            name: &name,
            slug: &opts.slug,
            region: opts.region.as_deref(),
            database_kind: opts.database.as_deref(),
        },
    ) {
        Ok(c) => c,
        Err(e) => {
            output::print_error(&format!("Create failed: {e}"));
            return ExitCode::Error;
        }
    };

    // Pin the new project as the local context right away (same as
    // `pylon projects use`) so a follow-up `pylon deploy` targets it
    // even if the caller skips or aborts the provisioning wait.
    set_default_project(&created.slug);
    let context_path = write_context_file(&created.slug).ok();

    if !json_mode {
        println!(
            "✓ Project {} created in org {} — provisioning...",
            created.slug, org.slug
        );
        if let Some(p) = &context_path {
            println!(
                "  Context set: {} (pylon commands now target it)",
                p.display()
            );
        }
    }

    let url = hosted_project_url(&created.slug);
    if !opts.wait {
        if json_mode {
            let out = serde_json::json!({
                "ok": true,
                "slug": created.slug,
                "org": org.slug,
                "status": "provisioning",
                "url": url,
            });
            println!("{}", serde_json::to_string(&out).unwrap_or_default());
        } else {
            println!("  Not waiting (--no-wait). Check progress: pylon status");
            println!("  URL once live: {url}");
        }
        return ExitCode::Ok;
    }

    // Poll until the machine reports running. SQLite projects come up
    // in well under a minute; Postgres adds a PlanetScale provision
    // ahead of the machine boot, so give it a longer leash.
    let deadline = if opts.database.as_deref() == Some("postgres") {
        Duration::from_secs(600)
    } else {
        Duration::from_secs(300)
    };
    let started = Instant::now();
    let mut last_status = String::from("provisioning");
    loop {
        if started.elapsed() > deadline {
            let msg = format!(
                "Timed out after {}s — the project is still \"{last_status}\". It may finish on its own; check `pylon projects list` or the dashboard.",
                deadline.as_secs()
            );
            if json_mode {
                let out = serde_json::json!({
                    "ok": false,
                    "slug": created.slug,
                    "org": org.slug,
                    "status": last_status,
                    "error": msg,
                });
                println!("{}", serde_json::to_string(&out).unwrap_or_default());
            } else {
                output::print_error(&msg);
            }
            return ExitCode::Error;
        }
        std::thread::sleep(Duration::from_secs(3));

        #[derive(serde::Serialize)]
        struct StatusArgs<'a> {
            slug: &'a str,
        }
        let row: ProjectStatusRow = match post_json(
            &creds,
            "/api/fn/getProjectForCli",
            &StatusArgs {
                slug: &created.slug,
            },
        ) {
            Ok(r) => r,
            // Transient poll failures (network blip, control-plane
            // deploy) shouldn't kill a create that already succeeded —
            // keep polling until the deadline decides.
            Err(_) => continue,
        };
        if row.status != last_status {
            if !json_mode {
                println!("  status: {}", row.status);
            }
            last_status = row.status.clone();
        }
        match row.status.as_str() {
            "running" => {
                if json_mode {
                    let out = serde_json::json!({
                        "ok": true,
                        "slug": created.slug,
                        "org": org.slug,
                        "status": "running",
                        "url": url,
                    });
                    println!("{}", serde_json::to_string(&out).unwrap_or_default());
                } else {
                    println!(
                        "✓ Project {} is live ({}s)",
                        created.slug,
                        started.elapsed().as_secs()
                    );
                    println!("  URL:  {url}");
                    println!("  Next: pylon deploy");
                }
                return ExitCode::Ok;
            }
            "error" => {
                let reason = row
                    .error_message
                    .unwrap_or_else(|| "provisioning failed (no detail recorded)".to_string());
                if json_mode {
                    let out = serde_json::json!({
                        "ok": false,
                        "slug": created.slug,
                        "org": org.slug,
                        "status": "error",
                        "error": reason,
                    });
                    println!("{}", serde_json::to_string(&out).unwrap_or_default());
                } else {
                    output::print_error(&format!("Provisioning failed: {reason}"));
                }
                return ExitCode::Error;
            }
            _ => {}
        }
    }
}

fn run_use(slug_arg: Option<&str>, json_mode: bool) -> ExitCode {
    let slug = match slug_arg {
        Some(s) => s.trim(),
        None => {
            output::print_error("Usage: pylon projects use <slug>");
            eprintln!("  Or pass an empty slug to clear: pylon projects use \"\"");
            return ExitCode::Usage;
        }
    };
    if slug.is_empty() {
        // Clear the context.
        match clear_context_file() {
            Ok(true) => {
                if json_mode {
                    println!("{{\"ok\":true,\"cleared\":true}}");
                } else {
                    println!("✓ Cleared project context.");
                }
                ExitCode::Ok
            }
            Ok(false) => {
                if json_mode {
                    println!("{{\"ok\":true,\"cleared\":false}}");
                } else {
                    println!("No project context was set.");
                }
                ExitCode::Ok
            }
            Err(e) => {
                output::print_error(&format!("Failed to clear context: {e}"));
                ExitCode::Error
            }
        }
    } else {
        // Validate the slug against the cloud BEFORE persisting it.
        // Without this a typo silently writes a bad context that fails
        // every subsequent command with "Could not resolve project" —
        // the user thinks they're targeting `my-app` but every secret
        // they set goes nowhere.
        let creds = match require_credentials() {
            Ok(c) => c,
            Err(e) => {
                output::print_error(&e);
                eprintln!("  Run: pylon login");
                return ExitCode::Usage;
            }
        };
        #[derive(serde::Serialize)]
        struct Args<'a> {
            slug: &'a str,
        }
        #[derive(serde::Deserialize)]
        #[allow(dead_code)]
        struct ValidateOut {
            id: String,
        }
        if let Err(e) = crate::cloud_client::post_json::<_, ValidateOut>(
            &creds,
            "/api/fn/getProjectForCli",
            &Args { slug },
        ) {
            output::print_error(&format!("Could not verify project \"{slug}\": {e}"));
            eprintln!("  List available projects: pylon projects list");
            return ExitCode::Usage;
        }
        // Persist the global default too so subsequent invocations
        // from any cwd remember the selection. Per-dir context still
        // wins when present; this is purely the fallback for callers
        // running outside the .pylon/ tree.
        set_default_project(slug);
        match write_context_file(slug) {
            Ok(path) => {
                if json_mode {
                    let out = serde_json::json!({
                        "ok": true,
                        "slug": slug,
                        "path": path.to_string_lossy(),
                        "default_project_persisted": true,
                    });
                    println!("{}", serde_json::to_string(&out).unwrap_or_default());
                } else {
                    println!("✓ Project context set to {slug}");
                    println!("  Local:  {}", path.display());
                    println!("  Global: ~/.config/pylon/state.json");
                    println!(
                        "  Subsequent `pylon` commands anywhere will target it without --project."
                    );
                }
                ExitCode::Ok
            }
            Err(e) => {
                output::print_error(&format!("Failed to write context: {e}"));
                ExitCode::Error
            }
        }
    }
}

fn run_current(json_mode: bool) -> ExitCode {
    let creds = require_credentials().ok();
    let args: Vec<String> = Vec::new();
    let resolved = if let Some(c) = &creds {
        // Resolver mirrors what other commands see. We pass json_mode=true
        // so the resolver skips the interactive picker — `projects
        // current` is supposed to be a quiet status check.
        crate::project_context::resolve_project_slug(&args, c, true).ok()
    } else {
        None
    };
    if json_mode {
        let out = serde_json::json!({
            "slug": resolved,
            "cloud_url": creds.as_ref().map(|c| c.cloud_url.clone()),
            "user_email": creds.as_ref().and_then(|c| c.user_email.clone()),
        });
        println!("{}", serde_json::to_string(&out).unwrap_or_default());
        return ExitCode::Ok;
    }
    match resolved {
        Some(slug) => println!("{slug}"),
        None => {
            println!("(no project context)");
            println!("  Set one with: pylon projects use <slug>");
        }
    }
    ExitCode::Ok
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn create_parses_slug_and_flags() {
        let opts = parse_create_opts(&argv(&[
            "projects", "create", "my-app", "--name", "My App", "--org", "acme", "--region", "iad",
            "--db", "postgres",
        ]))
        .unwrap();
        assert_eq!(
            opts,
            CreateOpts {
                slug: "my-app".into(),
                name: Some("My App".into()),
                org: Some("acme".into()),
                region: Some("iad".into()),
                database: Some("postgres".into()),
                wait: true,
            }
        );
    }

    #[test]
    fn create_parses_equals_form_and_no_wait() {
        let opts = parse_create_opts(&argv(&[
            "projects",
            "create",
            "--org=acme",
            "app2",
            "--no-wait",
        ]))
        .unwrap();
        assert_eq!(opts.slug, "app2");
        assert_eq!(opts.org.as_deref(), Some("acme"));
        assert!(!opts.wait);
    }

    #[test]
    fn create_flag_value_never_becomes_slug() {
        // The bug the hand parser exists to prevent: `--name "My App"`
        // leaking "My App" in as the positional slug.
        let err =
            parse_create_opts(&argv(&["projects", "create", "--name", "My App"])).unwrap_err();
        assert!(err.contains("missing project slug"), "{err}");
    }

    #[test]
    fn create_rejects_bad_input() {
        assert!(parse_create_opts(&argv(&["projects", "create"])).is_err());
        assert!(parse_create_opts(&argv(&["projects", "create", "a"])).is_err()); // too short
        assert!(parse_create_opts(&argv(&["projects", "create", "-bad-slug"])).is_err());
        assert!(parse_create_opts(&argv(&["projects", "create", "app", "--db", "mysql"])).is_err());
        assert!(parse_create_opts(&argv(&["projects", "create", "app", "extra"])).is_err());
        assert!(parse_create_opts(&argv(&["projects", "create", "app", "--bogus"])).is_err());
        assert!(parse_create_opts(&argv(&[
            "projects", "create", "app", "--org", "a", "--org", "b"
        ]))
        .is_err());
        assert!(parse_create_opts(&argv(&["projects", "create", "app", "--name"])).is_err());
    }

    #[test]
    fn create_slug_named_like_subcommand_still_works() {
        // Only the FIRST `create` token is the subcommand; a project
        // literally named "create" is odd but legal per the slug rule.
        let opts = parse_create_opts(&argv(&["projects", "create", "create"])).unwrap();
        assert_eq!(opts.slug, "create");
    }

    #[test]
    fn slug_rule_mirrors_server() {
        assert!(valid_slug("my-app"));
        assert!(valid_slug("a2"));
        assert!(valid_slug("0app"));
        assert!(!valid_slug("a"));
        assert!(!valid_slug("-app"));
        assert!(!valid_slug("My-App"));
        assert!(!valid_slug("app_1"));
        assert!(!valid_slug(&"a".repeat(41)));
    }

    #[test]
    fn hosted_project_url_uses_the_smallware_app_domain() {
        assert_eq!(hosted_project_url("my-app"), "https://my-app.smallware.run");
    }
}
