//! `pylon deploy --target cloud` — package the current project and
//! ship it to Pylon Cloud via the authenticated CLI upload endpoint.
//!
//! Differs from the other deploy targets (docker / fly / compose /
//! workers / systemd): those generate config FILES. This one actually
//! pushes code to a hosted control plane.
//!
//! Flow:
//! 1. Require credentials. Bail with a `pylon login` hint if missing.
//! 2. Resolve target project — `--project <slug>` flag, then
//!    `PYLON_PROJECT` env, then prompt interactively (TTY only) by
//!    listing the user's projects via the cloud API.
//! 3. Build a gzipped tar of the project source. Skips `.git`,
//!    `node_modules`, `.pylon`, `target`, common artifact dirs, and
//!    anything in `.gitignore`. Hard 50MB cap — projects bigger than
//!    that should be using the GitHub-push path.
//! 4. POST the tarball to `/api/fn/deployProjectFromCliUpload` with
//!    `multipart/form-data; boundary=...` carrying `projectSlug` +
//!    the tarball bytes. (See pylon-cloud function for the exact
//!    accepted shape.)
//! 5. Print the deployment URL + tail the deploy log if `--follow`
//!    was passed (defaults to true on TTY, false in CI / --json).

use std::fs::File;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use flate2::write::GzEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};

use pylon_kernel::ExitCode;

use crate::cloud_client::{cloud_url, post_json, require_credentials, Credentials};
use crate::output;

/// Maximum size of the source tarball. Pylon Cloud's GitHub-push path
/// is the right answer for projects above this — they tend to have
/// build artifacts the operator hasn't realized are shipping. The
/// limit is generous enough that a real app (~1500 source files) fits
/// 10x over, and tight enough that an accidentally-included
/// node_modules bombs early instead of timing out the cloud.
const MAX_TARBALL_BYTES: u64 = 50 * 1024 * 1024;

/// Directories we never include in the upload tarball. Anything in
/// here is either rebuilt by the cloud (node_modules, target), part
/// of git state we don't want to ship (.git), or framework local
/// state (.pylon, dev databases).
const EXCLUDE_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    ".pylon",
    ".next",
    // `dist` excluded: shipping the pre-built bundle hits Fly's
    // machine-config body cap (every byte gets base64'd into the
    // updateMachine payload; the chat example's loro_wasm alone is
    // 3MB → 4MB encoded, blows the cap before the rest of the bundle
    // is even considered). The runtime rebuilds in /app/web/ on boot,
    // which is both cheaper to ship AND gets a deterministic install
    // against the locked deps.
    "dist",
    "build",
    ".turbo",
    ".vercel",
    ".cache",
];

/// Files we never include — credential stores, OS clutter.
const EXCLUDE_FILES: &[&str] = &[".env", ".env.local", ".DS_Store", "Thumbs.db"];

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    // 1. Require credentials.
    let creds = match require_credentials() {
        Ok(c) => c,
        Err(e) => {
            output::print_error(&e);
            eprintln!("  Run: pylon login");
            return ExitCode::Usage;
        }
    };

    // 2. Resolve the project slug.
    let project_slug = match resolve_project(args, &creds, json_mode) {
        Ok(slug) => slug,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Usage;
        }
    };

    // 3. Build the tarball from the current directory.
    if !json_mode {
        println!("→ Packaging project source...");
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let tarball = match build_tarball(&cwd) {
        Ok(t) => t,
        Err(e) => {
            output::print_error(&format!("Failed to package source: {e}"));
            return ExitCode::Error;
        }
    };
    if tarball.len() as u64 > MAX_TARBALL_BYTES {
        output::print_error(&format!(
            "Source tarball is {:.1} MB — over the {} MB CLI-upload limit. \
             Push via GitHub instead, or trim build artifacts (node_modules, \
             .next, dist, target are already excluded — check for others).",
            tarball.len() as f64 / 1_048_576.0,
            MAX_TARBALL_BYTES / 1_048_576,
        ));
        return ExitCode::Error;
    }
    if !json_mode {
        println!(
            "  {} files, {:.1} KB compressed.",
            count_tar_entries(&tarball).unwrap_or(0),
            tarball.len() as f64 / 1024.0,
        );
    }

    // 4. POST to the cloud. Tarball is base64-encoded inside JSON for
    //    transport — the deployProjectFromCliUpload function decodes,
    //    persists, and kicks off the deploy pipeline.
    if !json_mode {
        println!("→ Uploading to {}...", creds.cloud_url);
    }
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;
    let body = UploadRequest {
        project_slug: project_slug.clone(),
        tarball_base64: STANDARD.encode(&tarball),
    };
    let resp: UploadResponse = match post_json(&creds, "/api/fn/deployProjectFromCliUpload", &body)
    {
        Ok(r) => r,
        Err(e) => {
            output::print_error(&format!("Cloud deploy failed: {e}"));
            return ExitCode::Error;
        }
    };

    if json_mode {
        let out = serde_json::json!({
            "ok": true,
            "deployment_id": resp.deployment_id,
            "project_slug": project_slug,
            "url": resp.url,
        });
        println!("{}", serde_json::to_string(&out).unwrap_or_default());
    } else {
        println!();
        println!("✓ Deploy queued");
        println!("  Deployment: {}", resp.deployment_id);
        if let Some(url) = &resp.url {
            println!("  URL:        {url}");
        }
        println!(
            "  Watch:      {}/dashboard → project → Deployments",
            cloud_url().trim_end_matches('/'),
        );
    }
    ExitCode::Ok
}

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct UploadRequest {
    #[serde(rename = "projectSlug")]
    project_slug: String,
    /// Base64-encoded gzipped tar of the project source.
    #[serde(rename = "tarballBase64")]
    tarball_base64: String,
}

