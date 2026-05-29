//! `pylon deployments` — list / rollback / promote a deploy.

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
struct Deployment {
    id: String,
    sha: String,
    message: String,
    author: Option<String>,
    status: String,
    #[serde(rename = "startedAt")]
    started_at: String,
}

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let positional: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with('-') && *a != "deployments")
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
        Some("list") | None => run_list(&creds, &project_id, json_mode),
        Some("rollback") => {
            run_rollback(&creds, &project_id, positional.get(1).copied(), json_mode)
        }
        Some(sub) => {
            output::print_error(&format!("unknown subcommand: \"{sub}\""));
            eprintln!("Usage: pylon deployments [list | rollback <id>]");
            ExitCode::Usage
        }
    }
}

fn run_list(creds: &Credentials, project_id: &str, json_mode: bool) -> ExitCode {
    #[derive(serde::Serialize)]
    struct Args<'a> {
        #[serde(rename = "projectId")]
        project_id: &'a str,
        limit: u32,
    }
    let deploys: Vec<Deployment> = match post_json(
        creds,
        "/api/fn/listDeployments",
        &Args {
            project_id,
            limit: 20,
        },
    ) {
        Ok(d) => d,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Error;
        }
    };
    if json_mode {
        let out = serde_json::json!({"deployments": deploys.iter().map(|d| serde_json::json!({
			"id": d.id, "sha": d.sha, "message": d.message, "author": d.author,
			"status": d.status, "startedAt": d.started_at,
		})).collect::<Vec<_>>()});
        println!("{}", serde_json::to_string(&out).unwrap_or_default());
        return ExitCode::Ok;
    }
    if deploys.is_empty() {
        println!("No deployments yet.");
        return ExitCode::Ok;
    }
    println!("SHA       STATUS    STARTED               MESSAGE");
    for d in &deploys {
        let sha = d.sha.chars().take(8).collect::<String>();
        let when = d
            .started_at
            .split('.')
            .next()
            .unwrap_or(&d.started_at)
            .replace('T', " ");
        println!(
            "{:<8}  {:<8}  {:<19}  {}",
            sha,
            d.status,
            &when[..when.len().min(19)],
            d.message
        );
    }
    ExitCode::Ok
}

fn run_rollback(
    creds: &Credentials,
    project_id: &str,
    deployment_id: Option<&str>,
    json_mode: bool,
) -> ExitCode {
    let Some(id) = deployment_id else {
        output::print_error("Usage: pylon deployments rollback <deployment-id>");
        eprintln!("  Find ids with: pylon deployments list");
        return ExitCode::Usage;
    };
    #[derive(serde::Serialize)]
    struct Args<'a> {
        #[serde(rename = "projectId")]
        project_id: &'a str,
        // Field name must match the cloud function exactly. The
        // server-side `rollbackDeployment` takes `deploymentId`
        // (v.id("Deployment")); sending `targetDeploymentId` instead
        // returns "Missing required field deploymentId" and the
        // rollback never queues.
        #[serde(rename = "deploymentId")]
        deployment_id: &'a str,
    }
    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct Out {
        #[serde(rename = "deploymentId")]
        deployment_id: Option<String>,
    }
    let r: Out = match post_json(
        creds,
        "/api/fn/rollbackDeployment",
        &Args {
            project_id,
            deployment_id: id,
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
                "ok": true, "rolledBackTo": id, "newDeploymentId": r.deployment_id,
            }))
            .unwrap_or_default()
        );
    } else {
        println!("✓ Rollback queued to deployment {id}");
        if let Some(d) = &r.deployment_id {
            println!("  New deployment: {d}");
        }
    }
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
