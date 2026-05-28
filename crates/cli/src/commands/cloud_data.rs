//! `pylon data` — entity rows from the shell.
//!
//! `pylon data entities` — list every entity the customer's app declared.
//! `pylon data list <Entity>` — first 50 rows.
//! `pylon data get <Entity> <id>` — single row by id.

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
struct EntityField {
    name: String,
    #[serde(rename = "type")]
    ty: String,
    #[serde(default)]
    optional: bool,
}

#[derive(Deserialize)]
struct Entity {
    name: String,
    #[serde(default)]
    fields: Vec<EntityField>,
}

#[derive(Deserialize)]
#[serde(tag = "kind")]
enum EntitiesResp {
    #[serde(rename = "ok")]
    Ok { entities: Vec<Entity> },
    #[serde(rename = "unavailable")]
    Unavailable { reason: String },
}

#[derive(Deserialize)]
#[serde(tag = "kind")]
enum RowsResp {
    #[serde(rename = "ok")]
    Ok {
        rows: Vec<serde_json::Value>,
        #[serde(rename = "has_more")]
        #[allow(dead_code)]
        has_more: Option<bool>,
    },
    #[serde(rename = "unavailable")]
    Unavailable { reason: String },
}

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let positional: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with('-') && *a != "data")
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
        Some("entities") | None => run_entities(&creds, &project_id, json_mode),
        Some("list") => {
            let limit = parse_limit_flag(args).unwrap_or(50).clamp(1, 1000);
            run_list(
                &creds,
                &project_id,
                positional.get(1).copied(),
                limit,
                json_mode,
            )
        }
        Some("get") => run_get(
            &creds,
            &project_id,
            positional.get(1).copied(),
            positional.get(2).copied(),
            json_mode,
        ),
        Some(sub) => {
            output::print_error(&format!("unknown subcommand: \"{sub}\""));
            eprintln!("Usage: pylon data [entities | list <Entity> | get <Entity> <id>]");
            ExitCode::Usage
        }
    }
}

fn run_entities(creds: &Credentials, project_id: &str, json_mode: bool) -> ExitCode {
    #[derive(serde::Serialize)]
    struct Args<'a> {
        #[serde(rename = "projectId")]
        project_id: &'a str,
    }
    let resp: EntitiesResp =
        match post_json(creds, "/api/fn/listProjectEntities", &Args { project_id }) {
            Ok(r) => r,
            Err(e) => {
                output::print_error(&e);
                return ExitCode::Error;
            }
        };
    let entities = match resp {
        EntitiesResp::Ok { entities } => entities,
        EntitiesResp::Unavailable { reason } => {
            output::print_error(&format!("Entities unavailable: {reason}"));
            return ExitCode::Error;
        }
    };
    if json_mode {
        let out = serde_json::json!({"entities": entities.iter().map(|e| serde_json::json!({
			"name": e.name,
			"fields": e.fields.iter().map(|f| serde_json::json!({
				"name": f.name, "type": f.ty, "optional": f.optional,
			})).collect::<Vec<_>>(),
		})).collect::<Vec<_>>()});
        println!("{}", serde_json::to_string(&out).unwrap_or_default());
        return ExitCode::Ok;
    }
    for e in &entities {
        println!("{}  ({} fields)", e.name, e.fields.len());
    }
    ExitCode::Ok
}

/// Parse `--limit N` or `--limit=N` from the raw argv slice. None
/// when the flag is absent OR the value doesn't parse — caller picks
/// the default. Lives here (instead of project_context) because every
/// data-shaped subcommand wants the same shape.
fn parse_limit_flag(args: &[String]) -> Option<u32> {
    args.windows(2)
        .find(|w| w[0] == "--limit")
        .and_then(|w| w[1].parse().ok())
        .or_else(|| {
            args.iter()
                .find(|a| a.starts_with("--limit="))
                .and_then(|a| a.trim_start_matches("--limit=").parse().ok())
        })
}

fn run_list(
    creds: &Credentials,
    project_id: &str,
    entity: Option<&str>,
    limit: u32,
    json_mode: bool,
) -> ExitCode {
    let Some(entity) = entity else {
        output::print_error("Usage: pylon data list <Entity> [--limit N]");
        return ExitCode::Usage;
    };
    #[derive(serde::Serialize)]
    struct Args<'a> {
        #[serde(rename = "projectId")]
        project_id: &'a str,
        entity: &'a str,
        limit: u32,
    }
    let resp: RowsResp = match post_json(
        creds,
        "/api/fn/listProjectEntityRows",
        &Args {
            project_id,
            entity,
            limit,
        },
    ) {
        Ok(r) => r,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Error;
        }
    };
    let rows = match resp {
        RowsResp::Ok { rows, .. } => rows,
        RowsResp::Unavailable { reason } => {
            output::print_error(&format!("Rows unavailable: {reason}"));
            return ExitCode::Error;
        }
    };
    if json_mode {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({"rows": rows})).unwrap_or_default()
        );
        return ExitCode::Ok;
    }
    for row in &rows {
        println!("{}", row);
    }
    ExitCode::Ok
}

fn run_get(
    creds: &Credentials,
    project_id: &str,
    entity: Option<&str>,
    id: Option<&str>,
    json_mode: bool,
) -> ExitCode {
    let Some(entity) = entity else {
        output::print_error("Usage: pylon data get <Entity> <id>");
        return ExitCode::Usage;
    };
    let Some(id) = id else {
        output::print_error("Usage: pylon data get <Entity> <id>");
        return ExitCode::Usage;
    };
    // V1: paginate listProjectEntityRows (capped at 200/page) and grep
    // for the id client-side. Works fine for small entities; large
    // entities need a dedicated /admin/entities/<E>/<id> endpoint on
    // the customer machine which we'd add as a framework feature.
    #[derive(serde::Serialize)]
    struct Args<'a> {
        #[serde(rename = "projectId")]
        project_id: &'a str,
        entity: &'a str,
        limit: u32,
    }
    let resp: RowsResp = match post_json(
        creds,
        "/api/fn/listProjectEntityRows",
        &Args {
            project_id,
            entity,
            limit: 200,
        },
    ) {
        Ok(r) => r,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Error;
        }
    };
    let rows = match resp {
        RowsResp::Ok { rows, .. } => rows,
        RowsResp::Unavailable { reason } => {
            output::print_error(&format!("Rows unavailable: {reason}"));
            return ExitCode::Error;
        }
    };
    let row = rows
        .iter()
        .find(|r| r.get("id").and_then(|v| v.as_str()) == Some(id));
    match row {
        Some(row) => {
            if json_mode {
                println!("{}", serde_json::to_string(row).unwrap_or_default());
            } else {
                println!("{}", serde_json::to_string_pretty(row).unwrap_or_default());
            }
            ExitCode::Ok
        }
        None => {
            output::print_error(&format!(
				"No {entity} with id \"{id}\" in the first 200 rows. Get-by-id beyond that page isn't wired yet."
			));
            ExitCode::Error
        }
    }
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
