use std::path::Path;

use pylon_kernel::{Diagnostic, ExitCode, Severity};

use crate::bun::run_bun_codegen;
use crate::output::{print_diagnostics, print_json};

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let positional: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with('-') && *a != "codegen")
        .map(|s| s.as_str())
        .collect();

    let out_path = args
        .windows(2)
        .find(|w| w[0] == "--out")
        .map(|w| w[1].as_str());

    let entry_file = match positional.first() {
        Some(f) => *f,
        None => {
            print_diagnostics(
                &[Diagnostic {
                    severity: Severity::Error,
                    code: "CODEGEN_NO_ENTRY".into(),
                    message: "No entry file provided".into(),
                    span: None,
                    hint: Some("Usage: pylon codegen <entry-file> [--out <path>]".into()),
                }],
                json_mode,
            );
            return ExitCode::Usage;
        }
    };

    if !Path::new(entry_file).exists() {
        print_diagnostics(
            &[Diagnostic {
                severity: Severity::Error,
                code: "CODEGEN_ENTRY_NOT_FOUND".into(),
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

    let manifest_json = match run_bun_codegen(entry_file) {
        Ok(json) => json,
        Err(diag) => {
            print_diagnostics(&[diag], json_mode);
            return ExitCode::Error;
        }
    };

    match out_path {
        Some(path) => {
            if let Some(parent) = Path::new(path).parent() {
                if !parent.as_os_str().is_empty() && !parent.exists() {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        print_diagnostics(
                            &[Diagnostic {
                                severity: Severity::Error,
                                code: "CODEGEN_WRITE_FAILED".into(),
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

            let contents = format!("{manifest_json}\n");
            match std::fs::write(path, &contents) {
                Ok(()) => {
                    if json_mode {
                        print_json(&serde_json::json!({
                            "code": "CODEGEN_OK",
                            "path": path
                        }));
                    } else {
                        print_diagnostics(
                            &[Diagnostic {
                                severity: Severity::Info,
                                code: "CODEGEN_OK".into(),
                                message: format!("Manifest written to {path}"),
                                span: None,
                                hint: None,
                            }],
                            false,
                        );
                    }
                    ExitCode::Ok
                }
                Err(e) => {
                    print_diagnostics(
                        &[Diagnostic {
                            severity: Severity::Error,
                            code: "CODEGEN_WRITE_FAILED".into(),
                            message: format!("Could not write manifest to {path}: {e}"),
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
            println!("{manifest_json}");
            ExitCode::Ok
        }
    }
}
