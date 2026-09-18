//! `pylon jobs` — read the project's job queue from the shell.
//!
//! `pylon status` prints a failed-job count and nothing else. This command
//! reads the rows behind that count: which job, which function, when it
//! ran, and the error text. It wraps the cloud's listProjectJobs function,
//! which proxies /admin/jobs on the customer machine with the read-only
//! metrics token.

use pylon_kernel::ExitCode;
use serde::Deserialize;

use crate::cloud_client::{post_json, require_credentials, Credentials};
use crate::commands::args::collect_positional;
use crate::output;
use crate::project_context::resolve_project_slug;

#[derive(Deserialize)]
struct ProjectForCli {
    id: String,
}

/// One queue row as the runtime serializes it (crates/runtime/src/jobs.rs).
#[derive(Deserialize, serde::Serialize)]
pub struct JobRow {
    pub id: String,
    pub name: String,
    pub status: String,
    #[serde(default)]
    pub queue: Option<String>,
    #[serde(default)]
    pub retry_count: u32,
    #[serde(default)]
    pub max_retries: u32,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub completed_at: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Deserialize, serde::Serialize)]
struct JobStats {
    #[serde(default)]
    pending: u64,
    #[serde(default)]
    running: u64,
    #[serde(default)]
    completed: u64,
    #[serde(default)]
    failed: u64,
    #[serde(default)]
    dead: u64,
}

#[derive(Deserialize)]
#[serde(tag = "kind")]
enum JobsResp {
    #[serde(rename = "ok")]
    Ok { data: serde_json::Value },
    #[serde(rename = "unavailable")]
    Unavailable { reason: String },
}

#[derive(serde::Serialize)]
struct JobsArgs<'a> {
    #[serde(rename = "projectId")]
    project_id: &'a str,
    kind: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    queue: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<&'a str>,
}

/// What the user asked for, parsed from argv.
#[derive(Debug, PartialEq)]
pub enum Request {
    List {
        status: Option<String>,
        queue: Option<String>,
        limit: u64,
    },
    Stats,
    Dead,
    One(String),
}

const DEFAULT_LIMIT: u64 = 50;
const MAX_LIMIT: u64 = 500;

/// Parse `pylon jobs ...` argv. `args` includes the leading "jobs".
pub fn parse_request(args: &[String]) -> Result<Request, String> {
    let positional = collect_positional(args, "jobs");
    let status_flag = flag_value(args, "--status");
    let queue_flag = flag_value(args, "--queue");
    let limit = match flag_value(args, "--limit") {
        Some(raw) => raw
            .parse::<u64>()
            .ok()
            .filter(|n| *n >= 1)
            .ok_or_else(|| format!("--limit must be a whole number from 1 to {MAX_LIMIT}"))?
            .min(MAX_LIMIT),
        None => DEFAULT_LIMIT,
    };
    match positional.first().copied() {
        None | Some("list") => Ok(Request::List {
            status: status_flag,
            queue: queue_flag,
            limit,
        }),
        Some("failed") => Ok(Request::List {
            status: Some("failed".into()),
            queue: queue_flag,
            limit,
        }),
        Some("stats") => Ok(Request::Stats),
        Some("dead") => Ok(Request::Dead),
        Some("get") => match positional.get(1) {
            Some(id) => Ok(Request::One((*id).to_string())),
            None => Err("jobs get requires a job id".into()),
        },
        Some(other) => Err(format!(
            "unknown jobs subcommand: \"{other}\" (expected list, failed, dead, stats, or get <id>)"
        )),
    }
}

