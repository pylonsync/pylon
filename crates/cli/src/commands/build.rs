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
}

/// `pylon build` — production build. One command, no flags required.
///
/// Equivalent to next.js's `next build`: produces every artifact a
/// production deploy needs from the source schema. Specifically:
///
///   1. Run `bun run app.ts` (or `schema.ts`) to evaluate the manifest.
///   2. Write `pylon.manifest.json` (the canonical manifest).
///   3. Write `pylon.client.ts` (TS client bindings — what the web /
///      mobile app imports for typed query/action access).
///   4. If any routes are declared static, render them to `dist/`.
///
/// All four steps live behind one command because that's the universal
/// flow — there's no project that wants step (2) without (3), or wants
/// to re-render static pages without first regenerating the manifest.
/// Override the defaults with --manifest-out / --client-out / --out
/// when needed; otherwise the conventional paths Just Work.
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
    let codegen_result = if let Some(p) = entry_or_manifest {
        if p.ends_with(".json") {
            // Legacy mode: load existing manifest, no codegen.
            let manifest = match load_manifest(p) {
                Ok(m) => m,
                Err(diags) => {
                    print_diagnostics(&diags, json_mode);
                    return ExitCode::Error;
                }
            };
            return run_static_only(&manifest, p, out_dir, json_mode, no_static);
        } else {
            run_codegen_step(p, manifest_out, client_out, no_client, json_mode)
        }
    } else {
        // No positional → auto-discover app.ts / schema.ts.
        const CANDIDATES: &[&str] = &["app.ts", "schema.ts"];
        match CANDIDATES.iter().find(|c| Path::new(c).exists()) {
            Some(f) => run_codegen_step(f, manifest_out, client_out, no_client, json_mode),
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
    };
    let (manifest_json, manifest_path_for_static) = match codegen_result {
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

    let static_count = if no_static {
        0
    } else {
        match render_static_pages(&manifest, out_dir, json_mode) {
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
        if static_count > 0 {
            summary.push_str(&format!(", {static_count} static pages → {out_dir}/"));
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

    let manifest_json = match run_bun_codegen(entry_file) {
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
