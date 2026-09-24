use std::path::Path;

use pylon_kernel::{Diagnostic, ExitCode, Severity};
use serde::Serialize;

use crate::bun::run_bun_codegen;
use crate::client_codegen::generate_client_ts;
use crate::manifest::{load_manifest, parse_manifest};
use crate::output::{print_diagnostics, print_json};

#[derive(Serialize)]
struct BuildResult {
    code: &'static str,
    manifest: String,
    client: String,
    static_pages: usize,
    out_dir: String,
    /// The production artifact, when one was built.
    #[serde(skip_serializing_if = "Option::is_none")]
    artifact: Option<serde_json::Value>,
}

/// `pylon build` — production build. One command, no flags required.
///
///   1. Run `bun run app.ts` (or `schema.ts`) to evaluate the manifest.
///   2. Write `pylon.manifest.json` (the canonical manifest).
///   3. Write `pylon.client.ts` (TS client bindings — what the web /
///      mobile app imports for typed query/action access).
///   4. Write the production artifact to `dist/` (see
///      `@pylonsync/functions/src/production-build.ts`): the server bundle,
///      the client bundle, `public/`, and the manifest. `pylon start dist`
///      runs it without the source tree or `node_modules`.
///   5. Render static routes to `dist/static/`, build a `web/` SPA into
///      `dist/web/dist/`, and copy studio files into `dist/.pylon/`.
///
/// `--no-bundle` stops after step 3 and renders static routes into `--out`
/// directly (the behavior before the artifact existed). `--compile` also
/// copies the `pylon` and `bun` binaries into `dist/bin/`.
pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let manifest_out = args
        .windows(2)
        .find(|w| w[0] == "--manifest-out")
        .map(|w| w[1].as_str())
        .unwrap_or("pylon.manifest.json");
    let client_out = args
        .windows(2)
        .find(|w| w[0] == "--client-out")
        .map(|w| w[1].as_str())
        .unwrap_or("pylon.client.ts");
    let out_dir = args
        .windows(2)
        .find(|w| w[0] == "--out")
        .map(|w| w[1].as_str())
        .unwrap_or("dist");
    let no_client = args.iter().any(|a| a == "--no-client");
    let no_static = args.iter().any(|a| a == "--no-static");
    let no_bundle = args.iter().any(|a| a == "--no-bundle");
    let compile = args.iter().any(|a| a == "--compile");

    // Skip flag-value strings (those don't start with `-` so they look
    // like positional args otherwise).
    let positional: Vec<&str> = args
        .iter()
        .filter(|a| {
            !a.starts_with('-')
                && *a != "build"
                && *a != manifest_out
                && *a != client_out
                && *a != out_dir
        })
        .map(|s| s.as_str())
        .collect();

    // Auto-discover the entry file. If the first positional arg is a
    // pre-built manifest JSON path (legacy `pylon build path/to/manifest.json`
    // for static-only generation), honor it for back-compat. Otherwise
    // treat positional[0] as a TS entry file.
    let entry_or_manifest = positional.first().copied();

    // Decide path: if the user passed a `.json` file, treat as a
    // pre-built manifest and skip codegen. Otherwise run codegen first.
    let entry_file: String = match entry_or_manifest {
        Some(p) if p.ends_with(".json") => {
            // Legacy mode: load existing manifest, no codegen.
            let manifest = match load_manifest(p) {
                Ok(m) => m,
                Err(diags) => {
                    print_diagnostics(&diags, json_mode);
                    return ExitCode::Error;
                }
            };
            return run_static_only(&manifest, p, out_dir, json_mode, no_static);
        }
        Some(p) => p.to_string(),
        None => {
            // No positional → auto-discover app.ts / schema.ts.
            const CANDIDATES: &[&str] = &["app.ts", "schema.ts"];
            match CANDIDATES.iter().find(|c| Path::new(c).exists()) {
                Some(f) => f.to_string(),
                None => {
                    print_diagnostics(
                        &[Diagnostic {
                            severity: Severity::Error,
                            code: "BUILD_NO_ENTRY".into(),
                            message:
                                "No entry file provided and neither app.ts nor schema.ts found in current directory"
                                    .into(),
                            span: None,
                            hint: Some(
                                "Usage: pylon build [app.ts | schema.ts] [--manifest-out <p>] [--client-out <p>] [--out <dir>]"
                                    .into(),
                            ),
                        }],
                        json_mode,
                    );
                    return ExitCode::Usage;
                }
            }
        }
    };
    let (manifest_json, manifest_path_for_static) =
        match run_codegen_step(&entry_file, manifest_out, client_out, no_client, json_mode) {
            Ok(v) => v,
            Err(code) => return code,
        };

    let manifest = match parse_manifest(&manifest_json, &manifest_path_for_static) {
        Ok(m) => m,
        Err(diags) => {
            print_diagnostics(&diags, json_mode);
            return ExitCode::Error;
        }
    };

    let artifact = if no_bundle {
        None
    } else {
        let step = ArtifactStep {
            entry_file: &entry_file,
            manifest_path: &manifest_path_for_static,
            manifest: &manifest,
            out_dir,
            compile,
            json_mode,
        };
        match build_artifact(&step) {
            Ok(v) => Some(v),
            Err(code) => return code,
        }
    };

    let static_dir = if artifact.is_some() {
        Path::new(out_dir)
            .join("static")
            .to_string_lossy()
            .into_owned()
    } else {
        out_dir.to_string()
    };
    let static_count = if no_static {
        0
    } else {
        match render_static_pages(&manifest, &static_dir, json_mode) {
            Ok(n) => n,
            Err(code) => return code,
        }
    };

    if json_mode {
        print_json(&BuildResult {
            code: "BUILD_OK",
            manifest: manifest_out.to_string(),
            client: if no_client {
                String::new()
            } else {
                client_out.to_string()
            },
            static_pages: static_count,
            out_dir: out_dir.to_string(),
            artifact,
        });
    } else {
        let mut summary = format!(
            "Built {} ({} entities, {} queries, {} actions)",
            manifest_out,
            manifest.entities.len(),
            manifest.queries.len(),
            manifest.actions.len(),
        );
        if !no_client {
            summary.push_str(&format!(", {client_out}"));
        }
        if let Some(a) = &artifact {
            let n = |k: &str| a.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
            summary.push_str(&format!(
                "\n  Artifact: {out_dir}/ ({} functions, {} app modules, {} client routes{})",
                n("functions"),
                n("appModules"),
                n("clientRoutes"),
                if a.get("polyfills").and_then(|v| v.as_bool()) == Some(true) {
                    ", polyfills"
                } else {
                    ""
                },
            ));
            summary.push_str(&format!("\n  Run it:   pylon start {out_dir}"));
        }
        if static_count > 0 {
            summary.push_str(&format!(", {static_count} static pages → {static_dir}/"));
        }
        print_diagnostics(
            &[Diagnostic {
                severity: Severity::Info,
                code: "BUILD_OK".into(),
                message: summary,
                span: None,
                hint: None,
            }],
            false,
        );
    }
    ExitCode::Ok
}