/// `--flag value` or `--flag=value`.
fn flag_value(args: &[String], flag: &str) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == flag {
            return args.get(i + 1).cloned();
        }
        if let Some(v) = a.strip_prefix(&format!("{flag}=")) {
            return Some(v.to_string());
        }
        i += 1;
    }
    None
}

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let request = match parse_request(args) {
        Ok(r) => r,
        Err(e) => {
            output::print_error(&e);
            eprintln!("  Run: pylon jobs --help");
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
    let project_slug = match resolve_project_slug(args, &creds, json_mode) {
        Ok(s) => s,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Usage;
        }
    };
    let project = match resolve_project(&creds, &project_slug) {
        Ok(p) => p,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Error;
        }
    };

    let body = match &request {
        Request::List {
            status,
            queue,
            limit,
        } => JobsArgs {
            project_id: &project.id,
            kind: "list",
            status: status.as_deref(),
            queue: queue.as_deref(),
            limit: Some(*limit),
            id: None,
        },
        Request::Stats => JobsArgs {
            project_id: &project.id,
            kind: "stats",
            status: None,
            queue: None,
            limit: None,
            id: None,
        },
        Request::Dead => JobsArgs {
            project_id: &project.id,
            kind: "dead",
            status: None,
            queue: None,
            limit: None,
            id: None,
        },
        Request::One(id) => JobsArgs {
            project_id: &project.id,
            kind: "one",
            status: None,
            queue: None,
            limit: None,
            id: Some(id),
        },
    };

    let resp: JobsResp = match post_json(&creds, "/api/fn/listProjectJobs", &body) {
        Ok(r) => r,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Error;
        }
    };
    let data = match resp {
        JobsResp::Ok { data } => data,
        JobsResp::Unavailable { reason } => {
            if json_mode {
                println!(
                    "{}",
                    serde_json::json!({ "slug": project_slug, "jobs": null, "reason": reason })
                );
            } else {
                println!("{project_slug}");
                println!("  Job queue unavailable: {reason}");
            }
            return ExitCode::Ok;
        }
    };

    if json_mode {
        println!("{}", serde_json::to_string(&data).unwrap_or_default());
        return ExitCode::Ok;
    }

    match request {
        Request::Stats => match serde_json::from_value::<JobStats>(data) {
            Ok(s) => {
                println!("{project_slug}");
                println!(
                    "  Jobs: {} pending · {} running · {} completed · {} failed · {} dead",
                    s.pending, s.running, s.completed, s.failed, s.dead
                );
                ExitCode::Ok
            }
            Err(e) => {
                output::print_error(&format!("unexpected stats payload: {e}"));
                ExitCode::Error
            }
        },
        Request::One(_) => match serde_json::from_value::<JobRow>(data) {
            Ok(row) => {
                print_job_detail(&row);
                ExitCode::Ok
            }
            Err(e) => {
                output::print_error(&format!("unexpected job payload: {e}"));
                ExitCode::Error
            }
        },
        Request::List { .. } | Request::Dead => match serde_json::from_value::<Vec<JobRow>>(data) {
            Ok(rows) => {
                print_job_table(&rows);
                ExitCode::Ok
            }
            Err(e) => {
                output::print_error(&format!("unexpected jobs payload: {e}"));
                ExitCode::Error
            }
        },
    }
}

/// The human table: one row per job, error text truncated to one line.
pub fn format_job_table(rows: &[JobRow]) -> String {
    if rows.is_empty() {
        return "No jobs.\n".to_string();
    }
    let mut out = String::new();
    out.push_str(&format!(
        "{:<26} {:<28} {:<10} {:<8} {:<20} ERROR\n",
        "ID", "NAME", "STATUS", "TRIES", "STARTED"
    ));
    for r in rows {
        let tries = format!("{}/{}", r.retry_count, r.max_retries);
        let started = display_time(r.started_at.as_deref().or(r.created_at.as_deref()).unwrap_or(""));
        out.push_str(&format!(
            "{:<26} {:<28} {:<10} {:<8} {:<20} {}\n",
            truncate(&r.id, 26),
            truncate(&r.name, 28),
            r.status,
            tries,
            truncate(&started, 20),
            one_line(r.error.as_deref().unwrap_or(""), 100),
        ));
    }
    out
}

fn print_job_table(rows: &[JobRow]) {
    print!("{}", format_job_table(rows));
}

fn print_job_detail(r: &JobRow) {
    println!("{}", r.id);
    println!("  Name:      {}", r.name);
    println!("  Status:    {}", r.status);
    if let Some(q) = &r.queue {
        println!("  Queue:     {q}");
    }
    println!("  Tries:     {}/{}", r.retry_count, r.max_retries);
    if let Some(t) = &r.created_at {
        println!("  Created:   {}", display_time(t));
    }
    if let Some(t) = &r.started_at {
        println!("  Started:   {}", display_time(t));
    }
    if let Some(t) = &r.completed_at {
        println!("  Completed: {}", display_time(t));
    }
    if let Some(e) = &r.error {
        println!("  Error:");
        for line in e.lines() {
            println!("    {line}");
        }
    }
}

