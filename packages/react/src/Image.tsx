// Pylon's Next.js-style <Image>. Generates optimized + responsive
// images via Pylon's built-in Rust image processor.
//
// SSR output: a plain <img> with src + srcset + sizes pointing at
// `/_pylon/image?src=<your-src>&w=<width>&q=<quality>`. The Rust
// handler decodes, resizes with SIMD (fast_image_resize), and
// re-encodes with mozjpeg / libwebp on the fly, then caches the
// result by content hash so the second request is a static-file
// serve.
//
// What it does for you:
//   - Multiple widths in `srcset` so the browser picks the smallest
//     candidate for the viewport DPR.
//   - `sizes="100vw"` default — override for tighter layouts.
//   - `loading="lazy"` + `decoding="async"` by default (override
//     with `priority` for above-the-fold).
//   - `width` + `height` attrs locked to the explicit props so the
//     browser reserves space before paint (no CLS).
//
// What it doesn't do (Phase 2):
//   - No automatic blur placeholder. Pass `placeholder="<color>"` or
//     `placeholder={dataUri}` if you want one.
//   - No remote allowlist enforcement client-side — the server
//     refuses unknown hosts per `PYLON_IMAGE_REMOTE_ALLOWLIST`.
//   - No AVIF (server only emits WebP / JPEG / PNG today).

import React from "react";

export interface ImageProps
  extends Omit<
    React.ImgHTMLAttributes<HTMLImageElement>,
    "src" | "width" | "height" | "loading" | "srcSet"
  > {
  /** Source URL — site-relative (`/foo.jpg`) or http(s) (allowlisted via env). */
  src: string;
  /** Intrinsic width in CSS px (used for aspect ratio + the 1x candidate). */
  width: number;
  /** Intrinsic height in CSS px. */
  height: number;
  /** Required alt text. Pass `""` for purely decorative images. */
  alt: string;
  /**
   * JPEG/WebP quality 1..=100. Default 75 — matches Next.js.
   * PNG ignores it (lossless).
   */
  quality?: number;
  /**
   * Override the candidate widths used in `srcset`. By default we
   * emit 1x and 2x of `width`, capped at 3840px.
   */
  widths?: number[];
  /**
   * `<img sizes>` attribute. Default `100vw` — change to match the
   * container width so the browser picks the smallest srcset
   * candidate that fits. Example: `(max-width: 768px) 100vw, 50vw`.
   */
  sizes?: string;
  /** Skip lazy-loading + bump fetch priority. Use for above-the-fold hero images. */
  priority?: boolean;
}

const DEFAULT_QUALITY = 75;
const MAX_WIDTH = 3840;

/**
 * Build the optimized URL for a given width / quality. The server
 * picks the output format from the request's Accept header
 * (WebP > JPEG); explicit `format=` could be added later.
 */
function optimizedUrl(src: string, width: number, quality: number): string {
  const w = Math.min(Math.max(Math.round(width), 32), MAX_WIDTH);
  const q = Math.min(Math.max(quality, 1), 100);
  return `/_pylon/image?src=${encodeURIComponent(src)}&w=${w}&q=${q}`;
}

export function Image({
  src,
  width,
  height,
  alt,
  quality = DEFAULT_QUALITY,
  widths,
  sizes = "100vw",
  priority = false,
  className,
  style,
  ...rest
}: ImageProps) {
  // Candidate widths: explicit list, else 1x + 2x (capped).
  const candidates =
    widths && widths.length > 0
      ? Array.from(new Set(widths.map((w) => Math.round(w)))).sort((a, b) => a - b)
      : Array.from(new Set([width, Math.min(width * 2, MAX_WIDTH)])).sort(
          (a, b) => a - b,
        );

  const baseSrc = optimizedUrl(src, candidates[candidates.length - 1], quality);
  const srcSet = candidates
    .map((w) => `${optimizedUrl(src, w, quality)} ${w}w`)
    .join(", ");

  return (
    <img
      src={baseSrc}
      srcSet={srcSet}
      sizes={sizes}
      width={width}
      height={height}
      alt={alt}
      loading={priority ? "eager" : "lazy"}
      // React 19 uses the camelCase `fetchPriority`; older React
      // emits `fetchpriority`. Both serialize to the same HTML
      // attribute. We cast to bypass the typings since the prop
      // is still marked experimental on @types/react.
      {...({ fetchPriority: priority ? "high" : "auto" } as Record<string, string>)}
      decoding="async"
      className={className}
      style={style}
      {...rest}
    />
  );
}
