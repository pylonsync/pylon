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
//! 5. Print the deployment id + URL, then WAIT for the build to reach a
//!    terminal status, printing the build log if it fails. `--no-wait`
//!    returns at "queued" instead.

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

/// Upload attempts before giving up. Smallware's ephemeral Fly builder can
/// fail to start transiently, and the control plane itself redeploys — both
/// resolve on their own, so the backoff (3s/8s/15s ≈ 26s) is sized to outlast
/// a restart rather than to be merely polite.
const MAX_UPLOAD_ATTEMPTS: u32 = 4;

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

    // 2. Resolve the project slug (flag → env → context → default). When
    //    nothing is linked, `pylon deploy` PROVISIONS one on the spot instead
    //    of dead-ending — so a first-time deploy is a single command.
    use crate::project_context::ProjectSource;
    let (project_slug, project_source) =
        match crate::project_context::resolve_project_with_source(args) {
            Some(resolved) => resolved,
            None => match ensure_deploy_project(&creds, json_mode) {
                // ensure_deploy_project writes the context file itself.
                Ok(slug) => (slug, ProjectSource::ContextFile),
                Err(e) => {
                    output::print_error(&e);
                    return ExitCode::Usage;
                }
            },
        };

    // Deploy overwrites what's live, so the machine-global default (which
    // just follows the last `pylon login` / `pylon projects use` run
    // ANYWHERE) is only trusted when it plausibly names this directory's
    // app. Anything else needs explicit intent — this is the guard that
    // stops `pylon deploy` in app A from clobbering project B.
    let deploy_cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if project_source == ProjectSource::GlobalDefault
        && !deploy_target_matches_dir(&project_slug, &deploy_cwd)
    {
        match confirm_global_default_deploy(&project_slug, &deploy_cwd, json_mode) {
            Ok(true) => {}
            Ok(false) => return ExitCode::Usage,
            Err(e) => {
                output::print_error(&e);
                return ExitCode::Usage;
            }
        }
    }

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

    // Pin the resolved project to this (now-known-deployable) directory so
    // the next deploy here can't drift with the machine-global selection.
    // Flag + (confirmed) global-default only: env-resolved runs are
    // typically CI, and a context file already pins. Ordered after the
    // app.ts check so a stray `pylon deploy --project x` from a non-app
    // directory (worst case: $HOME, whose pin would shadow every child
    // dir) never writes one.
    if matches!(
        project_source,
        ProjectSource::Flag | ProjectSource::GlobalDefault
    ) {
        if let Ok(path) = crate::project_context::write_context_file(&project_slug) {
            if !json_mode {
                println!(
                    "  Linked this directory to '{project_slug}' ({}) — future deploys here target it.",
                    path.display()
                );
            }
        }
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
    // The cloud's ephemeral Fly builder occasionally fails to start with a
    // transient BUILD_START_FAILED ("an unexpected error"); a fresh attempt
    // almost always succeeds. Auto-retry transient failures so a flaky builder
    // doesn't surface as a deploy failure. Permanent errors (4xx like
    // PAYLOAD_TOO_LARGE, a missing project) are not retried.
    let resp: UploadResponse = {
        let mut attempt: u32 = 0;
        loop {
            attempt += 1;
            match post_json(&creds, "/api/fn/deployProjectFromCliUpload", &body) {
                Ok(r) => break r,
                Err(e) => {
                    // Classification lives next to the formatter that produces
                    // these strings, so the two stay in step. The old inline
                    // version substring-matched "502"/"503"/"504" and missed
                    // Cloudflare's 520-527 family entirely — which is exactly
                    // what a control plane mid-redeploy returns.
                    let transient = crate::cloud_client::is_transient_cloud_error(&e);
                    if transient && attempt < MAX_UPLOAD_ATTEMPTS {
                        // Backoff has to outlast a control-plane redeploy. The
                        // old 1.5s/3s pair gave up ~4.5s in, long before the
                        // origin came back, so deploying while the cloud itself
                        // was deploying always failed.
                        let wait = std::time::Duration::from_secs(match attempt {
                            1 => 3,
                            2 => 8,
                            3 => 15,
                            _ => 30,
                        });
                        if !json_mode {
                            println!(
                                "  Smallware didn't accept the upload (attempt {attempt}/{MAX_UPLOAD_ATTEMPTS}) — retrying in {}s…",
                                wait.as_secs()
                            );
                        }
                        std::thread::sleep(wait);
                        continue;
                    }
                    output::print_error(&format!("Deploy failed: {e}"));
                    if e.contains("PAYLOAD_TOO_LARGE") || e.contains("413") {
                        eprintln!(
                            "  The upload ({:.1} MB source, ~{:.1} MB on the wire as base64 JSON) \
                             exceeds the server's request cap.",
                            tarball.len() as f64 / 1_048_576.0,
                            (tarball.len() as f64 * 4.0 / 3.0) / 1_048_576.0,
                        );
                        eprintln!(
                            "  On Pylon Cloud: deploy via the connected git repo instead \
                             (Settings → Git), or trim large static assets out of the upload."
                        );
                        eprintln!(
                            "  Self-hosting the control plane: raise PYLON_HTTP_BODY_MAX_BYTES."
                        );
                    }
                    return ExitCode::Error;
                }
            }
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
            "  Dashboard:  {}/dashboard → project → Deployments",
            crate::cloud_client::dashboard_url(),
        );
    }

    let wants_verify = args.iter().any(|a| a == "--verify");
    // Waiting is the DEFAULT. Returning at "queued" meant the command exited
    // successfully while the build could still fail a minute later, so the
    // only way to learn the outcome was to remember to run another command —
    // and an agent scripting this would report success for a broken deploy.
    // `--no-wait` restores fire-and-forget for CI that tracks it elsewhere;
    // --json keeps its single-shot contract so existing parsers don't hang.
    let wait = !json_mode && !args.iter().any(|a| a == "--no-wait");

    if wait || wants_verify {
        if !json_mode {
            println!();
            println!("→ Waiting for the build (Ctrl-C is safe — it keeps going)...");
        }
        let (project_id, outcome) =
            match wait_for_flip(&creds, &project_slug, &resp.deployment_id, json_mode) {
                Ok(v) => v,
                Err(e) => {
                    output::print_error(&e);
                    return ExitCode::Error;
                }
            };
        match outcome {
            FlipOutcome::Live => {
                if !json_mode {
                    println!("✓ Live");
                }
            }
            FlipOutcome::Ended { status } => {
                output::print_error(&format!("Deployment {status}."));
                // Print the log right here. Failing and then telling someone
                // to go run `pylon deployments logs` is the one moment a
                // detour is least welcome.
                print_build_log(&creds, &project_id, &resp.deployment_id);
                return ExitCode::Error;
            }
            FlipOutcome::TimedOut { status } => {
                output::print_error(&format!(
                    "Still {status} after 15m — the build is still running. \
                     Follow it with: pylon deployments logs"
                ));
                return ExitCode::Error;
            }
            FlipOutcome::Vanished => {
                output::print_error(
                    "Lost track of this deployment — it no longer appears in the \
                     project's recent deployments. The project may have been deleted, \
                     or the deployment was superseded. Check with: pylon deployments list",
                );
                return ExitCode::Error;
            }
        }
    }

    // `--verify`: having waited for THIS deployment to flip live (the old
    // build keeps serving during the bake, so verifying immediately would pass
    // against stale code), walk the live URL with the same checks as
    // `pylon verify --url`.
    if wants_verify {
        let Some(url) = resp.url.as_deref() else {
            output::print_error("--verify: the cloud response carried no URL to verify");
            return ExitCode::Error;
        };
        let route_paths: Vec<String> = match crate::manifest::load_manifest("pylon.manifest.json") {
            Ok(m) => m.routes.iter().map(|r| r.path.clone()).collect(),
            // No local manifest (rare — deploy just validated one, but
            // tolerate): verify the root route only.
            Err(_) => vec!["/".to_string()],
        };
        let report = crate::commands::verify::verify_target(url, &route_paths);
        crate::commands::verify::print_report(&report, json_mode);
        if report.failed() {
            return ExitCode::Error;
        }
    }
    ExitCode::Ok
}

