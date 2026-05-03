//! Build pipeline for `studio.config.ts` and `studio.entry.tsx`.
//!
//! Both files live in the user's pylon project, alongside `app.ts`. They
//! are optional — projects with neither still get a fully-functional
//! Studio (auto-derived sidebar, default Used Space footer, emerald
//! theme).
//!
//!   `studio.config.ts`
//!     Pure declarative config (`defineStudioConfig`). We run it via bun
//!     using a one-line `--print` shim that imports the user file and
//!     emits its default export as JSON. The result is parsed against
//!     `pylon_kernel::StudioConfig` for type-safety, then written to
//!     `.pylon/studio.config.json`. If parse fails, we surface the error
//!     as a structured `Diagnostic`.
//!
//!   `studio.entry.tsx`
//!     React extensions (custom column renderers, custom pages, custom
//!     layout slots). We bundle it via `bun build` to a single ESM file
//!     at `.pylon/studio.entry.js`, with `react` / `react-dom` /
//!     `react/jsx-runtime` marked external — the Studio HTML provides
//!     them via an import map. The runtime serves the bundle at
//!     `/studio/extensions.js`.

use std::path::{Path, PathBuf};

use pylon_kernel::{Diagnostic, Severity, StudioConfig};

/// File names in the user project root, relative to wherever `app.ts` lives.
pub const STUDIO_CONFIG_TS: &str = "studio.config.ts";
pub const STUDIO_ENTRY_TSX: &str = "studio.entry.tsx";

/// Output paths inside the project's `.pylon/` data dir.
pub const STUDIO_CONFIG_OUT: &str = "studio.config.json";
pub const STUDIO_ENTRY_OUT: &str = "studio.entry.js";

/// Resolve `studio.config.ts` next to `entry_file`. Returns `None` if it
/// doesn't exist — that's a valid state.
pub fn locate_config(entry_file: &str) -> Option<PathBuf> {
    let dir = Path::new(entry_file).parent().unwrap_or(Path::new("."));
    let p = dir.join(STUDIO_CONFIG_TS);
    p.exists().then_some(p)
}

/// Resolve `studio.entry.tsx` next to `entry_file`.
pub fn locate_entry(entry_file: &str) -> Option<PathBuf> {
    let dir = Path::new(entry_file).parent().unwrap_or(Path::new("."));
    let p = dir.join(STUDIO_ENTRY_TSX);
    p.exists().then_some(p)
}

/// Run the user's `studio.config.ts` through bun and parse the output as
/// a [`StudioConfig`]. Wraps any failure in a structured diagnostic so
/// the dev loop can surface it without panicking.
pub fn load_config(config_path: &Path) -> Result<StudioConfig, Diagnostic> {
    let abs = match config_path.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            return Err(Diagnostic {
                severity: Severity::Error,
                code: "STUDIO_CONFIG_PATH".into(),
                message: format!("Could not resolve {}: {e}", config_path.display()),
                span: None,
                hint: None,
            });
        }
    };

    // Bun can `import()` a `.ts` file directly. The shim is a one-liner
    // whose *value* is the JSON string we want — `--print` then writes
    // exactly that to stdout (trailing newline stripped on the Rust
    // side). The `file://` URL avoids any cwd-relative resolution
    // surprises across editors / CIs.
    //
    // Don't use `process.stdout.write` inside `--print`: bun will write
    // the returned value of the whole expression *in addition* to
    // anything you wrote yourself, doubling the output and breaking the
    // JSON parse on the Rust side.
    let url = format!("file://{}", abs.display());
    let shim = format!(
        "JSON.stringify((await import({json_url})).default ?? {{}})",
        json_url = serde_json::to_string(&url).unwrap()
    );

    let output = std::process::Command::new("bun")
        .arg("--print")
        .arg(&shim)
        .output()
        .map_err(|e| Diagnostic {
            severity: Severity::Error,
            code: "BUN_EXEC_FAILED".into(),
            message: format!("Failed to execute bun for studio.config.ts: {e}"),
            span: None,
            hint: Some("Ensure bun is installed and available on PATH".into()),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        return Err(Diagnostic {
            severity: Severity::Error,
            code: "STUDIO_CONFIG_BUN_EXIT".into(),
            message: format!(
                "studio.config.ts failed under bun (status {})",
                output.status.code().unwrap_or(-1)
            ),
            span: None,
            hint: if detail.is_empty() {
                None
            } else {
                Some(detail)
            },
        });
    }

    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if raw.is_empty() {
        // The user wrote `studio.config.ts` but didn't `export default`
        // their config. Treat this as an empty config — the Studio
        // falls back to defaults and we don't fail the build.
        return Ok(StudioConfig::default());
    }

    serde_json::from_str(&raw).map_err(|e| Diagnostic {
        severity: Severity::Error,
        code: "STUDIO_CONFIG_PARSE".into(),
        message: format!("Invalid studio.config.ts output: {e}"),
        span: Some(pylon_kernel::Span {
            file: config_path.display().to_string(),
            line: None,
            column: None,
        }),
        hint: Some(
            "Make sure your file `export default defineStudioConfig({...})` from @pylon/sdk"
                .into(),
        ),
    })
}