/// Run the codegen step (manifest + client) and return the manifest JSON +
/// the path it was written to. Bubbles errors up as the outer `Result::Err`.
fn run_codegen_step(
    entry_file: &str,
    manifest_out: &str,
    client_out: &str,
    no_client: bool,
    json_mode: bool,
) -> Result<(String, String), ExitCode> {
    if !Path::new(entry_file).exists() {
        print_diagnostics(
            &[Diagnostic {
                severity: Severity::Error,
                code: "BUILD_ENTRY_NOT_FOUND".into(),
                message: format!("Entry file not found: {entry_file}"),
                span: None,
                hint: None,
            }],
            json_mode,
        );
        return Err(ExitCode::Error);
    }

    let manifest_json = match run_bun_codegen(entry_file, true) {
        Ok(json) => json,
        Err(diag) => {
            print_diagnostics(&[diag], json_mode);
            return Err(ExitCode::Error);
        }
    };

    if let Err(code) = ensure_parent_dir(manifest_out, json_mode) {
        return Err(code);
    }
    if let Err(e) = std::fs::write(manifest_out, format!("{manifest_json}\n")) {
        print_diagnostics(
            &[Diagnostic {
                severity: Severity::Error,
                code: "BUILD_WRITE_FAILED".into(),
                message: format!("Could not write manifest to {manifest_out}: {e}"),
                span: None,
                hint: None,
            }],
            json_mode,
        );
        return Err(ExitCode::Error);
    }

    if !no_client {
        let manifest = match parse_manifest(&manifest_json, entry_file) {
            Ok(m) => m,
            Err(diags) => {
                print_diagnostics(&diags, json_mode);
                return Err(ExitCode::Error);
            }
        };
        if let Err(code) = ensure_parent_dir(client_out, json_mode) {
            return Err(code);
        }
        let client_ts = generate_client_ts(&manifest);
        if let Err(e) = std::fs::write(client_out, client_ts) {
            print_diagnostics(
                &[Diagnostic {
                    severity: Severity::Error,
                    code: "BUILD_WRITE_FAILED".into(),
                    message: format!("Could not write client bindings to {client_out}: {e}"),
                    span: None,
                    hint: None,
                }],
                json_mode,
            );
            return Err(ExitCode::Error);
        }
    }

    Ok((manifest_json, manifest_out.to_string()))
}

