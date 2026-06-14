use std::path::Path;

use pylon_kernel::{Diagnostic, ExitCode, Severity};

use crate::bun::run_bun_codegen;
use crate::client_codegen::generate_client_ts;
use crate::manifest::parse_manifest;
use crate::output::{print_diagnostics, print_json};

/// `pylon codegen` (no subcommand) generates BOTH the manifest JSON
/// and the TypeScript client bindings in a single shot. This is what
/// `pylon dev` does on every reload, exposed as a one-shot for
/// production builds.
///
/// Outputs (default — paths can be overridden with --manifest-out /
/// --client-out / --no-client):
///   - `pylon.manifest.json`  (manifest JSON)
///   - `pylon.client.ts`      (TS client bindings)
///
/// `pylon codegen client` is still a separate subcommand for the rare
/// case where the manifest already exists and only the client needs
/// regenerating (e.g. CI generating Swift bindings from a checked-in
/// manifest). 99% of projects just want `pylon codegen` and shouldn't
/// have to know the two-step pipeline exists.
pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let manifest_out = args
        .windows(2)
        .find(|w| w[0] == "--manifest-out" || w[0] == "--out")
        .map(|w| w[1].as_str());
    let client_out = args
        .windows(2)
        .find(|w| w[0] == "--client-out")
        .map(|w| w[1].as_str());
    let no_client = args.iter().any(|a| a == "--no-client");

    // Filter positional args, excluding flag values that follow them.
    // The flag values themselves are strings that don't start with `-`,
    // so without filtering they'd be mistaken for the entry file.
    let positional: Vec<&str> = args
        .iter()
        .filter(|a| {
            !a.starts_with('-')
                && *a != "codegen"
                && Some(a.as_str()) != manifest_out
                && Some(a.as_str()) != client_out
        })
        .map(|s| s.as_str())
        .collect();

    // Auto-discover the entry file when no positional arg is given.
    // Same precedence as `pylon dev`: app.ts first, schema.ts as a
    // fallback. Both are conventional Pylon entry names.
    let entry_file_owned: String;
    let entry_file: &str = match positional.first() {
        Some(f) => *f,
        None => {
            const CANDIDATES: &[&str] = &["app.ts", "schema.ts"];
            match CANDIDATES.iter().find(|c| Path::new(c).exists()) {
                Some(f) => {
                    entry_file_owned = (*f).to_string();
                    entry_file_owned.as_str()
                }
                None => {
                    print_diagnostics(
                        &[Diagnostic {
                            severity: Severity::Error,
                            code: "CODEGEN_NO_ENTRY".into(),
                            message:
                                "No entry file provided and neither app.ts nor schema.ts found in current directory"
                                    .into(),
                            span: None,
                            hint: Some(
                                "Usage: pylon codegen [app.ts | schema.ts] [--manifest-out <path>] [--client-out <path>] [--no-client]"
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

    // Resolve output paths. Pass `--manifest-out -` to print manifest
    // JSON to stdout (and skip the client write) for piping.
    let manifest_target: ManifestTarget = match manifest_out {
        Some("-") => ManifestTarget::Stdout,
        Some(p) => ManifestTarget::File(p.to_string()),
        None => ManifestTarget::File("pylon.manifest.json".to_string()),
    };
    let client_path = client_out.unwrap_or("pylon.client.ts").to_string();

    let manifest_json = match run_bun_codegen(entry_file, false) {
        Ok(json) => json,
        Err(diag) => {
            print_diagnostics(&[diag], json_mode);
            return ExitCode::Error;
        }
    };

    // Stdout mode: dump JSON, skip the file writes entirely.
    if matches!(manifest_target, ManifestTarget::Stdout) {
        println!("{manifest_json}");
        return ExitCode::Ok;
    }

    // Write the manifest first; then parse + generate client.
    let manifest_path = match &manifest_target {
        ManifestTarget::File(p) => p.clone(),
        ManifestTarget::Stdout => unreachable!(),
    };
    if let Err(code) = ensure_parent_dir(&manifest_path, json_mode) {
        return code;
    }
    if let Err(e) = std::fs::write(&manifest_path, format!("{manifest_json}\n")) {
        print_diagnostics(
            &[Diagnostic {
                severity: Severity::Error,
                code: "CODEGEN_WRITE_FAILED".into(),
                message: format!("Could not write manifest to {manifest_path}: {e}"),
                span: None,
                hint: None,
            }],
            json_mode,
        );
        return ExitCode::Error;
    }

    if no_client {
        if json_mode {
            print_json(&serde_json::json!({
                "code": "CODEGEN_OK",
                "manifest": manifest_path,
                "client": serde_json::Value::Null,
            }));
        } else {
            print_diagnostics(
                &[Diagnostic {
                    severity: Severity::Info,
                    code: "CODEGEN_OK".into(),
                    message: format!("Manifest written to {manifest_path}"),
                    span: None,
                    hint: None,
                }],
                false,
            );
        }
        return ExitCode::Ok;
    }

    // Parse the manifest we just wrote so we can generate client
    // bindings from it. Parse failures here are extraordinary (the
    // manifest just emerged from buildManifest()) so they map to a
    // hard error rather than a warning.
    let manifest = match parse_manifest(&manifest_json, entry_file) {
        Ok(m) => m,
        Err(diags) => {
            print_diagnostics(&diags, json_mode);
            return ExitCode::Error;
        }
    };

    if let Err(code) = ensure_parent_dir(&client_path, json_mode) {
        return code;
    }
    let client_ts = generate_client_ts(&manifest);
    if let Err(e) = std::fs::write(&client_path, client_ts) {
        print_diagnostics(
            &[Diagnostic {
                severity: Severity::Error,
                code: "CODEGEN_WRITE_FAILED".into(),
                message: format!("Could not write client bindings to {client_path}: {e}"),
                span: None,
                hint: None,
            }],
            json_mode,
        );
        return ExitCode::Error;
    }

    if json_mode {
        print_json(&serde_json::json!({
            "code": "CODEGEN_OK",
            "manifest": manifest_path,
            "client": client_path,
            "entities": manifest.entities.len(),
            "queries": manifest.queries.len(),
            "actions": manifest.actions.len(),
        }));
    } else {
        print_diagnostics(
            &[Diagnostic {
                severity: Severity::Info,
                code: "CODEGEN_OK".into(),
                message: format!(
                    "Wrote {manifest_path} + {client_path} ({} entities, {} queries, {} actions)",
                    manifest.entities.len(),
                    manifest.queries.len(),
                    manifest.actions.len(),
                ),
                span: None,
                hint: None,
            }],
            false,
        );
    }
    ExitCode::Ok
}

enum ManifestTarget {
    File(String),
    Stdout,
}

fn ensure_parent_dir(path: &str, json_mode: bool) -> Result<(), ExitCode> {
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                print_diagnostics(
                    &[Diagnostic {
                        severity: Severity::Error,
                        code: "CODEGEN_WRITE_FAILED".into(),
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
