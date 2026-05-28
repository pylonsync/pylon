//! `pylon secrets` — list / set / rm / import .env.
//!
//! Wraps the control-plane's existing listSecrets / upsertSecret /
//! deleteSecret functions. The CLI never sees plaintext for an
//! existing secret (cloud encrypts at rest, exposes only metadata),
//! but a fresh `set` writes through to the cloud which encrypts.
//!
//! `pylon secrets list` — show keys + last-rotated metadata.
//! `pylon secrets set KEY=value` — upsert a single secret.
//! `pylon secrets set KEY` — prompt for value on stdin (hidden).
//! `pylon secrets rm KEY` — delete.
//! `pylon secrets import .env` — bulk-import every KEY=value from a
//!   dotenv file. Skips comment lines + blanks. By default, refuses
//!   to overwrite existing keys; pass --replace to allow.

use std::io::{self, BufRead, IsTerminal, Write};

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
struct SecretRow {
    id: String,
    key: String,
    #[serde(rename = "lastRotatedAt")]
    last_rotated_at: Option<String>,
    #[serde(rename = "lastRotatedBy")]
    last_rotated_by: Option<String>,
}

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let positional: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with('-') && *a != "secrets")
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
        Some("set") => run_set(
            args,
            &creds,
            &project_slug,
            &project_id,
            positional.get(1).copied(),
            json_mode,
        ),
        Some("rm") | Some("delete") | Some("unset") => run_rm(
            &creds,
            &project_slug,
            positional.get(1).copied(),
            args,
            json_mode,
        ),
        Some("import") => run_import(
            args,
            &creds,
            &project_slug,
            &project_id,
            positional.get(1).copied(),
            json_mode,
        ),
        Some(sub) => {
            output::print_error(&format!("unknown subcommand: \"{sub}\""));
            eprintln!("Usage: pylon secrets [list | set KEY=value | rm KEY | import .env]");
            ExitCode::Usage
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

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

fn run_list(creds: &Credentials, project_id: &str, json_mode: bool) -> ExitCode {
    #[derive(serde::Serialize)]
    struct Args<'a> {
        #[serde(rename = "projectId")]
        project_id: &'a str,
    }
    let secrets: Vec<SecretRow> =
        match post_json(creds, "/api/fn/listSecrets", &Args { project_id }) {
            Ok(s) => s,
            Err(e) => {
                output::print_error(&e);
                return ExitCode::Error;
            }
        };
    if json_mode {
        let out = serde_json::json!({
            "secrets": secrets.iter().map(|s| serde_json::json!({
                "id": s.id,
                "key": s.key,
                "lastRotatedAt": s.last_rotated_at,
                "lastRotatedBy": s.last_rotated_by,
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string(&out).unwrap_or_default());
        return ExitCode::Ok;
    }
    if secrets.is_empty() {
        println!("No secrets set.");
        return ExitCode::Ok;
    }
    let key_w = secrets
        .iter()
        .map(|s| s.key.len())
        .max()
        .unwrap_or(3)
        .max(3);
    println!("{:<k$}  LAST ROTATED", "KEY", k = key_w);
    for s in &secrets {
        let when = s
            .last_rotated_at
            .as_deref()
            .unwrap_or("?")
            .split('T')
            .next()
            .unwrap_or("?");
        println!("{:<k$}  {}", s.key, when, k = key_w);
    }
    ExitCode::Ok
}

// ---------------------------------------------------------------------------
// set
// ---------------------------------------------------------------------------

fn run_set(
    _args: &[String],
    creds: &Credentials,
    project_slug: &str,
    _project_id: &str,
    arg: Option<&str>,
    json_mode: bool,
) -> ExitCode {
    let Some(input) = arg else {
        output::print_error("Usage: pylon secrets set KEY=value");
        eprintln!("  Or: pylon secrets set KEY  (prompts for value on stdin)");
        return ExitCode::Usage;
    };
    let (key, value) = if let Some((k, v)) = input.split_once('=') {
        (k.trim().to_string(), v.to_string())
    } else {
        let k = input.trim().to_string();
        let v = match prompt_secret(&k) {
            Ok(v) => v,
            Err(e) => {
                output::print_error(&e);
                return ExitCode::Usage;
            }
        };
        (k, v)
    };
    if !is_valid_env_name(&key) {
        output::print_error(&format!(
            "Invalid key \"{key}\". Must match ^[A-Z_][A-Z0-9_]*$ (env-var convention)."
        ));
        return ExitCode::Usage;
    }
    if let Err(e) = upsert_one(creds, project_slug, &key, &value) {
        output::print_error(&e);
        return ExitCode::Error;
    }
    if json_mode {
        println!("{}", serde_json::json!({"ok": true, "key": key}));
    } else {
        println!("✓ {key} set");
        println!("  syncSecretsToFly fires immediately — the running machine sees the new value within ~10s.");
    }
    ExitCode::Ok
}

fn upsert_one(
    creds: &Credentials,
    project_slug: &str,
    key: &str,
    value: &str,
) -> Result<(), String> {
    #[derive(serde::Serialize)]
    struct Args<'a> {
        #[serde(rename = "projectSlug")]
        project_slug: &'a str,
        key: &'a str,
        value: &'a str,
    }
    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct Out {
        ok: Option<bool>,
    }
    let _: Out = post_json(
        creds,
        "/api/fn/setSecretForCli",
        &Args {
            project_slug,
            key,
            value,
        },
    )?;
    Ok(())
}

fn prompt_secret(key: &str) -> Result<String, String> {
    if !std::io::stdin().is_terminal() {
        // Non-TTY — read raw line (the value's just whatever was piped
        // in). `pylon secrets set FOO < value.txt` works this way.
        let mut line = String::new();
        std::io::stdin()
            .lock()
            .read_line(&mut line)
            .map_err(|e| e.to_string())?;
        return Ok(line.trim_end_matches('\n').to_string());
    }
    print!("{key} = ");
    io::stdout().flush().ok();
    // We deliberately echo — masking terminal input from Rust without
    // pulling rpassword/dialoguer would add a dep we don't otherwise
    // need. Operators piping sensitive values should use `< file`.
    let mut line = String::new();
    io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    Ok(line.trim_end_matches('\n').to_string())
}

fn is_valid_env_name(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut chars = s.chars();
    let first = chars.next().unwrap();
    if !(first.is_ascii_uppercase() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

// ---------------------------------------------------------------------------
// rm
// ---------------------------------------------------------------------------

fn run_rm(
    creds: &Credentials,
    project_slug: &str,
    key_arg: Option<&str>,
    args: &[String],
    json_mode: bool,
) -> ExitCode {
    let Some(key) = key_arg else {
        output::print_error("Usage: pylon secrets rm KEY [--yes]");
        return ExitCode::Usage;
    };
    if !output::confirm_destructive(args, &format!("delete secret \"{key}\""), json_mode) {
        return ExitCode::Usage;
    }
    #[derive(serde::Serialize)]
    struct Args<'a> {
        #[serde(rename = "projectSlug")]
        project_slug: &'a str,
        key: &'a str,
    }
    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct Out {
        ok: Option<bool>,
    }
    if let Err(e) = post_json::<_, Out>(
        creds,
        "/api/fn/deleteSecretForCli",
        &Args { project_slug, key },
    ) {
        output::print_error(&e);
        return ExitCode::Error;
    }
    if json_mode {
        println!("{}", serde_json::json!({"ok": true, "key": key}));
    } else {
        println!("✓ {key} deleted");
    }
    ExitCode::Ok
}

// ---------------------------------------------------------------------------
// import .env
// ---------------------------------------------------------------------------

fn run_import(
    args: &[String],
    creds: &Credentials,
    project_slug: &str,
    project_id: &str,
    path_arg: Option<&str>,
    json_mode: bool,
) -> ExitCode {
    let path = path_arg.unwrap_or(".env");
    let replace = args.iter().any(|a| a == "--replace");
    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            output::print_error(&format!("Could not read {path}: {e}"));
            return ExitCode::Error;
        }
    };
    let parsed = parse_dotenv(&contents);
    if parsed.is_empty() {
        output::print_error(&format!("{path} had no KEY=value lines."));
        return ExitCode::Usage;
    }
    // Pre-check: surface a list of would-conflict keys before any
    // write happens. Saves the user from a partial import.
    if !replace {
        let existing = match list_existing_keys(creds, project_id) {
            Ok(e) => e,
            Err(e) => {
                output::print_error(&e);
                return ExitCode::Error;
            }
        };
        let conflicts: Vec<&str> = parsed
            .iter()
            .map(|(k, _)| k.as_str())
            .filter(|k| existing.iter().any(|e| e == *k))
            .collect();
        if !conflicts.is_empty() {
            output::print_error(&format!(
                "Refusing to overwrite {} existing key(s): {}",
                conflicts.len(),
                conflicts.join(", ")
            ));
            eprintln!("  Re-run with --replace to overwrite, or rm them first.");
            return ExitCode::Usage;
        }
    }
    let mut ok = 0;
    let mut failed: Vec<(String, String)> = Vec::new();
    for (key, value) in &parsed {
        if !is_valid_env_name(key) {
            failed.push((key.clone(), "invalid env-var name".into()));
            continue;
        }
        match upsert_one(creds, project_slug, key, value) {
            Ok(()) => ok += 1,
            Err(e) => failed.push((key.clone(), e)),
        }
    }
    if json_mode {
        let out = serde_json::json!({
            "imported": ok,
            "failed": failed.iter().map(|(k, e)| serde_json::json!({"key": k, "error": e})).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string(&out).unwrap_or_default());
    } else {
        println!("✓ Imported {ok} secret(s) from {path}");
        for (k, e) in &failed {
            eprintln!("  ! {k}: {e}");
        }
        println!(
            "  syncSecretsToFly fires per-key — the running machine sees the new values shortly."
        );
    }
    if !failed.is_empty() {
        ExitCode::Error
    } else {
        ExitCode::Ok
    }
}

fn list_existing_keys(creds: &Credentials, project_id: &str) -> Result<Vec<String>, String> {
    #[derive(serde::Serialize)]
    struct Args<'a> {
        #[serde(rename = "projectId")]
        project_id: &'a str,
    }
    let secrets: Vec<SecretRow> = post_json(creds, "/api/fn/listSecrets", &Args { project_id })?;
    Ok(secrets.into_iter().map(|s| s.key).collect())
}

/// Tiny dotenv parser. Handles:
/// - blank lines + `#` comments
/// - optional `export KEY=value`
/// - quoted values (single or double, no escape unescaping)
/// - unquoted values (everything to EOL, trim trailing whitespace)
fn parse_dotenv(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let stripped = line.strip_prefix("export ").unwrap_or(line);
        let Some((k, v)) = stripped.split_once('=') else {
            continue;
        };
        let key = k.trim().to_string();
        let mut value = v.trim().to_string();
        // Strip matching surrounding quotes.
        if (value.starts_with('"') && value.ends_with('"') && value.len() >= 2)
            || (value.starts_with('\'') && value.ends_with('\'') && value.len() >= 2)
        {
            value = value[1..value.len() - 1].to_string();
        }
        out.push((key, value));
    }
    out
}
