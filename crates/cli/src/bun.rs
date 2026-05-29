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

/// Scan a package.json blob for dep names whose version specifier is
/// `workspace:*` (or `workspace:^*`/`workspace:~*` — bun accepts all
/// three). Returns the dep names; the caller decides whether they're
/// safe to strip (i.e. already satisfied by node_modules).
///
/// Tiny custom scan rather than full JSON parsing because we don't
/// want to take a serde_json hit in the hot path — package.json is
/// read on every `pylon start`. The format we care about is stable
/// (`"name": "workspace:*"` inside a `dependencies` /
/// `devDependencies` / `peerDependencies` / `optionalDependencies`
/// object) and the regex-style parse is good enough.
fn scan_workspace_deps(pkg_json: &str) -> Vec<String> {
    let mut out = Vec::new();
    // Match `"<name>": "workspace:*"` (with optional `^` or `~`
    // after the colon) anywhere in the file. Quoting is canonical;
    // bun rewrites lockfiles to this form, and humans rarely
    // hand-format with extra whitespace.
    let mut i = 0;
    let bytes = pkg_json.as_bytes();
    while let Some(rel) = pkg_json[i..].find("\"workspace:") {
        let start = i + rel;
        // Walk back to find the preceding `"<name>"`.
        let before = &pkg_json[..start];
        let Some(colon) = before.rfind(':') else {
            i = start + 1;
            continue;
        };
        let key_seg = &before[..colon];
        let Some(close_q) = key_seg.rfind('"') else {
            i = start + 1;
            continue;
        };
        let key_seg2 = &key_seg[..close_q];
        let Some(open_q) = key_seg2.rfind('"') else {
            i = start + 1;
            continue;
        };
        let name = &before[open_q + 1..close_q];
        if !name.is_empty() && !name.contains('"') {
            out.push(name.to_string());
        }
        i = start + 1;
        // bytes is referenced just to keep the borrow checker calm
        // about the slice indexing above; unused otherwise.
        let _ = bytes;
    }
    out
}

