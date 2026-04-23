use std::path::Path;

use pylon_kernel::{Diagnostic, ExitCode, Severity};

use crate::client_codegen::generate_client_ts;
use crate::manifest::load_manifest;
use crate::output::{print_diagnostics, print_json};

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let out_path = args
        .windows(2)
        .find(|w| w[0] == "--out")
        .map(|w| w[1].as_str());

    let positional: Vec<&str> = args
        .iter()
        .filter(|a| {
            !a.starts_with('-')
                && *a != "codegen"
                && *a != "client"
                && Some(a.as_str()) != out_path
        })
        .map(|s| s.as_str())
        .collect();

    let manifest_path = match positional.first() {
        Some(p) => *p,
        None => {
            print_diagnostics(
                &[Diagnostic {
                    severity: Severity::Error,
                    code: "CODEGEN_CLIENT_NO_MANIFEST".into(),
                    message: "No manifest path provided".into(),
                    span: None,
                    hint: Some(
                        "Usage: pylon codegen client <manifest> --out <path>".into(),
                    ),
                }],
                json_mode,
            );
            return ExitCode::Usage;
        }
    };

    let manifest = match load_manifest(manifest_path) {
        Ok(m) => m,
        Err(diags) => {
            print_diagnostics(&diags, json_mode);
            return ExitCode::Error;
        }
    };

    let ts_content = generate_client_ts(&manifest);

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
