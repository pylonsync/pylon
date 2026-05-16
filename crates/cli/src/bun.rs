use std::path::{Path, PathBuf};

use pylon_kernel::Diagnostic;
use pylon_kernel::Severity;

use crate::manifest::parse_manifest;

/// Run a TS entry file with Bun and return the trimmed stdout as manifest JSON.
/// Validates that the output parses as a valid manifest.
///
/// Before the eval, we make sure the app's npm dependencies are
/// installed (see [`ensure_npm_deps_installed`]). Without this, any
/// `import "@sentry/bun"` / `import "stripe"` etc. in the user's
/// schema or its transitive imports fails the eval with a generic
/// "Cannot find module" — fine in dev where the user already ran
/// `bun install`, but fatal on Pylon Cloud which ships TS source +
/// `package.json` straight into a stock runtime image with no
/// install step of its own. Letting the framework own the install
/// keeps cloud and self-host on the same footing.
pub fn run_bun_codegen(entry_file: &str) -> Result<String, Diagnostic> {
    ensure_npm_deps_installed(entry_file)?;

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
        // The actual TypeError / SyntaxError from the user's schema lives
        // in stderr (Bun) or sometimes stdout (manifest builder errors).
        // Prior versions stuffed this into `hint`, but the dev watcher's
        // error printer only renders `message` — so the user saw
        // "bun run schema.ts exited with status 1" with no clue what
        // went wrong. Inline the detail into `message` so it shows up
        // regardless of which printer is used.
        let detail = if !stderr.is_empty() {
            stderr.trim()
        } else {
            stdout.trim()
        };
        let exit_code = output.status.code().unwrap_or(-1);
        let message = if detail.is_empty() {
            format!("Schema evaluation of `{entry_file}` failed (exit {exit_code}). No output captured — try running it manually to debug.")
        } else {
            format!("Schema evaluation of `{entry_file}` failed (exit {exit_code}):\n\n{detail}")
        };
        return Err(Diagnostic {
            severity: Severity::Error,
            code: "SCHEMA_EVAL_FAILED".into(),
            message,
            span: None,
            hint: Some(format!("Reproduce locally with: bun run {entry_file}")),
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

/// Walk up from `entry_file`'s directory to the nearest
/// `package.json`. Used to decide where (and whether) to run
/// `bun install`. Returns `None` when no package.json is found by
/// the time we reach the filesystem root — common for tiny example
/// apps that have no npm deps and don't bother with a manifest.
fn find_nearest_package_json(entry_file: &str) -> Option<PathBuf> {
    let entry = Path::new(entry_file);
    let start = entry
        .parent()
        .map(|p| {
            if p.as_os_str().is_empty() {
                Path::new(".")
            } else {
                p
            }
        })
        .unwrap_or(Path::new("."));
    let mut current = start.canonicalize().ok()?;
    loop {
        if current.join("package.json").is_file() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

/// Ensure the npm dependencies declared in the nearest
/// `package.json` are installed before we eval the user's schema.
///
/// Behaviour:
///
/// - No `package.json` anywhere up the tree → no-op. Many small
///   Pylon apps don't have one (everything they need ships with
///   the framework).
/// - `node_modules/` present AND a `.pylon-install-marker` file
///   inside it is newer than `package.json` + any lockfile → skip.
///   This is the fast path; once an app installs cleanly, repeat
///   invocations (dev watcher restart, machine reboot) are free.
/// - Otherwise → run `bun install`. Uses `--frozen-lockfile` when a
///   `bun.lock` or `bun.lockb` exists so the resolved tree matches
///   what the user committed; without a lockfile (e.g. a freshly
///   scaffolded app) we let bun write one.
///
/// Errors get their own `BUN_INSTALL_FAILED` diagnostic code so the
/// deploy log can distinguish "dep install broke" from "schema eval
/// broke" — same surfacing as the existing `SCHEMA_EVAL_FAILED`,
/// distinguishable cause for the operator.
pub fn ensure_npm_deps_installed(entry_file: &str) -> Result<(), Diagnostic> {
    let Some(pkg_dir) = find_nearest_package_json(entry_file) else {
        return Ok(());
    };

    let pkg_json = pkg_dir.join("package.json");
    let node_modules = pkg_dir.join("node_modules");
    let lockfile_text = pkg_dir.join("bun.lock");
    let lockfile_bin = pkg_dir.join("bun.lockb");
    let marker = node_modules.join(".pylon-install-marker");

    // Fast-skip when nothing's changed since the last successful
    // install. Compares marker mtime against every input file's
    // mtime; if any is newer (or the marker is missing), we
    // (re)install. Cheap stat calls, no hashing.
    if node_modules.is_dir() && marker.is_file() {
        if let Ok(marker_mtime) = std::fs::metadata(&marker).and_then(|m| m.modified()) {
            let mut inputs_newer = false;
            for input in [&pkg_json, &lockfile_text, &lockfile_bin] {
                if let Ok(t) = std::fs::metadata(input).and_then(|m| m.modified()) {
                    if t > marker_mtime {
                        inputs_newer = true;
                        break;
                    }
                }
            }
            if !inputs_newer {
                return Ok(());
            }
        }
    }

    let has_lockfile = lockfile_text.is_file() || lockfile_bin.is_file();
    let mut cmd = std::process::Command::new("bun");
    cmd.arg("install").current_dir(&pkg_dir);
    if has_lockfile {
        // `--frozen-lockfile` makes the install deterministic and
        // fails if the lockfile is out of sync with package.json.
        // That's the right default for Pylon Cloud (we want every
        // deploy to install the same tree the user committed) and
        // for self-host (the user is the source of truth for what
        // ships). The error message bun emits in this case is
        // specific enough that we don't need to rewrite it.
        cmd.arg("--frozen-lockfile");
    }

    let output = cmd.output().map_err(|e| Diagnostic {
        severity: Severity::Error,
        code: "BUN_EXEC_FAILED".into(),
        message: format!(
            "Failed to execute `bun install` in {}: {e}",
            pkg_dir.display()
        ),
        span: None,
        hint: Some("Ensure bun is installed and available on PATH".into()),
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if !stderr.is_empty() {
            stderr.trim()
        } else {
            stdout.trim()
        };
        let exit_code = output.status.code().unwrap_or(-1);
        // Cleaner hint for the common case (committed package.json but
        // no lockfile committed, or lockfile drifted from package.json).
        // The deploy operator should know which side to fix.
        let hint = if has_lockfile {
            format!(
                "`bun install --frozen-lockfile` failed in {}. Most often this means package.json was edited without re-committing bun.lock. Run `bun install` locally and commit the lockfile.",
                pkg_dir.display()
            )
        } else {
            format!(
                "`bun install` failed in {}. Try running it locally first to surface any registry / version-resolution issues — then commit bun.lock so the deploy can run in --frozen-lockfile mode.",
                pkg_dir.display()
            )
        };
        return Err(Diagnostic {
            severity: Severity::Error,
            code: "BUN_INSTALL_FAILED".into(),
            message: format!(
                "Installing npm dependencies failed (exit {exit_code}) in {}:\n\n{detail}",
                pkg_dir.display(),
            ),
            span: None,
            hint: Some(hint),
        });
    }

    // Mark the install as fresh so the next invocation can fast-skip.
    // Failure to write the marker isn't fatal — worst case we
    // re-install next time, which is wasteful but correct.
    let _ = std::fs::File::create(&marker);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    fn write(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut f = fs::File::create(path).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
    }

    #[test]
    fn find_nearest_package_json_walks_up() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join("package.json"), "{}");
        write(&root.join("apps/api/app.ts"), "");
        let found = find_nearest_package_json(root.join("apps/api/app.ts").to_str().unwrap());
        assert_eq!(
            found.map(|p| p.canonicalize().unwrap()),
            Some(root.canonicalize().unwrap())
        );
    }

    #[test]
    fn find_nearest_package_json_returns_none_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join("app.ts"), "");
        // No package.json anywhere under tmpdir. The walk-up reaches
        // a real filesystem ancestor — there may or may not be a
        // package.json above tmpdir (the dev's home dir might have
        // one), so we can't strictly assert None here. Instead
        // assert the function doesn't panic and returns *something*
        // we can reason about: either None, or a path that's
        // strictly an ancestor of tmpdir (i.e. not inside it).
        let result = find_nearest_package_json(root.join("app.ts").to_str().unwrap());
        if let Some(p) = result {
            let canon_root = root.canonicalize().unwrap();
            assert!(
                !p.starts_with(&canon_root),
                "should not find a package.json inside the temp dir when none was written"
            );
        }
    }

    #[test]
    fn ensure_npm_deps_skips_when_marker_fresh() {
        // Synthetic happy path: prepopulated node_modules + marker
        // newer than package.json → ensure_ skips the install
        // entirely. We rely on bun NOT being invoked; the test
        // succeeds because no `Cannot find module` etc. surfaces.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join("package.json"), r#"{"name":"x","version":"0"}"#);
        write(&root.join("app.ts"), "");
        std::fs::create_dir_all(root.join("node_modules")).unwrap();
        write(&root.join("node_modules/.pylon-install-marker"), "");
        // Marker now > package.json mtime (just-written).
        // Sleep one ms isn't reliable; instead, touch the marker
        // explicitly to ensure ordering.
        let marker_path = root.join("node_modules/.pylon-install-marker");
        // Re-stamp with explicit set_modified via filetime would be
        // ideal but adds a dep — instead, re-write to bump mtime.
        std::thread::sleep(std::time::Duration::from_millis(20));
        write(&marker_path, "");

        let result = ensure_npm_deps_installed(root.join("app.ts").to_str().unwrap());
        assert!(
            result.is_ok(),
            "fresh marker path should not invoke bun: {result:?}"
        );
    }
}
