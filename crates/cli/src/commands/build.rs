use agentdb_core::{Diagnostic, ExitCode, Severity};
use serde::Serialize;

use crate::manifest::load_manifest;
use crate::output::{print_diagnostics, print_json};

#[derive(Serialize)]
struct BuildResult {
    code: &'static str,
    pages: usize,
    out_dir: String,
}

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let out_dir = args
        .windows(2)
        .find(|w| w[0] == "--out")
        .map(|w| w[1].as_str())
        .unwrap_or("dist");

    let positional: Vec<&str> = args
        .iter()
        .filter(|a| {
            !a.starts_with('-') && *a != "build" && Some(a.as_str()) != Some(out_dir)
        })
        .map(|s| s.as_str())
        .collect();

    let manifest_path = positional.first().copied().unwrap_or("agentdb.manifest.json");

    let manifest = match load_manifest(manifest_path) {
        Ok(m) => m,
        Err(diags) => {
            print_diagnostics(&diags, json_mode);
            return ExitCode::Error;
        }
    };

    let pages = agentdb_staticgen::generate_static_pages(&manifest);

    if pages.is_empty() {
        if json_mode {
            print_json(&BuildResult {
                code: "BUILD_NO_PAGES",
                pages: 0,
                out_dir: out_dir.to_string(),
            });
        } else {
            println!("No static routes to build.");
            println!("  Only routes with mode \"static\" are rendered.");
        }
        return ExitCode::Ok;
    }

    let out_path = std::path::Path::new(out_dir);
    match agentdb_staticgen::write_pages(&pages, out_path) {
        Ok(count) => {
            if json_mode {
                print_json(&BuildResult {
                    code: "BUILD_OK",
                    pages: count,
                    out_dir: out_dir.to_string(),
                });
            } else {
                println!("Built {} static pages to {}/", count, out_dir);
                for page in &pages {
                    println!("  {}", page.path);
                }
            }
            ExitCode::Ok
        }
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
            ExitCode::Error
        }
    }
}