/// Legacy `pylon build <manifest.json>` path — loads a pre-built manifest
/// and only renders static pages from it. Kept for back-compat with the
/// old build semantics; new code should use the codegen-first path.
fn run_static_only(
    manifest: &pylon_kernel::AppManifest,
    manifest_path: &str,
    out_dir: &str,
    json_mode: bool,
    no_static: bool,
) -> ExitCode {
    let static_count = if no_static {
        0
    } else {
        match render_static_pages(manifest, out_dir, json_mode) {
            Ok(n) => n,
            Err(code) => return code,
        }
    };
    if json_mode {
        print_json(&BuildResult {
            code: "BUILD_OK",
            manifest: manifest_path.to_string(),
            client: String::new(),
            static_pages: static_count,
            out_dir: out_dir.to_string(),
            artifact: None,
        });
    } else if static_count > 0 {
        println!("Built {static_count} static pages to {out_dir}/");
    } else {
        println!("No static routes to build.");
    }
    ExitCode::Ok
}

fn render_static_pages(
    manifest: &pylon_kernel::AppManifest,
    out_dir: &str,
    json_mode: bool,
) -> Result<usize, ExitCode> {
    let pages = pylon_staticgen::generate_static_pages(manifest);
    if pages.is_empty() {
        return Ok(0);
    }
    let out_path = Path::new(out_dir);
    match pylon_staticgen::write_pages(&pages, out_path) {
        Ok(count) => Ok(count),
        Err(e) => {
            print_diagnostics(
                &[Diagnostic {
                    severity: Severity::Error,
                    code: "BUILD_WRITE_FAILED".into(),
                    message: e,
                    span: None,
                    hint: None,
                }],
                json_mode,
            );
            Err(ExitCode::Error)
        }
    }
}

fn ensure_parent_dir(path: &str, json_mode: bool) -> Result<(), ExitCode> {
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                print_diagnostics(
                    &[Diagnostic {
                        severity: Severity::Error,
                        code: "BUILD_WRITE_FAILED".into(),
                        message: format!("Could not create directory {}: {e}", parent.display()),
                        span: None,
                        hint: None,
                    }],
                    json_mode,
                );
                return Err(ExitCode::Error);
            }
        }
    }
    Ok(())
}

/// Inputs to the production artifact step.
struct ArtifactStep<'a> {
    entry_file: &'a str,
    manifest_path: &'a str,
    manifest: &'a pylon_kernel::AppManifest,
    out_dir: &'a str,
    compile: bool,
    json_mode: bool,
}

fn artifact_error(
    step: &ArtifactStep,
    code: &str,
    message: String,
    hint: Option<String>,
) -> ExitCode {
    print_diagnostics(
        &[Diagnostic {
            severity: Severity::Error,
            code: code.into(),
            message,
            span: None,
            hint,
        }],
        step.json_mode,
    );
    ExitCode::Error
}

