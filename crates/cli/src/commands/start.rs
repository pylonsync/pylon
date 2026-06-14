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

use crate::commands::args::collect_positional;
use crate::manifest::{parse_manifest, validate_all};
use crate::output::{print_diagnostics, print_json};
use crate::studio_config;

const DEFAULT_PORT: u16 = 4321;

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    // Prebuilt-artifact boot (Pylon Cloud build pipeline). When the control
    // plane ships a prebuilt bundle instead of inlining source, fetch + verify
    // + extract it and chdir into it BEFORE anything reads app.ts — on a fresh
    // machine the app root doesn't exist until this runs. No-op (returns None)
    // unless PYLON_ARTIFACT_ID is set, so the legacy inline-files boot is
    // unchanged. See crate::artifact.
    let artifact_active = match crate::artifact::prepare() {
        Ok(Some(app_root)) => {
            if let Err(e) = std::env::set_current_dir(&app_root) {
                print_diagnostics(
                    &[Diagnostic {
                        severity: Severity::Error,
                        code: "START_ARTIFACT_CHDIR".into(),
                        message: format!(
                            "could not enter extracted artifact dir {}: {e}",
                            app_root.display()
                        ),
                        span: None,
                        hint: None,
                    }],
                    json_mode,
                );
                return ExitCode::Error;
            }
            true
        }
        Ok(None) => false,
        Err(e) => {
            print_diagnostics(
                &[Diagnostic {
                    severity: Severity::Error,
                    code: "START_ARTIFACT_FETCH".into(),
                    message: format!("prebuilt artifact bootstrap failed: {e}"),
                    span: None,
                    hint: Some(
                        "the deploy's bundle could not be fetched and no cached bundle exists"
                            .into(),
                    ),
                }],
                json_mode,
            );
            return ExitCode::Error;
        }
    };

    let port: u16 = args
        .windows(2)
        .find(|w| w[0] == "--port" || w[0] == "-p")
        .and_then(|w| w[1].parse().ok())
        .unwrap_or(DEFAULT_PORT);

    let positional: Vec<&str> = collect_positional(args, "start");

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
    let manifest_json = match crate::bun::run_bun_codegen(&entry_file, true) {
        Ok(json) => json,
        Err(diag) => {
            print_diagnostics(&[diag], json_mode);
            return ExitCode::Error;
        }
    };

    // Build the frontend SPA in the BACKGROUND if the project ships
    // one (web/ with a build script). On Pylon Cloud / Fly this is
    // what materializes web/dist/ on the machine — the deploy ships
    // source but never dist.
    //
    // Why background: cold-boot `bun install + bun run build` for a
    // React/Vite project takes 20-60s. Fly's healthcheck (Dockerfile
    // start_period=15s + 3*30s retries = 105s total) is generous but
    // not guaranteed to cover us, and even when it does the autostop
    // model relies on hitting /health within seconds of a wakeup.
    // Blocking startup on the build means /health 503s, the load
    // balancer marks the machine unhealthy, and the deploy crashloops.
    //
    // Async pattern: the runtime's frontend module exposes a build-
    // state handle. We start the build now, the HTTP server starts
    // immediately, /health responds in ms. Non-API GETs serve a
    // "building" shell until dist/index.html appears, then flip over
    // to the real SPA. The fast path (warm boot with dist/.pylon-build-
    // marker present) returns Ok within ~5ms anyway and the user
    // sees the SPA from the first request.
    let entry_file_clone = entry_file.clone();
    let build_state = pylon_runtime::frontend::shared_build_state();
    pylon_runtime::frontend::mark_build_in_progress(&build_state);
    std::thread::Builder::new()
        .name("pylon-frontend-build".into())
        .spawn({
            let build_state = build_state.clone();
            move || {
                // Wrap the entire build in catch_unwind so a panic
                // anywhere in ensure_frontend_built (FS quirk on
                // /data, bun.lock metadata race, std::fs::* unwrap,
                // etc.) flips the BuildState to Failed instead of
                // leaving InProgress forever. Without this guard,
                // a thread panic terminates the worker silently and
                // the operator sees an infinite "Building..." page
                // with no way to know what went wrong.
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    crate::bun::ensure_frontend_built(&entry_file_clone, true)
                }));
                match result {
                    Ok(Ok(())) => {
                        pylon_runtime::frontend::mark_build_ready(&build_state);
                    }
                    Ok(Err(diag)) => {
                        eprintln!("[frontend] build failed ({}): {}", diag.code, diag.message);
                        pylon_runtime::frontend::mark_build_failed(&build_state, diag.message);
                    }
                    Err(panic) => {
                        let msg = if let Some(s) = panic.downcast_ref::<String>() {
                            s.clone()
                        } else if let Some(s) = panic.downcast_ref::<&str>() {
                            (*s).to_string()
                        } else {
                            "build thread panicked with non-string payload".to_string()
                        };
                        eprintln!("[frontend] build panicked: {msg}");
                        pylon_runtime::frontend::mark_build_failed(
                            &build_state,
                            format!("build thread panicked: {msg}"),
                        );
                    }
                }
            }
        })
        .expect("spawn frontend build thread");

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

    // Build studio artefacts (config JSON + extension bundle if present).
    // Failures are non-fatal — the runtime falls back to defaults so a
    // bad studio.config.ts can't take down a production server.
    let entry_dir = Path::new(&entry_file).parent().unwrap_or(Path::new("."));
    let studio_data_dir = entry_dir.join(".pylon");
    if studio_config::locate_config(&entry_file).is_some()
        || studio_config::locate_entry(&entry_file).is_some()
    {
        if let Err(diags) = studio_config::build_artefacts(&entry_file, &studio_data_dir) {
            let warnings: Vec<Diagnostic> = diags
                .into_iter()
                .map(|mut d| {
                    d.severity = Severity::Warning;
                    d
                })
                .collect();
            print_diagnostics(&warnings, json_mode);
        }
    }

    // Database target — Postgres takes precedence when DATABASE_URL is set
    // so multi-replica deployments (pylon-cloud, k8s, Fly clusters) only
    // need that one env var. Falls back to PYLON_DB_PATH (SQLite path),
    // then `.pylon/dev.db` so a local `pylon start` against an existing
    // `pylon dev` workspace still "just works."
    let db_target = std::env::var("DATABASE_URL")
        .ok()
        .or_else(|| std::env::var("PYLON_DB_PATH").ok())
        .unwrap_or_else(|| ".pylon/dev.db".to_string());
    let is_pg = db_target.starts_with("postgres://") || db_target.starts_with("postgresql://");

    // When booting from a prebuilt artifact the cwd is an ephemeral, PRUNABLE
    // per-deploy dir on /data. A cwd-relative SQLite default (`.pylon/dev.db`)
    // would land inside it and be deleted on the 4th deploy → silent data
    // loss. Require an absolute SQLite path (or Postgres) instead.
    if artifact_active && !is_pg && Path::new(&db_target).is_relative() {
        print_diagnostics(
            &[Diagnostic {
                severity: Severity::Error,
                code: "START_ARTIFACT_DB_RELATIVE".into(),
                message: format!(
                    "artifact boot requires an absolute DATABASE_URL or PYLON_DB_PATH; \
                     got relative SQLite path {db_target:?} which would be pruned with the \
                     deploy"
                ),
                span: None,
                hint: Some("set PYLON_DB_PATH=/data/pylon.db (or DATABASE_URL)".into()),
            }],
            json_mode,
        );
        return ExitCode::Error;
    }

    if !is_pg {
        // SQLite path: ensure parent dir exists.
        if let Some(parent) = Path::new(&db_target).parent() {
            if !parent.as_os_str().is_empty() {
                let _ = std::fs::create_dir_all(parent);
            }
        }
    }

    // Apply schema diff. In prod we only expand-compatible; destructive
    // migrations are the operator's explicit responsibility via
    // `pylon migrate`. Postgres apply uses `LivePostgresAdapter`.
    if is_pg {
        match pylon_storage::postgres::live::LivePostgresAdapter::connect(&db_target) {
            Ok(mut adapter) => {
                if let Ok(plan) = adapter.plan_from_live(&manifest) {
                    if let Err(e) = adapter.apply_plan(&plan) {
                        if !json_mode {
                            eprintln!("[start] Postgres schema apply failed: {e}");
                        }
                    }
                }
            }
            Err(e) => {
                if !json_mode {
                    eprintln!("[start] Could not connect to Postgres for schema apply: {e}");
                }
            }
        }
    } else if let Ok(adapter) = pylon_storage::sqlite::SqliteAdapter::open(&db_target) {
        if let Ok(plan) = adapter.plan_from_live(&manifest) {
            let meta = pylon_storage::sqlite::PushMetadata {
                manifest_version: manifest.manifest_version,
                app_version: &manifest.version,
                baseline: "start",
            };
            let _ = adapter.apply_with_history(&plan, &meta);
        }
    }

    let runtime = match pylon_runtime::Runtime::open(&db_target, manifest.clone()) {
        Ok(rt) => Arc::new(rt),
        Err(e) => {
            if !json_mode {
                eprintln!("[start] Failed to open runtime: {e}");
            }
            return ExitCode::Error;
        }
    };

    // Hand the runtime the paths to studio artefacts (if present) so
    // /studio renders against the user's config and /studio/extensions.js
    // can serve the bundled custom components.
    let cfg_path = studio_data_dir.join(studio_config::STUDIO_CONFIG_OUT);
    if cfg_path.exists() {
        runtime.set_studio_config_path(Some(cfg_path));
    }
    let ext_path = studio_data_dir.join(studio_config::STUDIO_ENTRY_OUT);
    if ext_path.exists() {
        runtime.set_studio_entry_path(Some(ext_path));
    }

    let backend_label = if is_pg { "postgres" } else { "sqlite" };
    if json_mode {
        print_json(&serde_json::json!({
            "code": "START_OK",
            "name": manifest.name,
            "version": manifest.version,
            "entry": entry_file,
            "port": port,
            "db_path": db_target,
            "backend": backend_label,
        }));
    } else {
        println!("pylon start");
        println!("  App:      {} v{}", manifest.name, manifest.version);
        println!("  Server:   http://0.0.0.0:{port}");
        println!("  Backend:  {backend_label}");
        println!("  Database: {db_target}");
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
