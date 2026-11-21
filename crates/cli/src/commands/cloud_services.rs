//! `pylon services` — the services an app declares with `service({...})`.
//!
//! `pylon services` / `pylon services list` — each service, its status,
//!   how long it ran in the last day, and its cost this month.
//! `pylon services logs <name>` — the last lines the server printed.
//! `pylon services restart <name>` — restart the server on the release it
//!   already runs. Drops requests in flight, so it asks first.
//!
//! A service is deployed by `pylon deploy`, with the app. There is no
//! command to deploy one alone: the app's environment and the service's
//! change together.

use pylon_kernel::ExitCode;
use serde::{Deserialize, Serialize};

use crate::cloud_client::{post_json, require_credentials};
use crate::output;
use crate::project_context::resolve_project_slug;

#[derive(Serialize)]
struct ProjectArgs<'a> {
    #[serde(rename = "projectSlug")]
    project_slug: &'a str,
}

#[derive(Serialize)]
struct ServiceArgs<'a> {
    #[serde(rename = "projectSlug")]
    project_slug: &'a str,
    name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    lines: Option<u32>,
}

#[derive(Deserialize, Serialize)]
struct LastDeploy {
    status: String,
    error: Option<String>,
    #[serde(rename = "startedAt")]
    started_at: String,
}

#[derive(Deserialize, Serialize)]
struct Service {
    name: String,
    status: String,
    url: String,
    #[serde(rename = "memoryMb", default)]
    memory_mb: u32,
    #[serde(rename = "monthCents", default)]
    month_cents: f64,
    #[serde(rename = "awakeSecondsDay", default)]
    awake_seconds_day: f64,
    #[serde(rename = "lastDeploy")]
    last_deploy: Option<LastDeploy>,
}

#[derive(Deserialize)]
struct Logs {
    #[serde(default)]
    lines: Vec<String>,
}

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return ExitCode::Ok;
    }
    let positional: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with('-') && *a != "services")
        .map(|s| s.as_str())
        .collect();

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

    match positional.first().copied() {
        Some("list") | None => run_list(&creds, &slug, json_mode),
        Some("logs") => match positional.get(1).copied() {
            Some(name) => run_logs(&creds, &slug, name, lines_flag(args), json_mode),
            None => usage("pylon services logs <name>"),
        },
        Some("restart") => match positional.get(1).copied() {
            Some(name) => run_restart(args, &creds, &slug, name, json_mode),
            None => usage("pylon services restart <name>"),
        },
        Some(other) => {
            output::print_error(&format!("Unknown services command: {other}"));
            print_help();
            ExitCode::Usage
        }
    }
}

fn usage(line: &str) -> ExitCode {
    output::print_error(&format!("Usage: {line}"));
    ExitCode::Usage
}

/// `--lines N` or `--lines=N`. None when absent or not a number.
fn lines_flag(args: &[String]) -> Option<u32> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if let Some(value) = arg.strip_prefix("--lines=") {
            return value.parse().ok();
        }
        if arg == "--lines" {
            return iter.next().and_then(|v| v.parse().ok());
        }
    }
    None
}

/// Cents as dollars: "$0.00", "<$0.01", "$12.40".
fn cents_label(cents: f64) -> String {
    if cents.is_nan() || cents <= 0.0 {
        "$0.00".to_string()
    } else if cents < 1.0 {
        "<$0.01".to_string()
    } else {
        format!("${:.2}", cents / 100.0)
    }
}

/// Seconds as "0 min", "42 min", "3.5 h".
fn run_time_label(seconds: f64) -> String {
    if seconds.is_nan() || seconds <= 0.0 {
        "0 min".to_string()
    } else if seconds < 3600.0 {
        format!("{} min", (seconds / 60.0).round().max(1.0) as u64)
    } else {
        format!("{:.1} h", seconds / 3600.0)
    }
}

fn status_label(service: &Service) -> &str {
    match service.status.as_str() {
        "live" => "running",
        "installing" => "not deployed",
        other => other,
    }
}

fn run_list(creds: &crate::cloud_client::Credentials, slug: &str, json_mode: bool) -> ExitCode {
    let services: Vec<Service> = match post_json(
        creds,
        "/api/fn/listProjectServices",
        &ProjectArgs { project_slug: slug },
    ) {
        Ok(s) => s,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Error;
        }
    };
    if json_mode {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({ "project": slug, "services": services }))
                .unwrap_or_default()
        );
        return ExitCode::Ok;
    }
    if services.is_empty() {
        println!("{slug} has no services.");
        println!("  Declare one in app.ts with service({{...}}), then run: pylon deploy");
        return ExitCode::Ok;
    }
    let width = services
        .iter()
        .map(|s| s.name.len())
        .max()
        .unwrap_or(4)
        .max(7);
    println!(
        "{:<width$}  {:<13}  {:<12}  {:>10}  {:>10}",
        "SERVICE", "STATUS", "LAST DEPLOY", "RAN (24H)", "THIS MONTH"
    );
    for s in &services {
        let deploy = s
            .last_deploy
            .as_ref()
            .map(|d| d.status.replace('_', " "))
            .unwrap_or_else(|| "none".to_string());
        println!(
            "{:<width$}  {:<13}  {:<12}  {:>10}  {:>10}",
            s.name,
            status_label(s),
            deploy,
            run_time_label(s.awake_seconds_day),
            cents_label(s.month_cents),
        );
    }
    for s in &services {
        if let Some(error) = s.last_deploy.as_ref().and_then(|d| d.error.as_deref()) {
            println!();
            println!("{}: last deploy failed: {error}", s.name);
        }
    }
    ExitCode::Ok
}

