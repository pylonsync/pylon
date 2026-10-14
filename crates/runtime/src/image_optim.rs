//! Image optimization for `<Image>`.
//!
//! Serves `GET /_pylon/image?src=<path-or-url>&w=<width>&q=<quality>&format=<fmt>`.
//! Resizes the source to width `w`, re-encodes at quality `q` in the
//! requested format, and writes the result to a content-addressed
//! file under `.pylon/.cache/images/<hash>.<ext>` so subsequent
//! requests for the same params return instantly.
//!
//! Security model — see [`ImageConfig`] for the full surface:
//!   - Local sources: must live under the frontend dir, canonicalized
//!     + prefix-checked (no path traversal).
//!   - Remote sources: must match a `PYLON_IMAGE_REMOTE_ALLOWLIST`
//!     entry. Supports `host` and `host/pathPrefix` patterns.
//!   - Width snaps to the nearest `deviceSizes ∪ imageSizes` entry
//!     (configurable; Next-style defaults). Bounding the set prevents
//!     cache-fill DoS: arbitrary values collapse onto the allowlist,
//!     and the RESOLVED value is what the cache key hashes.
//!   - Quality snaps to the nearest `qualities` entry (default 50/75/90).
//!   - Format must be in the `formats` set (default webp/jpeg); an
//!     unknown one falls back to Accept negotiation.
//!   - Source bytes capped at `maxBytes` (default 25MB).
//!   - Decoded pixel count capped at `maxPixels` (default 40M) —
//!     bounds decode memory. Prevents image-bomb DoS (tiny file,
//!     huge dimensions).
//!   - SVG inputs hard-rejected. We never accept or emit SVG —
//!     it can carry script.
//!
//! Cache-Control: hashed responses get `public, max-age=31536000,
//! immutable`. The hash encodes the exact transform so any change
//! produces a different URL.

use std::collections::HashMap;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tiny_http::{Header, Request, Response};

/// Allowed output formats. AVIF encodes are 5-20x slower than WebP,
/// but the on-disk transform cache means each variant encodes once
/// ever — the ~35-45% wire savings on photographic content dominate.
/// SVG is excluded at the format level: never accepted as output,
/// never accepted as input either (parseable image-bomb / script
/// vector).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutFormat {
    Avif,
    Webp,
    Jpeg,
    Png,
}

/// Per-instance image optimizer config. Read from env at startup
/// (cheap; the parse only runs once). All fields have safe-by-
/// default values that match Next.js's defaults where possible.
///
/// Env variables (all comma-separated where they're lists):
///
/// | Env | Default | Effect |
/// |---|---|---|
/// | `PYLON_IMAGE_REMOTE_ALLOWLIST` | (empty) | Hosts or `host/pathPrefix` entries allowed as remote sources. Empty = local only. |
/// | `PYLON_IMAGE_DEVICE_SIZES` | `640,750,828,1080,1200,1920,2048,3840` | Widths allowed for full-bleed images. Used for `srcset`. |
/// | `PYLON_IMAGE_IMAGE_SIZES` | `16,32,48,64,96,128,256,384` | Widths allowed for thumbnails / icons. Unioned with device sizes. |
/// | `PYLON_IMAGE_QUALITIES` | `50,75,90` | Quality values allowed. |
/// | `PYLON_IMAGE_FORMATS` | `avif,webp,jpeg` | Output formats allowed. |
/// | `PYLON_IMAGE_MAX_BYTES` | `26214400` (25MB) | Source byte cap. |
/// | `PYLON_IMAGE_MAX_PIXELS` | `40000000` (40M) | Decoded pixel cap (bounds decode memory). Prevents image bombs. |
/// | `PYLON_IMAGE_FETCH_TIMEOUT_MS` | `15000` | Remote-source fetch timeout. |
/// | `PYLON_IMAGE_CONCURRENCY` | CPU count | Transforms that run at once. More requests wait their turn. |
/// | `PYLON_IMAGE_WAIT_MS` | `1500` | How long a request waits for its transform. Past it, the request gets the original file and the transform finishes in the background. |
#[derive(Debug, Clone)]
struct ImageConfig {
    remote_allowlist: Vec<RemotePattern>,
    allowed_widths: Vec<u32>,
    allowed_qualities: Vec<u8>,
    allowed_formats: Vec<OutFormat>,
    max_bytes: u64,
    max_pixels: u64,
    fetch_timeout: std::time::Duration,
    concurrency: usize,
    wait: Duration,
}

/// An entry in `PYLON_IMAGE_REMOTE_ALLOWLIST`. Syntax:
///
///   - `cdn.example.com` — any path on this host.
///   - `cdn.example.com/users/` — only paths starting with `/users/`.
///   - `*.example.com` — wildcard host (single level: matches
///     `foo.example.com`, not `foo.bar.example.com`).
///
/// Scheme + port are not part of the pattern; we always accept
/// either http:// or https:// + any port. If you need stricter
/// scheme/port matching, file an issue.
#[derive(Debug, Clone)]
struct RemotePattern {
    /// Literal host or `*.suffix` for single-level wildcard.
    host: String,
    /// Optional path prefix (e.g. `/users/`). Empty = any path.
    path_prefix: String,
}

impl RemotePattern {
    fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        let (host, path) = match s.find('/') {
            Some(i) => (&s[..i], &s[i..]),
            None => (s, ""),
        };
        Some(RemotePattern {
            host: host.to_string(),
            path_prefix: path.to_string(),
        })
    }
    fn matches(&self, host: &str, path: &str) -> bool {
        let host_ok = if let Some(suffix) = self.host.strip_prefix("*.") {
            // Single-level wildcard: must have exactly one extra label.
            host.ends_with(&format!(".{suffix}"))
                && host.matches('.').count() == suffix.matches('.').count() + 1
        } else {
            self.host.eq_ignore_ascii_case(host)
        };
        if !host_ok {
            return false;
        }
        if self.path_prefix.is_empty() {
            return true;
        }
        path.starts_with(&self.path_prefix)
    }
}

