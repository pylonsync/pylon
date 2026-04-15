use agentdb_core::{ExitCode, Severity};
use serde::Serialize;

use crate::output;

// ---------------------------------------------------------------------------
// Check result — one line in the doctor report
// ---------------------------------------------------------------------------

struct Check {
    label: String,
    severity: Severity,
    detail: Option<String>,
}

impl Check {
    fn pass(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            severity: Severity::Info,
            detail: None,
        }
    }

    fn warn(label: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            severity: Severity::Warning,
            detail: Some(detail.into()),
        }
    }

    fn error(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            severity: Severity::Error,
            detail: None,
        }
    }

    fn icon(&self) -> &'static str {
        match self.severity {
            Severity::Info => "\u{2713}",    // ✓
            Severity::Warning => "\u{26A0}", // ⚠
            Severity::Error => "\u{2717}",   // ✗
        }
    }
}

// ---------------------------------------------------------------------------
// Individual checks
// ---------------------------------------------------------------------------

fn check_bun() -> Check {
    match std::process::Command::new("bun")
        .arg("--version")
        .output()
    {
        Ok(out) if out.status.success() => {
            let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
            Check::pass(format!("Bun {version} installed"))
        }
        _ => Check::error("Bun not found"),
    }
}

fn check_database() -> Check {
    if std::fs::metadata("agentdb.dev.db").is_ok() {
        Check::pass("Database exists (agentdb.dev.db)")
    } else {
        Check::warn(
            "Database not found (agentdb.dev.db)",
            "run `agentdb dev` to create it",
        )
    }
}

fn check_admin_token() -> Check {
    if std::env::var("AGENTDB_ADMIN_TOKEN").is_ok() {
        Check::pass("AGENTDB_ADMIN_TOKEN set")
    } else {
        Check::warn(
            "AGENTDB_ADMIN_TOKEN not set",
            "auth endpoints will be unprotected",
        )
    }
}

fn check_port() -> Check {
    match std::net::TcpListener::bind("127.0.0.1:4321") {
        Ok(_listener) => Check::pass("Port 4321 available"),
        Err(_) => Check::warn("Port 4321 in use", "dev server may fail to start"),
    }
}

fn check_disk_space() -> Check {
    // Use `df` on unix-like systems to check available space.
    match std::process::Command::new("df")
        .args(["-k", "."])
        .output()
    {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            // Second line contains the stats; 4th column is available KB.
            if let Some(line) = text.lines().nth(1) {
                let cols: Vec<&str> = line.split_whitespace().collect();
                if let Some(avail_kb) = cols.get(3).and_then(|s| s.parse::<u64>().ok()) {
                    let avail_mb = avail_kb / 1024;
                    if avail_mb < 100 {
                        return Check::warn(
                            format!("Low disk space ({avail_mb} MB available)"),
                            "at least 100 MB recommended",
                        );
                    }
                    return Check::pass(format!("Disk space OK ({avail_mb} MB available)"));
                }
            }
            // Could not parse — just pass silently.
            Check::pass("Disk space check skipped (could not parse df output)")
        }
        _ => Check::pass("Disk space check skipped (df unavailable)"),
    }
}

fn check_manifest() -> Check {
    if std::fs::metadata("agentdb.manifest.json").is_ok() {
        Check::pass("Manifest found (agentdb.manifest.json)")
    } else if std::fs::metadata("app.ts").is_ok() {
        Check::pass("Manifest found (app.ts)")
    } else {
        Check::error("No manifest found (expected agentdb.manifest.json or app.ts)")
    }
}

fn check_migrations() -> Check {
    match std::fs::read_dir("migrations") {
        Ok(entries) => {
            let count = entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.path()
                        .extension()
                        .map(|ext| ext == "sql")
                        .unwrap_or(false)
                })
                .count();
            if count > 0 {
                Check::pass(format!(
                    "Migrations directory ({count} migration{})",
                    if count == 1 { "" } else { "s" }
                ))
            } else {
                Check::warn(
                    "Migrations directory is empty",
                    "run `agentdb schema push` to generate migrations",
                )
            }
        }
        Err(_) => Check::warn(
            "No migrations directory found",
            "run `agentdb schema push` to create one",
        ),
    }
}