/// Fetch and print a finished deployment's build log.
///
/// There is no incremental log endpoint — `getDeploymentBuildLog` attaches the
/// whole thing at the terminal transition — so this is a one-shot print after
/// the wait, not a live tail. Called automatically when a deploy fails, since
/// "it failed, go run another command" is the moment you least want a detour.
fn print_build_log(
    creds: &crate::cloud_client::Credentials,
    project_id: &str,
    deployment_id: &str,
) {
    #[derive(Serialize)]
    struct Args<'a> {
        #[serde(rename = "projectId")]
        project_id: &'a str,
        #[serde(rename = "deploymentId")]
        deployment_id: &'a str,
    }
    #[derive(Deserialize)]
    struct LogOut {
        error: Option<String>,
        #[serde(rename = "buildLog")]
        build_log: Option<String>,
    }
    let out: LogOut = match post_json(
        creds,
        "/api/fn/getDeploymentBuildLog",
        &Args {
            project_id,
            deployment_id,
        },
    ) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("  (could not fetch build log: {e})");
            return;
        }
    };
    if let Some(err) = out.error.as_deref().filter(|e| !e.is_empty()) {
        println!("  Error: {err}");
    }
    match out.build_log.as_deref().filter(|l| !l.is_empty()) {
        Some(log) => {
            println!("{}", "─".repeat(60));
            println!("{log}");
            println!("{}", "─".repeat(60));
        }
        None => println!("  (no build log captured)"),
    }
}

