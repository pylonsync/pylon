use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use pylon_kernel::{Diagnostic, ExitCode, Severity};
use serde::Serialize;

use crate::bun::run_bun_codegen;
use crate::client_codegen::generate_client_ts;
use crate::commands::args::collect_positional;
use crate::manifest::{parse_manifest, validate_all};
use crate::output::{print_diagnostics, print_json};
use crate::studio_config;

const DEFAULT_PORT: u16 = 4321;

// ---------------------------------------------------------------------------
// Output types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct DevOutput {
    code: &'static str,
    name: String,
    version: String,
    entry: String,
    entities: Vec<String>,
    queries: Vec<String>,
    actions: Vec<String>,
    policies: Vec<String>,
    routes: Vec<String>,
    warnings: Vec<DevWarning>,
}

#[derive(Serialize)]
struct DevWarning {
    code: String,
    message: String,
}

#[derive(Serialize)]
struct WatchEvent {
    code: &'static str,
    rebuild: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    diagnostics: Vec<Diagnostic>,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let once_mode = args.iter().any(|a| a == "--once");

    // Load .env / .env.local before anything else so the runtime sees
    // the resulting env vars when it boots. Process env always wins; among
    // files, .env.local overrides .env. We walk up from cwd to find them
    // — monorepos like pylon-cloud put env files at the workspace root
    // while `pylon dev` runs from apps/<name>/.
    let loaded_env = load_env_files();
    if !json_mode && !loaded_env.is_empty() {
        for p in &loaded_env {
            println!("  env: {}", p.display());
        }
    }

    let port: u16 = args
        .windows(2)
        .find(|w| w[0] == "--port" || w[0] == "-p")
        .and_then(|w| w[1].parse().ok())
        .unwrap_or(DEFAULT_PORT);

    let positional: Vec<&str> = collect_positional(args, "dev");

    let entry_file = match positional.first() {
        Some(f) => f.to_string(),
        None => {
            // Auto-discover the entry file. Both `app.ts` and `schema.ts`
            // are conventional names; the create-pylon templates use
            // `app.ts`, but several of Eric's downstream apps (yapless,
            // pulse-therapy) named theirs `schema.ts` before the
            // convention settled. Supporting both removes the need for
            // every package.json to spell out `pylon dev schema.ts`
            // explicitly.
            const CANDIDATES: &[&str] = &["app.ts", "schema.ts"];
            match CANDIDATES.iter().find(|c| Path::new(c).exists()) {
                Some(f) => (*f).to_string(),
                None => {
                    print_diagnostics(
                        &[Diagnostic {
                            severity: Severity::Error,
                            code: "DEV_NO_ENTRY".into(),
                            message:
                                "No entry file provided and neither app.ts nor schema.ts found in current directory"
                                    .into(),
                            span: None,
                            hint: Some(
                                "Usage: pylon dev [app.ts | schema.ts] [--once]".into(),
                            ),
                        }],
                        json_mode,
                    );
                    return ExitCode::Usage;
                }
            }
        }
    };

    if !Path::new(&entry_file).exists() {
        print_diagnostics(
            &[Diagnostic {
                severity: Severity::Error,
                code: "DEV_ENTRY_NOT_FOUND".into(),
                message: format!("Entry file not found: {entry_file}"),
                span: None,
                hint: Some(
                    "Provide a path to a .ts file that exports a manifest via buildManifest".into(),
                ),
            }],
            json_mode,
        );
        return ExitCode::Error;
    }

    if once_mode {
        run_once(&entry_file, json_mode)
    } else {
        run_watch(&entry_file, json_mode, port)
    }
}

// ---------------------------------------------------------------------------
// One-shot mode (existing behavior)
// ---------------------------------------------------------------------------