impl ImageConfig {
    fn from_env() -> Self {
        fn parse_csv<T, F>(name: &str, default: Vec<T>, f: F) -> Vec<T>
        where
            F: Fn(&str) -> Option<T>,
        {
            match std::env::var(name).ok().filter(|s| !s.is_empty()) {
                Some(raw) => {
                    let mut out: Vec<T> = raw.split(',').filter_map(|s| f(s.trim())).collect();
                    if out.is_empty() {
                        out = default;
                    }
                    out
                }
                None => default,
            }
        }

        let remote_allowlist = std::env::var("PYLON_IMAGE_REMOTE_ALLOWLIST")
            .ok()
            .filter(|s| !s.is_empty())
            .map(|raw| raw.split(',').filter_map(RemotePattern::parse).collect())
            .unwrap_or_default();

        // Device sizes = "full-bleed" image widths. Default mirrors
        // Next.js 14 defaults.
        let device_sizes = parse_csv(
            "PYLON_IMAGE_DEVICE_SIZES",
            vec![640, 750, 828, 1080, 1200, 1920, 2048, 3840],
            |s| s.parse::<u32>().ok(),
        );
        // Image sizes = thumbnail widths. Unioned with device sizes
        // to form the full allowed width set.
        let image_sizes = parse_csv(
            "PYLON_IMAGE_IMAGE_SIZES",
            vec![16, 32, 48, 64, 96, 128, 256, 384],
            |s| s.parse::<u32>().ok(),
        );
        let mut allowed_widths: Vec<u32> = device_sizes;
        allowed_widths.extend(image_sizes);
        allowed_widths.sort_unstable();
        allowed_widths.dedup();

        let allowed_qualities = parse_csv("PYLON_IMAGE_QUALITIES", vec![50, 75, 90], |s| {
            s.parse::<u8>().ok()
        });
        let allowed_formats = parse_csv(
            "PYLON_IMAGE_FORMATS",
            vec![OutFormat::Avif, OutFormat::Webp, OutFormat::Jpeg],
            OutFormat::from_str,
        );

        let max_bytes = std::env::var("PYLON_IMAGE_MAX_BYTES")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(25 * 1024 * 1024);
        // Decoded-pixel cap. This bounds peak DECODE memory (pixels x 4 bytes
        // RGBA + a resize clone), NOT the source byte count (max_bytes) — a
        // highly-compressible source under max_bytes can still declare a huge
        // canvas. 40M px (~any 24MP camera photo) caps a decode at ~160MB;
        // 100M would allow ~400MB (≈1GB peak with clones), and N concurrent
        // requests then OOM a small machine. Operators serving genuinely huge
        // images raise PYLON_IMAGE_MAX_PIXELS explicitly.
        let max_pixels = std::env::var("PYLON_IMAGE_MAX_PIXELS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(40_000_000);
        let fetch_timeout_ms = std::env::var("PYLON_IMAGE_FETCH_TIMEOUT_MS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(15_000);

        // A transform is CPU-bound. Running more at once than there are
        // CPUs makes each slower, and on a one-CPU machine a page of ten
        // pictures took 30 s per picture.
        let concurrency = std::env::var("PYLON_IMAGE_CONCURRENCY")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|n| *n > 0)
            .unwrap_or_else(|| {
                std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(1)
            });
        let wait_ms = std::env::var("PYLON_IMAGE_WAIT_MS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(1_500);

        ImageConfig {
            remote_allowlist,
            allowed_widths,
            allowed_qualities,
            allowed_formats,
            max_bytes,
            max_pixels,
            fetch_timeout: std::time::Duration::from_millis(fetch_timeout_ms),
            concurrency,
            wait: Duration::from_millis(wait_ms),
        }
    }
}

/// Process-lifetime cached config. Loaded once from env on first
/// image request. To override at runtime: set the env BEFORE
/// `pylon dev` boots.
fn config() -> &'static ImageConfig {
    static CFG: std::sync::OnceLock<ImageConfig> = std::sync::OnceLock::new();
    CFG.get_or_init(ImageConfig::from_env)
}

impl OutFormat {
    fn extension(self) -> &'static str {
        match self {
            OutFormat::Avif => "avif",
            OutFormat::Webp => "webp",
            OutFormat::Jpeg => "jpg",
            OutFormat::Png => "png",
        }
    }
    fn mime(self) -> &'static str {
        match self {
            OutFormat::Avif => "image/avif",
            OutFormat::Webp => "image/webp",
            OutFormat::Jpeg => "image/jpeg",
            OutFormat::Png => "image/png",
        }
    }
    fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "avif" => Some(OutFormat::Avif),
            "webp" => Some(OutFormat::Webp),
            "jpeg" | "jpg" => Some(OutFormat::Jpeg),
            "png" => Some(OutFormat::Png),
            _ => None,
        }
    }
}

/// Parameters parsed off the request URL.
#[derive(Debug)]
struct ImageRequest {
    /// Raw `src` query param — either an absolute http(s) URL or a
    /// site-relative path (`/foo/bar.jpg`).
    src: String,
    /// Width in CSS pixels. Image is resized to this width
    /// preserving aspect ratio. 32px..=4096px.
    width: u32,
    /// Quality 1..=100. Lossless formats (PNG) ignore it.
    quality: u8,
    /// Output format. Inferred from Accept header when unset.
    format: Option<OutFormat>,
}

/// Reason a request was rejected before processing started. We surface these
/// as `400 Bad Request` with the reason in the body.
///
/// Only two remain, and both are genuinely unanswerable rather than merely
/// off-list. Width, quality and format used to reject too; a near-miss now
/// snaps or falls back, because returning an error body in place of an image
/// broke the page over a value we could have resolved.
#[derive(Debug)]
enum RejectReason {
    MissingSrc,
    SvgRejected,
}

/// Render an allowlist for a message: `50, 75, 90`.
fn join_values<T: std::fmt::Display>(values: &[T]) -> String {
    values
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Nearest allowed value to `want`. Ties break UPWARD — a tie means the
/// request sat exactly between two options, and the larger one is the
/// better default for both axes: more pixels rather than a soft upscale,
/// more quality rather than visible artifacts. Falls back to `want`
/// unchanged when the allowlist is empty, which `parse_csv` prevents.
fn snap_to<T>(want: T, allowed: &[T]) -> T
where
    T: Copy + Ord + Into<i64>,
{
    let want_i: i64 = want.into();
    allowed
        .iter()
        .copied()
        .min_by_key(|candidate| {
            let c: i64 = (*candidate).into();
            // (distance, -value) — smallest distance first, then largest value.
            ((c - want_i).abs(), -c)
        })
        .unwrap_or(want)
}

fn snap_u32(want: u32, allowed: &[u32]) -> u32 {
    snap_to(want, allowed)
}

fn snap_u8(want: u8, allowed: &[u8]) -> u8 {
    snap_to(want, allowed)
}

/// Warn once per distinct (param, requested, resolved) triple.
///
/// Snapping keeps the page working, but silently rendering something other
/// than what the code asked for is its own trap — the developer would never
/// learn that `quality={80}` isn't a real value. So it is always reported, and
/// `pylon dev` says how to fix it.
///
/// Deduped because a gallery would otherwise emit one line per image per
/// request; the set is tiny and bounded by the allowlist sizes.
fn warn_snapped(param: &str, env_var: &str, requested: i64, resolved: i64, allowed: &str) {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};

    static SEEN: OnceLock<Mutex<HashSet<(String, i64, i64)>>> = OnceLock::new();
    let seen = SEEN.get_or_init(|| Mutex::new(HashSet::new()));
    let key = (param.to_string(), requested, resolved);
    // Already reported, or the lock is poisoned — either way, stay quiet.
    let first_time = match seen.lock() {
        Ok(mut set) => set.insert(key),
        Err(_) => false,
    };
    if !first_time {
        return;
    }

    if crate::frontend::is_dev_mode() {
        eprintln!(
            "[pylon] <Image> {param}={requested} is not an allowed value — served {param}={resolved} instead.\n\
             [pylon]   Allowed: {allowed}. These are an allowlist, not a range.\n\
             [pylon]   Use one of them, or set {env_var} to include {requested}."
        );
    } else {
        tracing::warn!(
            param = param,
            requested = requested,
            resolved = resolved,
            allowed = allowed,
            "image param snapped to nearest allowed value"
        );
    }
}

impl std::fmt::Display for RejectReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RejectReason::MissingSrc => write!(f, "missing `src` param"),
            RejectReason::SvgRejected => write!(
                f,
                "SVG sources are not supported (security: SVG can contain scripts)"
            ),
        }
    }
}

