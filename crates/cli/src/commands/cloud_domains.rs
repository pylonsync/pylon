//! `pylon domains` — list / add / verify / rm custom domains.

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
struct Domain {
    id: String,
    hostname: String,
    status: String,
    #[serde(rename = "dnsTarget")]
    dns_target: Option<String>,
    error: Option<String>,
}

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let positional: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with('-') && *a != "domains")
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
        Some("add") => run_add(&creds, &project_id, positional.get(1).copied(), json_mode),
        Some("verify") => run_verify(&creds, &project_id, positional.get(1).copied(), json_mode),
        Some("rm") | Some("delete") => run_rm(
            &creds,
            &project_slug,
            positional.get(1).copied(),
            args,
            json_mode,
        ),
        Some(sub) => {
            output::print_error(&format!("unknown subcommand: \"{sub}\""));
            eprintln!("Usage: pylon domains [list | add <host> | verify <host> | rm <host>]");
            ExitCode::Usage
        }
    }
}

fn run_list(creds: &Credentials, project_id: &str, json_mode: bool) -> ExitCode {
    #[derive(serde::Serialize)]
    struct Args<'a> {
        #[serde(rename = "projectId")]
        project_id: &'a str,
    }
    let domains: Vec<Domain> =
        match post_json(creds, "/api/fn/listProjectDomains", &Args { project_id }) {
            Ok(d) => d,
            Err(e) => {
                output::print_error(&e);
                return ExitCode::Error;
            }
        };
    if json_mode {
        let out = serde_json::json!({"domains": domains.iter().map(|d| serde_json::json!({
				"id": d.id, "hostname": d.hostname, "status": d.status,
				"dnsTarget": d.dns_target, "error": d.error,
			})).collect::<Vec<_>>()});
        println!("{}", serde_json::to_string(&out).unwrap_or_default());
        return ExitCode::Ok;
    }
    if domains.is_empty() {
        println!("No custom domains. The default hostname is always available.");
        return ExitCode::Ok;
    }
    let w = domains
        .iter()
        .map(|d| d.hostname.len())
        .max()
        .unwrap_or(8)
        .max(8);
    println!("{:<w$}  STATUS    DNS TARGET", "HOSTNAME", w = w);
    for d in &domains {
        println!(
            "{:<w$}  {:<8}  {}",
            d.hostname,
            d.status,
            d.dns_target.as_deref().unwrap_or("-"),
            w = w
        );
        if let Some(e) = &d.error {
            if !e.is_empty() {
                println!("    ! {e}");
            }
        }
    }
    ExitCode::Ok
}

fn run_add(
    creds: &Credentials,
    project_id: &str,
    host_arg: Option<&str>,
    json_mode: bool,
) -> ExitCode {
    let Some(hostname) = host_arg else {
        output::print_error("Usage: pylon domains add <hostname>");
        return ExitCode::Usage;
    };
    #[derive(serde::Serialize)]
    struct Args<'a> {
        #[serde(rename = "projectId")]
        project_id: &'a str,
        hostname: &'a str,
    }
    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct Out {
        id: Option<String>,
        #[serde(rename = "dnsTarget")]
        dns_target: Option<String>,
    }
    let r: Out = match post_json(
        creds,
        "/api/fn/addProjectDomain",
        &Args {
            project_id,
            hostname,
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
                "ok": true,
                "hostname": hostname,
                "dnsTarget": r.dns_target,
            }))
            .unwrap_or_default()
        );
    } else {
        println!("✓ Added {hostname}");
        if let Some(t) = &r.dns_target {
            println!("  Point a CNAME to: {t}");
            println!("  Then: pylon domains verify {hostname}");
        }
    }
    ExitCode::Ok
}

fn run_verify(
    creds: &Credentials,
    project_id: &str,
    host_arg: Option<&str>,
    json_mode: bool,
) -> ExitCode {
    let Some(hostname) = host_arg else {
        output::print_error("Usage: pylon domains verify <hostname>");
        return ExitCode::Usage;
    };
    #[derive(serde::Serialize)]
    struct Args<'a> {
        #[serde(rename = "projectId")]
        project_id: &'a str,
        hostname: &'a str,
    }
    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct Out {
        ok: Option<bool>,
        status: Option<String>,
        error: Option<String>,
    }
    let r: Out = match post_json(
        creds,
        "/api/fn/verifyDomainDNS",
        &Args {
            project_id,
            hostname,
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
                "hostname": hostname, "status": r.status, "error": r.error,
            }))
            .unwrap_or_default()
        );
    } else {
        println!("  Status: {}", r.status.as_deref().unwrap_or("?"),);
        if let Some(e) = &r.error {
            if !e.is_empty() {
                println!("  Error: {e}");
            }
        }
    }
    ExitCode::Ok
}

fn run_rm(
    creds: &Credentials,
    project_slug: &str,
    host_arg: Option<&str>,
    args: &[String],
    json_mode: bool,
) -> ExitCode {
    let Some(hostname) = host_arg else {
        output::print_error("Usage: pylon domains rm <hostname> [--yes]");
        return ExitCode::Usage;
    };
    if !output::confirm_destructive(args, &format!("remove domain {hostname}"), json_mode) {
        return ExitCode::Usage;
    }
    #[derive(serde::Serialize)]
    struct Args<'a> {
        #[serde(rename = "projectSlug")]
        project_slug: &'a str,
        hostname: &'a str,
    }
    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct Out {
        ok: Option<bool>,
    }
    if let Err(e) = post_json::<_, Out>(
        creds,
        "/api/fn/deleteProjectDomainForCli",
        &Args {
            project_slug,
            hostname,
        },
    ) {
        output::print_error(&e);
        return ExitCode::Error;
    }
    if json_mode {
        println!("{}", serde_json::json!({"ok": true, "hostname": hostname}));
    } else {
        println!("✓ Removed {hostname}");
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