fn run_once(entry_file: &str, json_mode: bool) -> ExitCode {
    let manifest_json = match run_bun_codegen(entry_file) {
        Ok(json) => json,
        Err(diag) => {
            print_diagnostics(&[diag], json_mode);
            return ExitCode::Error;
        }
    };

    let manifest = match parse_manifest(&manifest_json, entry_file) {
        Ok(m) => m,
        Err(diags) => {
            print_diagnostics(&diags, json_mode);
            return ExitCode::Error;
        }
    };

    let diagnostics = validate_all(&manifest);
    let has_errors = diagnostics.iter().any(|d| d.severity == Severity::Error);

    if has_errors {
        print_diagnostics(&diagnostics, json_mode);
        return ExitCode::Error;
    }

    // Write generated files alongside the entry file.
    write_generated_files(entry_file, &manifest_json, &manifest);

    if json_mode {
        let output = DevOutput {
            code: "DEV_OK",
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            entry: entry_file.to_string(),
            entities: manifest.entities.iter().map(|e| e.name.clone()).collect(),
            queries: manifest.queries.iter().map(|q| q.name.clone()).collect(),
            actions: manifest.actions.iter().map(|a| a.name.clone()).collect(),
            policies: manifest.policies.iter().map(|p| p.name.clone()).collect(),
            routes: manifest.routes.iter().map(|r| r.path.clone()).collect(),
            warnings: diagnostics
                .iter()
                .filter(|d| d.severity == Severity::Warning)
                .map(|d| DevWarning {
                    code: d.code.clone(),
                    message: d.message.clone(),
                })
                .collect(),
        };
        print_json(&output);
    } else {
        println!("pylon dev");
        println!();
        println!("  App:       {} v{}", manifest.name, manifest.version);
        println!("  Entry:     {entry_file}");
        println!("  Entities:  {}", manifest.entities.len());
        println!("  Queries:   {}", manifest.queries.len());
        println!("  Actions:   {}", manifest.actions.len());
        println!("  Policies:  {}", manifest.policies.len());
        println!("  Routes:    {}", manifest.routes.len());
        println!();

        if !diagnostics.is_empty() {
            for d in &diagnostics {
                println!("  {d}");
            }
            println!();
        }

        // Policy lint (esp. PYL002: an entity with no policy default-denies at
        // runtime). Surface it in --once too so scripts / CI catch it.
        let lint_findings = crate::commands::lint::lint_manifest(&manifest);
        if !lint_findings.is_empty() {
            for f in &lint_findings {
                let loc = match (&f.entity, &f.policy) {
                    (Some(e), Some(p)) => format!(" [{e} · {p}]"),
                    (Some(e), None) => format!(" [{e}]"),
                    _ => String::new(),
                };
                println!("  \u{26a0} policy {}{}: {}", f.rule, loc, f.message);
                println!("       fix: {}", f.fix);
            }
            println!();
        }

        println!("Schema valid. Use 'pylon dev' (without --once) to start the dev server.");
    }

    ExitCode::Ok
}

// ---------------------------------------------------------------------------
// Watch mode
// ---------------------------------------------------------------------------

