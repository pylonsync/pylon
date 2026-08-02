//! Pylon Cloud client — credential storage + authenticated HTTP.
//!
//! Used by `pylon login`, `pylon logout`, and `pylon deploy --target
//! cloud`. Talks to https://www.usesmallware.com by default — hosted Pylon
//! Cloud is Smallware, one unified app serving both the API and the
//! dashboard. The cloud origin is overridable via `PYLON_CLOUD_URL` for
//! staging / self-hosted installations.
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
///
/// Hosted Pylon Cloud is Smallware. `www.pylonsync.com` now serves the Pylon
/// framework's marketing site and no longer answers `/api/*`, so a CLI pointed
/// there gets HTML where it expects JSON.
pub const DEFAULT_CLOUD_URL: &str = "https://www.usesmallware.com";

pub fn cloud_url() -> String {
    std::env::var("PYLON_CLOUD_URL").unwrap_or_else(|_| DEFAULT_CLOUD_URL.to_string())
}

/// Hosts that used to serve hosted Pylon Cloud and no longer do.
///
/// `api.pylonsync.com` was retired when the API and dashboard merged onto one
/// origin. It is not merely a redirect — it has no certificate, so anything
/// still pointed at it fails the TLS handshake and surfaces as a bare
/// Cloudflare 525 with no hint that the host moved.
///
/// `(www.)pylonsync.com` were retired when hosted Pylon Cloud became Smallware:
/// those hosts now serve the framework's marketing site, so a stored credential
/// pointing at them reaches a real, healthy server that simply has no `/api/*`
/// — the failure is an HTML body parsed as JSON rather than a network error,
/// which is even less legible than the 525. Both the bare and www forms are
/// listed because credentials were minted against both.
const RETIRED_CLOUD_HOSTS: &[&str] = &[
    "https://api.pylonsync.com",
    "https://pylonsync.com",
    "https://www.pylonsync.com",
];

/// Point a stored origin at wherever hosted Pylon Cloud actually lives now.
///
/// Credentials persist the origin they were minted against, and every request
/// is built from that stored value — so a login predating the consolidation
/// keeps addressing a host that stopped answering, and the operator sees an
/// SSL error instead of anything actionable. Rewriting on load fixes the
/// whole surface at once (deploy, logs, project list) rather than one call
/// site at a time.
///
/// Only EXACT retired origins are rewritten. A self-hosted or staging install
/// is returned untouched — silently redirecting somebody's own cloud to ours
/// would be far worse than the error this replaces.
pub fn normalize_cloud_url(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    if RETIRED_CLOUD_HOSTS.contains(&trimmed) {
        return DEFAULT_CLOUD_URL.to_string();
    }
    trimmed.to_string()
}

/// Where the human-facing dashboard lives. Hosted Pylon Cloud is one unified
/// app on www.usesmallware.com serving both the API and the dashboard, so this
/// is normally the same origin the CLI talks to. Retired hosts are rewritten and
/// self-hosted / staging origins (PYLON_CLOUD_URL) pass through untouched —
/// both via [`normalize_cloud_url`], which the request path uses too.
pub fn dashboard_url() -> String {
    dashboard_url_for(&cloud_url())
}

fn dashboard_url_for(api: &str) -> String {
    // One list of retired hosts, in `normalize_cloud_url`. This used to carry
    // its own copy, which is how the API base and the dashboard link came to
    // disagree about whether api.pylonsync.com still existed: browser links
    // moved to www while every actual request kept going to the dead host.
    normalize_cloud_url(api)
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
    let mut creds: Credentials = serde_json::from_str(&raw).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("credentials.json is malformed ({e}). Delete it and re-run `pylon login`."),
        )
    })?;
    // A token minted before the API and dashboard merged onto one origin
    // stored the now-retired host, and every request below is built from this
    // field. Normalizing here — rather than at each call site — means the
    // whole CLI surface follows the move at once. The token itself is still
    // valid; it is only the address that changed.
    creds.cloud_url = normalize_cloud_url(&creds.cloud_url);
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
    let json =
        serde_json::to_string_pretty(state).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
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