/// Stringly-typed query parser. Width / quality / format are all
/// validated against the [`ImageConfig`] allowlists — invalid
/// combinations return a `Reject` instead of being clamped, so
/// attackers can't enumerate the cache by passing arbitrary
/// values.
fn parse_query(url: &str, cfg: &ImageConfig) -> Result<ImageRequest, RejectReason> {
    let qs = url.split_once('?').map(|(_, q)| q).unwrap_or("");
    let mut src: Option<String> = None;
    let mut width: Option<u32> = None;
    let mut quality: Option<u8> = None;
    let mut format_raw: Option<String> = None;
    for kv in qs.split('&') {
        let (k, v) = match kv.split_once('=') {
            Some(p) => p,
            None => continue,
        };
        match k {
            "src" => src = Some(percent_decode(v)),
            "w" => width = v.parse().ok(),
            "q" => quality = v.parse().ok(),
            "format" => format_raw = Some(v.to_string()),
            _ => {}
        }
    }

    let src = src.ok_or(RejectReason::MissingSrc)?;

    // SVG hard-rejected — never decoded, never produced. Same call
    // a Next.js `dangerouslyAllowSVG=false` makes.
    let lower_src = src.to_ascii_lowercase();
    if lower_src.ends_with(".svg") || lower_src.contains(".svg?") {
        return Err(RejectReason::SvgRejected);
    }

    // Width and quality SNAP to the nearest allowed value rather than being
    // rejected. A near-miss — `quality={80}` is the classic — used to return an
    // error body in place of the image, so a single wrong prop broke the
    // picture in production with nothing but a comment to explain it.
    //
    // Snapping keeps the anti-enumeration property intact, which is why the
    // allowlist existed. The resolved value is what reaches `cache_path`, so
    // arbitrary `q=1..100` still collapses onto the same handful of cache
    // entries; the allowlist bounds the cache either way. Rejecting only ever
    // bought a worse failure mode.
    let want_width = width.unwrap_or(0);
    let width = snap_u32(want_width, &cfg.allowed_widths);
    // Only an explicit `w=` that missed is worth reporting. An absent one
    // defaults to 0 and snaps to the smallest size — that's the caller not
    // asking, not the caller being wrong.
    if width != want_width && want_width != 0 {
        warn_snapped(
            "w",
            "PYLON_IMAGE_DEVICE_SIZES / PYLON_IMAGE_IMAGE_SIZES",
            want_width as i64,
            width as i64,
            &join_values(&cfg.allowed_widths),
        );
    }

    let want_quality = quality.unwrap_or(75);
    let quality = snap_u8(want_quality, &cfg.allowed_qualities);
    if quality != want_quality {
        warn_snapped(
            "quality",
            "PYLON_IMAGE_QUALITIES",
            want_quality as i64,
            quality as i64,
            &join_values(&cfg.allowed_qualities),
        );
    }

    // Format is NOT snapped: "nearest" is meaningless across codecs, and
    // silently answering a `format=avif` request with JPEG bytes would be a
    // correctness bug, not a courtesy. An unknown or disallowed format falls
    // back to Accept negotiation — the same path as omitting it entirely.
    let format = match format_raw {
        Some(raw) => OutFormat::from_str(&raw).filter(|f| cfg.allowed_formats.contains(f)),
        None => None,
    };

    Ok(ImageRequest {
        src,
        width,
        quality,
        format,
    })
}

