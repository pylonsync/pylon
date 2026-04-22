use statecraft_core::{ExitCode, Severity};
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
        _ => {
            // Bun is a hard dep for the function runtime. New users hit this
            // on a fresh machine and the error "Bun not found" doesn't tell
            // them what to do next. Offer the install command inline so
            // `statecraft doctor` is actionable without Googling.
            let hint = if cfg!(target_os = "windows") {
                "Install Bun: powershell -c \"irm bun.sh/install.ps1 | iex\""
            } else {
                "Install Bun: curl -fsSL https://bun.sh/install | bash"
            };
            Check::warn("Bun not found (required by function runtime)", hint)
        }
    }
}

fn check_database() -> Check {
    if std::fs::metadata("statecraft.dev.db").is_ok() {
        Check::pass("Database exists (statecraft.dev.db)")
    } else {
        Check::warn(
            "Database not found (statecraft.dev.db)",
            "run `statecraft dev` to create it",
        )
    }
}

fn check_admin_token() -> Check {
    if std::env::var("STATECRAFT_ADMIN_TOKEN").is_ok() {
        Check::pass("STATECRAFT_ADMIN_TOKEN set")
    } else {
        Check::warn(
            "STATECRAFT_ADMIN_TOKEN not set",
            "auth endpoints will be unprotected",
        )
    }
}

// ---------------------------------------------------------------------------
// Security-surface checks — flag insecure env var combinations
// ---------------------------------------------------------------------------

fn is_dev_mode() -> bool {
    // Same resolution as the server: default true when unset, so an unset
    // dev-mode in a production container is a dev-mode deployment.
    match std::env::var("STATECRAFT_DEV_MODE") {
        Ok(v) => v == "1" || v == "true",
        Err(_) => true,
    }
}

fn check_dev_mode_in_prod_shape() -> Check {
    if !is_dev_mode() {
        return Check::pass("STATECRAFT_DEV_MODE=false (prod mode)");
    }
    // Heuristic for "looks like production": CORS set to something other
    // than `*`, OR a TLS proxy domain hinted via env, OR a non-localhost
    // FILES_DIR. If any of those are set, STATECRAFT_DEV_MODE=true is
    // probably a misconfig.
    let cors = std::env::var("STATECRAFT_CORS_ORIGIN").unwrap_or_else(|_| "*".into());
    let has_non_dev_cors = cors != "*" && !cors.contains("localhost") && !cors.contains("127.0.0.1");
    if has_non_dev_cors {
        Check::warn(
            "STATECRAFT_DEV_MODE=true but STATECRAFT_CORS_ORIGIN looks production-shaped",
            "dev mode keeps the legacy /api/auth/session + OAuth email-shortcut paths open — set STATECRAFT_DEV_MODE=false before going live",
        )
    } else {
        Check::pass("STATECRAFT_DEV_MODE=true (development)")
    }
}

fn check_cors_safety() -> Check {
    let dev = is_dev_mode();
    match std::env::var("STATECRAFT_CORS_ORIGIN") {
        Ok(v) if v == "*" && !dev => Check::error(
            "STATECRAFT_CORS_ORIGIN=\"*\" in production — server will refuse to start",
        ),
        Ok(v) if v == "*" => Check::warn(
            "STATECRAFT_CORS_ORIGIN=\"*\"",
            "fine for dev; must be set to an explicit origin in production",
        ),
        Ok(_) => Check::pass("STATECRAFT_CORS_ORIGIN set"),
        Err(_) if dev => Check::pass("STATECRAFT_CORS_ORIGIN unset (dev mode → defaults to *)"),
        Err(_) => Check::error(
            "STATECRAFT_CORS_ORIGIN unset in production — server will refuse to start",
        ),
    }
}

fn check_csrf_origins() -> Check {
    if is_dev_mode() {
        return Check::pass("STATECRAFT_CSRF_ORIGINS (dev mode → wildcard)");
    }
    let csrf = std::env::var("STATECRAFT_CSRF_ORIGINS").ok();
    let cors = std::env::var("STATECRAFT_CORS_ORIGIN").ok();
    match (csrf, cors) {
        (Some(v), _) if !v.trim().is_empty() => Check::pass("STATECRAFT_CSRF_ORIGINS set"),
        (_, Some(c)) if c != "*" => Check::warn(
            "STATECRAFT_CSRF_ORIGINS not set — falling back to STATECRAFT_CORS_ORIGIN",
            "CSRF check uses CORS origin by default. Set STATECRAFT_CSRF_ORIGINS explicitly if they differ.",
        ),
        _ => Check::warn(
            "STATECRAFT_CSRF_ORIGINS and STATECRAFT_CORS_ORIGIN both unset",
            "CSRF protection on state-changing routes will be bypassed",
        ),
    }
}

