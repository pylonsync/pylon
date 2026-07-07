import type { ReactNode } from "react";

/** A font for `ImageResponse`. Satori requires static TTF/OTF (not woff2,
 *  not variable fonts) — a variable font crashes the parser. */
export interface ImageResponseFont {
  name: string;
  data: ArrayBuffer | Buffer | Uint8Array;
  weight?: 100 | 200 | 300 | 400 | 500 | 600 | 700 | 800 | 900;
  style?: "normal" | "italic";
}

export interface ImageResponseOptions {
  /** Output width in px. Default 1200 (the standard OG card width). */
  width?: number;
  /** Output height in px. Default 630 (1.91:1). */
  height?: number;
  /** Override/extra fonts. When omitted the framework's bundled Inter
   *  (weights 400 + 600) is used. */
  fonts?: ImageResponseFont[];
  /** Response content type. Only "image/png" is produced today. */
  contentType?: string;
  /** Extra headers merged onto the image response (e.g. a custom
   *  `Cache-Control`). The framework sets a sensible default otherwise. */
  headers?: Record<string, string>;
  /** Satori emoji/asset loader passthrough (`graphemeImages` /
   *  `loadAdditionalAsset`) for rendering emoji or remote images. */
  loadAdditionalAsset?: (code: string, text: string) => Promise<string> | string;
}

/**
 * A response that renders a React element to an image (PNG), for the
 * `opengraph-image.tsx` / `twitter-image.tsx` file convention — the Pylon
 * equivalent of Next.js's `next/og` `ImageResponse`.
 *
 * Return one from an `opengraph-image` module's default export:
 *
 * ```tsx
 * import { ImageResponse } from "@pylonsync/react";
 * export const size = { width: 1200, height: 630 };
 * export default function OG() {
 *   return new ImageResponse(
 *     <div style={{ width: "100%", height: "100%", display: "flex" }}>
 *       Hello
 *     </div>,
 *     { ...size },
 *   );
 * }
 * ```
 *
 * This is a thin holder: it captures the element + options and the SSR
 * runtime does the Satori→resvg render server-side (so this package pulls
 * in no image-rendering deps). Satori is flexbox-only — an element with
 * more than one child must set `display: flex` (same rule as next/og).
 */
export class ImageResponse {
  /** Brand for structural detection in the runtime (survives bundling and
   *  cross-realm boundaries where `instanceof` would not). */
  readonly __pylonImageResponse = true as const;
  readonly element: ReactNode;
  readonly options: ImageResponseOptions;

  constructor(element: ReactNode, options: ImageResponseOptions = {}) {
    this.element = element;
    this.options = options;
  }
}

/** Structural check the runtime uses instead of `instanceof` (the user's
 *  module and the runtime may load different copies of this class). */
export function isImageResponse(v: unknown): v is ImageResponse {
  return (
    typeof v === "object" &&
    v !== null &&
    (v as { __pylonImageResponse?: unknown }).__pylonImageResponse === true
  );
}
