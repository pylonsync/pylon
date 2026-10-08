//! Dev-only git endpoints for a hosted sandbox.
//!
//! - `POST /_pylon/dev/git/commit` stages every change in the workspace,
//!   commits it, and pushes `HEAD` to a branch of a remote.
//! - `POST /_pylon/dev/git/sync` fetches a branch of a remote and resets the
//!   workspace to one commit on it.
//!
//! The caller sends the remote URL, with its credential, in each request. The
//! sandbox does not store it: no remote is added and no credential helper
//! runs, so a token is only on the machine while one git command runs. Errors
//! that echo the URL are redacted before they go back.
//!
//! Access is the same as `/_pylon/dev/files/*` ([`super::dev_workspace_access`]).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use serde::Deserialize;
use tiny_http::{Header, Method, Request, Response};

/// One git operation at a time. Two concurrent commits in one work tree race
/// on the index lock.
static GIT_LOCK: Mutex<()> = Mutex::new(());

const MAX_BODY: u64 = 64 * 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommitBody {
    remote_url: String,
    branch: String,
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SyncBody {
    remote_url: String,
    branch: String,
    sha: String,
}

/// Route `/_pylon/dev/git/*`. The caller has checked dev mode.
pub(super) fn serve(mut request: Request, path_only: &str, cors_origin: &str) {
    use std::io::Read as _;

    let cors_owned = super::dev_api_cors_origin(&request, cors_origin);
    let cors_origin = cors_owned.as_str();

    if matches!(request.method(), Method::Options) {
        let mut resp = Response::empty(204u16);
        for (k, v) in [
            ("Access-Control-Allow-Origin", cors_origin),
            ("Access-Control-Allow-Methods", "POST, OPTIONS"),
            (
                "Access-Control-Allow-Headers",
                "Authorization, Content-Type",
            ),
        ] {
            if let Ok(h) = Header::from_bytes(k, v) {
                resp = resp.with_header(h);
            }
        }
        let _ = request.respond(resp);
        return;
    }

    if let Err(status) = super::dev_workspace_access(&request) {
        let error = if status == 401 {
            "unauthorized"
        } else {
            "forbidden"
        };
        let _ = request.respond(json(
            status,
            serde_json::json!({ "ok": false, "error": error }),
            cors_origin,
        ));
        return;
    }

    if !matches!(request.method(), Method::Post) {
        let body = serde_json::json!({ "ok": false, "error": "method not allowed" });
        let _ = request.respond(json(405, body, cors_origin));
        return;
    }

    let mut body = Vec::new();
    let read = request
        .as_reader()
        .take(MAX_BODY + 1)
        .read_to_end(&mut body);
    if read.is_err() || body.len() as u64 > MAX_BODY {
        let body = serde_json::json!({ "ok": false, "error": "body too large" });
        let _ = request.respond(json(413, body, cors_origin));
        return;
    }

    let root = workspace_root();
    let outcome = match path_only {
        "/_pylon/dev/git/commit" => serde_json::from_slice::<CommitBody>(&body)
            .map_err(|e| GitError::bad_request(format!("bad body: {e}")))
            .and_then(|b| commit(&root, &b)),
        "/_pylon/dev/git/sync" => serde_json::from_slice::<SyncBody>(&body)
            .map_err(|e| GitError::bad_request(format!("bad body: {e}")))
            .and_then(|b| sync(&root, &b)),
        _ => Err(GitError {
            status: 404,
            code: "not_found",
            message: "no such git endpoint".into(),
        }),
    };

    let resp = match outcome {
        Ok(value) => json(200, value, cors_origin),
        Err(e) => json(
            e.status,
            serde_json::json!({ "ok": false, "error": e.code, "message": e.message }),
            cors_origin,
        ),
    };
    let _ = request.respond(resp);
}

fn workspace_root() -> PathBuf {
    std::env::var("PYLON_DEV_WATCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

#[derive(Debug)]
struct GitError {
    status: u16,
    code: &'static str,
    message: String,
}

impl GitError {
    fn bad_request(message: String) -> Self {
        Self {
            status: 400,
            code: "bad_request",
            message,
        }
    }
}

/// Stage everything, commit when something changed, push `HEAD`.
///
/// Pushes even when nothing changed, so a commit whose earlier push failed
/// still reaches the remote. A rejected push (the remote branch moved) is
/// `409 push_rejected`; the caller syncs and retries.
fn commit(root: &Path, body: &CommitBody) -> Result<serde_json::Value, GitError> {
    check_remote(&body.remote_url)?;
    commit_to(root, body)
}

/// [`commit`] after the remote URL check, which tests skip to use a local
/// `file://` remote.
fn commit_to(root: &Path, body: &CommitBody) -> Result<serde_json::Value, GitError> {
    check_branch(&body.branch)?;
    let message = body.message.trim();
    if message.is_empty() {
        return Err(GitError::bad_request("message is empty".into()));
    }

    let _guard = GIT_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let redact = |s: &str| redact(s, &body.remote_url);

    run(root, &["add", "-A"], &redact)?;
    // `diff --cached --quiet` exits 1 when the index differs from HEAD.
    let staged = !git(root, &["diff", "--cached", "--quiet"])
        .status()
        .map_err(spawn_error)?
        .success();
    if staged {
        run(root, &["commit", "-q", "-m", message], &redact)?;
    }

    let refspec = format!("HEAD:refs/heads/{}", body.branch);
    let push = git(
        root,
        &["push", "-q", "--porcelain", &body.remote_url, &refspec],
    )
    .output()
    .map_err(spawn_error)?;
    if !push.status.success() {
        let text = redact(&format!(
            "{}{}",
            String::from_utf8_lossy(&push.stdout),
            String::from_utf8_lossy(&push.stderr)
        ));
        let rejected = text.contains("[rejected]")
            || text.contains("non-fast-forward")
            || text.contains("fetch first");
        return Err(GitError {
            status: if rejected { 409 } else { 502 },
            code: if rejected {
                "push_rejected"
            } else {
                "push_failed"
            },
            message: text.trim().to_string(),
        });
    }

    let sha = run(root, &["rev-parse", "HEAD"], &redact)?;
    Ok(serde_json::json!({ "ok": true, "sha": sha, "committed": staged }))
}

/// Fetch `branch` and reset the work tree to `sha` on it. Tracked files that
/// `sha` does not have are removed. Untracked files stay: `git clean` would
/// delete `node_modules` when `sha` predates the `.gitignore` that covers it.
/// The caller commits before it syncs, so nothing of the agent's is untracked.
fn sync(root: &Path, body: &SyncBody) -> Result<serde_json::Value, GitError> {
    check_remote(&body.remote_url)?;
    sync_to(root, body)
}

/// [`sync`] after the remote URL check.
fn sync_to(root: &Path, body: &SyncBody) -> Result<serde_json::Value, GitError> {
    check_branch(&body.branch)?;
    check_sha(&body.sha)?;

    let _guard = GIT_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let redact = |s: &str| redact(s, &body.remote_url);

    let refspec = format!("refs/heads/{}", body.branch);
    run(root, &["fetch", "-q", &body.remote_url, &refspec], &redact)?;
    // The commit must be on the fetched branch, so a caller cannot reset the
    // workspace to an object the branch does not contain.
    let on_branch = git(
        root,
        &["merge-base", "--is-ancestor", &body.sha, "FETCH_HEAD"],
    )
    .status()
    .map_err(spawn_error)?
    .success();
    if !on_branch {
        return Err(GitError {
            status: 409,
            code: "not_on_branch",
            message: format!("{} is not on {}", body.sha, body.branch),
        });
    }
    run(root, &["reset", "-q", "--hard", &body.sha], &redact)?;

    let sha = run(root, &["rev-parse", "HEAD"], &redact)?;
    Ok(serde_json::json!({ "ok": true, "sha": sha }))
}

/// A git command in `root` that never prompts and never stores a credential.
fn git(root: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(root)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .args([
            "-c",
            "credential.helper=",
            "-c",
            "http.lowSpeedLimit=1000",
            "-c",
            "http.lowSpeedTime=30",
        ])
        .args(args);
    cmd
}

/// Run a git command; its trimmed stdout on success.
fn run(root: &Path, args: &[&str], redact: &dyn Fn(&str) -> String) -> Result<String, GitError> {
    let out = git(root, args).output().map_err(spawn_error)?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).trim().to_string());
    }
    Err(GitError {
        status: 500,
        code: "git_failed",
        message: redact(&format!(
            "git {}: {}",
            args[0],
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    })
}

fn spawn_error(e: std::io::Error) -> GitError {
    GitError {
        status: 500,
        code: "git_unavailable",
        message: format!("could not run git: {e}"),
    }
}

/// Remove the remote URL, and any `user:token@` userinfo, from git output.
fn redact(text: &str, remote_url: &str) -> String {
    let mut out = text.replace(remote_url, "<remote>");
    while let Some(scheme) = out.find("https://") {
        let rest_start = scheme + "https://".len();
        let rest = &out[rest_start..];
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '\'' || c == '"')
            .unwrap_or(rest.len());
        match rest[..end].find('@') {
            Some(at) => out.replace_range(rest_start..rest_start + at + 1, ""),
            None => break,
        }
    }
    out
}

fn check_remote(url: &str) -> Result<(), GitError> {
    if !url.starts_with("https://") || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(GitError::bad_request(
            "remoteUrl must be an https URL".into(),
        ));
    }
    Ok(())
}