fn run_watch(entry_file: &str, json_mode: bool, port: u16) -> ExitCode {
    let entry_path = Path::new(entry_file);
    let watch_dir = entry_path.parent().unwrap_or(Path::new("."));

    if !json_mode {
        println!("pylon dev");
        println!("  Watching: {} (*.ts)", watch_dir.display());
        println!("  Server:   http://localhost:{port}");
        println!();
    }

    // Initial build — also start the dev server on success.
    let mut rebuild_count: u32 = 0;
    let manifest = run_rebuild_and_get_manifest(entry_file, json_mode, &mut rebuild_count);

    // Start dev server in background if initial build succeeded.
    if let Some(m) = manifest {
        // Keep the project root clean: machine-local dev data lives in a
        // hidden `.pylon/` folder alongside source, the same way
        // `.next/` or `target/` do. The sessions + jobs siblings that
        // the server derives from `db_path` follow automatically.
        let data_dir = watch_dir.join(".pylon");
        if let Err(e) = std::fs::create_dir_all(&data_dir) {
            if !json_mode {
                eprintln!(
                    "[dev] Failed to create data dir {}: {e}",
                    data_dir.display()
                );
            }
            return ExitCode::Error;
        }
        // DATABASE_URL takes precedence — local dev against a real Postgres
        // (e.g. `pylon dev` against the SST docker-compose stack) just sets
        // DATABASE_URL=postgres://... and the runtime opens that instead of
        // the hidden SQLite file.
        let (db_str, is_pg) = match std::env::var("DATABASE_URL") {
            Ok(url) if url.starts_with("postgres://") || url.starts_with("postgresql://") => {
                (url, true)
            }
            _ => {
                let db_path = data_dir.join("dev.db");
                (db_path.to_string_lossy().to_string(), false)
            }
        };

        // Default uploads into the hidden data dir too so `examples/*/uploads/`
        // stops littering project roots. Operators can still override via
        // PYLON_FILES_DIR for production layouts.
        if std::env::var("PYLON_FILES_DIR").is_err() {
            let uploads = data_dir.join("uploads");
            // Safety: single-threaded here; server thread spawns below.
            unsafe {
                std::env::set_var("PYLON_FILES_DIR", uploads);
            }
        }

        // `pylon dev` is interactive local development — opt into dev
        // mode automatically. The runtime defaults this to false now
        // (production-safe), so without this flip every `pylon dev`
        // would refuse to start without PYLON_CORS_ORIGIN set, magic
        // codes wouldn't appear in JSON for testing, etc.
        unsafe {
            if std::env::var("PYLON_DEV_MODE").is_err() {
                std::env::set_var("PYLON_DEV_MODE", "1");
            }
        }

        // Dev-mode rate limits — production defaults (30 fn calls / min)
        // strangle realtime demos like world3d that write pose at 10 Hz.
        // Operators can still override any of these for stress testing.
        unsafe {
            if std::env::var("PYLON_RATE_LIMIT_MAX").is_err() {
                std::env::set_var("PYLON_RATE_LIMIT_MAX", "100000");
            }
            if std::env::var("PYLON_FN_RATE_LIMIT_MAX").is_err() {
                std::env::set_var("PYLON_FN_RATE_LIMIT_MAX", "100000");
            }
        }

        // Auto-push schema to the dev database. Postgres path uses
        // `LivePostgresAdapter`; SQLite path keeps the existing
        // history-tracked apply.
        if is_pg {
            match pylon_storage::postgres::live::LivePostgresAdapter::connect(&db_str) {
                Ok(mut adapter) => {
                    if let Ok(plan) = adapter.plan_from_live(&m) {
                        if let Err(e) = adapter.apply_plan(&plan) {
                            if !json_mode {
                                eprintln!("[dev] Postgres schema apply failed: {e}");
                            }
                        } else if !json_mode && !plan.is_empty() {
                            println!("  Database: {db_str} (postgres, schema synced)");
                            println!();
                        }
                    }
                }
                Err(e) => {
                    if !json_mode {
                        eprintln!("[dev] Could not connect to Postgres: {e}");
                    }
                }
            }
        } else if let Ok(adapter) = pylon_storage::sqlite::SqliteAdapter::open(&db_str) {
            if let Ok(plan) = adapter.plan_from_live(&m) {
                let meta = pylon_storage::sqlite::PushMetadata {
                    manifest_version: m.manifest_version,
                    app_version: &m.version,
                    baseline: "dev",
                };
                let _ = adapter.apply_with_history(&plan, &meta);
                if !json_mode && !plan.is_empty() {
                    println!("  Database: {db_str} (schema synced)");
                    println!();
                }
            }
        }

        // Open runtime against the persistent DB.
        let runtime = match pylon_runtime::Runtime::open(&db_str, m) {
            Ok(rt) => Arc::new(rt),
            Err(e) => {
                if !json_mode {
                    eprintln!("[dev] Failed to start runtime: {e}");
                }
                return ExitCode::Error;
            }
        };

        // Point /studio at the studio artefacts inside .pylon/. The
        // runtime re-reads the config file on every render so dev
        // edits to studio.config.ts take effect on refresh — no
        // server restart needed. The extension bundle is served at
        // /studio/extensions.js from disk for the same reason.
        let studio_cfg_path = data_dir.join(studio_config::STUDIO_CONFIG_OUT);
        if studio_cfg_path.exists() {
            runtime.set_studio_config_path(Some(studio_cfg_path));
        }
        let studio_ext_path = data_dir.join(studio_config::STUDIO_ENTRY_OUT);
        if studio_ext_path.exists() {
            runtime.set_studio_entry_path(Some(studio_ext_path));
        }

        // Frontend dev server — single-command DX for full-stack apps.
        // If the project has a `web/` directory with a package.json that
        // defines a `dev` script, spawn `bun run dev` in it as a child
        // process AND tell pylon-runtime to reverse-proxy non-API GETs
        // to it. From the user's perspective, only :4321 exists — the
        // Vite child runs on :5173 but they never type that URL.
        //
        // Must run BEFORE pylon-runtime starts so PYLON_FRONTEND_DEV_PROXY
        // is set when the runtime reads the env at boot.
        spawn_frontend_dev_server(watch_dir, json_mode);

        let rt_clone = Arc::clone(&runtime);
        std::thread::spawn(move || {
            // Previously this dropped the error with `let _ = ...` which made
            // misconfigurations (e.g. PYLON_CORS_ORIGIN unset in prod) look
            // like a hang: machine boots, init logs scroll by, port 4321 never
            // accepts, no error anywhere. Print to stderr so the operator sees
            // the real reason in `fly logs` / container stdout.
            if let Err(e) = pylon_runtime::server::start(rt_clone, port) {
                eprintln!("[pylon] server failed to start: {e}");
                std::process::exit(1);
            }
        });
    }

    // Poll loop. We track file mtimes under the watch dir and react
    // depending on what changed:
    //
    //   - Any file in `functions/` (including new files): the functions
    //     runtime is a bun subprocess that reads the functions dir once
    //     at startup. To pick up new handlers or edits, we need to
    //     restart the whole process. We exec a fresh `pylon dev` with
    //     the same argv so the terminal session continues seamlessly.
    //   - Other .ts files (primarily app.ts): regenerate the manifest +
    //     typed client in place. No restart needed; the running server
    //     still serves the old in-memory schema until the next restart,
    //     but client bindings are up to date for codegen consumers.
    let mut last_mtimes = collect_ts_mtimes(watch_dir);
    let functions_dir = watch_dir.join("functions");

    // Watch env files too so editing OAuth secrets etc. triggers a restart
    // without needing to Ctrl-C. We re-derive the candidate set here (env
    // files were already loaded in run() so the runtime above sees them);
    // this just gives us paths to mtime-poll.
    let env_watch_paths = env_watch_paths();
    let mut last_env_mtimes = collect_env_mtimes(&env_watch_paths);

    loop {
        std::thread::sleep(Duration::from_millis(500));

        let current_env_mtimes = collect_env_mtimes(&env_watch_paths);
        let env_changed = current_env_mtimes != last_env_mtimes;
        last_env_mtimes = current_env_mtimes;

        let current_mtimes = collect_ts_mtimes(watch_dir);
        if env_changed {
            if !json_mode {
                println!();
                println!("  ✓ env changed — restarting");
                println!();
            }
            last_mtimes = current_mtimes;
            exec_restart(json_mode);
            continue;
        }

        if current_mtimes != last_mtimes {
            // Compute which files actually changed.
            let functions_changed = current_mtimes.iter().any(|(path, mtime)| {
                let is_in_functions = Path::new(path).starts_with(&functions_dir);
                if !is_in_functions {
                    return false;
                }
                match last_mtimes.get(path) {
                    Some(prev) => prev != mtime,
                    None => true, // new file in functions/
                }
            }) || last_mtimes.iter().any(|(path, _)| {
                Path::new(path).starts_with(&functions_dir) && !current_mtimes.contains_key(path)
            }); // deletion

            last_mtimes = current_mtimes;

            if functions_changed {
                if !json_mode {
                    println!();
                    println!("  ✓ functions changed — restarting");
                    println!();
                }
                exec_restart(json_mode);
                continue;
            }

            // Non-functions change (app.ts / schema helpers / lib). The
            // manifest — entities, policies, auth config, routes — lives in
            // the running server's memory, so re-running codegen alone left
            // the server enforcing the OLD schema until the next manual
            // restart (edit a policy, see no effect). Rebuild first; if
            // codegen + validation succeed, restart so the new schema takes
            // effect. On error, keep the running server alive (the user is
            // mid-edit) and just surface the diagnostics.
            if run_rebuild_and_get_manifest(entry_file, json_mode, &mut rebuild_count).is_some() {
                if !json_mode {
                    println!();
                    println!("  ✓ schema changed — restarting");
                    println!();
                }
                exec_restart(json_mode);
            }
        }
    }
}