fn check_dependencies() -> Check {
    let has_node_modules = std::fs::metadata("node_modules").is_ok();
    let has_lockfile = std::fs::metadata("bun.lockb").is_ok();
    if has_node_modules || has_lockfile {
        Check::pass("Dependencies installed")
    } else {
        Check::warn(
            "Dependencies not installed",
            "run `bun install` to install packages",
        )
    }
}

// ---------------------------------------------------------------------------
// JSON output shape
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct JsonReport {
    checks: Vec<JsonCheck>,
    passed: usize,
    warnings: usize,
    errors: usize,
}

#[derive(Serialize)]
struct JsonCheck {
    name: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

// ---------------------------------------------------------------------------
// ANSI helpers (mirrors output.rs)
// ---------------------------------------------------------------------------

const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

fn use_color() -> bool {
    std::env::var("NO_COLOR").is_err()
        && std::env::var("TERM")
            .map(|t| t != "dumb")
            .unwrap_or(true)
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn run(_args: &[String], json_mode: bool) -> ExitCode {
    let checks = vec![
        check_bun(),
        check_manifest(),
        check_database(),
        check_admin_token(),
        check_port(),
        check_dependencies(),
        check_migrations(),
        check_disk_space(),
    ];

    let passed = checks.iter().filter(|c| c.severity == Severity::Info).count();
    let warnings = checks.iter().filter(|c| c.severity == Severity::Warning).count();
    let errors = checks.iter().filter(|c| c.severity == Severity::Error).count();

    if json_mode {
        let report = JsonReport {
            checks: checks
                .iter()
                .map(|c| JsonCheck {
                    name: c.label.clone(),
                    status: match c.severity {
                        Severity::Info => "pass".into(),
                        Severity::Warning => "warning".into(),
                        Severity::Error => "error".into(),
                    },
                    detail: c.detail.clone(),
                })
                .collect(),
            passed,
            warnings,
            errors,
        };
        output::print_json(&report);
    } else {
        let color = use_color();
        println!();
        println!("agentdb doctor");
        println!();

        for check in &checks {
            let icon = check.icon();
            if color {
                let icon_color = match check.severity {
                    Severity::Info => GREEN,
                    Severity::Warning => YELLOW,
                    Severity::Error => RED,
                };
                print!("  {icon_color}{icon}{RESET} {}", check.label);
                if let Some(detail) = &check.detail {
                    print!(" {DIM}({detail}){RESET}");
                }
                println!();
            } else {
                print!("  {icon} {}", check.label);
                if let Some(detail) = &check.detail {
                    print!(" ({detail})");
                }
                println!();
            }
        }

        println!();
        if color {
            println!(
                "  {BOLD}{passed}{RESET} passed, {BOLD}{warnings}{RESET} warning{}, {BOLD}{errors}{RESET} error{}",
                if warnings == 1 { "" } else { "s" },
                if errors == 1 { "" } else { "s" },
            );
        } else {
            println!(
                "  {passed} passed, {warnings} warning{}, {errors} error{}",
                if warnings == 1 { "" } else { "s" },
                if errors == 1 { "" } else { "s" },
            );
        }
        println!();
    }

    if errors > 0 {
        ExitCode::Error
    } else {
        ExitCode::Ok
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_pass_icon() {
        let c = Check::pass("ok");
        assert_eq!(c.icon(), "\u{2713}");
        assert_eq!(c.severity, Severity::Info);
    }

    #[test]
    fn check_warn_icon() {
        let c = Check::warn("hmm", "detail");
        assert_eq!(c.icon(), "\u{26A0}");
        assert_eq!(c.severity, Severity::Warning);
    }

    #[test]
    fn check_error_icon() {
        let c = Check::error("bad");
        assert_eq!(c.icon(), "\u{2717}");
        assert_eq!(c.severity, Severity::Error);
    }

    #[test]
    fn check_pass_has_no_detail() {
        let c = Check::pass("label");
        assert!(c.detail.is_none());
    }

    #[test]
    fn check_warn_has_detail() {
        let c = Check::warn("label", "detail");
        assert_eq!(c.detail.as_deref(), Some("detail"));
    }

    #[test]
    fn check_error_has_no_detail() {
        let c = Check::error("label");
        assert!(c.detail.is_none());
    }
}
