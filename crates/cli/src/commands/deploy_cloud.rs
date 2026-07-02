//! `pylon deploy --target cloud` — package the current project and
//! ship it to Pylon Cloud via the authenticated CLI upload endpoint.
//!
//! Differs from the other deploy targets (docker / fly / compose /
//! workers / systemd): those generate config FILES. This one actually
//! pushes code to a hosted control plane.
//!
//! Flow:
//! 1. Require credentials. Bail with a `pylon login` hint if missing.
//! 2. Resolve target project via the shared resolver
//!    (`project_context::resolve_project_slug`): `--project` flag →
//!    `PYLON_PROJECT` env → `.pylon/project` context file (written by
//!    `pylon projects use`) → global default → interactive picker
//!    (TTY only).
//! 3. Build a gzipped tar of the project source. Skips `.git`,
//!    `node_modules`, `.pylon`, `target`, common artifact dirs, and
//!    anything in `.gitignore`. Hard 50MB cap — projects bigger than
//!    that should be using the GitHub-push path.
//! 4. POST the tarball to `/api/fn/deployProjectFromCliUpload` with
//!    `multipart/form-data; boundary=...` carrying `projectSlug` +
//!    the tarball bytes. (See pylon-cloud function for the exact
//!    accepted shape.)
//! 5. Print the deployment id + URL. The upload QUEUES the deploy;
//!    `pylon deployments logs` follows the build server-side.

use std::fs::File;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use flate2::write::GzEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};

use pylon_kernel::ExitCode;

