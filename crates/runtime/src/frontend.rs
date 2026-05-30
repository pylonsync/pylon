//! SPA serving — make Pylon a unified full-stack server.
//!
//! Pylon's promise to the user is "one binary, one port, one process — your
//! whole app." For the API half, that already works (entities, sync, auth,
//! functions, etc.). For the UI half, this module fills the gap: if the
//! project has a built frontend (typically `web/dist/index.html` from a
//! Vite/Next/Astro build), Pylon serves it directly. No reverse proxy
//! sidecar, no `vercel deploy` for the frontend, no CORS gymnastics.
//!
//! Two modes:
//!
//! 1. **Production / single-port**: `PYLON_FRONTEND_DIR` points at a built
//!    `dist/`. The serve path is: requested asset → SPA fallback to
//!    `index.html` for anything that doesn't match a file. This is what
//!    runs on Fly / Docker / Pylon Cloud.
//!
//! 2. **Dev / Vite proxy**: `PYLON_FRONTEND_DEV_PROXY=http://localhost:5173`
//!    forwards non-API GETs to a running Vite dev server (spawned by
//!    `pylon dev`). The user sees one port (:4321) but gets HMR.
//!
//! Either mode is OPT-OUT: if neither env is set AND `./web/dist` doesn't
//! exist next to `app.ts`, this module no-ops and the API-only behavior
//! is preserved.
//!
//! Path safety: every served path is canonicalized and verified to remain
//! inside the frontend root. Path-traversal attempts (`../../etc/passwd`)
//! fall through to SPA fallback or 404.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tiny_http::{Header, Method, Request, Response};

/// Build progress / outcome surfaced to the SPA handler so non-API
/// GETs can render a useful holding page while `bun install + bun
/// run build` finishes on first boot.
#[derive(Clone, Debug)]
pub enum BuildState {
    /// No build was kicked off (no `web/` dir or no build script).
    /// Default state — SPA serving falls through to API routing.
    Idle,
    /// Build thread is running. Non-API GETs see the building page.
    InProgress,
    /// Build finished successfully. SPA serves from disk.
    Ready,
    /// Build failed. Non-API GETs see an error page with the diagnostic.
    Failed(String),
}

/// Shared handle the CLI uses to publish build progress and the
/// runtime reads to decide what to serve. Cheap to `Clone` (Arc) so
/// the spawn closure can own one and the FrontendConfig can hold
/// another.
pub type SharedBuildState = Arc<RwLock<BuildState>>;

/// Process-wide singleton so the CLI's `pylon start` can publish
/// build progress and the runtime's start_server can read it without
/// threading a handle through every API boundary.
///
/// Static — one per process. The build only fires once per pylon
/// start (mtime-marker fast-path on warm boots means the worker
/// flips straight to Ready), so a single global state matches
/// reality and avoids leaking an Arc through every CLI → runtime
/// call site.
pub fn shared_build_state() -> SharedBuildState {
    use std::sync::OnceLock;
    static STATE: OnceLock<SharedBuildState> = OnceLock::new();
    STATE
        .get_or_init(|| Arc::new(RwLock::new(BuildState::Idle)))
        .clone()
}

pub fn mark_build_in_progress(state: &SharedBuildState) {
    if let Ok(mut s) = state.write() {
        *s = BuildState::InProgress;
    }
}

pub fn mark_build_ready(state: &SharedBuildState) {
    if let Ok(mut s) = state.write() {
        *s = BuildState::Ready;
    }
}

pub fn mark_build_failed(state: &SharedBuildState, msg: String) {
    if let Ok(mut s) = state.write() {
        *s = BuildState::Failed(msg);
    }
}

fn read_build_state(state: &SharedBuildState) -> BuildState {
    state.read().map(|s| s.clone()).unwrap_or(BuildState::Idle)
}

/// Resolved frontend config. Built once at startup and shared (cheap to
/// `Clone`).
#[derive(Clone)]
pub struct FrontendConfig {
    /// Absolute path to the built frontend dir. `index.html` must live
    /// directly inside.
    pub dir: Option<PathBuf>,
    /// Vite-style dev server URL. If set, takes precedence over `dir` —
    /// non-API GETs are proxied here instead of served from disk.
    pub dev_proxy: Option<String>,
    /// SSR routes (mode == "ssr") + their components, walked at every
    /// HTTP GET to decide whether to dispatch to the Bun-side renderer
    /// instead of the static/proxy path. Empty when the manifest has
    /// no SSR routes; the SSR branch becomes a no-op in that case.
    pub ssr_routes: std::sync::Arc<Vec<pylon_kernel::ManifestRoute>>,
    /// Function-runner handle, used to invoke `render_route` for SSR
    /// matches. None when no functions backend is wired (test stubs,
    /// pre-functions builds) — SSR branch falls through.
    pub fn_ops: Option<std::sync::Arc<dyn pylon_router::FnOps>>,
}

