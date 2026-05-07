use std::path::Path;

use pylon_kernel::{Diagnostic, ExitCode, Severity};

use crate::client_codegen::generate_client_ts;
use crate::manifest::load_manifest;
use crate::output::{print_diagnostics, print_json};
use crate::swift_codegen::generate_client_swift;

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let out_path = args
        .windows(2)
        .find(|w| w[0] == "--out")
        .map(|w| w[1].as_str());

    let target = args
        .windows(2)
        .find(|w| w[0] == "--target")
        .map(|w| w[1].as_str())
        .unwrap_or("ts");

    let positional: Vec<&str> = args
        .iter()
        .filter(|a| {
            !a.starts_with('-')
                && *a != "codegen"
                && *a != "client"
                && Some(a.as_str()) != out_path
                && Some(a.as_str()) != Some(target)
        })
        .map(|s| s.as_str())
        .collect();

    // Default to `pylon.manifest.json` (matches `pylon codegen`'s
    // default output path). Removes the need for projects to spell out
    // `pylon codegen client pylon.manifest.json` in package.json.
    let manifest_path = positional.first().copied().unwrap_or("pylon.manifest.json");
    if !Path::new(manifest_path).exists() {
        print_diagnostics(
            &[Diagnostic {
                severity: Severity::Error,
                code: "CODEGEN_CLIENT_NO_MANIFEST".into(),
                message: format!("Manifest not found: {manifest_path}"),
                span: None,
                hint: Some(
                    "Run `pylon codegen` first to generate pylon.manifest.json, or pass an explicit path."
                        .into(),
                ),
            }],
            json_mode,
        );
        return ExitCode::Usage;
    }

    let manifest = match load_manifest(manifest_path) {
        Ok(m) => m,
        Err(diags) => {
            print_diagnostics(&diags, json_mode);
            return ExitCode::Error;
        }
    };

    let ts_content = match target {
        "ts" | "typescript" => generate_client_ts(&manifest),
        "swift" => generate_client_swift(&manifest),
        other => {
            print_diagnostics(
                &[Diagnostic {
                    severity: Severity::Error,
                    code: "CODEGEN_CLIENT_BAD_TARGET".into(),
                    message: format!("Unknown codegen target: {other}"),
                    span: None,
                    hint: Some("Valid targets: ts, swift".into()),
                }],
                json_mode,
            );
            return ExitCode::Usage;
        }
    };

    // Default to `pylon.client.ts` / `pylon.client.swift` based on
    // target. Same rationale as `pylon codegen`: every project ends up
    // writing the same `--out pylon.client.ts` flag, so make it the
    // default. Pass `--out -` to print to stdout.
    let default_out = match target {
        "swift" => "pylon.client.swift",
        _ => "pylon.client.ts",
    };
    let out_path: Option<&str> = match out_path {
        Some("-") => None,
        Some(p) => Some(p),
        None => Some(default_out),
    };

    match out_path {
        Some(path) => {
            if let Some(parent) = Path::new(path).parent() {
                if !parent.as_os_str().is_empty() && !parent.exists() {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        print_diagnostics(
                            &[Diagnostic {
                                severity: Severity::Error,
                                code: "CODEGEN_CLIENT_WRITE_FAILED".into(),
                                message: format!(
                                    "Could not create directory {}: {e}",
                                    parent.display()
                                ),
                                span: None,
                                hint: None,
                            }],
                            json_mode,
                        );
                        return ExitCode::Error;
                    }
                }
            }

            match std::fs::write(path, &ts_content) {
                Ok(()) => {
                    if json_mode {
                        print_json(&serde_json::json!({
                            "code": "CODEGEN_CLIENT_OK",
                            "path": path,
                            "entities": manifest.entities.len(),
                            "queries": manifest.queries.len(),
                            "actions": manifest.actions.len(),
                        }));
                    } else {
                        println!(
                            "Generated client bindings: {} ({} entities, {} queries, {} actions)",
                            path,
                            manifest.entities.len(),
                            manifest.queries.len(),
                            manifest.actions.len(),
                        );
                    }
                    ExitCode::Ok
                }
                Err(e) => {
                    print_diagnostics(
                        &[Diagnostic {
                            severity: Severity::Error,
                            code: "CODEGEN_CLIENT_WRITE_FAILED".into(),
                            message: format!("Could not write client bindings to {path}: {e}"),
                            span: None,
                            hint: None,
                        }],
                        json_mode,
                    );
                    ExitCode::Error
                }
            }
        }
        None => {
            // Print to stdout.
            print!("{ts_content}");
            ExitCode::Ok
        }
    }
}