use crate::cloud_client::{post_json, require_credentials};
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

    // 2. Resolve the project slug — same resolver as every other
    //    cloud-aware command, so `pylon projects use <slug>` sticks
    //    for deploys too.
    let project_slug = match crate::project_context::resolve_project_slug(args, &creds, json_mode) {
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
    let workspace = detect_workspace(&cwd);

    // Fail fast when there's nothing deployable here: a single-app pack
    // with no app.ts at its root is exactly the tarball the server
    // rejects with MISSING_APP_TS — catch it before the upload with an
    // error that says where to cd.
    if workspace.is_none() && !cwd.join("app.ts").is_file() {
        output::print_error(
            "No app.ts in this directory — nothing to deploy from here. \
             Run `pylon deploy` from your app's root (the directory with \
             app.ts, e.g. apps/api in a monorepo).",
        );
        return ExitCode::Usage;
    }
    if let Some(ws) = &workspace {
        if !json_mode {
            println!(
                "  monorepo workspace — packing {} (app: {}) + {} workspace dep(s)",
                ws.root
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("workspace"),
                ws.app_subdir,
                ws.member_dirs.len().saturating_sub(1),
            );
        }
    }
    let tarball = match workspace
        .as_ref()
        .map(build_workspace_tarball)
        .unwrap_or_else(|| build_tarball(&cwd))
    {
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
        app_subdir: workspace.as_ref().map(|w| w.app_subdir.clone()),
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
        println!("  Build log:  pylon deployments logs");
        println!(
            "  Dashboard:  {}/dashboard → project → Deployments",
            crate::cloud_client::dashboard_url(),
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
    /// For a monorepo deploy: the app's path relative to the packed workspace
    /// root (e.g. "examples/market"). Absent for a single-app deploy. The
    /// builder installs at the workspace root and the runtime chdirs here.
    #[serde(rename = "appSubdir", skip_serializing_if = "Option::is_none")]
    app_subdir: Option<String>,
}

#[derive(Deserialize)]
struct UploadResponse {
    #[serde(rename = "deploymentId")]
    deployment_id: String,
    url: Option<String>,
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

// ---------------------------------------------------------------------------
// Monorepo deploy: pack the pruned workspace, not just the app dir.
//
// An app inside a bun/npm workspace can depend on sibling packages via
// `workspace:*`. Carved out alone, those don't resolve — an UNPUBLISHED one
// (e.g. examples/_shared) can't be recovered at all. The fix: ship the
// workspace so `bun install` resolves `workspace:*` locally (the builder runs
// the install at the workspace root and the runtime chdirs into the app
// subdir). We pack only the app + its TRANSITIVE workspace-dep closure so a
// giant monorepo's unrelated members don't bloat the upload.
// ---------------------------------------------------------------------------

/// The app at `app_subdir` inside the workspace rooted at `root`, plus
/// `member_dirs` (the app + the workspace packages it transitively depends on).
struct WorkspaceDeploy {
    root: PathBuf,
    app_subdir: String,
    member_dirs: Vec<PathBuf>,
}

/// Resolve the workspace context for `app_dir`, or `None` if it's a standalone
/// app (no ancestor declares `workspaces`, or it IS the workspace root).
fn detect_workspace(app_dir: &Path) -> Option<WorkspaceDeploy> {
    let app_dir = app_dir.canonicalize().ok()?;
    let (root, patterns) = find_workspace_root(&app_dir)?;
    let app_subdir = app_dir
        .strip_prefix(&root)
        .ok()?
        .to_string_lossy()
        .replace('\\', "/");
    if app_subdir.is_empty() {
        return None; // the app IS the workspace root — handled as standalone
    }
    let by_name = enumerate_members(&root, &patterns);
    // BFS the transitive workspace-dep closure from the app.
    let mut needed: std::collections::BTreeSet<PathBuf> = std::collections::BTreeSet::new();
    let mut queue = vec![app_dir.clone()];
    needed.insert(app_dir.clone());
    while let Some(dir) = queue.pop() {
        for dep in read_dep_names(&dir.join("package.json")) {
            if let Some(dep_dir) = by_name.get(&dep) {
                if needed.insert(dep_dir.clone()) {
                    queue.push(dep_dir.clone());
                }
            }
        }
    }
    Some(WorkspaceDeploy {
        root,
        app_subdir,
        member_dirs: needed.into_iter().collect(),
    })
}

/// Walk up from `start` for the nearest package.json with a non-empty
/// `workspaces`. Returns (canonical root dir, glob patterns).
fn find_workspace_root(start: &Path) -> Option<(PathBuf, Vec<String>)> {
    for dir in start.ancestors() {
        let Ok(text) = std::fs::read_to_string(dir.join("package.json")) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let patterns = workspaces_patterns(&v);
        if !patterns.is_empty() {
            return Some((dir.to_path_buf(), patterns));
        }
    }
    None
}

/// `workspaces` is either `["pkgs/*"]` or `{ "packages": ["pkgs/*"] }`.
fn workspaces_patterns(pkg: &serde_json::Value) -> Vec<String> {
    let ws = &pkg["workspaces"];
    let arr = if ws.is_array() {
        ws.as_array()
    } else {
        ws.get("packages").and_then(|p| p.as_array())
    };
    arr.map(|a| {
        a.iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect()
    })
    .unwrap_or_default()
}

/// Expand every workspace pattern under `root` → map of package name → dir.
fn enumerate_members(
    root: &Path,
    patterns: &[String],
) -> std::collections::HashMap<String, PathBuf> {
    let mut out = std::collections::HashMap::new();
    for pat in patterns {
        for dir in glob_dirs(root, pat) {
            let Ok(dir) = dir.canonicalize() else {
                continue;
            };
            if let Ok(text) = std::fs::read_to_string(dir.join("package.json")) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                    if let Some(name) = v.get("name").and_then(|n| n.as_str()) {
                        out.insert(name.to_string(), dir);
                    }
                }
            }
        }
    }
    out
}

/// Minimal workspace glob — expands `*` segments (the only wildcard bun
/// workspaces use: `packages/*`, `examples/*/web`).
fn glob_dirs(root: &Path, pattern: &str) -> Vec<PathBuf> {
    let mut frontier = vec![root.to_path_buf()];
    for seg in pattern.split('/').filter(|s| !s.is_empty()) {
        let mut next = Vec::new();
        for base in &frontier {
            if seg == "*" {
                if let Ok(rd) = std::fs::read_dir(base) {
                    for e in rd.flatten() {
                        if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                            next.push(e.path());
                        }
                    }
                }
            } else {
                let p = base.join(seg);
                if p.is_dir() {
                    next.push(p);
                }
            }
        }
        frontier = next;
    }
    frontier
}

/// dependency + devDependency + optionalDependency names from a package.json.
fn read_dep_names(pkg: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(pkg) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for field in ["dependencies", "devDependencies", "optionalDependencies"] {
        if let Some(obj) = v.get(field).and_then(|f| f.as_object()) {
            names.extend(obj.keys().cloned());
        }
    }
    names
}