/// The runtime stamps job times as epoch seconds with a trailing "Z"
/// (crates/runtime/src/jobs.rs, now_iso). Show them as ISO 8601; pass any
/// other form through unchanged.
pub fn display_time(raw: &str) -> String {
    match raw.strip_suffix('Z').and_then(|d| d.parse::<u64>().ok()) {
        Some(secs) if raw.len() <= 12 => pylon_kernel::util::epoch_to_iso(secs),
        _ => raw.to_string(),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

/// Collapse an error to its first line, bounded, so the table stays a table.
fn one_line(s: &str, max: usize) -> String {
    let first = s.lines().next().unwrap_or("");
    truncate(first, max)
}

fn resolve_project(creds: &Credentials, slug: &str) -> Result<ProjectForCli, String> {
    #[derive(serde::Serialize)]
    struct Args<'a> {
        slug: &'a str,
    }
    post_json(creds, "/api/fn/getProjectForCli", &Args { slug })
        .map_err(|e| format!("Could not resolve project \"{slug}\": {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn bare_jobs_lists_with_defaults() {
        assert_eq!(
            parse_request(&argv(&["jobs"])).unwrap(),
            Request::List {
                status: None,
                queue: None,
                limit: DEFAULT_LIMIT
            }
        );
    }

    #[test]
    fn failed_is_list_with_status_filter() {
        assert_eq!(
            parse_request(&argv(&["jobs", "failed", "--limit", "10"])).unwrap(),
            Request::List {
                status: Some("failed".into()),
                queue: None,
                limit: 10
            }
        );
    }

    #[test]
    fn project_flag_value_is_not_a_subcommand() {
        assert_eq!(
            parse_request(&argv(&["jobs", "--project", "acme", "stats"])).unwrap(),
            Request::Stats
        );
    }

    #[test]
    fn get_needs_an_id() {
        assert!(parse_request(&argv(&["jobs", "get"])).is_err());
        assert_eq!(
            parse_request(&argv(&["jobs", "get", "job_1"])).unwrap(),
            Request::One("job_1".into())
        );
    }

    #[test]
    fn limit_is_validated_and_clamped() {
        assert!(parse_request(&argv(&["jobs", "--limit", "0"])).is_err());
        assert!(parse_request(&argv(&["jobs", "--limit=abc"])).is_err());
        match parse_request(&argv(&["jobs", "--limit=9999"])).unwrap() {
            Request::List { limit, .. } => assert_eq!(limit, MAX_LIMIT),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn status_and_queue_flags_reach_the_list() {
        assert_eq!(
            parse_request(&argv(&["jobs", "--status=dead", "--queue", "mail"])).unwrap(),
            Request::List {
                status: Some("dead".into()),
                queue: Some("mail".into()),
                limit: DEFAULT_LIMIT
            }
        );
    }

    #[test]
    fn table_keeps_errors_on_one_line() {
        let rows = vec![JobRow {
            id: "job_1".into(),
            name: "snapshotDaily".into(),
            status: "failed".into(),
            queue: None,
            retry_count: 3,
            max_retries: 3,
            created_at: Some("2026-09-18T16:00:00Z".into()),
            started_at: None,
            completed_at: None,
            error: Some("CALL_CANCELLED: idle timeout\nat writeDaily".into()),
        }];
        let table = format_job_table(&rows);
        let lines: Vec<&str> = table.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[1].contains("snapshotDaily"));
        assert!(lines[1].contains("3/3"));
        assert!(lines[1].contains("CALL_CANCELLED: idle timeout"));
        assert!(!lines[1].contains("writeDaily"));
    }

    #[test]
    fn epoch_z_times_render_as_iso() {
        assert_eq!(display_time("1789750643Z"), "2026-09-18T16:57:23Z");
        assert_eq!(display_time("2026-09-18T16:57:23Z"), "2026-09-18T16:57:23Z");
        assert_eq!(display_time(""), "");
    }

    #[test]
    fn empty_table_says_so() {
        assert_eq!(format_job_table(&[]), "No jobs.\n");
    }
}