fn run_logs(
    creds: &crate::cloud_client::Credentials,
    slug: &str,
    name: &str,
    lines: Option<u32>,
    json_mode: bool,
) -> ExitCode {
    let logs: Logs = match post_json(
        creds,
        "/api/fn/getProjectServiceLogs",
        &ServiceArgs {
            project_slug: slug,
            name,
            lines,
        },
    ) {
        Ok(l) => l,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Error;
        }
    };
    if json_mode {
        println!(
            "{}",
            serde_json::to_string(
                &serde_json::json!({ "project": slug, "service": name, "lines": logs.lines })
            )
            .unwrap_or_default()
        );
        return ExitCode::Ok;
    }
    if logs.lines.is_empty() {
        println!("{name} has printed nothing.");
    }
    for line in &logs.lines {
        println!("{line}");
    }
    ExitCode::Ok
}

fn run_restart(
    args: &[String],
    creds: &crate::cloud_client::Credentials,
    slug: &str,
    name: &str,
    json_mode: bool,
) -> ExitCode {
    if !output::confirm_destructive(
        args,
        &format!("restart the service {name} of {slug} (drops requests in flight)"),
        json_mode,
    ) {
        return ExitCode::Usage;
    }
    let result: Result<serde_json::Value, String> = post_json(
        creds,
        "/api/fn/restartProjectService",
        &ServiceArgs {
            project_slug: slug,
            name,
            lines: None,
        },
    );
    match result {
        Ok(_) => {
            if json_mode {
                println!(
                    "{}",
                    serde_json::json!({ "ok": true, "project": slug, "service": name })
                );
            } else {
                println!("✓ Restarted {name}");
                println!("  Check its output: pylon services logs {name}");
            }
            ExitCode::Ok
        }
        Err(e) => {
            output::print_error(&e);
            ExitCode::Error
        }
    }
}

fn print_help() {
    println!("pylon services — the services an app declares with service({{...}})");
    println!();
    println!("USAGE");
    println!("  pylon services [list]");
    println!("  pylon services logs <name> [--lines N]");
    println!("  pylon services restart <name> [--yes]");
    println!();
    println!("`pylon deploy` deploys the services with the app. `logs` and `restart`");
    println!("need the owner or admin role, and `logs` wakes a sleeping machine.");
    println!();
    println!("OPTIONS");
    println!("  --project <SLUG>   The project (default: the linked one)");
    println!("  --lines <N>        Lines of output to print, up to 2000 (default 200)");
    println!("  --yes              Do not ask before a restart");
    println!("  --json             Machine-readable output");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn lines_flag_reads_both_forms() {
        assert_eq!(
            lines_flag(&args(&["services", "logs", "w", "--lines", "50"])),
            Some(50)
        );
        assert_eq!(
            lines_flag(&args(&["services", "logs", "w", "--lines=75"])),
            Some(75)
        );
        assert_eq!(lines_flag(&args(&["services", "logs", "w"])), None);
        assert_eq!(
            lines_flag(&args(&["services", "logs", "w", "--lines", "many"])),
            None
        );
    }

    #[test]
    fn labels() {
        assert_eq!(cents_label(0.0), "$0.00");
        assert_eq!(cents_label(0.4), "<$0.01");
        assert_eq!(cents_label(1240.0), "$12.40");
        assert_eq!(cents_label(f64::NAN), "$0.00");
        assert_eq!(run_time_label(0.0), "0 min");
        assert_eq!(run_time_label(20.0), "1 min");
        assert_eq!(run_time_label(2520.0), "42 min");
        assert_eq!(run_time_label(12600.0), "3.5 h");
    }

    #[test]
    fn a_service_row_parses_what_cloud_returns() {
        let json = r#"[{"name":"worker","status":"live","url":"https://x.sprites.app","memoryMb":4096,
            "liveReleaseId":"d1","updatedAt":"2026-11-21T10:00:00.000Z","monthCents":12.5,"awakeSecondsDay":2520,
            "lastDeploy":{"deploymentId":"d1","status":"rolled_back","error":null,"startedAt":"2026-11-21T10:00:00.000Z","finishedAt":null}}]"#;
        let rows: Vec<Service> = serde_json::from_str(json).unwrap();
        assert_eq!(rows[0].name, "worker");
        assert_eq!(status_label(&rows[0]), "running");
        assert_eq!(rows[0].last_deploy.as_ref().unwrap().status, "rolled_back");
    }
}
