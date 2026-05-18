//! `pylon members` — list / invite / role within an organization.
//!
//! Org context: derived from the current project's orgId (resolved
//! via `getProjectForCli`). Pass `--project <slug>` to switch.

use pylon_kernel::ExitCode;
use serde::Deserialize;

use crate::cloud_client::{post_json, require_credentials, Credentials};
use crate::output;
use crate::project_context::resolve_project_slug;

#[derive(Deserialize)]
struct ProjectForCli {
    #[allow(dead_code)] // Deserialized for forward compat but only org_id is used here.
    id: String,
    #[serde(rename = "orgId")]
    org_id: String,
}

#[derive(Deserialize)]
struct Member {
    #[serde(rename = "userId")]
    user_id: String,
    role: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default, rename = "displayName")]
    display_name: Option<String>,
}

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let positional: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with('-') && *a != "members")
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
    let project = match resolve_project(&creds, &project_slug) {
        Ok(p) => p,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Error;
        }
    };

    match positional.first().copied() {
        Some("list") | None => run_list(&creds, &project.org_id, json_mode),
        Some("invite") => run_invite(
            &creds,
            &project.org_id,
            positional.get(1).copied(),
            positional.get(2).copied(),
            json_mode,
        ),
        Some(sub) => {
            output::print_error(&format!("unknown subcommand: \"{sub}\""));
            eprintln!("Usage: pylon members [list | invite <email> [role]]");
            ExitCode::Usage
        }
    }
}

fn run_list(creds: &Credentials, org_id: &str, json_mode: bool) -> ExitCode {
    #[derive(serde::Serialize)]
    struct Args<'a> {
        #[serde(rename = "orgId")]
        org_id: &'a str,
    }
    let members: Vec<Member> = match post_json(creds, "/api/fn/listOrgMembers", &Args { org_id }) {
        Ok(m) => m,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Error;
        }
    };
    if json_mode {
        let out = serde_json::json!({"members": members.iter().map(|m| serde_json::json!({
			"userId": m.user_id, "role": m.role, "email": m.email, "displayName": m.display_name,
		})).collect::<Vec<_>>()});
        println!("{}", serde_json::to_string(&out).unwrap_or_default());
        return ExitCode::Ok;
    }
    if members.is_empty() {
        println!("No members in this org.");
        return ExitCode::Ok;
    }
    let email_w = members
        .iter()
        .map(|m| m.email.as_deref().unwrap_or("?").len())
        .max()
        .unwrap_or(5)
        .max(5);
    println!("{:<e$}  ROLE     NAME", "EMAIL", e = email_w);
    for m in &members {
        println!(
            "{:<e$}  {:<7}  {}",
            m.email.as_deref().unwrap_or("?"),
            m.role,
            m.display_name.as_deref().unwrap_or(""),
            e = email_w
        );
    }
    ExitCode::Ok
}

fn run_invite(
    creds: &Credentials,
    org_id: &str,
    email: Option<&str>,
    role_arg: Option<&str>,
    json_mode: bool,
) -> ExitCode {
    let Some(email) = email else {
        output::print_error("Usage: pylon members invite <email> [owner|admin|member]");
        return ExitCode::Usage;
    };
    let role = role_arg.unwrap_or("member");
    #[derive(serde::Serialize)]
    struct Args<'a> {
        #[serde(rename = "orgId")]
        org_id: &'a str,
        email: &'a str,
        role: &'a str,
    }
    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct Out {
        #[serde(rename = "inviteUrl")]
        invite_url: Option<String>,
        email: Option<String>,
    }
    let r: Out = match post_json(
        creds,
        "/api/fn/inviteMember",
        &Args {
            org_id,
            email,
            role,
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
                "ok": true, "email": email, "role": role, "inviteUrl": r.invite_url,
            }))
            .unwrap_or_default()
        );
    } else {
        println!("✓ Invited {email} as {role}");
        if let Some(url) = &r.invite_url {
            println!("  Invite URL: {url}");
        }
    }
    ExitCode::Ok
}

fn resolve_project(creds: &Credentials, slug: &str) -> Result<ProjectForCli, String> {
    #[derive(serde::Serialize)]
    struct Args<'a> {
        slug: &'a str,
    }
    post_json(creds, "/api/fn/getProjectForCli", &Args { slug })
        .map_err(|e| format!("Could not resolve project \"{slug}\": {e}"))
}
