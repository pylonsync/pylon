//! Pylon Cloud client — credential storage + authenticated HTTP.
//!
//! Used by `pylon login`, `pylon logout`, and `pylon deploy --target
//! cloud`. Talks to https://cloud.pylonsync.com by default; the
//! cloud origin is overridable via `PYLON_CLOUD_URL` for staging /
//! self-hosted Pylon Cloud installations.
//!
//! Credentials live in `$XDG_CONFIG_HOME/pylon/credentials.json` (or
//! `~/.config/pylon/credentials.json` when XDG isn't set). The file
//! is chmod 0600 — never world-readable. Same trust model as
//! `~/.aws/credentials`, `~/.npmrc`, `~/.config/fly/config.yml`.

use std::fs;
use std::io::{self};
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Where the user's CLI token + the cloud they minted it against
/// are persisted between invocations. Token format follows Pylon's
/// API-key scheme (`pk.<id>.<secret>`) — see crates/auth/src/api_key.rs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    /// Origin of the Pylon Cloud install this token was minted on.
    /// Stored alongside the token so we don't accidentally send a
    /// staging token to prod (or vice versa) when PYLON_CLOUD_URL is
    /// flipped between invocations.
    pub cloud_url: String,
    /// The bearer token. Plaintext at rest, file mode 0600.
    pub token: String,
    /// Email of the authenticated user. Surfaced by `pylon login` so
    /// the operator can verify they signed in to the right account.
    /// Refreshed on every `login`.
    pub user_email: Option<String>,
}

/// Default cloud origin. Override with `PYLON_CLOUD_URL` for staging
/// or self-hosted installs.
pub const DEFAULT_CLOUD_URL: &str = "https://cloud.pylonsync.com";

pub fn cloud_url() -> String {
    std::env::var("PYLON_CLOUD_URL").unwrap_or_else(|_| DEFAULT_CLOUD_URL.to_string())
}

/// Path to the credentials file. Honors XDG_CONFIG_HOME; falls back
/// to `~/.config/pylon/credentials.json`.
pub fn credentials_path() -> io::Result<PathBuf> {
    let base = if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(xdg)
    } else {
        let home = std::env::var("HOME")
            .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "HOME not set"))?;
        PathBuf::from(home).join(".config")
    };
    Ok(base.join("pylon").join("credentials.json"))
}

/// Read stored credentials, or None if the file doesn't exist.
/// Bubbles other errors (permissions, malformed JSON) up.
pub fn load_credentials() -> io::Result<Option<Credentials>> {
    let path = credentials_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path)?;
    let creds: Credentials = serde_json::from_str(&raw).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("credentials.json is malformed ({e}). Delete it and re-run `pylon login`."),
        )
    })?;
    Ok(Some(creds))
}

/// Write credentials atomically (tmp + rename) with mode 0600.
/// Creates the parent directory if it doesn't exist.
pub fn save_credentials(creds: &Credentials) -> io::Result<()> {
    let path = credentials_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let json =
        serde_json::to_string_pretty(creds).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    fs::write(&tmp, json)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
    }
    fs::rename(&tmp, &path)?;
    Ok(())
}

/// Path to the persistent CLI state file (separate from credentials so
/// rotating the auth token doesn't blow away cached selections like
/// the default project). Honors XDG_CONFIG_HOME the same way.
pub fn state_path() -> io::Result<PathBuf> {
    let base = if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(xdg)
    } else {
        let home = std::env::var("HOME")
            .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "HOME not set"))?;
        PathBuf::from(home).join(".config")
    };
    Ok(base.join("pylon").join("state.json"))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CliState {
    /// Slug of the last project the user selected via
    /// `pylon projects use <slug>` (or interactive picker). Acts as
    /// the global fallback when no per-dir `.pylon/project` is found,
    /// so agents don't need to keep passing `--project` from every
    /// cwd. Per-dir context still wins when present — single project
    /// repos benefit from the global fallback, monorepos can pin a
    /// different project per subtree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_project: Option<String>,
}

pub fn load_state() -> io::Result<CliState> {
    let path = state_path()?;
    if !path.exists() {
        return Ok(CliState::default());
    }
    let raw = fs::read_to_string(&path)?;
    // Malformed state isn't fatal — fall back to empty and overwrite
    // on the next save. Worse to brick every CLI invocation than to
    // quietly forget the last-used project.
    Ok(serde_json::from_str(&raw).unwrap_or_default())
}

pub fn save_state(state: &CliState) -> io::Result<()> {
    let path = state_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(state)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    fs::write(&tmp, json)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
    }
    fs::rename(&tmp, &path)?;
    Ok(())
}

/// Update the global default project in state.json. Best-effort —
/// failures (read-only $HOME, disk full) don't fail the calling
/// command; the per-dir context still works and the worst case is
/// the next `pylon` invocation re-prompts for a project.
pub fn set_default_project(slug: &str) {
    if let Ok(mut state) = load_state() {
        state.default_project = Some(slug.to_string());
        let _ = save_state(&state);
    }
}