/// A plain branch name: no leading `-`, no `..`, no ref syntax.
fn check_branch(branch: &str) -> Result<(), GitError> {
    let ok = !branch.is_empty()
        && branch.len() <= 200
        && !branch.starts_with('-')
        && !branch.starts_with('/')
        && !branch.ends_with('/')
        && !branch.ends_with(".lock")
        && !branch.contains("..")
        && !branch.contains("//")
        && branch
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'));
    if ok {
        Ok(())
    } else {
        Err(GitError::bad_request(format!("bad branch name: {branch}")))
    }
}

fn check_sha(sha: &str) -> Result<(), GitError> {
    if (7..=64).contains(&sha.len()) && sha.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(GitError::bad_request("sha must be a hex commit id".into()))
    }
}

fn json(
    status: u16,
    body: serde_json::Value,
    cors_origin: &str,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let mut resp = Response::from_string(body.to_string()).with_status_code(status);
    for (k, v) in [
        ("Content-Type", "application/json"),
        ("Cache-Control", "no-store"),
        ("Access-Control-Allow-Origin", cors_origin),
    ] {
        if let Ok(h) = Header::from_bytes(k, v) {
            resp = resp.with_header(h);
        }
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .current_dir(dir)
            .args(args)
            .output()
            .expect("git");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A bare "remote" and a clone of it with one commit, as the sandbox has.
    fn fixture() -> (tempfile::TempDir, PathBuf, String) {
        let tmp = tempfile::tempdir().unwrap();
        let remote = tmp.path().join("remote.git");
        let work = tmp.path().join("work");
        std::fs::create_dir_all(&remote).unwrap();
        sh(&remote, &["init", "-q", "--bare", "-b", "main"]);
        std::fs::create_dir_all(&work).unwrap();
        sh(&work, &["init", "-q", "-b", "main"]);
        sh(&work, &["config", "user.email", "agent@pylonsync.com"]);
        sh(&work, &["config", "user.name", "Pylon Build"]);
        std::fs::write(work.join("app.ts"), "export default 1;\n").unwrap();
        sh(&work, &["add", "-A"]);
        sh(&work, &["commit", "-q", "-m", "Starter"]);
        let url = format!("file://{}", remote.display());
        sh(&work, &["push", "-q", &url, "HEAD:refs/heads/main"]);
        (tmp, work, url)
    }

    fn commit_main(root: &Path, url: &str, message: &str) -> Result<serde_json::Value, GitError> {
        commit_to(
            root,
            &CommitBody {
                remote_url: url.into(),
                branch: "main".into(),
                message: message.into(),
            },
        )
    }

    #[test]
    fn commit_pushes_a_change_and_reports_no_change_after() {
        let (_tmp, work, url) = fixture();
        std::fs::write(work.join("page.tsx"), "export default () => null;\n").unwrap();
        let first = commit_main(&work, &url, "Add a page").unwrap();
        assert_eq!(first["committed"], true);
        let remote_head = sh(&work, &["ls-remote", &url, "refs/heads/main"]);
        assert!(remote_head.starts_with(first["sha"].as_str().unwrap()));

        let second = commit_main(&work, &url, "Nothing").unwrap();
        assert_eq!(second["committed"], false);
        assert_eq!(second["sha"], first["sha"]);
    }

    #[test]
    fn sync_resets_to_an_earlier_commit_and_keeps_untracked_files() {
        let (_tmp, work, url) = fixture();
        let starter = sh(&work, &["rev-parse", "HEAD"]);
        std::fs::write(work.join(".gitignore"), "node_modules\n").unwrap();
        std::fs::write(work.join("page.tsx"), "1").unwrap();
        commit_main(&work, &url, "Turn 1").unwrap();
        std::fs::create_dir_all(work.join("node_modules")).unwrap();
        std::fs::write(work.join("node_modules/x.js"), "1").unwrap();
        std::fs::write(work.join("scratch.txt"), "untracked").unwrap();

        let body = SyncBody {
            remote_url: url.clone(),
            branch: "main".into(),
            sha: starter.clone(),
        };
        let synced = sync_to(&work, &body).unwrap();
        assert_eq!(synced["sha"], starter.as_str());

        assert!(!work.join("page.tsx").exists());
        assert!(work.join("scratch.txt").exists());
        assert!(work.join("node_modules/x.js").exists());
    }

    #[test]
    fn sync_refuses_a_commit_that_is_not_on_the_branch() {
        let (_tmp, work, url) = fixture();
        std::fs::write(work.join("local.txt"), "1").unwrap();
        sh(&work, &["add", "-A"]);
        sh(&work, &["commit", "-q", "-m", "Local only"]);
        let local = sh(&work, &["rev-parse", "HEAD"]);
        let err = sync_to(
            &work,
            &SyncBody {
                remote_url: url,
                branch: "main".into(),
                sha: local,
            },
        )
        .unwrap_err();
        assert_eq!(err.code, "not_on_branch");
        assert!(work.join("local.txt").exists());
    }

    #[test]
    fn redact_removes_the_url_and_userinfo() {
        let url = "https://t:secret.jwt@acme.code.storage/o/p.git";
        assert_eq!(
            redact(&format!("fatal: unable to access '{url}'"), url),
            "fatal: unable to access '<remote>'"
        );
        assert_eq!(
            redact(
                "fatal: https://t:other@acme.code.storage/x.git not found",
                url
            ),
            "fatal: https://acme.code.storage/x.git not found"
        );
    }

    #[test]
    fn rejects_unsafe_arguments() {
        assert!(check_remote("file:///etc").is_err());
        assert!(check_remote("--upload-pack=sh").is_err());
        assert!(check_remote("https://a b").is_err());
        assert!(check_remote("https://t:x@acme.code.storage/o/p.git").is_ok());
        assert!(check_branch("main").is_ok());
        assert!(check_branch("turns/abc-1").is_ok());
        for bad in ["", "-x", "a..b", "a b", "/a", "a/", "a.lock", "a~1", "a:b"] {
            assert!(check_branch(bad).is_err(), "{bad}");
        }
        assert!(check_sha("abc1234").is_ok());
        assert!(check_sha("HEAD").is_err());
        assert!(check_sha("--all").is_err());
    }
}