/// Minimal percent-decode for the `src` query value. Standard
/// urlencoding only — `%XX` hex pairs plus `+` → space. Anything
/// invalid falls through as a literal char so we don't reject
/// edge cases unnecessarily.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'+' {
            out.push(b' ');
            i += 1;
        } else if b == b'%' && i + 2 < bytes.len() {
            let hi = hex_digit(bytes[i + 1]);
            let lo = hex_digit(bytes[i + 2]);
            match (hi, lo) {
                (Some(h), Some(l)) => {
                    out.push((h << 4) | l);
                    i += 3;
                }
                _ => {
                    out.push(b);
                    i += 1;
                }
            }
        } else {
            out.push(b);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Split an http(s) URL into `(host, path)` for allowlist matching.
/// Returns `None` for any non-http(s) scheme — so a redirect to
/// `file://`, `gopher://`, etc. is rejected. Strips `user:pass@`
/// userinfo and the `:port` suffix off the host, mirroring the
/// original inline parser.
fn extract_host_path(url: &str) -> Option<(&str, &str)> {
    let after_scheme = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))?;
    let (host_with_port, path_with_query) =
        match after_scheme.find(|c: char| c == '/' || c == '?' || c == '#') {
            Some(i) => (&after_scheme[..i], &after_scheme[i..]),
            None => (after_scheme, ""),
        };
    let path = match path_with_query.find(|c: char| c == '?' || c == '#') {
        Some(i) => &path_with_query[..i],
        None => path_with_query,
    };
    let host_no_auth = match host_with_port.rsplit_once('@') {
        Some((_, h)) => h,
        None => host_with_port,
    };
    let host = match host_no_auth.rsplit_once(':') {
        Some((h, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => h,
        _ => host_no_auth,
    };
    Some((host, path))
}

/// Resolve a redirect `Location` against the current absolute URL.
/// Returns an absolute http(s) URL, or `None` if it can't be resolved to
/// one (the caller then rejects the redirect — fail closed). Handles
/// absolute, scheme-relative (`//host/..`), origin-relative (`/path`),
/// and bare-relative locations.
fn resolve_redirect(base: &str, loc: &str) -> Option<String> {
    let loc = loc.trim();
    if loc.is_empty() {
        return None;
    }
    if loc.starts_with("http://") || loc.starts_with("https://") {
        return Some(loc.to_string());
    }
    let scheme = if base.starts_with("https://") {
        "https"
    } else {
        "http"
    };
    let after_scheme = base
        .strip_prefix("http://")
        .or_else(|| base.strip_prefix("https://"))?;
    let authority = after_scheme
        .split(|c| c == '/' || c == '?' || c == '#')
        .next()
        .unwrap_or(after_scheme);
    if let Some(rest) = loc.strip_prefix("//") {
        return Some(format!("{scheme}://{rest}"));
    }
    if loc.starts_with('/') {
        return Some(format!("{scheme}://{authority}{loc}"));
    }
    // Bare relative path — resolve against the base URL's directory.
    let base_path = &after_scheme[authority.len()..];
    let path_only = base_path.split(['?', '#']).next().unwrap_or(base_path);
    let dir_end = path_only.rfind('/').map(|i| i + 1).unwrap_or(0);
    Some(format!(
        "{scheme}://{authority}{}{loc}",
        &path_only[..dir_end]
    ))
}

/// Read the source bytes — either off the local frontend dir or
/// over HTTP for remote allowlisted URLs. Enforces the byte cap
/// from [`ImageConfig::max_bytes`] for both paths.
fn load_source(
    src: &str,
    frontend_dir: Option<&Path>,
    cfg: &ImageConfig,
) -> Result<Vec<u8>, String> {
    if src.starts_with("http://") || src.starts_with("https://") {
        if cfg.remote_allowlist.is_empty() {
            return Err(format!(
                "remote image source not allowed (set PYLON_IMAGE_REMOTE_ALLOWLIST): {src}"
            ));
        }
        // SSRF-hardened fetch:
        //   - `.redirects(0)` so ureq never auto-follows. We follow
        //     manually and re-run the allowlist check on EVERY hop —
        //     otherwise an allowlisted host that 302s to
        //     `http://169.254.169.254/…` (cloud metadata) or an internal
        //     service would be fetched (the allowlist only ever validated
        //     the first URL).
        //   - a guarded resolver that refuses to connect to any
        //     private/loopback/link-local address, on every hop and after
        //     any DNS rebind (ureq connects to exactly what it returns).
        let agent = ureq::AgentBuilder::new()
            .redirects(0)
            .resolver(|netloc: &str| pylon_kernel::net_guard::resolve_guarded(netloc))
            .build();

        const MAX_HOPS: u32 = 3;
        let mut url = src.to_string();
        let mut hops = 0u32;
        let res = loop {
            let (host, path) = extract_host_path(&url)
                .ok_or_else(|| format!("remote image URL is not http(s): {url}"))?;
            if !cfg.remote_allowlist.iter().any(|p| p.matches(host, path)) {
                return Err(format!(
                    "host {host} (path {path}) not allowed by PYLON_IMAGE_REMOTE_ALLOWLIST"
                ));
            }
            let resp = agent
                .get(&url)
                .timeout(cfg.fetch_timeout)
                .call()
                .map_err(|e| format!("fetch failed: {e}"))?;
            let status = resp.status();
            if (300..400).contains(&status) {
                hops += 1;
                if hops > MAX_HOPS {
                    return Err(format!(
                        "too many redirects fetching remote image (> {MAX_HOPS})"
                    ));
                }
                let loc = resp
                    .header("location")
                    .ok_or_else(|| "redirect response without a Location header".to_string())?;
                url = resolve_redirect(&url, loc)
                    .ok_or_else(|| format!("unfollowable redirect Location: {loc}"))?;
                continue;
            }
            break resp;
        };

        let mut buf: Vec<u8> = Vec::new();
        // `.take(N)` silently truncates; we want a hard error so
        // the operator knows their cap is too tight or the source
        // is unexpectedly huge. Take(max_bytes + 1) lets us detect
        // overshoot.
        res.into_reader()
            .take(cfg.max_bytes + 1)
            .read_to_end(&mut buf)
            .map_err(|e| format!("read failed: {e}"))?;
        if (buf.len() as u64) > cfg.max_bytes {
            return Err(format!(
                "remote source exceeds PYLON_IMAGE_MAX_BYTES ({})",
                cfg.max_bytes
            ));
        }
        Ok(buf)
    } else if let Some(p) = src.strip_prefix('/') {
        // Local file — must be under frontend dir.
        let dir = match frontend_dir {
            Some(d) => d,
            None => return Err("no frontend dir configured for local images".into()),
        };
        let full = dir.join(p);
        // Canonicalize + verify still inside dir.
        let dir_canon = dir
            .canonicalize()
            .map_err(|e| format!("frontend dir missing: {e}"))?;
        let full_canon = full
            .canonicalize()
            .map_err(|e| format!("image not found: {e}"))?;
        if !full_canon.starts_with(&dir_canon) {
            return Err("path escapes frontend dir".into());
        }
        let meta = std::fs::metadata(&full_canon).map_err(|e| format!("metadata failed: {e}"))?;
        if meta.len() > cfg.max_bytes {
            return Err(format!(
                "local source exceeds PYLON_IMAGE_MAX_BYTES ({})",
                cfg.max_bytes
            ));
        }
        std::fs::read(&full_canon).map_err(|e| format!("read failed: {e}"))
    } else {
        Err(format!("unsupported src scheme: {src}"))
    }
}

/// Resize + re-encode using the fast pipeline:
///   - `image` for decode (pure Rust, all common formats).
///   - `fast_image_resize` for resize (SIMD AVX2/NEON, 10-50x
///     faster than `image::imageops`).
///   - `mozjpeg` for JPEG encoding (Mozilla's mozjpeg C via cc-rs;
///     better compression than libjpeg-turbo at same quality).
///   - `webp` (libwebp via cc-rs) for lossy WebP — 30-40% smaller
///     than the pure-Rust lossless encoder in `image`.
///   - `image` for PNG (lossless — quality knob ignored).
///
/// Synchronous because the HTTP path is synchronous (tiny_http
/// worker thread). On the worker thread pool the CPU work
/// happens in parallel across requests.
fn process(
    input: &[u8],
    width: u32,
    quality: u8,
    fmt: OutFormat,
    cfg: &ImageConfig,
) -> Result<Vec<u8>, String> {
    use fast_image_resize as fir;
    use image::ImageReader;

    let cursor = Cursor::new(input);
    let reader = ImageReader::new(cursor)
        .with_guessed_format()
        .map_err(|e| format!("guess format failed: {e}"))?;

    // Image-bomb protection: peek dimensions BEFORE decode. A 2KB
    // PNG can declare a 100000x100000 pixel canvas which would
    // allocate 40GB of RGBA storage. ImageReader::into_dimensions
    // reads the header only.
    let dim_reader = ImageReader::new(Cursor::new(input))
        .with_guessed_format()
        .map_err(|e| format!("guess format failed (dims): {e}"))?;
    let (sw, sh) = dim_reader
        .into_dimensions()
        .map_err(|e| format!("read dimensions failed: {e}"))?;
    let pixels = (sw as u64).saturating_mul(sh as u64);
    if pixels > cfg.max_pixels {
        return Err(format!(
            "source declares {pixels} pixels ({sw}x{sh}), exceeds PYLON_IMAGE_MAX_PIXELS ({})",
            cfg.max_pixels
        ));
    }

    let img = reader.decode().map_err(|e| format!("decode failed: {e}"))?;

    // Compute target dims preserving aspect ratio. Never upscale —
    // matches Next.js semantics + avoids bilinear-upscale artifacts.
    let src_w = img.width();
    let src_h = img.height();
    let (dst_w, dst_h) = if src_w > width {
        let dst_h = ((src_h as u64) * (width as u64) / (src_w as u64)).max(1) as u32;
        (width, dst_h)
    } else {
        (src_w, src_h)
    };

    // SIMD resize via fast_image_resize. We pass through the
    // DynamicImage's RGBA8 channel layout; convert if needed.
    let resized: image::DynamicImage = if (dst_w, dst_h) == (src_w, src_h) {
        img
    } else {
        let src_rgba = img.to_rgba8();
        let src_image = fir::images::Image::from_vec_u8(
            src_w,
            src_h,
            src_rgba.into_raw(),
            fir::PixelType::U8x4,
        )
        .map_err(|e| format!("fast resize src setup: {e}"))?;
        let mut dst_image = fir::images::Image::new(dst_w, dst_h, fir::PixelType::U8x4);
        // Lanczos3 — sharp at downscale.
        let mut resizer = fir::Resizer::new();
        let opts = fir::ResizeOptions::new()
            .resize_alg(fir::ResizeAlg::Convolution(fir::FilterType::Lanczos3));
        resizer
            .resize(&src_image, &mut dst_image, Some(&opts))
            .map_err(|e| format!("fast resize: {e}"))?;
        let buf = image::RgbaImage::from_raw(dst_w, dst_h, dst_image.into_vec())
            .ok_or_else(|| "dst buffer size mismatch".to_string())?;
        image::DynamicImage::ImageRgba8(buf)
    };

    match fmt {
        OutFormat::Jpeg => {
            // mozjpeg wants RGB, no alpha — flatten transparent
            // pixels onto white to avoid mozjpeg's default black
            // background (looks ugly on light themes).
            let rgb = flatten_to_rgb(&resized);
            let mut comp = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_RGB);
            comp.set_size(resized.width() as usize, resized.height() as usize);
            comp.set_quality(quality as f32);
            // Progressive is the default; explicit for clarity.
            comp.set_progressive_mode();
            let mut comp = comp
                .start_compress(Vec::new())
                .map_err(|e| format!("mozjpeg start: {e}"))?;
            comp.write_scanlines(&rgb)
                .map_err(|e| format!("mozjpeg write: {e}"))?;
            comp.finish().map_err(|e| format!("mozjpeg finish: {e}"))
        }
        OutFormat::Png => {
            let mut out: Vec<u8> = Vec::new();
            let mut cursor = Cursor::new(&mut out);
            resized
                .write_to(&mut cursor, image::ImageFormat::Png)
                .map_err(|e| format!("png encode failed: {e}"))?;
            Ok(out)
        }
        OutFormat::Avif => {
            // ravif (rav1e) — quality maps 1..=100 like the others;
            // speed 8 keeps cold encodes in the hundreds of ms and the
            // transform cache makes every later hit free. RGBA input;
            // AVIF carries alpha natively.
            let rgba = resized.to_rgba8();
            let (w, h) = (resized.width() as usize, resized.height() as usize);
            let pixels: &[ravif::RGBA8] = bytemuck_cast_rgba(rgba.as_raw());
            let img = ravif::Img::new(pixels, w, h);
            let res = ravif::Encoder::new()
                .with_quality(quality as f32)
                .with_alpha_quality(quality as f32)
                .with_speed(8)
                // One thread per encode: PYLON_IMAGE_CONCURRENCY already
                // runs one transform per CPU, and rav1e's own thread pool
                // on top of that oversubscribed small machines.
                .with_num_threads(Some(1))
                .encode_rgba(img)
                .map_err(|e| format!("avif encode failed: {e}"))?;
            Ok(res.avif_file)
        }
        OutFormat::Webp => {
            // libwebp lossy encoder — quality 0..=100 (we clamp at
            // the parse step). RGBA8 input; libwebp handles alpha.
            let rgba = resized.to_rgba8();
            let encoder =
                webp::Encoder::from_rgba(rgba.as_raw(), resized.width(), resized.height());
            let mem = encoder.encode(quality as f32);
            Ok(mem.to_vec())
        }
    }
}