/// How a watched deployment ended. Distinct from `Err`, which means we could
/// not talk to the control plane at all.
enum FlipOutcome {
    Live,
    /// Reached a terminal non-live status (failed / error / canceled).
    Ended {
        status: String,
    },
    /// Still building when the ceiling elapsed — not a failure, just unwatched
    /// from here on.
    TimedOut {
        status: String,
    },
    /// The deployment stopped appearing in the project's recent list. The
    /// build is not "still running" — we lost track of it, which is a
    /// different thing and deserves a different message.
    Vanished,
}

/// Poll the deployments list until `deployment_id` reaches a terminal
/// status. "live" returns Ok; failed/error/canceled return Err. The
/// build path (workspace tarballs especially) legitimately takes many
/// minutes, so the ceiling is generous; progress prints every poll so
/// an agent tailing this knows it isn't hung.
///
/// Returns the resolved project id alongside the outcome so the caller can
/// fetch the build log without resolving the slug a second time. `Err` is
/// reserved for transport failures — a deployment that legitimately ends
/// "failed" is an `Ok(Ended)`, because the caller still wants to print its
/// build log rather than treat it as an unreachable server.
fn wait_for_flip(
    creds: &crate::cloud_client::Credentials,
    project_slug: &str,
    deployment_id: &str,
    json_mode: bool,
) -> Result<(String, FlipOutcome), String> {
    #[derive(Serialize)]
    struct SlugArgs<'a> {
        slug: &'a str,
    }
    #[derive(Deserialize)]
    struct ProjectIdResponse {
        id: String,
    }
    #[derive(Serialize)]
    struct ListArgs<'a> {
        #[serde(rename = "projectId")]
        project_id: &'a str,
        limit: u32,
    }
    #[derive(Deserialize)]
    struct DeploymentRow {
        id: String,
        status: String,
    }

    let project: ProjectIdResponse = post_json(
        creds,
        "/api/fn/getProjectForCli",
        &SlugArgs { slug: project_slug },
    )
    .map_err(|e| format!("could not resolve project id: {e}"))?;

    // Six consecutive misses at a 10s poll = one minute of the deployment not
    // appearing in the project's recent list. Enough to ride out a row that is
    // slow to show up, short enough that a deleted project fails fast.
    const MISSING_POLLS_BEFORE_GIVING_UP: u32 = 6;

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15 * 60);
    let mut last_status = String::new();
    let mut missing_polls: u32 = 0;
    loop {
        let rows: Vec<DeploymentRow> = post_json(
            creds,
            "/api/fn/listDeployments",
            &ListArgs {
                project_id: &project.id,
                limit: 20,
            },
        )
        .map_err(|e| format!("could not poll deployments: {e}"))?;
        // A missing row is NOT a status. It means the deployment is not in the
        // project's recent list at all — deleted project, or it aged out —
        // and reporting it as "unknown" made the timeout claim the build was
        // "still running" when there was nothing left to run. Track it as its
        // own condition and give up early rather than waiting out the full
        // ceiling for a row that will never appear.
        let found = rows.iter().find(|d| d.id == deployment_id);
        let status = match found {
            Some(row) => {
                missing_polls = 0;
                row.status.clone()
            }
            None => {
                missing_polls += 1;
                if missing_polls >= MISSING_POLLS_BEFORE_GIVING_UP {
                    return Ok((project.id, FlipOutcome::Vanished));
                }
                last_status.clone()
            }
        };
        if status != last_status {
            if !json_mode {
                println!("  build status: {status}");
            }
            last_status = status.clone();
        }
        match status.as_str() {
            "live" => return Ok((project.id, FlipOutcome::Live)),
            "failed" | "error" | "canceled" => {
                return Ok((project.id, FlipOutcome::Ended { status }));
            }
            _ => {}
        }
        if std::time::Instant::now() > deadline {
            return Ok((project.id, FlipOutcome::TimedOut { status }));
        }
        std::thread::sleep(std::time::Duration::from_secs(10));
    }
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
            // pylon.manifest.json ships EVEN when gitignored (it near-
            // universally is — it's generated output). The cloud build
            // reads manifest-derived features (fonts, SSR route metadata)
            // from it; without it a bundle builds fine and silently loses
            // them. The builder can also re-derive it, but the local copy
            // matches what the developer just ran, so prefer shipping it.
            if name == "pylon.manifest.json" {
                tar.append_path_with_name(&path, path.strip_prefix(root).unwrap_or(&path))?;
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

/// No project is linked in this directory. On a TTY, offer to create + link
/// one so `pylon deploy` is a true one-command first deploy; off a TTY, point
/// the user at the explicit path. Reuses createProject + the same context
/// files `pylon projects use` writes, so subsequent deploys target it.
fn ensure_deploy_project(
    creds: &crate::cloud_client::Credentials,
    json_mode: bool,
) -> Result<String, String> {
    use std::io::{BufRead, IsTerminal, Write};

    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let dirname = cwd.file_name().and_then(|n| n.to_str()).unwrap_or("app");
    let default_slug = sanitize_slug(dirname);

    // Non-interactive (CI / --json / piped): don't guess a slug — tell them how.
    if json_mode || !std::io::stdin().is_terminal() {
        return Err(format!(
            "No project is linked here. Create one first:\n  pylon projects create {default_slug}\nthen re-run `pylon deploy` (or pass --project <slug>)."
        ));
    }

    #[derive(serde::Deserialize)]
    struct ProjRow {
        slug: String,
        name: String,
        #[serde(rename = "orgSlug")]
        org_slug: Option<String>,
    }
    let projects: Vec<ProjRow> =
        crate::cloud_client::post_json(creds, "/api/fn/listMyProjectsForCli", &())
            .map_err(|e| format!("Couldn't list your projects: {e}"))?;

    println!();
    if projects.is_empty() {
        print!("No project is linked here. Create \"{default_slug}\" and deploy? [Y/n] ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        std::io::stdin()
            .lock()
            .read_line(&mut line)
            .map_err(|e| e.to_string())?;
        let lc = line.trim().to_ascii_lowercase();
        if lc == "n" || lc == "no" {
            return Err("Aborted — nothing deployed.".into());
        }
    } else {
        println!("No project is linked in this directory.");
        println!("  [Enter]  create a new project \"{default_slug}\"");
        for (i, p) in projects.iter().enumerate() {
            let org = p.org_slug.as_deref().unwrap_or("?");
            println!(
                "  {:>2}.     use existing {}/{} ({})",
                i + 1,
                org,
                p.slug,
                p.name
            );
        }
        print!("Choice: ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        std::io::stdin()
            .lock()
            .read_line(&mut line)
            .map_err(|e| e.to_string())?;
        let choice = line.trim();
        if !choice.is_empty() {
            match choice.parse::<usize>() {
                Ok(n) if n >= 1 && n <= projects.len() => {
                    let slug = projects[n - 1].slug.clone();
                    crate::cloud_client::set_default_project(&slug);
                    let _ = crate::project_context::write_context_file(&slug);
                    return Ok(slug);
                }
                _ => return Err("Out of range.".into()),
            }
        }
        // empty (Enter) falls through to create.
    }

    create_project_for_deploy(creds, &default_slug)
}

/// Create a project for the current directory via createProject, then link it
/// (default project + `.pylon/project`) so this and future deploys target it.
fn create_project_for_deploy(
    creds: &crate::cloud_client::Credentials,
    slug: &str,
) -> Result<String, String> {
    let bytes = slug.as_bytes();
    let valid_slug = bytes.len() >= 2
        && bytes.len() <= 40
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes[1..]
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-');
    if !valid_slug {
        return Err(
            "Couldn't derive a valid project name from this directory — run \
             `pylon projects create <slug>` with an explicit slug (lowercase \
             letters, digits, hyphens), then `pylon deploy`."
                .into(),
        );
    }

    #[derive(serde::Deserialize)]
    struct OrgRow {
        id: String,
        slug: String,
    }
    let orgs: Vec<OrgRow> = crate::cloud_client::post_json(creds, "/api/fn/listMyOrgsForCli", &())
        .map_err(|e| format!("Couldn't list your orgs: {e}"))?;
    if orgs.is_empty() {
        return Err(format!(
            "Your account has no organizations — finish signup at {}/dashboard.",
            crate::cloud_client::dashboard_url()
        ));
    }
    let org = if orgs.len() == 1 {
        &orgs[0]
    } else {
        return Err(
            "You belong to multiple orgs — run `pylon projects create <slug> --org <org>`, \
             then `pylon deploy`."
                .into(),
        );
    };

    #[derive(serde::Serialize)]
    struct CreateArgs<'a> {
        #[serde(rename = "orgId")]
        org_id: &'a str,
        name: &'a str,
        slug: &'a str,
    }
    #[derive(serde::Deserialize)]
    struct Created {
        slug: String,
    }
    let created: Created = crate::cloud_client::post_json(
        creds,
        "/api/fn/createProject",
        &CreateArgs {
            org_id: &org.id,
            name: slug,
            slug,
        },
    )
    .map_err(|e| format!("Create failed: {e}"))?;
    crate::cloud_client::set_default_project(&created.slug);
    let _ = crate::project_context::write_context_file(&created.slug);
    println!(
        "✓ Created project {} in org {} — deploying...",
        created.slug, org.slug
    );
    Ok(created.slug)
}

/// Turn a directory name into a valid project slug (lowercase alnum + hyphens,
/// 2–40 chars, starts alnum). Best-effort; the user can always pass an explicit
/// slug via `pylon projects create`.
fn sanitize_slug(name: &str) -> String {
    let lowered: String = name
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let mut collapsed = lowered;
    while collapsed.contains("--") {
        collapsed = collapsed.replace("--", "-");
    }
    let trimmed = collapsed.trim_matches('-');
    let based = if trimmed
        .chars()
        .next()
        .map(|c| c.is_ascii_alphanumeric())
        .unwrap_or(false)
    {
        trimmed.to_string()
    } else {
        format!("app-{trimmed}")
    };
    let capped: String = based.chars().take(40).collect();
    let capped = capped.trim_matches('-').to_string();
    if capped.len() < 2 {
        "app".to_string()
    } else {
        capped
    }
}

/// Does the machine-global project selection plausibly name this
/// directory's app? Compares the sanitized directory name and the
/// package.json "name" (scope stripped) against the slug. Used only to
/// let the global-default fallback pass silently — any mismatch requires
/// explicit confirmation before deploy will overwrite the target.
fn deploy_target_matches_dir(slug: &str, cwd: &Path) -> bool {
    let slug_norm = sanitize_slug(slug);
    if let Some(dir) = cwd.file_name().and_then(|n| n.to_str()) {
        if sanitize_slug(dir) == slug_norm {
            return true;
        }
    }
    if let Ok(raw) = std::fs::read_to_string(cwd.join("package.json")) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            if let Some(name) = v.get("name").and_then(|n| n.as_str()) {
                let base = name.rsplit('/').next().unwrap_or(name);
                if sanitize_slug(base) == slug_norm {
                    return true;
                }
            }
        }
    }
    false
}