/// Bundle `studio.entry.tsx` to a single ESM file via `bun build`.
/// `react`, `react-dom`, and `react/jsx-runtime` are marked external —
/// the Studio HTML provides them through an import map.
pub fn bundle_entry(entry_path: &Path, out_file: &Path) -> Result<(), Diagnostic> {
    if let Some(parent) = out_file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Diagnostic {
            severity: Severity::Error,
            code: "STUDIO_ENTRY_OUTDIR".into(),
            message: format!("Could not create {}: {e}", parent.display()),
            span: None,
            hint: None,
        })?;
    }

    let output = std::process::Command::new("bun")
        .arg("build")
        .arg(entry_path)
        .arg("--target=browser")
        .arg("--format=esm")
        .arg("--external=react")
        .arg("--external=react-dom")
        .arg("--external=react/jsx-runtime")
        .arg("--external=react-dom/client")
        .arg("--outfile")
        .arg(out_file)
        .output()
        .map_err(|e| Diagnostic {
            severity: Severity::Error,
            code: "BUN_BUILD_EXEC".into(),
            message: format!("Failed to execute `bun build` for studio.entry.tsx: {e}"),
            span: None,
            hint: Some("Ensure bun is installed and available on PATH".into()),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        return Err(Diagnostic {
            severity: Severity::Error,
            code: "STUDIO_ENTRY_BUILD".into(),
            message: format!(
                "`bun build studio.entry.tsx` failed (status {})",
                output.status.code().unwrap_or(-1)
            ),
            span: Some(pylon_kernel::Span {
                file: entry_path.display().to_string(),
                line: None,
                column: None,
            }),
            hint: if detail.is_empty() {
                None
            } else {
                Some(detail)
            },
        });
    }

    Ok(())
}

/// Build both studio artefacts (config JSON + entry bundle) for a
/// project rooted at the directory containing `entry_file`.
///
/// Writes `.pylon/studio.config.json` (or removes it if no
/// studio.config.ts) and `.pylon/studio.entry.js` (or removes it if no
/// studio.entry.tsx). Returns the loaded config — callers serve this
/// directly when starting the runtime, avoiding a re-read.
///
/// On error, leaves any pre-existing artefacts untouched so the running
/// dev server keeps the last good config.
pub fn build_artefacts(
    entry_file: &str,
    data_dir: &Path,
) -> Result<StudioConfig, Vec<Diagnostic>> {
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut config = StudioConfig::default();

    // ---- studio.config.ts ----------------------------------------------
    if let Some(cfg_path) = locate_config(entry_file) {
        match load_config(&cfg_path) {
            Ok(cfg) => {
                config = cfg;
            }
            Err(d) => {
                diagnostics.push(d);
            }
        }
    }

    // ---- studio.entry.tsx ----------------------------------------------
    let entry_out = data_dir.join(STUDIO_ENTRY_OUT);
    if let Some(entry_path) = locate_entry(entry_file) {
        match bundle_entry(&entry_path, &entry_out) {
            Ok(()) => {
                config.has_extensions = true;
            }
            Err(d) => {
                diagnostics.push(d);
            }
        }
    } else {
        // Entry was removed — clean up the stale bundle so the runtime
        // doesn't keep serving it.
        let _ = std::fs::remove_file(&entry_out);
        config.has_extensions = false;
    }

    // ---- write config JSON ---------------------------------------------
    if let Err(e) = std::fs::create_dir_all(data_dir) {
        diagnostics.push(Diagnostic {
            severity: Severity::Error,
            code: "STUDIO_DATA_DIR".into(),
            message: format!("Could not create {}: {e}", data_dir.display()),
            span: None,
            hint: None,
        });
    }
    let cfg_out = data_dir.join(STUDIO_CONFIG_OUT);
    let cfg_json = serde_json::to_string_pretty(&config).unwrap_or_else(|_| "{}".into());
    if let Err(e) = std::fs::write(&cfg_out, format!("{cfg_json}\n")) {
        diagnostics.push(Diagnostic {
            severity: Severity::Error,
            code: "STUDIO_CONFIG_WRITE".into(),
            message: format!("Could not write {}: {e}", cfg_out.display()),
            span: None,
            hint: None,
        });
    }

    if diagnostics.is_empty() {
        Ok(config)
    } else {
        Err(diagnostics)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_files_resolve_to_none() {
        let tmp = std::env::temp_dir().join("pylon-studio-test-empty");
        let _ = std::fs::create_dir_all(&tmp);
        let app = tmp.join("app.ts");
        // Don't actually create studio.config.ts / studio.entry.tsx.
        assert!(locate_config(&app.to_string_lossy()).is_none());
        assert!(locate_entry(&app.to_string_lossy()).is_none());
    }

    #[test]
    fn build_artefacts_writes_default_when_no_user_files() {
        let tmp = std::env::temp_dir().join("pylon-studio-test-default");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let app = tmp.join("app.ts");
        let data = tmp.join(".pylon");
        let cfg = build_artefacts(&app.to_string_lossy(), &data).unwrap();
        assert_eq!(cfg, StudioConfig::default());
        let written = std::fs::read_to_string(data.join(STUDIO_CONFIG_OUT)).unwrap();
        let parsed: StudioConfig = serde_json::from_str(&written).unwrap();
        assert_eq!(parsed, StudioConfig::default());
    }
}