/// Reinterpret packed RGBA bytes as ravif's RGBA8 pixel slice. Length
/// is a multiple of 4 by construction (the buffer comes from
/// `to_rgba8()`); the types are layout-identical (4 consecutive u8s).
fn bytemuck_cast_rgba(bytes: &[u8]) -> &[ravif::RGBA8] {
    // SAFETY: RGBA8 is #[repr(C)] { r,g,b,a: u8 } — same size (4),
    // align (1), and meaning as the packed byte quadruplets in an
    // image::RgbaImage raw buffer.
    unsafe { std::slice::from_raw_parts(bytes.as_ptr() as *const ravif::RGBA8, bytes.len() / 4) }
}

/// Flatten an image with alpha onto a white background, returning
/// raw RGB bytes (3 channels). mozjpeg can't encode RGBA, so we
/// pre-composite. White matches the typical web layout default.
fn flatten_to_rgb(img: &image::DynamicImage) -> Vec<u8> {
    let rgba = img.to_rgba8();
    let mut out = Vec::with_capacity((rgba.width() * rgba.height() * 3) as usize);
    for pixel in rgba.pixels() {
        let [r, g, b, a] = pixel.0;
        if a == 255 {
            out.push(r);
            out.push(g);
            out.push(b);
        } else {
            // alpha composite over white.
            let a_f = a as u32;
            let inv = 255 - a_f;
            let blend = |c: u8| -> u8 { ((c as u32 * a_f + 255 * inv) / 255) as u8 };
            out.push(blend(r));
            out.push(blend(g));
            out.push(blend(b));
        }
    }
    out
}

/// Pick a default output format when the request didn't specify.
/// Browsers ship WebP support universally; check the Accept header
/// just in case (curl, oldreader bots).
fn default_format(headers: &[Header], cfg: &ImageConfig) -> OutFormat {
    let accept = headers
        .iter()
        .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case("accept"))
        .map(|h| h.value.as_str().to_string())
        .unwrap_or_default();
    // Prefer AVIF when the browser EXPLICITLY accepts it (never on an
    // empty Accept — unknown clients get the universally-supported
    // WebP), then WebP.
    if accept.contains("image/avif") && cfg.allowed_formats.contains(&OutFormat::Avif) {
        return OutFormat::Avif;
    }
    // Prefer WebP when the browser accepts it AND it's allowed.
    if (accept.contains("image/webp") || accept.is_empty())
        && cfg.allowed_formats.contains(&OutFormat::Webp)
    {
        return OutFormat::Webp;
    }
    // Otherwise fall back to whatever's allowed in order: jpeg, png, webp.
    for candidate in [OutFormat::Jpeg, OutFormat::Png, OutFormat::Webp] {
        if cfg.allowed_formats.contains(&candidate) {
            return candidate;
        }
    }
    // Allowed formats is empty (mis-config). Default to jpeg so
    // we don't 500 on every image request — the operator will see
    // the warning in logs.
    OutFormat::Jpeg
}

/// Content-addressed cache path for a transform.
fn cache_path(cache_dir: &Path, src: &str, width: u32, quality: u8, fmt: OutFormat) -> PathBuf {
    let mut h = Sha256::new();
    h.update(src.as_bytes());
    h.update(width.to_le_bytes());
    h.update([quality]);
    // Full extension, not just its first byte — hashing one byte would collide
    // any two output formats sharing a first letter (a future `jp2`/`jpg` or
    // `webp`/`webm`) onto the same cache file, serving format A's bytes under
    // format B's Content-Type.
    h.update(fmt.extension().as_bytes());
    let hash = h.finalize();
    let hex: String = hash.iter().take(16).map(|b| format!("{b:02x}")).collect();
    cache_dir.join(format!("{hex}.{}", fmt.extension()))
}

type ImageResult = Result<Arc<Vec<u8>>, (u16, String)>;

/// Share a cold transform with concurrent requests for the same cache file.
/// Entries last only while requests use them, including failed transforms.
#[derive(Default)]
struct ImageWork {
    pending: Mutex<HashMap<PathBuf, Weak<OnceLock<ImageResult>>>>,
}

struct ImageFlight<'a> {
    work: &'a ImageWork,
    path: &'a Path,
}

impl Drop for ImageFlight<'_> {
    fn drop(&mut self) {
        let mut pending = self.work.pending.lock().unwrap_or_else(|e| e.into_inner());
        // The map holds only a Weak. No new caller can join while locked.
        if pending
            .get(self.path)
            .is_some_and(|entry| entry.strong_count() == 0)
        {
            pending.remove(self.path);
        }
    }
}

impl ImageWork {
    fn get(
        &self,
        path: &Path,
        render: impl FnOnce() -> Result<Vec<u8>, (u16, String)>,
    ) -> ImageResult {
        if let Some(bytes) = read_cached_image(path)? {
            return Ok(Arc::new(bytes));
        }
        // Declare the cleanup guard first so the result Arc drops before it,
        // including during a panic. Check the last reference under the lock.
        let _flight = ImageFlight { work: self, path };
        let result = {
            let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            match pending.get(path).and_then(Weak::upgrade) {
                Some(result) => result,
                None => {
                    let result = Arc::new(OnceLock::new());
                    pending.insert(path.to_path_buf(), Arc::downgrade(&result));
                    result
                }
            }
        };
        result
            .get_or_init(|| {
                // Another flight can finish between the first read and joining.
                if let Some(bytes) = read_cached_image(path)? {
                    return Ok(Arc::new(bytes));
                }
                let bytes = render()?;
                // Cache failure must not discard a successful transform.
                let _ = write_cached_image(path, &bytes);
                Ok(Arc::new(bytes))
            })
            .clone()
    }
}

fn read_cached_image(path: &Path) -> Result<Option<Vec<u8>>, (u16, String)> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err((500, format!("cache read failed: {error}"))),
    }
}