/// The resolved project came from the machine-global default and doesn't
/// look like this directory's app. On a TTY, spell out the mismatch and
/// ask (default No); off a TTY, hard-error — CI must pass --project.
/// Returns Ok(true) to proceed, Ok(false) after an explicit decline.
fn confirm_global_default_deploy(slug: &str, cwd: &Path, json_mode: bool) -> Result<bool, String> {
    use std::io::{BufRead, IsTerminal, Write};
    let dirname = cwd.file_name().and_then(|n| n.to_str()).unwrap_or("?");
    if json_mode || !std::io::stdin().is_terminal() {
        return Err(format!(
            "This directory ('{dirname}') isn't linked to a cloud project, and the \
             machine-wide default is '{slug}' — which doesn't look like this app. \
             Refusing to guess: deploying would overwrite what's live on '{slug}'.\n  \
             Pass --project <slug> (links this directory), or run `pylon projects use <slug>` here."
        ));
    }
    println!();
    println!("This directory ('{dirname}') isn't linked to a cloud project.");
    println!("The machine-wide default is '{slug}' (whatever the last `pylon login` /");
    println!("`pylon projects use` selected) — deploying would overwrite what's live there.");
    print!("Deploy '{dirname}' to project '{slug}' anyway? [y/N] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    if line.trim().eq_ignore_ascii_case("y") {
        return Ok(true);
    }
    println!("Aborted. Deploy with --project <slug> to pick the target explicitly.");
    Ok(false)
}

