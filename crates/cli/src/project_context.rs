//! Current-project resolver shared across every cloud-aware CLI
//! command (secrets / logs / db / data / domains / deployments /
//! members / status).
//!
//! Resolution order (first match wins):
//!   1. `--project <slug>` flag on the command
//!   2. `--project=<slug>` form of the same flag
//!   3. `$PYLON_PROJECT` env var
//!   4. `.pylon/project` file in the cwd or any ancestor (written by
//!      `pylon projects use <slug>`)
//!   5. TTY interactive picker via `listMyProjectsForCli`
//!   6. Hard error pointing at #1 — non-TTY callers (CI, --json) MUST
//!      pass --project explicitly so a misconfigured pipeline doesn't
//!      silently target the wrong project.

use std::fs;
use std::path::PathBuf;

use crate::cloud_client::Credentials;

/// Marker file written by `pylon projects use <slug>`. Walked upward
/// from cwd so any subdirectory inside a Pylon project inherits the
/// context. Pattern matches git (.git/), bun/turbo (turbo.json),
/// and direnv (.envrc) — operators expect this.
const CONTEXT_FILENAME: &str = ".pylon/project";

#[derive(serde::Deserialize)]
struct ProjectPickerEntry {
    slug: String,
    name: String,
    #[serde(rename = "orgSlug")]
    org_slug: Option<String>,
}

/// Resolve the project slug WITHOUT any interactive fallback: `--project`
/// flag → `$PYLON_PROJECT` → `.pylon/project` context file → global default.
/// Returns `None` when nothing is linked. Callers like `deploy` use this to
/// detect "no project here" so they can offer to provision one, instead of
/// dropping into the shared picker (which only lists existing projects).
pub fn resolve_project_slug_noninteractive(args: &[String]) -> Option<String> {
    resolve_project_with_source(args).map(|(slug, _)| slug)
}

/// Where a resolved slug came from. Mutating commands (deploy) treat the
/// machine-global default as a WEAK signal: it follows the last
/// `pylon login` / `pylon projects use` run anywhere on the machine, so
/// deploying an unlinked directory against it can silently overwrite an
/// unrelated production app. Explicit sources (flag / env / context file)
/// are per-invocation or per-directory and carry intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectSource {
    Flag,
    Env,
    ContextFile,
    GlobalDefault,
}

/// Same resolution order as `resolve_project_slug_noninteractive`, but
/// reports WHERE the slug came from so callers can gate the dangerous
/// global-default fallback (see `ProjectSource`).
pub fn resolve_project_with_source(args: &[String]) -> Option<(String, ProjectSource)> {
    // 1 + 2. --project flag (with or without =).
    if let Some(slug) = args
        .windows(2)
        .find(|w| w[0] == "--project")
        .map(|w| w[1].clone())
    {
        return Some((slug, ProjectSource::Flag));
    }
    if let Some(slug) = args
        .iter()
        .find(|a| a.starts_with("--project="))
        .map(|a| a.trim_start_matches("--project=").to_string())
    {
        return Some((slug, ProjectSource::Flag));
    }
    // 3. $PYLON_PROJECT
    if let Ok(slug) = std::env::var("PYLON_PROJECT") {
        if !slug.is_empty() {
            return Some((slug, ProjectSource::Env));
        }
    }
    // 4. .pylon/project from cwd or any ancestor.
    if let Some(slug) = read_context_file() {
        return Some((slug, ProjectSource::ContextFile));
    }
    // 5. Global default from ~/.config/pylon/state.json (set by
    //    `pylon projects use <slug>` / the picker / auto-provision).
    if let Ok(state) = crate::cloud_client::load_state() {
        if let Some(slug) = state.default_project {
            if !slug.is_empty() {
                return Some((slug, ProjectSource::GlobalDefault));
            }
        }
    }
    None
}