/// Turn a non-2xx response into one readable line.
///
/// The body is NOT the message. When the control plane is restarting, the
/// request never reaches it and Cloudflare answers with a full HTML error page
/// — interpolating that into an error dumped several hundred lines of markup
/// into the terminal and buried the one fact that mattered (a 525 means the
/// edge could not reach the origin). So: JSON errors surface their message,
/// HTML is reduced to its `<title>`, and anything else is truncated.
pub fn describe_http_error(code: u16, body: &str) -> String {
    let trimmed = body.trim();

    // Our own API errors are JSON — `{"error":{"code":…,"message":…}}` or a
    // flat `{"message":…}`. Prefer the message; it is written for a human.
    if trimmed.starts_with('{') {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
            let msg = v
                .pointer("/error/message")
                .or_else(|| v.pointer("/message"))
                .or_else(|| v.pointer("/error"))
                .and_then(|m| m.as_str());
            if let Some(msg) = msg {
                let code_str = v
                    .pointer("/error/code")
                    .and_then(|c| c.as_str())
                    .map(|c| format!(" [{c}]"))
                    .unwrap_or_default();
                return format!("Cloud returned {code}{code_str}: {msg}");
            }
        }
    }

    let looks_html = trimmed.starts_with("<!DOCTYPE")
        || trimmed.starts_with("<!doctype")
        || trimmed.starts_with("<html")
        || trimmed.contains("<title>");
    if looks_html {
        let title = trimmed
            .split_once("<title>")
            .and_then(|(_, rest)| rest.split_once("</title>"))
            .map(|(t, _)| t.trim())
            .filter(|t| !t.is_empty())
            .unwrap_or("HTML error page");
        let hint = edge_error_hint(code);
        return format!("Cloud returned {code}: {title}{hint}");
    }

    let mut oneline = trimmed.replace('\n', " ");
    if oneline.chars().count() > 300 {
        oneline = oneline.chars().take(300).collect::<String>() + "…";
    }
    if oneline.is_empty() {
        return format!("Cloud returned {code}{}", edge_error_hint(code));
    }
    format!("Cloud returned {code}: {oneline}{}", edge_error_hint(code))
}

/// Plain-English gloss for the gateway codes that mean "the control plane is
/// not answering right now", which is almost always a deploy in progress
/// rather than anything wrong with the caller's request.
fn edge_error_hint(code: u16) -> &'static str {
    match code {
        502 | 503 | 504 => {
            "\n  Smallware isn't responding — it may be redeploying. Try again shortly."
        }
        // Cloudflare origin-reachability family. 525/526 are TLS between the
        // edge and the origin; 521/523 are the origin being down or unroutable.
        520..=527 => {
            "\n  Cloudflare couldn't reach the Smallware origin — it's likely mid-deploy. \
             Try again shortly."
        }
        _ => "",
    }
}

/// True when a failed request is worth retrying: the request never reached the
/// control plane, so nothing was half-applied.
///
/// Matches on the shape `describe_http_error` produces, so the formatter and
/// this classifier stay in step. Deliberately narrow — a 4xx means the request
/// itself was wrong and retrying only repeats it.
pub fn is_transient_cloud_error(msg: &str) -> bool {
    let gateway_code = msg
        .strip_prefix("Cloud returned ")
        .and_then(|rest| rest.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|c| c.parse::<u16>().ok())
        .is_some_and(|code| matches!(code, 502 | 503 | 504 | 520..=527));

    gateway_code
        || msg.contains("BUILD_START_FAILED")
        || msg.contains("timed out")
        || msg.contains("Cloud request failed")
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
            Err(describe_http_error(code, &body))
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
            Err(describe_http_error(code, &body))
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
            Err(describe_http_error(code, &body))
        }
        Err(e) => Err(format!("Cloud request failed: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_cloud_url_is_the_smallware_host() {
        // Hosted Pylon Cloud is Smallware. (www.)pylonsync.com now serves the
        // framework's marketing site and has no /api/*, so a CLI pointed there
        // parses HTML as JSON; api.pylonsync.com has no certificate at all and
        // dies in the TLS handshake. All three are retired.
        assert_eq!(DEFAULT_CLOUD_URL, "https://www.usesmallware.com");
    }

    #[test]
    fn a_credential_minted_against_the_retired_host_follows_the_move() {
        // The bug: credentials persist the origin they were minted against
        // and every request is built from that stored value, so a login from
        // before the consolidation kept addressing api.pylonsync.com — which
        // answers with a bare Cloudflare 525, no hint that the host moved.
        assert_eq!(
            normalize_cloud_url("https://api.pylonsync.com"),
            "https://www.usesmallware.com"
        );
        // Trailing slash is the shape `pylon login` actually wrote.
        assert_eq!(
            normalize_cloud_url("https://api.pylonsync.com/"),
            "https://www.usesmallware.com"
        );
    }

    #[test]
    fn a_credential_minted_against_pylonsync_follows_the_rebrand() {
        // Hosted Pylon Cloud became Smallware, and (www.)pylonsync.com was
        // handed to the framework's marketing site. Those hosts still answer —
        // with HTML — so an un-rewritten credential fails as a JSON parse error
        // on a 200, which reads like a corrupt response rather than a moved
        // host. Every stored form has to map across.
        for stored in [
            "https://www.pylonsync.com",
            "https://www.pylonsync.com/",
            "https://pylonsync.com",
            "https://pylonsync.com/",
        ] {
            assert_eq!(normalize_cloud_url(stored), "https://www.usesmallware.com");
        }
    }

    #[test]
    fn a_self_hosted_origin_is_never_rewritten() {
        // The one thing this must not do. Silently redirecting somebody's own
        // install to OUR cloud would send their token to a host they never
        // chose — far worse than the error being fixed.
        for url in [
            "https://pylon.internal.example.com",
            "http://localhost:4321",
            "https://staging.pylonsync.com",
        ] {
            assert_eq!(normalize_cloud_url(url), url);
        }
    }

    #[test]
    fn normalization_is_idempotent_and_strips_a_trailing_slash() {
        let once = normalize_cloud_url("https://www.pylonsync.com/");
        assert_eq!(once, "https://www.usesmallware.com");
        assert_eq!(normalize_cloud_url(&once), once);
    }

    #[test]
    fn dashboard_url_maps_hosted_origins_to_the_smallware_host() {
        // www.usesmallware.com is the canonical dashboard host. Credentials
        // minted against any retired host still map to it so browser links
        // don't land on the marketing site (or a dead one).
        assert_eq!(
            dashboard_url_for("https://www.usesmallware.com"),
            "https://www.usesmallware.com"
        );
        assert_eq!(
            dashboard_url_for("https://www.pylonsync.com"),
            "https://www.usesmallware.com"
        );
        assert_eq!(
            dashboard_url_for("https://api.pylonsync.com"),
            "https://www.usesmallware.com"
        );
        assert_eq!(
            dashboard_url_for("https://api.pylonsync.com/"),
            "https://www.usesmallware.com"
        );
    }

    #[test]
    fn dashboard_url_keeps_self_hosted_origin() {
        assert_eq!(
            dashboard_url_for("https://pylon.internal.example.com"),
            "https://pylon.internal.example.com"
        );
        assert_eq!(
            dashboard_url_for("http://localhost:8080/"),
            "http://localhost:8080"
        );
    }
}