/// Delete stored credentials. Idempotent — returns Ok(false) if the
/// file didn't exist.
pub fn delete_credentials() -> io::Result<bool> {
    let path = credentials_path()?;
    if !path.exists() {
        return Ok(false);
    }
    fs::remove_file(&path)?;
    Ok(true)
}

/// Either the loaded credentials or a clear error pointing the user
/// at `pylon login`. Used by every command that needs auth.
pub fn require_credentials() -> Result<Credentials, String> {
    match load_credentials() {
        Ok(Some(c)) => Ok(c),
        Ok(None) => Err("Not logged in. Run `pylon login` first.".into()),
        Err(e) => Err(format!("Failed to read credentials: {e}")),
    }
}

// ---------------------------------------------------------------------------
// HTTP wrapper
// ---------------------------------------------------------------------------

/// Build a ureq agent with sane CLI defaults: 30s timeouts (deploys
/// can be slow), a User-Agent that includes the pylon version so
/// the cloud can log + correlate CLI traffic separately from
/// browser / GitHub-webhook traffic.
fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(300))
        .timeout_write(Duration::from_secs(300))
        .user_agent(&format!("pylon-cli/{}", env!("CARGO_PKG_VERSION")))
        .build()
}

/// One JSON POST against the cloud, bearer-authed with the loaded
/// token. Returns parsed JSON or a structured error.
pub fn post_json<I, O>(creds: &Credentials, path: &str, body: &I) -> Result<O, String>
where
    I: Serialize,
    O: for<'de> Deserialize<'de>,
{
    let url = format!("{}{}", creds.cloud_url.trim_end_matches('/'), path);
    // Coerce a `null` body to `{}`. Several no-arg endpoints
    // (`listMyProjectsForCli`, etc.) are invoked with the unit type
    // `&()`, which `serde_json` serializes as JSON `null`. Pylon's
    // function dispatcher then rejects with
    // `INVALID_ARGS: args must be an object` — every call from the
    // CLI dies with that error. Callers shouldn't have to remember to
    // pass `&json!({})` for the empty case; normalize here.
    let mut payload = serde_json::to_value(body).map_err(|e| e.to_string())?;
    if payload.is_null() {
        payload = serde_json::Value::Object(Default::default());
    }
    let res = agent()
        .post(&url)
        .set("Authorization", &format!("Bearer {}", creds.token))
        .set("Content-Type", "application/json")
        .send_json(payload);
    match res {
        Ok(resp) => resp
            .into_json::<O>()
            .map_err(|e| format!("Cloud returned 200 but the body wasn't the expected shape: {e}")),
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(format!("Cloud returned {code}: {body}"))
        }
        Err(e) => Err(format!("Cloud request failed: {e}")),
    }
}

/// POST a binary body (the tarball) with bearer auth. Returns the
/// parsed JSON response or a structured error.
#[allow(dead_code)] // Reserved for future raw-upload endpoints; deploy --target cloud uses post_json + base64 today.
pub fn post_bytes<O>(
    creds: &Credentials,
    path: &str,
    content_type: &str,
    bytes: &[u8],
) -> Result<O, String>
where
    O: for<'de> Deserialize<'de>,
{
    let url = format!("{}{}", creds.cloud_url.trim_end_matches('/'), path);
    let res = agent()
        .post(&url)
        .set("Authorization", &format!("Bearer {}", creds.token))
        .set("Content-Type", content_type)
        .send_bytes(bytes);
    match res {
        Ok(resp) => resp
            .into_json::<O>()
            .map_err(|e| format!("Cloud returned 200 but the body wasn't the expected shape: {e}")),
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(format!("Cloud returned {code}: {body}"))
        }
        Err(e) => Err(format!("Cloud request failed: {e}")),
    }
}

/// Validate a freshly-pasted token by calling `getMe` against the
/// cloud. Returns the user's email on success — this is the
/// post-login confirmation message that proves the token works AND
/// shows the operator which account they're in.
#[derive(Deserialize)]
struct GetMeResponse {
    email: String,
}

pub fn validate_token(cloud_url: &str, token: &str) -> Result<String, String> {
    let url = format!("{}/api/fn/getMe", cloud_url.trim_end_matches('/'));
    let res = agent()
        .post(&url)
        .set("Authorization", &format!("Bearer {token}"))
        .set("Content-Type", "application/json")
        .send_string("{}");
    match res {
        Ok(resp) => {
            let me: GetMeResponse = resp
                .into_json()
                .map_err(|e| format!("getMe returned a body we couldn't parse: {e}"))?;
            Ok(me.email)
        }
        Err(ureq::Error::Status(401 | 403, _)) => {
            Err("Token rejected by cloud. Generate a new one and re-paste.".into())
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(format!("Cloud returned {code}: {body}"))
        }
        Err(e) => Err(format!("Cloud request failed: {e}")),
    }
}
