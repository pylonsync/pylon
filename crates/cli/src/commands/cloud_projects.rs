//! `pylon projects` — list / use / current.
//!
//! `pylon projects list` — every project the user can see, grouped by
//!   org. Output as a numbered table or JSON.
//! `pylon projects use <slug>` — pin a slug to `.pylon/project` so
//!   subsequent commands (secrets, logs, deploy, ...) auto-target it.
//! `pylon projects current` — print the currently-set slug + how it
//!   was resolved (file / env / flag).
//!
//! Project creation lives in the dashboard for now — provisioning a
//! Fly machine + a Postgres DB + DNS isn't a one-call operation, so
//! `pylon projects create` would mostly be a wrapper around a wizard
//! that's already nicer in the browser. Surface it later if the demand
//! shows up.

use pylon_kernel::ExitCode;
use serde::Deserialize;

use crate::cloud_client::{post_json, require_credentials, set_default_project};
use crate::output;
use crate::project_context::{clear_context_file, write_context_file};

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
            eprintln!("Usage: pylon projects [list|use <slug>|current]");
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
        println!(
            "  Create one at: {}/dashboard",
            crate::cloud_client::dashboard_url()
        );
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
