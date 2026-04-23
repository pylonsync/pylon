//! `pylon test` — run tests against functions in an in-memory runtime.
//!
//! Discovers `*.test.ts` files in the `functions/` and `tests/` directories,
//! spawns Bun with the test runner, runs against an in-memory pylon,
//! and reports results.

use std::path::Path;
use std::process::Command;

use pylon_kernel::ExitCode;

use crate::output;

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let filter: Option<&str> = args
        .iter()
        .skip(1)
        .find(|a| !a.starts_with('-'))
        .map(|s| s.as_str());

    let test_dir = std::env::var("PYLON_TEST_DIR").ok().unwrap_or_else(|| {
        if Path::new("tests").exists() {
            "tests".into()
        } else {
            "functions".into()
        }
    });

    if !Path::new(&test_dir).exists() {
        output::print_error(&format!("No test directory found: {test_dir}"));
        eprintln!("Create a `tests/` or `functions/` directory with `*.test.ts` files.");
        return ExitCode::Error;
    }

    // Find test files.
    let test_files = discover_test_files(&test_dir);
    if test_files.is_empty() {
        if json_mode {
            output::print_json(&serde_json::json!({
                "total": 0,
                "passed": 0,
                "failed": 0,
                "files": [],
            }));
        } else {
            eprintln!("No test files found in {test_dir}/");
        }
        return ExitCode::Ok;
    }

    let filtered: Vec<&String> = match filter {
        Some(f) => test_files.iter().filter(|p| p.contains(f)).collect(),
        None => test_files.iter().collect(),
    };

    if filtered.is_empty() {
        eprintln!("No test files match filter: {:?}", filter);
        return ExitCode::Ok;
    }

    // Run bun test on each file.
    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut results: Vec<serde_json::Value> = Vec::new();

    for file in &filtered {
        if !json_mode {
            eprintln!("  running: {file}");
        }
        // `--` separates Bun options from the positional file path so a
        // filename starting with `-` isn't parsed as a flag.
        let status = Command::new("bun")
            .args(["test", "--", file])
            .env("PYLON_IN_MEMORY", "1")
            .env("PYLON_DEV_MODE", "1")
            .status();

        match status {
            Ok(s) if s.success() => {
                passed += 1;
                results.push(serde_json::json!({"file": file, "status": "passed"}));
            }
            Ok(s) => {
                failed += 1;
                results.push(serde_json::json!({
                    "file": file,
                    "status": "failed",
                    "exit_code": s.code(),
                }));
            }
            Err(e) => {
                failed += 1;
                results.push(serde_json::json!({
                    "file": file,
                    "status": "error",
                    "error": e.to_string(),
                }));
                if !json_mode {
                    eprintln!("    error: could not run bun (is it installed?): {e}");
                }
            }
        }
    }

    let total = passed + failed;
    if json_mode {
        output::print_json(&serde_json::json!({
            "total": total,
            "passed": passed,
            "failed": failed,
            "files": results,
        }));
    } else {
        eprintln!();
        eprintln!("Tests: {total} total, {passed} passed, {failed} failed");
    }

    if failed > 0 {
        ExitCode::Error
    } else {
        ExitCode::Ok
    }
}

fn discover_test_files(dir: &str) -> Vec<String> {
    let mut out = Vec::new();
    let path = Path::new(dir);
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.ends_with(".test.ts") || name.ends_with(".test.js") {
                    if let Some(s) = p.to_str() {
                        out.push(s.to_string());
                    }
                }
            } else if p.is_dir() {
                if let Some(s) = p.to_str() {
                    out.extend(discover_test_files(s));
                }
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_test_files() {
        let dir = std::env::temp_dir().join(format!("pylon_test_discover_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("a.test.ts"), "// test").unwrap();
        std::fs::write(dir.join("b.ts"), "// not a test").unwrap();
        std::fs::write(dir.join("c.test.js"), "// test").unwrap();

        let files = discover_test_files(dir.to_str().unwrap());
        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|f| f.ends_with("a.test.ts")));
        assert!(files.iter().any(|f| f.ends_with("c.test.js")));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
