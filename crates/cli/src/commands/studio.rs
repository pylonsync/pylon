//! `pylon studio` — open your deployed app's Studio in the browser.
//!
//! You own the Cloud project, so you should not have to know the app's
//! admin token, edit an env var, or restart the app to inspect your own
//! data. This command asks the control plane (which holds the token) to
//! mint a single-use, short-lived Studio ticket for the project, then
//! opens the sign-in URL. The token never reaches your machine.

use pylon_kernel::ExitCode;
use serde::Deserialize;

use crate::cloud_client::{post_json, require_credentials};
use crate::output;
use crate::project_context::resolve_project_slug;

#[derive(Deserialize)]
struct StudioSession {
    /// One-time sign-in URL to the app's `/studio/enter`. Valid for a
    /// couple of minutes and usable once.
    url: String,
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

    #[derive(serde::Serialize)]
    struct Args<'a> {
        #[serde(rename = "projectSlug")]
        project_slug: &'a str,
    }
    let resp: StudioSession = match post_json(
        &creds,
        "/api/fn/openStudioSession",
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
            serde_json::to_string(&serde_json::json!({ "url": resp.url })).unwrap_or_default()
        );
        return ExitCode::Ok;
    }

    println!("Opening Studio for {slug}…");
    if let Err(e) = open_browser(&resp.url) {
        println!("Could not open a browser: {e}");
        println!("Open this URL now (it is single-use and expires in a couple of minutes):");
        println!("  {}", resp.url);
    }
    ExitCode::Ok
}

/// Best-effort browser open. macOS `open`, Linux `xdg-open`, Windows `start`.
fn open_browser(url: &str) -> std::io::Result<()> {
    let (cmd, cargs): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("open", vec![url])
    } else if cfg!(target_os = "windows") {
        ("cmd", vec!["/C", "start", "", url])
    } else {
        ("xdg-open", vec![url])
    };
    std::process::Command::new(cmd)
        .args(cargs)
        .spawn()
        .map(|_| ())
}

fn print_help() {
    println!("pylon studio — open your deployed app's Studio in the browser");
    println!();
    println!("USAGE");
    println!("  pylon studio [--project <slug>] [--json]");
    println!();
    println!("Signs you into Studio for the linked Cloud project. You never handle the");
    println!("app's admin token: the control plane mints a single-use, short-lived");
    println!("ticket and this command opens the sign-in URL.");
    println!();
    println!("FLAGS");
    println!("  --project <slug>   Target project (default: the linked project)");
    println!("  --json             Print the sign-in URL as JSON instead of opening a browser");
    println!("  -h, --help         Show this help");
}