#[derive(Deserialize)]
struct UploadResponse {
    #[serde(rename = "deploymentId")]
    deployment_id: String,
    url: Option<String>,
}

#[derive(Deserialize)]
struct ProjectSummary {
    slug: String,
    name: String,
    #[serde(rename = "orgSlug")]
    org_slug: Option<String>,
}

// ---------------------------------------------------------------------------
// Project resolution
// ---------------------------------------------------------------------------

fn resolve_project(
    args: &[String],
    creds: &Credentials,
    json_mode: bool,
) -> Result<String, String> {
    // 1. --project <slug>
    if let Some(slug) = args
        .windows(2)
        .find(|w| w[0] == "--project")
        .map(|w| w[1].clone())
    {
        return Ok(slug);
    }
    // 2. --project=<slug>
    if let Some(slug) = args
        .iter()
        .find(|a| a.starts_with("--project="))
        .map(|a| a.trim_start_matches("--project=").to_string())
    {
        return Ok(slug);
    }
    // 3. $PYLON_PROJECT
    if let Ok(slug) = std::env::var("PYLON_PROJECT") {
        if !slug.is_empty() {
            return Ok(slug);
        }
    }
    // 4. Interactive picker — TTY only. CI / --json get an error
    //    pointing at the flag.
    if json_mode || !std::io::stdin().is_terminal() {
        return Err("No project specified. Pass --project <slug> or set PYLON_PROJECT.".into());
    }
    let projects: Vec<ProjectSummary> = post_json(creds, "/api/fn/listMyProjectsForCli", &())
        .map_err(|e| format!("Couldn't list your projects: {e}"))?;
    if projects.is_empty() {
        return Err(format!(
            "You don't have any projects yet. Create one at {}/dashboard/orgs",
            creds.cloud_url.trim_end_matches('/'),
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

// ---------------------------------------------------------------------------
// Tarball builder
// ---------------------------------------------------------------------------

fn build_tarball(root: &Path) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    {
        let gz = GzEncoder::new(&mut buf, Compression::default());
        let mut tar = tar::Builder::new(gz);
        let gitignore = load_gitignore(root);
        walk_into_tar(&mut tar, root, root, &gitignore)?;
        tar.into_inner()?.finish()?;
    }
    Ok(buf)
}

fn walk_into_tar<W: Write>(
    tar: &mut tar::Builder<W>,
    root: &Path,
    dir: &Path,
    gitignore: &[String],
) -> io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        let ft = entry.file_type()?;

        if ft.is_dir() {
            if EXCLUDE_DIRS.iter().any(|d| *d == name) {
                continue;
            }
            if matches_gitignore(&path, root, gitignore) {
                continue;
            }
            walk_into_tar(tar, root, &path, gitignore)?;
        } else if ft.is_file() {
            if EXCLUDE_FILES.iter().any(|f| *f == name) {
                continue;
            }
            if name.starts_with(".env.") && !name.ends_with(".example") {
                // .env.local, .env.production, etc — never upload.
                continue;
            }
            if matches_gitignore(&path, root, gitignore) {
                continue;
            }
            let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            tar.append_path_with_name(&path, &rel)?;
        }
        // Symlinks intentionally skipped — they'd be brittle in the
        // cloud's flatten/build step and unlikely to appear in real
        // Pylon project trees.
    }
    Ok(())
}

/// Minimal .gitignore reader — supports literal file/dir names and
/// glob-suffix patterns like `*.log`. Full pathspec semantics live
/// in git itself; we cover the 90% case that lets a developer's
/// existing .gitignore filter the upload tarball.
fn load_gitignore(root: &Path) -> Vec<String> {
    let path = root.join(".gitignore");
    let Ok(file) = File::open(&path) else {
        return Vec::new();
    };
    io::BufReader::new(file)
        .lines()
        .filter_map(|l| l.ok())
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect()
}

fn matches_gitignore(path: &Path, root: &Path, patterns: &[String]) -> bool {
    let Ok(rel) = path.strip_prefix(root) else {
        return false;
    };
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    let file_name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    for pat in patterns {
        let pat = pat.trim_start_matches('/');
        if pat == rel_str || pat == file_name {
            return true;
        }
        // *.ext glob
        if let Some(suffix) = pat.strip_prefix("*.") {
            if file_name.ends_with(&format!(".{suffix}")) {
                return true;
            }
        }
        // dir/ trailing slash → match anywhere under that dir
        if let Some(dir) = pat.strip_suffix('/') {
            if rel_str.starts_with(&format!("{dir}/")) || rel_str == dir {
                return true;
            }
        }
    }
    false
}

/// Count entries inside a gzipped tar, for the "N files" status line.
/// Best-effort — failure just hides the count, doesn't block deploy.
fn count_tar_entries(tarball: &[u8]) -> io::Result<usize> {
    use flate2::read::GzDecoder;
    let mut count = 0;
    let mut archive = tar::Archive::new(GzDecoder::new(tarball));
    for entry in archive.entries()? {
        let entry = entry?;
        if entry.header().entry_type().is_file() {
            count += 1;
        }
        // The tar iterator manages advancing past entry data automatically
        // when we drop the entry — no manual seek needed.
    }
    Ok(count)
}