/// Like run_rebuild but returns the manifest on success (for server init).
fn run_rebuild_and_get_manifest(
    entry_file: &str,
    json_mode: bool,
    count: &mut u32,
) -> Option<pylon_kernel::AppManifest> {
    *count += 1;
    let n = *count;

    let manifest_json = match run_bun_codegen(entry_file) {
        Ok(json) => json,
        Err(diag) => {
            if json_mode {
                print_json(&WatchEvent {
                    code: "DEV_ERROR",
                    rebuild: n,
                    name: None,
                    version: None,
                    error: Some(diag.message.clone()),
                    diagnostics: vec![diag],
                });
            } else {
                // Print message, then hint on its own line. Prior version
                // dropped the hint silently — schema TypeErrors landed in
                // hint via run_bun_codegen and the user saw a useless
                // "exited with status 1" line. Now inline diagnostic
                // formatting matches the rest of the CLI.
                eprintln!("[{n}] {} {}", diag.severity, diag.message);
                if let Some(hint) = &diag.hint {
                    eprintln!("    hint: {hint}");
                }
            }
            return None;
        }
    };

    let manifest = match parse_manifest(&manifest_json, entry_file) {
        Ok(m) => m,
        Err(diags) => {
            if json_mode {
                print_json(&WatchEvent {
                    code: "DEV_ERROR",
                    rebuild: n,
                    name: None,
                    version: None,
                    error: Some(diags.first().map(|d| d.message.clone()).unwrap_or_default()),
                    diagnostics: diags,
                });
            } else {
                for d in &diags {
                    eprintln!("[{n}] {d}");
                }
            }
            return None;
        }
    };

    let diagnostics = validate_all(&manifest);
    let has_errors = diagnostics.iter().any(|d| d.severity == Severity::Error);

    if !has_errors {
        write_generated_files(entry_file, &manifest_json, &manifest);
        build_studio_artefacts(entry_file, json_mode);
    }

    if json_mode {
        print_json(&WatchEvent {
            code: if has_errors { "DEV_ERROR" } else { "DEV_OK" },
            rebuild: n,
            name: Some(manifest.name.clone()),
            version: Some(manifest.version.clone()),
            error: None,
            diagnostics: diagnostics.clone(),
        });
    } else if has_errors {
        for d in &diagnostics {
            eprintln!("[{n}] {d}");
        }
    } else {
        println!(
            "[{n}] OK: {} v{} — {} entities, {} queries, {} actions, {} policies, {} routes",
            manifest.name,
            manifest.version,
            manifest.entities.len(),
            manifest.queries.len(),
            manifest.actions.len(),
            manifest.policies.len(),
            manifest.routes.len(),
        );
        // Surface policy-lint findings here at dev startup AND on every
        // rebuild. The big one is PYL002: an entity with NO policy
        // default-DENIES at runtime, so reads/writes return 403 with no other
        // signal — a newcomer who forgot a policy would otherwise just see
        // silent 403s in the browser. Make it a loud, actionable warning with
        // the one-line fix. Non-fatal: the dev server still starts (you may be
        // mid-edit). Run `pylon lint` for the full report.
        let lint_findings = crate::commands::lint::lint_manifest(&manifest);
        for f in &lint_findings {
            let loc = match (&f.entity, &f.policy) {
                (Some(e), Some(p)) => format!(" [{e} · {p}]"),
                (Some(e), None) => format!(" [{e}]"),
                _ => String::new(),
            };
            eprintln!("[{n}] \u{26a0} policy {}{}: {}", f.rule, loc, f.message);
            eprintln!("       fix: {}", f.fix);
        }
    }

    if has_errors {
        None
    } else {
        Some(manifest)
    }
}

