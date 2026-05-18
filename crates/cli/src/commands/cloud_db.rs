//! `pylon db` — backup / restore / url.

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
struct Backup {
    id: String,
    #[serde(rename = "flySnapshotId")]
    fly_snapshot_id: String,
    status: String,
    #[serde(rename = "createdAt")]
    created_at: String,
}

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let positional: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with('-') && *a != "db")
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
        Some("backup") => run_backup(&creds, &project_id, json_mode),
        Some("list") | Some("backups") | None => run_list(&creds, &project_id, json_mode),
        Some("restore") => run_restore(&creds, &project_id, positional.get(1).copied(), json_mode),
        Some(sub) => {
            output::print_error(&format!("unknown subcommand: \"{sub}\""));
            eprintln!("Usage: pylon db [list | backup | restore <id>]");
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
    let backups: Vec<Backup> =
        match post_json(creds, "/api/fn/listProjectBackups", &Args { project_id }) {
            Ok(b) => b,
            Err(e) => {
                output::print_error(&e);
                return ExitCode::Error;
            }
        };
    if json_mode {
        let out = serde_json::json!({"backups": backups.iter().map(|b| serde_json::json!({
			"id": b.id, "flySnapshotId": b.fly_snapshot_id, "status": b.status, "createdAt": b.created_at,
		})).collect::<Vec<_>>()});
        println!("{}", serde_json::to_string(&out).unwrap_or_default());
        return ExitCode::Ok;
    }
    if backups.is_empty() {
        println!("No backups yet. Run `pylon db backup` to take one.");
        return ExitCode::Ok;
    }
    println!("ID                                STATUS    CREATED");
    for b in &backups {
        println!(
            "{:32}  {:<8}  {}",
            b.id,
            b.status,
            b.created_at.split('T').next().unwrap_or(&b.created_at)
        );
    }
    ExitCode::Ok
}

fn run_backup(creds: &Credentials, project_id: &str, json_mode: bool) -> ExitCode {
    #[derive(serde::Serialize)]
    struct Args<'a> {
        #[serde(rename = "projectId")]
        project_id: &'a str,
    }
    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct Out {
        id: Option<String>,
        #[serde(rename = "flySnapshotId")]
        fly_snapshot_id: Option<String>,
    }
    let r: Out = match post_json(creds, "/api/fn/backupProject", &Args { project_id }) {
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
                "ok": true, "id": r.id, "flySnapshotId": r.fly_snapshot_id,
            }))
            .unwrap_or_default()
        );
    } else {
        println!("✓ Backup queued");
        if let Some(id) = &r.id {
            println!("  Backup ID: {id}");
        }
    }
    ExitCode::Ok
}

fn run_restore(
    creds: &Credentials,
    project_id: &str,
    backup_id: Option<&str>,
    json_mode: bool,
) -> ExitCode {
    let Some(id) = backup_id else {
        output::print_error("Usage: pylon db restore <backup-id>");
        eprintln!("  Find ids with: pylon db list");
        return ExitCode::Usage;
    };
    #[derive(serde::Serialize)]
    struct Args<'a> {
        #[serde(rename = "projectId")]
        project_id: &'a str,
        #[serde(rename = "backupId")]
        backup_id: &'a str,
    }
    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct Out {
        ok: Option<bool>,
    }
    if let Err(e) = post_json::<_, Out>(
        creds,
        "/api/fn/restoreProject",
        &Args {
            project_id,
            backup_id: id,
        },
    ) {
        output::print_error(&e);
        return ExitCode::Error;
    }
    if json_mode {
        println!("{}", serde_json::json!({"ok": true, "backupId": id}));
    } else {
        println!("✓ Restore queued for backup {id}");
        println!("  Watch the project's Deployments page for progress.");
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