impl std::fmt::Debug for FrontendConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrontendConfig")
            .field("dir", &self.dir)
            .field("dev_proxy", &self.dev_proxy)
            .field("ssr_routes_count", &self.ssr_routes.len())
            .field("has_fn_ops", &self.fn_ops.is_some())
            .finish()
    }
}

impl FrontendConfig {
    /// Resolve config from env + the app's working dir.
    ///
    /// `app_dir` is where `app.ts` was loaded from — typically the cwd
    /// the runtime was started in. `web/dist` is checked relative to it.
    pub fn from_env(app_dir: &Path) -> Self {
        let dev_proxy = std::env::var("PYLON_FRONTEND_DEV_PROXY")
            .ok()
            .filter(|s| !s.is_empty());

        let dir = if let Ok(explicit) = std::env::var("PYLON_FRONTEND_DIR") {
            if explicit.is_empty() {
                None
            } else {
                let p = PathBuf::from(&explicit);
                if p.is_absolute() {
                    Some(p)
                } else {
                    Some(app_dir.join(p))
                }
            }
        } else {
            // Default discovery: <app>/web/dist or <app>/apps/web/dist.
            // First hit wins; matches the layout the `pylon init`
            // template + the examples use.
            //
            // Fallback candidates: /data/.pylon-frontend-build/web/dist
            // and /tmp/.pylon-frontend-build/web/dist — these are where
            // the CLI's `ensure_frontend_built` writes when /app/web/
            // is read-only (Pylon Cloud / Fly files-mount with root
            // ownership of source dir).
            let candidates = [
                app_dir.join("web/dist"),
                app_dir.join("apps/web/dist"),
                PathBuf::from("/data/.pylon-frontend-build/web/dist"),
                PathBuf::from("/tmp/.pylon-frontend-build/web/dist"),
            ];
            candidates
                .into_iter()
                .find(|p| p.join("index.html").is_file())
        };

        Self {
            dir,
            dev_proxy,
            ssr_routes: std::sync::Arc::new(Vec::new()),
            fn_ops: None,
        }
    }

    /// Populate SSR routes + fn_ops for the SSR dispatch branch. Called
    /// once from server bootstrap after the runtime + functions are
    /// online. Passing an empty `routes` vec is a no-op (SSR branch
    /// stays inactive).
    pub fn with_ssr(
        mut self,
        routes: std::sync::Arc<Vec<pylon_kernel::ManifestRoute>>,
        fn_ops: Option<std::sync::Arc<dyn pylon_router::FnOps>>,
    ) -> Self {
        self.ssr_routes = routes;
        self.fn_ops = fn_ops;
        self
    }

    /// Is anything wired up?
    ///
    /// True when we have a built dist on disk, a dev proxy URL, OR
    /// the async builder was at least started. The build-state check
    /// is what lets the runtime serve a "building" page during first
    /// boot even though FrontendConfig::from_env at startup saw no
    /// dist; the Ready branch keeps it active so the discover-after-
    /// build path in `try_handle` can pick up the freshly-written
    /// dist/. Only `Idle` (no build kicked off at all) returns false,
    /// preserving the API-only behavior for projects without a `web/`.
    pub fn is_active(&self) -> bool {
        if self.dir.is_some() || self.dev_proxy.is_some() {
            return true;
        }
        !matches!(read_build_state(&shared_build_state()), BuildState::Idle)
    }
}

/// Is this URL eligible for SPA handling?
///
/// API + framework routes always take precedence — even if they 404 in
/// the router, we don't want to swap that for an HTML page (which would
/// confuse JSON clients and mask real bugs). Returning HTML for `/api/*`
/// hits would also be a security smell (response-type confusion).
fn is_spa_eligible(url: &str) -> bool {
    let path = url.split('?').next().unwrap_or(url);
    !(path.starts_with("/api/")
        || path == "/api"
        || path.starts_with("/studio")
        || path.starts_with("/events")
        || path.starts_with("/metrics")
        || path == "/health"
        || path.starts_with("/health/")
        || path.starts_with("/admin/")
        || path.starts_with("/.well-known/"))
}

/// Best-effort MIME detection from extension. The set covers what a
/// Vite/Next/Astro build typically emits; unknown extensions fall to
/// `application/octet-stream` (which browsers handle fine for downloads
/// but won't auto-execute, so it's the safe default).
fn content_type_for(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") | Some("htm") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "application/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("map") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("avif") => "image/avif",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        Some("wasm") => "application/wasm",
        Some("txt") => "text/plain; charset=utf-8",
        Some("xml") => "application/xml",
        Some("pdf") => "application/pdf",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        _ => "application/octet-stream",
    }
}