/// Write generated manifest and client bindings alongside the entry file.
fn write_generated_files(
    entry_file: &str,
    manifest_json: &str,
    manifest: &pylon_kernel::AppManifest,
) {
    let entry_path = Path::new(entry_file);
    let dir = entry_path.parent().unwrap_or(Path::new("."));

    // Write manifest.
    let manifest_path = dir.join("pylon.manifest.json");
    let _ = std::fs::write(&manifest_path, format!("{manifest_json}\n"));

    // Write client bindings.
    let client_path = dir.join("pylon.client.ts");
    let client_ts = generate_client_ts(manifest);
    let _ = std::fs::write(&client_path, client_ts);
}

/// Build studio artefacts (`.pylon/studio.config.json` + optional
/// `.pylon/studio.entry.js`). Studio config failures are non-fatal —
/// they're surfaced as warnings so the manifest build still goes
/// through. The runtime falls back to defaults when the config is
/// missing or invalid.
fn build_studio_artefacts(entry_file: &str, json_mode: bool) {
    let entry_path = Path::new(entry_file);
    let data_dir = entry_path.parent().unwrap_or(Path::new(".")).join(".pylon");

    // Skip when neither file exists. The runtime treats a missing
    // config the same as an empty one, so writing a default JSON
    // would just churn .pylon/ on every rebuild.
    if studio_config::locate_config(entry_file).is_none()
        && studio_config::locate_entry(entry_file).is_none()
    {
        return;
    }

    match studio_config::build_artefacts(entry_file, &data_dir) {
        Ok(_) => {}
        Err(diags) => {
            // Demote to warnings so the manifest pass dominates.
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
}

/// Walk up from cwd looking for `.env.local` and `.env`, then load them
/// into the process environment. Stops at the first directory that
/// contains either file (so monorepo roots win over per-app dirs that
/// happen to have nothing). Process env always wins; among files,
/// `.env.local` overrides `.env`.
fn load_env_files() -> Vec<PathBuf> {
    let mut loaded = Vec::new();
    let Ok(cwd) = std::env::current_dir() else {
        return loaded;
    };
    let mut dir: Option<&Path> = Some(cwd.as_path());
    while let Some(d) = dir {
        let local = d.join(".env.local");
        let base = d.join(".env");
        if local.exists() || base.exists() {
            // Load .env.local first — dotenvy::from_path doesn't override
            // already-set vars, so subsequent .env loads only fill gaps.
            if local.exists() && dotenvy::from_path(&local).is_ok() {
                loaded.push(local);
            }
            if base.exists() && dotenvy::from_path(&base).is_ok() {
                loaded.push(base);
            }
            break;
        }
        dir = d.parent();
    }
    loaded
}

/// Paths to watch for env-file changes. Mirrors `load_env_files`'s walk:
/// stop at the first ancestor with either file. If none exist, fall back
/// to cwd candidates so creating `.env.local` mid-session still fires.
fn env_watch_paths() -> Vec<PathBuf> {
    let Ok(cwd) = std::env::current_dir() else {
        return Vec::new();
    };
    let mut dir: Option<&Path> = Some(cwd.as_path());
    while let Some(d) = dir {
        let local = d.join(".env.local");
        let base = d.join(".env");
        if local.exists() || base.exists() {
            return vec![local, base];
        }
        dir = d.parent();
    }
    vec![cwd.join(".env.local"), cwd.join(".env")]
}

/// mtime per env-watch path. Missing files map to `None`, so a transition
/// from missing → present (or vice versa) registers as a change.
fn collect_env_mtimes(paths: &[PathBuf]) -> HashMap<PathBuf, Option<SystemTime>> {
    paths
        .iter()
        .map(|p| {
            let mtime = std::fs::metadata(p).and_then(|m| m.modified()).ok();
            (p.clone(), mtime)
        })
        .collect()
}

/// Replace the current process with a fresh `pylon dev` invocation.
/// On-disk state (sessions, DB, uploads) survives in `.pylon/`. WS
/// connections drop and reconnect in ~100ms.
fn exec_restart(_json_mode: bool) {
    let args: Vec<String> = std::env::args().collect();
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[dev] could not locate self for restart: {e}");
            return;
        }
    };
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = std::process::Command::new(&exe).args(&args[1..]).exec();
        eprintln!("[dev] exec failed: {err}");
    }
    #[cfg(not(unix))]
    {
        let _ = std::process::Command::new(&exe).args(&args[1..]).spawn();
        std::process::exit(0);
    }
}