fn check_session_db() -> Check {
    match std::env::var("STATECRAFT_SESSION_DB") {
        Ok(v) if !v.is_empty() => Check::pass(format!("STATECRAFT_SESSION_DB={v}")),
        _ => Check::warn(
            "STATECRAFT_SESSION_DB not set",
            "sessions + OAuth state stay in-memory and are lost on restart",
        ),
    }
}

fn check_oauth_pairs() -> Check {
    // Partial config (client id without secret or vice versa) silently
    // disables the provider; surfacing it here saves a confusing 404.
    let google_id = std::env::var("STATECRAFT_OAUTH_GOOGLE_CLIENT_ID").ok();
    let google_sec = std::env::var("STATECRAFT_OAUTH_GOOGLE_CLIENT_SECRET").ok();
    let github_id = std::env::var("STATECRAFT_OAUTH_GITHUB_CLIENT_ID").ok();
    let github_sec = std::env::var("STATECRAFT_OAUTH_GITHUB_CLIENT_SECRET").ok();

    let google_partial = google_id.is_some() != google_sec.is_some();
    let github_partial = github_id.is_some() != github_sec.is_some();

    if google_partial && github_partial {
        Check::warn(
            "Partial OAuth config: both Google and GitHub missing a client id/secret",
            "provider stays disabled until both halves are set",
        )
    } else if google_partial {
        Check::warn(
            "Partial OAuth config: Google missing client id or secret",
            "Google sign-in stays disabled until both STATECRAFT_OAUTH_GOOGLE_CLIENT_ID and ..._SECRET are set",
        )
    } else if github_partial {
        Check::warn(
            "Partial OAuth config: GitHub missing client id or secret",
            "GitHub sign-in stays disabled until both STATECRAFT_OAUTH_GITHUB_CLIENT_ID and ..._SECRET are set",
        )
    } else if google_id.is_none() && github_id.is_none() {
        Check::pass("OAuth not configured (magic-code + session tokens only)")
    } else {
        Check::pass("OAuth configured")
    }
}

fn check_fn_rate_limits() -> Check {
    // Non-essential — just surface the current caps so operators can tell
    // which values apply (defaults vs overrides).
    let max = std::env::var("STATECRAFT_FN_RATE_LIMIT_MAX").unwrap_or_else(|_| "30".into());
    let window =
        std::env::var("STATECRAFT_FN_RATE_LIMIT_WINDOW").unwrap_or_else(|_| "60".into());
    Check::pass(format!(
        "Fn rate limit: {max} calls per {window}s per (user, fn)"
    ))
}

fn check_jobs_db() -> Check {
    match std::env::var("STATECRAFT_JOBS_DB") {
        Ok(v) if !v.is_empty() => Check::pass(format!("STATECRAFT_JOBS_DB={v}")),
        _ => Check::pass("STATECRAFT_JOBS_DB unset (defaults to statecraft.jobs.db in cwd)"),
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
    if std::fs::metadata("statecraft.manifest.json").is_ok() {
        Check::pass("Manifest found (statecraft.manifest.json)")
    } else if std::fs::metadata("app.ts").is_ok() {
        Check::pass("Manifest found (app.ts)")
    } else {
        Check::error("No manifest found (expected statecraft.manifest.json or app.ts)")
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
                    "run `statecraft schema push` to generate migrations",
                )
            }
        }
        Err(_) => Check::warn(
            "No migrations directory found",
            "run `statecraft schema push` to create one",
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
        // Security-surface checks — catch insecure env var combinations
        // before they ship. Most return Info/Warn; only "CORS=* in prod"
        // is Error because the server refuses to start in that state.
        check_dev_mode_in_prod_shape(),
        check_cors_safety(),
        check_csrf_origins(),
        check_session_db(),
        check_oauth_pairs(),
        check_fn_rate_limits(),
        check_jobs_db(),
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
        println!("statecraft doctor");
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
