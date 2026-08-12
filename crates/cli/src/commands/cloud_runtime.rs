//! `pylon runtime` — show or upgrade the pylon RUNTIME version a Cloud project
//! runs on.
//!
//! Cloud machines are image-pinned: a deploy swaps the app artifact, never the
//! pylon binary. So a framework/binary fix (e.g. a security fix in the auth
//! routes) reaches an existing app only when its runtime image is bumped.
//! `pylon runtime <version>` does that in place — every machine reboots on the
//! new binary keeping its app code, data, and secrets.
//!
//!   pylon runtime                 # show the version this project is running
//!   pylon runtime 0.3.330         # upgrade to pylon 0.3.330
//!   pylon runtime latest          # upgrade to the newest published image

use pylon_kernel::ExitCode;
use serde::Deserialize;

use crate::cloud_client::{post_json, require_credentials, Credentials};
use crate::output;
use crate::project_context::resolve_project_slug;

#[derive(Deserialize)]
struct ProjectIdResponse {
    id: String,
}

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    // Must drop flag VALUES, not just flags: `pylon runtime --project pad`
    // read "pad" as the version and tried to upgrade the runtime to a
    // release called "pad".
    let positional = crate::commands::args::collect_positional(args, "runtime");
    let creds = match require_credentials() {
        Ok(c) => c,
        Err(e) => {
            output::print_error(&e);
            eprintln!("  Run: pylon login");
            return ExitCode::Usage;
        }
    };
    let project_slug = match resolve_project_slug(args, &creds, json_mode) {
        Ok(s) => s,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Usage;
        }
    };
    let project_id = match resolve_project_id(&creds, &project_slug) {
        Ok(id) => id,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Error;
        }
    };

    match positional.first().copied() {
        None | Some("show") | Some("status") => run_show(&creds, &project_id, json_mode),
        Some(version) => run_upgrade(&creds, &project_id, version, json_mode),
    }
}

fn run_show(creds: &Credentials, project_id: &str, json_mode: bool) -> ExitCode {
    #[derive(serde::Serialize)]
    struct Args<'a> {
        #[serde(rename = "projectId")]
        project_id: &'a str,
    }
    #[derive(Deserialize)]
    struct Out {
        image: Option<String>,
        version: Option<String>,
    }
    let out: Out = match post_json(
        creds,
        "/api/fn/getProjectRuntimeImage",
        &Args { project_id },
    ) {
        Ok(o) => o,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Error;
        }
    };
    if json_mode {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "image": out.image, "version": out.version,
            }))
            .unwrap_or_default()
        );
        return ExitCode::Ok;
    }
    match out.version {
        Some(v) => println!("Runtime: pylon {v}"),
        None => println!("Runtime: unknown (no live machine yet)"),
    }
    eprintln!("  Upgrade with:  pylon runtime <version>   (e.g. pylon runtime 0.3.330)");
    ExitCode::Ok
}

fn run_upgrade(creds: &Credentials, project_id: &str, version: &str, json_mode: bool) -> ExitCode {
    #[derive(serde::Serialize)]
    struct Args<'a> {
        #[serde(rename = "projectId")]
        project_id: &'a str,
        version: &'a str,
    }
    #[derive(Deserialize)]
    struct Out {
        image: String,
        flipped: u32,
    }
    if !json_mode {
        println!("Upgrading runtime to pylon {version}…");
    }
    let out: Out = match post_json(
        creds,
        "/api/fn/upgradeProjectRuntime",
        &Args {
            project_id,
            version,
        },
    ) {
        Ok(o) => o,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Error;
        }
    };
    if json_mode {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "ok": true, "image": out.image, "flipped": out.flipped,
            }))
            .unwrap_or_default()
        );
        return ExitCode::Ok;
    }
    println!(
        "✓ Runtime → {} ({} machine{} rebooting on the new binary)",
        out.image,
        out.flipped,
        if out.flipped == 1 { "" } else { "s" }
    );
    ExitCode::Ok
}

fn resolve_project_id(creds: &Credentials, slug: &str) -> Result<String, String> {
    #[derive(serde::Serialize)]
    struct Args<'a> {
        slug: &'a str,
    }
    let proj: ProjectIdResponse = post_json(creds, "/api/fn/getProjectForCli", &Args { slug })
        .map_err(|e| format!("Could not resolve project \"{slug}\": {e}"))?;
    Ok(proj.id)
}