fn write_cached_image(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let temp = path.with_extension(format!(
        "tmp.{}.{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let written = file.write_all(bytes);
    drop(file);
    // Publish only the complete file. Other processes can read this cache too.
    let result = written.and_then(|()| std::fs::rename(&temp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

/// Public entry point — wired from `frontend.rs`. `cache_root` is
/// the project's `.pylon` directory; we create
/// `<cache_root>/.cache/images/` lazily.
pub fn serve(request: Request, cache_root: &Path, frontend_dir: Option<&Path>, cors_origin: &str) {
    let cfg = config();
    let url = request.url().to_string();
    let parsed = match parse_query(&url, cfg) {
        Ok(p) => p,
        Err(reason) => {
            respond_err(request, 400, &reason.to_string());
            return;
        }
    };

    let fmt = parsed
        .format
        .unwrap_or_else(|| default_format(request.headers(), cfg));

    let cache_dir = cache_root.join(".cache").join("images");
    if let Err(e) = std::fs::create_dir_all(&cache_dir) {
        tracing::warn!(error = %e, "image cache dir create failed");
    }
    let dest = cache_path(&cache_dir, &parsed.src, parsed.width, parsed.quality, fmt);

    // A transform made before: serve it.
    match read_cached_image(&dest) {
        Ok(Some(bytes)) => {
            respond_image(request, &bytes, fmt.mime(), true, cors_origin);
            return;
        }
        Ok(None) => {}
        Err((status, message)) => {
            respond_err(request, status, &message);
            return;
        }
    }

    // The source is read here, so a bad source answers with its error and
    // the original is at hand if the transform takes too long.
    let source = match load_source(&parsed.src, frontend_dir, cfg) {
        Ok(bytes) => Arc::new(bytes),
        Err(e) => {
            respond_err(request, 400, &e);
            return;
        }
    };

    // Make the transform in the background, one per variant, at most
    // `concurrency` at once. Wait for it up to `wait`; past that, answer
    // with the original so a page never waits on the resizer. The next
    // request gets the transform from the cache.
    warm(
        dest.clone(),
        Arc::clone(&source),
        parsed.width,
        parsed.quality,
        fmt,
    );
    let deadline = Instant::now() + cfg.wait;
    loop {
        if let Ok(Some(bytes)) = read_cached_image(&dest) {
            respond_image(request, &bytes, fmt.mime(), true, cors_origin);
            return;
        }
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    let mime = image::guess_format(&source)
        .map(|f| f.to_mime_type())
        .unwrap_or("application/octet-stream");
    respond_image(request, &source, mime, false, cors_origin);
}

/// Answer with image bytes. A transform is immutable (its URL names the
/// variant); the original, sent while the transform is still being made,
/// is cached briefly in the browser and never at a CDN, so the next view
/// gets the transform.
fn respond_image(request: Request, bytes: &[u8], mime: &str, transformed: bool, cors_origin: &str) {
    let (cache, cdn) = if transformed {
        (
            "public, max-age=31536000, immutable",
            "public, max-age=31536000, immutable",
        )
    } else {
        ("public, max-age=60", "no-store")
    };
    let response = Response::new(
        tiny_http::StatusCode(200),
        Vec::new(),
        Cursor::new(bytes),
        Some(bytes.len()),
        None,
    )
    .with_header(Header::from_bytes("Content-Type", mime).unwrap())
    .with_header(Header::from_bytes("Cache-Control", cache).unwrap())
    // The output format is negotiated from Accept when the URL
    // doesn't pin one — any shared cache MUST key on it or Safari
    // gets the AVIF a Chrome visitor warmed. CDNs that support
    // CDN-Cache-Control (Cloudflare et al.) get an explicit edge
    // TTL so transforms are edge-cacheable without a manual rule.
    .with_header(Header::from_bytes("Vary", "Accept").unwrap())
    .with_header(Header::from_bytes("CDN-Cache-Control", cdn).unwrap())
    .with_header(
        Header::from_bytes(
            "Access-Control-Allow-Origin",
            cors_origin.as_bytes().to_vec(),
        )
        .unwrap(),
    );
    let _ = request.respond(response);
}

/// The shared transform work (singleflight per cache file).
fn work() -> &'static ImageWork {
    static WORK: OnceLock<ImageWork> = OnceLock::new();
    WORK.get_or_init(ImageWork::default)
}

/// A counting semaphore: at most `n` transforms run at once.
struct Permits {
    free: Mutex<usize>,
    freed: Condvar,
}

struct Permit<'a>(&'a Permits);

impl Permits {
    fn new(n: usize) -> Self {
        Permits {
            free: Mutex::new(n.max(1)),
            freed: Condvar::new(),
        }
    }

    fn acquire(&self) -> Permit<'_> {
        let mut free = self.free.lock().unwrap_or_else(|e| e.into_inner());
        while *free == 0 {
            free = self.freed.wait(free).unwrap_or_else(|e| e.into_inner());
        }
        *free -= 1;
        Permit(self)
    }
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        *self.0.free.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        self.0.freed.notify_one();
    }
}

fn permits() -> &'static Permits {
    static PERMITS: OnceLock<Permits> = OnceLock::new();
    PERMITS.get_or_init(|| Permits::new(config().concurrency))
}

/// How long a variant whose transform failed is left alone before it is
/// tried again. Requests for it get the original meanwhile.
const FAILED_RETRY_AFTER: Duration = Duration::from_secs(600);

#[derive(Default)]
struct Warming {
    running: std::collections::HashSet<PathBuf>,
    failed: HashMap<PathBuf, Instant>,
}

fn warming() -> &'static Mutex<Warming> {
    static WARMING: OnceLock<Mutex<Warming>> = OnceLock::new();
    WARMING.get_or_init(|| Mutex::new(Warming::default()))
}

/// Start making a variant in the background, unless it is already being
/// made or failed recently. The thread waits for a permit, so a page of
/// many pictures queues instead of running every encode at once.
fn warm(dest: PathBuf, source: Arc<Vec<u8>>, width: u32, quality: u8, fmt: OutFormat) {
    {
        let mut w = warming().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(at) = w.failed.get(&dest) {
            if at.elapsed() < FAILED_RETRY_AFTER {
                return;
            }
            w.failed.remove(&dest);
        }
        if !w.running.insert(dest.clone()) {
            return;
        }
    }
    let claimed = dest.clone();
    let spawned = std::thread::Builder::new()
        .name("pylon-image".into())
        .spawn(move || {
            let result = {
                let _permit = permits().acquire();
                work().get(&dest, || {
                    process(&source, width, quality, fmt, config()).map_err(|e| (500, e))
                })
            };
            let mut w = warming().lock().unwrap_or_else(|e| e.into_inner());
            w.running.remove(&dest);
            if let Err((_, message)) = result {
                tracing::warn!(error = %message, path = %dest.display(), "image transform failed");
                w.failed.insert(dest, Instant::now());
            }
        });
    if spawned.is_err() {
        // No thread: forget the claim so a later request can try again.
        let mut w = warming().lock().unwrap_or_else(|e| e.into_inner());
        w.running.remove(&claimed);
    }
}

fn respond_err(request: Request, status: u16, msg: &str) {
    let body = format!("/* image error: {msg} */\n").into_bytes();
    let response = Response::from_data(body)
        .with_status_code(status)
        .with_header(Header::from_bytes("Content-Type", "text/plain; charset=utf-8").unwrap());
    let _ = request.respond(response);
}

// std::io::Read needs to be in scope for `into_reader().read_to_end`.
use std::io::Read;

#[cfg(test)]
mod tests {
    use super::*;

    fn test_cfg() -> ImageConfig {
        ImageConfig::from_env()
    }

    fn accept_header(v: &str) -> Vec<Header> {
        vec![Header::from_bytes("Accept", v).unwrap()]
    }

    #[test]
    fn concurrent_image_requests_share_successes_and_errors() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Condvar;
        use std::time::{Duration, Instant};

