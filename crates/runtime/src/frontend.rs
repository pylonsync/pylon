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
    /// Session store for resolving the SSR request's auth context
    /// from the request's session cookie. None → SSR pages render
    /// with anonymous AuthInfo (Phase 1 behavior).
    pub session_store: Option<std::sync::Arc<pylon_auth::SessionStore>>,
    /// Cookie config — used to find the session cookie by name on
    /// the incoming request. Pair with session_store.
    pub cookie_config: Option<std::sync::Arc<pylon_auth::CookieConfig>>,
}

impl std::fmt::Debug for FrontendConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrontendConfig")
            .field("dir", &self.dir)
            .field("dev_proxy", &self.dev_proxy)
            .field("ssr_routes_count", &self.ssr_routes.len())
            .field("has_fn_ops", &self.fn_ops.is_some())
            .field("has_session_store", &self.session_store.is_some())
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
            session_store: None,
            cookie_config: None,
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

    /// Wire session resolution so SSR pages render with the right
    /// auth context. Without this, render_route fires with anonymous
    /// `AuthInfo` regardless of whether the request carries a valid
    /// session cookie.
    pub fn with_session(
        mut self,
        session_store: std::sync::Arc<pylon_auth::SessionStore>,
        cookie_config: std::sync::Arc<pylon_auth::CookieConfig>,
    ) -> Self {
        self.session_store = Some(session_store);
        self.cookie_config = Some(cookie_config);
        self
    }

    /// Is anything wired up?
    ///
    /// True when we have a built dist on disk, a dev proxy URL, native
    /// SSR routes from the manifest, OR the async builder was at least
    /// started.
    ///
    /// The SSR-routes check is load-bearing for the native full-stack
    /// app: a project whose `app/**/page.tsx` files produced `mode:"ssr"`
    /// routes has NO `web/dist` (the per-route chunks live under
    /// `.pylon/client-build`, built lazily by the function runtime on the
    /// first `render_route`) and may not run the legacy `web/` build
    /// thread at all — so without this clause `is_active()` returned false
    /// and every page fell through to the API router as a 404. An app
    /// with SSR routes IS a frontend app; activate the dispatcher so
    /// `try_handle` can match + render them.
    ///
    /// The build-state check still lets the runtime serve a "building"
    /// page during first boot of a legacy `web/` frontend. Only `Idle`
    /// (no dist, no proxy, no SSR routes, no build kicked off) returns
    /// false, preserving API-only behavior for backends with no frontend.
    pub fn is_active(&self) -> bool {
        if self.dir.is_some() || self.dev_proxy.is_some() || !self.ssr_routes.is_empty() {
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

/// Minimal percent-decoder for a query-string value. `encodeURIComponent`
/// (the og `src` encoder) escapes `/` to `%2F` and never emits `+`, so we
/// only need `%XX` handling — no `+`→space.
fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hi = (b[i + 1] as char).to_digit(16);
            let lo = (b[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// True iff `rel` is a colocated metadata asset: `app/.../<base>.<ext>`
/// where `base` is one of the Next-style file conventions
/// {opengraph-image, twitter-image, icon, apple-icon, favicon} and `ext`
/// is an image type. Rejects absolute paths, `..`/`.` segments,
/// backslashes, and any other basename — so `/_pylon/og` can only ever
/// serve those conventional files and never an arbitrary source file.
///
/// `svg`/`ico` are allowed (favicons are commonly SVG/ICO). Unlike the
/// `<Image>` optimizer — which rejects SVG because it proxies remote /
/// user-supplied images — this endpoint only serves the developer's OWN
/// colocated convention files (first-party app source), so an inline-
/// script SVG is no more dangerous than the app's own JS.
fn is_valid_colocated_asset_rel(rel: &str) -> bool {
    if rel.is_empty() || rel.starts_with('/') || rel.starts_with('\\') {
        return false;
    }
    let segs: Vec<&str> = rel.split('/').collect();
    if segs.first() != Some(&"app") {
        return false;
    }
    for s in &segs {
        if s.is_empty() || *s == "." || *s == ".." || s.contains('\\') {
            return false;
        }
    }
    let file = match segs.last() {
        Some(f) => *f,
        None => return false,
    };
    let (base, ext) = match file.rsplit_once('.') {
        Some(x) => x,
        None => return false,
    };
    let base_ok = matches!(
        base,
        "opengraph-image" | "twitter-image" | "icon" | "apple-icon" | "favicon"
    );
    let ext_ok = matches!(
        ext.to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "avif" | "svg" | "ico"
    );
    base_ok && ext_ok
}

/// Serve a colocated `opengraph-image.*` / `twitter-image.*` file for the
/// SSR file convention. Returns a clean 404 (not an SPA fallthrough) on a
/// missing / invalid `src` so a broken `<img>` fails cleanly.
fn serve_og_image(request: Request, url: &str, cors_origin: &str) -> Result<(), Request> {
    let four_oh_four = |req: Request| -> Result<(), Request> {
        let resp = Response::from_data(Vec::new())
            .with_status_code(404)
            .with_header(
                Header::from_bytes(
                    "Access-Control-Allow-Origin",
                    cors_origin.as_bytes().to_vec(),
                )
                .unwrap(),
            );
        let _ = req.respond(resp);
        Ok(())
    };

    let rel = url
        .split("src=")
        .nth(1)
        .and_then(|s| s.split('&').next())
        .map(pct_decode)
        .unwrap_or_default();
    if !is_valid_colocated_asset_rel(&rel) {
        return four_oh_four(request);
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    // Canonicalize both the app root and the candidate, then prefix-check.
    // The rel is already traversal-free, but a symlink under app/ could
    // still escape — this closes that.
    let (app_root, canon) = match (
        cwd.join("app").canonicalize(),
        cwd.join(&rel).canonicalize(),
    ) {
        (Ok(a), Ok(c)) => (a, c),
        _ => return four_oh_four(request),
    };
    if !canon.starts_with(&app_root) || !canon.is_file() {
        return four_oh_four(request);
    }
    let bytes = match std::fs::read(&canon) {
        Ok(b) => b,
        Err(_) => return four_oh_four(request),
    };
    let response = Response::from_data(bytes)
        .with_status_code(200)
        .with_header(Header::from_bytes("Content-Type", content_type_for(&canon)).unwrap())
        .with_header(
            Header::from_bytes(
                "Access-Control-Allow-Origin",
                cors_origin.as_bytes().to_vec(),
            )
            .unwrap(),
        )
        // The og URL carries a `&v=<mtime>` cache-buster, so a 1h public
        // cache is safe — a changed image gets a new `v` and re-fetches.
        .with_header(Header::from_bytes("Cache-Control", "public, max-age=3600").unwrap());
    let _ = request.respond(response);
    Ok(())
}

/// True when this runtime was launched by `pylon dev` (which exports
/// `PYLON_DEV_MODE=1`). Gates dev-only surfaces — currently the
/// live-reload SSE endpoint — so they can never exist in production.
fn is_dev_mode() -> bool {
    std::env::var("PYLON_DEV_MODE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Per-process boot id: stable for the life of this process, distinct on
/// the next one. `pylon dev` re-execs a fresh process on every file edit,
/// so a changed id is exactly the "the server just restarted" signal the
/// live-reload client watches for.
fn dev_boot_id() -> &'static str {
    static BOOT_ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    BOOT_ID.get_or_init(|| {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        format!("{}-{}", std::process::id(), nanos)
    })
}

/// Dev-only Server-Sent-Events endpoint backing browser live-reload. Holds
/// the connection open with a 1s heartbeat and emits one `hello` event
/// carrying this process's boot id. On the next `pylon dev` restart the
/// process dies, the page's EventSource reconnects to the fresh process,
/// reads a different boot id, and reloads the tab. The matching client
/// script is injected into every SSR page in dev (see ssr-runtime.ts).
fn serve_dev_live_reload(request: Request, cors_origin: &str) -> Result<(), Request> {
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let streaming_body = crate::server::StreamingBody::new(rx);

    // `retry:` sets the client's reconnect delay; `hello` carries the id.
    let hello = format!("retry: 500\nevent: hello\ndata: {}\n\n", dev_boot_id());
    if tx.send(hello.into_bytes()).is_err() {
        return Ok(());
    }

    // Heartbeat keeps proxies/connections alive and lets us notice when the
    // client has gone (send fails → StreamingBody's rx was dropped) so the
    // thread exits instead of leaking.
    std::thread::Builder::new()
        .name("pylon-dev-live-reload".into())
        .spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            if tx.send(b": ping\n\n".to_vec()).is_err() {
                return;
            }
        })
        .ok();

    let response = Response::new(
        tiny_http::StatusCode(200),
        vec![
            Header::from_bytes("Content-Type", "text/event-stream").unwrap(),
            Header::from_bytes("Cache-Control", "no-cache").unwrap(),
            Header::from_bytes("Connection", "keep-alive").unwrap(),
            // Disable proxy buffering so events flush immediately.
            Header::from_bytes("X-Accel-Buffering", "no").unwrap(),
            Header::from_bytes(
                "Access-Control-Allow-Origin",
                cors_origin.as_bytes().to_vec(),
            )
            .unwrap(),
        ],
        streaming_body,
        None,
        None,
    );
    let _ = request.respond(response);
    Ok(())
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

    // Hydration build assets. SSR pages emit per-route
    // `<script type="module" src="/_pylon/build/<file>">` tags
    // and `<link rel="modulepreload" href="/_pylon/build/chunks/...">`,
    // which all route through this handler. First request triggers
    // the Bun.build via the bundle_client RPC; subsequent requests
    // stream files off disk from the cached outdir.
    let path_only = url.split('?').next().unwrap_or(&url);
    if path_only.starts_with("/_pylon/build/") && cfg.fn_ops.is_some() {
        return serve_pylon_client_bundle(cfg, request, path_only, cors_origin);
    }

    // Optimized image endpoint. `<Image>` renders an `<img>` whose
    // src points here; the handler resizes + re-encodes the source
    // on demand and caches the result under `<cwd>/.pylon/.cache/
    // images/<hash>.<ext>`.
    if path_only == "/_pylon/image" {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let pylon_dir = cwd.join(".pylon");
        let _ = std::fs::create_dir_all(&pylon_dir);
        crate::image_optim::serve(request, &pylon_dir, cfg.dir.as_deref(), cors_origin);
        return Ok(());
    }

    // Social-card image file convention (Next-style `opengraph-image.png`
    // / `twitter-image.png` colocated with a `page.tsx`). SSR pages
    // auto-emit `<meta property="og:image" content="<origin>/_pylon/og?
    // src=app/.../opengraph-image.png">` (see ssr-runtime.ts
    // `applyAutoSocialImages`); this serves the colocated file off disk.
    // Strict allowlist — only `opengraph-image` / `twitter-image`
    // basenames with an image extension, under `app/`, canonicalized so
    // the endpoint can't be turned into an arbitrary-file read.
    if path_only == "/_pylon/og" {
        return serve_og_image(request, &url, cors_origin);
    }

    // Dev-only browser live-reload signal. `pylon dev` injects a tiny
    // EventSource client into every SSR page; it subscribes here and
    // reloads the tab when this process's boot id changes — which happens
    // on every restart (edit a page / component / globals.css → the dev
    // server re-execs a fresh process). 404 outside dev so prod never
    // exposes the endpoint.
    if path_only == "/_pylon/dev/live" {
        if is_dev_mode() {
            return serve_dev_live_reload(request, cors_origin);
        }
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
            return serve_via_ssr_rpc(cfg, matched, request, cors_origin, None);
        }
        // No page matched. If the app defines a `not-found.tsx` boundary
        // and this looks like a document navigation (not a static asset),
        // render the boundary at HTTP 404 instead of silently SPA-falling-
        // back to the home shell at 200. Asset 404s and apps without a
        // not-found boundary keep the existing fallthrough (proxy / disk).
        if looks_like_document_nav(&url) {
            if let Some(nf) = find_not_found_route(&url, &cfg.ssr_routes) {
                tracing::debug!(url = %url, boundary = %nf.path, "SSR not-found");
                let matched = SsrMatch {
                    route: nf.clone(),
                    params: std::collections::HashMap::new(),
                };
                return serve_via_ssr_rpc(cfg, matched, request, cors_origin, Some(404));
            }
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
        // Boundary routes (not-found / error) are never navigable — they're
        // consulted by the host for unmatched-URL 404s + render-failure
        // 500s, not matched as page URLs.
        if matches!(r.kind.as_deref(), Some("not-found") | Some("error")) {
            continue;
        }
        let route_segs: Vec<&str> = r.path.split('/').filter(|s| !s.is_empty()).collect();

        // Catch-all routes encode their last segment as `*name` (required,
        // ≥1 segment) or `*?name` (optional, ≥0). It greedily consumes the
        // remaining path; the matched sub-path is joined with `/` into one
        // param value (split it in the page if you want segments). The
        // marker is only honored on the LAST segment — discoverAppRoutes
        // never emits one elsewhere, and a stray `*` mid-path falls through
        // to the literal comparison below (no match).
        let catch_all = route_segs
            .last()
            .and_then(|s| s.strip_prefix('*'))
            .map(|rest| match rest.strip_prefix('?') {
                Some(name) => (name, true),  // *?name → optional
                None => (rest, false),       // *name  → required
            });

        if let Some((ca_name, optional)) = catch_all {
            let prefix = &route_segs[..route_segs.len() - 1];
            // Required catch-all needs at least one segment to consume.
            let min_len = prefix.len() + if optional { 0 } else { 1 };
            if url_segs.len() < min_len {
                continue;
            }
            let mut params = std::collections::HashMap::new();
            let mut matched = true;
            for (rs, us) in prefix.iter().zip(url_segs.iter()) {
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
                // Join the remaining segments as the catch-all value (empty
                // string when an optional catch-all matched zero segments).
                let rest = url_segs[prefix.len()..].join("/");
                params.insert(ca_name.to_string(), rest);
                return Some(SsrMatch {
                    route: r.clone(),
                    params,
                });
            }
            continue;
        }

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

/// Find the nearest `not-found.tsx` boundary covering an unmatched URL.
///
/// Among `kind == "not-found"` routes, pick the one whose segment path is
/// the longest prefix of the request path — so `app/blog/not-found.tsx`
/// (path `/blog`) covers `/blog/anything` and the root `app/not-found.tsx`
/// (path `/`) covers everything else. Returns `None` when the app defines
/// no not-found boundary, in which case the host keeps its default
/// behaviour (SPA fallback / API 404).
pub fn find_not_found_route<'a>(
    url: &str,
    routes: &'a [pylon_kernel::ManifestRoute],
) -> Option<&'a pylon_kernel::ManifestRoute> {
    let path = url.split('?').next().unwrap_or(url);
    let url_segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let mut best: Option<(&pylon_kernel::ManifestRoute, usize)> = None;
    for r in routes {
        if r.mode != "ssr" || r.kind.as_deref() != Some("not-found") {
            continue;
        }
        let route_segs: Vec<&str> = r.path.split('/').filter(|s| !s.is_empty()).collect();
        // A boundary at `/a/b` covers `/a/b/...`; its segments must be a
        // prefix of the URL's. The root boundary (no segments) covers all.
        if route_segs.len() > url_segs.len() {
            continue;
        }
        let is_prefix = route_segs
            .iter()
            .zip(url_segs.iter())
            .all(|(rs, us)| rs == us);
        if is_prefix {
            let depth = route_segs.len();
            if best.map_or(true, |(_, d)| depth > d) {
                best = Some((r, depth));
            }
        }
    }
    best.map(|(r, _)| r)
}

/// True when `url`'s last path segment carries no file extension, i.e. it
/// looks like a document navigation (`/blog/missing`) rather than a static
/// asset request (`/favicon.ico`, `/assets/app.css`). Used to gate the
/// not-found render so a 404'd asset still falls through to disk instead of
/// being answered with an HTML boundary.
fn looks_like_document_nav(url: &str) -> bool {
    let path = url.split('?').next().unwrap_or(url);
    let last = path.rsplit('/').next().unwrap_or("");
    !last.contains('.')
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
    initial_status: Option<u16>,
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

    // Resolve auth from the request's session cookie if a
    // SessionStore + CookieConfig are wired (the standard case for
    // pylon dev / pylon start). Without them, fall back to anonymous
    // AuthInfo — matches Phase 1 behavior.
    let auth = resolve_request_auth(cfg, &cookies_map);

    // Stream render output via tiny_http chunked transfer encoding.
    // The render thread writes each base64-decoded chunk into `body_tx`;
    // the response writer reads from `body_rx` through `StreamingBody`.
    //
    // Two channels coordinate the response head with the body:
    //   - `rs_tx`   carries the ONE `response_start` (status + headers)
    //               the page emits BEFORE the first body byte. The main
    //               thread blocks on `rs_rx` so the HTTP head is built
    //               from the page's chosen status/headers/cookies (a
    //               page that calls `response.setStatus`/`redirect`/
    //               `notFound`/`setCookie`), not a hardcoded 200.
    //   - `body_tx` carries decoded body chunks (bounded for backpressure).
    // `sync_channel` buffers, so even a tiny page that finishes the whole
    // render before the main thread reads still delivers both the head
    // and the body. If the render fails BEFORE emitting response_start
    // (e.g. a bad import), `rs_tx` drops without a send → `rs_rx.recv()`
    // returns Err → we serve a structured 500 instead of a dropped
    // connection.
    let (rs_tx, rs_rx) =
        std::sync::mpsc::sync_channel::<(u16, std::collections::HashMap<String, String>)>(1);
    let (body_tx, body_rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(64);
    let streaming_body = crate::server::StreamingBody::new(body_rx);
    // Carries the render error (code, message) from the render thread to the
    // main thread for the dev error overlay. Separate from rs_tx because the
    // failure path is exactly "rs_tx dropped without a response_start" — we
    // can't piggyback the detail on the channel that just closed. recv() on
    // err_rx blocks until the render thread reaches the send (error) or drops
    // err_tx (success), so it's race-free.
    let (err_tx, err_rx) = std::sync::mpsc::sync_channel::<(String, String)>(1);

    let fn_ops_for_render = std::sync::Arc::clone(&fn_ops);
    let component_owned = component.clone();
    let layouts = matched.route.layouts.clone();
    let route_path_owned = matched.route.path.clone();
    let path_only_owned = path_only.clone();
    let render_thread = std::thread::Builder::new()
        .name("pylon-ssr-render".into())
        .stack_size(512 * 1024)
        .spawn(move || {
            let on_response_start: pylon_functions::runner::ResponseStartCallback = Box::new(
                move |status: u16, headers: std::collections::HashMap<String, String>| {
                    // Capacity-1, fires at most once — never blocks.
                    let _ = rs_tx.send((status, headers));
                },
            );
            let on_chunk: pylon_functions::runner::ByteStreamCallback =
                Box::new(move |bytes: &[u8]| {
                    // sync_channel(64) bounds memory; a full channel
                    // blocks the render thread (= backpressure from a
                    // slow client). Send failure (body_rx dropped) means
                    // the client disconnected — drop quietly.
                    let _ = body_tx.send(bytes.to_vec());
                });
            let render_result = fn_ops_for_render.render_route(
                &component_owned,
                layouts,
                &route_path_owned,
                &path_only_owned,
                params_json,
                search_params_json,
                headers_map,
                cookies_map,
                auth,
                initial_status,
                Some(on_response_start),
                on_chunk,
            );
            if let Err(e) = render_result {
                tracing::error!(
                    code = %e.code,
                    message = %e.message,
                    "SSR render failed"
                );
                // Hand the detail to the main thread for the dev overlay. If
                // this fired BEFORE response_start, rs_tx drops with no value
                // → the main thread reads err_rx and serves a 500 (a styled
                // overlay in dev). If AFTER (head + some body already went
                // out), the body channel closes → partial HTML; React's
                // onError already logged and this send is simply unread.
                let _ = err_tx.send((e.code.clone(), e.message.clone()));
            }
            // Both senders drop here → channels close → StreamingBody
            // hits EOF and tiny_http finalizes the chunked transfer.
        });

    if let Err(e) = render_thread {
        // Couldn't spawn the render thread. Inline 500.
        tracing::error!(error = ?e, "failed to spawn SSR render thread");
        let detail = is_dev_mode().then(|| {
            (
                "SSR_RENDER_THREAD_SPAWN_FAILED".to_string(),
                format!("could not spawn the SSR render thread: {e}"),
            )
        });
        let _ = request.respond(ssr_render_error_response(detail, cors_origin));
        return Ok(());
    }

    // Block until the page emits response_start (status + headers), or
    // the render thread dies first (Err = failed before any head). The
    // page controls the status here: setStatus(201), redirect() → 3xx +
    // Location, notFound() → 404. Set-Cookie lines arrive newline-joined
    // in the `set-cookie` header value and are split back out below.
    let (status, page_headers) = match rs_rx.recv() {
        Ok(v) => v,
        Err(_) => {
            // Render produced no response_start → an early failure (bad
            // import, a throw in the shell render with no error.tsx boundary).
            // In dev, surface the error + stack in an overlay; in prod, a
            // generic 500 (the detail stays in logs). err_rx.recv() blocks
            // until the render thread sends the detail or finishes.
            let detail = if is_dev_mode() {
                err_rx.recv().ok()
            } else {
                None
            };
            let _ = request.respond(ssr_render_error_response(detail, cors_origin));
            return Ok(());
        }
    };

    let response = Response::new(
        tiny_http::StatusCode(status),
        build_ssr_response_headers(&page_headers, cors_origin),
        streaming_body,
        None, // content-length unknown → tiny_http uses chunked transfer
        None,
    );
    // request.respond drains StreamingBody chunk-by-chunk as HTTP
    // chunked-transfer frames; returns at EOF (render thread done).
    let _ = request.respond(response);
    Ok(())
}

/// Build the response header set for an SSR reply from the page's
/// `response_start` headers, layering in defaults + CORS and splitting
/// the newline-joined `set-cookie` value into one `Set-Cookie` header
/// per cookie.
///
/// `Header::from_bytes` validates names/values, so a page that tries to
/// smuggle a CRLF (`response.setHeader("x", "a\r\nevil: 1")`) gets that
/// header dropped rather than injected — header-injection-safe by
/// construction. `content-type` and `cache-control` defaults only apply
/// when the page didn't set them.
fn build_ssr_response_headers(
    page_headers: &std::collections::HashMap<String, String>,
    cors_origin: &str,
) -> Vec<Header> {
    let mut out: Vec<Header> = Vec::new();
    let mut saw_content_type = false;
    let mut saw_cache_control = false;
    let mut saw_set_cookie = false;

    // A header VALUE carrying CR/LF/NUL could smuggle extra headers (HTTP
    // response splitting). Drop it rather than trust the header library.
    fn header_value_is_safe(v: &str) -> bool {
        !v.bytes().any(|b| matches!(b, b'\r' | b'\n' | 0))
    }
    // A header NAME must be a separator-free token. `tiny_http`'s
    // `Header::from_bytes` does NOT reject CR/LF/NUL in the name, so
    // without this a page calling `response.setHeader("x\r\nevil: 1", v)`
    // could inject a header. Reject control chars + the separators that
    // would terminate the name (`:`, space, tab).
    fn header_name_is_safe(n: &str) -> bool {
        !n.is_empty()
            && !n
                .bytes()
                .any(|b| matches!(b, b'\r' | b'\n' | 0 | b' ' | b'\t' | b':'))
    }

    for (name, value) in page_headers {
        let lname = name.to_ascii_lowercase();
        if lname == "set-cookie" {
            // One Set-Cookie header per cookie. Newline is the join
            // delimiter (forbidden inside a real cookie value), so a
            // split piece that still contains a CR is an injection
            // attempt and gets dropped.
            for line in value.split('\n') {
                let line = line.trim();
                if line.is_empty() || !header_value_is_safe(line) {
                    continue;
                }
                if let Ok(h) = Header::from_bytes("Set-Cookie", line.as_bytes()) {
                    out.push(h);
                    saw_set_cookie = true;
                }
            }
            continue;
        }
        if !header_value_is_safe(value) || !header_name_is_safe(name) {
            continue;
        }
        if let Ok(h) = Header::from_bytes(name.as_bytes(), value.as_bytes()) {
            if lname == "content-type" {
                saw_content_type = true;
            }
            if lname == "cache-control" {
                saw_cache_control = true;
            }
            out.push(h);
        }
    }

    if !saw_content_type {
        out.push(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap());
    }
    if !saw_cache_control {
        // A cookie-setting SSR response is personalized — a shared cache (e.g.
        // Cloudflare) must NEVER store it and replay it to another user, so it
        // gets `no-store`. Otherwise `no-cache` (don't serve stale without
        // revalidation; anonymous pages opt into edge caching explicitly).
        let cc = if saw_set_cookie { "no-store" } else { "no-cache" };
        out.push(Header::from_bytes("Cache-Control", cc).unwrap());
    }
    if let Ok(h) = Header::from_bytes("Access-Control-Allow-Origin", cors_origin.as_bytes()) {
        out.push(h);
    }
    // Baseline security headers on the user-facing SSR HTML. Previously only
    // Studio carried these, so app pages were clickjackable (no X-Frame-Options)
    // and MIME-sniffable. X-Frame-Options is SAMEORIGIN (not Studio's DENY) so
    // an app can still embed its OWN pages in an iframe; cross-origin framing —
    // the clickjacking vector — is blocked. A Content-Security-Policy is left to
    // the app (via response.setHeader) because a default `default-src 'self'`
    // would break any page loading external fonts/images/analytics. Each is a
    // DEFAULT: skip it when the page already set that header (an embeddable
    // widget can override X-Frame-Options, a page can ship its own CSP), so we
    // never emit a conflicting duplicate.
    let has = |out: &[Header], name: &str| {
        out.iter()
            .any(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
    };
    for (name, value) in [
        ("X-Content-Type-Options", "nosniff"),
        ("X-Frame-Options", "SAMEORIGIN"),
        ("Referrer-Policy", "strict-origin-when-cross-origin"),
        (
            "Permissions-Policy",
            "accelerometer=(), camera=(), geolocation=(), gyroscope=(), microphone=(), payment=(), usb=()",
        ),
    ] {
        if !has(&out, name) {
            if let Ok(h) = Header::from_bytes(name, value.as_bytes()) {
                out.push(h);
            }
        }
    }
    out
}

/// A minimal, structured SSR error response (used when the render can't
/// even start, or fails before emitting `response_start`).
fn ssr_error_response(status: u16, cors_origin: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    let body = format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>{status}</title></head><body><h1>{status}</h1><p>The server could not render this page.</p></body></html>"
    )
    .into_bytes();
    let mut resp = Response::from_data(body)
        .with_status_code(status)
        .with_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap());
    if let Ok(h) = Header::from_bytes("Access-Control-Allow-Origin", cors_origin.as_bytes()) {
        resp = resp.with_header(h);
    }
    resp
}

/// The 500 served when an SSR render fails before it can emit any response —
/// e.g. a bad import or a throw in the shell render with no `error.tsx`
/// boundary to catch it. In dev (`detail` is `Some((code, message))`, only
/// populated when `is_dev_mode()`), paint a readable error overlay with the
/// code + message/stack so the developer sees the actual failure instead of
/// an opaque page. In prod (`detail` is `None`), fall back to the generic
/// minimal 500 — the detail stays in the server logs, never on the wire.
/// The dev error-overlay HTML body. Split out (pure) so it's unit-testable
/// without constructing a tiny_http Response. `code` + `message` are both
/// HTML-escaped here — `message` is the JS error stack in dev, so it can
/// contain `<`, `>`, quotes, and newlines (preserved by `white-space:
/// pre-wrap`).
fn ssr_dev_error_overlay_html(code: &str, message: &str) -> String {
    format!(
        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<title>SSR error — {code}</title>\
<style>\
:root{{color-scheme:dark light}}\
body{{margin:0;font:14px/1.6 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;\
background:#0b0d12;color:#e6e8eb}}\
.wrap{{max-width:920px;margin:0 auto;padding:48px 24px}}\
.badge{{display:inline-block;background:#7f1d1d;color:#fecaca;border-radius:6px;\
padding:2px 10px;font-size:12px;font-weight:600;letter-spacing:.02em}}\
h1{{font-size:20px;font-weight:600;margin:16px 0 4px}}\
.sub{{color:#9aa3ad;margin:0 0 24px;font-size:13px}}\
pre{{background:#11141b;border:1px solid #1f2430;border-radius:10px;\
padding:16px 18px;overflow:auto;white-space:pre-wrap;word-break:break-word;\
color:#f2c0c0}}\
.hint{{margin-top:20px;color:#9aa3ad;font-size:13px}}\
.hint code{{background:#11141b;border:1px solid #1f2430;border-radius:5px;padding:1px 6px}}\
</style></head><body><div class=\"wrap\">\
<span class=\"badge\">SSR render error</span>\
<h1>{code}</h1>\
<p class=\"sub\">This page threw while rendering on the server, and there's no \
<code>error.tsx</code> boundary above it to catch it.</p>\
<pre>{message}</pre>\
<p class=\"hint\">Add an <code>error.tsx</code> in this route's folder (or an \
ancestor) to render a friendly fallback. This overlay only appears with \
<code>PYLON_DEV_MODE</code> set — production shows a generic 500.</p>\
</div></body></html>",
        code = html_escape(code),
        message = html_escape(message),
    )
}

fn ssr_render_error_response(
    detail: Option<(String, String)>,
    cors_origin: &str,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let (code, message) = match detail {
        Some(d) => d,
        None => return ssr_error_response(500, cors_origin),
    };
    let body = ssr_dev_error_overlay_html(&code, &message).into_bytes();
    let mut resp = Response::from_data(body)
        .with_status_code(500u16)
        .with_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap())
        // Never let an error overlay be cached by a shared cache.
        .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap());
    if let Ok(h) = Header::from_bytes("Access-Control-Allow-Origin", cors_origin.as_bytes()) {
        resp = resp.with_header(h);
    }
    resp
}

/// Memoized bundle outdir. The first request triggers `Bun.build`
/// via the `bundle_client` RPC; subsequent requests stream files
/// out of the cached directory. Phase 1.5e: cached for the process
/// lifetime — file-watcher invalidation (Phase 1.5f) will clear
/// this on app/ changes.
fn cached_bundle_outdir() -> &'static std::sync::Mutex<Option<std::path::PathBuf>> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<Option<std::path::PathBuf>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(None))
}

/// Pick a content-type for a build asset based on its extension.
/// Phase 1.5e emits `.js` entries + `.js` chunks; `.json` for the
/// manifest; `.css` reserved for 1.5f.
fn bundle_content_type_for(name: &str) -> &'static str {
    if name.ends_with(".js") || name.ends_with(".mjs") {
        "application/javascript; charset=utf-8"
    } else if name.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if name.ends_with(".json") {
        "application/json; charset=utf-8"
    } else if name.ends_with(".map") {
        "application/json; charset=utf-8"
    } else {
        "application/octet-stream"
    }
}

/// Serve any file under the bundle outdir. Calls `bundle_client`
/// on the function runner on first hit, caches the outdir, then
/// reads files relative to it. Path traversal is rejected hard.
///
/// Hash-named entries (`*-<hash>.js`) get `Cache-Control: public,
/// max-age=31536000, immutable` — they're content-addressed so
/// safe forever. The manifest (`manifest.json`) gets `no-cache`
/// since it's the only mutable URL.
///
/// Returns a 500 (or falls through to API routing on
/// HYDRATION_NOT_IMPLEMENTED) if the bundler fails. The SSR'd HTML
/// stays renderable without hydration — pages just don't become
/// interactive.
fn serve_pylon_client_bundle(
    cfg: &FrontendConfig,
    request: Request,
    request_path: &str,
    cors_origin: &str,
) -> Result<(), Request> {
    let fn_ops = match cfg.fn_ops.as_ref() {
        Some(f) => f.clone(),
        None => return Err(request),
    };

    // Acquire the outdir — build on first miss.
    let mut cache = cached_bundle_outdir().lock().unwrap();
    if cache.is_none() {
        match fn_ops.bundle_client() {
            Ok(paths) => {
                tracing::info!(
                    outdir = %paths.outdir,
                    manifest = %paths.manifest_path,
                    "SSR client bundle built"
                );
                *cache = Some(std::path::PathBuf::from(paths.outdir));
            }
            Err(e) => {
                drop(cache);
                tracing::error!(
                    code = %e.code,
                    message = %e.message,
                    "SSR client bundle failed"
                );
                let body = format!(
                    "// Pylon client bundle failed to build:\n// {}: {}\n",
                    e.code, e.message
                );
                let response = Response::from_data(body.into_bytes())
                    .with_status_code(500u16)
                    .with_header(
                        Header::from_bytes("Content-Type", "application/javascript; charset=utf-8")
                            .unwrap(),
                    )
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
        }
    }
    let outdir = cache.clone().unwrap();
    drop(cache);

    // Extract the path component after `/_pylon/build/`. Anything
    // containing `..`, NUL, or backslash gets rejected — we don't
    // want a hostile request asking for `../../etc/passwd`.
    let suffix = request_path.strip_prefix("/_pylon/build/").unwrap_or("");
    if suffix.is_empty()
        || suffix.contains("..")
        || suffix.contains('\0')
        || suffix.contains('\\')
        || suffix.starts_with('/')
    {
        let response = Response::from_data(b"// invalid bundle path\n".to_vec())
            .with_status_code(400u16)
            .with_header(
                Header::from_bytes("Content-Type", "application/javascript; charset=utf-8")
                    .unwrap(),
            );
        let _ = request.respond(response);
        return Ok(());
    }

    let file_path = outdir.join(suffix);
    // Belt-and-suspenders: canonicalize to make sure the resolved
    // path still lives under outdir. If outdir itself can't be
    // canonicalized (rare — disk vanished between build + read)
    // bail out.
    let outdir_canon = match outdir.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            // Invalidate cache so the next request rebuilds.
            *cached_bundle_outdir().lock().unwrap() = None;
            let body = b"// bundle outdir missing\n".to_vec();
            let response = Response::from_data(body)
                .with_status_code(500u16)
                .with_header(
                    Header::from_bytes("Content-Type", "application/javascript; charset=utf-8")
                        .unwrap(),
                );
            let _ = request.respond(response);
            return Ok(());
        }
    };
    if let Ok(canon) = file_path.canonicalize() {
        if !canon.starts_with(&outdir_canon) {
            let response = Response::from_data(b"// path escapes outdir\n".to_vec())
                .with_status_code(400u16)
                .with_header(
                    Header::from_bytes("Content-Type", "application/javascript; charset=utf-8")
                        .unwrap(),
                );
            let _ = request.respond(response);
            return Ok(());
        }
    }

    let bytes = match std::fs::read(&file_path) {
        Ok(b) => b,
        Err(_) => {
            // 404 — no such bundle file. Don't invalidate the
            // outdir cache; just this one file is missing.
            let body = format!("// not found: {suffix}\n");
            let response = Response::from_data(body.into_bytes())
                .with_status_code(404u16)
                .with_header(
                    Header::from_bytes("Content-Type", "application/javascript; charset=utf-8")
                        .unwrap(),
                );
            let _ = request.respond(response);
            return Ok(());
        }
    };

    // Hash-named files are content-addressed and safe to cache
    // forever. `manifest.json` is the only mutable URL: it gets
    // no-cache. Heuristic: files containing `-` followed by 8+
    // hex chars before the extension are hash-named.
    let cache_control = if suffix == "manifest.json" {
        "no-cache"
    } else if is_hashed_name(suffix) {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };

    let response = Response::from_data(bytes)
        .with_status_code(200u16)
        .with_header(Header::from_bytes("Content-Type", bundle_content_type_for(suffix)).unwrap())
        .with_header(Header::from_bytes("Cache-Control", cache_control).unwrap())
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

/// Heuristic: does this filename look hash-stamped? Matches
/// `<name>-<hash>.<ext>` where `<hash>` is at least 8 chars long
/// and uses only the base36 alphabet (Bun's hashing emits lowercase
/// alphanumerics — 0-9, a-z). Used to decide whether to send a long
/// `Cache-Control: immutable`. False negatives (no immutable) are
/// harmless; false positives would cache a mutable file, so we err
/// strict on the alphabet.
fn is_hashed_name(name: &str) -> bool {
    let base = name.rsplit_once('/').map(|(_, b)| b).unwrap_or(name);
    let stem = match base.rsplit_once('.') {
        Some((s, _)) => s,
        None => return false,
    };
    let hash = match stem.rsplit_once('-') {
        Some((_, h)) => h,
        None => return false,
    };
    hash.len() >= 8
        && hash
            .chars()
            .all(|c| c.is_ascii_digit() || c.is_ascii_lowercase())
}

/// Build the AuthInfo for an SSR render by looking up the request's
/// session cookie in the configured SessionStore. Returns anonymous
/// AuthInfo when:
///   - no SessionStore is wired (test stubs, in-memory smoke tests)
///   - no session cookie is present on the request
///   - the cookie token doesn't resolve to a live session
///
/// Only covers session cookies — bearer tokens / API keys / JWTs
/// don't apply to SSR HTML routes (those auth modes target
/// programmatic / API consumers, not browser-rendered pages).
fn resolve_request_auth(
    cfg: &FrontendConfig,
    cookies: &std::collections::HashMap<String, String>,
) -> pylon_functions::protocol::AuthInfo {
    let anonymous = pylon_functions::protocol::AuthInfo {
        user_id: None,
        is_admin: false,
        tenant_id: None,
        roles: vec![],
    };
    let (Some(store), Some(cookie_cfg)) = (cfg.session_store.as_ref(), cfg.cookie_config.as_ref())
    else {
        return anonymous;
    };
    let token = cookies.get(&cookie_cfg.name);
    let ctx = store.resolve(token.map(|s| s.as_str()));
    pylon_functions::protocol::AuthInfo {
        user_id: ctx.user_id.clone(),
        is_admin: ctx.is_admin,
        tenant_id: ctx.tenant_id.clone(),
        roles: ctx.roles.clone(),
    }
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
    fn social_image_rel_allowlist() {
        // Valid colocated social-card images.
        assert!(is_valid_colocated_asset_rel("app/opengraph-image.png"));
        assert!(is_valid_colocated_asset_rel("app/blog/opengraph-image.jpg"));
        assert!(is_valid_colocated_asset_rel(
            "app/(marketing)/twitter-image.webp"
        ));
        assert!(is_valid_colocated_asset_rel(
            "app/x/[slug]/opengraph-image.PNG"
        ));
        // Wrong basename — must never serve arbitrary source files.
        assert!(!is_valid_colocated_asset_rel("app/page.tsx"));
        assert!(!is_valid_colocated_asset_rel("app/secret.png"));
        assert!(!is_valid_colocated_asset_rel("app/blog/cover.png"));
        // Icon conventions (svg/ico allowed here).
        assert!(is_valid_colocated_asset_rel("app/icon.png"));
        assert!(is_valid_colocated_asset_rel("app/icon.svg"));
        assert!(is_valid_colocated_asset_rel("app/favicon.ico"));
        assert!(is_valid_colocated_asset_rel("app/apple-icon.png"));
        // Wrong root / extension.
        assert!(!is_valid_colocated_asset_rel("public/opengraph-image.png"));
        assert!(!is_valid_colocated_asset_rel("app/opengraph-image.ts"));
        assert!(!is_valid_colocated_asset_rel("app/icon.tsx"));
        assert!(!is_valid_colocated_asset_rel("app/logo.svg"));
        // Traversal / absolute / malformed.
        assert!(!is_valid_colocated_asset_rel(
            "app/../secret/opengraph-image.png"
        ));
        assert!(!is_valid_colocated_asset_rel("/app/opengraph-image.png"));
        assert!(!is_valid_colocated_asset_rel("../app/opengraph-image.png"));
        assert!(!is_valid_colocated_asset_rel("app/./opengraph-image.png"));
        assert!(!is_valid_colocated_asset_rel(""));
        assert!(!is_valid_colocated_asset_rel("opengraph-image.png"));
    }

    #[test]
    fn pct_decode_handles_encoded_slashes() {
        assert_eq!(
            pct_decode("app%2Fblog%2Fopengraph-image.png"),
            "app/blog/opengraph-image.png"
        );
        assert_eq!(pct_decode("app/no-encoding.png"), "app/no-encoding.png");
        // Malformed escape is left literal rather than panicking.
        assert_eq!(pct_decode("a%zz"), "a%zz");
    }

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

    #[test]
    fn ssr_routes_activate_dispatcher_without_a_dist() {
        // A native full-stack app's app/**/page.tsx files produce
        // mode:"ssr" routes but NO web/dist (per-route chunks live under
        // .pylon/client-build, built lazily on first render). Pre-fix
        // is_active() saw no dir, no proxy, and (for a fresh app) no
        // build kicked off → returned false, so every page fell through
        // to the API router as a 404. The SSR-routes clause fixes it.
        let saved_dir = std::env::var("PYLON_FRONTEND_DIR").ok();
        let saved_proxy = std::env::var("PYLON_FRONTEND_DEV_PROXY").ok();
        std::env::remove_var("PYLON_FRONTEND_DIR");
        std::env::remove_var("PYLON_FRONTEND_DEV_PROXY");

        let tmp = TempDir::new().unwrap();
        // No web/dist here → dir resolves to None.
        let cfg = FrontendConfig::from_env(tmp.path()).with_ssr(
            std::sync::Arc::new(vec![pylon_kernel::ManifestRoute {
                path: "/".into(),
                mode: "ssr".into(),
                ..Default::default()
            }]),
            None,
        );
        assert!(cfg.dir.is_none(), "a fresh native-SSR app has no web/dist");
        assert!(
            cfg.is_active(),
            "an app with mode:ssr routes must activate the frontend dispatcher"
        );

        if let Some(v) = saved_dir {
            std::env::set_var("PYLON_FRONTEND_DIR", v);
        }
        if let Some(v) = saved_proxy {
            std::env::set_var("PYLON_FRONTEND_DEV_PROXY", v);
        }
    }

    // --- SSR boundary routing (match_ssr_route / find_not_found_route) ---

    fn route(path: &str, kind: Option<&str>) -> pylon_kernel::ManifestRoute {
        pylon_kernel::ManifestRoute {
            path: path.into(),
            mode: "ssr".into(),
            component: Some(format!("app{path}").replace("//", "/")),
            kind: kind.map(|k| k.into()),
            ..Default::default()
        }
    }

    #[test]
    fn match_ssr_route_skips_boundary_kinds() {
        let routes = vec![
            route("/", None),
            route("/", Some("not-found")),
            route("/", Some("error")),
        ];
        // A navigable URL matches the page, never a boundary.
        let m = match_ssr_route("/", &routes).expect("page / should match");
        assert_eq!(m.route.kind, None, "matched the page, not a boundary");
    }

    #[test]
    fn find_not_found_root_covers_unmatched() {
        let routes = vec![route("/", None), route("/", Some("not-found"))];
        let nf = find_not_found_route("/anything/here", &routes)
            .expect("root not-found covers any unmatched URL");
        assert_eq!(nf.kind.as_deref(), Some("not-found"));
        assert_eq!(nf.path, "/");
    }

    #[test]
    fn find_not_found_prefers_longest_prefix() {
        let routes = vec![
            route("/", Some("not-found")),
            route("/blog", Some("not-found")),
        ];
        // A /blog/* miss is covered by the /blog boundary, not the root.
        let nf = find_not_found_route("/blog/missing-post", &routes).unwrap();
        assert_eq!(nf.path, "/blog");
        // A /other miss falls back to the root boundary.
        let nf2 = find_not_found_route("/other", &routes).unwrap();
        assert_eq!(nf2.path, "/");
    }

    #[test]
    fn find_not_found_none_when_no_boundary() {
        let routes = vec![route("/", None), route("/blog", None)];
        assert!(find_not_found_route("/missing", &routes).is_none());
    }

    #[test]
    fn match_ssr_route_catch_all_consumes_rest() {
        // app/docs/[...slug]/page.tsx → "/docs/*slug"
        let routes = vec![route("/docs/*slug", None)];
        let m = match_ssr_route("/docs/a/b/c", &routes).expect("catch-all matches");
        assert_eq!(m.params.get("slug").map(String::as_str), Some("a/b/c"));
        // Single trailing segment still matches a required catch-all.
        let m1 = match_ssr_route("/docs/intro", &routes).expect("one segment matches");
        assert_eq!(m1.params.get("slug").map(String::as_str), Some("intro"));
        // The bare prefix does NOT match a REQUIRED catch-all (needs ≥1).
        assert!(match_ssr_route("/docs", &routes).is_none());
    }

    #[test]
    fn match_ssr_route_optional_catch_all_matches_bare_prefix() {
        // app/shop/[[...filters]]/page.tsx → "/shop/*?filters"
        let routes = vec![route("/shop/*?filters", None)];
        // Zero trailing segments → empty-string param.
        let m0 = match_ssr_route("/shop", &routes).expect("optional matches bare prefix");
        assert_eq!(m0.params.get("filters").map(String::as_str), Some(""));
        // One or more → joined.
        let m2 = match_ssr_route("/shop/red/small", &routes).expect("optional matches deep");
        assert_eq!(m2.params.get("filters").map(String::as_str), Some("red/small"));
    }

    #[test]
    fn match_ssr_route_specificity_static_beats_catch_all() {
        // Routes arrive pre-sorted by discoverAppRoutes: static, then param,
        // then catch-all. First match wins, so specificity is honored.
        let routes = vec![
            route("/docs/api", None),    // static
            route("/docs/:section", None), // dynamic param
            route("/docs/*slug", None),  // catch-all
        ];
        // Exact static wins over both dynamic and catch-all.
        let exact = match_ssr_route("/docs/api", &routes).unwrap();
        assert_eq!(exact.route.path, "/docs/api");
        // Single non-static segment → dynamic param wins over catch-all.
        let dyn_ = match_ssr_route("/docs/guides", &routes).unwrap();
        assert_eq!(dyn_.route.path, "/docs/:section");
        assert_eq!(dyn_.params.get("section").map(String::as_str), Some("guides"));
        // Multi-segment → only the catch-all can match.
        let deep = match_ssr_route("/docs/guides/intro", &routes).unwrap();
        assert_eq!(deep.route.path, "/docs/*slug");
        assert_eq!(deep.params.get("slug").map(String::as_str), Some("guides/intro"));
    }

    #[test]
    fn match_ssr_route_catch_all_with_param_prefix() {
        // app/users/[id]/[...rest]/page.tsx → "/users/:id/*rest"
        let routes = vec![route("/users/:id/*rest", None)];
        let m = match_ssr_route("/users/42/posts/7", &routes).expect("param + catch-all");
        assert_eq!(m.params.get("id").map(String::as_str), Some("42"));
        assert_eq!(m.params.get("rest").map(String::as_str), Some("posts/7"));
    }

    #[test]
    fn ssr_dev_error_overlay_escapes_and_includes_detail() {
        // A JS stack with HTML-special chars must be escaped (no markup
        // injection from the error text) while staying readable in the <pre>.
        let html = ssr_dev_error_overlay_html(
            "SSR_RENDER_FAILED",
            "TypeError: x is not a function\n  at <Page> (app/page.tsx:3)",
        );
        assert!(html.contains("SSR_RENDER_FAILED"));
        assert!(html.contains("TypeError: x is not a function"));
        // The "<Page>" in the stack is escaped, not emitted as a tag.
        assert!(html.contains("&lt;Page&gt;"));
        assert!(!html.contains("<Page>"));
        // Mentions the error.tsx remedy + the dev-only nature.
        assert!(html.contains("error.tsx"));
        assert!(html.contains("PYLON_DEV_MODE"));
    }

    #[test]
    fn document_nav_excludes_assets() {
        assert!(looks_like_document_nav("/blog/missing"));
        assert!(looks_like_document_nav("/"));
        assert!(looks_like_document_nav("/deep/path?q=1"));
        assert!(!looks_like_document_nav("/favicon.ico"));
        assert!(!looks_like_document_nav("/assets/app.css"));
        assert!(!looks_like_document_nav("/img/logo.png?v=2"));
    }

    // --- SSR response_start header construction (build_ssr_response_headers) ---

    fn header_pairs(hdrs: &[Header]) -> Vec<(String, String)> {
        hdrs.iter()
            .map(|h| {
                (
                    h.field.as_str().as_str().to_ascii_lowercase(),
                    h.value.as_str().to_string(),
                )
            })
            .collect()
    }

    #[test]
    fn ssr_headers_inject_defaults_and_preserve_page_headers() {
        let mut page = std::collections::HashMap::new();
        page.insert("x-custom".to_string(), "hi".to_string());
        let pairs = header_pairs(&build_ssr_response_headers(&page, "https://app.example"));
        assert!(pairs.iter().any(|(k, v)| k == "x-custom" && v == "hi"));
        assert!(pairs.iter().any(|(k, _)| k == "content-type"));
        assert!(pairs.iter().any(|(k, _)| k == "cache-control"));
        assert!(pairs
            .iter()
            .any(|(k, v)| k == "access-control-allow-origin" && v == "https://app.example"));
    }

    #[test]
    fn ssr_headers_page_content_type_wins_without_duplicate() {
        let mut page = std::collections::HashMap::new();
        page.insert(
            "content-type".to_string(),
            "application/rss+xml".to_string(),
        );
        let pairs = header_pairs(&build_ssr_response_headers(&page, "*"));
        let cts: Vec<&(String, String)> =
            pairs.iter().filter(|(k, _)| k == "content-type").collect();
        assert_eq!(cts.len(), 1, "exactly one content-type (no default added)");
        assert_eq!(cts[0].1, "application/rss+xml");
    }

    #[test]
    fn ssr_headers_split_multiple_set_cookie() {
        let mut page = std::collections::HashMap::new();
        page.insert(
            "set-cookie".to_string(),
            "a=1; Path=/; HttpOnly\nb=2; Path=/; HttpOnly".to_string(),
        );
        let pairs = header_pairs(&build_ssr_response_headers(&page, "*"));
        let cookies: Vec<&String> = pairs
            .iter()
            .filter(|(k, _)| k == "set-cookie")
            .map(|(_, v)| v)
            .collect();
        assert_eq!(cookies.len(), 2, "one Set-Cookie header per cookie");
        assert!(cookies.iter().any(|c| c.starts_with("a=1")));
        assert!(cookies.iter().any(|c| c.starts_with("b=2")));
    }

    #[test]
    fn ssr_cookie_response_is_no_store_else_no_cache() {
        let cc = |page: &std::collections::HashMap<String, String>| {
            header_pairs(&build_ssr_response_headers(page, "*"))
                .into_iter()
                .find(|(k, _)| k == "cache-control")
                .map(|(_, v)| v)
        };
        // Anonymous page → no-cache (it can opt into edge caching explicitly).
        let anon = std::collections::HashMap::new();
        assert_eq!(cc(&anon).as_deref(), Some("no-cache"));
        // A cookie-setting (personalized) page → no-store, so a shared cache
        // can never store it and replay it to another user.
        let mut authed = std::collections::HashMap::new();
        authed.insert("set-cookie".to_string(), "sid=abc; HttpOnly".to_string());
        assert_eq!(cc(&authed).as_deref(), Some("no-store"));
        // A page that sets its OWN Cache-Control is never overridden.
        let mut custom = std::collections::HashMap::new();
        custom.insert("set-cookie".to_string(), "sid=abc".to_string());
        custom.insert("cache-control".to_string(), "private, max-age=0".to_string());
        assert_eq!(cc(&custom).as_deref(), Some("private, max-age=0"));
    }

    #[test]
    fn ssr_headers_carry_security_defaults_overridable() {
        let pairs = header_pairs(&build_ssr_response_headers(
            &std::collections::HashMap::new(),
            "*",
        ));
        let get = |k: &str| pairs.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        assert_eq!(get("x-frame-options").as_deref(), Some("SAMEORIGIN")); // clickjacking
        assert_eq!(get("x-content-type-options").as_deref(), Some("nosniff"));
        assert!(get("referrer-policy").is_some());
        assert!(get("permissions-policy").is_some());
        // A page can override X-Frame-Options (embeddable widget) with NO
        // duplicate header emitted.
        let mut page = std::collections::HashMap::new();
        page.insert("x-frame-options".to_string(), "ALLOWALL".to_string());
        let p2 = header_pairs(&build_ssr_response_headers(&page, "*"));
        let xfo: Vec<&String> = p2
            .iter()
            .filter(|(n, _)| n == "x-frame-options")
            .map(|(_, v)| v)
            .collect();
        assert_eq!(xfo.len(), 1, "no duplicate X-Frame-Options");
        assert_eq!(xfo[0], "ALLOWALL");
    }

    #[test]
    fn ssr_headers_reject_crlf_injection() {
        // A page that tries to smuggle a second header via CR/LF must be
        // dropped entirely — no response splitting.
        let mut page = std::collections::HashMap::new();
        page.insert(
            "x-evil".to_string(),
            "ok\r\nset-cookie: stolen=1".to_string(),
        );
        let pairs = header_pairs(&build_ssr_response_headers(&page, "*"));
        assert!(
            pairs.iter().all(|(_, v)| !v.contains("stolen")),
            "CRLF-injected value must not appear in any header"
        );
        assert!(
            !pairs.iter().any(|(k, _)| k == "x-evil"),
            "the unsafe header is dropped, not partially applied"
        );
    }

    #[test]
    fn ssr_headers_reject_unsafe_header_name() {
        // A page-set header NAME carrying CR/LF (tiny_http does NOT reject
        // these) or a separator (`:`) is dropped entirely — no injected
        // header survives.
        let mut page = std::collections::HashMap::new();
        page.insert("x-evil\r\nset-cookie".to_string(), "stolen=1".to_string());
        page.insert("x:colon".to_string(), "v".to_string());
        let pairs = header_pairs(&build_ssr_response_headers(&page, "*"));
        assert!(pairs
            .iter()
            .all(|(k, _)| !k.contains('\r') && !k.contains('\n') && !k.contains(':')));
        assert!(
            !pairs.iter().any(|(_, v)| v == "stolen=1" || v == "v"),
            "values of unsafe-named headers must not leak"
        );
    }
}