/// Write the production artifact into `out_dir`. The server and client
/// bundles come from `@pylonsync/functions` (build-cli.ts); this adds the
/// parts that live on the Rust side: a `web/` SPA, studio files, and with
/// `--compile` the binaries. Returns the bundle step's JSON summary.
fn build_artifact(step: &ArtifactStep) -> Result<serde_json::Value, ExitCode> {
    let entry_dir = Path::new(step.entry_file)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let project_dir = entry_dir.canonicalize().map_err(|e| {
        artifact_error(
            step,
            "BUILD_PROJECT_DIR",
            format!(
                "could not resolve the project directory {}: {e}",
                entry_dir.display()
            ),
            None,
        )
    })?;
    let cwd = std::env::current_dir().map_err(|e| {
        artifact_error(
            step,
            "BUILD_PROJECT_DIR",
            format!("could not read the working directory: {e}"),
            None,
        )
    })?;
    let out_abs = cwd.join(step.out_dir);
    let manifest_abs = cwd.join(step.manifest_path);

    let Some(build_cli) = find_build_cli(&project_dir) else {
        return Err(artifact_error(
            step,
            "BUILD_FUNCTIONS_MISSING",
            "pylon build needs @pylonsync/functions, and it is not installed in this project"
                .into(),
            Some("Run: bun add @pylonsync/functions (or pass --no-bundle for codegen only)".into()),
        ));
    };

    let ssr_routes: Vec<pylon_kernel::ManifestRoute> = step
        .manifest
        .routes
        .iter()
        .filter(|r| r.mode == "ssr")
        .cloned()
        .collect();
    let app_dir = pylon_runtime::frontend::derive_app_dir(&ssr_routes);

    let output = std::process::Command::new("bun")
        .arg("run")
        .arg("--")
        .arg(&build_cli)
        .arg("--out")
        .arg(&out_abs)
        .arg("--app-dir")
        .arg(&app_dir)
        .arg("--manifest")
        .arg(&manifest_abs)
        .current_dir(&project_dir)
        .stderr(std::process::Stdio::inherit())
        .output()
        .map_err(|e| {
            artifact_error(
                step,
                "BUN_EXEC_FAILED",
                format!("Failed to execute bun: {e}"),
                Some("Ensure bun is installed and available on PATH".into()),
            )
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    // The bundler logs progress on stdout; the summary is the last line.
    let mut lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    let summary_line = lines.pop().unwrap_or("");
    if !step.json_mode {
        for l in &lines {
            eprintln!("{l}");
        }
    }
    if !output.status.success() {
        return Err(artifact_error(
            step,
            "BUILD_BUNDLE_FAILED",
            "the production bundle failed (details above)".into(),
            None,
        ));
    }
    let summary: serde_json::Value = serde_json::from_str(summary_line).map_err(|e| {
        artifact_error(
            step,
            "BUILD_BUNDLE_FAILED",
            format!("the bundle step printed no result ({e}): {summary_line}"),
            None,
        )
    })?;

    build_web_spa(step, &project_dir, &out_abs)?;

    // Studio config + extension bundle, where `pylon start <dir>` reads them.
    if crate::studio_config::locate_config(step.entry_file).is_some()
        || crate::studio_config::locate_entry(step.entry_file).is_some()
    {
        if let Err(diags) =
            crate::studio_config::build_artefacts(step.entry_file, &out_abs.join(".pylon"))
        {
            print_diagnostics(&diags, step.json_mode);
            return Err(ExitCode::Error);
        }
    }

    if step.compile {
        copy_binaries(step, &out_abs)?;
    }
    Ok(summary)
}

/// `@pylonsync/functions/src/build-cli.ts` for this project: next to the
/// runtime `PYLON_FUNCTIONS_RUNTIME` names (the runtime image), in
/// `node_modules` walking up from the project, or in the monorepo source.
fn find_build_cli(project_dir: &Path) -> Option<std::path::PathBuf> {
    if let Ok(rt) = std::env::var("PYLON_FUNCTIONS_RUNTIME") {
        if let Some(dir) = Path::new(&rt).parent() {
            let p = dir.join("build-cli.ts");
            if p.is_file() {
                return Some(p);
            }
        }
    }
    let mut dir = Some(project_dir);
    while let Some(d) = dir {
        for rel in [
            "node_modules/@pylonsync/functions/src/build-cli.ts",
            "packages/functions/src/build-cli.ts",
        ] {
            let p = d.join(rel);
            if p.is_file() {
                return Some(p);
            }
        }
        dir = d.parent();
    }
    None
}

/// Build a `web/` (or `apps/web/`) SPA and copy its `dist/` into the
/// artifact, where the runtime's frontend discovery finds it.
fn build_web_spa(step: &ArtifactStep, project_dir: &Path, out_abs: &Path) -> Result<(), ExitCode> {
    let Some(web_dir) = ["web", "apps/web"]
        .iter()
        .map(|d| project_dir.join(d))
        .find(|d| crate::bun::package_json_has_build_script(&d.join("package.json")))
    else {
        return Ok(());
    };
    let has_lock = web_dir.join("bun.lock").is_file() || web_dir.join("bun.lockb").is_file();
    let install: &[&str] = if has_lock {
        &["install", "--frozen-lockfile"]
    } else {
        &["install"]
    };
    for args in [install, &["run", "build"][..]] {
        let status = std::process::Command::new("bun")
            .args(args)
            .current_dir(&web_dir)
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .status();
        match status {
            Ok(s) if s.success() => {}
            Ok(s) => {
                return Err(artifact_error(
                    step,
                    "FRONTEND_BUILD_FAILED",
                    format!(
                        "`bun {}` in {} exited with {s}",
                        args.join(" "),
                        web_dir.display()
                    ),
                    None,
                ));
            }
            Err(e) => {
                return Err(artifact_error(
                    step,
                    "BUN_EXEC_FAILED",
                    format!("Failed to execute bun: {e}"),
                    None,
                ));
            }
        }
    }
    let dist = web_dir.join("dist");
    if !dist.join("index.html").is_file() {
        return Err(artifact_error(
            step,
            "FRONTEND_BUILD_FAILED",
            format!("{} has no index.html after `bun run build`", dist.display()),
            None,
        ));
    }
    let rel = web_dir
        .strip_prefix(project_dir)
        .unwrap_or(Path::new("web"));
    copy_dir(&dist, &out_abs.join(rel).join("dist")).map_err(|e| {
        artifact_error(
            step,
            "BUILD_WRITE_FAILED",
            format!("could not copy {}: {e}", dist.display()),
            None,
        )
    })
}

/// `--compile`: copy this `pylon` binary and the `bun` on PATH into
/// `<out>/bin/`. `pylon start <out>` puts that directory first on PATH, so
/// the artifact runs on a host with neither installed. The binaries are for
/// the build host's platform: build inside the target image for Docker.
fn copy_binaries(step: &ArtifactStep, out_abs: &Path) -> Result<(), ExitCode> {
    let bin = out_abs.join("bin");
    let fail = |e: String| artifact_error(step, "BUILD_COMPILE_FAILED", e, None);
    std::fs::create_dir_all(&bin)
        .map_err(|e| fail(format!("could not create {}: {e}", bin.display())))?;
    let exe = std::env::consts::EXE_SUFFIX;
    let pylon = std::env::current_exe()
        .map_err(|e| fail(format!("could not locate the pylon binary: {e}")))?;
    let bun = find_on_path(&format!("bun{exe}"))
        .ok_or_else(|| fail("could not find bun on PATH".into()))?;
    for (src, name) in [(pylon, format!("pylon{exe}")), (bun, format!("bun{exe}"))] {
        let dest = bin.join(&name);
        std::fs::copy(&src, &dest).map_err(|e| {
            fail(format!(
                "could not copy {} to {}: {e}",
                src.display(),
                dest.display()
            ))
        })?;
    }
    Ok(())
}

fn find_on_path(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
        .and_then(|p| p.canonicalize().ok())
}

fn copy_dir(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let to = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}