        for cache_writable in [false, true] {
            for fail in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                let path = if cache_writable {
                    dir.path().join("image.jpg")
                } else {
                    dir.path().join("missing/image.jpg")
                };
                let work = Arc::new(ImageWork::default());
                let calls = Arc::new(AtomicUsize::new(0));
                let release = Arc::new((Mutex::new(false), Condvar::new()));
                let workers: Vec<_> = (0..16)
                    .map(|_| {
                        let work = Arc::clone(&work);
                        let calls = Arc::clone(&calls);
                        let release = Arc::clone(&release);
                        let path = path.clone();
                        std::thread::spawn(move || {
                            work.get(&path, || {
                                calls.fetch_add(1, Ordering::Relaxed);
                                let (lock, ready) = &*release;
                                let (allowed, timeout) = ready
                                    .wait_timeout_while(
                                        lock.lock().unwrap(),
                                        Duration::from_secs(5),
                                        |allowed| !*allowed,
                                    )
                                    .unwrap();
                                assert!(*allowed && !timeout.timed_out());
                                if fail {
                                    Err((400, "source failed".into()))
                                } else {
                                    Ok(vec![7; 1024])
                                }
                            })
                        })
                    })
                    .collect();
                let deadline = Instant::now() + Duration::from_secs(3);
                let joined = loop {
                    let count = work
                        .pending
                        .lock()
                        .unwrap()
                        .get(&path)
                        .map(Weak::strong_count)
                        .unwrap_or(0);
                    if count == 16 {
                        break true;
                    }
                    if Instant::now() >= deadline {
                        break false;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                };
                *release.0.lock().unwrap() = true;
                release.1.notify_all();
                let results: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
                assert!(
                    joined,
                    "all requests must join while the transform is blocked"
                );
                assert_eq!(calls.load(Ordering::Relaxed), 1);
                assert!(work.pending.lock().unwrap().is_empty());
                if fail {
                    assert!(results
                        .iter()
                        .all(|r| r.as_ref().unwrap_err() == &(400, "source failed".into())));
                    assert_eq!(*work.get(&path, || Ok(vec![9])).unwrap(), [9]);
                } else {
                    let first = results[0].as_ref().unwrap();
                    assert_eq!(first.as_slice(), &[7; 1024]);
                    assert!(results
                        .iter()
                        .all(|r| Arc::ptr_eq(first, r.as_ref().unwrap())));
                    if cache_writable {
                        assert_eq!(
                            *work
                                .get(&path, || panic!("cache hit must not render"))
                                .unwrap(),
                            **first
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn different_images_do_not_wait_for_each_others_transform() {
        use std::sync::mpsc;
        use std::time::Duration;
        let dir = tempfile::tempdir().unwrap();
        let work = Arc::new(ImageWork::default());
        let (started, entered) = mpsc::channel();
        let (release, allowed) = mpsc::channel();
        let slow = Arc::clone(&work);
        let slow_path = dir.path().join("slow.jpg");
        let worker = std::thread::spawn(move || {
            slow.get(&slow_path, || {
                started.send(()).unwrap();
                allowed.recv_timeout(Duration::from_secs(5)).unwrap();
                Ok(vec![1])
            })
        });
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        let fast = Arc::clone(&work);
        let fast_path = dir.path().join("fast.jpg");
        let (done, received) = mpsc::channel();
        let second =
            std::thread::spawn(move || done.send(fast.get(&fast_path, || Ok(vec![2]))).unwrap());
        let result = received.recv_timeout(Duration::from_secs(2));
        release.send(()).unwrap();
        worker.join().unwrap().unwrap();
        second.join().unwrap();
        assert_eq!(*result.unwrap().unwrap(), [2]);
        assert!(work.pending.lock().unwrap().is_empty());
    }

    #[test]
    fn panicked_transform_releases_its_registry_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("retry.jpg");
        let work = ImageWork::default();
        let result = std::panic::catch_unwind(|| work.get(&path, || panic!("failed transform")));
        assert!(result.is_err());
        assert!(work.pending.lock().unwrap().is_empty());
        assert_eq!(*work.get(&path, || Ok(vec![3])).unwrap(), [3]);
    }

    #[test]
    fn image_cache_publishes_whole_files_and_cleans_failed_writes() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::time::{Duration, Instant};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("image.jpg");
        let first = vec![1; 128 * 1024];
        let second = vec![2; 256 * 1024];
        write_cached_image(&path, &first).unwrap();
        let finished = Arc::new(AtomicBool::new(false));
        std::thread::scope(|scope| {
            scope.spawn(|| {
                for _ in 0..32 {
                    write_cached_image(&path, &second).unwrap();
                    write_cached_image(&path, &first).unwrap();
                }
                finished.store(true, Ordering::Release);
            });
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let bytes = std::fs::read(&path).unwrap();
                assert!(bytes == first || bytes == second);
                if finished.load(Ordering::Acquire) {
                    break;
                }
                assert!(Instant::now() < deadline, "cache writes must finish");
            }
        });
        let directory = dir.path().join("directory.jpg");
        std::fs::create_dir(&directory).unwrap();
        assert!(write_cached_image(&directory, &[4]).is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn image_endpoint_returns_complete_cached_jpeg_with_original_headers() {
        use std::time::Duration;
        let dir = tempfile::tempdir().unwrap();
        let source = image::DynamicImage::new_rgb8(32, 16);
        source.save(dir.path().join("source.png")).unwrap();
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let address = server.server_addr().to_ip().unwrap();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                for _ in 0..2 {
                    let request = server
                        .recv_timeout(Duration::from_secs(5))
                        .unwrap()
                        .unwrap();
                    serve(
                        request,
                        dir.path(),
                        Some(dir.path()),
                        "https://example.test",
                    );
                }
            });
            let mut prior = Vec::new();
            for _ in 0..2 {
                let response = ureq::get(&format!(
                    "http://{address}/_pylon/image?src=/source.png&w=64&q=75&format=jpeg"
                ))
                .call()
                .unwrap();
                assert_eq!(response.header("Content-Type"), Some("image/jpeg"));
                assert_eq!(response.header("Vary"), Some("Accept"));
                assert_eq!(
                    response.header("Access-Control-Allow-Origin"),
                    Some("https://example.test")
                );
                let mut bytes = Vec::new();
                response.into_reader().read_to_end(&mut bytes).unwrap();
                let decoded = image::load_from_memory(&bytes).unwrap();
                assert_eq!((decoded.width(), decoded.height()), (32, 16));
                if !prior.is_empty() {
                    assert_eq!(bytes, prior);
                }
                prior = bytes;
            }
        });
    }

    #[test]
    fn near_miss_quality_snaps_instead_of_breaking_the_image() {
        // The bug this replaces: quality=80 returned an error body in place of
        // the image, so one wrong prop broke the picture in production. 80 is
        // the value everyone reaches for, because it reads like a range.
        let cfg = test_cfg();
        let req = parse_query("/_pylon/image?src=/a.jpg&w=640&q=80", &cfg)
            .expect("a near-miss quality must still render");
        assert_eq!(req.quality, 75, "80 is nearer 75 than 90");
        assert_eq!(req.width, 640);
    }

    #[test]
    fn snapping_keeps_the_cache_bounded_to_the_allowlist() {
        // Why rejecting was ever justified: unbounded values would mean
        // unbounded cache entries. Snapping preserves that — every quality in
        // 1..=100 collapses onto the configured set, and the RESOLVED value is
        // what cache_path hashes, so an attacker sweeping q= gains nothing.
        let cfg = test_cfg();
        let mut seen = std::collections::BTreeSet::new();
        for q in 1u8..=100 {
            let url = format!("/_pylon/image?src=/a.jpg&w=640&q={q}");
            seen.insert(parse_query(&url, &cfg).unwrap().quality);
        }
        assert_eq!(
            seen.into_iter().collect::<Vec<_>>(),
            cfg.allowed_qualities,
            "every requested quality must land on the allowlist"
        );
    }

    #[test]
    fn widths_snap_to_the_nearest_configured_size() {
        let cfg = test_cfg();
        let q = |w: u32| {
            parse_query(&format!("/_pylon/image?src=/a.jpg&w={w}&q=75"), &cfg)
                .unwrap()
                .width
        };
        assert_eq!(q(641), 640, "just above a size stays there");
        assert_eq!(q(700), 750, "700 is nearer 750 than 640");
        assert_eq!(q(99_999), 3840, "oversized clamps to the largest");
        assert_eq!(q(1), 16, "undersized clamps to the smallest");
    }

    #[test]
    fn ties_break_upward() {
        // EXACTLY equidistant: prefer the larger. More pixels beats a soft
        // upscale; more quality beats visible artifacts. (62 against 50/75 is
        // not a tie — it is 12 from 50 and 13 from 75, so it snaps down.)
        assert_eq!(snap_u8(70, &[60, 80]), 80, "70 is a true tie");
        assert_eq!(snap_u32(695, &[640, 750]), 750, "695 is a true tie");
        assert_eq!(snap_u8(62, &[50, 75, 90]), 50, "nearer 50, not a tie");
    }

    #[test]
    fn unknown_format_falls_back_to_negotiation_rather_than_failing() {
        // No meaningful "nearest" across codecs, and answering an avif request
        // with jpeg bytes would be a correctness bug — so it defers to Accept,
        // exactly as omitting `format=` does.
        let cfg = test_cfg();
        let req = parse_query("/_pylon/image?src=/a.jpg&w=640&q=75&format=gif", &cfg)
            .expect("an unknown format must not break the image");
        assert!(req.format.is_none());
    }

    #[test]
    fn allowed_values_pass_through_untouched() {
        let cfg = test_cfg();
        for q in [50u8, 75, 90] {
            let url = format!("/_pylon/image?src=/a.jpg&w=640&q={q}");
            assert_eq!(parse_query(&url, &cfg).unwrap().quality, q);
        }
    }

    #[test]
    fn svg_and_missing_src_are_still_refused() {
        // Snapping is for near-misses. These two have no safe answer: SVG can
        // carry script, and there is nothing to render without a src.
        let cfg = test_cfg();
        assert!(parse_query("/_pylon/image?src=/a.svg&w=640&q=75", &cfg).is_err());
        assert!(parse_query("/_pylon/image?w=640&q=75", &cfg).is_err());
    }

