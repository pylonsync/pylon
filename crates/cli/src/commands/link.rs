//! `pylon link` — connect a Pylon Cloud project to a GitHub repo for
//! auto-deploy.
//!
//! Flow (Vercel-style — one command, run from the project directory):
//!
//!   1. Resolve project slug: --project flag → `.pylon/project` pin →
//!      interactive picker. getProjectForCli → projectId + orgId.
//!   2. Detect repo / branch / root-dir defaults from local git
//!      (`git rev-parse`, `git config remote.origin.url`,
//!      `git symbolic-ref`). Auto-fills prompt defaults; flags
//!      override.
//!   3. getOrgGithubConnection({orgId}). If the org isn't connected:
//!      bind a 127.0.0.1:0 loopback listener, beginGithubInstall
//!      with source=cli + the loopback URL as returnTo, open the
//!      GitHub install page in the browser, wait for the redirect.
//!      The dashboard's existing /github/installed page does the
//!      completeGithubInstall call (server validates the CSRF nonce
//!      + persists the installationId), then redirects to our
//!      loopback. We respond with a friendly close-tab page and
//!      proceed.
//!   4. listOrgGithubRepos({orgId}) → interactive picker (or
//!      validate --repo against the list).
//!   5. Branch (default: chosen repo's defaultBranch; falls back to
//!      local HEAD), root-dir (default: `git rev-parse --show-prefix`).
//!   6. updateProjectGitConfig({projectId, gitRepo, gitBranch,
//!      gitRootDir}).
//!   7. Print summary. Next push to the chosen branch fires the
//!      webhook → auto-deploy.
//!
//! Security note on the loopback: the server-side returnTo allowlist
//! in completeGithubInstall.ts only permits `http://127.0.0.1:<port>`
//! / `http://localhost:<port>` when the originating GithubInstallIntent
//! row has `source: "cli"`. A dashboard intent can never resolve to a
//! loopback returnTo even if the URL is tampered with — beginGithubInstall
//! is what sets `source`, not the client.

use std::io::{self, BufRead, BufReader, Write};
use std::net::TcpListener;
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

use pylon_kernel::ExitCode;
use serde::{Deserialize, Serialize};

use crate::cloud_client::{post_json, require_credentials, Credentials};
use crate::output;
use crate::project_context::resolve_project_slug;

#[derive(Deserialize)]
struct ProjectForCli {
    id: String,
    #[serde(rename = "orgId")]
    org_id: String,
    slug: String,
}

#[derive(Deserialize)]
struct OrgGithubConnection {
    connected: bool,
    #[serde(rename = "installationId")]
    #[allow(dead_code)]
    installation_id: Option<String>,
    #[serde(rename = "accountLogin")]
    account_login: Option<String>,
    #[serde(rename = "installUrl")]
    install_url: Option<String>,
    #[allow(dead_code)]
    #[serde(rename = "manageUrl")]
    manage_url: Option<String>,
}

#[derive(Deserialize)]
struct BeginInstall {
    token: String,
}

