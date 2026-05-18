//! `pylon logs tail` — stream the project's request log to stdout.
//!
//! Wraps the control plane's `tailProjectLogs` function, which proxies
//! to the customer machine's in-process log ring via the cloud's
//! stamped read-only metrics token. Polls at 2s intervals (matches the
//! dashboard Logs page) and threads a cursor through so each iteration
//! only fetches NEW rows.
//!
//! On `kind: "unavailable"` (machine stopped, older framework, etc.)
//! the CLI prints the reason once and keeps polling at 30s — the
//! surface auto-recovers when the machine comes back, same pattern
//! the dashboard uses. Ctrl-C exits.

use std::io::Write;
use std::time::Duration;

use pylon_kernel::ExitCode;
use serde::Deserialize;

use crate::cloud_client::{post_json, require_credentials, Credentials};
use crate::output;
use crate::project_context::resolve_project_slug;

#[derive(Deserialize)]
struct ProjectIdResponse {
    id: String,
}

#[derive(Deserialize)]
struct LogRow {
    timestamp: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    status: Option<u16>,
    #[serde(default)]
    cpu_ms: Option<u64>,
    #[serde(default)]
    bytes_out: Option<u64>,
    #[serde(default)]
    region: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "kind")]
enum TailResponse {
    #[serde(rename = "ok")]
    Ok {
        rows: Vec<LogRow>,
        cursor: Option<String>,
    },
    #[serde(rename = "unavailable")]
    Unavailable { reason: String },
}

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let positional: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with('-') && *a != "logs")
        .map(|s| s.as_str())
        .collect();
    match positional.first().copied() {
        Some("tail") | None => run_tail(args, json_mode),
        Some(sub) => {
            output::print_error(&format!("unknown subcommand: \"{sub}\""));
            eprintln!("Usage: pylon logs [tail]");
            ExitCode::Usage
        }
    }
}

fn run_tail(args: &[String], json_mode: bool) -> ExitCode {
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

    if !json_mode {
        eprintln!("→ Tailing logs for {project_slug}. Ctrl-C to stop.");
    }

    let mut cursor: Option<String> = None;
    let mut printed_unavailable = false;
    loop {
        match fetch_chunk(&creds, &project_id, cursor.as_deref()) {
            Ok(TailResponse::Ok { rows, cursor: next }) => {
                printed_unavailable = false;
                for row in &rows {
                    if json_mode {
                        println!(
                            "{}",
                            serde_json::to_string(&serde_json::json!({
                                "timestamp": row.timestamp,
                                "method": row.method,
                                "path": row.path,
                                "status": row.status,
                                "cpu_ms": row.cpu_ms,
                                "bytes_out": row.bytes_out,
                                "region": row.region,
                                "error": row.error,
                            }))
                            .unwrap_or_default()
                        );
                    } else {
                        print_human(row);
                    }
                }
                let _ = std::io::stdout().flush();
                if let Some(c) = next {
                    cursor = Some(c);
                }
                std::thread::sleep(Duration::from_millis(2_000));
            }
            Ok(TailResponse::Unavailable { reason }) => {
                if !printed_unavailable {
                    eprintln!("! Tail unavailable: {reason}");
                    eprintln!("  Will retry every 30s — Ctrl-C to stop.");
                    printed_unavailable = true;
                }
                std::thread::sleep(Duration::from_secs(30));
            }
            Err(e) => {
                eprintln!("! Tail request failed: {e}");
                eprintln!("  Will retry in 10s — Ctrl-C to stop.");
                std::thread::sleep(Duration::from_secs(10));
            }
        }
    }
}

fn fetch_chunk(
    creds: &Credentials,
    project_id: &str,
    cursor: Option<&str>,
) -> Result<TailResponse, String> {
    #[derive(serde::Serialize)]
    struct Args<'a> {
        #[serde(rename = "projectId")]
        project_id: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        cursor: Option<&'a str>,
    }
    post_json(
        creds,
        "/api/fn/tailProjectLogs",
        &Args { project_id, cursor },
    )
}

fn print_human(row: &LogRow) {
    let when = row.timestamp.split('.').next().unwrap_or(&row.timestamp);
    let method = row.method.as_deref().unwrap_or("?");
    let path = row.path.as_deref().unwrap_or("?");
    let status = row
        .status
        .map(|s| s.to_string())
        .unwrap_or_else(|| "?".into());
    let cpu = row
        .cpu_ms
        .map(|m| format!("{m}ms"))
        .unwrap_or_else(|| "?".into());
    let region = row.region.as_deref().unwrap_or("");
    let mut line = format!("{when} {region:>3} {method:<6} {status:>3} {cpu:>6}  {path}");
    if let Some(err) = &row.error {
        if !err.is_empty() {
            line.push_str(&format!("  · {err}"));
        }
    }
    println!("{line}");
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