/// Pack the pruned workspace: the root package.json (with `workspaces` narrowed
/// to the packed members, so `bun install` doesn't look for trimmed ones) + each
/// member dir at its workspace-relative path. No lockfile — the builder
/// fresh-resolves the pruned set (the monorepo lock references trimmed members).
fn build_workspace_tarball(ws: &WorkspaceDeploy) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    {
        let gz = GzEncoder::new(&mut buf, Compression::default());
        let mut tar = tar::Builder::new(gz);
        let gitignore = load_gitignore(&ws.root);

        let root_pkg = rewrite_root_workspaces(&ws.root, &ws.member_dirs)?;
        let mut header = tar::Header::new_gnu();
        header.set_size(root_pkg.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, "package.json", root_pkg.as_slice())?;

        for dir in &ws.member_dirs {
            walk_into_tar(&mut tar, &ws.root, dir, &gitignore)?;
        }
        tar.into_inner()?.finish()?;
    }
    Ok(buf)
}

/// Root package.json with `workspaces` rewritten to the packed members' paths.
fn rewrite_root_workspaces(root: &Path, members: &[PathBuf]) -> io::Result<Vec<u8>> {
    let text = std::fs::read_to_string(root.join("package.json"))?;
    let mut v: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let rels: Vec<String> = members
        .iter()
        .filter_map(|d| d.strip_prefix(root).ok())
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    v["workspaces"] = serde_json::json!(rels);
    let mut out =
        serde_json::to_vec_pretty(&v).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    out.push(b'\n');
    Ok(out)
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

#[cfg(test)]
mod workspace_deploy_tests {
    use super::*;
    use std::fs;

    fn write(p: &Path, body: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }

    // app → @scope/lib (workspace) → (nothing). @scope/unused is a member but
    // NOT a dep of app, so it must be pruned out.
    fn fake_workspace(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("pylon-ws-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        write(
            &root.join("package.json"),
            r#"{"name":"root","private":true,"workspaces":["packages/*"]}"#,
        );
        write(
            &root.join("packages/app/package.json"),
            r#"{"name":"@scope/app","dependencies":{"@scope/lib":"workspace:*","lodash":"^4"}}"#,
        );
        write(&root.join("packages/app/app.ts"), "// app");
        write(
            &root.join("packages/lib/package.json"),
            r#"{"name":"@scope/lib"}"#,
        );
        write(&root.join("packages/lib/index.ts"), "// lib");
        write(
            &root.join("packages/unused/package.json"),
            r#"{"name":"@scope/unused"}"#,
        );
        write(&root.join("packages/unused/index.ts"), "// unused");
        root.canonicalize().unwrap()
    }

    #[test]
    fn detect_computes_transitive_closure_and_prunes_unused() {
        let root = fake_workspace("detect");
        let ws = detect_workspace(&root.join("packages/app")).expect("workspace detected");
        assert_eq!(ws.app_subdir, "packages/app");
        let mut rels: Vec<String> = ws
            .member_dirs
            .iter()
            .map(|d| {
                d.strip_prefix(&ws.root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        rels.sort();
        assert_eq!(
            rels,
            vec!["packages/app", "packages/lib"],
            "must include app + its workspace dep, not unused"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn tarball_packs_pruned_members_and_rewrites_workspaces() {
        use flate2::read::GzDecoder;
        let root = fake_workspace("tarball");
        let ws = detect_workspace(&root.join("packages/app")).unwrap();
        let tar_bytes = build_workspace_tarball(&ws).unwrap();
        let mut ar = tar::Archive::new(GzDecoder::new(&tar_bytes[..]));
        let mut paths = Vec::new();
        let mut root_pkg = String::new();
        for e in ar.entries().unwrap() {
            let mut e = e.unwrap();
            let p = e.path().unwrap().to_string_lossy().to_string();
            if p == "package.json" {
                use std::io::Read;
                e.read_to_string(&mut root_pkg).unwrap();
            }
            paths.push(p);
        }
        assert!(
            paths.iter().any(|p| p == "packages/app/app.ts"),
            "app source packed"
        );
        assert!(
            paths.iter().any(|p| p == "packages/lib/index.ts"),
            "workspace dep packed"
        );
        assert!(
            !paths.iter().any(|p| p.starts_with("packages/unused")),
            "unused member pruned: {paths:?}"
        );
        let v: serde_json::Value = serde_json::from_str(&root_pkg).unwrap();
        let ws_field: Vec<String> = v["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap().to_string())
            .collect();
        assert!(
            ws_field.contains(&"packages/app".to_string())
                && ws_field.contains(&"packages/lib".to_string())
        );
        assert!(
            !ws_field.contains(&"packages/unused".to_string()),
            "root workspaces narrowed to packed members"
        );
        let _ = fs::remove_dir_all(&root);
    }
}