#[derive(Deserialize, Clone)]
struct GithubRepo {
    #[serde(rename = "fullName")]
    full_name: String,
    #[serde(rename = "defaultBranch")]
    default_branch: String,
    private: bool,
}

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let creds = match require_credentials() {
        Ok(c) => c,
        Err(e) => {
            output::print_error(&e);
            eprintln!("  Run: pylon login");
            return ExitCode::Usage;
        }
    };

    let opts = match LinkOpts::parse(args) {
        Ok(o) => o,
        Err(e) => {
            output::print_error(&e);
            eprintln!(
                "Usage: pylon link [--project <slug>] [--repo <owner/repo>] \
                 [--branch <name>] [--root-dir <path>] [--yes] [--no-browser]"
            );
            return ExitCode::Usage;
        }
    };

    // Step 1: resolve project.
    let slug = match opts.project_slug.clone() {
        Some(s) => s,
        None => match resolve_project_slug(args, &creds, json_mode || opts.yes) {
            Ok(s) => s,
            Err(e) => {
                output::print_error(&e);
                eprintln!("  Set the active project: pylon projects use <slug>");
                return ExitCode::Usage;
            }
        },
    };
    let project: ProjectForCli = match post_json(
        &creds,
        "/api/fn/getProjectForCli",
        &SlugArgs { slug: &slug },
    ) {
        Ok(p) => p,
        Err(e) => {
            output::print_error(&format!("Could not resolve project \"{slug}\": {e}"));
            return ExitCode::Error;
        }
    };

    // Step 2: local git detection. None on each is fine — used only as
    // prompt defaults; the explicit flags + interactive picker fill the gap.
    let git = GitContext::detect_in_cwd();

    // Step 3: org GitHub App install state.
    let conn: OrgGithubConnection = match post_json(
        &creds,
        "/api/fn/getOrgGithubConnection",
        &OrgArgs {
            org_id: &project.org_id,
        },
    ) {
        Ok(c) => c,
        Err(e) => {
            output::print_error(&format!("getOrgGithubConnection failed: {e}"));
            return ExitCode::Error;
        }
    };

    if !conn.connected {
        if json_mode || opts.yes {
            // Non-interactive path can't open a browser + wait. Bail
            // with an actionable message instead of hanging.
            output::print_error(
                "GitHub App not yet installed on this org. Re-run interactively (drop --yes) to install it.",
            );
            if let Some(u) = conn.install_url {
                eprintln!("  Or install manually: {u}");
            }
            return ExitCode::Usage;
        }
        if let Err(e) = install_github_app(&creds, &project, &conn, opts.no_browser) {
            output::print_error(&e);
            return ExitCode::Error;
        }
    } else if !json_mode {
        let who = conn.account_login.as_deref().unwrap_or("?");
        println!("✓ GitHub App connected (account: {who})");
    }

    // Step 4: repo. --repo flag wins; else interactive picker.
    let repos: Vec<GithubRepo> = match post_json(
        &creds,
        "/api/fn/listOrgGithubRepos",
        &OrgArgs {
            org_id: &project.org_id,
        },
    ) {
        Ok(r) => r,
        Err(e) => {
            output::print_error(&format!("listOrgGithubRepos failed: {e}"));
            return ExitCode::Error;
        }
    };
    let (git_repo, default_branch) = match resolve_repo(&opts, &repos, &git, json_mode || opts.yes)
    {
        Ok(pair) => pair,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Usage;
        }
    };

    // Step 5: branch + rootDir.
    let git_branch = opts
        .branch
        .clone()
        .or_else(|| default_branch.clone())
        .or_else(|| git.head_branch.clone())
        .unwrap_or_else(|| "main".to_string());
    let git_root_dir = opts
        .root_dir
        .clone()
        .or_else(|| git.repo_prefix.clone())
        .unwrap_or_default();

    if !opts.yes && !json_mode {
        let confirmed = confirm_summary(&project.slug, &git_repo, &git_branch, &git_root_dir);
        if !confirmed {
            output::print_error("Aborted by user.");
            return ExitCode::Usage;
        }
    }

    // Step 6: persist.
    let _: serde_json::Value = match post_json(
        &creds,
        "/api/fn/updateProjectGitConfig",
        &UpdateGitArgs {
            project_id: &project.id,
            git_repo: &git_repo,
            git_branch: &git_branch,
            git_root_dir: &git_root_dir,
        },
    ) {
        Ok(v) => v,
        Err(e) => {
            output::print_error(&format!("updateProjectGitConfig failed: {e}"));
            return ExitCode::Error;
        }
    };

    if json_mode {
        let out = serde_json::json!({
            "ok": true,
            "projectSlug": project.slug,
            "projectId": project.id,
            "orgId": project.org_id,
            "gitRepo": git_repo,
            "gitBranch": git_branch,
            "gitRootDir": git_root_dir,
        });
        println!("{}", serde_json::to_string(&out).unwrap_or_default());
    } else {
        println!();
        println!("✓ Linked {} → github.com/{}", project.slug, git_repo);
        println!("  Branch:   {git_branch}");
        if !git_root_dir.is_empty() {
            println!("  Root dir: {git_root_dir}");
        } else {
            println!("  Root dir: (repo root)");
        }
        println!();
        println!("Next push to {git_branch} will auto-deploy.");
        println!(
            "  View: {}/dashboard/projects/{}/settings/git",
            creds.cloud_url.trim_end_matches('/'),
            project.slug
        );
    }
    ExitCode::Ok
}

