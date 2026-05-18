//! `pylon status` — one-glance project health.
//!
//! Wraps getProjectMetrics + a quick deployment-count summary. Useful
//! for an SSH-into-a-server check before pushing the next deploy.

use pylon_kernel::ExitCode;
use serde::Deserialize;

use crate::cloud_client::{post_json, require_credentials, Credentials};
use crate::output;
use crate::project_context::resolve_project_slug;

#[derive(Deserialize)]
struct ProjectForCli {
    id: String,
    slug: String,
    status: String,
    region: String,
}

#[derive(Deserialize)]
struct Requests {
    total: u64,
    ok: u64,
    error: u64,
}

#[derive(Deserialize)]
struct Jobs {
    #[serde(default)]
    running: u64,
    #[serde(default)]
    pending: u64,
    #[serde(default)]
    failed: u64,
}

#[derive(Deserialize)]
struct Realtime {
    #[serde(default, rename = "ws_connections")]
    ws: u64,
    #[serde(default, rename = "sse_connections")]
    sse: u64,
}

#[derive(Deserialize)]
struct Metrics {
    #[serde(default)]
    uptime_secs: u64,
    requests: Requests,
    jobs: Option<Jobs>,
    realtime: Option<Realtime>,
}

#[derive(Deserialize)]
#[serde(tag = "kind")]
enum MetricsResp {
    #[serde(rename = "ok")]
    Ok { metrics: Metrics },
    #[serde(rename = "unavailable")]
    Unavailable { reason: String },
}

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
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

    #[derive(serde::Serialize)]
    struct Args<'a> {
        #[serde(rename = "projectId")]
        project_id: &'a str,
    }
    let resp: MetricsResp = match post_json(
        creds_arg(&creds),
        "/api/fn/getProjectMetrics",
        &Args {
            project_id: &project.id,
        },
    ) {
        Ok(r) => r,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Error;
        }
    };
    let m = match resp {
        MetricsResp::Ok { metrics } => metrics,
        MetricsResp::Unavailable { reason } => {
            if json_mode {
                println!(
                    "{}",
                    serde_json::to_string(&serde_json::json!({
                        "slug": project.slug,
                        "status": project.status,
                        "region": project.region,
                        "metrics": null,
                        "reason": reason,
                    }))
                    .unwrap_or_default()
                );
            } else {
                println!("{}  ({})", project.slug, project.status);
                println!("  Region: {}", project.region);
                println!("  Live metrics unavailable: {reason}");
            }
            return ExitCode::Ok;
        }
    };

    if json_mode {
        let out = serde_json::json!({
            "slug": project.slug,
            "status": project.status,
            "region": project.region,
            "uptime_secs": m.uptime_secs,
            "requests": {"total": m.requests.total, "ok": m.requests.ok, "error": m.requests.error},
            "jobs": m.jobs.as_ref().map(|j| serde_json::json!({"running": j.running, "pending": j.pending, "failed": j.failed})),
            "realtime": m.realtime.as_ref().map(|r| serde_json::json!({"ws": r.ws, "sse": r.sse})),
        });
        println!("{}", serde_json::to_string(&out).unwrap_or_default());
        return ExitCode::Ok;
    }
    println!("{}  ({})", project.slug, project.status);
    println!("  Region:   {}", project.region);
    println!("  Uptime:   {}", fmt_secs(m.uptime_secs));
    println!(
        "  Requests: {} total · {} ok · {} error",
        m.requests.total, m.requests.ok, m.requests.error
    );
    if let Some(j) = &m.jobs {
        println!(
            "  Jobs:     {} running · {} pending · {} failed",
            j.running, j.pending, j.failed
        );
    }
    if let Some(r) = &m.realtime {
        println!("  Realtime: {} WS · {} SSE", r.ws, r.sse);
    }
    ExitCode::Ok
}

fn fmt_secs(s: u64) -> String {
    if s < 60 {
        return format!("{s}s");
    }
    if s < 3600 {
        return format!("{}m {}s", s / 60, s % 60);
    }
    if s < 86_400 {
        return format!("{}h {}m", s / 3600, (s % 3600) / 60);
    }
    format!("{}d {}h", s / 86_400, (s % 86_400) / 3600)
}

fn creds_arg(c: &Credentials) -> &Credentials {
    c
}

fn resolve_project(creds: &Credentials, slug: &str) -> Result<ProjectForCli, String> {
    #[derive(serde::Serialize)]
    struct Args<'a> {
        slug: &'a str,
    }
    post_json(creds, "/api/fn/getProjectForCli", &Args { slug })
        .map_err(|e| format!("Could not resolve project \"{slug}\": {e}"))
}