/// Resolve a request path to a file inside `root`, defending against
/// path traversal.
///
/// `root` must already exist on disk. The returned path is guaranteed
/// to be a regular file inside `root` (after canonicalization) or
/// `None`. Symlinks pointing outside `root` are rejected.
fn resolve_safe(root: &Path, request_path: &str) -> Option<PathBuf> {
    // Strip leading '/' and any query/fragment. Reject explicit
    // traversal up front — even if the canonicalize check below would
    // catch it, refusing here avoids a syscall.
    let path_only = request_path.split('?').next()?.split('#').next()?;
    let trimmed = path_only.trim_start_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    if trimmed
        .split('/')
        .any(|seg| seg == ".." || seg == "." || seg.is_empty())
    {
        return None;
    }

    let candidate = root.join(trimmed);
    let canonical_candidate = candidate.canonicalize().ok()?;
    let canonical_root = root.canonicalize().ok()?;
    if !canonical_candidate.starts_with(&canonical_root) {
        return None;
    }
    if canonical_candidate.is_file() {
        Some(canonical_candidate)
    } else {
        None
    }
}

/// Top-level entry. Decides between dev-proxy and disk-serving based
/// on config, applies eligibility rules, sends the response.
///
/// `Ok(())` = response was sent (success or upstream error). The caller
/// should record metrics and exit the worker. `Err(request)` = path was
/// API-bound or unhandled; the caller continues to existing routing
/// with the same request restored.
pub fn try_handle(
    cfg: &FrontendConfig,
    request: Request,
    cors_origin: &str,
) -> Result<(), Request> {
    // Only GET / HEAD get the SPA treatment. POST/PATCH/DELETE always
    // belong to API routing — serving them HTML would be a silent
    // failure mode that's hard to debug.
    if !matches!(request.method(), Method::Get | Method::Head) {
        return Err(request);
    }

    let url = request.url().to_string();
    if !is_spa_eligible(&url) {
        return Err(request);
    }

    // SSR branch sits ABOVE dev_proxy so file-based pages take
    // precedence over Vite's catch-all (Vite would serve the SPA
    // shell, masking the SSR'd output). Falls through to proxy/disk
    // when no SSR route matches.
    if !cfg.ssr_routes.is_empty() && cfg.fn_ops.is_some() {
        if let Some(matched) = match_ssr_route(&url, &cfg.ssr_routes) {
            tracing::debug!(
                url = %url,
                route = %matched.route.path,
                "SSR match"
            );
            return serve_via_ssr_rpc(cfg, matched, request, cors_origin);
        }
    }

    if let Some(proxy_base) = cfg.dev_proxy.as_deref() {
        return serve_via_proxy(proxy_base, request, cors_origin);
    }

    // Build state takes precedence over a stale dist on disk. If the
    // current boot's build failed (mark_build_failed), surface the
    // operator-visible Failed page instead of silently serving the
    // PREVIOUS deploy's SPA shell. Without this gate, a broken deploy
    // that landed after a working one would render the old UI and
    // the operator would never see the build-error page — they'd
    // only notice when the new feature didn't appear.
    //
    // InProgress similarly takes precedence so the user sees the
    // Building... shell instead of momentarily-stale SPA flashes
    // during a re-build triggered by a config edit.
    match read_build_state(&shared_build_state()) {
        BuildState::Failed(msg) => return serve_build_failed(request, &msg, cors_origin),
        BuildState::InProgress => {
            // Only intercept with the building page when there's no
            // disk-served SPA yet — once the FIRST build of this
            // machine succeeded, subsequent restarts serve from disk
            // immediately (the marker-fast-skip path keeps state
            // Ready in practice; InProgress on a warm-marker boot
            // would be a re-build kicked off by source changes).
            //
            // Without this guard, a brief InProgress window during
            // a config edit would 503 every existing SPA request
            // even though dist/ on disk is fully serviceable.
            if cfg.dir.is_none()
                && discover_dist_dir(&std::env::current_dir().ok().unwrap_or_default()).is_none()
            {
                return serve_build_in_progress(request, cors_origin);
            }
        }
        BuildState::Idle | BuildState::Ready => {}
    }

    // Resolve the serve directory, accounting for a build that
    // finished AFTER startup (FrontendConfig::from_env captured a
    // snapshot at boot, but the async builder writes to disk after
    // that). The lookup is cheap (a couple of stat calls) and only
    // runs when the boot-captured dir was None — once we get a hit
    // we'd ideally cache, but the cost is small enough vs the
    // refactor needed to mutate config from here that we re-check
    // per request. Operators who set PYLON_FRONTEND_DIR get the
    // configured path with zero ambiguity.
    let resolved_dir: Option<PathBuf> = cfg
        .dir
        .clone()
        .or_else(|| discover_dist_dir(&std::env::current_dir().ok().unwrap_or_default()));

    if resolved_dir.is_none() {
        // Nothing to serve from disk. If we got here, BuildState is
        // Idle or Ready-with-no-dist (build claimed success but
        // produced no index.html — operator should see the API 404
        // for that misconfiguration).
        return Err(request);
    }
    let dir = resolved_dir.unwrap();
    serve_from_disk(&dir, request, &url, cors_origin)
}

