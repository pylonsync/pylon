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
    /// Org store — used to enrich the SSR auth context's `roles` with the
    /// caller's role in their active org, so SSR pages see the same
    /// `auth.roles` the main request handler resolves. None → SSR `roles`
    /// stays whatever the session carries (empty for plain sessions).
    pub orgs: Option<std::sync::Arc<pylon_auth::org::OrgStore>>,
    /// Runtime handle — lets the SSR auth resolver run the SAME per-user
    /// admin-lift (`auth.user.adminField` + `PYLON_ADMIN_EMAILS`) the main
    /// HTTP handler applies, so SSR pages see the same `auth.is_admin`. None →
    /// SSR `is_admin` reflects only what the session carries (never admin).
    pub runtime: Option<std::sync::Arc<crate::Runtime>>,
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
            // Plus the fallback locations the CLI's `ensure_frontend_built`
            // writes to when /app/web/ is read-only (Pylon Cloud / Fly
            // files-mount with root ownership of the source dir).
            discover_dist_dir(app_dir)
        };

        Self {
            dir,
            dev_proxy,
            ssr_routes: std::sync::Arc::new(Vec::new()),
            fn_ops: None,
            session_store: None,
            cookie_config: None,
            orgs: None,
            runtime: None,
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

    /// Attach the org store so SSR auth resolution can enrich `roles` with the
    /// caller's active-org role (mirrors the main HTTP request handler).
    pub fn with_orgs(mut self, orgs: std::sync::Arc<pylon_auth::org::OrgStore>) -> Self {
        self.orgs = Some(orgs);
        self
    }

    /// Attach the runtime so SSR auth resolution runs the same per-user
    /// admin-lift (`crate::server::lift_admin`) the main HTTP handler applies —
    /// keeping SSR `auth.is_admin` in parity with the API/sync paths.
    pub fn with_runtime(mut self, runtime: std::sync::Arc<crate::Runtime>) -> Self {
        self.runtime = Some(runtime);
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
    // Framework routes are anchored to an exact match OR a trailing-slash
    // subpath (like /health below) — a bare `starts_with("/studio")` also
    // swallowed legitimate SSR routes such as /studios, /eventsfeed,
    // /metrics-report and 404'd them.
    !(path.starts_with("/api/")
        || path == "/api"
        || path == "/studio"
        || path.starts_with("/studio/")
        || path == "/events"
        || path.starts_with("/events/")
        || path == "/metrics"
        || path.starts_with("/metrics/")
        || path == "/health"
        || path.starts_with("/health/")
        // The framework's admin surface is these specific token-gated
        // endpoints (see server.rs), NOT the whole /admin/* namespace —
        // apps legitimately serve SSR pages like /admin/orgs. A blanket
        // starts_with("/admin/") 404'd every app-defined admin page.
        || path == "/admin/entities"
        || path.starts_with("/admin/entities/")
        || path.starts_with("/admin/fn/")
        || path == "/admin/jobs"
        || path.starts_with("/admin/jobs/")
        || path.starts_with("/admin/logs/")
        || path == "/admin/workflows"
        || path.starts_with("/admin/workflows/")
        || path.starts_with("/.well-known/")
        // The OIDC provider's whole surface. `/.well-known/` above already
        // keeps the discovery doc out of the SPA, but the endpoints it
        // ADVERTISES live under /oidc/ — leaving them SPA-eligible meant any
        // app with a frontend served its 404 page for /oidc/jwks while a
        // headless app served real keys. IdP mode worked in every test and
        // failed on the first production app, because every production app
        // has a frontend.
        || path == "/oidc"
        || path.starts_with("/oidc/"))
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
        Some("glb") => "model/gltf-binary",
        Some("gltf") => "model/gltf+json",
        Some("hdr") => "image/vnd.radiance",
        Some("mp3") => "audio/mpeg",
        Some("ogg") => "audio/ogg",
        Some("txt") => "text/plain; charset=utf-8",
        // RFC 7763. Without this a committed `public/llms.md` or agent skill
        // file served as `application/octet-stream`, which agents download
        // instead of read.
        Some("md") | Some("markdown") => "text/markdown; charset=utf-8",
        Some("xml") => "application/xml",
        Some("pdf") => "application/pdf",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        Some("mov") => "video/quicktime",
        _ => "application/octet-stream",
    }
}

/// RFC 7233 single-range parse result for a body of `total` bytes.
#[derive(Debug, PartialEq, Eq)]
enum RangeSpec {
    /// No usable `Range` header → serve the whole body (200).
    Full,
    /// A satisfiable single range, as inclusive byte offsets.
    Partial(u64, u64),
    /// A syntactically valid but unsatisfiable range → 416.
    Unsatisfiable,
}

/// Parse a `Range` header value against a body of `total` bytes. Handles the
/// three single-range forms — `bytes=start-end`, `bytes=start-` (open-ended),
/// and `bytes=-suffix` (last N bytes). A multi-range request
/// (`bytes=0-1,5-6`) or anything unparseable falls back to `Full` (we don't
/// emit `multipart/byteranges`), so serving degrades to a plain 200.
fn parse_byte_range(header: &str, total: u64) -> RangeSpec {
    let spec = match header.trim().strip_prefix("bytes=") {
        Some(s) => s.trim(),
        None => return RangeSpec::Full,
    };
    // Multi-range → fall back to a full response.
    if spec.contains(',') {
        return RangeSpec::Full;
    }
    let (start_s, end_s) = match spec.split_once('-') {
        Some(p) => (p.0.trim(), p.1.trim()),
        None => return RangeSpec::Full,
    };

    if start_s.is_empty() {
        // Suffix range: the last N bytes (`bytes=-N`).
        let suffix: u64 = match end_s.parse() {
            Ok(n) => n,
            Err(_) => return RangeSpec::Full,
        };
        if suffix == 0 || total == 0 {
            return RangeSpec::Unsatisfiable;
        }
        return RangeSpec::Partial(total.saturating_sub(suffix), total - 1);
    }

    let start: u64 = match start_s.parse() {
        Ok(n) => n,
        Err(_) => return RangeSpec::Full,
    };
    if start >= total {
        return RangeSpec::Unsatisfiable;
    }
    let end: u64 = if end_s.is_empty() {
        total - 1
    } else {
        match end_s.parse::<u64>() {
            Ok(n) => n.min(total - 1),
            Err(_) => return RangeSpec::Full,
        }
    };
    if end < start {
        return RangeSpec::Unsatisfiable;
    }
    RangeSpec::Partial(start, end)
}