#[cfg(test)]
mod global_default_guard_tests {
    use super::*;
    use std::fs;

    fn dir_with_pkg(tag: &str, dirname: &str, pkg_name: Option<&str>) -> PathBuf {
        let root = std::env::temp_dir()
            .join(format!("pylon-guard-{}-{tag}", std::process::id()))
            .join(dirname);
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        if let Some(name) = pkg_name {
            fs::write(root.join("package.json"), format!(r#"{{"name":"{name}"}}"#)).unwrap();
        }
        root
    }

    #[test]
    fn matching_dirname_passes() {
        let cwd = dir_with_pkg("dirname", "revtrail", None);
        assert!(deploy_target_matches_dir("revtrail", &cwd));
    }

    #[test]
    fn sanitization_bridges_naming_variants() {
        // dir "My Cool App" vs slug "my-cool-app" — both sanitize the same.
        let cwd = dir_with_pkg("sanitize", "My Cool App", None);
        assert!(deploy_target_matches_dir("my-cool-app", &cwd));
    }

    #[test]
    fn package_name_matches_when_dirname_does_not() {
        // monorepo-style: dir "web", package "@acme/reelbear".
        let cwd = dir_with_pkg("pkg", "web", Some("@acme/reelbear"));
        assert!(deploy_target_matches_dir("reelbear", &cwd));
    }

    #[test]
    fn unrelated_project_is_rejected() {
        // The incident: deploying from revtrail/ while the machine-global
        // default points at reelbear must NOT pass silently.
        let cwd = dir_with_pkg("mismatch", "revtrail", Some("revtrail"));
        assert!(!deploy_target_matches_dir("reelbear", &cwd));
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tar_entry_names(tarball: &[u8]) -> Vec<String> {
        let gz = flate2::read::GzDecoder::new(tarball);
        let mut archive = tar::Archive::new(gz);
        archive
            .entries()
            .unwrap()
            .map(|e| e.unwrap().path().unwrap().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn tarball_ships_manifest_even_when_gitignored() {
        let dir = std::env::temp_dir().join(format!("pylon-tar-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("app.ts"), "export {};").unwrap();
        std::fs::write(dir.join("pylon.manifest.json"), "{}").unwrap();
        std::fs::write(dir.join("notes.txt"), "x").unwrap();
        // Both gitignored — but the manifest is required build input.
        std::fs::write(dir.join(".gitignore"), "pylon.manifest.json\nnotes.txt\n").unwrap();

        let tarball = build_tarball(&dir).unwrap();
        let names = tar_entry_names(&tarball);
        assert!(
            names.iter().any(|n| n == "pylon.manifest.json"),
            "gitignored manifest must still ship: {names:?}"
        );
        assert!(
            !names.iter().any(|n| n == "notes.txt"),
            "other gitignored files must stay excluded: {names:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