/// Strip the given dep names from package.json by replacing each
/// `"<name>": "workspace:*",?` line with nothing, then cleaning up
/// any stray trailing-comma artifact. Preserves indentation and
/// non-dep content verbatim so the temp file remains valid JSON.
///
/// JSON-aware enough for the package.json shape (no embedded
/// {}-escaped strings that would confuse the trim), not a general
/// JSON rewriter.
fn strip_workspace_deps_from_pkg_json(pkg_json: &str, deps: &[String]) -> String {
    let mut out = pkg_json.to_string();
    for name in deps {
        // Match `"<name>": "workspace:..."` with optional trailing
        // comma and surrounding whitespace. The dep object always
        // has each entry on its own line in practice (bun + npm both
        // emit that shape), so deleting the whole line is safe.
        let needle = format!("\"{name}\"");
        loop {
            let Some(pos) = out.find(&needle) else { break };
            // Only strip if this occurrence is followed by `:
            // "workspace:` — guards against the same string appearing
            // elsewhere (e.g. as a value, in a comment, etc.).
            let after = &out[pos + needle.len()..];
            let trimmed = after.trim_start();
            if !trimmed.starts_with(':') {
                break;
            }
            let after_colon = trimmed[1..].trim_start();
            if !after_colon.starts_with("\"workspace:") {
                break;
            }
            // Locate end of the line (including any trailing comma
            // and the newline itself).
            let line_end = out[pos..]
                .find('\n')
                .map(|n| pos + n + 1)
                .unwrap_or(out.len());
            // Walk back to the start of the line so we drop the full
            // line, not a mid-line fragment.
            let line_start = out[..pos].rfind('\n').map(|n| n + 1).unwrap_or(0);
            out.replace_range(line_start..line_end, "");
        }
    }
    // Clean up `,\n  }` / `,\n}` artifacts left behind when the
    // stripped entry was the last in its object — JSON parsers
    // (including bun's) reject trailing commas.
    let cleaned: String = out
        .lines()
        .scan(false, |prev_was_comma_only, line| {
            let trimmed = line.trim_start();
            if *prev_was_comma_only && (trimmed.starts_with('}') || trimmed.starts_with(']')) {
                *prev_was_comma_only = false;
            }
            *prev_was_comma_only = trimmed.ends_with(',');
            Some(line.to_string())
        })
        .collect::<Vec<_>>()
        .join("\n");
    // Final pass: drop ",\n  }" → "\n  }" so we don't ship trailing
    // commas into bun. Done as text replace rather than full reparse
    // because the package.json shape we touch is well-defined.
    let cleaned = cleaned
        .replace(",\n  }", "\n  }")
        .replace(",\n  ]", "\n  ]")
        .replace(",\n\t}", "\n\t}")
        .replace(",\n\t]", "\n\t]")
        .replace(",\n}", "\n}")
        .replace(",\n]", "\n]");
    cleaned
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

    // workspace:* dep handling. Apps authored inside a monorepo
    // (the pylon-chat example, anyone else using bun workspaces)
    // pin their @pylonsync/* deps as `workspace:*` so local dev
    // gets the live source. In Pylon Cloud, the base image has
    // already symlinked those packages into node_modules — bun
    // install in the deployed container has no workspace root to
    // resolve `workspace:*` against, so it dies with "Workspace
    // dependency X not found" and the machine enters a restart
    // loop. Symptom on 2026-05-17: chat-api spent ~hours bouncing.
    //
    // Detect this and write a temp package.json with workspace:*
    // entries stripped (only those whose target is already in
    // node_modules — anything missing should still error so the
    // operator notices). Run install against the temp, restore
    // the original on success/failure. Self-host monorepo dev
    // never hits this branch because bun install resolves the
    // workspace deps normally.
    let pkg_json_text = std::fs::read_to_string(&pkg_json).map_err(|e| Diagnostic {
        severity: Severity::Error,
        code: "PKG_JSON_READ_FAILED".into(),
        message: format!("Failed to read {}: {e}", pkg_json.display()),
        span: None,
        hint: None,
    })?;
    let workspace_deps = scan_workspace_deps(&pkg_json_text);
    let satisfied_workspace_deps: Vec<&String> = workspace_deps
        .iter()
        .filter(|name| node_modules.join(name).join("package.json").is_file())
        .collect();
    // Stripping only kicks in when we DID find workspace:* entries
    // AND every one of them is satisfied by an existing
    // node_modules entry (typically a symlink the base image
    // pre-populated). Mixed states fall through to the normal
    // install path, which will fail loudly so the operator sees
    // exactly which dep is missing.
    let pkg_json_backup_path = pkg_dir.join("package.json.pylon-bak");
    let restore_after =
        if !workspace_deps.is_empty() && satisfied_workspace_deps.len() == workspace_deps.len() {
            let stripped = strip_workspace_deps_from_pkg_json(&pkg_json_text, &workspace_deps);
            if let Err(e) = std::fs::rename(&pkg_json, &pkg_json_backup_path) {
                return Err(Diagnostic {
                    severity: Severity::Error,
                    code: "PKG_JSON_BACKUP_FAILED".into(),
                    message: format!(
                        "Failed to back up package.json before stripping workspace:* deps: {e}"
                    ),
                    span: None,
                    hint: None,
                });
            }
            if let Err(e) = std::fs::write(&pkg_json, stripped) {
                // Restore before bailing so the next attempt sees the
                // original file, not a half-state.
                let _ = std::fs::rename(&pkg_json_backup_path, &pkg_json);
                return Err(Diagnostic {
                    severity: Severity::Error,
                    code: "PKG_JSON_WRITE_FAILED".into(),
                    message: format!("Failed to write stripped package.json: {e}"),
                    span: None,
                    hint: None,
                });
            }
            true
        } else {
            false
        };

    let has_lockfile = lockfile_text.is_file() || lockfile_bin.is_file();
    let mut cmd = std::process::Command::new("bun");
    cmd.arg("install").current_dir(&pkg_dir);
    if has_lockfile && !restore_after {
        // `--frozen-lockfile` makes the install deterministic and
        // fails if the lockfile is out of sync with package.json.
        // That's the right default for Pylon Cloud (we want every
        // deploy to install the same tree the user committed) and
        // for self-host (the user is the source of truth for what
        // ships). The error message bun emits in this case is
        // specific enough that we don't need to rewrite it.
        //
        // Skipped on the strip-workspace-deps path because the
        // committed lockfile WILL be out of sync with the
        // temporarily-stripped package.json — frozen-lockfile would
        // false-positive there and abort an otherwise valid deploy.
        cmd.arg("--frozen-lockfile");
    }

    let output = cmd.output().map_err(|e| {
        if restore_after {
            let _ = std::fs::rename(&pkg_json_backup_path, &pkg_json);
        }
        Diagnostic {
            severity: Severity::Error,
            code: "BUN_EXEC_FAILED".into(),
            message: format!(
                "Failed to execute `bun install` in {}: {e}",
                pkg_dir.display()
            ),
            span: None,
            hint: Some("Ensure bun is installed and available on PATH".into()),
        }
    })?;

    // Always restore the original package.json before returning so
    // callers see the file as the user authored it (matters for
    // subsequent `pylon dev` watcher restarts, `pylon codegen` runs,
    // and any operator inspecting the running container).
    if restore_after {
        let _ = std::fs::rename(&pkg_json_backup_path, &pkg_json);
    }

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

/// Build the frontend SPA if the project ships one — the runtime
/// counterpart to [`ensure_npm_deps_installed`] for the `web/dist/`
/// half of a unified full-stack app.
///
/// Detection: looks for `<entry-dir>/web/package.json` then
/// `<entry-dir>/apps/web/package.json`. First hit wins. The
/// `package.json` must declare a `build` script.
///
/// Skipping: if `dist/index.html` exists AND every input under
/// `src/`, plus `package.json` + `vite.config.*` + `tsconfig.json`,
/// is older than the marker, we skip both install and build. The
/// marker file lives at `dist/.pylon-build-marker` and is touched
/// after a successful build. This keeps warm-boot cost at ~5ms
/// instead of ~15s.
///
/// Behaviour on Pylon Cloud / Fly: the deploy machinery ships
/// `web/` source via files-mount but never `dist/` (excluded). On
/// first boot this runs `bun install + bun run build`, taking the
/// hit once. Subsequent reboots of the same machine reuse the
/// existing `dist/`.
///
/// `BUN_INSTALL_FAILED` / `FRONTEND_BUILD_FAILED` diagnostics give
/// the operator a clear distinction between dep-install issues vs
/// build-script failures.
pub fn ensure_frontend_built(entry_file: &str) -> Result<(), Diagnostic> {
    let entry_dir = Path::new(entry_file)
        .parent()
        .and_then(|p| if p.as_os_str().is_empty() { None } else { Some(p) })
        .unwrap_or(Path::new("."));
    let candidates = [entry_dir.join("web"), entry_dir.join("apps/web")];
    let Some(web_dir) = candidates
        .into_iter()
        .find(|p| p.join("package.json").is_file())
    else {
        return Ok(());
    };

    if !package_json_has_build_script(&web_dir.join("package.json")) {
        return Ok(());
    }

    let dist = web_dir.join("dist");
    let marker = dist.join(".pylon-build-marker");
    let index = dist.join("index.html");

    // Fast-skip when nothing's changed since the last successful build.
    // Compare the marker mtime against every relevant input. If the
    // input set drifts (new build config in some future framework),
    // worst case is we under-rebuild — operator can rm -rf dist/ to
    // force. Better than over-rebuilding every reboot.
    if index.is_file() && marker.is_file() {
        if let Ok(marker_mtime) = std::fs::metadata(&marker).and_then(|m| m.modified()) {
            let mut newest_input = std::time::SystemTime::UNIX_EPOCH;
            let mut walked = false;
            for input in [
                web_dir.join("package.json"),
                web_dir.join("bun.lock"),
                web_dir.join("bun.lockb"),
                web_dir.join("vite.config.ts"),
                web_dir.join("vite.config.js"),
                web_dir.join("next.config.ts"),
                web_dir.join("next.config.js"),
                web_dir.join("tsconfig.json"),
                web_dir.join("astro.config.ts"),
                web_dir.join("astro.config.mjs"),
            ] {
                if let Ok(t) = std::fs::metadata(&input).and_then(|m| m.modified()) {
                    walked = true;
                    if t > newest_input {
                        newest_input = t;
                    }
                }
            }
            if let Ok(src_newest) = newest_mtime_in(&web_dir.join("src")) {
                walked = true;
                if src_newest > newest_input {
                    newest_input = src_newest;
                }
            }
            if walked && newest_input <= marker_mtime {
                return Ok(());
            }
        }
    }

    // Install deps for the frontend. Mirrors ensure_npm_deps_installed's
    // marker-fast-skip but keyed off node_modules/.pylon-install-marker
    // INSIDE the web dir.
    let node_modules = web_dir.join("node_modules");
    let install_marker = node_modules.join(".pylon-install-marker");
    let pkg_json = web_dir.join("package.json");
    let lockfile_text = web_dir.join("bun.lock");
    let lockfile_bin = web_dir.join("bun.lockb");
    let install_fresh = node_modules.is_dir()
        && install_marker.is_file()
        && std::fs::metadata(&install_marker)
            .and_then(|m| m.modified())
            .ok()
            .map(|marker_t| {
                [&pkg_json, &lockfile_text, &lockfile_bin].iter().all(|p| {
                    std::fs::metadata(p)
                        .and_then(|m| m.modified())
                        .map(|t| t <= marker_t)
                        .unwrap_or(true)
                })
            })
            .unwrap_or(false);

    if !install_fresh {
        let has_lockfile = lockfile_text.is_file() || lockfile_bin.is_file();
        let mut cmd = std::process::Command::new("bun");
        cmd.arg("install").current_dir(&web_dir);
        if has_lockfile {
            cmd.arg("--frozen-lockfile");
        }
        let output = cmd.output().map_err(|e| Diagnostic {
            severity: Severity::Error,
            code: "BUN_EXEC_FAILED".into(),
            message: format!(
                "Failed to execute `bun install` for frontend in {}: {e}",
                web_dir.display()
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
            return Err(Diagnostic {
                severity: Severity::Error,
                code: "BUN_INSTALL_FAILED".into(),
                message: format!(
                    "Installing frontend npm dependencies failed in {}:\n\n{detail}",
                    web_dir.display()
                ),
                span: None,
                hint: Some(
                    "Commit bun.lock so the deploy can run in --frozen-lockfile mode.".into(),
                ),
            });
        }
        let _ = std::fs::File::create(&install_marker);
    }

    // Run the build script. Use the explicit `bun run build` form so
    // the call works against any framework whose package.json defines
    // `"build": "..."` — vite, next, astro, parcel, etc.
    let output = std::process::Command::new("bun")
        .args(["run", "build"])
        .current_dir(&web_dir)
        .output()
        .map_err(|e| Diagnostic {
            severity: Severity::Error,
            code: "BUN_EXEC_FAILED".into(),
            message: format!(
                "Failed to execute `bun run build` for frontend in {}: {e}",
                web_dir.display()
            ),
            span: None,
            hint: None,
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if !stderr.is_empty() {
            stderr.trim()
        } else {
            stdout.trim()
        };
        return Err(Diagnostic {
            severity: Severity::Error,
            code: "FRONTEND_BUILD_FAILED".into(),
            message: format!(
                "`bun run build` failed in {}:\n\n{detail}",
                web_dir.display()
            ),
            span: None,
            hint: Some(
                "Test the build locally with `cd web && bun run build` to reproduce.".into(),
            ),
        });
    }

    let _ = std::fs::File::create(&marker);
    Ok(())
}

/// Does `path` (a package.json) declare a `build` script?
fn package_json_has_build_script(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return false;
    };
    value
        .get("scripts")
        .and_then(|s| s.get("build"))
        .and_then(|d| d.as_str())
        .map(|s| !s.is_empty())
        .unwrap_or(false)
}

/// Recursively find the newest mtime under `dir`. Used by
/// [`ensure_frontend_built`] to detect source edits across an
/// arbitrary `src/` tree.
fn newest_mtime_in(dir: &Path) -> std::io::Result<std::time::SystemTime> {
    let mut newest = std::time::SystemTime::UNIX_EPOCH;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = match std::fs::read_dir(&d) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if let Ok(ft) = entry.file_type() {
                if ft.is_dir() {
                    stack.push(path);
                } else if let Ok(m) = entry.metadata() {
                    if let Ok(t) = m.modified() {
                        if t > newest {
                            newest = t;
                        }
                    }
                }
            }
        }
    }
    Ok(newest)
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

    /// Regression: chat-example's package.json uses `workspace:*`
    /// for @pylonsync/*. Without scan_workspace_deps, the deployed
    /// container's `bun install` died with "Workspace dependency X
    /// not found" → restart loop. Confirms we detect both styles
    /// bun normalizes to.
    #[test]
    fn scan_workspace_deps_finds_workspace_specifiers() {
        let pkg_json = r#"{
            "name": "chat-example",
            "dependencies": {
                "@pylonsync/functions": "workspace:*",
                "@pylonsync/react": "workspace:^*",
                "@pylonsync/sdk": "workspace:~*",
                "stripe": "^14.0.0"
            }
        }"#;
        let mut deps = scan_workspace_deps(pkg_json);
        deps.sort();
        assert_eq!(
            deps,
            vec![
                "@pylonsync/functions".to_string(),
                "@pylonsync/react".to_string(),
                "@pylonsync/sdk".to_string(),
            ],
        );
    }

    /// Non-workspace values must not be misidentified — e.g. a
    /// version string that happens to contain "workspace" inside
    /// a comment-shaped field. Sanity guard against the regex-style
    /// scan over-matching.
    #[test]
    fn scan_workspace_deps_ignores_non_workspace_values() {
        let pkg_json = r#"{
            "description": "uses workspace: pattern in docs",
            "dependencies": {
                "stripe": "^14.0.0",
                "@sentry/bun": "8.0.0"
            }
        }"#;
        assert!(scan_workspace_deps(pkg_json).is_empty());
    }

    /// Strip should leave a parse-clean JSON behind — no trailing
    /// commas, no half-deleted entries. We don't reparse the result
    /// (no serde_json dep) but check the shape via simple string
    /// invariants the bun parser cares about.
    #[test]
    fn strip_workspace_deps_leaves_valid_json_shape() {
        let pkg_json = r#"{
  "name": "chat-example",
  "dependencies": {
    "@pylonsync/functions": "workspace:*",
    "@pylonsync/react": "workspace:*",
    "@pylonsync/sdk": "workspace:*"
  }
}"#;
        let stripped = strip_workspace_deps_from_pkg_json(
            pkg_json,
            &[
                "@pylonsync/functions".into(),
                "@pylonsync/react".into(),
                "@pylonsync/sdk".into(),
            ],
        );
        // None of the workspace entries should survive.
        assert!(!stripped.contains("workspace:"));
        assert!(!stripped.contains("@pylonsync/functions"));
        // No trailing-comma artifacts — the most common shape that
        // breaks bun's parser when we strip the last dep in an
        // object.
        assert!(!stripped.contains(",\n  }"));
        assert!(!stripped.contains(",\n}"));
        // The rest of the package.json structure remains intact.
        assert!(stripped.contains("\"name\": \"chat-example\""));
        assert!(stripped.contains("\"dependencies\""));
    }

    /// Mixed deps: only the workspace ones get stripped, the rest
    /// stay in place. Important because the chat example will
    /// eventually grow non-pylon deps that DO need installing.
    #[test]
    fn strip_workspace_deps_preserves_non_workspace_deps() {
        let pkg_json = r#"{
  "dependencies": {
    "@pylonsync/functions": "workspace:*",
    "stripe": "^14.0.0",
    "@pylonsync/sdk": "workspace:*"
  }
}"#;
        let stripped = strip_workspace_deps_from_pkg_json(
            pkg_json,
            &["@pylonsync/functions".into(), "@pylonsync/sdk".into()],
        );
        assert!(stripped.contains("\"stripe\": \"^14.0.0\""));
        assert!(!stripped.contains("@pylonsync/functions"));
        assert!(!stripped.contains("@pylonsync/sdk"));
        assert!(!stripped.contains("workspace:"));
    }
}