/// Re-run the default frontend-dir discovery (matches
/// `FrontendConfig::from_env`'s lookup). Used after the async build
/// finishes so the next request picks up the freshly-built dist
/// without a process restart.
fn discover_dist_dir(app_dir: &Path) -> Option<PathBuf> {
    let candidates = [
        app_dir.join("web/dist"),
        app_dir.join("apps/web/dist"),
        // CLI's ensure_frontend_built falls back to these locations
        // when /app/web/ is read-only (Pylon Cloud / Fly): keep the
        // discovery list in sync with bun.rs's resolve_build_dir.
        PathBuf::from("/data/.pylon-frontend-build/web/dist"),
        PathBuf::from("/tmp/.pylon-frontend-build/web/dist"),
    ];
    candidates
        .into_iter()
        .find(|p| p.join("index.html").is_file())
}

/// Status page shown while `bun install + bun run build` runs in the
/// background. Auto-refreshes every 3 seconds so the user lands on
/// the real SPA as soon as the build completes (next request will
/// either still 503 if dist isn't there yet, or hit the cfg.dir
/// branch if it was discovered on the next FrontendConfig::from_env).
fn serve_build_in_progress(request: Request, cors_origin: &str) -> Result<(), Request> {
    // 503 with auto-refresh so the page becomes a live status panel.
    // The 3-second interval is fast enough that the user perceives a
    // "loading" feel but slow enough not to hammer the server on a
    // long npm install. Plain-text fallback if HTML rendering is off
    // (e.g. curl).
    let body = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta http-equiv="refresh" content="3">
  <title>Building…</title>
  <style>
    html, body { margin: 0; padding: 0; height: 100%; font-family: ui-sans-serif, system-ui, sans-serif; background: #0a0a0a; color: #fafafa; }
    .wrap { height: 100%; display: grid; place-items: center; }
    .card { text-align: center; }
    .spinner { width: 24px; height: 24px; margin: 0 auto 16px; border: 2px solid #333; border-top-color: #fafafa; border-radius: 50%; animation: spin 0.8s linear infinite; }
    @keyframes spin { to { transform: rotate(360deg); } }
    h1 { font-size: 16px; font-weight: 500; margin: 0 0 8px; }
    p { font-size: 14px; color: #999; margin: 0; }
  </style>
</head>
<body>
  <div class="wrap">
    <div class="card">
      <div class="spinner"></div>
      <h1>Building your app…</h1>
      <p>First boot — installing dependencies and bundling the frontend.</p>
    </div>
  </div>
</body>
</html>"#;
    let response = Response::from_data(body.as_bytes().to_vec())
        .with_status_code(503u16)
        .with_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap())
        .with_header(
            Header::from_bytes(
                "Access-Control-Allow-Origin",
                cors_origin.as_bytes().to_vec(),
            )
            .unwrap(),
        )
        // Retry-After tells well-behaved clients (and Fly's healthcheck) to
        // back off. 5s aligns with the auto-refresh and signals "transient,
        // not a permanent error."
        .with_header(Header::from_bytes("Retry-After", "5").unwrap())
        .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap());
    let _ = request.respond(response);
    Ok(())
}

/// Status page shown when the frontend build failed at startup.
/// Includes the diagnostic message so the operator can see what
/// broke without grepping logs. Returns 500 (not 503) because this
/// is a permanent error until the deploy is fixed.
fn serve_build_failed(request: Request, msg: &str, cors_origin: &str) -> Result<(), Request> {
    let body = format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>Frontend build failed</title>
  <style>
    html, body {{ margin: 0; padding: 0; height: 100%; font-family: ui-sans-serif, system-ui, sans-serif; background: #0a0a0a; color: #fafafa; }}
    .wrap {{ max-width: 720px; margin: 0 auto; padding: 64px 24px; }}
    h1 {{ font-size: 18px; margin: 0 0 16px; color: #ef4444; }}
    pre {{ background: #18181b; color: #fafafa; padding: 16px; border-radius: 8px; overflow-x: auto; font-size: 13px; line-height: 1.5; }}
    p {{ font-size: 14px; color: #999; }}
  </style>
</head>
<body>
  <div class="wrap">
    <h1>Frontend build failed</h1>
    <p>The runtime started but the SPA couldn't be built. Check the deploy logs and fix the underlying issue, then redeploy.</p>
    <pre>{}</pre>
  </div>
</body>
</html>"#,
        html_escape(msg)
    );
    let response = Response::from_data(body.into_bytes())
        .with_status_code(500u16)
        .with_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap())
        .with_header(
            Header::from_bytes(
                "Access-Control-Allow-Origin",
                cors_origin.as_bytes().to_vec(),
            )
            .unwrap(),
        )
        .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap());
    let _ = request.respond(response);
    Ok(())
}

/// Minimal HTML escape so a build error containing `<` / `>` doesn't
/// break the status page layout (or worse, get treated as markup).
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Serve a path off disk with SPA fallback to `index.html`.
fn serve_from_disk(
    dir: &Path,
    request: Request,
    url: &str,
    cors_origin: &str,
) -> Result<(), Request> {
    let path_only = url.split('?').next().unwrap_or(url);

    // Direct file hit. `resolve_safe` returns Some only when the path
    // points to a regular file inside the dir.
    if let Some(file_path) = resolve_safe(dir, path_only) {
        let bytes = match std::fs::read(&file_path) {
            Ok(b) => b,
            Err(_) => return Err(request),
        };
        let ct = content_type_for(&file_path);
        let response = Response::from_data(bytes)
            .with_status_code(200)
            .with_header(Header::from_bytes("Content-Type", ct).unwrap())
            .with_header(
                Header::from_bytes(
                    "Access-Control-Allow-Origin",
                    cors_origin.as_bytes().to_vec(),
                )
                .unwrap(),
            )
            .with_header(
                // Hashed assets (Vite emits ?v= and chunk-hash filenames)
                // can be cached aggressively, but the SPA shell itself
                // must always re-validate so a deploy bump is picked up
                // on the next page load. Use a conservative one-hour
                // public cache as the default — operators who want
                // longer can put a CDN in front.
                Header::from_bytes("Cache-Control", "public, max-age=3600").unwrap(),
            );
        let _ = request.respond(response);
        return Ok(());
    }

    // SPA fallback: any non-matching path under a client-side router
    // should resolve to index.html. Browser does the rest.
    let index = dir.join("index.html");
    if !index.is_file() {
        // No SPA shell at all — let the caller continue to API routing
        // so a misconfigured deploy at least returns the API's NOT_FOUND
        // hint instead of a confusing 404 from us.
        return Err(request);
    }
    let bytes = match std::fs::read(&index) {
        Ok(b) => b,
        Err(_) => return Err(request),
    };
    let response = Response::from_data(bytes)
        .with_status_code(200)
        .with_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap())
        .with_header(
            Header::from_bytes(
                "Access-Control-Allow-Origin",
                cors_origin.as_bytes().to_vec(),
            )
            .unwrap(),
        )
        .with_header(
            // index.html must NEVER be long-cached or a deploy bump
            // shows stale UI until the browser cache expires.
            Header::from_bytes("Cache-Control", "no-cache, must-revalidate").unwrap(),
        );
    let _ = request.respond(response);
    Ok(())
}

/// Dev mode: proxy the request to a Vite-like dev server. Used by
/// `pylon dev` so the user only ever sees :4321.
///
/// Errors fall back to a 502 (rather than continuing to API routing)
/// because in dev mode the user has explicitly opted into the proxy —
/// silently routing to the API on a proxy failure would mask the
/// real "Vite isn't running" problem.
fn serve_via_proxy(proxy_base: &str, request: Request, cors_origin: &str) -> Result<(), Request> {
    let url = request.url().to_string();
    let target = format!("{}{}", proxy_base.trim_end_matches('/'), url);

    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(30))
        .build();

    // Forward a minimal set of headers — host gets rewritten by ureq
    // automatically. Don't forward Authorization (the dev server has
    // no business seeing API bearer tokens).
    let mut req = agent.request("GET", &target);
    for h in request.headers() {
        let name = h.field.as_str().as_str();
        if matches!(
            name.to_ascii_lowercase().as_str(),
            "host" | "authorization" | "cookie" | "connection" | "content-length"
        ) {
            continue;
        }
        req = req.set(name, h.value.as_str());
    }

    match req.call() {
        Ok(upstream) => {
            let status = upstream.status();
            let ct = upstream
                .header("Content-Type")
                .unwrap_or("application/octet-stream")
                .to_string();
            let mut buf = Vec::new();
            if upstream.into_reader().read_to_end(&mut buf).is_err() {
                // Connection broke mid-stream — return 502 rather than
                // a partial body.
                let body = b"upstream read failed".to_vec();
                let response = Response::from_data(body)
                    .with_status_code(502u16)
                    .with_header(Header::from_bytes("Content-Type", "text/plain").unwrap());
                let _ = request.respond(response);
                return Ok(());
            }
            let response = Response::from_data(buf)
                .with_status_code(status)
                .with_header(Header::from_bytes("Content-Type", ct.as_bytes()).unwrap())
                .with_header(
                    Header::from_bytes(
                        "Access-Control-Allow-Origin",
                        cors_origin.as_bytes().to_vec(),
                    )
                    .unwrap(),
                );
            let _ = request.respond(response);
            Ok(())
        }
        Err(ureq::Error::Status(status, response)) => {
            let ct = response
                .header("Content-Type")
                .unwrap_or("text/plain")
                .to_string();
            let mut buf = Vec::new();
            let _ = response.into_reader().read_to_end(&mut buf);
            let resp = Response::from_data(buf)
                .with_status_code(status)
                .with_header(Header::from_bytes("Content-Type", ct.as_bytes()).unwrap());
            let _ = request.respond(resp);
            Ok(())
        }
        Err(ureq::Error::Transport(_)) => {
            // Vite not up / crashed. 502 with a hint so the user knows
            // to check the dev process, not the API.
            let body =
                b"frontend dev server not reachable (set PYLON_FRONTEND_DEV_PROXY correctly or start `bun run dev`)"
                    .to_vec();
            let response = Response::from_data(body)
                .with_status_code(502u16)
                .with_header(Header::from_bytes("Content-Type", "text/plain").unwrap());
            let _ = request.respond(response);
            Ok(())
        }
    }
}

// `into_reader` returns a `Box<dyn Read>`. Bring `Read` into scope so
// `.read_to_end` resolves without a turbofish.
use std::io::Read as _;

/// An SSR route hit by an incoming GET, with dynamic-segment
/// parameters extracted from the URL path.
pub struct SsrMatch {
    /// The matched route's manifest entry (route_path, component,
    /// layout chain, auth requirement).
    route: pylon_kernel::ManifestRoute,
    /// `:slug` → "hello-world" mappings extracted from the URL.
    /// Empty when the route has no dynamic segments.
    params: std::collections::HashMap<String, String>,
}

/// Match an incoming URL path against the SSR route table.
///
/// Comparison strategy: split each route_path and the URL path on
/// `/`. Lengths must match. Each segment matches literally unless
/// it starts with `:` (e.g. `:slug`), in which case it captures
/// any non-empty segment value. First match wins — the discoverer
/// orders routes deterministically (literal segments before
/// parameterized ones at the same depth) so longest-prefix winners
/// surface first.
///
/// Returns `None` for non-SSR URLs (no match in the table). The
/// caller falls through to the dev-proxy / disk branch.
pub fn match_ssr_route(url: &str, routes: &[pylon_kernel::ManifestRoute]) -> Option<SsrMatch> {
    let path = url.split('?').next().unwrap_or(url);
    let url_segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    for r in routes {
        if r.mode != "ssr" {
            continue;
        }
        let route_segs: Vec<&str> = r.path.split('/').filter(|s| !s.is_empty()).collect();
        if route_segs.len() != url_segs.len() {
            continue;
        }
        let mut params = std::collections::HashMap::new();
        let mut matched = true;
        for (rs, us) in route_segs.iter().zip(url_segs.iter()) {
            if let Some(name) = rs.strip_prefix(':') {
                if us.is_empty() {
                    matched = false;
                    break;
                }
                params.insert(name.to_string(), (*us).to_string());
            } else if rs != us {
                matched = false;
                break;
            }
        }
        if matched {
            return Some(SsrMatch {
                route: r.clone(),
                params,
            });
        }
    }
    None
}

/// Dispatch a matched SSR request to the Bun-side renderer via the
/// `render_route` RPC and serve the response.
///
/// Phase 1: buffered. The full rendered body accumulates into a
/// `Vec<u8>` before the response goes back to the client. True
/// chunked streaming follows in Phase 1.5 — the streaming pipe is
/// already non-buffered end-to-end (`Bun stdout → Rust mpsc →
/// tiny_http chunked`); this just wires it to a `StreamingBody`
/// instead of a `Vec`. Buffered now keeps the diff small enough to
/// land safely; streaming pulls in `StreamingBody` visibility +
/// `spawn_streaming_response` from `server.rs`.
fn serve_via_ssr_rpc(
    cfg: &FrontendConfig,
    matched: SsrMatch,
    request: Request,
    cors_origin: &str,
) -> Result<(), Request> {
    let fn_ops = match cfg.fn_ops.as_ref() {
        Some(f) => f.clone(),
        None => return Err(request),
    };
    let component = match matched.route.component.as_deref() {
        Some(c) => c.to_string(),
        None => {
            // Misconfiguration: an SSR-mode route without a component.
            // Should be caught by `pylon lint` before reaching here;
            // log + fall through so the SPA branch can take over.
            tracing::warn!(
                route = %matched.route.path,
                "SSR route has no component; falling through to SPA"
            );
            return Err(request);
        }
    };

    let url = request.url().to_string();
    let (path_only, query) = match url.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (url.clone(), String::new()),
    };

    // Headers + cookies forwarded to the page so layout/auth can
    // render. Cookies are parsed out as a separate map (Set-Cookie-
    // style, not the raw header line) — most page components want
    // the named map.
    let mut headers_map: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut cookies_map: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for h in request.headers() {
        let name = h.field.as_str().as_str().to_ascii_lowercase();
        let value = h.value.as_str().to_string();
        if name == "cookie" {
            for pair in value.split(';') {
                let p = pair.trim();
                if let Some((k, v)) = p.split_once('=') {
                    cookies_map.insert(k.to_string(), v.to_string());
                }
            }
        }
        headers_map.insert(name, value);
    }

    let params_json =
        serde_json::to_value(&matched.params).unwrap_or_else(|_| serde_json::json!({}));
    let search_params_json = parse_query_string(&query);

    // Phase 1: no auth plumbing yet (Phase 2). Anonymous AuthInfo so
    // the page renders public-mode HTML for the smoke test. Auth-aware
    // rendering follows the session-cookie resolution + needs the same
    // SessionStore wiring that /api/auth/me uses.
    let auth = pylon_functions::protocol::AuthInfo {
        user_id: None,
        is_admin: false,
        tenant_id: None,
        roles: vec![],
    };

    // Buffer the full render. Phase 1.5 streams this through tiny_http
    // chunked transfer.
    let body_buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let status_holder = std::sync::Arc::new(std::sync::Mutex::new(200u16));
    let headers_holder = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::<
        String,
        String,
    >::new()));

    let body_clone = std::sync::Arc::clone(&body_buf);
    let status_clone = std::sync::Arc::clone(&status_holder);
    let headers_clone = std::sync::Arc::clone(&headers_holder);

    let on_chunk: pylon_functions::runner::ByteStreamCallback = Box::new(move |bytes: &[u8]| {
        if let Ok(mut buf) = body_clone.lock() {
            buf.extend_from_slice(bytes);
        }
    });
    let on_response_start: Option<pylon_functions::runner::ResponseStartCallback> =
        Some(Box::new(move |status, hdrs| {
            if let Ok(mut s) = status_clone.lock() {
                *s = status;
            }
            if let Ok(mut h) = headers_clone.lock() {
                *h = hdrs;
            }
        }));

    let render_result = fn_ops.render_route(
        &component,
        &matched.route.path,
        &path_only,
        params_json,
        search_params_json,
        headers_map,
        cookies_map,
        auth,
        on_response_start,
        on_chunk,
    );

    if let Err(e) = render_result {
        // Render failed BEFORE the first chunk — return 500 with the
        // error code embedded. Phase 1.5 will distinguish "before any
        // bytes flushed" (recoverable to 500) vs "mid-stream" (must
        // truncate-and-drop).
        tracing::error!(
            code = %e.code,
            message = %e.message,
            "SSR render failed"
        );
        let body = format!(
            "<!DOCTYPE html><html><body><h1>SSR error</h1><pre>{}: {}</pre></body></html>",
            html_escape(&e.code),
            html_escape(&e.message)
        );
        let response = Response::from_data(body.into_bytes())
            .with_status_code(500u16)
            .with_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap())
            .with_header(
                Header::from_bytes(
                    "Access-Control-Allow-Origin",
                    cors_origin.as_bytes().to_vec(),
                )
                .unwrap(),
            );
        let _ = request.respond(response);
        return Ok(());
    }

    let final_status = *status_holder.lock().unwrap();
    let final_headers = headers_holder.lock().unwrap().clone();
    let body_vec = body_buf.lock().unwrap().clone();

    let content_type = final_headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| "text/html; charset=utf-8".to_string());

    let mut response = Response::from_data(body_vec)
        .with_status_code(final_status)
        .with_header(Header::from_bytes("Content-Type", content_type.as_bytes()).unwrap())
        .with_header(
            Header::from_bytes(
                "Access-Control-Allow-Origin",
                cors_origin.as_bytes().to_vec(),
            )
            .unwrap(),
        );
    for (k, v) in final_headers.iter() {
        if k.eq_ignore_ascii_case("content-type")
            || k.eq_ignore_ascii_case("content-length")
            || k.eq_ignore_ascii_case("access-control-allow-origin")
        {
            continue;
        }
        if let Ok(h) = Header::from_bytes(k.as_bytes(), v.as_bytes().to_vec()) {
            response.add_header(h);
        }
    }
    let _ = request.respond(response);
    Ok(())
}

