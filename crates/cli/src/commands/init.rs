use std::path::Path;

use agentdb_core::{Diagnostic, ExitCode, Severity};
use serde::Serialize;

use crate::bun::run_bun_codegen;
use crate::client_codegen::generate_client_ts;
use crate::manifest::parse_manifest;
use crate::output::{print_diagnostics, print_json};

const TEMPLATE_BASIC_APP: &str = include_str!("../../../../templates/basic/app.ts");
const TEMPLATE_BASIC_TSCONFIG: &str = include_str!("../../../../templates/basic/tsconfig.json");
const SDK_SOURCE: &str = include_str!("../../../../packages/sdk/src/index.ts");

#[derive(Serialize)]
struct InitOutput {
    code: &'static str,
    name: String,
    path: String,
    template: String,
    files: Vec<String>,
}

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let flag_values: std::collections::HashSet<&str> = args
        .windows(2)
        .filter(|w| w[0] == "--template")
        .map(|w| w[1].as_str())
        .collect();

    let positional: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with('-') && *a != "init" && !flag_values.contains(a.as_str()))
        .map(|s| s.as_str())
        .collect();

    let template = args
        .windows(2)
        .find(|w| w[0] == "--template")
        .map(|w| w[1].as_str())
        .unwrap_or("basic");

    let target_arg = match positional.first() {
        Some(n) => *n,
        None => {
            print_diagnostics(
                &[Diagnostic {
                    severity: Severity::Error,
                    code: "INIT_NO_PATH".into(),
                    message: "No target path provided".into(),
                    span: None,
                    hint: Some("Usage: agentdb init <path> [--template basic]".into()),
                }],
                json_mode,
            );
            return ExitCode::Usage;
        }
    };

    let target = Path::new(target_arg);

    let app_name = match target.file_name().and_then(|n| n.to_str()) {
        Some(n) if !n.is_empty() && !n.starts_with('.') => n.to_string(),
        _ => {
            print_diagnostics(
                &[Diagnostic {
                    severity: Severity::Error,
                    code: "INIT_INVALID_NAME".into(),
                    message: format!(
                        "Could not derive a valid app name from path: \"{}\"",
                        target.display()
                    ),
                    span: None,
                    hint: Some(
                        "The final path segment must be a valid name (non-empty, no leading dot)"
                            .into(),
                    ),
                }],
                json_mode,
            );
            return ExitCode::Usage;
        }
    };

    if template != "basic" {
        print_diagnostics(
            &[Diagnostic {
                severity: Severity::Error,
                code: "INIT_UNKNOWN_TEMPLATE".into(),
                message: format!("Unknown template: \"{template}\""),
                span: None,
                hint: Some("Available templates: basic".into()),
            }],
            json_mode,
        );
        return ExitCode::Usage;
    }

    if target.exists() {
        let is_empty = match std::fs::read_dir(target) {
            Ok(mut entries) => entries.next().is_none(),
            Err(_) => false,
        };
        if !is_empty {
            print_diagnostics(
                &[Diagnostic {
                    severity: Severity::Error,
                    code: "INIT_DIR_EXISTS".into(),
                    message: format!(
                        "Directory \"{}\" already exists and is not empty",
                        target.display()
                    ),
                    span: None,
                    hint: Some("Choose a different path or remove the existing directory".into()),
                }],
                json_mode,
            );
            return ExitCode::Error;
        }
    }

    if let Err(e) = std::fs::create_dir_all(target) {
        print_diagnostics(
            &[Diagnostic {
                severity: Severity::Error,
                code: "INIT_MKDIR_FAILED".into(),
                message: format!("Could not create directory \"{}\": {e}", target.display()),
                span: None,
                hint: None,
            }],
            json_mode,
        );
        return ExitCode::Error;
    }

    let app_ts = TEMPLATE_BASIC_APP.replace("__APP_NAME__", &app_name);
    let package_json = serde_json::to_string_pretty(&serde_json::json!({
        "name": app_name,
        "version": "0.1.0",
        "private": true,
        "type": "module",
        "scripts": {
            "codegen": "agentdb codegen app.ts --out agentdb.manifest.json",
            "doctor": "agentdb doctor agentdb.manifest.json",
            "check": "tsc -p tsconfig.json --noEmit"
        }
    }))
    .unwrap()
        + "\n";

    let files: &[(&str, &str)] = &[
        ("sdk.ts", SDK_SOURCE),
        ("app.ts", &app_ts),
        ("tsconfig.json", TEMPLATE_BASIC_TSCONFIG),
        ("package.json", &package_json),
    ];

    for (name, contents) in files {
        let file_path = target.join(name);
        if let Err(e) = std::fs::write(&file_path, contents) {
            print_diagnostics(
                &[Diagnostic {
                    severity: Severity::Error,
                    code: "INIT_WRITE_FAILED".into(),
                    message: format!("Could not write {}: {e}", file_path.display()),
                    span: None,
                    hint: None,
                }],
                json_mode,
            );
            return ExitCode::Error;
        }
    }

    let entry_path = target.join("app.ts");
    let manifest_path = target.join("agentdb.manifest.json");
    let entry_str = entry_path.to_string_lossy().to_string();

    match run_bun_codegen(&entry_str) {
        Ok(manifest_json) => {
            let contents = format!("{manifest_json}\n");
            if let Err(e) = std::fs::write(&manifest_path, &contents) {
                print_diagnostics(
                    &[Diagnostic {
                        severity: Severity::Warning,
                        code: "INIT_CODEGEN_WRITE_FAILED".into(),
                        message: format!("Files created but could not write manifest: {e}"),
                        span: None,
                        hint: Some(
                            "Run 'agentdb codegen app.ts --out agentdb.manifest.json' manually"
                                .into(),
                        ),
                    }],
                    json_mode,
                );
            }

            // Generate client bindings.
            if let Ok(manifest) = parse_manifest(&manifest_json, &entry_str) {
                let client_ts = generate_client_ts(&manifest);
                let client_path = target.join("agentdb.client.ts");
                let _ = std::fs::write(&client_path, client_ts);
            }
        }
        Err(diag) => {
            print_diagnostics(
                &[Diagnostic {
                    severity: Severity::Warning,
                    code: "INIT_CODEGEN_FAILED".into(),
                    message: format!(
                        "Files created but manifest generation failed: {}",
                        diag.message
                    ),
                    span: None,
                    hint: Some(
                        "Run 'agentdb codegen app.ts --out agentdb.manifest.json' manually".into(),
                    ),
                }],
                json_mode,
            );
        }
    }

    let target_display = target.display().to_string();
    let mut file_list: Vec<String> = files
        .iter()
        .map(|(name, _)| format!("{target_display}/{name}"))
        .collect();
    if manifest_path.exists() {
        file_list.push(format!("{target_display}/agentdb.manifest.json"));
    }
    let client_path = target.join("agentdb.client.ts");
    if client_path.exists() {
        file_list.push(format!("{target_display}/agentdb.client.ts"));
    }

    if json_mode {
        print_json(&InitOutput {
            code: "INIT_OK",
            name: app_name.clone(),
            path: target_display.clone(),
            template: template.to_string(),
            files: file_list,
        });
    } else {
        println!("Created {target_display}/");
        for (name, _) in files {
            println!("  {name}");
        }
        if manifest_path.exists() {
            println!("  agentdb.manifest.json");
        }
        if client_path.exists() {
            println!("  agentdb.client.ts");
        }
        println!();
        println!("Next steps:");
        println!("  cd {target_display}");
        println!("  agentdb doctor agentdb.manifest.json");
        println!("  agentdb explain agentdb.manifest.json");
    }

    ExitCode::Ok
}