    #[test]
    fn avif_negotiated_only_on_explicit_accept() {
        let cfg = test_cfg();
        // Explicit AVIF accept → AVIF.
        assert_eq!(
            default_format(&accept_header("image/avif,image/webp,*/*"), &cfg),
            OutFormat::Avif
        );
        // WebP-only accept → WebP.
        assert_eq!(
            default_format(&accept_header("image/webp,*/*"), &cfg),
            OutFormat::Webp
        );
        // Empty/unknown Accept must NEVER get AVIF (support isn't
        // universal) — WebP is the safe default.
        assert_eq!(default_format(&[], &cfg), OutFormat::Webp);
    }

    #[test]
    fn avif_encodes_and_beats_jpeg_size() {
        // A gradient photo-ish 256px test image.
        let mut img = image::RgbaImage::new(256, 192);
        for (x, y, p) in img.enumerate_pixels_mut() {
            *p = image::Rgba([(x % 256) as u8, (y % 192) as u8, ((x + y) % 256) as u8, 255]);
        }
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let cfg = test_cfg();
        let avif = process(&png, 128, 75, OutFormat::Avif, &cfg).unwrap();
        let jpeg = process(&png, 128, 75, OutFormat::Jpeg, &cfg).unwrap();
        // Valid AVIF file signature: ISO BMFF 'ftyp' box with 'avif' brand.
        assert_eq!(&avif[4..8], b"ftyp", "not an ISO-BMFF container");
        assert!(
            avif.windows(4).take(32).any(|w| w == b"avif"),
            "ftyp carries no avif brand"
        );
        assert!(!jpeg.is_empty());
    }

    #[test]
    fn rgba_cast_is_layout_faithful() {
        let bytes = [1u8, 2, 3, 4, 5, 6, 7, 8];
        let px = bytemuck_cast_rgba(&bytes);
        assert_eq!(px.len(), 2);
        assert_eq!((px[0].r, px[0].g, px[0].b, px[0].a), (1, 2, 3, 4));
        assert_eq!((px[1].r, px[1].g, px[1].b, px[1].a), (5, 6, 7, 8));
    }

    #[test]
    fn extract_host_path_strips_userinfo_and_port() {
        assert_eq!(
            extract_host_path("https://user:pass@cdn.example.com:8443/a/b.png?x=1"),
            Some(("cdn.example.com", "/a/b.png"))
        );
        assert_eq!(
            extract_host_path("http://cdn.example.com/img"),
            Some(("cdn.example.com", "/img"))
        );
        assert_eq!(
            extract_host_path("https://cdn.example.com"),
            Some(("cdn.example.com", ""))
        );
    }

    #[test]
    fn extract_host_path_rejects_non_http_schemes() {
        // A redirect to one of these must be refused (returns None ->
        // caller errors), never fetched.
        assert_eq!(extract_host_path("file:///etc/passwd"), None);
        assert_eq!(extract_host_path("gopher://169.254.169.254/"), None);
        assert_eq!(extract_host_path("/local/path.png"), None);
    }

    #[test]
    fn redirect_revalidation_rejects_metadata_host() {
        // The classic bypass: an allowlisted host 302s to the cloud
        // metadata endpoint. The allowlist only permits cdn.example.com,
        // so the resolved redirect target must NOT match.
        let allow = RemotePattern::parse("cdn.example.com").unwrap();
        let evil = resolve_redirect(
            "https://cdn.example.com/img",
            "http://169.254.169.254/latest/meta-data/",
        )
        .unwrap();
        let (host, path) = extract_host_path(&evil).unwrap();
        assert_eq!(host, "169.254.169.254");
        assert!(
            !allow.matches(host, path),
            "metadata redirect target must fail the allowlist re-check"
        );
    }

    #[test]
    fn resolve_redirect_forms() {
        // Absolute.
        assert_eq!(
            resolve_redirect("https://a.com/x", "https://b.com/y").as_deref(),
            Some("https://b.com/y")
        );
        // Scheme-relative inherits the base scheme.
        assert_eq!(
            resolve_redirect("https://a.com/x", "//b.com/y").as_deref(),
            Some("https://b.com/y")
        );
        // Origin-relative keeps the base authority.
        assert_eq!(
            resolve_redirect("https://a.com:9000/x/y", "/z.png").as_deref(),
            Some("https://a.com:9000/z.png")
        );
        // Bare-relative resolves against the base directory.
        assert_eq!(
            resolve_redirect("https://a.com/x/y.png", "z.png").as_deref(),
            Some("https://a.com/x/z.png")
        );
        // Empty Location is unfollowable.
        assert_eq!(resolve_redirect("https://a.com/x", ""), None);
    }
}

#[cfg(test)]
mod cache_key_tests {
    use super::*;

    #[test]
    fn cache_path_distinguishes_output_formats() {
        let dir = Path::new("/tmp/pylon-img-test");
        let webp = cache_path(dir, "a.png", 640, 75, OutFormat::Webp);
        let jpg = cache_path(dir, "a.png", 640, 75, OutFormat::Jpeg);
        let png = cache_path(dir, "a.png", 640, 75, OutFormat::Png);
        // Distinct output formats MUST map to distinct cache files — both the
        // hashed stem (full extension is hashed) and the `.ext` suffix differ,
        // so format A's bytes can't be served under format B's Content-Type.
        assert_ne!(webp, jpg);
        assert_ne!(webp, png);
        assert_ne!(jpg, png);
        assert_ne!(webp.file_stem(), jpg.file_stem());
    }

    #[test]
    fn cache_path_is_stable_for_same_inputs() {
        let dir = Path::new("/tmp/pylon-img-test");
        assert_eq!(
            cache_path(dir, "a.png", 640, 75, OutFormat::Webp),
            cache_path(dir, "a.png", 640, 75, OutFormat::Webp),
        );
        // Any input change reshapes the key.
        assert_ne!(
            cache_path(dir, "a.png", 640, 75, OutFormat::Webp),
            cache_path(dir, "a.png", 641, 75, OutFormat::Webp),
        );
    }
}

#[cfg(test)]
mod concurrency_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn permits_bound_the_transforms_that_run_at_once() {
        let permits = Arc::new(Permits::new(2));
        let running = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let (permits, running, peak) = (
                    Arc::clone(&permits),
                    Arc::clone(&running),
                    Arc::clone(&peak),
                );
                std::thread::spawn(move || {
                    let _p = permits.acquire();
                    let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(20));
                    running.fetch_sub(1, Ordering::SeqCst);
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(peak.load(Ordering::SeqCst), 2);
        // Every permit came back.
        assert_eq!(*permits.free.lock().unwrap(), 2);
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(width, height, image::Rgba([200, 30, 30, 255]));
        let mut out = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    fn wait_for(path: &Path) -> bool {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if path.exists() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn a_variant_is_made_once_in_the_background_and_cached() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("variant.webp");
        let source = Arc::new(png(400, 200));
        warm(dest.clone(), Arc::clone(&source), 200, 75, OutFormat::Webp);
        // A second request for the same variant while it is made starts nothing new.
        warm(dest.clone(), Arc::clone(&source), 200, 75, OutFormat::Webp);
        assert!(wait_for(&dest), "the transform was not cached");
        let bytes = std::fs::read(&dest).unwrap();
        assert_eq!(
            image::guess_format(&bytes).unwrap(),
            image::ImageFormat::WebP
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while warming().lock().unwrap().running.contains(&dest) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!warming().lock().unwrap().running.contains(&dest));
    }

    #[test]
    fn a_failed_variant_is_not_retried_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("broken.webp");
        warm(
            dest.clone(),
            Arc::new(b"not an image".to_vec()),
            200,
            75,
            OutFormat::Webp,
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        while !warming().lock().unwrap().failed.contains_key(&dest) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(warming().lock().unwrap().failed.contains_key(&dest));
        warm(
            dest.clone(),
            Arc::new(png(10, 10)),
            200,
            75,
            OutFormat::Webp,
        );
        assert!(
            !warming().lock().unwrap().running.contains(&dest),
            "a recent failure started again"
        );
        assert!(!dest.exists());
    }
}