fn parse_query_string(q: &str) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for pair in q.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = match pair.split_once('=') {
            Some((k, v)) => (percent_decode(k), percent_decode(v)),
            None => (percent_decode(pair), String::new()),
        };
        map.insert(k, serde_json::Value::String(v));
    }
    serde_json::Value::Object(map)
}

/// Minimal %XX + `+` decoder for query-string values. Loud-fails on
/// malformed sequences by leaving them as-is; the Phase 1 contract
/// is "best-effort decode, never crash" — strict validation belongs
/// at the page-component layer.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'+' {
            out.push(b' ');
            i += 1;
        } else if b == b'%' && i + 2 < bytes.len() {
            let h = bytes[i + 1];
            let l = bytes[i + 2];
            let hi = hex_nibble(h);
            let lo = hex_nibble(l);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi << 4) | lo);
                i += 3;
            } else {
                out.push(b);
                i += 1;
            }
        } else {
            out.push(b);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn api_paths_are_ineligible() {
        assert!(!is_spa_eligible("/api/auth/me"));
        assert!(!is_spa_eligible("/api"));
        assert!(!is_spa_eligible("/api/sync/pull?since=0"));
        assert!(!is_spa_eligible("/studio"));
        assert!(!is_spa_eligible("/studio/api/login"));
        assert!(!is_spa_eligible("/events/foo"));
        assert!(!is_spa_eligible("/metrics"));
        assert!(!is_spa_eligible("/health"));
        assert!(!is_spa_eligible("/health/deep"));
        assert!(!is_spa_eligible("/admin/logs/tail"));
        assert!(!is_spa_eligible("/.well-known/acme-challenge/x"));
    }

    #[test]
    fn html_paths_are_eligible() {
        assert!(is_spa_eligible("/"));
        assert!(is_spa_eligible("/channels"));
        assert!(is_spa_eligible("/channels/general"));
        assert!(is_spa_eligible("/assets/index-abc123.js"));
        assert!(is_spa_eligible("/favicon.ico"));
    }

    #[test]
    fn resolve_safe_rejects_traversal() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("assets")).unwrap();
        fs::write(root.join("assets/app.js"), b"// ok").unwrap();

        // Real file hit
        assert!(resolve_safe(root, "/assets/app.js").is_some());

        // Traversal
        assert!(resolve_safe(root, "/../etc/passwd").is_none());
        assert!(resolve_safe(root, "/assets/../../../etc/passwd").is_none());
        assert!(resolve_safe(root, "/./assets/app.js").is_none());

        // Missing file
        assert!(resolve_safe(root, "/assets/missing.js").is_none());

        // Empty / root path returns None (caller will use index.html fallback)
        assert!(resolve_safe(root, "/").is_none());
        assert!(resolve_safe(root, "").is_none());
    }

    #[test]
    fn from_env_finds_default_layout() {
        let tmp = TempDir::new().unwrap();
        let app = tmp.path();
        fs::create_dir_all(app.join("web/dist")).unwrap();
        fs::write(app.join("web/dist/index.html"), "<!doctype html>").unwrap();

        // PYLON_FRONTEND_DIR / PYLON_FRONTEND_DEV_PROXY are per-process
        // env vars — running tests in parallel against this would race.
        // Take a serial-test lock if/when this grows; for now, both
        // tests in this file are read-only re: env.
        let saved_dir = std::env::var("PYLON_FRONTEND_DIR").ok();
        let saved_proxy = std::env::var("PYLON_FRONTEND_DEV_PROXY").ok();
        std::env::remove_var("PYLON_FRONTEND_DIR");
        std::env::remove_var("PYLON_FRONTEND_DEV_PROXY");

        let cfg = FrontendConfig::from_env(app);
        assert!(cfg.is_active());
        assert_eq!(
            cfg.dir.as_ref().map(|p| p.canonicalize().unwrap()),
            Some(app.join("web/dist").canonicalize().unwrap())
        );

        if let Some(v) = saved_dir {
            std::env::set_var("PYLON_FRONTEND_DIR", v);
        }
        if let Some(v) = saved_proxy {
            std::env::set_var("PYLON_FRONTEND_DEV_PROXY", v);
        }
    }
}
