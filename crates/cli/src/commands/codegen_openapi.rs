//! `pylon codegen openapi` — write the app's OpenAPI 3.1 spec to a file.
//!
//! The same document the server serves at `GET /api/openapi.json`, produced
//! without booting anything. Two reasons to want it as a file: hosted docs
//! (Mintlify, Scalar, Redoc) that build from a checked-in artifact rather
//! than a live URL, and diffing the API surface in review — a spec in git
//! makes an accidentally-public function visible in the pull request.
//!
//! The spec is only as complete as the manifest. An app whose `app.ts`
//! passes `queries: []` / `actions: []` instead of `discoverFunctions()`
//! documents its entities and nothing else, so that case gets a warning
//! rather than a silently thin file.

use std::path::Path;

use pylon_kernel::{AppManifest, Diagnostic, ExitCode, Severity};

use crate::bun::run_bun_codegen;
use crate::commands::codegen::ensure_parent_dir;
use crate::output::print_diagnostics;

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return ExitCode::Ok;
    }

    let out = args
        .windows(2)
        .find(|w| w[0] == "--out")
        .map(|w| w[1].as_str())
        .unwrap_or("pylon.openapi.json");
    let base_url = args
        .windows(2)
        .find(|w| w[0] == "--base-url")
        .map(|w| w[1].as_str())
        .unwrap_or("/");

    let entry_file = match resolve_entry(args) {
        Ok(f) => f,
        Err(diag) => {
            print_diagnostics(&[diag], json_mode);
            return ExitCode::Usage;
        }
    };

    let manifest_json = match run_bun_codegen(&entry_file, false) {
        Ok(json) => json,
        Err(diag) => {
            print_diagnostics(&[diag], json_mode);
            return ExitCode::Error;
        }
    };
    let manifest: AppManifest = match serde_json::from_str(&manifest_json) {
        Ok(m) => m,
        Err(e) => {
            print_diagnostics(
                &[Diagnostic {
                    severity: Severity::Error,
                    code: "OPENAPI_BAD_MANIFEST".into(),
                    message: format!("Could not parse the generated manifest: {e}"),
                    span: None,
                    hint: Some("Run `pylon codegen` on its own to see the manifest error.".into()),
                }],
                json_mode,
            );
            return ExitCode::Error;
        }
    };

    let fn_count = manifest.queries.len() + manifest.actions.len();
    let entity_count = manifest.entities.len();
    let spec = pylon_runtime::openapi::generate_openapi(&manifest, base_url);
    let body = match serde_json::to_string_pretty(&spec) {
        Ok(s) => s,
        Err(e) => {
            print_diagnostics(
                &[Diagnostic {
                    severity: Severity::Error,
                    code: "OPENAPI_SERIALIZE".into(),
                    message: format!("Could not serialize the spec: {e}"),
                    span: None,
                    hint: None,
                }],
                json_mode,
            );
            return ExitCode::Error;
        }
    };

    if out == "-" {
        println!("{body}");
        return ExitCode::Ok;
    }
    if let Err(code) = ensure_parent_dir(out, json_mode) {
        return code;
    }
    if let Err(e) = std::fs::write(out, &body) {
        print_diagnostics(
            &[Diagnostic {
                severity: Severity::Error,
                code: "OPENAPI_WRITE_FAILED".into(),
                message: format!("Could not write {out}: {e}"),
                span: None,
                hint: None,
            }],
            json_mode,
        );
        return ExitCode::Error;
    }

    if json_mode {
        println!(
            "{}",
            serde_json::json!({
                "ok": true, "out": out, "entities": entity_count, "functions": fn_count,
            })
        );
    } else {
        println!("✓ {out} — {entity_count} entities, {fn_count} functions");
        // A manifest with entities but no functions is almost always an
        // app.ts still passing empty arrays, not an app with no server
        // functions. Saying so here costs one line and saves the "why is
        // my API doc empty" round trip.
        if fn_count == 0 && Path::new("functions").is_dir() {
            println!();
            println!("  ! functions/ exists but the manifest declares no functions.");
            println!("    In app.ts:  const fns = await discoverFunctions();");
            println!("                buildManifest({{ queries: fns.queries, actions: fns.actions, … }})");
        }
    }
    ExitCode::Ok
}

fn resolve_entry(args: &[String]) -> Result<String, Diagnostic> {
    let skip = ["--out", "--base-url"];
    let mut positional: Vec<&str> = Vec::new();
    let mut iter = args.iter();
    while let Some(a) = iter.next() {
        if a == "codegen" || a == "openapi" {
            continue;
        }
        if a.starts_with('-') {
            if skip.contains(&a.as_str()) {
                let _ = iter.next();
            }
            continue;
        }
        positional.push(a.as_str());
    }
    if let Some(f) = positional.first() {
        if !Path::new(f).exists() {
            return Err(Diagnostic {
                severity: Severity::Error,
                code: "CODEGEN_ENTRY_NOT_FOUND".into(),
                message: format!("Entry file not found: {f}"),
                span: None,
                hint: None,
            });
        }
        return Ok((*f).to_string());
    }
    // Same precedence as `pylon dev` and `pylon codegen`.
    for candidate in ["app.ts", "schema.ts"] {
        if Path::new(candidate).exists() {
            return Ok(candidate.to_string());
        }
    }
    Err(Diagnostic {
        severity: Severity::Error,
        code: "CODEGEN_NO_ENTRY".into(),
        message: "No entry file provided and neither app.ts nor schema.ts found here".into(),
        span: None,
        hint: Some("Usage: pylon codegen openapi [app.ts] [--out <path>]".into()),
    })
}

fn print_help() {
    println!("pylon codegen openapi — write the app's OpenAPI 3.1 spec");
    println!();
    println!("USAGE");
    println!("  pylon codegen openapi [app.ts] [--out <path>] [--base-url <url>]");
    println!();
    println!("The same document served at GET /api/openapi.json, without booting the");
    println!("server. Point Mintlify, Scalar, or Redoc at the output for rendered docs.");
    println!();
    println!("Only non-internal functions appear — internal ones aren't externally");
    println!("callable, so documenting them would describe endpoints that 404.");
    println!();
    println!("FLAGS");
    println!("  --out <path>       Output file, or - for stdout (default: pylon.openapi.json)");
    println!("  --base-url <url>   servers[0].url in the spec (default: /)");
    println!("  --json             Machine-readable result");
    println!("  -h, --help         Show this help");
}