#[cfg(test)]
mod error_format_tests {
    use super::*;

    #[test]
    fn html_error_pages_are_reduced_to_one_line() {
        // The bug: a Cloudflare 525 page was interpolated whole into the error,
        // dumping hundreds of lines of markup over the one fact that mattered.
        let body = "<!DOCTYPE html>\n<html>\n<head>\n<title>usesmallware.com | 525: SSL handshake failed</title>\n</head>\n<body><div>lots of markup</div></body></html>";
        let msg = describe_http_error(525, body);
        assert!(!msg.contains("<div"), "markup leaked: {msg}");
        assert!(!msg.contains("<!DOCTYPE"), "markup leaked: {msg}");
        assert!(msg.contains("525: SSL handshake failed"), "{msg}");
        assert!(
            msg.contains("mid-deploy"),
            "must explain what 525 means: {msg}"
        );
        assert!(msg.lines().count() <= 2, "should stay short: {msg}");
    }

    #[test]
    fn json_api_errors_surface_their_message() {
        let msg = describe_http_error(
            400,
            r#"{"error":{"code":"PAYLOAD_TOO_LARGE","message":"upload exceeds the cap"}}"#,
        );
        assert!(msg.contains("upload exceeds the cap"), "{msg}");
        assert!(msg.contains("PAYLOAD_TOO_LARGE"), "{msg}");
    }

    #[test]
    fn long_plain_bodies_are_truncated() {
        let body = "x".repeat(5000);
        let msg = describe_http_error(500, &body);
        assert!(
            msg.chars().count() < 400,
            "not truncated: {} chars",
            msg.chars().count()
        );
    }

    #[test]
    fn gateway_codes_retry_but_client_errors_do_not() {
        // 520-527 is the family a control plane mid-redeploy returns; the old
        // inline classifier only knew 502/503/504 and gave up on a 525.
        for code in [502u16, 503, 504, 520, 521, 523, 525, 526] {
            let msg = describe_http_error(code, "<html><title>oops</title></html>");
            assert!(is_transient_cloud_error(&msg), "{code} should retry: {msg}");
        }
        for code in [400u16, 401, 403, 404, 413, 422] {
            let msg = describe_http_error(code, r#"{"message":"nope"}"#);
            assert!(
                !is_transient_cloud_error(&msg),
                "{code} must not retry: {msg}"
            );
        }
    }
}