/// Resolve the project slug for a cloud-aware command. `args` is the
/// full argv slice; we inspect it for `--project` / `--project=` so
/// callers don't have to.
pub fn resolve_project_slug(
    args: &[String],
    creds: &Credentials,
    json_mode: bool,
) -> Result<String, String> {
    if let Some(slug) = resolve_project_slug_noninteractive(args) {
        return Ok(slug);
    }
    // 6. Interactive picker — TTY only. CI / --json gets an error
    //    pointing at the flag so a misconfigured pipeline doesn't
    //    silently target a previously-set context.
    use std::io::{BufRead, IsTerminal, Write};
    if json_mode || !std::io::stdin().is_terminal() {
        return Err(
			"No project specified. Pass --project <slug>, set PYLON_PROJECT, or run `pylon projects use <slug>`."
				.into(),
		);
    }
    let projects: Vec<ProjectPickerEntry> =
        crate::cloud_client::post_json(creds, "/api/fn/listMyProjectsForCli", &())
            .map_err(|e| format!("Couldn't list your projects: {e}"))?;
    if projects.is_empty() {
        return Err(format!(
            "You don't have any projects yet. Create one at {}/dashboard, then re-run this command.",
            crate::cloud_client::dashboard_url(),
        ));
    }
    println!();
    println!("Pick a project:");
    for (i, p) in projects.iter().enumerate() {
        let org = p.org_slug.as_deref().unwrap_or("?");
        println!("  {:>2}. {}/{} ({})", i + 1, org, p.slug, p.name);
    }
    print!("Number: ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    let idx: usize = line
        .trim()
        .parse::<usize>()
        .map_err(|_| "Not a number.".to_string())?;
    if idx == 0 || idx > projects.len() {
        return Err("Out of range.".into());
    }
    Ok(projects[idx - 1].slug.clone())
}

/// Walk up from cwd looking for `.pylon/project`. Returns the
/// trimmed first line of the file (the slug) or None.
fn read_context_file() -> Option<String> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let candidate = dir.join(CONTEXT_FILENAME);
        if candidate.is_file() {
            if let Ok(contents) = fs::read_to_string(&candidate) {
                let slug = contents.lines().next()?.trim().to_string();
                if !slug.is_empty() {
                    return Some(slug);
                }
            }
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Write the current project slug to `.pylon/project` in the cwd.
/// Used by `pylon projects use <slug>`. Creates `.pylon/` if missing
/// + adds the dir to `.gitignore` if a `.git` sibling exists, so the
/// per-developer context doesn't end up checked in.
pub fn write_context_file(slug: &str) -> std::io::Result<PathBuf> {
    let cwd = std::env::current_dir()?;
    let pylon_dir = cwd.join(".pylon");
    fs::create_dir_all(&pylon_dir)?;
    let path = pylon_dir.join("project");
    fs::write(&path, format!("{slug}\n"))?;
    maybe_add_to_gitignore(&cwd);
    Ok(path)
}

/// Best-effort: append `.pylon/` to .gitignore so the local context
/// doesn't leak into commits. Silent failure — no .git, no
/// .gitignore write access, no problem.
fn maybe_add_to_gitignore(repo_root: &std::path::Path) {
    let gitignore = repo_root.join(".gitignore");
    let needle = ".pylon/";
    if let Ok(existing) = fs::read_to_string(&gitignore) {
        if existing.lines().any(|l| l.trim() == needle) {
            return;
        }
        let _ = fs::write(
            &gitignore,
            format!(
                "{}\n# pylon CLI per-project context\n{}\n",
                existing.trim_end(),
                needle
            ),
        );
    }
    // No existing .gitignore → skip. Most repos have one, and creating
    // it just for this entry would be presumptuous (might break a
    // `git status` workflow the user has elsewhere).
}

/// Delete `.pylon/project` from cwd. Used by `pylon projects use ""`
/// to clear context. Idempotent.
pub fn clear_context_file() -> std::io::Result<bool> {
    let cwd = std::env::current_dir()?;
    let path = cwd.join(".pylon").join("project");
    if path.exists() {
        fs::remove_file(&path)?;
        return Ok(true);
    }
    Ok(false)
}
