//! `pylon restart` — cycle the project's machines without rebuilding.
//!
//! The recovery move when the image is fine but the process isn't: a
//! wedged runtime, a stuck SSR worker, a machine that came up before a
//! dependency was ready. `pylon deploy` rebuilds for no reason and
//! `pylon deployments rollback` points the project at older code;
//! restart just cycles the process on the code already deployed.
//!
//! Destructive enough to confirm — it drops in-flight requests and
//! every WebSocket session — so it goes through the same `--yes` gate
//! as `secrets rm` and `domains rm`.

use pylon_kernel::ExitCode;
use serde::Deserialize;

use crate::cloud_client::{post_json, require_credentials};
use crate::output;
use crate::project_context::resolve_project_slug;

#[derive(Deserialize)]
struct Restarted {
    #[serde(rename = "machineId")]
    machine_id: String,
    region: String,
    /// "restarted" for a running machine, "started" for one that was
    /// stopped. Worth showing: a machine that was down is a different
    /// story from one that was merely stuck.
    action: String,
}

#[derive(Deserialize)]
struct Failed {
    #[serde(rename = "machineId")]
    machine_id: String,
    region: String,
    error: String,
}

#[derive(Deserialize)]
struct RestartResponse {
    #[serde(default)]
    restarted: Vec<Restarted>,
    #[serde(default)]
    failed: Vec<Failed>,
}

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return ExitCode::Ok;
    }
    let creds = match require_credentials() {
        Ok(c) => c,
        Err(e) => {
            output::print_error(&e);
            eprintln!("  Run: pylon login");
            return ExitCode::Usage;
        }
    };
    let slug = match resolve_project_slug(args, &creds, json_mode) {
        Ok(s) => s,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Usage;
        }
    };

    if !output::confirm_destructive(
        args,
        &format!("restart {slug} (drops in-flight requests and open connections)"),
        json_mode,
    ) {
        return ExitCode::Usage;
    }

    #[derive(serde::Serialize)]
    struct Args<'a> {
        #[serde(rename = "projectSlug")]
        project_slug: &'a str,
    }
    let resp: RestartResponse = match post_json(
        &creds,
        "/api/fn/restartProjectMachine",
        &Args {
            project_slug: &slug,
        },
    ) {
        Ok(r) => r,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Error;
        }
    };

    if json_mode {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "ok": resp.failed.is_empty(),
                "project": slug,
                "restarted": resp.restarted.iter().map(|r| serde_json::json!({
                    "machineId": r.machine_id, "region": r.region, "action": r.action,
                })).collect::<Vec<_>>(),
                "failed": resp.failed.iter().map(|f| serde_json::json!({
                    "machineId": f.machine_id, "region": f.region, "error": f.error,
                })).collect::<Vec<_>>(),
            }))
            .unwrap_or_default()
        );
    } else {
        for r in &resp.restarted {
            let verb = if r.action == "started" {
                "started (was stopped)"
            } else {
                "restarted"
            };
            println!("✓ {} {} — {verb}", r.region, r.machine_id);
        }
        for f in &resp.failed {
            println!("✗ {} {} — {}", f.region, f.machine_id, f.error);
        }
        if resp.failed.is_empty() {
            println!("  Takes a few seconds to come back. Check: pylon status");
        }
    }

    // Any machine left un-restarted is a non-zero exit — a multi-region
    // project with one sick region hasn't recovered, and a script that
    // treats this as success moves on from a still-broken app.
    if resp.failed.is_empty() {
        ExitCode::Ok
    } else {
        ExitCode::Error
    }
}

fn print_help() {
    println!("pylon restart — restart the project's machines without rebuilding");
    println!();
    println!("USAGE");
    println!("  pylon restart [--project <slug>] [--yes] [--json]");
    println!();
    println!("Cycles the running process on the code already deployed. Use this when");
    println!("the app is wedged or crash-looping; use `pylon deploy` to ship new code");
    println!("and `pylon deployments rollback` to go back to an older build.");
    println!();
    println!("Stopped machines are started rather than restarted. Multi-region projects");
    println!("are cycled one region at a time.");
    println!();
    println!("FLAGS");
    println!("  --project <slug>   Target project (default: the linked project)");
    println!("  --yes, -y          Skip the confirmation prompt");
    println!("  --json             Machine-readable result");
    println!("  -h, --help         Show this help");
}
