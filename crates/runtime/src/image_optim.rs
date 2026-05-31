//! Image optimization for `<Image>`.
//!
//! Serves `GET /_pylon/image?src=<path-or-url>&w=<width>&q=<quality>&format=<fmt>`.
//! Resizes the source to width `w`, re-encodes at quality `q` in the
//! requested format, and writes the result to a content-addressed
//! file under `.pylon/.cache/images/<hash>.<ext>` so subsequent
//! requests for the same params return instantly.
//!
//! Source resolution:
//!   - Path starts with `/`: treated as a relative path under the
//!     frontend dir. Out-of-tree paths are rejected.
//!   - Path starts with `http://` or `https://`: fetched via ureq.
//!     For production, gate with `PYLON_IMAGE_REMOTE_ALLOWLIST`
//!     (comma-separated hostnames). Without that env, only same-
//!     origin paths work.
//!
//! Format selection:
//!   - Explicit `format=webp|jpeg|png` always wins.
//!   - Otherwise: WebP when the request's `Accept` advertises it
//!     (every modern browser does), else JPEG.
//!
//! Cache-Control: hashed responses get `public, max-age=31536000,
//! immutable`. The hash encodes the exact transform so any change
//! produces a different URL.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tiny_http::{Header, Request, Response};

/// Allowed output formats. We deliberately exclude AVIF — see the
/// Cargo.toml comment for the dep cost trade-off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutFormat {
    Webp,
    Jpeg,
    Png,
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

/// Stringly-typed query parser — tiny_http gives us the path with
/// the query as one chunk, so we split manually. No urlencoded
/// values are expected for w/q/format; for src we percent-decode.
fn parse_query(url: &str) -> Option<ImageRequest> {
    let qs = url.split_once('?')?.1;
    let mut src: Option<String> = None;
    let mut width: u32 = 0;
    let mut quality: u8 = 75;
    let mut format: Option<OutFormat> = None;
    for kv in qs.split('&') {
        let (k, v) = match kv.split_once('=') {
            Some(p) => p,
            None => continue,
        };
        match k {
            "src" => src = Some(percent_decode(v)),
            "w" => width = v.parse().unwrap_or(0),
            "q" => quality = v.parse().unwrap_or(75).clamp(1, 100),
            "format" => format = OutFormat::from_str(v),
            _ => {}
        }
    }
    let src = src?;
    if !(32..=4096).contains(&width) {
        return None;
    }
    Some(ImageRequest {
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

/// Read the source bytes — either off the local frontend dir or
/// over HTTP for remote allowlisted URLs.
fn load_source(src: &str, frontend_dir: Option<&Path>) -> Result<Vec<u8>, String> {
    if src.starts_with("http://") || src.starts_with("https://") {
        let allowlist = std::env::var("PYLON_IMAGE_REMOTE_ALLOWLIST").unwrap_or_default();
        if allowlist.is_empty() {
            return Err(format!(
                "remote image source not allowed (set PYLON_IMAGE_REMOTE_ALLOWLIST): {src}"
            ));
        }
        // Hand-extract the host. `src` starts with http:// or https://.
        // Skip scheme, then take up to the first `/`, `?`, or end.
        // Strip any `user:pass@` prefix. Strip any `:port` suffix.
        let after_scheme = src
            .strip_prefix("http://")
            .or_else(|| src.strip_prefix("https://"))
            .unwrap_or("");
        let host_with_port = after_scheme
            .split(|c: char| c == '/' || c == '?' || c == '#')
            .next()
            .unwrap_or("");
        let host_no_auth = match host_with_port.rsplit_once('@') {
            Some((_, h)) => h,
            None => host_with_port,
        };
        let host = match host_no_auth.rsplit_once(':') {
            Some((h, port)) if port.chars().all(|c| c.is_ascii_digit()) => h,
            _ => host_no_auth,
        };
        let ok = allowlist
            .split(',')
            .map(|s| s.trim())
            .any(|allowed| !allowed.is_empty() && host == allowed);
        if !ok {
            return Err(format!("host {host} not in PYLON_IMAGE_REMOTE_ALLOWLIST"));
        }
        let res = ureq::get(src)
            .timeout(std::time::Duration::from_secs(15))
            .call()
            .map_err(|e| format!("fetch failed: {e}"))?;
        let mut buf: Vec<u8> = Vec::new();
        res.into_reader()
            .take(20_000_000) // 20MB safety cap
            .read_to_end(&mut buf)
            .map_err(|e| format!("read failed: {e}"))?;
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
fn process(input: &[u8], width: u32, quality: u8, fmt: OutFormat) -> Result<Vec<u8>, String> {
    use fast_image_resize as fir;
    use image::ImageReader;

    let cursor = Cursor::new(input);
    let reader = ImageReader::new(cursor)
        .with_guessed_format()
        .map_err(|e| format!("guess format failed: {e}"))?;
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
        // Lanczos3 — matches what we used to use, sharp at downscale.
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
fn default_format(headers: &[Header]) -> OutFormat {
    let accept = headers
        .iter()
        .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case("accept"))
        .map(|h| h.value.as_str().to_string())
        .unwrap_or_default();
    if accept.contains("image/webp") || accept.is_empty() {
        OutFormat::Webp
    } else {
        OutFormat::Jpeg
    }
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
    let url = request.url().to_string();
    let parsed = match parse_query(&url) {
        Some(p) => p,
        None => {
            let body = b"bad request: src / w required, w in [32, 4096]".to_vec();
            let _ = request.respond(Response::from_data(body).with_status_code(400u16));
            return;
        }
    };

    let fmt = parsed
        .format
        .unwrap_or_else(|| default_format(request.headers()));

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
        let source = match load_source(&parsed.src, frontend_dir) {
            Ok(s) => s,
            Err(e) => {
                respond_err(request, 400, &e);
                return;
            }
        };
        let processed = match process(&source, parsed.width, parsed.quality, fmt) {
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