#[derive(Default)]
struct LinkOpts {
    project_slug: Option<String>,
    repo: Option<String>,
    branch: Option<String>,
    root_dir: Option<String>,
    yes: bool,
    no_browser: bool,
}

impl LinkOpts {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut opts = LinkOpts::default();
        let mut i = 0;
        while i < args.len() {
            let a = &args[i];
            match a.as_str() {
                "link" | "projects" => {} // command words, ignored
                "--yes" | "-y" => opts.yes = true,
                "--no-browser" => opts.no_browser = true,
                "--project" | "-p" => {
                    opts.project_slug =
                        Some(args.get(i + 1).cloned().ok_or("--project needs a value")?);
                    i += 1;
                }
                "--repo" => {
                    opts.repo = Some(args.get(i + 1).cloned().ok_or("--repo needs a value")?);
                    i += 1;
                }
                "--branch" => {
                    opts.branch = Some(args.get(i + 1).cloned().ok_or("--branch needs a value")?);
                    i += 1;
                }
                "--root-dir" => {
                    opts.root_dir =
                        Some(args.get(i + 1).cloned().ok_or("--root-dir needs a value")?);
                    i += 1;
                }
                s if s.starts_with("--project=") => {
                    opts.project_slug = Some(s.trim_start_matches("--project=").to_string());
                }
                s if s.starts_with("--repo=") => {
                    opts.repo = Some(s.trim_start_matches("--repo=").to_string());
                }
                s if s.starts_with("--branch=") => {
                    opts.branch = Some(s.trim_start_matches("--branch=").to_string());
                }
                s if s.starts_with("--root-dir=") => {
                    opts.root_dir = Some(s.trim_start_matches("--root-dir=").to_string());
                }
                "--json" => {} // handled by main dispatch
                s if s.starts_with('-') => {
                    return Err(format!("unknown flag: {s}"));
                }
                _ => {} // tolerate stray positionals
            }
            i += 1;
        }
        Ok(opts)
    }
}

#[derive(Default)]
struct GitContext {
    /// Canonical owner/repo (lowercased), parsed from remote.origin.url.
    origin_full_name: Option<String>,
    /// Branch from `git symbolic-ref --short HEAD`.
    head_branch: Option<String>,
    /// `git rev-parse --show-prefix` (cwd-relative to repo root).
    repo_prefix: Option<String>,
}

impl GitContext {
    fn detect_in_cwd() -> Self {
        let mut g = GitContext::default();
        if let Some(url) = run_git(&["config", "--get", "remote.origin.url"]) {
            g.origin_full_name = canonicalize_repo_url(&url);
        }
        g.head_branch = run_git(&["symbolic-ref", "--short", "HEAD"]);
        g.repo_prefix = run_git(&["rev-parse", "--show-prefix"]).map(|p| {
            // Strip trailing slash for parity with the dashboard's normalization.
            p.trim_end_matches('/').to_string()
        });
        g
    }
}

fn run_git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Pull `owner/repo` (lowercase) out of every common remote URL form.
/// Returns None when the URL doesn't look like a github.com remote —
/// caller falls back to the interactive picker / explicit --repo flag.
fn canonicalize_repo_url(url: &str) -> Option<String> {
    let s = url.trim();
    // https://github.com/owner/repo[.git]
    if let Some(rest) = s
        .strip_prefix("https://github.com/")
        .or_else(|| s.strip_prefix("http://github.com/"))
    {
        return canonical_owner_repo(rest);
    }
    // git@github.com:owner/repo[.git]
    if let Some(rest) = s.strip_prefix("git@github.com:") {
        return canonical_owner_repo(rest);
    }
    // ssh://git@github.com/owner/repo[.git]
    if let Some(rest) = s.strip_prefix("ssh://git@github.com/") {
        return canonical_owner_repo(rest);
    }
    None
}

