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
/// `frozen` controls `bun install --frozen-lockfile`: pass `false` for the
/// local dev loop (`pylon dev` / `codegen`) so adding a dependency to
/// package.json just installs + updates the lockfile, and `true` for
/// deploy/prod (`pylon start` / `build`) so the resolved tree is reproducible.
pub fn run_bun_codegen(entry_file: &str, frozen: bool) -> Result<String, Diagnostic> {
    ensure_npm_deps_installed(entry_file, frozen)?;

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
pub fn ensure_npm_deps_installed(entry_file: &str, frozen: bool) -> Result<(), Diagnostic> {
    let Some(pkg_dir) = find_nearest_package_json(entry_file) else {
        return Ok(());
    };
    bun_install_in_dir(&pkg_dir, frozen)
}

/// Public wrapper kept for back-compat — root install path doesn't
/// know about a "source" web dir, so just delegates.
fn bun_install_in_dir(pkg_dir: &Path, frozen: bool) -> Result<(), Diagnostic> {
    bun_install_in_dir_with_source(pkg_dir, None, frozen)
}

/// Workhorse used by both the root install ([`ensure_npm_deps_installed`])
/// and the frontend install ([`ensure_frontend_built`]). Runs `bun install`
/// in `pkg_dir` with these guarantees:
///
/// - Fast-skip via mtime marker at `<pkg_dir>/node_modules/.pylon-install-marker`.
/// - Workspace:* dep handling for monorepo-authored apps deployed standalone
///   (Pylon Cloud / chat dogfood): if every workspace dep is already satisfied
///   in `node_modules` OR resolvable via the walk-up candidates from
///   `find_workspace_package_dir`, strip them from package.json before
///   install and restore after. Otherwise let bun fail loudly so the operator
///   knows which dep is missing.
/// - Uses `--frozen-lockfile` when a lockfile is present AND we're not on
///   the strip-workspace-deps path (that path always drifts the committed
///   lockfile).
///
/// `source_web_dir` is the original (possibly read-only) source web dir
/// when `pkg_dir` is a build fallback — `find_workspace_package_dir` can
/// then also satisfy workspace deps via the source's monorepo node_modules.
/// Pass `None` for the root install (no fallback monorepo concept there).
///
/// Returns `BUN_INSTALL_FAILED` on non-zero exit; `BUN_EXEC_FAILED` when
/// bun itself can't spawn; `PKG_JSON_*` on the rare backup/write hiccup.
fn bun_install_in_dir_with_source(
    pkg_dir: &Path,
    source_web_dir: Option<&Path>,
    frozen: bool,
) -> Result<(), Diagnostic> {
    let pkg_json = pkg_dir.join("package.json");
    let node_modules = pkg_dir.join("node_modules");
    let lockfile_text = pkg_dir.join("bun.lock");
    let lockfile_bin = pkg_dir.join("bun.lockb");
    let marker = node_modules.join(".pylon-install-marker");
    let pkg_json_backup_path = pkg_dir.join("package.json.pylon-bak");

    // Crash-recovery: a previous run that died between the first
    // rename(pkg_json → backup) and the final rename(backup → pkg_json)
    // leaves package.json holding a stripped version and the backup
    // holding the original. Without recovery, this run reads the
    // stripped package.json, scan_workspace_deps returns empty, the
    // strip path doesn't fire, and `bun install --frozen-lockfile`
    // rejects the install (lockfile still references the workspace
    // deps that package.json no longer declares).
    //
    // The backup is the source of truth — rename overwrites the
    // current (possibly partial) package.json. Idempotent: a normal
    // boot has no backup file and skips this block.
    if pkg_json_backup_path.is_file() {
        if let Err(e) = std::fs::rename(&pkg_json_backup_path, &pkg_json) {
            return Err(Diagnostic {
                severity: Severity::Error,
                code: "PKG_JSON_RECOVERY_FAILED".into(),
                message: format!(
                    "Found stranded {} from a previous crashed install — failed to restore: {e}",
                    pkg_json_backup_path.display(),
                ),
                span: None,
                hint: Some(format!(
                    "Manually `mv {} {}` and retry.",
                    pkg_json_backup_path.display(),
                    pkg_json.display(),
                )),
            });
        }
    }

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

    let pkg_json_text = std::fs::read_to_string(&pkg_json).map_err(|e| Diagnostic {
        severity: Severity::Error,
        code: "PKG_JSON_READ_FAILED".into(),
        message: format!("Failed to read {}: {e}", pkg_json.display()),
        span: None,
        hint: None,
    })?;
    let workspace_deps = scan_workspace_deps(&pkg_json_text);
    // A workspace:* dep is "satisfied" when bun's import resolution can
    // find a real package for it. Check, in order:
    //
    //   1. <pkg_dir>/node_modules/<dep>/package.json  ← local install
    //   2. /pylon/packages/<bare>/package.json        ← Pylon Cloud image
    //   3. /app/node_modules/<dep>/package.json       ← Dockerfile-seeded
    //   4. <source_web_dir>/node_modules/<dep>/...    ← local monorepo
    //
    // Without these walk-ups we'd false-negative on the "all satisfied"
    // gate and fall through to a normal `bun install --frozen-lockfile`
    // with workspace:* deps intact, which dies with "Workspace dependency
    // not found." This was the chat-api failure mode through v0.3.207.
    let satisfied_workspace_deps: Vec<&String> = workspace_deps
        .iter()
        .filter(|name| {
            if node_modules.join(name).join("package.json").is_file() {
                return true;
            }
            // find_workspace_package_dir handles all the other candidate
            // locations uniformly. Pass `pkg_dir` itself as the source
            // hint when the caller had no separate source dir.
            let source_hint = source_web_dir.unwrap_or(pkg_dir);
            find_workspace_package_dir(name, source_hint).is_some()
        })
        .collect();
    // pkg_json_backup_path declared at the top for the crash-recovery
    // block; the strip path below reuses the same binding.
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
    cmd.arg("install").current_dir(pkg_dir);
    // Frozen only for deploy/prod (reproducible tree). In the dev loop we let
    // bun resolve + update the lockfile so adding a dep to package.json just
    // works instead of dying with "lockfile is frozen".
    if frozen && has_lockfile && !restore_after {
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
pub fn ensure_frontend_built(entry_file: &str, frozen: bool) -> Result<(), Diagnostic> {
    let entry_dir = Path::new(entry_file)
        .parent()
        .and_then(|p| {
            if p.as_os_str().is_empty() {
                None
            } else {
                Some(p)
            }
        })
        .unwrap_or(Path::new("."));
    let candidates = [entry_dir.join("web"), entry_dir.join("apps/web")];
    let Some(source_web_dir) = candidates
        .into_iter()
        .find(|p| p.join("package.json").is_file())
    else {
        return Ok(());
    };

    if !package_json_has_build_script(&source_web_dir.join("package.json")) {
        return Ok(());
    }

    // If a dist/ already ships alongside the source, it was pre-built (Pylon
    // Cloud's deploy builder runs the frontend build before the flip). Serve it
    // as-is — don't rebuild at boot. This makes the build atomic (a frontend
    // build failure fails the deploy in the builder, never touching the live
    // machine) and skips the slow cold build. frontend.rs's discover_dist_dir
    // checks <app>/web/dist first, so the pre-built output is picked up.
    if source_web_dir.join("dist").join("index.html").is_file() {
        return Ok(());
    }

    // Resolve where to actually do the install + build. In a stock
    // dev or self-host monorepo, this is source_web_dir directly
    // (in-place, fast). On Pylon Cloud / Fly the source ships via
    // files-mount with root ownership of /app/web/, so pylon-uid
    // can read but can't create node_modules/, dist/, or even
    // rename package.json (rename needs write on parent dir).
    // Detect that and fall back to a writable location.
    //
    // Discovery order tried by `resolve_build_dir`:
    //   1. source_web_dir (in-place, when writable)
    //   2. the fallbacks in pylon_kernel::util::frontend_build_dirs — the
    //      persistent volume first (survives reboots so the mtime-marker
    //      fast-skip still works), then the temp directory
    let (build_dir, mirror_needed) =
        resolve_build_dir(&source_web_dir).map_err(|e| Diagnostic {
            severity: Severity::Error,
            code: "BUILD_DIR_RESOLVE_FAILED".into(),
            message: format!(
                "Failed to find a writable build dir for the frontend (tried {}): {e}",
                source_web_dir.display(),
            ),
            span: None,
            hint: Some(
                "Ensure web/ is writable, or that one of the fallback build directories is.".into(),
            ),
        })?;

    // Mirror source into the build dir if we're not building in place.
    // Smart mirror: only copy files whose source mtime is newer than
    // the corresponding dest mtime, so subsequent boots only re-copy
    // edits instead of re-shipping the whole tree. Excludes
    // node_modules (bun re-builds) and dist (output dir).
    if mirror_needed {
        smart_mirror(&source_web_dir, &build_dir, &["node_modules", "dist"]).map_err(|e| {
            Diagnostic {
                severity: Severity::Error,
                code: "BUILD_DIR_COPY_FAILED".into(),
                message: format!(
                    "Failed to mirror frontend source from {} to {}: {e}",
                    source_web_dir.display(),
                    build_dir.display()
                ),
                span: None,
                hint: None,
            }
        })?;
    }

    let dist = build_dir.join("dist");
    let marker = dist.join(".pylon-build-marker");
    let index = dist.join("index.html");

    // Fast-skip when nothing's changed since the last successful build.
    // Inputs are read from source_web_dir (the canonical source), NOT
    // build_dir (where they might've been freshly copied with new
    // mtimes by the mirror step). Marker lives in build_dir/dist
    // because that's where the artifact actually is.
    if index.is_file() && marker.is_file() {
        if let Ok(marker_mtime) = std::fs::metadata(&marker).and_then(|m| m.modified()) {
            let mut newest_input = std::time::SystemTime::UNIX_EPOCH;
            let mut walked = false;
            for input in [
                source_web_dir.join("package.json"),
                source_web_dir.join("bun.lock"),
                source_web_dir.join("bun.lockb"),
                source_web_dir.join("vite.config.ts"),
                source_web_dir.join("vite.config.js"),
                source_web_dir.join("next.config.ts"),
                source_web_dir.join("next.config.js"),
                source_web_dir.join("tsconfig.json"),
                source_web_dir.join("astro.config.ts"),
                source_web_dir.join("astro.config.mjs"),
            ] {
                if let Ok(t) = std::fs::metadata(&input).and_then(|m| m.modified()) {
                    walked = true;
                    if t > newest_input {
                        newest_input = t;
                    }
                }
            }
            if let Ok(src_newest) = newest_mtime_in(&source_web_dir.join("src")) {
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

    // Workspace:* support. Stage symlinks in build_dir/node_modules
    // pointing at the resolved package locations (tried in order:
    // /pylon/packages, /app/node_modules, source_web_dir/node_modules).
    // Vite resolves @pylonsync/* imports through these at bundle time.
    // Pair with `bun_install_in_dir`'s walk-up satisfaction check so
    // the strip-deps gate fires reliably.
    stage_workspace_symlinks(&build_dir, &source_web_dir);

    bun_install_in_dir_with_source(&build_dir, Some(&source_web_dir), frozen)?;

    // Run the build script. Use the explicit `bun run build` form so
    // the call works against any framework whose package.json defines
    // `"build": "..."` — vite, next, astro, parcel, etc.
    let output = std::process::Command::new("bun")
        .args(["run", "build"])
        .current_dir(&build_dir)
        .output()
        .map_err(|e| Diagnostic {
            severity: Severity::Error,
            code: "BUN_EXEC_FAILED".into(),
            message: format!(
                "Failed to execute `bun run build` for frontend in {}: {e}",
                build_dir.display()
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
                build_dir.display()
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

/// Pre-stage `@pylonsync/*` workspace symlinks inside
/// `build_dir/node_modules/`, looking up package locations in
/// `find_workspace_package_dir`. Combines with
/// `bun_install_in_dir`'s walk-up satisfaction check so the
/// strip-workspace-deps path fires reliably across:
///
/// - Pylon Cloud / Fly: `/pylon/packages/<bare>` (image-seeded)
/// - Self-hosted monorepo dev: `source_web_dir/node_modules/@pylonsync/<dep>`
///   (real bun-workspace symlink)
/// - CLI-upload deploys with read-only source: build runs in /tmp or
///   /data fallback, source's pre-existing node_modules may not be
///   present, so `/pylon/packages` is the only fallback.
///
/// Idempotent + best-effort: a write failure for any single dep
/// doesn't propagate; the strip-deps path downstream surfaces the
/// real "dep not found" error with a better explanation than this
/// helper could produce.
fn stage_workspace_symlinks(build_dir: &Path, source_web_dir: &Path) {
    let Ok(pkg_text) = std::fs::read_to_string(build_dir.join("package.json")) else {
        return;
    };
    let workspace_deps = scan_workspace_deps(&pkg_text);
    if workspace_deps.is_empty() {
        return;
    }
    let node_modules = build_dir.join("node_modules");
    let _ = std::fs::create_dir_all(node_modules.join("@pylonsync"));
    for dep in &workspace_deps {
        let dest = node_modules.join(dep);
        if dest.exists() || dest.is_symlink() {
            continue; // already linked or installed
        }
        let Some(src) = find_workspace_package_dir(dep, source_web_dir) else {
            continue; // bun install + strip path will surface the gap
        };
        #[cfg(unix)]
        {
            let _ = std::os::unix::fs::symlink(&src, &dest);
        }
        #[cfg(windows)]
        {
            let _ = std::os::windows::fs::symlink_dir(&src, &dest);
        }
    }
}

/// Find the directory of a workspace dep package by trying the
/// known candidate locations. First hit wins. Returns the dir that
/// CONTAINS `package.json` for the dep — the symlink target.
///
/// Candidates, in order:
/// 1. `/pylon/packages/<bare>` — Pylon Cloud's base image layout.
/// 2. `/app/node_modules/<dep>` — Pylon Cloud's pre-seeded symlink set.
/// 3. `<source_web_dir>/node_modules/<dep>` — local monorepo bun-workspace
///    install (the symlink bun creates in a monorepo).
///
/// Each is verified by checking for a `package.json` inside, so a stale
/// directory or empty placeholder doesn't false-positive.
fn find_workspace_package_dir(dep: &str, source_web_dir: &Path) -> Option<PathBuf> {
    let bare = dep.strip_prefix("@pylonsync/").unwrap_or(dep);
    let candidates = [
        PathBuf::from("/pylon/packages").join(bare),
        PathBuf::from("/app/node_modules").join(dep),
        source_web_dir.join("node_modules").join(dep),
    ];
    // canonicalize() resolves any leading `.` / `..` AND follows the
    // symlink so the returned PathBuf points at the package's real
    // location on disk. Critical because stage_workspace_symlinks
    // uses this output as a symlink TARGET — a relative path
    // (e.g. `./web/node_modules/@pylonsync/example-ui`) resolves
    // against the SYMLINK's dir, not the cwd, and creates a
    // self-loop when the symlink lives in a different tree
    // (the `/tmp/.pylon-frontend-build/web/...` fallback case).
    candidates
        .into_iter()
        .find(|p| p.join("package.json").is_file())
        .and_then(|p| p.canonicalize().ok())
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

/// Quick writability probe: try create + remove a tiny file. Used by
/// `resolve_build_dir` to decide between in-place builds (the natural
/// path) and fallback-dir builds (when the source dir is read-only,
/// e.g. /app/web/ comes from Fly's files-mount with root ownership).
fn is_dir_writable(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    let probe = dir.join(".pylon-write-probe");
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Resolve the directory where bun install + bun run build will run.
/// Returns `(build_dir, mirror_needed)` — when `mirror_needed` is true,
/// the caller must copy source files from web_dir into build_dir
/// before invoking bun.
///
/// Preference order:
///   1. `web_dir` (in-place build, fastest)
///   2. the fallbacks from `pylon_kernel::util::frontend_build_dirs` —
///      the Fly volume first (survives machine restarts, so the
///      mtime-marker fast-skip still works), then the temp directory
///      (rebuilds on every cold start but always works)
///
/// On Pylon Cloud / Fly the source dir comes from files-mount with
/// root ownership. The pylon-uid (10001) container user can read but
/// not write — including renaming package.json (which the
/// strip-workspace-deps path needs). The fallback paths give us a
/// writable build location without sacrificing the warm-boot
/// fast-skip.
fn resolve_build_dir(web_dir: &Path) -> std::io::Result<(PathBuf, bool)> {
    if is_dir_writable(web_dir) {
        return Ok((web_dir.to_path_buf(), false));
    }

    let candidates = pylon_kernel::util::frontend_build_dirs();
    let mut last_err = None;
    for candidate in &candidates {
        match std::fs::create_dir_all(candidate) {
            Ok(()) if is_dir_writable(candidate) => return Ok((candidate.clone(), true)),
            Ok(()) => {}
            Err(e) => last_err = Some(e),
        }
    }

    Err(last_err.unwrap_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!(
                "no writable frontend build directory among {}",
                candidates
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )
    }))
}

/// Smart copy of a source dir tree into a destination, skipping files
/// whose dest mtime is already ≥ source mtime. Excludes given
/// top-level directory names (typically `node_modules` and `dist`).
///
/// Used to seed build_dir from web_dir when the build runs in a
/// different location. Subsequent boots only re-copy edited files —
/// not the entire tree — keeping warm-boot cost low.
///
/// Symlinks in the source are SKIPPED (rare in handwritten source
/// trees; would otherwise dereference through to copied files). The
/// `exclude` list always wins, so a symlinked `node_modules` is
/// dropped before any decision about how to copy it.
fn smart_mirror(src: &Path, dest: &Path, exclude: &[&str]) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if exclude.contains(&name_str.as_ref()) {
            continue;
        }
        let src_path = entry.path();
        let dest_path = dest.join(&name);
        let ft = entry.file_type()?;
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            smart_mirror(&src_path, &dest_path, exclude)?;
        } else if ft.is_file() {
            let src_mtime = entry.metadata()?.modified()?;
            let should_copy = match std::fs::metadata(&dest_path).and_then(|m| m.modified()) {
                Ok(dest_mtime) => src_mtime > dest_mtime,
                Err(_) => true, // dest missing
            };
            if should_copy {
                std::fs::copy(&src_path, &dest_path)?;
            }
        }
    }
    Ok(())
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

        let result = ensure_npm_deps_installed(root.join("app.ts").to_str().unwrap(), true);
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
