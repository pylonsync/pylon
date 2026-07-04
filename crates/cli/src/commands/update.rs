//! `pylon update` — bump every @pylonsync/* dependency in the project
//! to the latest published release, in one command.
//!
//! The SDK packages release in lockstep with the binary, so keeping a
//! project current used to mean hand-editing package.json (and every
//! workspace member's) then reinstalling. Now:
//!
//! ```text
//! pylon update              # bump to latest, run bun install
//! pylon update --dry-run    # show what would change
//! pylon update --version 0.3.314   # pin to a specific release
//! ```
//!
//! Scope: `dependencies` + `devDependencies` in ./package.json and, if
//! a workspace, every member package.json. `workspace:*` pins are left
//! alone (they resolve to the monorepo sibling by design). Edits are
//! surgical string replacements per dep line — formatting, key order,
//! and comments-adjacent structure survive untouched.

use pylon_kernel::ExitCode;

use crate::output;

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let dry_run = args.iter().any(|a| a == "--dry-run");
    let pinned = args
        .windows(2)
        .find(|w| w[0] == "--version")
        .map(|w| w[1].clone());

    let target = match pinned {
        Some(v) => v,
        None => match fetch_latest_version() {
            Ok(v) => v,
            Err(e) => {
                // Offline / registry hiccup: the CLI's own version is
                // the release it shipped with — a safe floor.
                let fallback = env!("CARGO_PKG_VERSION").to_string();
                eprintln!("  (registry lookup failed: {e} — using this CLI's version {fallback})");
                fallback
            }
        },
    };

    let files = collect_package_jsons();
    if files.is_empty() {
        output::print_error("No package.json found here — run pylon update from your app root.");
        return ExitCode::Usage;
    }

    let mut changes: Vec<(String, String, String, String)> = Vec::new(); // (file, dep, from, to)
    for file in &files {
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        let (updated, file_changes) = rewrite_pylonsync_versions(&text, &target);
        for (dep, from) in file_changes {
            changes.push((file.clone(), dep, from, target.clone()));
        }
        if !dry_run && updated != text {
            if let Err(e) = std::fs::write(file, updated) {
                output::print_error(&format!("write {file}: {e}"));
                return ExitCode::Error;
            }
        }
    }

    if json_mode {
        let out = serde_json::json!({
            "target": target,
            "dryRun": dry_run,
            "changes": changes.iter().map(|(f, d, from, to)| serde_json::json!({
                "file": f, "dependency": d, "from": from, "to": to,
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
    } else if changes.is_empty() {
        println!("✓ Already current — every @pylonsync/* dependency is at {target}.");
    } else {
        println!(
            "{} @pylonsync/* dependenc{} → {target}{}:",
            changes.len(),
            if changes.len() == 1 { "y" } else { "ies" },
            if dry_run {
                " (dry run — nothing written)"
            } else {
                ""
            },
        );
        for (file, dep, from, to) in &changes {
            println!("  {file}: {dep}  {from} → {to}");
        }
    }

    if !dry_run && !changes.is_empty() {
        // Reinstall so the lockfile matches. Best-effort — a missing bun
        // is reported, not fatal (the operator may use npm/pnpm).
        if !json_mode {
            println!("→ bun install");
        }
        match std::process::Command::new("bun").arg("install").status() {
            Ok(s) if s.success() => {
                if !json_mode {
                    println!("✓ Updated. Now run: pylon verify");
                }
            }
            Ok(s) => {
                output::print_error(&format!("bun install exited {s} — run it manually"));
                return ExitCode::Error;
            }
            Err(e) => {
                eprintln!("  bun not runnable ({e}) — run your package manager's install manually");
            }
        }
    }
    ExitCode::Ok
}

/// Latest published @pylonsync/sdk version — the packages release in
/// lockstep, so one lookup covers all of them.
fn fetch_latest_version() -> Result<String, String> {
    let resp = ureq::get("https://registry.npmjs.org/@pylonsync/sdk/latest")
        .timeout(std::time::Duration::from_secs(10))
        .call()
        .map_err(|e| e.to_string())?;
    let body: serde_json::Value = resp.into_json().map_err(|e| e.to_string())?;
    body.get("version")
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| "registry response had no version".to_string())
}

/// ./package.json plus, when it declares workspaces, every member's.
fn collect_package_jsons() -> Vec<String> {
    let mut out = Vec::new();
    let root = "package.json";
    if !std::path::Path::new(root).exists() {
        return out;
    }
    out.push(root.to_string());
    let Ok(text) = std::fs::read_to_string(root) else {
        return out;
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&text) else {
        return out;
    };
    let patterns: Vec<String> = match parsed.get("workspaces") {
        Some(serde_json::Value::Array(a)) => a
            .iter()
            .filter_map(|v| v.as_str())
            .map(String::from)
            .collect(),
        Some(serde_json::Value::Object(o)) => o
            .get("packages")
            .and_then(|p| p.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    for pattern in patterns {
        // Minimal glob: `dir/*` expands one level; plain paths pass through.
        if let Some(prefix) = pattern.strip_suffix("/*") {
            if let Ok(entries) = std::fs::read_dir(prefix) {
                for entry in entries.flatten() {
                    let candidate = entry.path().join("package.json");
                    if candidate.is_file() {
                        out.push(candidate.to_string_lossy().into_owned());
                    }
                }
            }
        } else {
            let candidate = std::path::Path::new(&pattern).join("package.json");
            if candidate.is_file() {
                out.push(candidate.to_string_lossy().into_owned());
            }
        }
    }
    out
}

/// Rewrite `"@pylonsync/<pkg>": "<semverish>"` values to `target`,
/// returning the new text + (dep, old-version) pairs. Line-oriented so
/// the file's formatting survives. `workspace:*` (and any value
/// containing "workspace") is preserved — monorepo siblings resolve by
/// path, not registry version.
fn rewrite_pylonsync_versions(text: &str, target: &str) -> (String, Vec<(String, String)>) {
    let mut changes = Vec::new();
    let mut out_lines: Vec<String> = Vec::new();
    for line in text.lines() {
        let mut new_line = line.to_string();
        if let Some((dep, old)) = parse_pylonsync_dep_line(line) {
            if !old.contains("workspace") && strip_range_prefix(&old) != target {
                let prefix = &old[..old.len() - strip_range_prefix(&old).len()];
                new_line =
                    line.replacen(&format!("\"{old}\""), &format!("\"{prefix}{target}\""), 1);
                changes.push((dep, old));
            }
        }
        out_lines.push(new_line);
    }
    let mut joined = out_lines.join("\n");
    if text.ends_with('\n') {
        joined.push('\n');
    }
    (joined, changes)
}

/// `"@pylonsync/react": "^0.3.290",` → Some(("@pylonsync/react", "^0.3.290"))
fn parse_pylonsync_dep_line(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix("\"@pylonsync/")?;
    let (name_tail, rest) = rest.split_once('"')?;
    let rest = rest.trim_start().strip_prefix(':')?;
    let rest = rest.trim_start().strip_prefix('"')?;
    let (version, _) = rest.split_once('"')?;
    Some((format!("@pylonsync/{name_tail}"), version.to_string()))
}

/// "^0.3.290" → "0.3.290" (also ~, >=, =).
fn strip_range_prefix(v: &str) -> &str {
    v.trim_start_matches(['^', '~', '=', '>', '<', ' '])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_only_pylonsync_deps_preserving_range_prefix_and_formatting() {
        let src = r#"{
  "name": "my-app",
  "dependencies": {
    "@pylonsync/react": "^0.3.290",
    "@pylonsync/sdk": "0.3.290",
    "react": "^19.2.0"
  },
  "devDependencies": {
    "@pylonsync/functions": "~0.3.291"
  }
}
"#;
        let (out, changes) = rewrite_pylonsync_versions(src, "0.3.314");
        assert!(out.contains("\"@pylonsync/react\": \"^0.3.314\""));
        assert!(out.contains("\"@pylonsync/sdk\": \"0.3.314\""));
        assert!(out.contains("\"@pylonsync/functions\": \"~0.3.314\""));
        assert!(
            out.contains("\"react\": \"^19.2.0\""),
            "non-pylonsync deps untouched"
        );
        assert_eq!(changes.len(), 3);
        // Formatting (indentation, key order, trailing newline) survives.
        assert!(out.starts_with("{\n  \"name\": \"my-app\""));
        assert!(out.ends_with("}\n"));
    }

    #[test]
    fn workspace_pins_and_current_versions_are_left_alone() {
        let src = r#"{
  "dependencies": {
    "@pylonsync/loro": "workspace:*",
    "@pylonsync/react": "^0.3.314"
  }
}
"#;
        let (out, changes) = rewrite_pylonsync_versions(src, "0.3.314");
        assert_eq!(out, src);
        assert!(changes.is_empty());
    }
}