/// Spawn the frontend dev server if the project has one.
///
/// Detection: look for `<watch_dir>/web/package.json` with a `dev`
/// script. The `web/` directory convention matches the init template
/// (`apps/web/` in newer projects, plain `web/` in the chat/todo
/// examples). When found, spawn `bun run dev` in that directory and
/// inherit stdout/stderr so dev-server logs interleave with Pylon's.
///
/// Lifecycle: the child inherits stdin from the parent. When the
/// user Ctrl-Cs `pylon dev`, both processes are in the foreground
/// process group on Unix, so SIGINT is delivered to the child too
/// and Next/Vite shut down cleanly. (On Windows we rely on the
/// child observing stdin closure.)
///
/// Unified full-stack dev: spawn Vite + tell pylon-runtime to proxy
/// non-API GETs to it. After this, the user only ever types :4321 —
/// the Vite child on :5173 is an implementation detail.
///
/// Port selection: Vite's default is 5173. We pass `--port 5173` so
/// it stays deterministic even if a user has a stray VITE_PORT env
/// somewhere, then export `PYLON_FRONTEND_DEV_PROXY=http://localhost:5173`
/// so pylon-runtime's frontend module knows where to send traffic.
/// Users with a custom port can set `PYLON_FRONTEND_DEV_PROXY`
/// themselves and we honor it (env-var overrides take precedence
/// over our default — see frontend::FrontendConfig::from_env).
fn spawn_frontend_dev_server(watch_dir: &Path, json_mode: bool) {
    let candidates = [watch_dir.join("web"), watch_dir.join("apps").join("web")];
    let Some(web_dir) = candidates.into_iter().find(|p| {
        p.join("package.json").is_file() && package_json_has_dev_script(&p.join("package.json"))
    }) else {
        return;
    };

    // The proxy URL pylon-runtime will use. If the user already set
    // PYLON_FRONTEND_DEV_PROXY explicitly (custom port or remote dev
    // server), respect that and don't pin --port.
    let user_override = std::env::var("PYLON_FRONTEND_DEV_PROXY").ok().is_some();
    let vite_port: u16 = 5173;

    // `bun run dev -- --port 5173`. The `--` separator forwards the
    // port flag to vite without bun trying to interpret it.
    let mut cmd = std::process::Command::new("bun");
    cmd.args(["run", "dev"]).current_dir(&web_dir);
    if !user_override {
        cmd.args(["--", "--port", "5173"]);
        // Set env BEFORE spawning the child so pylon-runtime (started
        // moments later in the same process) reads it.
        std::env::set_var(
            "PYLON_FRONTEND_DEV_PROXY",
            format!("http://localhost:{vite_port}"),
        );
    }
    cmd.stdin(std::process::Stdio::inherit());
    cmd.stdout(std::process::Stdio::inherit());
    cmd.stderr(std::process::Stdio::inherit());

    match cmd.spawn() {
        Ok(child) => {
            if !json_mode {
                let proxy = std::env::var("PYLON_FRONTEND_DEV_PROXY")
                    .unwrap_or_else(|_| "(none — set PYLON_FRONTEND_DEV_PROXY)".into());
                eprintln!(
                    "[dev] Frontend dev server started from {} (pid {}), proxied via Pylon at {}",
                    web_dir.display(),
                    child.id(),
                    proxy,
                );
            }
            std::mem::forget(child);
        }
        Err(e) => {
            if !json_mode {
                eprintln!(
                    "[dev] Could not spawn frontend dev server in {}: {e}",
                    web_dir.display()
                );
                eprintln!("[dev] Skipping — run `bun run dev` in that directory yourself.");
            }
            // Clear the env var so pylon-runtime doesn't try to proxy
            // to a non-existent server and 502 every request.
            if !user_override {
                std::env::remove_var("PYLON_FRONTEND_DEV_PROXY");
            }
        }
    }
}

