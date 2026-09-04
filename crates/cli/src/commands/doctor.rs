use pylon_kernel::{ExitCode, Severity};
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
    match std::process::Command::new("bun").arg("--version").output() {
        Ok(out) if out.status.success() => {
            let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
            Check::pass(format!("Bun {version} installed"))
        }
        _ => {
            // Bun is a hard dep for the function runtime. New users hit this
            // on a fresh machine and the error "Bun not found" doesn't tell
            // them what to do next. Offer the install command inline so
            // `pylon doctor` is actionable without Googling.
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
    // `pylon dev` creates the SQLite file under `.pylon/dev.db` (see
    // crates/cli/src/commands/dev.rs). Some older workspaces used a
    // bare `pylon.dev.db` at cwd — accept either, prefer the new one.
    const CANDIDATES: &[&str] = &[".pylon/dev.db", "pylon.dev.db"];
    for path in CANDIDATES {
        if std::fs::metadata(path).is_ok() {
            return Check::pass(&format!("Database exists ({path})"));
        }
    }
    Check::warn(
        "Database not found (checked .pylon/dev.db, pylon.dev.db)",
        "run `pylon dev` to create it",
    )
}

fn check_admin_token() -> Check {
    if std::env::var("PYLON_ADMIN_TOKEN").is_ok() {
        Check::pass("PYLON_ADMIN_TOKEN set")
    } else {
        Check::warn(
            "PYLON_ADMIN_TOKEN not set",
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
    match std::env::var("PYLON_DEV_MODE") {
        Ok(v) => v == "1" || v == "true",
        Err(_) => true,
    }
}

fn check_dev_mode_in_prod_shape() -> Check {
    if !is_dev_mode() {
        return Check::pass("PYLON_DEV_MODE=false (prod mode)");
    }
    // Heuristic for "looks like production": CORS set to something other
    // than `*`, OR a TLS proxy domain hinted via env, OR a non-localhost
    // FILES_DIR. If any of those are set, PYLON_DEV_MODE=true is
    // probably a misconfig.
    let cors = std::env::var("PYLON_CORS_ORIGIN").unwrap_or_else(|_| "*".into());
    let has_non_dev_cors =
        cors != "*" && !cors.contains("localhost") && !cors.contains("127.0.0.1");
    if has_non_dev_cors {
        Check::warn(
            "PYLON_DEV_MODE=true but PYLON_CORS_ORIGIN looks production-shaped",
            "dev mode keeps the legacy /api/auth/session + OAuth email-shortcut paths open — set PYLON_DEV_MODE=false before going live",
        )
    } else {
        Check::pass("PYLON_DEV_MODE=true (development)")
    }
}

fn check_cors_safety() -> Check {
    let dev = is_dev_mode();
    match std::env::var("PYLON_CORS_ORIGIN") {
        Ok(v) if v == "*" && !dev => {
            Check::error("PYLON_CORS_ORIGIN=\"*\" in production — server will refuse to start")
        }
        Ok(v) if v == "*" => Check::warn(
            "PYLON_CORS_ORIGIN=\"*\"",
            "fine for dev; must be set to an explicit origin in production",
        ),
        Ok(_) => Check::pass("PYLON_CORS_ORIGIN set"),
        Err(_) if dev => Check::pass("PYLON_CORS_ORIGIN unset (dev mode → defaults to *)"),
        Err(_) => {
            Check::error("PYLON_CORS_ORIGIN unset in production — server will refuse to start")
        }
    }
}

fn check_csrf_origins() -> Check {
    if is_dev_mode() {
        return Check::pass("PYLON_CSRF_ORIGINS (dev mode → wildcard)");
    }
    let csrf = std::env::var("PYLON_CSRF_ORIGINS").ok();
    let cors = std::env::var("PYLON_CORS_ORIGIN").ok();
    match (csrf, cors) {
        (Some(v), _) if !v.trim().is_empty() => Check::pass("PYLON_CSRF_ORIGINS set"),
        (_, Some(c)) if c != "*" => Check::warn(
            "PYLON_CSRF_ORIGINS not set — falling back to PYLON_CORS_ORIGIN",
            "CSRF check uses CORS origin by default. Set PYLON_CSRF_ORIGINS explicitly if they differ.",
        ),
        _ => Check::warn(
            "PYLON_CSRF_ORIGINS and PYLON_CORS_ORIGIN both unset",
            "CSRF protection on state-changing routes will be bypassed",
        ),
    }
}

fn check_session_db() -> Check {
    match std::env::var("PYLON_SESSION_DB") {
        Ok(v) if !v.is_empty() => Check::pass(format!("PYLON_SESSION_DB={v}")),
        _ => Check::warn(
            "PYLON_SESSION_DB not set",
            "sessions + OAuth state stay in-memory and are lost on restart",
        ),
    }
}

fn check_oauth_pairs() -> Check {
    // Partial config (client id without secret or vice versa) silently
    // disables the provider; surfacing it here saves a confusing 404.
    let google_id = std::env::var("PYLON_OAUTH_GOOGLE_CLIENT_ID").ok();
    let google_sec = std::env::var("PYLON_OAUTH_GOOGLE_CLIENT_SECRET").ok();
    let github_id = std::env::var("PYLON_OAUTH_GITHUB_CLIENT_ID").ok();
    let github_sec = std::env::var("PYLON_OAUTH_GITHUB_CLIENT_SECRET").ok();

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
            "Google sign-in stays disabled until both PYLON_OAUTH_GOOGLE_CLIENT_ID and ..._SECRET are set",
        )
    } else if github_partial {
        Check::warn(
            "Partial OAuth config: GitHub missing client id or secret",
            "GitHub sign-in stays disabled until both PYLON_OAUTH_GITHUB_CLIENT_ID and ..._SECRET are set",
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
    let max = std::env::var("PYLON_FN_RATE_LIMIT_MAX").unwrap_or_else(|_| "30".into());
    let window = std::env::var("PYLON_FN_RATE_LIMIT_WINDOW").unwrap_or_else(|_| "60".into());
    Check::pass(format!(
        "Fn rate limit: {max} calls per {window}s per (user, fn)"
    ))
}

fn check_jobs_db() -> Check {
    let database_url = std::env::var("DATABASE_URL").ok();
    let jobs_in_memory = env_flag("PYLON_JOBS_IN_MEMORY");
    let cluster_required = env_flag("PYLON_CLUSTER_REQUIRED");
    let cluster_bus = std::env::var("PYLON_CLUSTER_BUS").ok();
    let jobs_db = std::env::var("PYLON_JOBS_DB").ok();
    let workflows_db = std::env::var("PYLON_WORKFLOWS_DB").ok();

    check_background_storage(
        database_url.as_deref(),
        jobs_in_memory,
        cluster_required,
        cluster_bus.as_deref(),
        jobs_db.as_deref(),
        workflows_db.as_deref(),
    )
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|value| matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

fn check_background_storage(
    database_url: Option<&str>,
    jobs_in_memory: bool,
    cluster_required: bool,
    cluster_bus: Option<&str>,
    jobs_db: Option<&str>,
    workflows_db: Option<&str>,
) -> Check {
    let has_postgres = database_url
        .is_some_and(|url| url.starts_with("postgres://") || url.starts_with("postgresql://"));
    if cluster_required && !has_postgres {
        return Check::error("PYLON_CLUSTER_REQUIRED needs a Postgres DATABASE_URL");
    }
    if cluster_required && cluster_bus.is_none_or(|value| value.is_empty()) {
        return Check::error("PYLON_CLUSTER_REQUIRED needs PYLON_CLUSTER_BUS");
    }
    if jobs_in_memory && cluster_required {
        return Check::error("PYLON_JOBS_IN_MEMORY conflicts with PYLON_CLUSTER_REQUIRED");
    }
    if jobs_in_memory {
        return Check::warn(
            "Jobs and workflows use memory only",
            "pending work is lost on restart and cannot fail over",
        );
    }

    if has_postgres {
        return Check::pass("Jobs and workflows use shared Postgres tables");
    }

    match (
        jobs_db.filter(|v| !v.is_empty()),
        workflows_db.filter(|v| !v.is_empty()),
    ) {
        (Some(jobs), Some(workflows)) => Check::pass(format!(
            "SQLite background stores: jobs={jobs}, workflows={workflows}"
        )),
        (Some(jobs), None) => Check::pass(format!(
            "SQLite background stores: jobs={jobs}, workflows beside app database"
        )),
        (None, Some(workflows)) => Check::pass(format!(
            "SQLite background stores: jobs beside app database, workflows={workflows}"
        )),
        (None, None) => Check::pass("Jobs and workflows use SQLite sidecars beside app database"),
    }
}

fn check_port() -> Check {
    match std::net::TcpListener::bind("127.0.0.1:4321") {
        Ok(_listener) => Check::pass("Port 4321 available"),
        Err(_) => Check::warn("Port 4321 in use", "dev server may fail to start"),
    }
}

fn check_disk_space() -> Check {
    let Some(avail_mb) = available_mb() else {
        return Check::pass("Disk space check skipped (free space unreadable)");
    };
    if avail_mb < 100 {
        return Check::warn(
            format!("Low disk space ({avail_mb} MB available)"),
            "at least 100 MB recommended",
        );
    }
    Check::pass(format!("Disk space OK ({avail_mb} MB available)"))
}

/// Megabytes free on the volume holding the current directory.
#[cfg(not(windows))]
fn available_mb() -> Option<u64> {
    let out = std::process::Command::new("df")
        .args(["-k", "."])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    // Second line carries the stats; 4th column is available KB.
    let line = text.lines().nth(1)?;
    let cols: Vec<&str> = line.split_whitespace().collect();
    let avail_kb: u64 = cols.get(3)?.parse().ok()?;
    Some(avail_kb / 1024)
}

/// Windows has no `df`. Ask the volume directly — and ask for the quota-aware
/// figure, since the bytes free on the disk are not necessarily bytes this
/// user may write.
#[cfg(windows)]
fn available_mb() -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let dir = std::env::current_dir().ok()?;
    let mut wide: Vec<u16> = dir.as_os_str().encode_wide().collect();
    wide.push(0);

    let mut free_to_caller: u64 = 0;
    // SAFETY: a null-terminated directory path in, one out-parameter written
    // on success; the two totals we do not need are passed as null.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free_to_caller,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return None;
    }
    Some(free_to_caller / (1024 * 1024))
}

fn check_manifest() -> Check {
    // Accept either the root layout (legacy / `pylon init --frontend=none`
    // before workspace scaffold) or the workspace layout `pylon init`
    // currently produces (`apps/api/app.ts` next to its sibling web/
    // app). Also `schema.ts` — some downstream apps named theirs that
    // way before the convention settled.
    const CANDIDATES: &[&str] = &[
        "pylon.manifest.json",
        "app.ts",
        "schema.ts",
        "apps/api/pylon.manifest.json",
        "apps/api/app.ts",
        "apps/api/schema.ts",
    ];
    for path in CANDIDATES {
        if std::fs::metadata(path).is_ok() {
            return Check::pass(&format!("Manifest found ({path})"));
        }
    }
    Check::error(
        "No manifest found (checked: pylon.manifest.json, app.ts, schema.ts, apps/api/{app,schema}.ts)",
    )
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
                    "run `pylon schema push` to generate migrations",
                )
            }
        }
        Err(_) => Check::warn(
            "No migrations directory found",
            "run `pylon schema push` to create one",
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
    std::env::var("NO_COLOR").is_err() && std::env::var("TERM").map(|t| t != "dumb").unwrap_or(true)
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

    let passed = checks
        .iter()
        .filter(|c| c.severity == Severity::Info)
        .count();
    let warnings = checks
        .iter()
        .filter(|c| c.severity == Severity::Warning)
        .count();
    let errors = checks
        .iter()
        .filter(|c| c.severity == Severity::Error)
        .count();

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
        println!("pylon doctor");
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

    #[test]
    fn postgres_uses_shared_background_tables() {
        let c = check_background_storage(
            Some("postgres://localhost/app"),
            false,
            true,
            Some("relay"),
            Some("ignored.jobs.db"),
            Some("ignored.workflows.db"),
        );
        assert_eq!(c.severity, Severity::Info);
        assert_eq!(c.label, "Jobs and workflows use shared Postgres tables");
    }

    #[test]
    fn cluster_mode_rejects_in_memory_background_state() {
        let c = check_background_storage(
            Some("postgres://localhost/app"),
            true,
            true,
            Some("relay"),
            None,
            None,
        );
        assert_eq!(c.severity, Severity::Error);
    }

    #[test]
    fn cluster_mode_rejects_sqlite_background_state() {
        let c = check_background_storage(None, false, true, Some("relay"), None, None);
        assert_eq!(c.severity, Severity::Error);
        assert_eq!(
            c.label,
            "PYLON_CLUSTER_REQUIRED needs a Postgres DATABASE_URL"
        );
    }

    #[test]
    fn cluster_mode_rejects_missing_bus() {
        let c = check_background_storage(
            Some("postgres://localhost/app"),
            false,
            true,
            None,
            None,
            None,
        );
        assert_eq!(c.severity, Severity::Error);
        assert_eq!(c.label, "PYLON_CLUSTER_REQUIRED needs PYLON_CLUSTER_BUS");
    }

    #[test]
    fn sqlite_reports_sidecar_defaults() {
        let c = check_background_storage(None, false, false, None, None, None);
        assert_eq!(c.severity, Severity::Info);
        assert_eq!(
            c.label,
            "Jobs and workflows use SQLite sidecars beside app database"
        );
    }
}
