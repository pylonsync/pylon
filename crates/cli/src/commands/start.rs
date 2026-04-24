//! `pylon start` — production server.
//!
//! The same underlying server as `pylon dev`, but without the file watcher
//! and with prod-shaped defaults:
//!
//! - No `.pylon/` auto-data-dir under the source tree — operators must set
//!   `PYLON_DB_PATH` (or accept the `pylon.db` default in CWD).
//! - No generous dev rate-limit bumps — the operator's `PYLON_RATE_LIMIT_*`
//!   values (or the server defaults) apply.
//! - No auto-migrate of a hidden dev database. The runtime opens whatever
//!   database path the operator supplied; it still applies schema diffs
//!   on startup, but against the configured target.
//! - Blocks on the server thread instead of backgrounding + polling.
//!
//! `PYLON_DEV_MODE=true` still flips the server into dev-behavior
//! (wildcard CORS, magic codes echoed in responses) — that env var
//! controls *server behavior*, this command controls *lifecycle*.

use std::path::Path;
use std::sync::Arc;

use pylon_kernel::{Diagnostic, ExitCode, Severity};

use crate::manifest::{parse_manifest, validate_all};
use crate::output::{print_diagnostics, print_json};

const DEFAULT_PORT: u16 = 4321;

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let port: u16 = args
        .windows(2)
        .find(|w| w[0] == "--port")
        .and_then(|w| w[1].parse().ok())
        .unwrap_or(DEFAULT_PORT);

    let positional: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with('-') && *a != "start")
        .map(|s| s.as_str())
        .collect();

    let entry_file = match positional.first() {
        Some(f) => f.to_string(),
        None => {
            if Path::new("app.ts").exists() {
                "app.ts".to_string()
            } else {
                print_diagnostics(
                    &[Diagnostic {
                        severity: Severity::Error,
                        code: "START_NO_ENTRY".into(),
                        message: "No entry file provided and no app.ts found in current directory"
                            .into(),
                        span: None,
                        hint: Some("Usage: pylon start [app.ts] [--port 4321]".into()),
                    }],
                    json_mode,
                );
                return ExitCode::Usage;
            }
        }
    };

    if !Path::new(&entry_file).exists() {
        print_diagnostics(
            &[Diagnostic {
                severity: Severity::Error,
                code: "START_ENTRY_NOT_FOUND".into(),
                message: format!("Entry file not found: {entry_file}"),
                span: None,
                hint: None,
            }],
            json_mode,
        );
        return ExitCode::Error;
    }

    // Build the manifest once. Production never re-reads app.ts.
    let manifest_json = match crate::bun::run_bun_codegen(&entry_file) {
        Ok(json) => json,
        Err(diag) => {
            print_diagnostics(&[diag], json_mode);
            return ExitCode::Error;
        }
    };

    let manifest = match parse_manifest(&manifest_json, &entry_file) {
        Ok(m) => m,
        Err(diags) => {
            print_diagnostics(&diags, json_mode);
            return ExitCode::Error;
        }
    };

    let diagnostics = validate_all(&manifest);
    if diagnostics.iter().any(|d| d.severity == Severity::Error) {
        print_diagnostics(&diagnostics, json_mode);
        return ExitCode::Error;
    }

    // Database path. Operator-driven in prod — default falls back to the
    // dev path so local `pylon start` against an existing `pylon dev`
    // workspace "just works," but the Docker image and any real deploy
    // should set PYLON_DB_PATH explicitly.
    let db_path = std::env::var("PYLON_DB_PATH").unwrap_or_else(|_| ".pylon/dev.db".to_string());
    if let Some(parent) = Path::new(&db_path).parent() {
        if !parent.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(parent);
        }
    }

    // Apply schema diff. In prod we only expand-compatible; destructive
    // migrations are the operator's explicit responsibility via
    // `pylon migrate`.
    if let Ok(adapter) = pylon_storage::sqlite::SqliteAdapter::open(&db_path) {
        if let Ok(plan) = adapter.plan_from_live(&manifest) {
            let meta = pylon_storage::sqlite::PushMetadata {
                manifest_version: manifest.manifest_version,
                app_version: &manifest.version,
                baseline: "start",
            };
            let _ = adapter.apply_with_history(&plan, &meta);
        }
    }

    let runtime = match pylon_runtime::Runtime::open(&db_path, manifest.clone()) {
        Ok(rt) => Arc::new(rt),
        Err(e) => {
            if !json_mode {
                eprintln!("[start] Failed to open runtime: {e}");
            }
            return ExitCode::Error;
        }
    };

    if json_mode {
        print_json(&serde_json::json!({
            "code": "START_OK",
            "name": manifest.name,
            "version": manifest.version,
            "entry": entry_file,
            "port": port,
            "db_path": db_path,
        }));
    } else {
        println!("pylon start");
        println!("  App:      {} v{}", manifest.name, manifest.version);
        println!("  Server:   http://0.0.0.0:{port}");
        println!("  Database: {db_path}");
        println!();
    }

    // Block on the server. Any error is fatal — we want it to bubble up to
    // the process supervisor (systemd, Fly's init, Docker) so the container
    // gets restarted instead of wedging in a "looks alive" state.
    if let Err(e) = pylon_runtime::server::start(runtime, port) {
        if !json_mode {
            eprintln!("[start] server failed to start: {e}");
        }
        return ExitCode::Error;
    }

    ExitCode::Ok
}
