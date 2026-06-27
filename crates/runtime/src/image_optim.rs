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
//!   - Width must be in the `deviceSizes ∪ imageSizes` set
//!     (configurable; Next-style defaults). Prevents cache-fill DoS
//!     where an attacker requests N variations.
//!   - Quality must be in the `qualities` set (default 50/75/90).
//!   - Format must be in the `formats` set (default webp/jpeg).
//!   - Source bytes capped at `maxBytes` (default 25MB).
//!   - Decoded pixel count capped at `maxPixels` (default 100M).
//!     Prevents image-bomb DoS (tiny file, huge dimensions).
//!   - SVG inputs hard-rejected. We never accept or emit SVG —
//!     it can carry script.
//!
//! Cache-Control: hashed responses get `public, max-age=31536000,
//! immutable`. The hash encodes the exact transform so any change
//! produces a different URL.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tiny_http::{Header, Request, Response};

/// Allowed output formats. We deliberately exclude AVIF — see the
/// Cargo.toml comment for the dep cost trade-off. SVG is excluded
/// at the format level: never accepted as output, never accepted as
/// input either (parseable image-bomb / script vector).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutFormat {
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
/// | `PYLON_IMAGE_FORMATS` | `webp,jpeg` | Output formats allowed. |
/// | `PYLON_IMAGE_MAX_BYTES` | `26214400` (25MB) | Source byte cap. |
/// | `PYLON_IMAGE_MAX_PIXELS` | `100000000` (100M) | Decoded pixel cap. Prevents image bombs. |
/// | `PYLON_IMAGE_FETCH_TIMEOUT_MS` | `15000` | Remote-source fetch timeout. |
#[derive(Debug, Clone)]
struct ImageConfig {
    remote_allowlist: Vec<RemotePattern>,
    allowed_widths: Vec<u32>,
    allowed_qualities: Vec<u8>,
    allowed_formats: Vec<OutFormat>,
    max_bytes: u64,
    max_pixels: u64,
    fetch_timeout: std::time::Duration,
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
            vec![OutFormat::Webp, OutFormat::Jpeg],
            OutFormat::from_str,
        );

        let max_bytes = std::env::var("PYLON_IMAGE_MAX_BYTES")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(25 * 1024 * 1024);
        let max_pixels = std::env::var("PYLON_IMAGE_MAX_PIXELS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(100_000_000);
        let fetch_timeout_ms = std::env::var("PYLON_IMAGE_FETCH_TIMEOUT_MS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(15_000);

        ImageConfig {
            remote_allowlist,
            allowed_widths,
            allowed_qualities,
            allowed_formats,
            max_bytes,
            max_pixels,
            fetch_timeout: std::time::Duration::from_millis(fetch_timeout_ms),
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
            OutFormat::Webp => "webp",
            OutFormat::Jpeg => "jpg",
            OutFormat::Png => "png",
        }
    }
    fn mime(self) -> &'static str {
        match self {
            OutFormat::Webp => "image/webp",
            OutFormat::Jpeg => "image/jpeg",
            OutFormat::Png => "image/png",
        }
    }
    fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "webp" => Some(OutFormat::Webp),
            "jpeg" | "jpg" => Some(OutFormat::Jpeg),
            "png" => Some(OutFormat::Png),
            _ => None,
        }
    }
}

/// Parameters parsed off the request URL.
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

/// Reason a request was rejected before processing started. We
/// surface these as `400 Bad Request` with the variant name in the
/// body so an operator can tell why their `<Image>` configuration
/// isn't lining up with the allowlists.
#[derive(Debug)]
enum RejectReason {
    MissingSrc,
    DisallowedWidth(u32),
    DisallowedQuality(u8),
    DisallowedFormat(String),
    SvgRejected,
}

impl std::fmt::Display for RejectReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RejectReason::MissingSrc => write!(f, "missing `src` param"),
            RejectReason::DisallowedWidth(w) => write!(
                f,
                "width {w} not in PYLON_IMAGE_DEVICE_SIZES / PYLON_IMAGE_IMAGE_SIZES"
            ),
            RejectReason::DisallowedQuality(q) => {
                write!(f, "quality {q} not in PYLON_IMAGE_QUALITIES")
            }
            RejectReason::DisallowedFormat(s) => {
                write!(f, "format `{s}` not in PYLON_IMAGE_FORMATS")
            }
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

    let width = width.unwrap_or(0);
    if !cfg.allowed_widths.contains(&width) {
        return Err(RejectReason::DisallowedWidth(width));
    }
    let quality = quality.unwrap_or(75);
    if !cfg.allowed_qualities.contains(&quality) {
        return Err(RejectReason::DisallowedQuality(quality));
    }

    let format = match format_raw {
        Some(raw) => {
            let fmt = OutFormat::from_str(&raw)
                .ok_or_else(|| RejectReason::DisallowedFormat(raw.clone()))?;
            if !cfg.allowed_formats.contains(&fmt) {
                return Err(RejectReason::DisallowedFormat(raw));
            }
            Some(fmt)
        }
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
    h.update([fmt.extension().as_bytes()[0]]);
    let hash = h.finalize();
    let hex: String = hash.iter().take(16).map(|b| format!("{b:02x}")).collect();
    cache_dir.join(format!("{hex}.{}", fmt.extension()))
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

    let bytes = if dest.exists() {
        match std::fs::read(&dest) {
            Ok(b) => b,
            Err(e) => {
                respond_err(request, 500, &format!("cache read failed: {e}"));
                return;
            }
        }
    } else {
        let source = match load_source(&parsed.src, frontend_dir, cfg) {
            Ok(s) => s,
            Err(e) => {
                respond_err(request, 400, &e);
                return;
            }
        };
        let processed = match process(&source, parsed.width, parsed.quality, fmt, cfg) {
            Ok(b) => b,
            Err(e) => {
                respond_err(request, 500, &e);
                return;
            }
        };
        // Best-effort write — failing to cache is non-fatal, the
        // browser still gets a working image.
        let _ = std::fs::write(&dest, &processed);
        processed
    };

    let response = Response::from_data(bytes)
        .with_status_code(200u16)
        .with_header(Header::from_bytes("Content-Type", fmt.mime()).unwrap())
        .with_header(
            Header::from_bytes("Cache-Control", "public, max-age=31536000, immutable").unwrap(),
        )
        .with_header(
            Header::from_bytes(
                "Access-Control-Allow-Origin",
                cors_origin.as_bytes().to_vec(),
            )
            .unwrap(),
        );
    let _ = request.respond(response);
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
