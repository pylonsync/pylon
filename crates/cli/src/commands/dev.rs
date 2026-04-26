use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use pylon_kernel::{Diagnostic, ExitCode, Severity};
use serde::Serialize;

use crate::bun::run_bun_codegen;
use crate::client_codegen::generate_client_ts;
use crate::manifest::{parse_manifest, validate_all};
use crate::output::{print_diagnostics, print_json};

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
        .find(|w| w[0] == "--port")
        .and_then(|w| w[1].parse().ok())
        .unwrap_or(DEFAULT_PORT);

    let positional: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with('-') && *a != "dev")
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
                        code: "DEV_NO_ENTRY".into(),
                        message: "No entry file provided and no app.ts found in current directory"
                            .into(),
                        span: None,
                        hint: Some("Usage: pylon dev [app.ts] [--once]".into()),
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
        let db_path = data_dir.join("dev.db");
        let db_str = db_path.to_string_lossy().to_string();

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

        // Auto-push schema to the dev database.
        if let Ok(adapter) = pylon_storage::sqlite::SqliteAdapter::open(&db_str) {
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

            // Non-functions change — incremental rebuild only.
            run_rebuild(entry_file, json_mode, &mut rebuild_count);
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
                eprintln!("[{n}] Error: {}", diag.message);
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
    }

    if has_errors {
        None
    } else {
        Some(manifest)
    }
}

fn run_rebuild(entry_file: &str, json_mode: bool, count: &mut u32) {
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
                eprintln!("[{n}] Error: {}", diag.message);
            }
            return;
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
            return;
        }
    };

    let diagnostics = validate_all(&manifest);
    let has_errors = diagnostics.iter().any(|d| d.severity == Severity::Error);

    // Write generated files on success.
    if !has_errors {
        write_generated_files(entry_file, &manifest_json, &manifest);
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
    }
}

/// Collect mtime of all `.ts` files in a directory (non-recursive).
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

/// Collect mtime of `.ts` files in a directory, excluding generated files.
fn collect_ts_mtimes(dir: &Path) -> HashMap<String, SystemTime> {
    let mut mtimes = HashMap::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("ts") {
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
    mtimes
}