fn package_json_has_dev_script(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return false;
    };
    value
        .get("scripts")
        .and_then(|s| s.get("dev"))
        .and_then(|d| d.as_str())
        .map(|s| !s.is_empty())
        .unwrap_or(false)
}

/// Collect mtime of `.ts` and `.tsx` files in a directory, excluding
/// generated files. `.tsx` is included so edits to `studio.entry.tsx`
/// trigger the same rebuild path as edits to `app.ts` /
/// `studio.config.ts`.
fn collect_ts_mtimes(dir: &Path) -> HashMap<String, SystemTime> {
    let mut mtimes = HashMap::new();
    collect_ts_mtimes_into(dir, &mut mtimes);
    mtimes
}

/// Recursively walk `dir` collecting mtimes of every `*.ts` / `*.tsx` /
/// `*.css` source file. Recursion matters: app source lives in
/// subdirectories (`functions/`, `lib/`, `emails/`, …), and the watcher's
/// restart-on-change logic keys off these paths — a flat top-level scan
/// silently missed every edit under `functions/`, so handler changes never
/// hot-reloaded (you had to Ctrl-C and restart by hand). `.css` is watched
/// too so editing `app/globals.css` (Tailwind / shadcn tokens) re-runs the
/// SSR build and recompiles the stylesheet instead of serving stale CSS.
/// Heavy / generated trees are skipped so the 500ms poll doesn't crawl
/// node_modules.
fn collect_ts_mtimes_into(dir: &Path, mtimes: &mut HashMap<String, SystemTime>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                // Vendored / generated / VCS trees can't hold app source
                // and would make the poll loop walk thousands of files.
                if matches!(
                    name,
                    "node_modules"
                        | ".pylon"
                        | ".git"
                        | "dist"
                        | ".next"
                        | "target"
                        | "build"
                        | ".turbo"
                        | "coverage"
                ) {
                    continue;
                }
            }
            collect_ts_mtimes_into(&path, mtimes);
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str());
        if ext == Some("ts") || ext == Some("tsx") || ext == Some("css") {
            // Skip generated files to avoid infinite rebuild loops.
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with("pylon.") {
                    continue;
                }
            }
            if let Ok(meta) = std::fs::metadata(&path) {
                if let Ok(mtime) = meta.modified() {
                    mtimes.insert(path.to_string_lossy().to_string(), mtime);
                }
            }
        }
    }
}
