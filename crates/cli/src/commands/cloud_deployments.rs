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
    // collect_positional drops flag VALUES as well as the flags. A
    // plain "skip anything starting with -" leaves the value of
    // `--project <slug>` floating, where it gets read as a deployment
    // id: `pylon deployments logs --project acme` looked up a
    // deployment called "acme" instead of the latest one.
    let positional = crate::commands::args::collect_positional(args, "deployments");
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
        Some("rollback") => run_rollback(
            &creds,
            &project_slug,
            &project_id,
            positional.get(1).copied(),
            args,
            json_mode,
        ),
        Some("logs") => run_logs(&creds, &project_id, positional.get(1).copied(), json_mode),
        Some(sub) => {
            output::print_error(&format!("unknown subcommand: \"{sub}\""));
            eprintln!("Usage: pylon deployments [list | logs [<id>] | rollback <id>]");
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
    project_slug: &str,
    project_id: &str,
    deployment_id: Option<&str>,
    args: &[String],
    json_mode: bool,
) -> ExitCode {
    let Some(id) = deployment_id else {
        output::print_error("Usage: pylon deployments rollback <deployment-id> [--yes]");
        eprintln!("  Find ids with: pylon deployments list");
        return ExitCode::Usage;
    };
    // Rollback replaces what production is serving. It sat behind no
    // confirmation at all while `db restore`, `secrets rm`, and
    // `domains rm` all required one, so a mistyped id silently shipped
    // the wrong build.
    if !output::confirm_destructive(
        args,
        &format!("roll {project_slug} back to deployment {id}"),
        json_mode,
    ) {
        return ExitCode::Usage;
    }
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

/// `pylon deployments logs [<id>]` — print a deployment's captured build log.
/// With no id, targets the most recent deployment (the common "my last deploy
/// failed, why?" case). The build log is tee'd to Tigris by the builder and
/// read back onto the Deployment row, so this is one server call.
fn run_logs(
    creds: &Credentials,
    project_id: &str,
    deployment_id: Option<&str>,
    json_mode: bool,
) -> ExitCode {
    // Resolve the target: the given id OR git sha (people paste the sha
    // from `deployments list` — passing it straight through used to 400
    // on the server's id validation), else the newest deployment.
    #[derive(serde::Serialize)]
    struct ListArgs<'a> {
        #[serde(rename = "projectId")]
        project_id: &'a str,
        limit: u32,
    }
    let list = |limit: u32| -> Result<Vec<Deployment>, String> {
        post_json(
            creds,
            "/api/fn/listDeployments",
            &ListArgs { project_id, limit },
        )
    };
    let id: String = match deployment_id {
        Some(arg) => {
            // Entity ids are 40 lowercase hex chars. Anything else — a
            // short or full git sha, a sha prefix — resolves via the
            // deployment list.
            let looks_like_id = arg.len() == 40
                && arg
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase());
            if looks_like_id {
                arg.to_string()
            } else {
                let deploys = match list(50) {
                    Ok(d) => d,
                    Err(e) => {
                        output::print_error(&e);
                        return ExitCode::Error;
                    }
                };
                match deploys
                    .iter()
                    .find(|d| d.sha.starts_with(arg) || d.id.starts_with(arg))
                {
                    Some(d) => d.id.clone(),
                    None => {
                        output::print_error(&format!(
                            "no deployment matches \"{arg}\" (checked sha + id prefixes of the last 50)"
                        ));
                        eprintln!("  See: pylon deployments list");
                        return ExitCode::Error;
                    }
                }
            }
        }
        None => {
            let deploys = match list(1) {
                Ok(d) => d,
                Err(e) => {
                    output::print_error(&e);
                    return ExitCode::Error;
                }
            };
            match deploys.first() {
                Some(d) => d.id.clone(),
                None => {
                    println!("No deployments yet.");
                    return ExitCode::Ok;
                }
            }
        }
    };

    #[derive(serde::Serialize)]
    struct Args<'a> {
        #[serde(rename = "projectId")]
        project_id: &'a str,
        #[serde(rename = "deploymentId")]
        deployment_id: &'a str,
    }
    #[derive(Deserialize)]
    struct LogOut {
        id: String,
        sha: Option<String>,
        status: Option<String>,
        error: Option<String>,
        #[serde(rename = "buildLog")]
        build_log: Option<String>,
    }
    let out: LogOut = match post_json(
        creds,
        "/api/fn/getDeploymentBuildLog",
        &Args {
            project_id,
            deployment_id: &id,
        },
    ) {
        Ok(o) => o,
        Err(e) => {
            output::print_error(&e);
            // The most common way to land here is asking for the log while
            // the build is still running (it attaches at the terminal
            // transition). Say so — a bare server error reads as a bug.
            eprintln!(
                "  If the deployment is still building, the log attaches when it \
                 finishes — check status with: pylon deployments list"
            );
            return ExitCode::Error;
        }
    };

    if json_mode {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "id": out.id, "sha": out.sha, "status": out.status,
                "error": out.error, "buildLog": out.build_log,
            }))
            .unwrap_or_default()
        );
        return ExitCode::Ok;
    }

    let sha = out
        .sha
        .as_deref()
        .unwrap_or("")
        .chars()
        .take(8)
        .collect::<String>();
    println!(
        "Deployment {} ({})  status: {}",
        out.id,
        sha,
        out.status.as_deref().unwrap_or("?")
    );
    if let Some(err) = out.error.as_deref().filter(|e| !e.is_empty()) {
        println!("Error: {err}");
    }
    println!("{}", "─".repeat(60));
    match out.build_log.as_deref().filter(|l| !l.is_empty()) {
        Some(log) => println!("{log}"),
        None => println!("(no build log captured for this deployment)"),
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use crate::commands::args::collect_positional;

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    fn positional(parts: &[&str]) -> Vec<String> {
        let args = argv(parts);
        collect_positional(&args, "deployments")
            .into_iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn positional_keeps_subcommand_and_id() {
        assert_eq!(
            positional(&["deployments", "rollback", "dep_123"]),
            vec!["rollback", "dep_123"]
        );
    }

    #[test]
    fn positional_drops_the_project_flag_value() {
        // The value must not survive as a positional — it would be read
        // as the deployment id.
        assert_eq!(
            positional(&["deployments", "logs", "--project", "acme"]),
            vec!["logs"]
        );
    }

    #[test]
    fn positional_drops_the_project_flag_value_before_the_id() {
        assert_eq!(
            positional(&["deployments", "rollback", "--project", "acme", "dep_9"]),
            vec!["rollback", "dep_9"]
        );
    }

    #[test]
    fn positional_handles_the_equals_form() {
        // `--project=acme` carries its own value, so the next token is a
        // real positional and must be kept.
        assert_eq!(
            positional(&["deployments", "rollback", "--project=acme", "dep_9"]),
            vec!["rollback", "dep_9"]
        );
    }

    #[test]
    fn positional_drops_valueless_flags() {
        assert_eq!(
            positional(&["deployments", "list", "--json", "--yes"]),
            vec!["list"]
        );
    }
}
