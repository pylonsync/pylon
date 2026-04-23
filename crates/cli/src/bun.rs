use pylon_kernel::Diagnostic;
use pylon_kernel::Severity;

use crate::manifest::parse_manifest;

/// Run a TS entry file with Bun and return the trimmed stdout as manifest JSON.
/// Validates that the output parses as a valid manifest.
pub fn run_bun_codegen(entry_file: &str) -> Result<String, Diagnostic> {
    // `--` before the entry file stops Bun from interpreting a filename
    // that starts with `-` as a flag. Without this, an attacker able to
    // smuggle a crafted filename into this call could inject flags like
    // `--eval <code>`. Bun treats everything after `--` as positional args.
    let output = std::process::Command::new("bun")
        .arg("run")
        .arg("--")
        .arg(entry_file)
        .output()
        .map_err(|e| Diagnostic {
            severity: Severity::Error,
            code: "BUN_EXEC_FAILED".into(),
            message: format!("Failed to execute bun: {e}"),
            span: None,
            hint: Some("Ensure bun is installed and available on PATH".into()),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if !stderr.is_empty() {
            stderr.trim().to_string()
        } else {
            stdout.trim().to_string()
        };
        return Err(Diagnostic {
            severity: Severity::Error,
            code: "BUN_EXIT".into(),
            message: format!(
                "bun run {entry_file} exited with status {}",
                output.status.code().unwrap_or(-1)
            ),
            span: None,
            hint: if detail.is_empty() { None } else { Some(detail) },
        });
    }

    let manifest_json = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Validate it parses as a manifest.
    parse_manifest(&manifest_json, entry_file).map_err(|diags| Diagnostic {
        severity: Severity::Error,
        code: "BUN_INVALID_OUTPUT".into(),
        message: "Output is not a valid manifest".into(),
        span: None,
        hint: diags.first().map(|d| d.message.clone()),
    })?;

    Ok(manifest_json)
}