fn canonical_owner_repo(s: &str) -> Option<String> {
    let trimmed = s.trim_end_matches('/').trim_end_matches(".git");
    let mut parts = trimmed.splitn(2, '/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some(format!("{}/{}", owner.to_lowercase(), repo.to_lowercase()))
}

#[derive(Serialize)]
struct SlugArgs<'a> {
    slug: &'a str,
}

#[derive(Serialize)]
struct OrgArgs<'a> {
    #[serde(rename = "orgId")]
    org_id: &'a str,
}

#[derive(Serialize)]
struct BeginInstallArgs<'a> {
    #[serde(rename = "orgId")]
    org_id: &'a str,
    #[serde(rename = "projectId")]
    project_id: &'a str,
    #[serde(rename = "returnTo")]
    return_to: &'a str,
    source: &'static str,
}

#[derive(Serialize)]
struct UpdateGitArgs<'a> {
    #[serde(rename = "projectId")]
    project_id: &'a str,
    #[serde(rename = "gitRepo")]
    git_repo: &'a str,
    #[serde(rename = "gitBranch")]
    git_branch: &'a str,
    #[serde(rename = "gitRootDir")]
    git_root_dir: &'a str,
}

/// Run the loopback-based GitHub App install round-trip. Returns Ok when
/// the dashboard's /github/installed page successfully bounces back to
/// our loopback — the server has already persisted the installation by
/// that point, no installation_id needed on our end.
fn install_github_app(
    creds: &Credentials,
    project: &ProjectForCli,
    conn: &OrgGithubConnection,
    no_browser: bool,
) -> Result<(), String> {
    let install_url_base = conn
        .install_url
        .as_deref()
        .ok_or("server didn't return an install URL (GITHUB_APP_SLUG missing?)")?;

    // Loopback: 127.0.0.1:0 → kernel picks a free ephemeral port.
    // 127.0.0.1 (not 0.0.0.0) — never expose to LAN.
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|e| format!("loopback bind failed: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("loopback addr: {e}"))?
        .port();
    let return_to = format!("http://127.0.0.1:{port}/cli-callback");

    let begin: BeginInstall = post_json(
        creds,
        "/api/fn/beginGithubInstall",
        &BeginInstallArgs {
            org_id: &project.org_id,
            project_id: &project.id,
            return_to: &return_to,
            source: "cli",
        },
    )
    .map_err(|e| format!("beginGithubInstall failed: {e}"))?;

    let full_install_url = if install_url_base.contains('?') {
        format!("{install_url_base}&state={}", begin.token)
    } else {
        format!("{install_url_base}?state={}", begin.token)
    };

    println!();
    println!("→ Install the Pylon Cloud GitHub App:");
    println!("  {full_install_url}");
    if no_browser {
        println!("  (--no-browser: open the URL above to continue)");
    } else {
        let _ = open_browser(&full_install_url);
        println!("  Opening browser… waiting for install callback");
    }
    println!("  (Ctrl-C to abort, 10-minute timeout)");
    println!();

    // Block on the loopback for up to ~10 minutes (matches the intent
    // TTL). Once we accept a connection, parse the HTTP request line
    // just enough to confirm it's the /cli-callback path, then respond
    // with a friendly close-tab page. We DON'T parse query params —
    // the install state is already persisted by completeGithubInstall
    // on the dashboard before this fires.
    listener
        .set_nonblocking(false)
        .map_err(|e| format!("listener mode: {e}"))?;

    let (tx, rx) = mpsc::channel::<Result<(), String>>();
    let timeout = Duration::from_secs(10 * 60);
    let listener_handle = std::thread::spawn(move || {
        let result = match listener.accept() {
            Ok((mut stream, _addr)) => {
                // Parse the request line only (no need to read body).
                let mut reader = BufReader::new(&stream);
                let mut request_line = String::new();
                let _ = reader.read_line(&mut request_line);
                // Drain headers so the client doesn't block on EOF.
                let mut header = String::new();
                loop {
                    header.clear();
                    if reader.read_line(&mut header).is_err() {
                        break;
                    }
                    if header == "\r\n" || header.is_empty() {
                        break;
                    }
                }
                let path_ok = request_line.starts_with("GET /cli-callback");
                let body = if path_ok {
                    SUCCESS_HTML
                } else {
                    NOT_FOUND_HTML
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
                if path_ok {
                    Ok(())
                } else {
                    Err(format!(
                        "loopback got unexpected request: {}",
                        request_line.trim_end()
                    ))
                }
            }
            Err(e) => Err(format!("loopback accept: {e}")),
        };
        let _ = tx.send(result);
    });

    match rx.recv_timeout(timeout) {
        Ok(Ok(())) => {
            // Reap the listener thread; it's already done.
            let _ = listener_handle.join();
            println!("✓ GitHub App installed.");
            Ok(())
        }
        Ok(Err(e)) => Err(e),
        Err(_) => Err(
            "timed out waiting for GitHub install callback (10 minutes). \
             Did you complete the install in the browser?"
                .to_string(),
        ),
    }
}

const SUCCESS_HTML: &str = r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>Pylon — GitHub App installed</title>
<style>
body{font-family:-apple-system,BlinkMacSystemFont,Segoe UI,sans-serif;display:grid;place-items:center;min-height:100vh;margin:0;background:#fafafa;color:#111}
.card{max-width:32rem;padding:2rem;border:1px solid #e5e5e5;border-radius:12px;background:#fff;text-align:center}
.dot{width:8px;height:8px;border-radius:50%;background:#22c55e;display:inline-block;margin-right:6px;vertical-align:middle}
h1{font-size:1.125rem;margin:0 0 .5rem;font-weight:600}
p{margin:.25rem 0;color:#555;font-size:.9rem;line-height:1.5}
</style></head>
<body><div class="card">
<h1><span class="dot"></span>GitHub App installed</h1>
<p>You can close this tab and return to your terminal.</p>
<p style="margin-top:1rem;font-size:.8rem;color:#888"><code>pylon link</code> is finishing up&hellip;</p>
</div></body></html>"#;

const NOT_FOUND_HTML: &str = r#"<!DOCTYPE html>
<html><body><p>Pylon CLI loopback — unexpected request. You can close this tab.</p></body></html>"#;

fn resolve_repo(
    opts: &LinkOpts,
    repos: &[GithubRepo],
    git: &GitContext,
    non_interactive: bool,
) -> Result<(String, Option<String>), String> {
    // Explicit flag wins. Validate against the list when present so the
    // user gets an early error for "the App can't see this repo".
    if let Some(want) = &opts.repo {
        let canonical = canonical_owner_repo(want)
            .or_else(|| canonicalize_repo_url(want))
            .ok_or_else(|| format!("--repo \"{want}\" must be owner/repo or a github.com URL"))?;
        let match_ = repos
            .iter()
            .find(|r| r.full_name.eq_ignore_ascii_case(&canonical));
        let default = match_.map(|r| r.default_branch.clone());
        if match_.is_none() && !repos.is_empty() {
            eprintln!(
                "  Warning: repo \"{canonical}\" not visible to the installed Pylon Cloud App."
            );
            eprintln!("  Manage the App's access: open the GitHub App settings.");
        }
        return Ok((canonical, default));
    }

    if repos.is_empty() {
        return Err(
            "GitHub App has no repos to pick from. Open the App settings on github.com and \
             grant access to at least one repo, then re-run."
                .to_string(),
        );
    }

    if non_interactive {
        // --yes path: pick the auto-detected origin if it matches a
        // visible repo, else fail loudly.
        if let Some(detected) = &git.origin_full_name {
            if let Some(r) = repos
                .iter()
                .find(|r| r.full_name.eq_ignore_ascii_case(detected))
            {
                return Ok((r.full_name.clone(), Some(r.default_branch.clone())));
            }
        }
        return Err(
            "--yes requires --repo or a detectable git remote that matches a visible repo."
                .to_string(),
        );
    }

    // Interactive picker. Pre-highlight the auto-detected origin if
    // it's in the list.
    let preselect_idx = git.origin_full_name.as_deref().and_then(|n| {
        repos
            .iter()
            .position(|r| r.full_name.eq_ignore_ascii_case(n))
    });

    println!();
    println!("Pick a repo to deploy from:");
    for (i, r) in repos.iter().enumerate() {
        let marker = if Some(i) == preselect_idx { "→" } else { " " };
        let lock = if r.private { " (private)" } else { "" };
        println!("  {marker} [{:>2}] {}{}", i + 1, r.full_name, lock);
    }
    let default_idx = preselect_idx.unwrap_or(0);
    print!(
        "Number [1-{}]{}: ",
        repos.len(),
        if preselect_idx.is_some() {
            format!(" (default {})", default_idx + 1)
        } else {
            String::new()
        }
    );
    let _ = io::stdout().flush();
    let mut line = String::new();
    if io::stdin().lock().read_line(&mut line).is_err() {
        return Err("Failed to read selection.".to_string());
    }
    let choice = line.trim();
    let idx = if choice.is_empty() {
        default_idx
    } else {
        match choice.parse::<usize>() {
            Ok(n) if n >= 1 && n <= repos.len() => n - 1,
            _ => return Err(format!("Invalid choice: \"{choice}\"")),
        }
    };
    let r = &repos[idx];
    Ok((r.full_name.clone(), Some(r.default_branch.clone())))
}

fn confirm_summary(slug: &str, repo: &str, branch: &str, root_dir: &str) -> bool {
    println!();
    println!("Will link:");
    println!("  Project:  {slug}");
    println!("  Repo:     github.com/{repo}");
    println!("  Branch:   {branch}");
    if root_dir.is_empty() {
        println!("  Root dir: (repo root)");
    } else {
        println!("  Root dir: {root_dir}");
    }
    print!("Confirm? [Y/n] ");
    let _ = io::stdout().flush();
    let mut line = String::new();
    if io::stdin().lock().read_line(&mut line).is_err() {
        return false;
    }
    let t = line.trim().to_lowercase();
    t.is_empty() || t == "y" || t == "yes"
}

fn open_browser(url: &str) -> io::Result<()> {
    let (cmd, args): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("open", vec![url])
    } else if cfg!(target_os = "windows") {
        ("cmd", vec!["/C", "start", "", url])
    } else {
        ("xdg-open", vec![url])
    };
    Command::new(cmd)
        .args(&args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalizes_https_repo() {
        assert_eq!(
            canonicalize_repo_url("https://github.com/Owner/Repo.git"),
            Some("owner/repo".to_string())
        );
        assert_eq!(
            canonicalize_repo_url("https://github.com/foo/bar"),
            Some("foo/bar".to_string())
        );
        assert_eq!(
            canonicalize_repo_url("https://github.com/foo/bar/"),
            Some("foo/bar".to_string())
        );
    }

    #[test]
    fn canonicalizes_ssh_repo() {
        assert_eq!(
            canonicalize_repo_url("git@github.com:Acme/My-Repo.git"),
            Some("acme/my-repo".to_string())
        );
        assert_eq!(
            canonicalize_repo_url("ssh://git@github.com/acme/my-repo"),
            Some("acme/my-repo".to_string())
        );
    }

    #[test]
    fn rejects_non_github_url() {
        assert_eq!(canonicalize_repo_url("https://gitlab.com/foo/bar"), None);
        assert_eq!(canonicalize_repo_url("malformed"), None);
        assert_eq!(canonicalize_repo_url("https://github.com/foo"), None);
    }

    #[test]
    fn opts_parse_basic() {
        let args = vec![
            "link".to_string(),
            "--project".to_string(),
            "chat-api".to_string(),
            "--repo".to_string(),
            "Pylonsync/Pylon".to_string(),
            "--branch=main".to_string(),
            "--root-dir=examples/chat".to_string(),
            "--yes".to_string(),
        ];
        let opts = LinkOpts::parse(&args).unwrap();
        assert_eq!(opts.project_slug.as_deref(), Some("chat-api"));
        assert_eq!(opts.repo.as_deref(), Some("Pylonsync/Pylon"));
        assert_eq!(opts.branch.as_deref(), Some("main"));
        assert_eq!(opts.root_dir.as_deref(), Some("examples/chat"));
        assert!(opts.yes);
    }

    #[test]
    fn opts_rejects_unknown_flag() {
        let args = vec!["link".to_string(), "--what".to_string()];
        assert!(LinkOpts::parse(&args).is_err());
    }
}