/// Serve `bytes` for a GET, honoring a single HTTP `Range` request (RFC 7233):
/// `206 Partial Content` + `Content-Range` for a satisfiable range, `416` for
/// an unsatisfiable one, else a full `200`. ALWAYS advertises
/// `Accept-Ranges: bytes` — iOS Safari refuses to play a `<video>` whose source
/// answers a range request with `200` instead of `206`, so without this every
/// video served from `public/` shows a black box with a slashed-out play button
/// on iPhone/iPad (desktop tolerates `200`, which masks it).
fn respond_static_file(
    request: Request,
    bytes: Vec<u8>,
    content_type: &'static str,
    cache: &str,
    cors_origin: &str,
) {
    let total = bytes.len() as u64;
    let range = request.headers().iter().find_map(|h| {
        let field = h.field.as_str();
        if field == "Range" || field == "range" {
            Some(h.value.as_str().to_string())
        } else {
            None
        }
    });

    let ct = Header::from_bytes("Content-Type", content_type).unwrap();
    let cors = Header::from_bytes(
        "Access-Control-Allow-Origin",
        cors_origin.as_bytes().to_vec(),
    )
    .unwrap();
    let cache_h = Header::from_bytes("Cache-Control", cache).unwrap();
    let accept_ranges = Header::from_bytes("Accept-Ranges", "bytes").unwrap();

    let spec = range
        .as_deref()
        .map(|r| parse_byte_range(r, total))
        .unwrap_or(RangeSpec::Full);

    let response = match spec {
        RangeSpec::Partial(start, end) => {
            let slice = bytes[start as usize..=end as usize].to_vec();
            let content_range =
                Header::from_bytes("Content-Range", format!("bytes {start}-{end}/{total}"))
                    .unwrap();
            Response::from_data(slice)
                .with_status_code(206)
                .with_header(ct)
                .with_header(cors)
                .with_header(cache_h)
                .with_header(accept_ranges)
                .with_header(content_range)
        }
        RangeSpec::Unsatisfiable => {
            let content_range =
                Header::from_bytes("Content-Range", format!("bytes */{total}")).unwrap();
            Response::from_data(Vec::new())
                .with_status_code(416)
                .with_header(cors)
                .with_header(accept_ranges)
                .with_header(content_range)
        }
        RangeSpec::Full => Response::from_data(bytes)
            .with_status_code(200)
            .with_header(ct)
            .with_header(cors)
            .with_header(cache_h)
            .with_header(accept_ranges),
    };
    let _ = request.respond(response);
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
fn is_valid_colocated_asset_rel(rel: &str, app_dir: &str) -> bool {
    if rel.is_empty() || rel.starts_with('/') || rel.starts_with('\\') {
        return false;
    }
    let segs: Vec<&str> = rel.split('/').collect();
    // Must live UNDER the app's frontend dir — `app` by default, but a subdir
    // appDir like `web/app` (control-plane / monorepo frontends) is equally
    // valid. The appDir is server config (derived from the manifest routes),
    // never request-controlled, so matching against it can't widen the read.
    let app_segs: Vec<&str> = app_dir.split('/').filter(|s| !s.is_empty()).collect();
    if app_segs.is_empty() || segs.len() <= app_segs.len() {
        return false;
    }
    for (i, a) in app_segs.iter().enumerate() {
        if segs[i] != *a {
            return false;
        }
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
fn serve_og_image(
    request: Request,
    url: &str,
    cors_origin: &str,
    app_dir: &str,
) -> Result<(), Request> {
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
    if !is_valid_colocated_asset_rel(&rel, app_dir) {
        return four_oh_four(request);
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    // Canonicalize both the app root and the candidate, then prefix-check.
    // The rel is already traversal-free, but a symlink under the app dir
    // could still escape — this closes that.
    let (app_root, canon) = match (
        cwd.join(app_dir).canonicalize(),
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
pub(crate) fn is_dev_mode() -> bool {
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
///
/// Written RAW via `Request::into_writer()` + an explicit flush per event —
/// NOT `request.respond()`. tiny_http buffers respond() output in a 1KB
/// BufWriter that it only flushes after the whole body EOFs; this stream
/// never EOFs, so the status line + `hello` event sat unsent (~45s of
/// heartbeats to fill 1KB) and the browser's EventSource never connected —
/// hot reload was dead. The raw writer also moves the connection off the
/// request-pool worker (the heartbeat loop runs on its own thread), so an
/// open dev tab doesn't pin a pool slot.
fn serve_dev_live_reload(request: Request, cors_origin: &str) -> Result<(), Request> {
    // Head + first event in one write. `Connection: close` because we take
    // the socket over for the stream's lifetime — no further requests ride
    // this connection (EventSource holds it open; reconnects open fresh).
    // `retry:` sets the client's reconnect delay; `hello` carries the id.
    let head = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: text/event-stream\r\n\
         Cache-Control: no-cache\r\n\
         Connection: close\r\n\
         X-Accel-Buffering: no\r\n\
         Access-Control-Allow-Origin: {cors_origin}\r\n\
         \r\n\
         retry: 500\nevent: hello\ndata: {}\n\n",
        dev_boot_id()
    );
    let mut writer = request.into_writer();
    // Register this connection so an in-process reload (`trigger_dev_reload`,
    // used for UI-only edits that don't re-exec the process) can push it a
    // reload event. Dead senders are reaped by the trigger on its next fire.
    let (tx, rx) = std::sync::mpsc::channel::<u64>();
    dev_reload_registry().lock().unwrap().push(tx);
    // Dedicated thread per open dev tab (dev-only endpoint). The heartbeat
    // keeps proxies/connections alive; a failed write/flush means the client
    // went away → exit, dropping the writer, which closes the connection.
    std::thread::Builder::new()
        .name("pylon-dev-live-reload".into())
        .stack_size(64 * 1024)
        .spawn(move || {
            use std::io::Write as _;
            if writer.write_all(head.as_bytes()).is_err() || writer.flush().is_err() {
                return;
            }
            loop {
                match rx.recv_timeout(std::time::Duration::from_secs(1)) {
                    Ok(generation) => {
                        // In-process reload: re-emit `hello` with a fresh id so
                        // the client's EventSource sees the data change and
                        // reloads — the same signal a restart gives, without one.
                        let frame =
                            format!("event: hello\ndata: {}:r{}\n\n", dev_boot_id(), generation);
                        if writer.write_all(frame.as_bytes()).is_err() || writer.flush().is_err() {
                            return;
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        if writer.write_all(b": ping\n\n").is_err() || writer.flush().is_err() {
                            return;
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
        })
        .ok();
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

/// Like `resolve_safe` but for a WRITE target: the leaf need not exist yet.
/// Rejects `..`/`.`/empty segments (so the join can't climb out of `root`) and,
/// as defense-in-depth against a symlinked subdir, canonicalizes the parent when
/// it exists and confirms it's still inside `root`. Returns the absolute path.
fn resolve_safe_for_write(root: &Path, rel_path: &str) -> Option<PathBuf> {
    let path_only = rel_path.split('?').next()?.split('#').next()?;
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
    let canonical_root = root.canonicalize().ok()?;
    let target = canonical_root.join(trimmed);
    if let Some(parent) = target.parent() {
        if parent.exists() {
            let cp = parent.canonicalize().ok()?;
            if !cp.starts_with(&canonical_root) {
                return None;
            }
        }
    }
    Some(target)
}

/// Dev-only remote file-write endpoint (`PUT`/`POST`/`DELETE
/// /_pylon/dev/files/<path>`). Writes/removes a file in the live `pylon dev`
/// workspace (`PYLON_DEV_WATCH_DIR`, else cwd); the fs-watcher hot-reloads the
/// change. Optionally requires `Authorization: Bearer <PYLON_DEV_FILE_API_TOKEN>`
/// when that env is set (the cloud sets a per-env token; unset = open locally).
fn serve_dev_file_write(
    mut request: Request,
    path_only: &str,
    cors_origin: &str,
) -> Result<(), Request> {
    use std::io::Read as _;

    // CORS preflight — build.pylonsync.com calls this cross-origin.
    if matches!(request.method(), Method::Options) {
        let mut resp = Response::empty(204u16);
        for (k, v) in [
            ("Access-Control-Allow-Origin", cors_origin),
            ("Access-Control-Allow-Methods", "PUT, POST, DELETE, OPTIONS"),
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
        return Ok(());
    }

    // Optional bearer-token gate (unset = open, and it's dev-mode only anyway).
    if let Ok(token) = std::env::var("PYLON_DEV_FILE_API_TOKEN") {
        if !token.is_empty() {
            let want = format!("Bearer {token}");
            let ok = request.headers().iter().any(|h| {
                let field = h.field.as_str();
                (field == "Authorization" || field == "authorization")
                    && h.value.as_str() == want.as_str()
            });
            if !ok {
                let _ =
                    request.respond(Response::from_string("unauthorized").with_status_code(401u16));
                return Ok(());
            }
        }
    }

    let root = std::env::var("PYLON_DEV_WATCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let rel = path_only.strip_prefix("/_pylon/dev/files/").unwrap_or("");
    let cors = |resp: Response<std::io::Cursor<Vec<u8>>>| match Header::from_bytes(
        "Access-Control-Allow-Origin",
        cors_origin.as_bytes(),
    ) {
        Ok(h) => resp.with_header(h),
        Err(_) => resp,
    };

    let Some(target) = resolve_safe_for_write(&root, rel) else {
        let _ = request.respond(cors(
            Response::from_string("{\"ok\":false,\"error\":\"bad path\"}").with_status_code(400u16),
        ));
        return Ok(());
    };

    let method = request.method().clone();
    match method {
        Method::Delete => {
            let resp = match std::fs::remove_file(&target) {
                Ok(()) => Response::from_string("{\"ok\":true}"),
                Err(e) => Response::from_string(format!("{{\"ok\":false,\"error\":\"{e}\"}}"))
                    .with_status_code(500u16),
            };
            let _ = request.respond(cors(resp));
            Ok(())
        }
        Method::Put | Method::Post => {
            const MAX: u64 = 10 * 1024 * 1024;
            let mut body = Vec::new();
            let read = request.as_reader().take(MAX + 1).read_to_end(&mut body);
            if read.is_err() || body.len() as u64 > MAX {
                let _ = request.respond(cors(
                    Response::from_string("{\"ok\":false,\"error\":\"body too large\"}")
                        .with_status_code(413u16),
                ));
                return Ok(());
            }
            if let Some(parent) = target.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    let _ = request.respond(cors(
                        Response::from_string(format!("{{\"ok\":false,\"error\":\"{e}\"}}"))
                            .with_status_code(500u16),
                    ));
                    return Ok(());
                }
            }
            let resp = match std::fs::write(&target, &body) {
                Ok(()) => Response::from_string("{\"ok\":true}"),
                Err(e) => Response::from_string(format!("{{\"ok\":false,\"error\":\"{e}\"}}"))
                    .with_status_code(500u16),
            };
            let _ = request.respond(cors(resp));
            Ok(())
        }
        _ => {
            let _ = request.respond(cors(
                Response::from_string("{\"ok\":false,\"error\":\"method not allowed\"}")
                    .with_status_code(405u16),
            ));
            Ok(())
        }
    }
}

/// Top-level entry. Decides between dev-proxy and disk-serving based
/// on config, applies eligibility rules, sends the response.
///
/// `Ok(())` = response was sent (success or upstream error). The caller
/// should record metrics and exit the worker. `Err(request)` = path was
/// API-bound or unhandled; the caller continues to existing routing
/// with the same request restored.
/// A page requested as markdown — either through `Accept: text/markdown` or
/// through its `<path>.md` URL. Threaded into the render so the page renders
/// under its own path (a `.md` URL must not leak into `canonical`/`og:url`)
/// and the response comes back converted.
#[derive(Debug, Clone)]
pub(crate) struct MarkdownRequest {
    /// Markdown, or markdown labelled `text/plain` — whichever the client asked
    /// for.
    representation: crate::markdown::Representation,
    /// True when the client named the `.md` URL. Such a request has no HTML
    /// fallback: the resource it asked for either exists or doesn't.
    explicit_url: bool,
    /// The page's own URL (path + query), with any `.md` suffix removed.
    render_url: String,
    /// Whether the client would also read HTML — the fallback when a page opts
    /// out of markdown.
    html_acceptable: bool,
}

/// Which representation of a page this request wants.
pub(crate) enum PageVariant {
    Html,
    Markdown(MarkdownRequest),
    /// The client ruled out every representation this server can produce.
    NotAcceptable,
}

/// Routes whose representation is fixed by the file convention that declares
/// them, not by the request: `app/sitemap.ts` is XML, `app/robots.ts` and
/// `app/llms.ts` are plain text, `opengraph-image` is a PNG.
fn is_data_route_kind(kind: Option<&str>) -> bool {
    matches!(
        kind,
        Some("sitemap") | Some("robots") | Some("llms") | Some("og-image")
    )
}

/// Read one request header, lower-cased name match.
fn header_value(request: &Request, name: &str) -> Option<String> {
    request.headers().iter().find_map(|h| {
        if h.field.as_str().as_str().eq_ignore_ascii_case(name) {
            Some(h.value.as_str().to_string())
        } else {
            None
        }
    })
}

/// Decide which representation of an SSR page to serve.
///
/// HTML wins in every ambiguous case — see `markdown::negotiate`. Two things
/// short-circuit the negotiation entirely:
///   - a client-router navigation (`x-pylon-nav`), which wants the hydration
///     payload for the URL, not a document at all; and
///   - a real file under `public/` at the requested `.md` path, which keeps
///     `public/pylon-skill.md` beating a `/pylon-skill` page's variant.
fn page_variant(request: &Request, url: &str, accept: Option<&str>) -> PageVariant {
    let path_only = url.split('?').next().unwrap_or(url);
    page_variant_for(
        is_nav_request(request),
        url,
        accept,
        resolve_safe(&public_dir(), path_only).is_some(),
    )
}

/// The decision behind [`page_variant`], with the two request-derived facts
/// passed in — so the routing rules are unit-testable without a live socket.
fn page_variant_for(
    nav: bool,
    url: &str,
    accept: Option<&str>,
    public_file_exists: bool,
) -> PageVariant {
    use crate::markdown::{Negotiation, Representation};
    if nav {
        return PageVariant::Html;
    }
    let (path_only, query) = match url.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (url, None),
    };
    let html_acceptable = crate::markdown::accepts_html(accept);
    if let Some(page_path) = crate::markdown::strip_md_suffix(path_only) {
        if public_file_exists {
            return PageVariant::Html;
        }
        let render_url = match query {
            Some(q) if !q.is_empty() => format!("{page_path}?{q}"),
            _ => page_path,
        };
        return PageVariant::Markdown(MarkdownRequest {
            representation: Representation::Markdown,
            explicit_url: true,
            render_url,
            html_acceptable,
        });
    }
    match crate::markdown::negotiate(accept) {
        Negotiation::Serve(Representation::Html) => PageVariant::Html,
        Negotiation::Serve(representation) => PageVariant::Markdown(MarkdownRequest {
            representation,
            explicit_url: false,
            render_url: url.to_string(),
            html_acceptable,
        }),
        Negotiation::NotAcceptable => PageVariant::NotAcceptable,
    }
}

/// RFC 9110 §15.5.7. The body names what this URL can produce, so an agent that
/// guessed a media type can correct itself without a second discovery step.
fn not_acceptable_response(
    path: &str,
    markdown_available: bool,
    cors_origin: &str,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let body = if markdown_available {
        format!(
            "# 406 Not Acceptable\n\n\
             `{path}` can be served as:\n\n\
             - `text/html` — the rendered page\n\
             - `text/markdown` — the same page as markdown (also at `{}`)\n\
             - `text/plain` — the markdown body, labelled as plain text\n",
            crate::markdown::md_url_for(path)
        )
    } else {
        format!(
            "# 406 Not Acceptable\n\n\
             `{path}` can be served as `text/html` only. This route declines its \
             markdown representation (`export const markdown = false`).\n"
        )
    };
    let mut resp = Response::from_data(body.into_bytes()).with_status_code(406);
    for (name, value) in [
        ("Content-Type", "text/markdown; charset=utf-8"),
        ("Vary", "Accept"),
        ("Cache-Control", "no-store"),
        ("X-Content-Type-Options", "nosniff"),
    ] {
        if let Ok(h) = Header::from_bytes(name, value) {
            resp = resp.with_header(h);
        }
    }
    if let Ok(h) = Header::from_bytes("Access-Control-Allow-Origin", cors_origin.as_bytes()) {
        resp = resp.with_header(h);
    }
    resp
}

pub fn try_handle(
    cfg: &FrontendConfig,
    request: Request,
    cors_origin: &str,
) -> Result<(), Request> {
    // Dev-only remote file-write API (#dev-mode-env). Handled FIRST: it's a
    // non-GET (PUT/POST/DELETE/OPTIONS) request that must NOT fall through the
    // non-GET early-return below into API routing. 404 in prod (dev-mode gate).
    {
        let url = request.url().to_string();
        let path_only = url.split('?').next().unwrap_or(&url);
        if path_only.starts_with("/_pylon/dev/files/") {
            if is_dev_mode() {
                return serve_dev_file_write(request, path_only, cors_origin);
            }
            return Err(request);
        }
    }

    // route.ts form/method handlers (#276): a non-GET request that matches a
    // discovered `kind:"route"` path is dispatched to its POST/PUT/PATCH/DELETE
    // handler. CSRF is enforced by the caller (`server.rs`) BEFORE `try_handle`
    // runs — the Origin/Referer gate clears first, so a cross-site form forgery
    // never reaches this dispatch. Everything else non-GET falls through to the
    // API router (the early return below).
    if !matches!(request.method(), Method::Get | Method::Head)
        && !cfg.ssr_routes.is_empty()
        && cfg.fn_ops.is_some()
    {
        let url = request.url().to_string();
        if let Some(matched) = match_form_route(&url, &cfg.ssr_routes) {
            return serve_via_form_rpc(cfg, matched, request, cors_origin);
        }
    }

    // Only GET / HEAD get the SPA treatment. POST/PATCH/DELETE that didn't
    // match a route handler belong to API routing — serving them HTML would be
    // a silent failure mode that's hard to debug.
    if !matches!(request.method(), Method::Get | Method::Head) {
        return Err(request);
    }

    let url = request.url().to_string();
    if !is_spa_eligible(&url) {
        // `/.well-known/*` is deliberately not SPA-eligible — the framework
        // answers `/.well-known/openid-configuration` there. But an app still
        // has to publish its OWN well-known documents (security.txt, an
        // apple-app-site-association, an MCP manifest), and before this every
        // one of them 404'd with no way around it. Serve them off
        // `public/.well-known/`; anything else falls through to the framework
        // router, which keeps its own endpoint unshadowable.
        let path_only = url.split('?').next().unwrap_or(&url);
        if path_only.starts_with("/.well-known/")
            && path_only != "/.well-known/openid-configuration"
        {
            if let Some(file_path) = resolve_safe(&public_dir(), path_only) {
                if let Ok(bytes) = std::fs::read(&file_path) {
                    let ct = content_type_for(&file_path);
                    let cache = if is_dev_mode() {
                        "no-cache, must-revalidate"
                    } else {
                        "public, max-age=3600"
                    };
                    respond_static_file(request, bytes, ct, cache, cors_origin);
                    return Ok(());
                }
            }
        }
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
        let img_src_dir = local_image_source_dir(cfg.dir.as_deref(), &cwd);
        crate::image_optim::serve(
            request,
            &pylon_dir,
            Some(img_src_dir.as_path()),
            cors_origin,
        );
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
        // The colocated-asset root is the app's frontend dir, which may be a
        // subdir (`web/app`) — derive it from the manifest routes so the og
        // endpoint matches the `src=<appDir>/.../opengraph-image.png` the SSR
        // metadata renderer emits. (Hardcoding `app/` 404'd every monorepo /
        // control-plane frontend — e.g. pylonsync.com under `web/app`.)
        let app_dir = derive_app_dir(&cfg.ssr_routes);
        return serve_og_image(request, &url, cors_origin, &app_dir);
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

    // Agent diagnostics (#hud-for-agents): the machine-readable side of the dev
    // HUD. Returns the recent-SSR-render ring (verdict + reason + timing per
    // route) as JSON so a coding agent — or `pylon diagnostics` — can read why a
    // page is/isn't caching without a browser. Dev-only (404 in prod).
    if path_only == "/_pylon/dev/diagnostics" {
        if is_dev_mode() {
            let body = crate::dev_diagnostics::snapshot_json();
            let mut resp = Response::from_data(body.into_bytes());
            if let Ok(h) = Header::from_bytes("Content-Type", "application/json") {
                resp = resp.with_header(h);
            }
            if let Ok(h) = Header::from_bytes("Access-Control-Allow-Origin", cors_origin.as_bytes())
            {
                resp = resp.with_header(h);
            }
            let _ = request.respond(resp);
            return Ok(());
        }
        return Err(request);
    }

    // SSR branch sits ABOVE dev_proxy so file-based pages take
    // precedence over Vite's catch-all (Vite would serve the SPA
    // shell, masking the SSR'd output). Falls through to proxy/disk
    // when no SSR route matches.
    if !cfg.ssr_routes.is_empty() && cfg.fn_ops.is_some() {
        // Which representation does this client want? A markdown request routes
        // to the SAME page — matched on the path with any `.md` suffix removed
        // — and is converted after the render (see `serve_via_ssr_rpc`).
        let accept = header_value(&request, "accept");
        let variant = page_variant(&request, &url, accept.as_deref());
        let match_url: String = match &variant {
            PageVariant::Markdown(m) => m.render_url.clone(),
            _ => url.clone(),
        };
        let md_request: Option<MarkdownRequest> = match &variant {
            PageVariant::Markdown(m) => Some(m.clone()),
            _ => None,
        };
        // Data routes (`sitemap.ts`, `robots.ts`, `llms.ts`, `opengraph-image`)
        // each have exactly ONE representation, chosen by the convention rather
        // than by the client: XML, plain text, a PNG. There is nothing to
        // negotiate, so `Accept` never converts them — and `/sitemap.xml.md`
        // names nothing at all, so it falls through to the 404 boundary.
        let explicit_md = md_request.as_ref().is_some_and(|m| m.explicit_url);
        let matched_page = match_ssr_route(&match_url, &cfg.ssr_routes)
            .filter(|m| !(explicit_md && is_data_route_kind(m.route.kind.as_deref())));
        let md_request = match matched_page.as_ref().map(|m| m.route.kind.as_deref()) {
            Some(kind) if is_data_route_kind(kind) => None,
            _ => md_request,
        };
        if let Some(matched) = matched_page {
            // 406 only once we know this URL really is a page — an unmatched
            // URL is a 404 regardless of what the client would have accepted.
            if matches!(variant, PageVariant::NotAcceptable) {
                let path = match_url.split('?').next().unwrap_or(&match_url);
                let _ = request.respond(not_acceptable_response(path, true, cors_origin));
                return Ok(());
            }
            tracing::debug!(
                url = %url,
                route = %matched.route.path,
                variant = if md_request.is_some() { "markdown" } else { "html" },
                "SSR match"
            );
            // A dynamic-segment match yields to a real `public/` file
            // (Next semantics) — see dynamic_match_public_override.
            if let Some(file_path) =
                dynamic_match_public_override(&public_dir(), &matched.params, path_only)
            {
                if let Ok(bytes) = std::fs::read(&file_path) {
                    let ct = content_type_for(&file_path);
                    let cache = if is_dev_mode() {
                        "no-cache, must-revalidate"
                    } else {
                        "public, max-age=3600"
                    };
                    respond_static_file(request, bytes, ct, cache, cors_origin);
                    return Ok(());
                }
            }
            // #277 Stage 2 — on-disk ISR fast path. A render that proved
            // itself anonymous-safe (Stage 1 `x-pylon-cacheable`) was teed to
            // `.pylon/.cache/ssr`. Serve it straight off disk — skipping the
            // Bun render entirely — when ALL hold:
            //   - GET with NO query string (the cache key is path-only; a
            //     query bypasses both read + write so distinct query strings
            //     can't poison/explode the cache),
            //   - NO session cookie present (defense-in-depth: an authed
            //     request never receives a shared entry, even though the
            //     entry is provably auth-independent), and
            //   - a FRESH entry exists (within its revalidate window).
            // A stale/missing entry falls through to a live render, which
            // rewrites the entry; CloudFlare absorbs the tail via the
            // stale-while-revalidate emitted on the live response.
            // Disabled in dev: the namespace is fixed ("dev") with no artifact
            // id, so a cached page would be served stale after an edit +
            // restart (the dev boot id changes but the cache key doesn't) —
            // a confusing "my change didn't show up" footgun. Dev always
            // renders live.
            // Bucket-eligible (PPR Phase 0): GET, no query, not dev — but a
            // session cookie IS allowed. Anon-eligible is the cookie-anonymous
            // subset of that. A bucket entry is keyed on session PRESENCE
            // (`ssr_cache_bucket_vary`) and is identity-free, so serving it to a
            // session-carrying request is safe; an anon entry stays restricted to
            // cookie-anonymous requests (defense-in-depth).
            let bucket_eligible = !is_dev_mode()
                && matches!(request.method(), Method::Get)
                && match_url
                    .split_once('?')
                    .map(|(_, q)| q.is_empty())
                    .unwrap_or(true);
            let cacheable_eligible = bucket_eligible && !session_cookie_present(cfg, &request);
            if bucket_eligible {
                let (cache_path, _) = match_url
                    .split_once('?')
                    .unwrap_or((match_url.as_str(), ""));
                let host = request_host(&request);
                // HTML and the navigation payload are different answers to the
                // same URL, so they key apart (see `ssr_cache_vary`).
                let nav = is_nav_request(&request);
                // …and so is the markdown representation. Both markdown types
                // (`text/markdown` / `text/plain`) share ONE entry: the body is
                // identical and only the Content-Type differs, which the serve
                // path applies per request.
                let md_key = md_request.as_ref().map(|_| ());
                // Anon entry (`export const revalidate`) — cookie-anonymous only.
                if cacheable_eligible {
                    let cache_vary = ssr_cache_vary(host.as_deref(), nav, md_key.is_some());
                    if let Some(entry) =
                        crate::ssr_cache::get(&matched.route.path, cache_path, &cache_vary)
                    {
                        if entry.fresh {
                            tracing::debug!(url = %url, "SSR cache hit (disk, anon)");
                            return serve_cached_ssr(
                                entry,
                                cors_origin,
                                nav,
                                md_request.as_ref(),
                                matched.route.kind.as_deref(),
                                request,
                            );
                        }
                    }
                }
                // Bucket entry (`export const cache = "auth-bucketed"`) — keyed on
                // RESOLVED session (signed-in vs not); ANY GET hits its bucket. An
                // expired/invalid cookie resolves to not-signed-in → sess=0, the
                // same bit the render path stores under.
                let session_present = session_authenticated(cfg, &request);
                let bucket_vary =
                    ssr_cache_bucket_vary(host.as_deref(), session_present, nav, md_key.is_some());
                if let Some(entry) =
                    crate::ssr_cache::get(&matched.route.path, cache_path, &bucket_vary)
                {
                    if entry.fresh {
                        tracing::debug!(url = %url, session = session_present, "SSR cache hit (disk, bucket)");
                        return serve_cached_bucket_ssr(
                            entry,
                            cors_origin,
                            nav,
                            md_request.as_ref(),
                            matched.route.kind.as_deref(),
                            request,
                        );
                    }
                }
            }
            return serve_via_ssr_rpc(
                cfg,
                matched,
                request,
                cors_origin,
                None,
                cacheable_eligible,
                bucket_eligible,
                md_request,
            );
        }
        // Raw GET routes: a `route.ts` exporting `GET` (kind:"route") matched
        // on this path. No page lives here (Next forbids page.tsx + route.ts in
        // one segment), so this only fires when the page match above missed.
        // The handler returns a body that the Bun runtime streams verbatim with
        // a custom content-type — the GET analogue of sitemap/robots, at an
        // arbitrary path (RSS/Atom, dynamic XML, .well-known, etc.). A route.ts
        // with no GET export answers 405 (Allow lists the methods it does have).
        if matches!(request.method(), Method::Get) {
            if let Some(matched) = match_form_route(&url, &cfg.ssr_routes) {
                tracing::debug!(url = %url, route = %matched.route.path, "SSR raw GET route");
                // Same `public/`-beats-dynamic-segments rule as pages.
                if let Some(file_path) =
                    dynamic_match_public_override(&public_dir(), &matched.params, path_only)
                {
                    if let Ok(bytes) = std::fs::read(&file_path) {
                        let ct = content_type_for(&file_path);
                        let cache = if is_dev_mode() {
                            "no-cache, must-revalidate"
                        } else {
                            "public, max-age=3600"
                        };
                        respond_static_file(request, bytes, ct, cache, cors_origin);
                        return Ok(());
                    }
                }
                return serve_via_form_rpc(cfg, matched, request, cors_origin);
            }
        }
        // No page matched. If the app defines a `not-found.tsx` boundary
        // and this looks like a document navigation (not a static asset),
        // render the boundary at HTTP 404 instead of silently SPA-falling-
        // back to the home shell at 200. Asset 404s and apps without a
        // not-found boundary keep the existing fallthrough (proxy / disk).
        //
        // Judged on `match_url`, so a markdown request for a missing page
        // (`/nope.md`, or `/nope` with `Accept: text/markdown`) gets the same
        // 404 boundary — as markdown. An agent that guessed a URL wrong reads
        // the recovery links instead of an opaque JSON error.
        if looks_like_document_nav(&match_url) {
            if let Some(nf) = find_not_found_route(&match_url, &cfg.ssr_routes) {
                tracing::debug!(url = %url, boundary = %nf.path, "SSR not-found");
                let matched = SsrMatch {
                    route: nf.clone(),
                    params: std::collections::HashMap::new(),
                };
                // A 404 boundary dispatch is never cacheable/bucketable.
                return serve_via_ssr_rpc(
                    cfg,
                    matched,
                    request,
                    cors_origin,
                    Some(404),
                    false,
                    false,
                    md_request,
                );
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
    // Static assets: `<app>/public/<path>` served verbatim at the
    // site root (Next-style). Sits BELOW the SSR branch — explicit
    // pages win — and ABOVE the dist/SPA fallback so a public file
    // can't be shadowed by index.html. Misses fall through. The same
    // resolve_safe traversal guard as dist serving applies (`..`,
    // symlink escapes, and non-files all return None).
    {
        if let Some(file_path) = resolve_safe(&public_dir(), path_only) {
            if let Ok(bytes) = std::fs::read(&file_path) {
                let ct = content_type_for(&file_path);
                let cache = if is_dev_mode() {
                    // Dev: always revalidate so edits show up.
                    "no-cache, must-revalidate"
                } else {
                    "public, max-age=3600"
                };
                // Range-aware: emits 206 for a `Range` request (iOS Safari
                // <video> needs it) + `Accept-Ranges: bytes` on every response.
                respond_static_file(request, bytes, ct, cache, cors_origin);
                return Ok(());
            }
        }
    }

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
    let mut candidates = [app_dir.join("web/dist"), app_dir.join("apps/web/dist")]
        .into_iter()
        // The CLI builds into these when /app/web/ is read-only (Pylon
        // Cloud / Fly). Both ends read the list from one place, because a
        // build written somewhere the server does not look reads to the
        // user as a frontend that silently never appears.
        .chain(
            pylon_kernel::util::frontend_build_dirs()
                .into_iter()
                .map(|d| d.join("dist")),
        );
    candidates.find(|p| p.join("index.html").is_file())
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
        // Range-aware (206 + Accept-Ranges) — same as public/. Hashed assets
        // (Vite emits ?v= and chunk-hash filenames) can be cached aggressively,
        // but keep a conservative one-hour public cache as the default so a
        // deploy bump is picked up on the next load; operators can front a CDN.
        respond_static_file(request, bytes, ct, "public, max-age=3600", cors_origin);
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

/// The Next-style `public/` root for this process.
fn public_dir() -> std::path::PathBuf {
    std::env::current_dir()
        .ok()
        .unwrap_or_default()
        .join("public")
}

/// A route match that consumed DYNAMIC segments yields to a real file
/// under `public/`. Next semantics: files in `public/` always win over
/// `[param]` / catch-all segments, so adding `app/[orgSlug]/page.tsx`
/// must not swallow `GET /icon.svg` — the segment matcher binds
/// `orgSlug="icon.svg"`, and the static-serving block sits BELOW the
/// SSR branch, so without this probe it never runs. A match with no
/// params is a fully-static route path; that keeps beating files
/// (Next refuses that collision outright, so the case is unambiguous).
///
/// Returns the resolved file path when the match should yield. The
/// same `resolve_safe` traversal guard as all static serving applies.
fn dynamic_match_public_override(
    public_dir: &std::path::Path,
    params: &std::collections::HashMap<String, String>,
    path_only: &str,
) -> Option<std::path::PathBuf> {
    if params.is_empty() {
        return None;
    }
    resolve_safe(public_dir, path_only)
}

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
        // Boundary routes (not-found / error) are never navigable, and a
        // `route` handler (route.ts form/method handler) is matched only for
        // non-GET methods by match_form_route — never rendered as a page.
        if matches!(
            r.kind.as_deref(),
            Some("not-found") | Some("error") | Some("route")
        ) {
            continue;
        }
        if let Some(params) = match_route_segments(&r.path, &url_segs) {
            return Some(SsrMatch {
                route: r.clone(),
                params,
            });
        }
    }
    None
}

/// Match a non-GET request to a `route.ts` form/method handler
/// (`kind == "route"`), reusing the same path-pattern matcher pages use (so
/// `:param` + `*catch-all` work in route paths too). First match wins.
pub fn match_form_route(url: &str, routes: &[pylon_kernel::ManifestRoute]) -> Option<SsrMatch> {
    let path = url.split('?').next().unwrap_or(url);
    let url_segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    for r in routes {
        if r.mode != "ssr" || r.kind.as_deref() != Some("route") {
            continue;
        }
        if let Some(params) = match_route_segments(&r.path, &url_segs) {
            return Some(SsrMatch {
                route: r.clone(),
                params,
            });
        }
    }
    None
}

/// Match ONE route's path pattern against the URL segments, returning the
/// captured params on a match. Shared by `match_ssr_route` (GET pages) and
/// `match_form_route` (non-GET handlers).
///
/// Catch-all routes encode their last segment as `*name` (required, ≥1
/// segment) or `*?name` (optional, ≥0) — it greedily consumes the remaining
/// path, joined with `/` into one param value. The marker is only honored on
/// the LAST segment; a stray `*` mid-path falls through to literal comparison.
fn match_route_segments(
    route_path: &str,
    url_segs: &[&str],
) -> Option<std::collections::HashMap<String, String>> {
    let route_segs: Vec<&str> = route_path.split('/').filter(|s| !s.is_empty()).collect();

    let catch_all = route_segs
        .last()
        .and_then(|s| s.strip_prefix('*'))
        .map(|rest| match rest.strip_prefix('?') {
            Some(name) => (name, true), // *?name → optional
            None => (rest, false),      // *name  → required
        });

    if let Some((ca_name, optional)) = catch_all {
        let prefix = &route_segs[..route_segs.len() - 1];
        let min_len = prefix.len() + if optional { 0 } else { 1 };
        if url_segs.len() < min_len {
            return None;
        }
        let mut params = std::collections::HashMap::new();
        for (rs, us) in prefix.iter().zip(url_segs.iter()) {
            if let Some(name) = rs.strip_prefix(':') {
                if us.is_empty() {
                    return None;
                }
                params.insert(name.to_string(), (*us).to_string());
            } else if rs != us {
                return None;
            }
        }
        let rest = url_segs[prefix.len()..].join("/");
        params.insert(ca_name.to_string(), rest);
        return Some(params);
    }

    if route_segs.len() != url_segs.len() {
        return None;
    }
    let mut params = std::collections::HashMap::new();
    for (rs, us) in route_segs.iter().zip(url_segs.iter()) {
        if let Some(name) = rs.strip_prefix(':') {
            if us.is_empty() {
                return None;
            }
            params.insert(name.to_string(), (*us).to_string());
        } else if rs != us {
            return None;
        }
    }
    Some(params)
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
    // #277 Stage 2: when true (GET, no query, no session cookie) this render's
    // body is teed to disk if it emits the Stage-1 `x-pylon-cacheable` proof,
    // completes cleanly (status 200, no Set-Cookie, no error), so subsequent
    // anonymous requests serve from disk. False for boundaries + authed/query
    // requests, which are never cached.
    cacheable_eligible: bool,
    // PPR Phase 0: when true (GET, no query, not dev — session cookie allowed)
    // this render's body is teed + stored under its session-presence BUCKET key
    // if it emits the `x-pylon-bucket` proof. Superset of `cacheable_eligible`.
    bucket_eligible: bool,
    // Set when the client asked for markdown (`Accept: text/markdown` or a
    // `<path>.md` URL). The render runs unchanged — same page, same props, same
    // auth — and the finished HTML is converted before it goes out. Buffered
    // rather than streamed, because the converter needs a whole document.
    md: Option<MarkdownRequest>,
) -> Result<(), Request> {
    let fn_ops = match cfg.fn_ops.as_ref() {
        Some(f) => f.clone(),
        None => return Err(request),
    };
    // Build the client bundle BEFORE dispatching the render. The Bun renderer
    // reads the bundle manifest off disk for head injection — a render racing
    // the boot-time warm (fire-and-forget thread in server.rs) finds no
    // manifest and emits HTML with NO stylesheet <link> and no hydration
    // entries. Served once, that self-heals; CACHED (revalidate /
    // auth-bucketed), it froze an unstyled shell for the full TTL — the
    // 2026-07-09 pylonsync.com launch incident, where a deploy under live
    // traffic poisoned every marketing page for an hour.
    // `warm_client_bundle` holds the outdir lock across the build, so
    // concurrent early requests serialize behind ONE build (~0.4-1.5s) and
    // all render styled. Already-warm boots pay one uncontended lock. On
    // build failure we log and fall through to the unstyled render — which
    // `maybe_cache_render` now refuses to store.
    if let Err(e) = warm_client_bundle(&fn_ops, &derive_app_dir(&cfg.ssr_routes)) {
        tracing::warn!("SSR render proceeding without client bundle (build failed): {e}");
    }
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

    // A `.md` request renders the PAGE's URL, not the `.md` one: the page's
    // `metadata.canonical`, its og:url, and its own `props.url` must all read
    // as the canonical page, or the markdown would advertise a URL that exists
    // only as a variant.
    let url = match md.as_ref() {
        Some(m) => m.render_url.clone(),
        None => request.url().to_string(),
    };
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

    // ISR cache key host dimension (see `ssr_cache_host_bucket`). Computed once
    // from this request's Host and threaded identically through the write +
    // stale-on-error paths so they key the same way the read path (in
    // `serve_frontend`) does.
    // Read off the already-lowercased map rather than the request, so this
    // agrees with `is_nav_request` on the read path AND with what the runner
    // sees — the runner switches shape off the same header.
    let nav = headers_map
        .get("x-pylon-nav")
        .map(|v| v.trim() == "1")
        .unwrap_or(false);
    let cache_vary = ssr_cache_vary(
        headers_map.get("host").map(String::as_str),
        nav,
        md.is_some(),
    );

    let params_json =
        serde_json::to_value(&matched.params).unwrap_or_else(|_| serde_json::json!({}));
    let search_params_json = parse_query_string(&query);

    // Resolve auth from the request's session cookie if a
    // SessionStore + CookieConfig are wired (the standard case for
    // pylon dev / pylon start). Without them, fall back to anonymous
    // AuthInfo — matches Phase 1 behavior.
    let auth = resolve_request_auth(cfg, &cookies_map);
    // Identity-FREE session presence for `props.session.exists` (Phase 0 auth
    // bucketing) — whether the request RESOLVED to a signed-in user, NOT who.
    // Derived from the already-resolved `auth` (free), so an expired/invalid
    // cookie is sess=0 — matching `session_authenticated` on the read path.
    let session_present = auth.user_id.is_some();
    // Bucket cache key — host + session presence. A render emitting the
    // `x-pylon-bucket` proof is stored here; the read path keys the same way.
    let bucket_vary = ssr_cache_bucket_vary(
        headers_map.get("host").map(String::as_str),
        session_present,
        nav,
        md.is_some(),
    );

    // Cold-start robustness: the Rust HTTP listener accepts connections the
    // moment it binds, but the Bun runner that executes this render boots
    // asynchronously (fresh artifact deploy, Fly cold-start from auto-stop).
    // A render landing in that window used to fail with RUNNER_NOT_STARTED →
    // a user-facing 500. Wait briefly for a warm runner so the request lands
    // on a ready one; if the pool is still not up after the bounded wait,
    // serve a retryable 503 ("starting up", auto-refresh) rather than a hard
    // 500. Static assets are served on a different path and stay fast
    // throughout. Set PYLON_SSR_RUNNER_READY_TIMEOUT_MS=0 to opt out.
    let ready_timeout = ssr_runner_ready_timeout();
    if !ready_timeout.is_zero() && !fn_ops.wait_for_runner_ready(ready_timeout) {
        tracing::warn!(
            route = %matched.route.path,
            "SSR runner not ready after {}ms",
            ready_timeout.as_millis()
        );
        // Stale-on-error: a shareable page that has EVER rendered stays up
        // through a total worker outage — serve the cached copy (even stale)
        // rather than a 503. Falls back to the warming 503 only when nothing
        // is cached for this route.
        return match try_serve_stale_on_error(
            &matched.route.path,
            &path_only,
            cacheable_eligible,
            &cache_vary,
            bucket_eligible,
            &bucket_vary,
            cors_origin,
            nav,
            md.as_ref(),
            request,
        ) {
            Ok(()) => Ok(()),
            Err(req) => {
                let _ = req.respond(ssr_warming_response(cors_origin));
                Ok(())
            }
        };
    }

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
    // Carries the render error (code, message) from the render thread to the
    // main thread for the dev error overlay. Separate from rs_tx because the
    // failure path is exactly "rs_tx dropped without a response_start" — we
    // can't piggyback the detail on the channel that just closed. recv() on
    // err_rx blocks until the render thread reaches the send (error) or drops
    // err_tx (success), so it's race-free.
    let (err_tx, err_rx) = std::sync::mpsc::sync_channel::<(String, String)>(1);

    // Absolute URL of the page, for the markdown frontmatter — captured before
    // `headers_map` moves into the render thread. A page that sets its own
    // `<link rel="canonical">` overrides this during conversion.
    let absolute_page_url: Option<String> =
        md.as_ref().map(|_| absolute_url(&headers_map, &path_only));

    let fn_ops_for_render = std::sync::Arc::clone(&fn_ops);
    let component_owned = component.clone();
    let layouts = matched.route.layouts.clone();
    let route_path_owned = matched.route.path.clone();
    let path_only_owned = path_only.clone();
    // #277 Stage 2 / Phase 0 write-tee buffer. Allocated for bucket-eligible
    // requests (the superset of cacheable-eligible); the chunk callback mirrors
    // every body byte here (push BEFORE the channel send, so the buffer is
    // complete once respond() drains the stream to EOF). None for non-eligible
    // renders → zero extra allocation.
    // A markdown response is buffered + converted on THIS thread (see below),
    // so the chunk tee would only collect HTML we are about to throw away — the
    // markdown path builds its own cache buffer from the converted bytes.
    let tee_buf: Option<std::sync::Arc<std::sync::Mutex<Vec<u8>>>> =
        if bucket_eligible && md.is_none() {
            Some(std::sync::Arc::new(std::sync::Mutex::new(Vec::new())))
        } else {
            None
        };
    let tee_for_chunk = tee_buf.clone();
    // Gate the tee on the response actually advertising a cache proof (the anon
    // `x-pylon-cacheable` or the Phase 0 `x-pylon-bucket`). The header arrives in
    // `response_start` BEFORE the first body byte (same render thread,
    // sequential), so a non-cacheable render never buffers its body.
    let should_tee = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let should_tee_rs = should_tee.clone();
    let should_tee_chunk = should_tee.clone();
    // Dev diagnostics: wall-clock around the render, so the ring + `pylon
    // diagnostics` report end-to-end render time. Dev-only (cheap Instant).
    let render_t0 = std::time::Instant::now();
    let render_thread = std::thread::Builder::new()
        .name("pylon-ssr-render".into())
        .stack_size(512 * 1024)
        .spawn(move || {
            let on_response_start: pylon_functions::runner::ResponseStartCallback = Box::new(
                move |status: u16, headers: std::collections::HashMap<String, String>| {
                    if headers.keys().any(|k| {
                        k.eq_ignore_ascii_case("x-pylon-cacheable")
                            || k.eq_ignore_ascii_case("x-pylon-bucket")
                    }) {
                        should_tee_rs.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    // Capacity-1, fires at most once — never blocks.
                    let _ = rs_tx.send((status, headers));
                },
            );
            let on_chunk: pylon_functions::runner::ByteStreamCallback =
                Box::new(move |bytes: &[u8]| {
                    // Tee to the cache buffer FIRST (push-before-send), so when
                    // the main thread observes stream EOF the buffer already
                    // holds every byte — no join race on the write path. Only
                    // when the render advertised the proof (should_tee).
                    if should_tee_chunk.load(std::sync::atomic::Ordering::Relaxed) {
                        if let Some(buf) = tee_for_chunk.as_ref() {
                            if let Ok(mut b) = buf.lock() {
                                b.extend_from_slice(bytes);
                            }
                        }
                    }
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
                session_present,
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

    let render_handle = match render_thread {
        Ok(h) => h,
        Err(e) => {
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
    };

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
            // In dev, surface the error + stack in an overlay so the developer
            // sees it immediately. In prod, prefer a stale cached copy
            // (stale-on-error) over a 500 for cacheable routes; fall back to a
            // generic 500 (detail stays in logs) only when nothing is cached.
            if is_dev_mode() {
                let detail = err_rx.recv().ok();
                let _ = request.respond(ssr_render_error_response(detail, cors_origin));
                return Ok(());
            }
            return match try_serve_stale_on_error(
                &matched.route.path,
                &path_only,
                cacheable_eligible,
                &cache_vary,
                bucket_eligible,
                &bucket_vary,
                cors_origin,
                nav,
                md.as_ref(),
                request,
            ) {
                Ok(()) => Ok(()),
                Err(req) => {
                    let _ = req.respond(ssr_render_error_response(None, cors_origin));
                    Ok(())
                }
            };
        }
    };

    // A bucket render is shareable WITHIN its session bucket only when the CDN is
    // configured to key on session-cookie presence (`PYLON_SSR_BUCKET_CDN`).
    // Without that, a bucket response stays browser-`private`/no-store (origin ISR
    // still serves it fast) so a cookie-blind shared cache can't mis-serve a
    // signed-in shell to a signed-out visitor or vice versa.
    let bucket_shareable = bucket_eligible && bucket_cdn_sharing_enabled() && !nav;
    // The markdown path replaces the streamed HTML with a converted body, and
    // hands back the bytes to cache under the markdown key (the chunk tee is
    // off for these requests).
    let md_cache_body: Option<std::sync::Arc<std::sync::Mutex<Vec<u8>>>> = match md.as_ref() {
        Some(md) => respond_markdown(
            md,
            status,
            &page_headers,
            body_rx,
            cors_origin,
            absolute_page_url.as_deref(),
            request,
        ),
        None => {
            // Data routes (sitemap/robots/llms/og-image) have no markdown
            // twin to point at — `/llms.txt.md` names nothing.
            let md_url = if nav || is_data_route_kind(matched.route.kind.as_deref()) {
                None
            } else {
                Some(crate::markdown::md_url_for(&path_only))
            };
            let response = Response::new(
                tiny_http::StatusCode(status),
                build_ssr_response_headers(
                    &page_headers,
                    cors_origin,
                    // A navigation payload answers the same URL as the page with
                    // different content. The origin keys them apart; a CDN keys on URL
                    // alone, so this must never advertise `public`.
                    cacheable_eligible && !nav,
                    bucket_shareable,
                    VariantHeaders {
                        content_type: None,
                        alternate_md: md_url.as_deref(),
                    },
                ),
                crate::server::StreamingBody::new(body_rx),
                None, // content-length unknown → tiny_http uses chunked transfer
                None,
            );
            // request.respond drains StreamingBody chunk-by-chunk as HTTP
            // chunked-transfer frames; returns at EOF (render thread done).
            let _ = request.respond(response);
            None
        }
    };

    // Dev diagnostics: record this render's verdict (from the dev-only
    // `x-pylon-dev` header the runtime emitted) + the Rust-measured render time
    // into the ring served at /_pylon/dev/diagnostics, and log one structured
    // line. The header is stripped from the client response by
    // build_ssr_response_headers; we read the raw `page_headers` here.
    if is_dev_mode() {
        let dev_header = page_headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("x-pylon-dev"))
            .map(|(_, v)| v.as_str());
        let render_ms = render_t0.elapsed().as_secs_f64() * 1000.0;
        if let Some(summary) = crate::dev_diagnostics::record_from_header(
            dev_header,
            &path_only,
            status,
            (render_ms * 10.0).round() / 10.0,
        ) {
            tracing::info!("[pylon:dev] SSR {} → {}", path_only, summary);
        }
    }

    // #277 Stage 2 / Phase 0 write-tee. The body has fully streamed to the
    // client; now persist it for subsequent requests IF this render earned it (a
    // cache proof + clean 200 + no Set-Cookie + no mid-render error). The anon
    // proof (`x-pylon-cacheable`) stores under `cache_vary`; the bucket proof
    // (`x-pylon-bucket`) stores under the session-keyed `bucket_vary`. Off the
    // hot path; best-effort. When not eligible the handle is simply dropped.
    if let Some(buf) = tee_buf.or(md_cache_body) {
        maybe_cache_render(
            &matched.route.path,
            &path_only,
            cacheable_eligible,
            &cache_vary,
            &bucket_vary,
            status,
            &page_headers,
            buf,
            render_handle,
            err_rx,
        );
    }
    Ok(())
}

/// Absolute URL for `path`, from the request's forwarding headers. Used only
/// for the markdown frontmatter, so a missing/odd Host degrades to a
/// path-relative value rather than failing the response.
fn absolute_url(headers: &std::collections::HashMap<String, String>, path: &str) -> String {
    let host = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get("host"))
        .map(|h| h.trim())
        .filter(|h| !h.is_empty());
    let host = match host {
        Some(h) => h,
        None => return path.to_string(),
    };
    // Behind a proxy the client's scheme is the forwarded one; direct-to-Pylon
    // in dev is plain HTTP on localhost.
    let scheme = headers
        .get("x-forwarded-proto")
        .map(|p| p.split(',').next().unwrap_or("https").trim().to_string())
        .unwrap_or_else(|| {
            if is_loopback_host(host) {
                "http".to_string()
            } else {
                "https".to_string()
            }
        });
    format!("{scheme}://{host}{path}")
}

/// Buffer a finished render, convert it to markdown, and send it.
///
/// Returns the bytes to store in the SSR cache under the markdown key — the
/// CONVERTED body, so a cache hit skips both the render and the conversion.
/// `None` when this response must not be cached (an opt-out fallback, a 406, or
/// a page that answered with its own non-HTML content type).
#[allow(clippy::too_many_arguments)]
fn respond_markdown(
    md: &MarkdownRequest,
    status: u16,
    page_headers: &std::collections::HashMap<String, String>,
    body_rx: std::sync::mpsc::Receiver<Vec<u8>>,
    cors_origin: &str,
    absolute_url: Option<&str>,
    request: Request,
) -> Option<std::sync::Arc<std::sync::Mutex<Vec<u8>>>> {
    // Drain the render to EOF. The converter needs a whole document — there is
    // no partial-markdown state to stream, and an agent reading a page is not
    // waiting on first-paint.
    let mut html = Vec::new();
    while let Ok(chunk) = body_rx.recv() {
        html.extend_from_slice(&chunk);
    }

    // A page can decline to be read as markdown (`export const markdown =
    // false`) — an app shell whose value is the interaction, not the prose.
    let opted_out = page_headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("x-pylon-md") && v.trim() == "0");
    // A page that set its own non-HTML content type already answered in a
    // format of its choosing; converting that would be destroying data.
    let page_content_type = page_headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.to_ascii_lowercase());
    let page_answered_non_html = page_content_type
        .as_deref()
        .is_some_and(|ct| !ct.starts_with("text/html"));

    if opted_out || page_answered_non_html {
        if md.explicit_url {
            // `/x.md` names a resource that does not exist for this page.
            let body = format!(
                "# 404 Not Found\n\n`{}` has no markdown representation.\n\nRead it at `{}` instead.\n",
                crate::markdown::md_url_for(md.render_url.split('?').next().unwrap_or("/")),
                md.render_url,
            );
            let mut resp = Response::from_data(body.into_bytes()).with_status_code(404);
            for (name, value) in [
                ("Content-Type", "text/markdown; charset=utf-8"),
                ("Vary", "Accept"),
                ("Cache-Control", "no-store"),
                ("X-Content-Type-Options", "nosniff"),
            ] {
                if let Ok(h) = Header::from_bytes(name, value) {
                    resp = resp.with_header(h);
                }
            }
            if let Ok(h) = Header::from_bytes("Access-Control-Allow-Origin", cors_origin.as_bytes())
            {
                resp = resp.with_header(h);
            }
            let _ = request.respond(resp);
            return None;
        }
        if !md.html_acceptable {
            let path = md.render_url.split('?').next().unwrap_or("/");
            let _ = request.respond(not_acceptable_response(path, false, cors_origin));
            return None;
        }
        // The client would read HTML too, and we already have it — send the
        // render we just buffered rather than making the client ask twice.
        let mut resp = Response::from_data(html).with_status_code(status);
        for h in build_ssr_response_headers(
            page_headers,
            cors_origin,
            false,
            false,
            VariantHeaders::default(),
        ) {
            resp = resp.with_header(h);
        }
        let _ = request.respond(resp);
        return None;
    }

    let source = String::from_utf8_lossy(&html);
    let markdown = crate::markdown::html_to_markdown(&source, absolute_url);
    let bytes = markdown.into_bytes();
    let cache_copy = std::sync::Arc::new(std::sync::Mutex::new(bytes.clone()));
    let mut resp = Response::from_data(bytes).with_status_code(status);
    for h in build_ssr_response_headers(
        page_headers,
        cors_origin,
        // Markdown bodies are never advertised as shared-cacheable at the CDN:
        // a cache keyed on URL alone (Cloudflare ignores Vary) would replay
        // them to browsers asking for HTML. The origin's own ISR entry — keyed
        // on the variant — still skips the render.
        false,
        false,
        VariantHeaders {
            content_type: Some(md.representation.content_type()),
            alternate_md: None,
        },
    ) {
        resp = resp.with_header(h);
    }
    let _ = request.respond(resp);
    Some(cache_copy)
}

/// Lowercase `host[:port]` from a bare host or a full URL; `""` when empty.
/// Mirrors `hostOf()` in ssr-runtime.ts so the Rust cache key agrees with the
/// origin the TS render bakes into og:image / canonical URLs.
fn host_of(value: &str) -> String {
    let t = value.trim();
    if t.is_empty() {
        return String::new();
    }
    let host = match t.find("://") {
        // scheme://host/path… → take the authority up to the next '/'.
        Some(i) => t[i + 3..].split('/').next().unwrap_or("").to_string(),
        None => t.trim_matches('/').to_string(),
    };
    host.to_ascii_lowercase()
}

/// Loopback host? Mirrors `LOOPBACK_HOST` in ssr-runtime.ts:
/// `/^(localhost|127\.|\[?::1|0\.0\.0\.0)/`.
fn is_loopback_host(host: &str) -> bool {
    host.starts_with("localhost")
        || host.starts_with("127.")
        || host.starts_with("::1")
        || host.starts_with("[::1")
        || host.starts_with("0.0.0.0")
}

/// The cache-key "host bucket" for an SSR render.
///
/// A cacheable (force-static / `revalidate`) render bakes an ABSOLUTE origin
/// into its og:image + canonical URLs. `resolveOrigin` (ssr-runtime.ts) derives
/// that origin from the request `Host` ONLY when the host is allowlisted (the
/// `PYLON_PUBLIC_URL` host, `PYLON_CANONICAL_HOST`, a `PYLON_TRUSTED_HOSTS`
/// entry, or loopback); any other Host is clamped to the configured public
/// origin. So the cached HTML varies by the *trusted* host, and the on-disk ISR
/// key MUST include that dimension — otherwise on a multi-trusted-host deploy
/// (e.g. apex + www served from one app) the first host to render bakes its
/// absolute URLs into the shared entry and every other trusted host serves them.
///
/// Every UNtrusted host collapses to the same `""` bucket: they all render
/// identical canonical-origin URLs, so they share one entry AND an attacker
/// spraying arbitrary `Host` headers can't multiply cache entries. Only the
/// finite set of operator-configured trusted hosts gets its own bucket.
///
/// MUST stay in sync with `resolveOrigin`'s allowlist in
/// packages/functions/src/ssr-runtime.ts (same env vars + loopback rule).
fn ssr_cache_host_bucket(request_host: Option<&str>) -> String {
    let host = host_of(request_host.unwrap_or(""));
    if host.is_empty() {
        return String::new();
    }
    if is_loopback_host(&host) {
        return host;
    }
    let mut allow: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut add = |v: &str| {
        let h = host_of(v);
        if !h.is_empty() {
            allow.insert(h);
        }
    };
    if let Ok(v) = std::env::var("PYLON_PUBLIC_URL") {
        add(&v);
    }
    if let Ok(v) = std::env::var("PYLON_CANONICAL_HOST") {
        add(&v);
    }
    if let Ok(v) = std::env::var("PYLON_TRUSTED_HOSTS") {
        for part in v.split(',') {
            add(part);
        }
    }
    if allow.contains(&host) {
        host
    } else if crate::tenant_hosts::is_trusted_host(&host) {
        // Platform (tenant) custom domain — trusted dynamically from the control
        // plane, so the SSR path treats the tenant's own hostname as a first-
        // class origin (its own cache bucket) without a per-domain restart.
        host
    } else {
        String::new()
    }
}

/// Does this request want the navigation payload (JSON) instead of the page
/// (HTML)? Set by the client runtime on a client-side navigation, which
/// re-renders from data and throws the markup away.
///
/// A header rather than a query string: the cache read path bypasses anything
/// with a query, so `?__nav=1` would silently disable ISR for every
/// navigation.
pub fn is_nav_request(request: &Request) -> bool {
    request.headers().iter().any(|h| {
        h.field
            .as_str()
            .as_str()
            .eq_ignore_ascii_case("x-pylon-nav")
            && h.value.as_str().trim() == "1"
    })
}

/// Cache-key `vary` for a render, derived from its request `Host` (see
/// `ssr_cache_host_bucket`) plus the response SHAPE. Threaded identically
/// through the read, write, and stale-on-error paths so their keys agree.
///
/// SECURITY: `nav` is part of the key, not a detail. The two shapes answer the
/// same URL with different content types — HTML for a document load, JSON for
/// a client-side navigation — so sharing a key would let one be served where
/// the other is expected: a navigation rendering a raw JSON blob, or a
/// crawler being handed a payload instead of a page.
fn ssr_cache_vary(request_host: Option<&str>, nav: bool, markdown: bool) -> Vec<(String, String)> {
    let mut vary = vec![
        ("host".to_string(), ssr_cache_host_bucket(request_host)),
        ("nav".to_string(), if nav { "1" } else { "0" }.to_string()),
    ];
    // Same reasoning as `nav`: the markdown representation is a different
    // answer to the same URL. Only added when markdown, so every existing HTML
    // entry keeps its key (no cache-wide invalidation on upgrade).
    if markdown {
        vary.push(("var".to_string(), "md".to_string()));
    }
    vary
}

/// PPR Phase 0 cache-key `vary` for an auth-BUCKET render: the host dimension
/// PLUS a `("sess", "1"|"0")` pair for session-cookie PRESENCE. A signed-in and
/// a signed-out request therefore key to DISTINCT entries (different shells),
/// while two different signed-in users key to the SAME entry — which is correct
/// precisely because a bucketable render is identity-free (it never read real
/// `auth`). Distinct from `ssr_cache_vary` so anon and bucket entries for the
/// same route never collide.
fn ssr_cache_bucket_vary(
    request_host: Option<&str>,
    session_present: bool,
    nav: bool,
    markdown: bool,
) -> Vec<(String, String)> {
    let mut vary = vec![
        ("host".to_string(), ssr_cache_host_bucket(request_host)),
        (
            "sess".to_string(),
            if session_present { "1" } else { "0" }.to_string(),
        ),
        ("nav".to_string(), if nav { "1" } else { "0" }.to_string()),
    ];
    if markdown {
        vary.push(("var".to_string(), "md".to_string()));
    }
    vary
}

/// PPR Phase 0: is the CDN configured to key its cache on session-cookie
/// PRESENCE (a Cloudflare cache-key rule / Worker)? Only then may a bucket
/// response advertise `public, s-maxage` — otherwise a cookie-blind shared cache
/// would mis-serve a signed-in shell to a signed-out visitor (and vice versa).
/// Default OFF (safe): buckets stay browser-`private`/no-store and the win is the
/// origin ISR render-skip. Operators flip `PYLON_SSR_BUCKET_CDN=1` after the CDN
/// rule is in place.
fn bucket_cdn_sharing_enabled() -> bool {
    std::env::var("PYLON_SSR_BUCKET_CDN").ok().as_deref() == Some("1")
}

/// The request `Host` header value, if present.
fn request_host(request: &Request) -> Option<String> {
    request.headers().iter().find_map(|h| {
        if h.field.as_str().as_str().eq_ignore_ascii_case("host") {
            Some(h.value.as_str().to_string())
        } else {
            None
        }
    })
}

/// #277 Stage 2: the host-edge shareability verdict. Returns `Some(revalidate
/// secs)` only when the render emitted the Stage-1 anonymity proof
/// (`x-pylon-cacheable: N`, N>0) AND the response is a clean, shareable 200
/// with no Set-Cookie. `None` otherwise — fail-closed. Pure (header map in,
/// verdict out) so the security-critical gate is directly unit-tested.
fn ssr_cache_verdict(
    status: u16,
    page_headers: &std::collections::HashMap<String, String>,
) -> Option<u64> {
    ssr_proof_verdict(status, page_headers, "x-pylon-cacheable")
}

/// PPR Phase 0: the host-edge BUCKET verdict — `Some(revalidate secs)` only when
/// the render emitted the bucket proof (`x-pylon-bucket: N`, N>0) AND the
/// response is a clean 200 with no Set-Cookie. Same fail-closed shape as
/// `ssr_cache_verdict`, different proof header. Pure.
fn ssr_bucket_verdict(
    status: u16,
    page_headers: &std::collections::HashMap<String, String>,
) -> Option<u64> {
    ssr_proof_verdict(status, page_headers, "x-pylon-bucket")
}

/// Shared fail-closed gate behind `ssr_cache_verdict` / `ssr_bucket_verdict`:
/// the named internal proof header parses to N>0, status is 200, and the render
/// set no cookie. A Set-Cookie can never ride a shared/bucket entry.
fn ssr_proof_verdict(
    status: u16,
    page_headers: &std::collections::HashMap<String, String>,
    proof_header: &str,
) -> Option<u64> {
    let secs = page_headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(proof_header))
        .and_then(|(_, v)| v.trim().parse::<u64>().ok())
        .filter(|s| *s > 0)?;
    if status != 200 {
        return None;
    }
    if page_headers
        .keys()
        .any(|k| k.eq_ignore_ascii_case("set-cookie"))
    {
        return None;
    }
    Some(secs)
}

/// Which cache lane a finished render writes to.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum CacheWriteLane {
    /// Shared anonymous entry (`x-pylon-cacheable`), keyed path+host.
    Anon,
    /// Session-presence bucket (`x-pylon-bucket`), keyed path+host+sess.
    Bucket,
}

/// Pure write decision: given the request's anon-eligibility and the finished
/// render's status+headers, which lane (if any) may store it, and for how long?
///
/// SECURITY (P0, codex 2026-06-28): the bucket proof writes regardless of
/// `cacheable_eligible` (its body is identity-free + the bucket key separates
/// signed-in/out). The ANON proof writes ONLY when `cacheable_eligible` — a
/// signed-in request can legitimately emit `x-pylon-cacheable` (it never read
/// auth/session), but its NON-bucket hydration tail carries the user's REAL
/// identity, so storing it under the shared anon key would replay that identity
/// to anonymous visitors. Fail-closed: no proof / wrong eligibility → None.
fn cache_write_plan(
    cacheable_eligible: bool,
    status: u16,
    page_headers: &std::collections::HashMap<String, String>,
) -> Option<(u64, CacheWriteLane)> {
    if let Some(s) = ssr_bucket_verdict(status, page_headers) {
        return Some((s, CacheWriteLane::Bucket));
    }
    if cacheable_eligible {
        if let Some(s) = ssr_cache_verdict(status, page_headers) {
            return Some((s, CacheWriteLane::Anon));
        }
    }
    None
}

/// #277 Stage 2: persist a finished render to the on-disk ISR cache, but only
/// when it provably earned it. Fail-closed at every gate — a render that read
/// auth, set a cookie, returned non-200, or errored mid-body is never stored.
fn maybe_cache_render(
    route_path: &str,
    pathname: &str,
    // True when THIS request is cookie-anonymous (the anon read gate). The anon
    // proof may only be STORED for such a request — a signed-in request can emit
    // `x-pylon-cacheable` (it never read auth/session), but its NON-bucket
    // hydration tail carries the user's REAL identity, so storing it under the
    // shared anon key would replay that identity to anonymous visitors. The
    // bucket proof has no such restriction (its tail is anonymized).
    cacheable_eligible: bool,
    anon_vary: &[(String, String)],
    bucket_vary: &[(String, String)],
    status: u16,
    page_headers: &std::collections::HashMap<String, String>,
    buf: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    render_handle: std::thread::JoinHandle<()>,
    err_rx: std::sync::mpsc::Receiver<(String, String)>,
) {
    // Which proof did the render earn, and under which key? Pure decision (see
    // `cache_write_plan`) — fail-closed, and unit-tested for the P0 where a
    // signed-in request's anon proof must NOT write the shared anon entry.
    let (revalidate_secs, vary) = match cache_write_plan(cacheable_eligible, status, page_headers) {
        Some((s, CacheWriteLane::Bucket)) => (s, bucket_vary),
        Some((s, CacheWriteLane::Anon)) => (s, anon_vary),
        None => return,
    };
    // Never store a render made without a built client bundle: its HTML lacks
    // the stylesheet <link> + hydration entries (the bundler manifest wasn't
    // on disk when the Bun renderer read it), and caching one freezes an
    // unstyled shell for the full revalidate window (the 2026-07-09
    // pylonsync.com launch incident). serve_via_ssr_rpc builds the bundle
    // before dispatch, so this only fires on its build-failed fallthrough —
    // which must stay servable, but never cacheable.
    if cached_bundle_outdir()
        .lock()
        .map(|c| c.is_none())
        .unwrap_or(true)
    {
        tracing::debug!(
            route = %route_path,
            "SSR cache write skipped: client bundle not built"
        );
        return;
    }
    // Wait for the render thread, then confirm it didn't error AFTER
    // response_start — a partial/aborted body must never be cached. respond()
    // already drained to EOF, so the join returns immediately.
    let _ = render_handle.join();
    if err_rx.try_recv().is_ok() {
        return;
    }
    // Single-flight: skip if another request is already writing this key.
    let _claim = match crate::ssr_cache::try_claim_write(route_path, pathname, vary) {
        Some(c) => c,
        None => return,
    };
    let body = match buf.lock() {
        Ok(b) => b.clone(),
        Err(_) => return,
    };
    // Store the raw page headers (incl. the proof) so the serve path rebuilds
    // a byte-equivalent response via build_ssr_response_headers.
    let headers_vec: Vec<(String, String)> = page_headers
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    crate::ssr_cache::put(
        route_path,
        pathname,
        vary,
        status,
        &headers_vec,
        &body,
        revalidate_secs,
    );
    tracing::debug!(
        route = %route_path,
        pathname = %pathname,
        ttl = revalidate_secs,
        bytes = body.len(),
        "SSR cache write"
    );
}

/// #277 Stage 2: serve a fresh cache entry straight off disk, skipping the Bun
/// render. Rebuilds headers via `build_ssr_response_headers` from the stored
/// page headers so the response matches a live cacheable render exactly (the
/// internal `x-pylon-cacheable` proof is stripped there; the public/s-maxage
/// Cache-Control + security defaults are re-applied).
fn serve_cached_ssr(
    entry: crate::ssr_cache::CacheEntry,
    cors_origin: &str,
    nav: bool,
    md: Option<&MarkdownRequest>,
    route_kind: Option<&str>,
    request: Request,
) -> Result<(), Request> {
    let page_headers: std::collections::HashMap<String, String> =
        entry.headers.into_iter().collect();
    let md_url = html_alternate_md_url(md, nav, route_kind, &request);
    let variant = VariantHeaders {
        content_type: md.map(|m| m.representation.content_type()),
        alternate_md: md_url.as_deref(),
    };
    let mut resp = Response::from_data(entry.body).with_status_code(entry.status);
    // Reached only for cookie-anonymous, eligible requests (the cache READ gate),
    // and the stored body is the anonymous render → safe to advertise `public`.
    // EXCEPT for a navigation payload: the origin keys HTML and JSON apart, but
    // a CDN keys on URL alone, so advertising `public` on the JSON would let it
    // be cached at the page's URL and then served to a real document request.
    for h in build_ssr_response_headers(&page_headers, cors_origin, !nav, false, variant) {
        resp = resp.with_header(h);
    }
    let _ = request.respond(resp);
    Ok(())
}

/// PPR Phase 0: serve a fresh BUCKET cache entry off disk, skipping the Bun
/// render. The stored body is identity-free (only the binary signed-in bit), and
/// the read gate already matched this request's session bucket. Re-derives
/// headers from the stored `x-pylon-bucket` proof: browser-`private`/no-store by
/// default, or `public, s-maxage` when the CDN keys on session presence
/// (`bucket_shareable`). Not anon-shareable (the request may carry a session
/// cookie), so `request_shareable=false`.
fn serve_cached_bucket_ssr(
    entry: crate::ssr_cache::CacheEntry,
    cors_origin: &str,
    nav: bool,
    md: Option<&MarkdownRequest>,
    route_kind: Option<&str>,
    request: Request,
) -> Result<(), Request> {
    let page_headers: std::collections::HashMap<String, String> =
        entry.headers.into_iter().collect();
    let md_url = html_alternate_md_url(md, nav, route_kind, &request);
    let variant = VariantHeaders {
        content_type: md.map(|m| m.representation.content_type()),
        alternate_md: md_url.as_deref(),
    };
    let mut resp = Response::from_data(entry.body).with_status_code(entry.status);
    for h in build_ssr_response_headers(
        &page_headers,
        cors_origin,
        false,
        // Never hand a navigation payload to a shared cache keyed on URL alone.
        bucket_cdn_sharing_enabled() && !nav,
        variant,
    ) {
        resp = resp.with_header(h);
    }
    let _ = request.respond(resp);
    Ok(())
}

/// Reliability: serve the last good ISR render — even if STALE — when the live
/// render can't produce a response (the Bun worker pool is down / wedged /
/// still cold-booting, or the render failed before emitting a head). This
/// decouples page availability from worker health: a shareable page that has
/// EVER been rendered stays up through a total worker outage instead of 503/
/// 500-ing. Only applies to cacheable-eligible requests (GET, no query, no
/// session cookie) — the only ones that have a stored, provably-anonymous
/// entry. Returns `Ok(())` if a stale copy was served, or `Err(request)` (the
/// request untouched) so the caller falls back to its normal error response.
fn try_serve_stale_on_error(
    route_path: &str,
    pathname: &str,
    cacheable_eligible: bool,
    vary: &[(String, String)],
    bucket_eligible: bool,
    bucket_vary: &[(String, String)],
    cors_origin: &str,
    nav: bool,
    // The variant keys are already markdown-specific (see `ssr_cache_vary`), so
    // this only decides the Content-Type of whatever the lookup found.
    md: Option<&MarkdownRequest>,
    request: Request,
) -> Result<(), Request> {
    // Prefer an anon entry; fall back to this request's session bucket. Both are
    // identity-free, so either is safe to serve as a stale fallback.
    let candidate = stale_on_error_candidate(route_path, pathname, cacheable_eligible, vary)
        .or_else(|| {
            if bucket_eligible {
                crate::ssr_cache::get(route_path, pathname, bucket_vary)
            } else {
                None
            }
        });
    match candidate {
        Some(entry) => {
            tracing::warn!(
                route = %route_path,
                pathname = %pathname,
                fresh = entry.fresh,
                "SSR render unavailable — serving cached copy (stale-on-error)"
            );
            serve_cached_ssr_stale(entry, cors_origin, nav, md, request)
        }
        None => Err(request),
    }
}

/// The cached render eligible to answer a render-failure, or `None` to fall
/// through to the 503/500. KEY PROPERTY: a STALE entry (past its revalidate
/// window) is still returned — stale-on-error deliberately prefers a slightly
/// old page over an error. Only cacheable-eligible requests (GET, no query, no
/// session cookie) have a stored, provably-anonymous entry to serve.
fn stale_on_error_candidate(
    route_path: &str,
    pathname: &str,
    cacheable_eligible: bool,
    vary: &[(String, String)],
) -> Option<crate::ssr_cache::CacheEntry> {
    if !cacheable_eligible {
        return None;
    }
    crate::ssr_cache::get(route_path, pathname, vary)
}

/// Serve a cached entry as a stale-on-error fallback. Like `serve_cached_ssr`
/// but (1) replaces the stored long-lived `Cache-Control` with a SHORT one — a
/// stale-error response must not be cached at the edge as if fresh, or a
/// recovered worker's output wouldn't surface — while still letting CloudFlare
/// absorb a thundering herd during the outage, and (2) stamps
/// `X-Pylon-Cache: stale-on-error` for observability.
fn serve_cached_ssr_stale(
    entry: crate::ssr_cache::CacheEntry,
    cors_origin: &str,
    nav: bool,
    md: Option<&MarkdownRequest>,
    request: Request,
) -> Result<(), Request> {
    let page_headers: std::collections::HashMap<String, String> =
        entry.headers.into_iter().collect();
    // A bucket entry (session-presence keyed) must NOT be served `public` to a
    // cookie-blind shared cache unless the CDN keys on session presence — else it
    // would mis-serve a signed-in shell to a signed-out visitor. The origin ISR
    // serves each request from this entry anyway (the worker is down, but disk is
    // up), so `private, no-store` loses nothing here.
    let is_bucket = page_headers
        .keys()
        .any(|k| k.eq_ignore_ascii_case("x-pylon-bucket"));
    let mut resp = Response::from_data(entry.body).with_status_code(entry.status);
    for h in build_ssr_response_headers(
        &page_headers,
        cors_origin,
        !nav,
        false,
        VariantHeaders {
            content_type: md.map(|m| m.representation.content_type()),
            alternate_md: None,
        },
    ) {
        // Drop the stored Cache-Control; we re-apply a short one below.
        if h.field
            .as_str()
            .as_str()
            .eq_ignore_ascii_case("cache-control")
        {
            continue;
        }
        resp = resp.with_header(h);
    }
    // A navigation payload is never shareable: a CDN keys on URL alone and
    // would serve this JSON to a document request.
    let stale_cc = if nav || (is_bucket && !bucket_cdn_sharing_enabled()) {
        "private, no-store"
    } else {
        "public, max-age=10, stale-while-revalidate=30"
    };
    if let Ok(h) = Header::from_bytes("Cache-Control", stale_cc) {
        resp = resp.with_header(h);
    }
    if let Ok(h) = Header::from_bytes("X-Pylon-Cache", "stale-on-error") {
        resp = resp.with_header(h);
    }
    let _ = request.respond(resp);
    Ok(())
}

/// Serve a non-GET request matched to a `route.ts` form/method handler (#276).
/// Reads + parses the request body, invokes the handler in the Bun runtime via
/// `handle_form`, and writes the response it shapes through `SsrResponse`
/// (usually a 303 redirect — POST-redirect-GET — plus any Set-Cookie). Mirrors
/// `serve_via_ssr_rpc`'s response plumbing; the body is read up front (a form
/// POST carries one) and the handler may WRITE to the DB. CSRF was enforced
/// upstream by the CsrfPlugin before this is reached.
fn serve_via_form_rpc(
    cfg: &FrontendConfig,
    matched: SsrMatch,
    mut request: Request,
    cors_origin: &str,
) -> Result<(), Request> {
    let fn_ops = match cfg.fn_ops.as_ref() {
        Some(f) => f.clone(),
        None => return Err(request),
    };
    let component = match matched.route.component.as_deref() {
        Some(c) => c.to_string(),
        None => return Err(request),
    };
    let method = match request.method() {
        // GET reaches here only via the raw-route dispatch below (a `route.ts`
        // exporting `GET`) — the Bun runtime streams that handler's returned
        // body verbatim. POST/PUT/PATCH/DELETE are the form/mutation handlers.
        Method::Get => "GET",
        Method::Post => "POST",
        Method::Put => "PUT",
        Method::Patch => "PATCH",
        Method::Delete => "DELETE",
        _ => return Err(request),
    }
    .to_string();

    let url = request.url().to_string();
    let (path_only, query) = match url.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (url.clone(), String::new()),
    };

    // Headers + cookies (immutable borrow) BEFORE reading the body (mutable).
    let mut headers_map: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut cookies_map: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut content_type = String::new();
    for h in request.headers() {
        let name = h.field.as_str().as_str().to_ascii_lowercase();
        let value = h.value.as_str().to_string();
        if name == "cookie" {
            for pair in value.split(';') {
                if let Some((k, v)) = pair.trim().split_once('=') {
                    cookies_map.insert(k.to_string(), v.to_string());
                }
            }
        }
        if name == "content-type" {
            content_type = value.to_ascii_lowercase();
        }
        headers_map.insert(name, value);
    }

    // Read the body, capped — a form POST body is bounded; reject oversized.
    const MAX_FORM_BODY: usize = 1024 * 1024; // 1 MiB
    let mut body: Vec<u8> = Vec::new();
    {
        use std::io::Read;
        let mut limited = request.as_reader().take((MAX_FORM_BODY as u64) + 1);
        if limited.read_to_end(&mut body).is_err() {
            let _ = request.respond(form_text_response(
                400,
                "could not read request body",
                cors_origin,
            ));
            return Ok(());
        }
    }
    if body.len() > MAX_FORM_BODY {
        let _ = request.respond(form_text_response(413, "form body too large", cors_origin));
        return Ok(());
    }

    // Parse the body. urlencoded (the browser default for `<form method=post>`)
    // is fully supported. multipart/form-data is STAGED — file uploads go
    // through the existing /api/files path; reject with a clear 415 rather than
    // silently dropping fields.
    let form_json = if content_type.starts_with("application/x-www-form-urlencoded") {
        parse_urlencoded_form(&body)
    } else if content_type.starts_with("multipart/form-data") {
        let _ = request.respond(form_text_response(
            415,
            "multipart/form-data isn't supported yet — use application/x-www-form-urlencoded, \
             or upload files via /api/files",
            cors_origin,
        ));
        return Ok(());
    } else if body.is_empty() || content_type.starts_with("application/json") {
        // A JSON body has no form fields to parse. Running it through the
        // urlencoded parser produced one nonsense key per `&`-free blob, which
        // read as "the fields are there, just wrong". The handler reads
        // `req.body` instead.
        serde_json::json!({})
    } else {
        // Unknown content-type with a body — best-effort urlencoded.
        parse_urlencoded_form(&body)
    };
    // The exact bytes, for handlers that answer machines: JSON APIs, JSON-RPC
    // (MCP), and webhooks that verify a signature over the raw body.
    let raw_body = String::from_utf8_lossy(&body).to_string();

    let params_json =
        serde_json::to_value(&matched.params).unwrap_or_else(|_| serde_json::json!({}));
    let search_params_json = parse_query_string(&query);
    let auth = resolve_request_auth(cfg, &cookies_map);

    // Same cold-start runner-readiness wait as serve_via_ssr_rpc — a route
    // handler hit during the boot window shouldn't 500 either.
    let ready_timeout = ssr_runner_ready_timeout();
    if !ready_timeout.is_zero() && !fn_ops.wait_for_runner_ready(ready_timeout) {
        tracing::warn!(
            "route handler runner not ready after {}ms; serving retryable 503",
            ready_timeout.as_millis()
        );
        let _ = request.respond(ssr_warming_response(cors_origin));
        return Ok(());
    }

    // Same head/body/error channel plumbing as serve_via_ssr_rpc.
    let (rs_tx, rs_rx) =
        std::sync::mpsc::sync_channel::<(u16, std::collections::HashMap<String, String>)>(1);
    let (body_tx, body_rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(64);
    let streaming_body = crate::server::StreamingBody::new(body_rx);
    let (err_tx, err_rx) = std::sync::mpsc::sync_channel::<(String, String)>(1);

    let fn_ops_for_form = std::sync::Arc::clone(&fn_ops);
    let component_owned = component.clone();
    let route_path_owned = matched.route.path.clone();
    let path_only_owned = path_only.clone();
    let form_thread = std::thread::Builder::new()
        .name("pylon-ssr-form".into())
        .stack_size(512 * 1024)
        .spawn(move || {
            let on_response_start: pylon_functions::runner::ResponseStartCallback = Box::new(
                move |status: u16, headers: std::collections::HashMap<String, String>| {
                    let _ = rs_tx.send((status, headers));
                },
            );
            let on_chunk: pylon_functions::runner::ByteStreamCallback =
                Box::new(move |bytes: &[u8]| {
                    let _ = body_tx.send(bytes.to_vec());
                });
            let result = fn_ops_for_form.handle_form(
                &component_owned,
                &route_path_owned,
                &method,
                &path_only_owned,
                params_json,
                search_params_json,
                form_json,
                raw_body,
                headers_map,
                cookies_map,
                auth,
                Some(on_response_start),
                on_chunk,
            );
            if let Err(e) = result {
                tracing::error!(code = %e.code, message = %e.message, "form handler failed");
                let _ = err_tx.send((e.code.clone(), e.message.clone()));
            }
        });

    if let Err(e) = form_thread {
        tracing::error!(error = ?e, "failed to spawn form handler thread");
        let detail =
            is_dev_mode().then(|| ("SSR_FORM_THREAD_SPAWN_FAILED".to_string(), format!("{e}")));
        let _ = request.respond(ssr_render_error_response(detail, cors_origin));
        return Ok(());
    }

    let (status, page_headers) = match rs_rx.recv() {
        Ok(v) => v,
        Err(_) => {
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
        // Form-handler (POST) path — never a shareable GET, so never `public`
        // (anon or bucket). A route handler sets its own content type, so the
        // variant carries nothing.
        build_ssr_response_headers(
            &page_headers,
            cors_origin,
            false,
            false,
            VariantHeaders::default(),
        ),
        streaming_body,
        None,
        None,
    );
    let _ = request.respond(response);
    Ok(())
}

/// Parse an `application/x-www-form-urlencoded` body into a JSON object:
/// `name → value` (single) or `name → [values]` (repeated fields), matching
/// the URLSearchParams get/getAll semantics the TS `FormFields` exposes.
fn parse_urlencoded_form(body: &[u8]) -> serde_json::Value {
    let s = String::from_utf8_lossy(body);
    let mut order: Vec<String> = Vec::new();
    let mut map: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for pair in s.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = match pair.split_once('=') {
            Some((k, v)) => (percent_decode(k), percent_decode(v)),
            None => (percent_decode(pair), String::new()),
        };
        if !map.contains_key(&k) {
            order.push(k.clone());
        }
        map.entry(k).or_default().push(v);
    }
    let mut obj = serde_json::Map::new();
    for k in order {
        let mut vals = map.remove(&k).unwrap_or_default();
        let val = if vals.len() == 1 {
            serde_json::Value::String(vals.pop().unwrap())
        } else {
            serde_json::Value::Array(vals.into_iter().map(serde_json::Value::String).collect())
        };
        obj.insert(k, val);
    }
    serde_json::Value::Object(obj)
}

/// A small text/plain response for the form path's pre-handler errors
/// (400 / 413 / 415), with CORS + no-store.
fn form_text_response(
    status: u16,
    body: &str,
    cors_origin: &str,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let mut resp = Response::from_data(body.as_bytes().to_vec())
        .with_status_code(status)
        .with_header(Header::from_bytes("Content-Type", "text/plain; charset=utf-8").unwrap())
        .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap());
    if let Ok(h) = Header::from_bytes("Access-Control-Allow-Origin", cors_origin.as_bytes()) {
        resp = resp.with_header(h);
    }
    resp
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
/// The `.md` URL to advertise on an HTML page response — `None` when this
/// response IS the markdown, or when it's a client-router navigation payload
/// (no document, nothing to offer an alternate for).
fn html_alternate_md_url(
    md: Option<&MarkdownRequest>,
    nav: bool,
    route_kind: Option<&str>,
    request: &Request,
) -> Option<String> {
    if md.is_some() || nav || is_data_route_kind(route_kind) {
        return None;
    }
    let url = request.url();
    let path = url.split('?').next().unwrap_or("/");
    Some(crate::markdown::md_url_for(path))
}

/// Representation metadata for one SSR response: what content type it actually
/// carries (set only when the runtime converted the body), and the `.md` URL to
/// advertise alongside an HTML page.
#[derive(Debug, Clone, Copy, Default)]
struct VariantHeaders<'a> {
    content_type: Option<&'static str>,
    alternate_md: Option<&'a str>,
}

fn build_ssr_response_headers(
    page_headers: &std::collections::HashMap<String, String>,
    cors_origin: &str,
    // Whether THIS request may receive a shared-cacheable (`public`) response —
    // i.e. it is itself eligible (GET, no query, NO session cookie, not dev).
    // The #277 render proof says "this render is anonymous-safe"; it does NOT say
    // "this request is anonymous". A logged-in request still carries the user's
    // real `auth` in the hydration tail, so emitting `public` for it would let a
    // shared cache replay one user's identity to another. Both must hold.
    request_shareable: bool,
    // PPR Phase 0: whether THIS request may receive a shared-WITHIN-BUCKET
    // (`public, s-maxage`) response for a `x-pylon-bucket` render — true only when
    // the request is bucket-eligible AND the CDN keys on session-cookie presence
    // (`PYLON_SSR_BUCKET_CDN`). When false, a bucket render stays browser-`private`
    // so a cookie-blind shared cache can't mis-serve a signed-in shell to a
    // signed-out visitor. A bucket body is identity-free (see `computeBucketVerdict`)
    // so this is about cache-KEYING, not body safety.
    bucket_shareable: bool,
    // Which representation this response carries, and where the other one
    // lives. Every SSR response gets `Vary: Accept` from this — the body now
    // depends on the request's Accept, and a cache that doesn't know that will
    // hand an agent the HTML (or a browser the markdown).
    variant: VariantHeaders<'_>,
) -> Vec<Header> {
    let mut out: Vec<Header> = Vec::new();
    let mut saw_content_type = false;
    // A page-set `Vary` is captured and merged with `Accept` rather than
    // emitted twice — two Vary headers are legal but read badly, and some
    // intermediaries only honor the first.
    let mut page_vary: Option<String> = None;
    // A page-set Cache-Control is captured (not emitted in the loop) so the final
    // value can be decided with `request_shareable` in mind — a page must not be
    // able to advertise shared caching (`public`/`s-maxage`) for a non-shareable
    // (e.g. logged-in) request whose body carries that request's auth.
    let mut page_cache_control: Option<String> = None;
    let mut saw_set_cookie = false;
    // #277: the render's internal cache proof — `Some(secs)` means it proved
    // itself anonymous-safe + opted into caching. Captured + STRIPPED below so
    // it never reaches the client or CDN.
    let mut cacheable_secs: Option<u64> = None;
    // PPR Phase 0: the bucket proof — `Some(secs)` means the render proved itself
    // identity-free (only the binary session bit). Captured + STRIPPED below.
    let mut bucket_secs: Option<u64> = None;

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
        if lname.starts_with("x-pylon-") {
            // Reserved internal namespace. Capture the #277 cache proof + the
            // Phase 0 bucket proof, then STRIP every `x-pylon-*` — none may reach
            // the client/CDN, and a value forged by userland (page setHeader, or a
            // route handler's returned headers) must never be honored.
            if lname == "x-pylon-cacheable" {
                cacheable_secs = value.trim().parse::<u64>().ok();
            } else if lname == "x-pylon-bucket" {
                bucket_secs = value.trim().parse::<u64>().ok();
            }
            continue;
        }
        if !header_value_is_safe(value) || !header_name_is_safe(name) {
            continue;
        }
        if lname == "cache-control" {
            // Defer to the post-loop decision (see `page_cache_control`).
            page_cache_control = Some(value.clone());
            continue;
        }
        if lname == "vary" {
            // Merged with `Accept` below.
            page_vary = Some(value.clone());
            continue;
        }
        if lname == "content-type" && variant.content_type.is_some() {
            // This response was converted after the render; the page's own
            // content type describes bytes that are no longer being sent.
            continue;
        }
        if let Ok(h) = Header::from_bytes(name.as_bytes(), value.as_bytes()) {
            if lname == "content-type" {
                saw_content_type = true;
            }
            out.push(h);
        }
    }

    if let Some(ct) = variant.content_type {
        out.push(Header::from_bytes("Content-Type", ct).unwrap());
    } else if !saw_content_type {
        out.push(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap());
    }
    {
        // `Vary: Accept` on EVERY SSR response, not just the markdown ones: a
        // shared cache that stored the HTML without it would serve that HTML to
        // the next agent asking for markdown.
        let vary = match page_vary {
            Some(v)
                if v.split(',')
                    .any(|t| t.trim().eq_ignore_ascii_case("accept")) =>
            {
                v
            }
            Some(v) if !v.trim().is_empty() => format!("{}, Accept", v.trim_end_matches(',')),
            _ => "Accept".to_string(),
        };
        if let Ok(h) = Header::from_bytes("Vary", vary.as_bytes()) {
            out.push(h);
        }
    }
    // Advertise the markdown twin on the HTML response, so an agent that never
    // sets an Accept header can still find it (RFC 8288 `alternate`).
    if let Some(md_url) = variant.alternate_md {
        let value = format!("<{md_url}>; rel=\"alternate\"; type=\"text/markdown\"");
        if let Ok(h) = Header::from_bytes("Link", value.as_bytes()) {
            out.push(h);
        }
    }
    {
        // Does a Cache-Control EXPLICITLY forbid SHARED storage? Only an
        // UNQUALIFIED `private` or `no-store` qualifies. A field-qualified
        // `private="…"` does NOT (RFC 9111 §5.2.2.7: a shared cache may still
        // store the rest of the response), and `no-cache` / bare `max-age` are
        // shared-STORABLE (just revalidation/freshness-gated). Case-insensitive.
        fn is_non_shared_cc(cc: &str) -> bool {
            cc.split(',').any(|d| {
                let d = d.trim();
                d.eq_ignore_ascii_case("private") || d.eq_ignore_ascii_case("no-store")
            })
        }
        // A response may be stored by a SHARED cache only when the request is
        // itself shareable AND it sets no cookie — otherwise the body may carry
        // this request's auth (hydration tail), or a cookie-blind cache could
        // mis-serve a session bucket. Two MUTUALLY-EXCLUSIVE lanes:
        //   - bucket render (`x-pylon-bucket` present): shareability is governed
        //     ONLY by `bucket_shareable` (the CDN keys on session presence). The
        //     anon `request_shareable` must NOT grant sharing here — a signed-out
        //     bucket request is request_shareable=true, but its sess=0 shell would
        //     be mis-served to signed-in visitors by a cookie-blind cache. This
        //     also downgrades a bucket page that set its OWN `public` Cache-Control.
        //   - otherwise (anon / default): cookie-anonymous GET (`request_shareable`).
        let may_share = !saw_set_cookie
            && if bucket_secs.is_some() {
                bucket_shareable
            } else {
                request_shareable
            };
        // Natural Cache-Control from the page / proofs / defaults.
        let mut cc: String = if let Some(pcc) = page_cache_control {
            pcc
        } else if saw_set_cookie {
            "no-store".to_string()
        } else if let Some(secs) = cacheable_secs {
            format!("public, s-maxage={secs}, stale-while-revalidate={secs}")
        } else if let Some(secs) = bucket_secs {
            // Identity-free bucket body. Public (shared-within-bucket) ONLY when
            // the CDN keys on session presence; otherwise browser-`private` so a
            // cookie-blind shared cache can't mis-serve across buckets. The origin
            // ISR still serves it fast either way.
            if bucket_shareable {
                format!("public, s-maxage={secs}, stale-while-revalidate={secs}")
            } else {
                "private, no-store".to_string()
            }
        } else {
            "no-cache".to_string()
        };
        // SECURITY INVARIANT (the point of this function): a non-shareable
        // response must NEVER be storable by a shared cache. If the chosen CC
        // doesn't already forbid shared storage, force `private, no-store`. This
        // catches every branch — page-set `public`/bare `max-age`, the bare
        // `no-cache` default, a `public` #277 proof on a logged-in request, and a
        // `public` bucket proof when the CDN isn't bucket-keyed — while respecting
        // a page's own `private`/`no-store` (e.g. `private, max-age=60`).
        if !may_share && !is_non_shared_cc(&cc) {
            cc = "private, no-store".to_string();
        }
        out.push(Header::from_bytes("Cache-Control", cc.as_bytes()).unwrap());
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
    // An operator can name specific origins allowed to frame this app
    // (PYLON_FRAME_ANCESTORS) — Pylon Cloud sets it on dev-mode envs so the
    // builder can show the running app in its live-preview iframe. That has to
    // be expressed as CSP `frame-ancestors`: X-Frame-Options can only say
    // DENY/SAMEORIGIN, and its ALLOW-FROM form is dead in every current
    // browser, so leaving SAMEORIGIN alongside would still block the frame.
    // Unset (the default) keeps SAMEORIGIN and ships no CSP.
    let ancestors = configured_frame_ancestors();
    match ancestors {
        Some(list) if !has(&out, "Content-Security-Policy") => {
            let value = format!("frame-ancestors 'self' {list}");
            if let Ok(h) = Header::from_bytes("Content-Security-Policy", value.as_bytes()) {
                out.push(h);
            }
        }
        _ => {}
    }
    let frame_default: &[(&str, &str)] = if configured_frame_ancestors().is_some() {
        &[]
    } else {
        &[("X-Frame-Options", "SAMEORIGIN")]
    };
    for (name, value) in [
        ("X-Content-Type-Options", "nosniff"),
        ("Referrer-Policy", "strict-origin-when-cross-origin"),
        (
            "Permissions-Policy",
            "accelerometer=(), camera=(), geolocation=(), gyroscope=(), microphone=(), payment=(), usb=()",
        ),
    ]
    .iter()
    .chain(frame_default.iter())
    {
        if !has(&out, name) {
            if let Ok(h) = Header::from_bytes(*name, value.as_bytes()) {
                out.push(h);
            }
        }
    }
    out
}

/// Origins allowed to frame this app, from `PYLON_FRAME_ANCESTORS`
/// (comma- or space-separated, e.g. "https://www.pylonsync.com").
///
/// Returns the validated, space-joined source list for a CSP
/// `frame-ancestors` directive, or `None` when unset/empty — in which case the
/// caller keeps the default `X-Frame-Options: SAMEORIGIN`.
///
/// Every entry must be a bare `scheme://host[:port]` origin. Anything else is
/// dropped: a value carrying a comma, semicolon, whitespace, or control
/// character could otherwise close the directive and inject a second CSP
/// directive into the header.
fn configured_frame_ancestors() -> Option<&'static str> {
    static ANCESTORS: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    ANCESTORS
        .get_or_init(|| parse_frame_ancestors(&std::env::var("PYLON_FRAME_ANCESTORS").ok()?))
        .as_deref()
}

/// Split + validate a `PYLON_FRAME_ANCESTORS` value. Pure so the filtering can
/// be tested directly — `configured_frame_ancestors` caches this in a OnceLock,
/// which a test can't re-drive.
fn parse_frame_ancestors(raw: &str) -> Option<String> {
    let list = raw
        .split([',', ' ', '\t'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter(|s| is_valid_frame_ancestor(s))
        .collect::<Vec<_>>()
        .join(" ");
    if list.is_empty() {
        None
    } else {
        Some(list)
    }
}

/// A single `frame-ancestors` source: `scheme://host[:port]`, nothing else.
fn is_valid_frame_ancestor(value: &str) -> bool {
    let Some(rest) = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))
    else {
        return false;
    };
    !rest.is_empty()
        && rest
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '*'))
}

/// A minimal, structured SSR error response (used when the render can't
/// even start, or fails before emitting `response_start`).
/// Friendly, framework-default error page. Styled (light/dark aware), no app
/// code required — this is the "provided default" so an html request never
/// sees raw JSON. Apps can still override per-route with `error.tsx` /
/// `not-found.tsx` (and `rate-limit.tsx`, served on 429). `heading`/`message`
/// are HTML-escaped by the caller's literals (static, no user input here).
pub(crate) fn builtin_error_page_html(status: u16, heading: &str, message: &str) -> String {
    format!(
        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<title>{status} — {heading}</title>\
<style>\
:root{{color-scheme:light dark}}\
*{{box-sizing:border-box}}\
body{{margin:0;min-height:100vh;display:grid;place-items:center;\
font:16px/1.6 -apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif;\
background:#fafafa;color:#18181b}}\
@media(prefers-color-scheme:dark){{body{{background:#0b0d12;color:#e6e8eb}}}}\
.card{{max-width:30rem;padding:0 24px;text-align:center}}\
.code{{font-size:13px;font-weight:600;letter-spacing:.08em;text-transform:uppercase;\
color:#a1a1aa}}\
h1{{font-size:24px;font-weight:600;margin:8px 0 8px}}\
p{{margin:0;color:#71717a}}\
@media(prefers-color-scheme:dark){{p{{color:#9aa3ad}}}}\
a{{display:inline-block;margin-top:24px;color:inherit;text-decoration:none;\
border:1px solid currentColor;border-radius:8px;padding:8px 18px;font-size:14px;font-weight:500;opacity:.85}}\
a:hover{{opacity:1}}\
</style></head><body><div class=\"card\">\
<div class=\"code\">{status}</div>\
<h1>{heading}</h1>\
<p>{message}</p>\
<a href=\"/\">Back home</a>\
</div></body></html>",
    )
}

fn ssr_error_response(status: u16, cors_origin: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    let (heading, message) = match status {
        404 => (
            "Page not found",
            "The page you're looking for doesn't exist or has moved.",
        ),
        429 => (
            "Too many requests",
            "You've hit the rate limit. Please slow down and try again shortly.",
        ),
        _ => (
            "Something went wrong",
            "The server ran into an error rendering this page. Please try again.",
        ),
    };
    let body = builtin_error_page_html(status, heading, message).into_bytes();
    let mut resp = Response::from_data(body)
        .with_status_code(status)
        .with_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap());
    if let Ok(h) = Header::from_bytes("Access-Control-Allow-Origin", cors_origin.as_bytes()) {
        resp = resp.with_header(h);
    }
    resp
}

/// Whether the client prefers an HTML response (a browser navigation) over
/// JSON (an API/fetch call). Used to decide if an early short-circuit (rate
/// limit, etc.) should render the friendly HTML error page instead of the API
/// JSON envelope. Defaults to false (JSON) when no Accept header is present.
pub(crate) fn request_prefers_html(request: &tiny_http::Request) -> bool {
    request.headers().iter().any(|h| {
        let field = h.field.as_str();
        (field == "Accept" || field == "accept") && {
            let v = h.value.as_str();
            v.contains("text/html") || v.contains("application/xhtml")
        }
    })
}

/// The app's own 429 page, if it shipped `app/rate-limit.tsx`. The client
/// bundler pre-renders it to `<client-build>/rate-limit.html` (compiled CSS
/// inlined, self-contained) — the rate limiter short-circuits before SSR, so a
/// per-request render would defeat the point; this is read once + cached and
/// served straight off memory. `None` when the app didn't ship one → the
/// caller falls back to the built-in default. Not cached until the first
/// successful read, so a 429 that races boot (bundle not warm yet) retries.
pub fn app_rate_limit_html() -> Option<String> {
    static CACHE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    if let Some(s) = CACHE.get() {
        return Some(s.clone());
    }
    let outdir = cached_bundle_outdir().lock().ok()?.clone()?;
    let html = std::fs::read_to_string(outdir.join("rate-limit.html")).ok()?;
    let _ = CACHE.set(html.clone());
    Some(html)
}

/// 429 page for browser navigations that get rate-limited, so they see a page
/// instead of raw `{"error":...}` JSON. Prefers the app's pre-rendered
/// `app/rate-limit.tsx`; otherwise the styled framework default. `retry_after`
/// rides the standard `Retry-After` header either way.
pub fn rate_limited_html_response(
    retry_after: u64,
    cors_origin: &str,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let body = app_rate_limit_html().unwrap_or_else(|| {
        let message = format!(
            "You've made too many requests. Please wait {retry_after} second{} and try again.",
            if retry_after == 1 { "" } else { "s" }
        );
        builtin_error_page_html(429, "Too many requests", &message)
    });
    let body = body.into_bytes();
    let mut resp = Response::from_data(body)
        .with_status_code(429u16)
        .with_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap())
        .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap())
        .with_header(
            Header::from_bytes("Retry-After", retry_after.to_string().as_bytes()).unwrap(),
        );
    if let Ok(h) = Header::from_bytes("Access-Control-Allow-Origin", cors_origin.as_bytes()) {
        resp = resp.with_header(h);
    }
    resp
}

/// How long an SSR/route render waits for the TypeScript runner to become
/// ready on a cold boot before falling back to a retryable 503. Override with
/// `PYLON_SSR_RUNNER_READY_TIMEOUT_MS`; default 20s (generous for a cold
/// `bun` boot + first bundle build, but bounded so a wedged pool still
/// returns). 0 disables the wait (legacy fail-fast behavior).
fn ssr_runner_ready_timeout() -> std::time::Duration {
    let ms = std::env::var("PYLON_SSR_RUNNER_READY_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(20_000);
    std::time::Duration::from_millis(ms)
}

/// The graceful 503 served when the TypeScript runner is still booting after
/// `ssr_runner_ready_timeout()` — used instead of a hard 500 so the browser
/// (and any CDN) retries shortly. The tiny page auto-refreshes. This is the
/// cold-start safety net; in practice the readiness wait succeeds first.
fn ssr_warming_response(cors_origin: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    let body = "<!DOCTYPE html><html><head><meta charset=\"utf-8\">\
<meta http-equiv=\"refresh\" content=\"2\"><title>Starting up…</title>\
<meta name=\"robots\" content=\"noindex\"></head>\
<body style=\"font-family:system-ui,sans-serif;display:grid;place-items:center;min-height:100vh;margin:0\">\
<div style=\"text-align:center;color:#555\"><p>Starting up…</p>\
<p style=\"font-size:.85rem\">This page is warming up and will load in a moment.</p></div>\
</body></html>"
        .as_bytes()
        .to_vec();
    let mut resp = Response::from_data(body)
        .with_status_code(503u16)
        .with_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap())
        .with_header(Header::from_bytes("Retry-After", "2").unwrap())
        // Never let a shared cache pin the warming page.
        .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap());
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

/// Build the SSR client (hydration) bundle now and memoize its outdir, off
/// the request path. Called once from the boot-time warm thread (server.rs)
/// so a fresh artifact deploy doesn't run the cold `Bun.build` on the first
/// user request.
///
/// It populates the SAME `cached_bundle_outdir` the asset route uses and
/// holds that lock across the build exactly like `serve_pylon_client_bundle`
/// does — so a request that races boot serializes on this mutex and finds the
/// finished result instead of triggering a SECOND build. The build also
/// writes `.pylon/client-build/manifest.json`, which the SSR render path's
/// `getManifest` reads directly, so that path stays cheap too. Returns the
/// bundler error (for logging) when the build fails; the lazy first-request
/// path remains the fallback.
pub fn warm_client_bundle(
    fn_ops: &std::sync::Arc<dyn pylon_router::FnOps>,
    app_dir: &str,
) -> Result<(), String> {
    let mut cache = cached_bundle_outdir().lock().unwrap();
    if cache.is_some() {
        // A real request already built it between boot and now — nothing to do.
        return Ok(());
    }
    match fn_ops.bundle_client(app_dir) {
        Ok(paths) => {
            *cache = Some(std::path::PathBuf::from(paths.outdir));
            Ok(())
        }
        Err(e) => Err(format!("{}: {}", e.code, e.message)),
    }
}

/// Registry of live-reload SSE senders (one per open dev tab). `trigger_dev_reload`
/// fans a reload out to them; each `serve_dev_live_reload` connection registers
/// itself. Reaped lazily: a send to a closed tab errors and is dropped.
fn dev_reload_registry() -> &'static std::sync::Mutex<Vec<std::sync::mpsc::Sender<u64>>> {
    static R: std::sync::OnceLock<std::sync::Mutex<Vec<std::sync::mpsc::Sender<u64>>>> =
        std::sync::OnceLock::new();
    R.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

/// Bun runner pool + the SSR app dir (None for an app with no SSR routes,
/// which has no client bundle to rebuild). Set once by the dev server at
/// boot; `None` elsewhere so `trigger_dev_reload_with` reports it can't and
/// the caller falls back to a full restart.
type DevRebuildCtx = (std::sync::Arc<dyn pylon_router::FnOps>, Option<String>);
fn dev_rebuild_ctx() -> &'static std::sync::Mutex<Option<DevRebuildCtx>> {
    static C: std::sync::OnceLock<std::sync::Mutex<Option<DevRebuildCtx>>> =
        std::sync::OnceLock::new();
    C.get_or_init(|| std::sync::Mutex::new(None))
}

/// Register the in-process reload context. Called once from the dev server
/// at boot for every app with a functions backend; `app_dir` is set only
/// when the app has SSR routes (and so a client bundle).
pub fn set_dev_rebuild_ctx(
    fn_ops: std::sync::Arc<dyn pylon_router::FnOps>,
    app_dir: Option<String>,
) {
    *dev_rebuild_ctx().lock().unwrap() = Some((fn_ops, app_dir));
}

static DEV_RELOAD_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// In-process dev reload for a UI-only edit (no manifest change): rebuild the
/// client bundle synchronously — so the fresh manifest + newly-hashed assets are
/// on disk before any tab reloads — then tell every connected live-reload client
/// to reload, WITHOUT re-exec'ing the process (the warm bun runner pool is kept).
///
/// Returns false when the rebuild context isn't registered (not a dev SSR
/// server) or the rebuild failed, so the caller falls back to a full restart.
pub fn trigger_dev_reload() -> bool {
    trigger_dev_reload_with(DevReload {
        respawn_runtime: false,
        rebuild_client: true,
    })
    .is_ok()
}

/// What an in-process dev reload has to redo for a given edit.
#[derive(Debug, Clone, Copy)]
pub struct DevReload {
    /// Respawn the function runners so server code (functions, SSR
    /// components, anything they import) is re-imported.
    pub respawn_runtime: bool,
    /// Rebuild the client bundle so hydrated code and styles match.
    pub rebuild_client: bool,
}

/// Run an in-process reload. Returns the function count when the runtime
/// was respawned (`Ok(0)` when it was not). The error names why an
/// in-place reload was not possible; the caller falls back to a full
/// restart and shows the reason.
pub fn trigger_dev_reload_with(what: DevReload) -> Result<usize, String> {
    let Some((fn_ops, app_dir)) = dev_rebuild_ctx().lock().unwrap().clone() else {
        return Err("no reload context registered by the dev server".to_string());
    };
    let mut fn_count = 0;
    if what.respawn_runtime {
        fn_count = fn_ops
            .reload_runtime()
            .map_err(|e| format!("runtime respawn failed: {e}"))?;
    }
    // No SSR routes means no client bundle: the respawn above is the whole
    // reload. Open tabs (a legacy `web/dist` frontend) still get the ping.
    let Some(app_dir) = app_dir.filter(|_| what.rebuild_client) else {
        ping_dev_reload_clients();
        return Ok(fn_count);
    };
    // Bundle filenames are content-hashed, so a tab must never reload against a
    // stale hash: invalidate the memo, then rebuild synchronously. The rebuild
    // re-memoizes the cache and rewrites `client-build/manifest.json` (read by
    // the SSR render path), keeping the reloaded HTML + its assets coherent.
    *cached_bundle_outdir().lock().unwrap() = None;
    warm_client_bundle(&fn_ops, &app_dir)
        .map_err(|e| format!("client bundle rebuild failed: {e}"))?;
    ping_dev_reload_clients();
    Ok(fn_count)
}

/// Tell every open `/_pylon/dev/live` tab to reload.
fn ping_dev_reload_clients() {
    let generation = DEV_RELOAD_GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    dev_reload_registry()
        .lock()
        .unwrap()
        .retain(|tx| tx.send(generation).is_ok());
}

/// Recover the project-relative route directory (the `appDir` the
/// manifest was built with) from the SSR routes. Every page's
/// `component` is `<appDir>/.../page`; the root route (`/`) is exactly
/// `<appDir>/page`, so its parent is the appDir. Falls back to the
/// shallowest component's parent, then `"app"` (the default layout) so
/// a manifest without a literal `/` route still resolves.
///
/// The client bundler walks this same dir; without it, an app that
/// namespaces its frontend under a subdir (e.g. `web/app`) renders SSR
/// HTML but ships no hydration bundle — the bundler's old hardcoded
/// `app/` found nothing.
/// Source dir for LOCAL `<Image src="/foo.png">` optimization: the legacy
/// static build (`web/dist`, when present) else the native-SSR static dir
/// `<cwd>/public` — the same dir the static-asset route serves from. Without
/// the fallback an SSR app (no `web/dist`) can't optimize its own `public/`
/// images: `<Image>` 400s with "no frontend dir configured for local images"
/// even though `GET /foo.png` serves fine.
fn local_image_source_dir(dist_dir: Option<&Path>, cwd: &Path) -> PathBuf {
    dist_dir
        .map(|d| d.to_path_buf())
        .unwrap_or_else(|| cwd.join("public"))
}

pub fn derive_app_dir(routes: &[pylon_kernel::ManifestRoute]) -> String {
    fn parent_dir(module: &str) -> Option<&str> {
        module.rfind('/').map(|i| &module[..i])
    }
    // Prefer the root page. Its component is `<appDir>/<(group)>*/page` — the
    // "/" route has NO dynamic/static route segments, only optional route
    // GROUPS like `(home)`. Strip the trailing `/page` and any `(group)`
    // segments to recover the appDir. (Bare `parent_dir` here returned
    // `app/(home)` for a grouped home page, so the client bundler only walked
    // the group dir and emitted ONE route's hydration entry.)
    if let Some(root) = routes.iter().find(|r| r.path == "/") {
        if let Some(comp) = root.component.as_deref() {
            let mut dir = comp.strip_suffix("/page").unwrap_or(comp);
            while let Some(i) = dir.rfind('/') {
                let last = &dir[i + 1..];
                if last.starts_with('(') && last.ends_with(')') {
                    dir = &dir[..i];
                } else {
                    break;
                }
            }
            if !dir.is_empty() && dir != comp {
                return dir.to_string();
            }
            // No trailing group (component was `<appDir>/page`): the bare
            // parent is correct.
            if let Some(dir) = parent_dir(comp) {
                return dir.to_string();
            }
        }
    }
    // Fallback (no literal `/` route): the appDir is the longest common
    // directory prefix across every module's parent dir. Every page is
    // `<appDir>/.../page` and every layout `<appDir>/.../layout`, so two
    // routes (or a route + its root layout) that diverge below the appDir
    // pin the prefix to exactly `<appDir>`. The shallowest-parent
    // heuristic was wrong — it returned the first route's own subdir.
    let dirs: Vec<&str> = routes
        .iter()
        .flat_map(|r| {
            r.component
                .as_deref()
                .and_then(parent_dir)
                .into_iter()
                .chain(r.layouts.iter().filter_map(|l| parent_dir(l)))
        })
        .collect();
    let Some((first, rest)) = dirs.split_first() else {
        return "app".to_string();
    };
    let mut common: Vec<&str> = first.split('/').collect();
    for d in rest {
        let segs: Vec<&str> = d.split('/').collect();
        let n = common
            .iter()
            .zip(segs.iter())
            .take_while(|(a, b)| a == b)
            .count();
        common.truncate(n);
        if common.is_empty() {
            break;
        }
    }
    if common.is_empty() {
        "app".to_string()
    } else {
        common.join("/")
    }
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
    } else if name.ends_with(".woff2") {
        "font/woff2"
    } else if name.ends_with(".woff") {
        "font/woff"
    } else if name.ends_with(".ttf") {
        "font/ttf"
    } else if name.ends_with(".otf") {
        "font/otf"
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
        let app_dir = derive_app_dir(&cfg.ssr_routes);
        match fn_ops.bundle_client(&app_dir) {
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
/// #277 Stage 2: cheap cookie-anonymity gate for the disk-cache fast path.
/// Returns true when the request carries the session cookie (by name) — in
/// which case it bypasses the shared on-disk cache entirely (never serve a
/// shared entry to a request that MIGHT be authed, and never tee its render).
/// A present-but-invalid session cookie just costs a live render; that's the
/// safe direction. Apps without a SessionStore/CookieConfig have no session
/// cookie, so every GET is cache-eligible.
fn session_cookie_present(cfg: &FrontendConfig, request: &Request) -> bool {
    let Some(cookie_cfg) = cfg.cookie_config.as_ref() else {
        return false;
    };
    for h in request.headers() {
        if !h.field.as_str().as_str().eq_ignore_ascii_case("cookie") {
            continue;
        }
        if cookie_header_has_named(h.value.as_str(), &cookie_cfg.name) {
            return true;
        }
    }
    false
}

/// PPR Phase 0: does the request's session cookie RESOLVE to a real signed-in
/// user — not merely "a cookie string is present"? This is what keys the auth
/// bucket + feeds `props.session.exists`, so an expired/invalid cookie maps to
/// the signed-OUT bucket (sess=0) instead of a misleading signed-in shell. The
/// render path derives the same bit from its already-resolved `auth`
/// (`auth.user_id.is_some()`); this helper is for the cache-READ fast path, where
/// no auth has been resolved yet. Cheap short-circuit: a request with no session
/// cookie pays NO store lookup (only cookie-bearing requests resolve).
fn session_authenticated(cfg: &FrontendConfig, request: &Request) -> bool {
    if !session_cookie_present(cfg, request) {
        return false;
    }
    let (Some(store), Some(cookie_cfg)) = (cfg.session_store.as_ref(), cfg.cookie_config.as_ref())
    else {
        return false;
    };
    let mut token: Option<String> = None;
    for h in request.headers() {
        if !h.field.as_str().as_str().eq_ignore_ascii_case("cookie") {
            continue;
        }
        for pair in h.value.as_str().split(';') {
            if let Some((k, v)) = pair.trim().split_once('=') {
                if k == cookie_cfg.name && !v.is_empty() {
                    token = Some(v.to_string());
                }
            }
        }
    }
    store.resolve(token.as_deref()).user_id.is_some()
}

/// Pure: does a `Cookie:` header value contain a non-empty cookie named
/// `name`? Extracted for unit testing the cache-eligibility gate.
fn cookie_header_has_named(header_value: &str, name: &str) -> bool {
    for pair in header_value.split(';') {
        if let Some((k, v)) = pair.trim().split_once('=') {
            if k == name && !v.is_empty() {
                return true;
            }
        }
    }
    false
}

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
    let mut ctx = store.resolve(token.map(|s| s.as_str()));
    // Mirror the main HTTP request handler EXACTLY, in the same order:
    //
    //   1. Per-user admin-lift (`auth.user.adminField` + `PYLON_ADMIN_EMAILS`).
    //      Without this, `SessionStore::resolve` returns `is_admin:false` for
    //      every user and SSR pages render as non-admin even for real admins —
    //      diverging from the API/sync paths (which DO lift it).
    //   2. Active-org role enrichment (`auth.roles`).
    //
    // Order matters: `enrich_active_org_role` early-returns when `is_admin`, so
    // the admin-lift must run first (an admin who is also an org owner then
    // carries `is_admin:true, roles:[]` — identical to the API path).
    if let Some(rt) = cfg.runtime.as_ref() {
        crate::server::lift_admin(rt, &mut ctx);
    }
    if let Some(orgs) = cfg.orgs.as_ref() {
        pylon_auth::org::enrich_active_org_role(orgs, &mut ctx);
    }
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
    fn parse_byte_range_forms() {
        // Closed range within bounds.
        assert_eq!(
            parse_byte_range("bytes=0-1023", 4096),
            RangeSpec::Partial(0, 1023)
        );
        // Open-ended range → to the last byte.
        assert_eq!(
            parse_byte_range("bytes=1000-", 4096),
            RangeSpec::Partial(1000, 4095)
        );
        // Suffix range → the last N bytes.
        assert_eq!(
            parse_byte_range("bytes=-500", 4096),
            RangeSpec::Partial(3596, 4095)
        );
        // End past EOF is clamped to the last byte (iOS probes `bytes=0-1` then
        // often `bytes=0-<huge>`).
        assert_eq!(
            parse_byte_range("bytes=0-99999", 4096),
            RangeSpec::Partial(0, 4095)
        );
        // Suffix larger than the file → the whole file.
        assert_eq!(
            parse_byte_range("bytes=-99999", 4096),
            RangeSpec::Partial(0, 4095)
        );
        // Whitespace tolerated.
        assert_eq!(
            parse_byte_range("bytes= 0-10 ", 4096),
            RangeSpec::Partial(0, 10)
        );
    }

    #[test]
    fn parse_byte_range_unsatisfiable_and_fallback() {
        // start at/after EOF → 416.
        assert_eq!(
            parse_byte_range("bytes=4096-5000", 4096),
            RangeSpec::Unsatisfiable
        );
        assert_eq!(
            parse_byte_range("bytes=4096-", 4096),
            RangeSpec::Unsatisfiable
        );
        // Zero-length suffix → 416.
        assert_eq!(parse_byte_range("bytes=-0", 4096), RangeSpec::Unsatisfiable);
        // Range against an empty body → 416.
        assert_eq!(parse_byte_range("bytes=0-10", 0), RangeSpec::Unsatisfiable);
        // Inverted range → 416.
        assert_eq!(
            parse_byte_range("bytes=500-100", 4096),
            RangeSpec::Unsatisfiable
        );
        // Multi-range, wrong unit, or garbage → serve the whole body (200).
        assert_eq!(parse_byte_range("bytes=0-1,5-6", 4096), RangeSpec::Full);
        assert_eq!(parse_byte_range("items=0-1", 4096), RangeSpec::Full);
        assert_eq!(parse_byte_range("bytes=abc-def", 4096), RangeSpec::Full);
        assert_eq!(parse_byte_range("", 4096), RangeSpec::Full);
    }

    /// Regression: the dev live-reload SSE must deliver its HTTP head + the
    /// `hello` event IMMEDIATELY — not after tiny_http's 1KB write buffer
    /// fills (~45s of heartbeats). The old `request.respond()` path buffered
    /// exactly that way (raw_print never returns for an infinite body, and
    /// tiny_http only flushes after it returns), so the browser's EventSource
    /// never connected and hot reload was dead. Drives a REAL tiny_http
    /// server + raw TcpStream so the flush behavior is what's actually
    /// asserted.
    #[test]
    fn dev_live_reload_sse_flushes_hello_immediately() {
        use std::io::{Read as _, Write as _};

        let server = tiny_http::Server::http("127.0.0.1:0").expect("bind ephemeral");
        let port = match server.server_addr() {
            tiny_http::ListenAddr::IP(addr) => addr.port(),
            _ => unreachable!("bound to an IP addr"),
        };
        let handler = std::thread::spawn(move || {
            if let Ok(req) = server.recv() {
                let _ = serve_dev_live_reload(req, "*");
            }
        });

        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
        stream
            .set_read_timeout(Some(std::time::Duration::from_millis(500)))
            .unwrap();
        stream
            .write_all(b"GET /_pylon/dev/live HTTP/1.1\r\nHost: localhost\r\nAccept: text/event-stream\r\n\r\n")
            .unwrap();

        // The head + hello must arrive well within 3s (they're sent + flushed
        // in one write; the generous budget is only for CI scheduling).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        let mut got: Vec<u8> = Vec::new();
        let mut buf = [0u8; 2048];
        while std::time::Instant::now() < deadline {
            match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    got.extend_from_slice(&buf[..n]);
                    if String::from_utf8_lossy(&got).contains("event: hello") {
                        break;
                    }
                }
                Err(_) => continue, // read timeout tick — keep polling until deadline
            }
        }
        let text = String::from_utf8_lossy(&got);
        assert!(
            text.starts_with("HTTP/1.1 200"),
            "no HTTP head flushed — got: {text:?}"
        );
        assert!(text.contains("Content-Type: text/event-stream"), "{text:?}");
        assert!(
            text.contains("event: hello") && text.contains(&format!("data: {}", dev_boot_id())),
            "hello event with boot id not delivered — got: {text:?}"
        );
        // Closing our end unblocks the heartbeat thread's next write → it
        // exits; the handler thread already returned after into_writer().
        drop(stream);
        let _ = handler.join();
    }

    #[test]
    fn social_image_rel_allowlist() {
        // Valid colocated social-card images (default `app` appDir).
        assert!(is_valid_colocated_asset_rel(
            "app/opengraph-image.png",
            "app"
        ));
        assert!(is_valid_colocated_asset_rel(
            "app/blog/opengraph-image.jpg",
            "app"
        ));
        assert!(is_valid_colocated_asset_rel(
            "app/(marketing)/twitter-image.webp",
            "app"
        ));
        assert!(is_valid_colocated_asset_rel(
            "app/x/[slug]/opengraph-image.PNG",
            "app"
        ));
        // Wrong basename — must never serve arbitrary source files.
        assert!(!is_valid_colocated_asset_rel("app/page.tsx", "app"));
        assert!(!is_valid_colocated_asset_rel("app/secret.png", "app"));
        assert!(!is_valid_colocated_asset_rel("app/blog/cover.png", "app"));
        // Icon conventions (svg/ico allowed here).
        assert!(is_valid_colocated_asset_rel("app/icon.png", "app"));
        assert!(is_valid_colocated_asset_rel("app/icon.svg", "app"));
        assert!(is_valid_colocated_asset_rel("app/favicon.ico", "app"));
        assert!(is_valid_colocated_asset_rel("app/apple-icon.png", "app"));
        // Wrong root / extension.
        assert!(!is_valid_colocated_asset_rel(
            "public/opengraph-image.png",
            "app"
        ));
        assert!(!is_valid_colocated_asset_rel(
            "app/opengraph-image.ts",
            "app"
        ));
        assert!(!is_valid_colocated_asset_rel("app/icon.tsx", "app"));
        assert!(!is_valid_colocated_asset_rel("app/logo.svg", "app"));
        // Traversal / absolute / malformed.
        assert!(!is_valid_colocated_asset_rel(
            "app/../secret/opengraph-image.png",
            "app"
        ));
        assert!(!is_valid_colocated_asset_rel(
            "/app/opengraph-image.png",
            "app"
        ));
        assert!(!is_valid_colocated_asset_rel(
            "../app/opengraph-image.png",
            "app"
        ));
        assert!(!is_valid_colocated_asset_rel(
            "app/./opengraph-image.png",
            "app"
        ));
        assert!(!is_valid_colocated_asset_rel("", "app"));
        assert!(!is_valid_colocated_asset_rel("opengraph-image.png", "app"));

        // Subdir appDir (`web/app`) — control-plane / monorepo frontends. This
        // is the pylonsync.com case that 404'd when the root was hardcoded.
        assert!(is_valid_colocated_asset_rel(
            "web/app/opengraph-image.png",
            "web/app"
        ));
        assert!(is_valid_colocated_asset_rel(
            "web/app/(marketing)/twitter-image.png",
            "web/app"
        ));
        // The src MUST match the configured appDir — an `app/...` src is
        // rejected when the appDir is `web/app`, and vice-versa.
        assert!(!is_valid_colocated_asset_rel(
            "app/opengraph-image.png",
            "web/app"
        ));
        assert!(!is_valid_colocated_asset_rel(
            "web/app/opengraph-image.png",
            "app"
        ));
        // No escaping the subdir root.
        assert!(!is_valid_colocated_asset_rel(
            "web/secret/opengraph-image.png",
            "web/app"
        ));
        assert!(!is_valid_colocated_asset_rel("web/app", "web/app"));
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
        assert!(!is_spa_eligible("/admin/entities"));
        assert!(!is_spa_eligible("/admin/entities/User"));
        assert!(!is_spa_eligible("/admin/fn/traces"));
        assert!(!is_spa_eligible("/admin/jobs"));
        assert!(!is_spa_eligible("/admin/workflows"));
        assert!(!is_spa_eligible("/.well-known/acme-challenge/x"));
        // The OIDC IdP surface — a frontend must never shadow these, or
        // /oidc/jwks serves the app's 404 page instead of signing keys.
        assert!(!is_spa_eligible("/oidc"));
        assert!(!is_spa_eligible("/oidc/jwks"));
        assert!(!is_spa_eligible("/oidc/authorize?client_id=x"));
        assert!(!is_spa_eligible("/oidc/token"));
        assert!(!is_spa_eligible("/oidc/userinfo"));
    }

    #[test]
    fn html_paths_are_eligible() {
        assert!(is_spa_eligible("/"));
        assert!(is_spa_eligible("/channels"));
        assert!(is_spa_eligible("/channels/general"));
        assert!(is_spa_eligible("/assets/index-abc123.js"));
        assert!(is_spa_eligible("/favicon.ico"));
        // App-defined admin pages live under /admin/* alongside the
        // framework's token-gated endpoints — only the latter are reserved.
        assert!(is_spa_eligible("/admin/orgs"));
        assert!(is_spa_eligible("/admin/fly-costs"));
        assert!(is_spa_eligible("/admin"));
        // Only the exact /oidc namespace is reserved — an app page that
        // merely starts with the letters stays an app page.
        assert!(is_spa_eligible("/oidc-help"));
    }

    #[test]
    fn public_dir_assets_resolve_with_model_mime() {
        // Regression for the `public/` static convention: files under
        // <app>/public resolve through the same traversal guard as
        // dist serving, and 3D model assets carry real mime types.
        let tmp = TempDir::new().unwrap();
        let public = tmp.path().join("public");
        fs::create_dir_all(public.join("models")).unwrap();
        fs::write(public.join("models/character.glb"), b"glTF").unwrap();

        let hit = resolve_safe(&public, "/models/character.glb").expect("public file resolves");
        assert_eq!(content_type_for(&hit), "model/gltf-binary");
        assert_eq!(content_type_for(Path::new("scene.gltf")), "model/gltf+json");

        // Traversal out of public/ stays rejected.
        assert!(resolve_safe(&public, "/models/../../secrets.txt").is_none());
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
    fn resolve_safe_for_write_allows_new_leaf_rejects_traversal() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("app")).unwrap();

        // A new (non-existent) leaf in an existing dir is allowed — a write
        // target need not exist yet (this is the whole point vs resolve_safe).
        let t = resolve_safe_for_write(root, "app/page.tsx").unwrap();
        assert!(t.starts_with(root.canonicalize().unwrap()));
        assert!(t.ends_with("app/page.tsx"));

        // A nested path whose parent dirs don't exist yet is allowed (the
        // caller mkdir -p's before writing).
        assert!(resolve_safe_for_write(root, "app/new/deep/file.ts").is_some());

        // Overwriting an existing file is allowed.
        fs::write(root.join("app/existing.ts"), b"x").unwrap();
        assert!(resolve_safe_for_write(root, "app/existing.ts").is_some());

        // Traversal / dot / empty segments are rejected.
        assert!(resolve_safe_for_write(root, "../etc/passwd").is_none());
        assert!(resolve_safe_for_write(root, "app/../../etc/passwd").is_none());
        assert!(resolve_safe_for_write(root, "./app/page.tsx").is_none());
        assert!(resolve_safe_for_write(root, "app//page.tsx").is_none());
        assert!(resolve_safe_for_write(root, "").is_none());
        assert!(resolve_safe_for_write(root, "/").is_none());
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

    /// Build a route whose component sits under an arbitrary appDir
    /// (the `route` helper hardcodes `app`; this lets us simulate a
    /// `web/app` subdir layout).
    fn route_in(dir: &str, path: &str) -> pylon_kernel::ManifestRoute {
        let suffix = if path == "/" {
            "page".to_string()
        } else {
            format!("{}/page", path.trim_start_matches('/'))
        };
        pylon_kernel::ManifestRoute {
            path: path.into(),
            mode: "ssr".into(),
            component: Some(format!("{dir}/{suffix}")),
            layouts: vec![format!("{dir}/layout")],
            ..Default::default()
        }
    }

    #[test]
    fn local_image_source_dir_falls_back_to_public() {
        let cwd = Path::new("/app");
        // Native SSR app (no web/dist) → optimize images out of <cwd>/public.
        assert_eq!(
            local_image_source_dir(None, cwd),
            PathBuf::from("/app/public")
        );
        // Legacy app with a built dist → unchanged.
        assert_eq!(
            local_image_source_dir(Some(Path::new("/app/web/dist")), cwd),
            PathBuf::from("/app/web/dist")
        );
    }

    #[test]
    fn derive_app_dir_recovers_route_root() {
        // Default single-`app/` layout.
        let app = vec![route_in("app", "/"), route_in("app", "/login")];
        assert_eq!(derive_app_dir(&app), "app");

        // Namespaced subdir layout (the unified pylon-cloud dashboard).
        // The root page is `web/app/page` → appDir `web/app`. This is the
        // exact case the client bundler must honor or it ships no
        // hydration bundle.
        let web = vec![
            route_in("web/app", "/"),
            route_in("web/app", "/dashboard"),
            route_in("web/app", "/dashboard/orgs/[slug]/settings"),
        ];
        assert_eq!(derive_app_dir(&web), "web/app");

        // No literal "/" route → fall back to the shallowest component's
        // parent (still resolves the subdir).
        let no_root = vec![
            route_in("web/app", "/login"),
            route_in("web/app", "/dashboard/settings"),
        ];
        assert_eq!(derive_app_dir(&no_root), "web/app");

        // Empty / componentless → safe default.
        assert_eq!(derive_app_dir(&[]), "app");

        // Route GROUP on the root page: `/` → `app/(home)/page`. The appDir is
        // `app`, NOT `app/(home)` — else the client bundler walks only the
        // group dir and emits one route's hydration entry (the yapless bug).
        let grouped = |comp: &str, path: &str| pylon_kernel::ManifestRoute {
            path: path.into(),
            mode: "ssr".into(),
            component: Some(comp.into()),
            layouts: vec!["app/layout".into()],
            ..Default::default()
        };
        let groups = vec![
            grouped("app/(home)/page", "/"),
            grouped("app/(auth)/login/page", "/login"),
            grouped("app/dashboard/page", "/dashboard"),
        ];
        assert_eq!(derive_app_dir(&groups), "app");

        // Grouped root under a namespaced subdir.
        let web_groups = vec![
            grouped("web/app/(home)/page", "/"),
            grouped("web/app/(marketing)/pricing/page", "/pricing"),
        ];
        assert_eq!(derive_app_dir(&web_groups), "web/app");
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
    fn match_ssr_route_accepts_data_routes() {
        // Data-route conventions (app/sitemap.ts → /sitemap.xml,
        // app/robots.ts → /robots.txt) are renderable SSR routes — unlike
        // not-found/error/route, they must NOT be skipped by the matcher.
        let routes = vec![
            route("/", None),
            route("/sitemap.xml", Some("sitemap")),
            route("/robots.txt", Some("robots")),
        ];
        let sm = match_ssr_route("/sitemap.xml", &routes).expect("/sitemap.xml matches");
        assert_eq!(sm.route.kind.as_deref(), Some("sitemap"));
        let rb = match_ssr_route("/robots.txt", &routes).expect("/robots.txt matches");
        assert_eq!(rb.route.kind.as_deref(), Some("robots"));
        // The homepage still matches the page, not a data route.
        let home = match_ssr_route("/", &routes).expect("/ matches");
        assert_eq!(home.route.kind, None);
    }

    #[test]
    fn match_ssr_route_matches_og_image() {
        // Dynamic OG images (app/**/opengraph-image.tsx → kind:"og-image")
        // are renderable SSR routes served as PNG by the runner — they must
        // match like sitemap/robots, NOT be skipped like boundary kinds.
        let routes = vec![
            route("/", None),
            route("/opengraph-image", Some("og-image")),
            route("/blog/:slug", None),
            route("/blog/:slug/opengraph-image", Some("og-image")),
        ];
        let root = match_ssr_route("/opengraph-image", &routes).expect("/opengraph-image matches");
        assert_eq!(root.route.kind.as_deref(), Some("og-image"));
        // Per-page OG with a concrete param resolves the pattern + captures it.
        let slug = match_ssr_route("/blog/hello/opengraph-image", &routes)
            .expect("/blog/hello/opengraph-image matches");
        assert_eq!(slug.route.kind.as_deref(), Some("og-image"));
        assert_eq!(slug.params.get("slug").map(String::as_str), Some("hello"));
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
        assert_eq!(
            m2.params.get("filters").map(String::as_str),
            Some("red/small")
        );
    }

    #[test]
    fn match_ssr_route_specificity_static_beats_catch_all() {
        // Routes arrive pre-sorted by discoverAppRoutes: static, then param,
        // then catch-all. First match wins, so specificity is honored.
        let routes = vec![
            route("/docs/api", None),      // static
            route("/docs/:section", None), // dynamic param
            route("/docs/*slug", None),    // catch-all
        ];
        // Exact static wins over both dynamic and catch-all.
        let exact = match_ssr_route("/docs/api", &routes).unwrap();
        assert_eq!(exact.route.path, "/docs/api");
        // Single non-static segment → dynamic param wins over catch-all.
        let dyn_ = match_ssr_route("/docs/guides", &routes).unwrap();
        assert_eq!(dyn_.route.path, "/docs/:section");
        assert_eq!(
            dyn_.params.get("section").map(String::as_str),
            Some("guides")
        );
        // Multi-segment → only the catch-all can match.
        let deep = match_ssr_route("/docs/guides/intro", &routes).unwrap();
        assert_eq!(deep.route.path, "/docs/*slug");
        assert_eq!(
            deep.params.get("slug").map(String::as_str),
            Some("guides/intro")
        );
    }

    #[test]
    fn public_file_beats_dynamic_segments_but_not_static_routes() {
        // Next semantics: a real file under `public/` wins over a route
        // match that consumed dynamic segments (`[orgSlug]` binding
        // "icon.svg"), while a fully-static route path keeps beating
        // files (Next forbids that collision, so no ambiguity).
        let dir = std::env::temp_dir().join(format!("pylon-public-{}", std::process::id()));
        let public = dir.join("public");
        std::fs::create_dir_all(&public).unwrap();
        std::fs::write(public.join("icon.svg"), b"<svg/>").unwrap();

        let routes = vec![route("/:orgSlug", None), route("/icon.svg", None)];

        // Dynamic match on a URL where the file exists → yield to file.
        let dynamic = match_ssr_route("/icon.svg", &[routes[0].clone()]).unwrap();
        assert!(!dynamic.params.is_empty());
        let hit = dynamic_match_public_override(&public, &dynamic.params, "/icon.svg");
        assert!(
            hit.is_some(),
            "a [param] match must yield to an existing public/ file"
        );

        // Dynamic match where no file exists → route keeps it.
        let dynamic = match_ssr_route("/some-org", &[routes[0].clone()]).unwrap();
        assert!(
            dynamic_match_public_override(&public, &dynamic.params, "/some-org").is_none(),
            "no file on disk — the dynamic route must render"
        );

        // Static route path match (no params) → route wins even though
        // the file exists.
        let static_match = match_ssr_route("/icon.svg", &routes).unwrap();
        assert_eq!(static_match.route.path, "/:orgSlug"); // first-match table order
        let fully_static = match_ssr_route("/icon.svg", &[routes[1].clone()]).unwrap();
        assert!(fully_static.params.is_empty());
        assert!(
            dynamic_match_public_override(&public, &fully_static.params, "/icon.svg").is_none(),
            "a fully-static route path keeps precedence over public/ files"
        );

        // Catch-all match yields too — params are never empty for `*name`.
        let ca = match_ssr_route("/icon.svg", &[route("/*rest", None)]).unwrap();
        assert!(
            dynamic_match_public_override(&public, &ca.params, "/icon.svg").is_some(),
            "a catch-all match must yield to an existing public/ file"
        );

        std::fs::remove_dir_all(&dir).ok();
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
    fn match_form_route_matches_route_handlers_only() {
        // A page and a route.ts handler can share a path: GET → page, POST → handler.
        let routes = vec![
            route("/notes", None),          // page
            route("/notes", Some("route")), // route.ts handler
            route("/hooks/*rest", Some("route")),
        ];
        // GET matcher returns the PAGE, never the handler.
        let g = match_ssr_route("/notes", &routes).expect("page matches GET");
        assert_eq!(g.route.kind, None);
        // Form matcher returns the kind:"route" handler.
        let f = match_form_route("/notes", &routes).expect("handler matches non-GET");
        assert_eq!(f.route.kind.as_deref(), Some("route"));
        // Catch-all route handler captures the rest.
        let c = match_form_route("/hooks/stripe/x", &routes).expect("catch-all handler");
        assert_eq!(c.params.get("rest").map(String::as_str), Some("stripe/x"));
        // No handler at an unknown path.
        assert!(match_form_route("/missing", &routes).is_none());
        // A bare page path has no form handler.
        let only_page = vec![route("/about", None)];
        assert!(match_form_route("/about", &only_page).is_none());
    }

    #[test]
    fn parse_urlencoded_form_repeats_encoding_and_empty() {
        let v = parse_urlencoded_form(b"body=hello+world&tag=a&tag=b&empty=");
        assert_eq!(v["body"], serde_json::json!("hello world")); // + → space
        assert_eq!(v["tag"], serde_json::json!(["a", "b"])); // repeated → array
        assert_eq!(v["empty"], serde_json::json!("")); // empty value
                                                       // percent-decoding of reserved chars.
        let v2 = parse_urlencoded_form(b"q=a%26b%3Dc");
        assert_eq!(v2["q"], serde_json::json!("a&b=c"));
        // empty body → empty object.
        assert_eq!(parse_urlencoded_form(b""), serde_json::json!({}));
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
    fn builtin_error_page_is_html_not_json() {
        // A rate-limited browser navigation must get a styled HTML page, never
        // the raw API JSON envelope. Regression for "{"error":{"code":
        // "RATE_LIMITED"...}}" showing as a full page on store.pyln.dev.
        let html = builtin_error_page_html(
            429,
            "Too many requests",
            "You've made too many requests. Please wait 8 seconds and try again.",
        );
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("429"));
        assert!(html.contains("Too many requests"));
        assert!(html.contains("Please wait 8 seconds"));
        // It's a real page (has a back-home link), not a JSON blob.
        assert!(html.contains("Back home"));
        assert!(!html.contains("\"error\""));
        assert!(!html.contains("RATE_LIMITED"));
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
    fn markdown_variant_is_chosen_only_when_the_client_asks() {
        use crate::markdown::Representation;
        let md = |v: &PageVariant| match v {
            PageVariant::Markdown(m) => Some(m.clone()),
            _ => None,
        };
        // A browser is untouched.
        assert!(md(&page_variant_for(
            false,
            "/product",
            Some("text/html,*/*;q=0.8"),
            false
        ))
        .is_none());
        // An agent asking for markdown gets it, rendered at the page's own URL.
        let m = md(&page_variant_for(
            false,
            "/product?ref=x",
            Some("text/markdown"),
            false,
        ))
        .expect("markdown");
        assert_eq!(m.render_url, "/product?ref=x");
        assert!(!m.explicit_url);
        assert!(!m.html_acceptable);
        // The `.md` URL renders the page path, query preserved.
        let m = md(&page_variant_for(false, "/product.md?ref=x", None, false)).expect("markdown");
        assert_eq!(m.render_url, "/product?ref=x");
        assert!(m.explicit_url);
        assert_eq!(m.representation, Representation::Markdown);
        // A client-router navigation always wants the hydration payload.
        assert!(md(&page_variant_for(
            true,
            "/product",
            Some("text/markdown"),
            false
        ))
        .is_none());
        // A real file under public/ keeps its URL — `public/pylon-skill.md`
        // must not be shadowed by a `/pylon-skill` page's variant.
        assert!(md(&page_variant_for(false, "/pylon-skill.md", None, true)).is_none());
        // Nothing acceptable at all.
        assert!(matches!(
            page_variant_for(false, "/product", Some("image/png"), false),
            PageVariant::NotAcceptable
        ));
    }

    #[test]
    fn every_ssr_response_varies_on_accept() {
        let page = std::collections::HashMap::new();
        let pairs = header_pairs(&build_ssr_response_headers(
            &page,
            "https://app.example",
            true,
            false,
            VariantHeaders::default(),
        ));
        assert!(
            pairs.iter().any(|(k, v)| k == "vary" && v == "Accept"),
            "{pairs:?}"
        );
    }

    #[test]
    fn a_page_set_vary_is_merged_not_duplicated() {
        let mut page = std::collections::HashMap::new();
        page.insert("vary".to_string(), "Cookie".to_string());
        let pairs = header_pairs(&build_ssr_response_headers(
            &page,
            "https://app.example",
            true,
            false,
            VariantHeaders::default(),
        ));
        let varys: Vec<&String> = pairs
            .iter()
            .filter(|(k, _)| k == "vary")
            .map(|(_, v)| v)
            .collect();
        assert_eq!(varys.len(), 1, "exactly one Vary: {pairs:?}");
        assert_eq!(varys[0], "Cookie, Accept");
        // A page that already said Accept is left alone.
        let mut page = std::collections::HashMap::new();
        page.insert("vary".to_string(), "Accept, Cookie".to_string());
        let pairs = header_pairs(&build_ssr_response_headers(
            &page,
            "https://app.example",
            true,
            false,
            VariantHeaders::default(),
        ));
        assert_eq!(
            pairs
                .iter()
                .filter(|(k, v)| k == "vary" && v == "Accept, Cookie")
                .count(),
            1
        );
    }

    #[test]
    fn html_advertises_its_markdown_twin_and_markdown_replaces_the_content_type() {
        let page = std::collections::HashMap::new();
        let html = header_pairs(&build_ssr_response_headers(
            &page,
            "https://app.example",
            true,
            false,
            VariantHeaders {
                content_type: None,
                alternate_md: Some("/product.md"),
            },
        ));
        assert!(html
            .iter()
            .any(|(k, v)| k == "link"
                && v == "</product.md>; rel=\"alternate\"; type=\"text/markdown\""));
        assert!(html
            .iter()
            .any(|(k, v)| k == "content-type" && v.starts_with("text/html")));

        // The markdown response carries the converted type, and does NOT point
        // at itself as an alternate.
        let md = header_pairs(&build_ssr_response_headers(
            &page,
            "https://app.example",
            false,
            false,
            VariantHeaders {
                content_type: Some("text/markdown; charset=utf-8"),
                alternate_md: None,
            },
        ));
        assert_eq!(
            md.iter().filter(|(k, _)| k == "content-type").count(),
            1,
            "{md:?}"
        );
        assert!(md
            .iter()
            .any(|(k, v)| k == "content-type" && v == "text/markdown; charset=utf-8"));
        assert!(!md.iter().any(|(k, _)| k == "link"));
    }

    #[test]
    fn a_page_set_content_type_loses_to_the_converted_body() {
        // The page said "text/html"; the runtime converted it. Sending both
        // would be a response-splitting-adjacent contradiction.
        let mut page = std::collections::HashMap::new();
        page.insert(
            "content-type".to_string(),
            "text/html; charset=utf-8".to_string(),
        );
        let pairs = header_pairs(&build_ssr_response_headers(
            &page,
            "https://app.example",
            false,
            false,
            VariantHeaders {
                content_type: Some("text/plain; charset=utf-8"),
                alternate_md: None,
            },
        ));
        let cts: Vec<&String> = pairs
            .iter()
            .filter(|(k, _)| k == "content-type")
            .map(|(_, v)| v)
            .collect();
        assert_eq!(cts, vec!["text/plain; charset=utf-8"]);
    }

    #[test]
    fn markdown_and_html_never_share_a_cache_entry() {
        // Same reasoning as the nav/html split: one URL, two representations.
        // A shared key would hand an agent the HTML or a browser the markdown.
        let key = |vary: &[(String, String)]| crate::ssr_cache::cache_key("/p", "/p", vary);
        assert_ne!(
            key(&ssr_cache_vary(Some("a.example.com"), false, false)),
            key(&ssr_cache_vary(Some("a.example.com"), false, true)),
            "anon lane: html and markdown are distinct entries"
        );
        assert_ne!(
            key(&ssr_cache_bucket_vary(
                Some("a.example.com"),
                true,
                false,
                false
            )),
            key(&ssr_cache_bucket_vary(
                Some("a.example.com"),
                true,
                false,
                true
            )),
            "bucket lane: html and markdown are distinct entries"
        );
        // Existing HTML entries keep their key across the upgrade — the pair is
        // only appended for markdown.
        assert_eq!(
            key(&ssr_cache_vary(Some("a.example.com"), true, false)),
            key(&[
                (
                    "host".to_string(),
                    ssr_cache_host_bucket(Some("a.example.com"))
                ),
                ("nav".to_string(), "1".to_string()),
            ])
        );
    }

    #[test]
    fn markdown_files_are_served_as_markdown() {
        // A committed `public/*.md` used to go out as application/octet-stream,
        // which agents download instead of read.
        assert_eq!(
            content_type_for(Path::new("/app/public/pylon-skill.md")),
            "text/markdown; charset=utf-8"
        );
    }

    #[test]
    fn ssr_headers_inject_defaults_and_preserve_page_headers() {
        let mut page = std::collections::HashMap::new();
        page.insert("x-custom".to_string(), "hi".to_string());
        let pairs = header_pairs(&build_ssr_response_headers(
            &page,
            "https://app.example",
            true,
            false,
            VariantHeaders::default(),
        ));
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
        let pairs = header_pairs(&build_ssr_response_headers(
            &page,
            "*",
            true,
            false,
            VariantHeaders::default(),
        ));
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
        let pairs = header_pairs(&build_ssr_response_headers(
            &page,
            "*",
            true,
            false,
            VariantHeaders::default(),
        ));
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
            header_pairs(&build_ssr_response_headers(
                page,
                "*",
                true,
                false,
                VariantHeaders::default(),
            ))
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
        custom.insert(
            "cache-control".to_string(),
            "private, max-age=0".to_string(),
        );
        assert_eq!(cc(&custom).as_deref(), Some("private, max-age=0"));
    }

    #[test]
    fn ssr_cacheable_proof_yields_public_smaxage_and_is_stripped() {
        let headers = |page: &std::collections::HashMap<String, String>| {
            header_pairs(&build_ssr_response_headers(
                page,
                "*",
                true,
                false,
                VariantHeaders::default(),
            ))
        };
        let cc = |page: &std::collections::HashMap<String, String>| {
            headers(page)
                .into_iter()
                .find(|(k, _)| k == "cache-control")
                .map(|(_, v)| v)
        };

        // #277: a proven-anonymous render (x-pylon-cacheable: N) → public,
        // s-maxage=N, stale-while-revalidate=N — and the internal proof header
        // NEVER reaches the client/CDN.
        let mut anon_cacheable = std::collections::HashMap::new();
        anon_cacheable.insert("x-pylon-cacheable".to_string(), "60".to_string());
        assert_eq!(
            cc(&anon_cacheable).as_deref(),
            Some("public, s-maxage=60, stale-while-revalidate=60")
        );
        assert!(
            !headers(&anon_cacheable)
                .iter()
                .any(|(k, _)| k == "x-pylon-cacheable"),
            "the internal proof header must be stripped"
        );

        // Set-Cookie is an ABSOLUTE veto — even with the cacheable proof, a
        // personalized response is no-store (never shared).
        let mut cacheable_but_cookie = std::collections::HashMap::new();
        cacheable_but_cookie.insert("x-pylon-cacheable".to_string(), "60".to_string());
        cacheable_but_cookie.insert("set-cookie".to_string(), "sid=abc".to_string());
        assert_eq!(cc(&cacheable_but_cookie).as_deref(), Some("no-store"));

        // No proof ⇒ fail closed (no-cache), never accidentally public.
        let no_proof = std::collections::HashMap::new();
        assert_eq!(cc(&no_proof).as_deref(), Some("no-cache"));
    }

    #[test]
    fn ssr_cacheable_proof_not_public_for_unshareable_request() {
        // P0 (codex 2026-06-28): the #277 proof means "this RENDER is anonymous-
        // safe", NOT "this REQUEST is anonymous". A logged-in request still
        // carries the user's real `auth` in the hydration tail, so even with the
        // proof its response must NEVER be advertised `public` — else a shared
        // cache (Cloudflare) could replay one user's identity to another.
        let cc = |page: &std::collections::HashMap<String, String>, shareable: bool| {
            header_pairs(&build_ssr_response_headers(
                page,
                "*",
                shareable,
                false,
                VariantHeaders::default(),
            ))
            .into_iter()
            .find(|(k, _)| k == "cache-control")
            .map(|(_, v)| v)
        };
        let mut proven = std::collections::HashMap::new();
        proven.insert("x-pylon-cacheable".to_string(), "60".to_string());
        // Shareable (cookie-anonymous, GET, no query) request → public.
        assert_eq!(
            cc(&proven, true).as_deref(),
            Some("public, s-maxage=60, stale-while-revalidate=60")
        );
        // NOT shareable (logged-in / query / non-GET) → never public; the proof
        // is downgraded to a private, non-stored response. Without the fix this
        // would still be `public, s-maxage=60` and leak the tail's auth.
        assert_eq!(cc(&proven, false).as_deref(), Some("private, no-store"));
    }

    #[test]
    fn ssr_bucket_proof_is_private_by_default_public_only_with_cdn_bucketing() {
        // PPR Phase 0: a `x-pylon-bucket` render carries an identity-free body, so
        // it's safe to SHARE within its session bucket — but only if the CDN keys
        // on session presence (`bucket_shareable`). Default (bucket_shareable
        // false) → browser-`private`/no-store so a cookie-blind shared cache can't
        // mis-serve across buckets. And the internal proof never leaks.
        let cc = |page: &std::collections::HashMap<String, String>,
                  req_shareable: bool,
                  bucket_shareable: bool| {
            header_pairs(&build_ssr_response_headers(
                page,
                "*",
                req_shareable,
                bucket_shareable,
                VariantHeaders::default(),
            ))
            .into_iter()
            .find(|(k, _)| k == "cache-control")
            .map(|(_, v)| v)
        };
        let mut bucket = std::collections::HashMap::new();
        bucket.insert("x-pylon-bucket".to_string(), "60".to_string());

        // Default (no CDN bucketing): private, no-store — even for a signed-out
        // (request_shareable=true) request, because a cookie-blind CDN would
        // otherwise serve this sess-keyed shell to the wrong bucket.
        assert_eq!(
            cc(&bucket, true, false).as_deref(),
            Some("private, no-store")
        );
        // Signed-in (request_shareable=false) request, no CDN bucketing → same.
        assert_eq!(
            cc(&bucket, false, false).as_deref(),
            Some("private, no-store")
        );
        // CDN bucketing ON → public, shared-within-bucket (the CDN keys on the
        // cookie-presence so it can't cross buckets). Survives the may_share
        // invariant even for a session-carrying (request_shareable=false) request.
        assert_eq!(
            cc(&bucket, false, true).as_deref(),
            Some("public, s-maxage=60, stale-while-revalidate=60")
        );

        // The internal proof header NEVER reaches the client/CDN.
        assert!(
            !header_pairs(&build_ssr_response_headers(
                &bucket,
                "*",
                false,
                true,
                VariantHeaders::default()
            ))
            .iter()
            .any(|(k, _)| k == "x-pylon-bucket"),
            "the internal bucket proof must be stripped"
        );

        // Set-Cookie is an ABSOLUTE veto — a bucket render that set a cookie is
        // never shared, even with CDN bucketing on.
        let mut bucket_cookie = std::collections::HashMap::new();
        bucket_cookie.insert("x-pylon-bucket".to_string(), "60".to_string());
        bucket_cookie.insert("set-cookie".to_string(), "sid=abc".to_string());
        assert_eq!(cc(&bucket_cookie, false, true).as_deref(), Some("no-store"));

        // P0 follow-up (codex round-2 #5): a bucket page that sets its OWN public
        // Cache-Control must NOT escape the bucket policy. For a signed-OUT
        // request (request_shareable=true) with CDN bucketing OFF, the page-set
        // `public` would otherwise survive and let a cookie-blind CDN serve the
        // sess=0 shell to a signed-in visitor. A bucket render's shareability is
        // governed ONLY by bucket_shareable.
        let mut bucket_pub = std::collections::HashMap::new();
        bucket_pub.insert("x-pylon-bucket".to_string(), "60".to_string());
        bucket_pub.insert(
            "cache-control".to_string(),
            "public, max-age=300".to_string(),
        );
        assert_eq!(
            cc(&bucket_pub, true, false).as_deref(),
            Some("private, no-store")
        );
        // With CDN bucketing ON, the page-set value is honored (the CDN keys on
        // session presence so it can't cross buckets).
        assert_eq!(
            cc(&bucket_pub, true, true).as_deref(),
            Some("public, max-age=300")
        );
    }

    #[test]
    fn ssr_bucket_verdict_only_stores_proven_identity_free_clean_200() {
        let mk = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<std::collections::HashMap<String, String>>()
        };
        // Proof + clean 200 + no Set-Cookie → Some(secs).
        assert_eq!(
            ssr_bucket_verdict(200, &mk(&[("x-pylon-bucket", "60")])),
            Some(60)
        );
        // The anon proof does NOT satisfy the bucket verdict (distinct header).
        assert_eq!(
            ssr_bucket_verdict(200, &mk(&[("x-pylon-cacheable", "60")])),
            None
        );
        // Set-Cookie veto.
        assert_eq!(
            ssr_bucket_verdict(200, &mk(&[("x-pylon-bucket", "60"), ("set-cookie", "s=x")])),
            None
        );
        // Non-200 veto.
        assert_eq!(
            ssr_bucket_verdict(404, &mk(&[("x-pylon-bucket", "60")])),
            None
        );
        // No proof / zero / garbage → fail-closed None.
        assert_eq!(ssr_bucket_verdict(200, &mk(&[])), None);
        assert_eq!(
            ssr_bucket_verdict(200, &mk(&[("x-pylon-bucket", "0")])),
            None
        );
        assert_eq!(
            ssr_bucket_verdict(200, &mk(&[("x-pylon-bucket", "nope")])),
            None
        );
    }

    #[test]
    fn cache_write_plan_never_stores_a_signed_in_anon_proof_under_the_shared_key() {
        // P0 (codex 2026-06-28): the write-tee now fires for bucket-eligible
        // requests (session allowed). A signed-in request to an ANON-opted page
        // (`export const revalidate`) that didn't read auth emits
        // `x-pylon-cacheable`, but its NON-bucket hydration tail carries the
        // user's real identity. It must NOT be written under the shared anon key.
        let mk = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<std::collections::HashMap<String, String>>()
        };
        let anon = mk(&[("x-pylon-cacheable", "60")]);
        let bucket = mk(&[("x-pylon-bucket", "60")]);

        // Anon proof: stored ONLY for a cookie-anonymous (cacheable_eligible) req.
        assert_eq!(
            cache_write_plan(true, 200, &anon),
            Some((60, CacheWriteLane::Anon))
        );
        // THE LEAK GATE: signed-in (cacheable_eligible=false) + anon proof → no
        // write. Without the fix this stored a signed-in user's identity under the
        // shared anon key, replaying it to anonymous visitors.
        assert_eq!(cache_write_plan(false, 200, &anon), None);

        // Bucket proof: stored regardless of cookie-anonymity (body is identity-
        // free; the bucket key separates signed-in/out).
        assert_eq!(
            cache_write_plan(false, 200, &bucket),
            Some((60, CacheWriteLane::Bucket))
        );
        assert_eq!(
            cache_write_plan(true, 200, &bucket),
            Some((60, CacheWriteLane::Bucket))
        );

        // No proof → never stored.
        assert_eq!(cache_write_plan(true, 200, &mk(&[])), None);
    }

    #[test]
    fn nav_payload_and_html_never_share_a_cache_entry() {
        // The same URL answers with HTML for a document load and with JSON for
        // a client-side navigation. If they shared a key, one would be served
        // where the other is expected: a navigation painting a raw payload, or
        // a crawler handed JSON instead of a page.
        let key = |vary: &[(String, String)]| crate::ssr_cache::cache_key("/p", "/p", vary);
        assert_ne!(
            key(&ssr_cache_vary(Some("a.example.com"), false, false)),
            key(&ssr_cache_vary(Some("a.example.com"), true, false)),
            "anon lane: html and nav are distinct entries"
        );
        assert_ne!(
            key(&ssr_cache_bucket_vary(
                Some("a.example.com"),
                true,
                false,
                false
            )),
            key(&ssr_cache_bucket_vary(
                Some("a.example.com"),
                true,
                true,
                false
            )),
            "bucket lane: html and nav are distinct entries"
        );
        // And the nav dimension must not collapse the dimensions already there.
        assert_ne!(
            key(&ssr_cache_bucket_vary(
                Some("a.example.com"),
                true,
                true,
                false
            )),
            key(&ssr_cache_bucket_vary(
                Some("a.example.com"),
                false,
                true,
                false
            )),
            "session presence still separates nav entries"
        );
        // Loopback hosts each get their own bucket (untrusted public hosts
        // deliberately collapse to one, so a Host sprayer can't multiply
        // entries — see ssr_cache_host_bucket).
        assert_ne!(
            key(&ssr_cache_vary(Some("127.0.0.1"), true, false)),
            key(&ssr_cache_vary(Some("localhost"), true, false)),
            "host still separates nav entries"
        );
    }

    #[test]
    fn bucket_vary_separates_session_presence_and_never_collides_with_anon() {
        // The session dimension actually changes the cache key, so a signed-in and
        // a signed-out request land on DIFFERENT bucket entries (different shells)
        // — and a bucket entry can never collide with an anon entry for the same
        // route (distinct vary shape).
        let key = |vary: &[(String, String)]| crate::ssr_cache::cache_key("/p", "/p", vary);
        let anon = ssr_cache_vary(Some("a.example.com"), false, false);
        let b_in = ssr_cache_bucket_vary(Some("a.example.com"), true, false, false);
        let b_out = ssr_cache_bucket_vary(Some("a.example.com"), false, false, false);
        assert_ne!(
            key(&b_in),
            key(&b_out),
            "sess=1 and sess=0 are distinct entries"
        );
        assert_ne!(
            key(&anon),
            key(&b_in),
            "anon and bucket entries never collide"
        );
        assert_ne!(
            key(&anon),
            key(&b_out),
            "anon and bucket entries never collide"
        );
        // Same inputs → same key (deterministic, so read/write/stale agree).
        assert_eq!(
            key(&ssr_cache_bucket_vary(
                Some("a.example.com"),
                true,
                false,
                false
            )),
            key(&b_in)
        );
    }

    #[test]
    fn ssr_page_set_public_cache_control_is_downgraded_for_unshareable_request() {
        // P0 follow-up (codex 2026-06-28): a page that sets its OWN Cache-Control
        // must NOT be able to advertise shared caching (`public`/`s-maxage`) for a
        // non-shareable (logged-in) request — its hydration tail carries that
        // user's auth. Respected when shareable, downgraded when not. Without the
        // fix, a page-set `cache-control` set `saw_cache_control` and skipped the
        // shareability gate entirely.
        let cc = |page: &std::collections::HashMap<String, String>, shareable: bool| {
            header_pairs(&build_ssr_response_headers(
                page,
                "*",
                shareable,
                false,
                VariantHeaders::default(),
            ))
            .into_iter()
            .find(|(k, _)| k == "cache-control")
            .map(|(_, v)| v)
        };
        let mut pub_page = std::collections::HashMap::new();
        pub_page.insert(
            "cache-control".to_string(),
            "public, max-age=300".to_string(),
        );
        assert_eq!(cc(&pub_page, true).as_deref(), Some("public, max-age=300"));
        assert_eq!(cc(&pub_page, false).as_deref(), Some("private, no-store"));

        // A page's NON-shared directive is respected even for an unshareable req.
        let mut priv_page = std::collections::HashMap::new();
        priv_page.insert(
            "cache-control".to_string(),
            "private, max-age=0".to_string(),
        );
        assert_eq!(cc(&priv_page, false).as_deref(), Some("private, max-age=0"));

        // Bare `max-age` (no `private`) is ALSO shared-storable per HTTP → it
        // must be downgraded for an unshareable request, not just `public`.
        let mut bare = std::collections::HashMap::new();
        bare.insert("cache-control".to_string(), "max-age=300".to_string());
        assert_eq!(cc(&bare, false).as_deref(), Some("private, no-store"));

        // A Set-Cookie (personalized) response can't be made shared-cacheable by
        // a page-set `public`, even on an otherwise-shareable request.
        let mut pub_cookie = std::collections::HashMap::new();
        pub_cookie.insert(
            "cache-control".to_string(),
            "public, max-age=300".to_string(),
        );
        pub_cookie.insert("set-cookie".to_string(), "sid=abc; HttpOnly".to_string());
        assert_eq!(cc(&pub_cookie, true).as_deref(), Some("private, no-store"));

        // Any forged `x-pylon-*` from page headers is stripped, never emitted.
        let mut forged = std::collections::HashMap::new();
        forged.insert("x-pylon-foo".to_string(), "1".to_string());
        let pairs = header_pairs(&build_ssr_response_headers(
            &forged,
            "*",
            true,
            false,
            VariantHeaders::default(),
        ));
        assert!(!pairs.iter().any(|(k, _)| k.starts_with("x-pylon-")));
    }

    #[test]
    fn ssr_unshareable_response_is_never_shared_cacheable() {
        // codex round-3 (RFC 9111): the invariant is "a non-shareable response
        // must not be storable by a SHARED cache". Only an UNQUALIFIED `private`
        // or `no-store` is safe; everything else (default no-cache, bare max-age,
        // max-age=0, must-revalidate, field-qualified private=) is shared-storable
        // and must be forced to `private, no-store` when request_shareable=false.
        let cc = |page_cc: Option<&str>| {
            let mut page = std::collections::HashMap::new();
            if let Some(v) = page_cc {
                page.insert("cache-control".to_string(), v.to_string());
            }
            header_pairs(&build_ssr_response_headers(
                &page,
                "*",
                false,
                false,
                VariantHeaders::default(),
            ))
            .into_iter()
            .find(|(k, _)| k == "cache-control")
            .map(|(_, v)| v)
        };
        // No page CC at all → the default must NOT be a bare `no-cache` (shared-
        // storable); it's forced to private,no-store for an unshareable request.
        assert_eq!(cc(None).as_deref(), Some("private, no-store"));
        for shared_storable in [
            "no-cache",
            "max-age=300",
            "max-age=0",
            "must-revalidate",
            "PUBLIC, max-age=60",         // mixed case
            "private=\"x\", max-age=300", // field-qualified private is NOT safe
        ] {
            assert_eq!(
                cc(Some(shared_storable)).as_deref(),
                Some("private, no-store"),
                "{shared_storable:?} must be downgraded for an unshareable request",
            );
        }
        // A page's OWN explicitly-non-shared directive is respected (the user's
        // browser may still privately cache their authed page).
        assert_eq!(
            cc(Some("private, max-age=60")).as_deref(),
            Some("private, max-age=60")
        );
        assert_eq!(cc(Some("no-store")).as_deref(), Some("no-store"));
    }

    #[test]
    fn ssr_cache_verdict_only_stores_proven_anonymous_clean_200() {
        let mk = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<std::collections::HashMap<String, String>>()
        };

        // Proof + clean 200 + no cookie → cache for the TTL.
        assert_eq!(
            ssr_cache_verdict(200, &mk(&[("x-pylon-cacheable", "60")])),
            Some(60)
        );

        // Set-Cookie vetoes (personalized) even with the proof.
        assert_eq!(
            ssr_cache_verdict(
                200,
                &mk(&[("x-pylon-cacheable", "60"), ("set-cookie", "sid=x")])
            ),
            None
        );
        // Non-200 is never cached (404/redirect/error).
        assert_eq!(
            ssr_cache_verdict(404, &mk(&[("x-pylon-cacheable", "60")])),
            None
        );
        assert_eq!(
            ssr_cache_verdict(307, &mk(&[("x-pylon-cacheable", "60")])),
            None
        );
        // No proof / zero TTL / garbage TTL → fail closed.
        assert_eq!(ssr_cache_verdict(200, &mk(&[])), None);
        assert_eq!(
            ssr_cache_verdict(200, &mk(&[("x-pylon-cacheable", "0")])),
            None
        );
        assert_eq!(
            ssr_cache_verdict(200, &mk(&[("x-pylon-cacheable", "nope")])),
            None
        );
    }

    #[test]
    fn cache_host_bucket_trusts_only_allowlisted_and_loopback_hosts() {
        // Drives global env; CI runs --test-threads=1 so this is safe.
        std::env::remove_var("PYLON_PUBLIC_URL");
        std::env::remove_var("PYLON_CANONICAL_HOST");
        std::env::set_var("PYLON_TRUSTED_HOSTS", "www.example.com, apex.example.com");
        std::env::set_var("PYLON_CANONICAL_HOST", "canon.example.com");
        std::env::set_var("PYLON_PUBLIC_URL", "https://public.example.com:8443/base");

        // Allowlisted hosts get their OWN bucket (they bake distinct absolute
        // URLs and must not share a cache entry).
        assert_eq!(
            ssr_cache_host_bucket(Some("www.example.com")),
            "www.example.com"
        );
        assert_eq!(
            ssr_cache_host_bucket(Some("apex.example.com")),
            "apex.example.com"
        );
        // PYLON_PUBLIC_URL contributes its host:port (parsed from the URL).
        assert_eq!(
            ssr_cache_host_bucket(Some("public.example.com:8443")),
            "public.example.com:8443"
        );
        // PYLON_CANONICAL_HOST is trusted too.
        assert_eq!(
            ssr_cache_host_bucket(Some("canon.example.com")),
            "canon.example.com"
        );
        // Case-insensitive match.
        assert_eq!(
            ssr_cache_host_bucket(Some("WWW.Example.COM")),
            "www.example.com"
        );
        // Loopback always trusted (dev).
        assert_eq!(
            ssr_cache_host_bucket(Some("localhost:4321")),
            "localhost:4321"
        );
        assert_eq!(
            ssr_cache_host_bucket(Some("127.0.0.1:4321")),
            "127.0.0.1:4321"
        );

        // UNtrusted / spoofed / absent hosts ALL collapse to the "" bucket —
        // they render identical canonical-origin URLs (so sharing is correct)
        // and an attacker spraying distinct Host values can't multiply entries.
        assert_eq!(ssr_cache_host_bucket(Some("evil.com")), "");
        assert_eq!(ssr_cache_host_bucket(Some("attacker.example.net")), "");
        assert_eq!(ssr_cache_host_bucket(None), "");
        assert_eq!(ssr_cache_host_bucket(Some("")), "");

        std::env::remove_var("PYLON_TRUSTED_HOSTS");
        std::env::remove_var("PYLON_CANONICAL_HOST");
        std::env::remove_var("PYLON_PUBLIC_URL");
    }

    #[test]
    fn cache_key_host_dimension_separates_distinct_buckets() {
        // The mechanism #347 relies on: the host bucket actually changes the
        // ISR cache key, so two trusted hosts land on DIFFERENT entries (each
        // serves its own absolute URLs) while everything in the "" untrusted
        // bucket shares ONE entry. Env-free — bucket *derivation* from the
        // allowlist is covered by
        // `cache_host_bucket_trusts_only_allowlisted_and_loopback_hosts`.
        let k = |bucket: &str| {
            crate::ssr_cache::cache_key("/p", "/p", &[("host".to_string(), bucket.to_string())])
        };
        assert_ne!(
            k("a.example.com"),
            k("b.example.com"),
            "distinct trusted host buckets → distinct cache keys"
        );
        assert_ne!(
            k("a.example.com"),
            k(""),
            "trusted host vs the untrusted bucket differ"
        );
        assert_eq!(k(""), k(""), "the untrusted bucket is one shared entry");
    }

    #[test]
    fn stale_on_error_prefers_a_stale_cached_entry_over_an_error() {
        // Reliability: when the live render can't produce a response, a page
        // that was EVER rendered should stay up. The fallback deliberately
        // returns a STALE entry (past its revalidate window) — a slightly-old
        // page beats a 503/500 — and only for cacheable-eligible requests.
        // Drives the real on-disk cache via the PYLON_SSR_CACHE_ROOT override
        // (CI runs `--test-threads=1`, so the global-env mutation is safe).
        let tmp = TempDir::new().unwrap();
        std::env::set_var("PYLON_SSR_CACHE_ROOT", tmp.path());
        std::env::set_var("PYLON_ARTIFACT_ID", "stale-on-error-test");

        let headers = vec![("content-type".to_string(), "text/html".to_string())];
        // TTL 0 → the entry is immediately STALE. Unique keys so the shared
        // process-wide in-memory cache layer can't collide with the ssr_cache
        // module's own tests under a parallel runner.
        let rp = "/__stale_oe__";
        crate::ssr_cache::put(rp, rp, &[], 200, &headers, b"<html>old</html>", 0);

        // Cacheable-eligible + a stale entry exists → it's a valid fallback,
        // and it's the STALE entry (the whole point of stale-on-error).
        let cand = stale_on_error_candidate(rp, rp, true, &[]).expect("stale entry is a candidate");
        assert!(!cand.fresh, "fallback is the stale entry");
        assert_eq!(cand.body, b"<html>old</html>");

        // NOT cacheable-eligible (authed / has query) → never serve a shared
        // anonymous page as a fallback, even though one is cached.
        assert!(stale_on_error_candidate(rp, rp, false, &[]).is_none());

        // Cacheable-eligible but nothing cached for this route → no fallback;
        // the caller emits its 503/500.
        assert!(stale_on_error_candidate("/__never_oe__", "/__never_oe__", true, &[]).is_none());

        std::env::remove_var("PYLON_SSR_CACHE_ROOT");
        std::env::remove_var("PYLON_ARTIFACT_ID");
    }

    #[test]
    fn cookie_header_named_detects_session_cookie_only() {
        // Present + non-empty → eligible-bypass true.
        assert!(cookie_header_has_named(
            "pylon_session=abc123",
            "pylon_session"
        ));
        assert!(cookie_header_has_named(
            "theme=dark; pylon_session=abc; foo=1",
            "pylon_session"
        ));
        // A DIFFERENT cookie (e.g. just a theme pref) is NOT a session — the
        // request stays cache-eligible.
        assert!(!cookie_header_has_named(
            "theme=dark; foo=1",
            "pylon_session"
        ));
        // Empty value (logged-out clear) is not a session.
        assert!(!cookie_header_has_named("pylon_session=", "pylon_session"));
        // A cookie whose name merely contains the session name doesn't match.
        assert!(!cookie_header_has_named(
            "not_pylon_session=x",
            "pylon_session"
        ));
    }

    #[test]
    fn ssr_headers_carry_security_defaults_overridable() {
        let pairs = header_pairs(&build_ssr_response_headers(
            &std::collections::HashMap::new(),
            "*",
            true,
            false,
            VariantHeaders::default(),
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
        let p2 = header_pairs(&build_ssr_response_headers(
            &page,
            "*",
            true,
            false,
            VariantHeaders::default(),
        ));
        let xfo: Vec<&String> = p2
            .iter()
            .filter(|(n, _)| n == "x-frame-options")
            .map(|(_, v)| v)
            .collect();
        assert_eq!(xfo.len(), 1, "no duplicate X-Frame-Options");
        assert_eq!(xfo[0], "ALLOWALL");
    }

    #[test]
    fn frame_ancestors_allowlist_parses_and_rejects_injection() {
        use super::parse_frame_ancestors;

        // Why this exists: Pylon Cloud's builder shows a running dev-mode env in
        // a live-preview iframe on another origin. The default
        // X-Frame-Options: SAMEORIGIN blocks that, and XFO has no way to name a
        // permitted cross origin (ALLOW-FROM is dead), so the allowlist has to
        // be emitted as CSP frame-ancestors instead.
        assert_eq!(
            parse_frame_ancestors("https://www.pylonsync.com").as_deref(),
            Some("https://www.pylonsync.com")
        );
        // Comma-, space-, and tab-separated all work; ports and wildcards too.
        assert_eq!(
            parse_frame_ancestors("https://a.com, http://localhost:4321\thttps://*.pyln.dev")
                .as_deref(),
            Some("https://a.com http://localhost:4321 https://*.pyln.dev")
        );

        // Nothing configured → None, so the caller keeps SAMEORIGIN.
        assert_eq!(parse_frame_ancestors(""), None);
        assert_eq!(parse_frame_ancestors("   "), None);

        // A source that could close the directive and append another one is
        // dropped — otherwise `PYLON_FRAME_ANCESTORS` becomes arbitrary CSP
        // injection (e.g. smuggling `script-src` into the policy).
        // `;` is not a separator, so the whole token fails validation and is
        // dropped — the origin does not survive in a "cleaned up" form. Failing
        // closed is deliberate: a half-honoured allowlist is worse than none.
        assert_eq!(
            parse_frame_ancestors("https://ok.com;script-src 'none'"),
            None,
            "a source carrying a directive separator must be rejected outright"
        );
        // A well-formed origin alongside a malformed one still works — only the
        // bad source is dropped.
        assert_eq!(
            parse_frame_ancestors("https://ok.com, https://bad.com;script-src").as_deref(),
            Some("https://ok.com")
        );
        assert_eq!(parse_frame_ancestors("'none'"), None);
        assert_eq!(parse_frame_ancestors("javascript:alert(1)"), None);
        assert_eq!(
            parse_frame_ancestors("example.com"),
            None,
            "scheme required"
        );
        assert_eq!(parse_frame_ancestors("https://"), None, "host required");
        assert_eq!(
            parse_frame_ancestors("https://a.com\r\nx-evil: 1"),
            None,
            "CR/LF must never survive into a header value"
        );
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
        let pairs = header_pairs(&build_ssr_response_headers(
            &page,
            "*",
            true,
            false,
            VariantHeaders::default(),
        ));
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
        let pairs = header_pairs(&build_ssr_response_headers(
            &page,
            "*",
            true,
            false,
            VariantHeaders::default(),
        ));
        assert!(pairs
            .iter()
            .all(|(k, _)| !k.contains('\r') && !k.contains('\n') && !k.contains(':')));
        assert!(
            !pairs.iter().any(|(_, v)| v == "stolen=1" || v == "v"),
            "values of unsafe-named headers must not leak"
        );
    }

    // --- SSR auth parity: `is_admin` must be resolved the same as the API path ---

    fn user_field(name: &str, ty: &str) -> pylon_kernel::ManifestField {
        pylon_kernel::ManifestField {
            name: name.into(),
            field_type: ty.into(),
            optional: true,
            unique: false,
            crdt: None,
            server_only: false,
            readonly: false,
            default: None,
            enum_values: None,
            encrypted: false,
            sync_omit: false,
        }
    }

    /// In-memory runtime with a `User` entity whose `isAdmin` bool field is the
    /// configured `adminField` (Path 1 of the admin-lift).
    fn admin_field_runtime() -> std::sync::Arc<crate::Runtime> {
        let mut manifest = pylon_kernel::AppManifest {
            manifest_version: 1,
            name: "ssr-admin-parity".into(),
            version: "0.1.0".into(),
            ..Default::default()
        };
        manifest.entities = vec![pylon_kernel::ManifestEntity {
            name: "User".into(),
            fields: vec![
                user_field("isAdmin", "bool"),
                user_field("email", "string"),
                user_field("emailVerified", "bool"),
            ],
            indexes: vec![],
            relations: vec![],
            search: None,
            crdt: false,
            sync: false,
            ..Default::default()
        }];
        manifest.auth.user.entity = "User".into();
        manifest.auth.user.admin_field = Some("isAdmin".into());
        std::sync::Arc::new(crate::Runtime::in_memory(manifest).unwrap())
    }

    /// Insert `user`, mint a session for it, and resolve SSR auth exactly as an
    /// SSR render would — returning the resolved `is_admin`.
    fn ssr_is_admin_for(runtime: std::sync::Arc<crate::Runtime>, user: serde_json::Value) -> bool {
        let uid = runtime.insert("User", &user).unwrap();
        let store = std::sync::Arc::new(pylon_auth::SessionStore::new());
        let session = store.create(uid);
        let cookie = std::sync::Arc::new(pylon_auth::CookieConfig {
            name: "sid".into(),
            domain: None,
            secure: false,
            same_site: pylon_auth::SameSite::Lax,
            max_age_secs: 3600,
            path: "/".into(),
        });
        let cfg = FrontendConfig::from_env(std::path::Path::new("."))
            .with_session(store, cookie)
            .with_runtime(runtime);
        let mut cookies = std::collections::HashMap::new();
        cookies.insert("sid".to_string(), session.token.clone());
        resolve_request_auth(&cfg, &cookies).is_admin
    }

    #[test]
    fn ssr_auth_lifts_is_admin_via_admin_field() {
        // Regression: `resolve_request_auth` never applied the admin-lift, so
        // SSR `is_admin` was ALWAYS false — diverging from the API/sync paths.
        // A User row whose configured `adminField` is truthy must now resolve
        // as admin on the SSR path too.
        let rt = admin_field_runtime();
        assert!(
            ssr_is_admin_for(
                rt,
                serde_json::json!({"isAdmin": true, "email": "a@x.test", "emailVerified": true}),
            ),
            "SSR auth must lift is_admin for a User whose adminField is truthy"
        );
    }

    #[test]
    fn ssr_auth_non_admin_stays_non_admin() {
        // Control: a plain User (adminField false, email unverified) must NOT
        // be lifted — the parity fix must not over-grant. Robust against a
        // polluted PYLON_ADMIN_EMAILS because the email is unverified.
        let rt = admin_field_runtime();
        assert!(
            !ssr_is_admin_for(
                rt,
                serde_json::json!({"isAdmin": false, "email": "b@x.test", "emailVerified": false}),
            ),
            "SSR auth must NOT grant is_admin to a non-admin User"
        );
    }

    #[test]
    fn is_spa_eligible_anchors_framework_prefixes() {
        // Framework routes + their subpaths are NOT SPA-eligible…
        for p in [
            "/studio",
            "/studio/x",
            "/events",
            "/metrics",
            "/health",
            "/api",
            "/api/x",
        ] {
            assert!(
                !is_spa_eligible(p),
                "{p} must route to the framework, not SPA"
            );
        }
        // …but sibling-prefixed SSR routes ARE (the bare starts_with bug 404'd
        // these).
        for p in ["/studios", "/eventsfeed", "/metrics-report", "/apiary", "/"] {
            assert!(is_spa_eligible(p), "{p} must be SPA-eligible");
        }
    }
}
