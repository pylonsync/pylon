// Self-hosted web fonts — Pylon's next/font-parity primitive.
//
// At build time, for each `font({...})` declared in app.ts the bundler calls
// `buildFonts`, which:
//   1. fetches the family's CSS from the Google Fonts CSS2 API (modern UA → woff2),
//   2. downloads each woff2 face and self-hosts it under the client-build outdir
//      (served same-origin at `/_pylon/build/<file>.woff2`, immutable-cached),
//   3. parses the real sfnt metrics out of the woff2 (no third-party dep),
//   4. computes a size-adjusted fallback `@font-face` (size-adjust +
//      ascent/descent/line-gap overrides) against the local system font so the
//      web font swaps in with ZERO layout shift,
//   5. emits structured faces + the CSS variable into the bundle manifest.
//
// `renderFontFaceCss` (used by the SSR head injector) turns the structured faces
// back into a `<style>` blob at request time, resolving the woff2 URLs against
// the LIVE `public_prefix` (same-origin in dev/self-host, the CDN on cloud) so
// the URLs always match wherever the assets are actually served.
//
// The metrics parser reads ONLY the head/hhea/OS-2 tables (the line-box +
// average-width fields). It supports raw sfnt (ttf/otf) and woff2 (brotli-
// decompressed); glyf/loca transforms are accounted for when computing table
// offsets but never decoded.

import { brotliDecompressSync } from "node:zlib";

/** One `@font-face` rule. A real face carries `src` (woff2 files); a generated
 *  fallback face carries `local` + the size-adjust/override metrics instead. */
export interface FontFace {
  /** `font-family` — the real family, or `"<Family> Fallback"` for the
   *  size-adjusted fallback face. */
  family: string;
  /** outdir-relative woff2 filenames (real face). Empty for a fallback face. */
  src: string[];
  /** `local("...")` source for a size-adjusted fallback face, e.g. `"Arial"`. */
  local?: string;
  weight?: string;
  style?: string;
  display?: string;
  unicodeRange?: string;
  /** `size-adjust` percentage (fallback face only), e.g. `"107.40%"`. */
  sizeAdjust?: string;
  /** `ascent-override` percentage (fallback face only). */
  ascentOverride?: string;
  /** `descent-override` percentage (fallback face only). */
  descentOverride?: string;
  /** `line-gap-override` percentage (fallback face only). */
  lineGapOverride?: string;
}

/** The bundle manifest's `fonts` block: structured faces + CSS variables +
 *  the woff2 files to `<link rel=preload>`. Rendered into every SSR `<head>`. */
export interface ManifestFonts {
  faces: FontFace[];
  /** CSS custom property → font-family stack, e.g.
   *  `{"--font-sans": '"Geist", "Geist Fallback", sans-serif'}`. */
  variables: Record<string, string>;
  /** outdir-relative woff2 filenames to preload. */
  preload: string[];
}

interface SfntMetrics {
  unitsPerEm: number;
  ascent: number;
  descent: number;
  lineGap: number;
  xAvgCharWidth: number;
}

/** A `ManifestFont` as it appears in `pylon.manifest.json` (snake_case wire
 *  shape — see the SDK `font()` helper). */
interface ManifestFontInput {
  family: string;
  variable: string;
  weights?: string[];
  styles?: string[];
  subsets?: string[];
  display?: string;
  preload?: boolean;
  fallback?: string[];
  adjust_font_fallback?: boolean;
}

// A modern desktop Safari UA makes the Google Fonts CSS2 API return woff2 src
// URLs (the smallest, universally-supported modern format). We parse metrics
// straight out of the woff2 — no second request, no UA guessing for a ttf.
const MODERN_UA =
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 " +
  "(KHTML, like Gecko) Version/16.0 Safari/605.1.15";

// Average-character-width + em of the two CSS generic fallback faces, used to
// compute `size-adjust`. Values are the fonts' own OS/2 xAvgCharWidth /
// unitsPerEm (the same measure we read off the web font), so the ratio is
// meaningful. Sourced from the Arial / Times New Roman metric tables that
// @capsizecss/metrics publishes (the set next/font uses).
const FALLBACK_FONTS: Record<
  "sans-serif" | "serif",
  { name: string; xWidthAvg: number; unitsPerEm: number }
> = {
  "sans-serif": { name: "Arial", xWidthAvg: 904, unitsPerEm: 2048 },
  serif: { name: "Times New Roman", xWidthAvg: 821, unitsPerEm: 2048 },
};

// The 63 known table tags woff2's compact table directory indexes by ordinal
// (W3C WOFF2 spec §5.2). Flag index 63 means an explicit 4-byte tag follows.
const WOFF2_KNOWN_TAGS = [
  "cmap", "head", "hhea", "hmtx", "maxp", "name", "OS/2", "post",
  "cvt ", "fpgm", "glyf", "loca", "prep", "CFF ", "VORG", "EBDT",
  "EBLC", "gasp", "hdmx", "kern", "LTSH", "PCLT", "VDMX", "vhea",
  "vmtx", "BASE", "GDEF", "GPOS", "GSUB", "EBSC", "JSTF", "MATH",
  "CBDT", "CBLC", "COLR", "CPAL", "SVG ", "sbix", "acnt", "avar",
  "bdat", "bloc", "bsln", "cvar", "fdsc", "feat", "fmtx", "fvar",
  "gvar", "hsty", "just", "lcar", "mort", "morx", "opbd", "prop",
  "trak", "Zapf", "Silf", "Glat", "Gloc", "Feat", "Sill",
];

// ---------------------------------------------------------------------------
// Metrics parsing
// ---------------------------------------------------------------------------

/** Decode the line-box metrics from a font buffer (woff2 or raw sfnt). Returns
 *  null when the format/tables can't be read — callers degrade gracefully. */
export function decodeFontMetrics(buf: Uint8Array): SfntMetrics | null {
  if (buf.length < 12) return null;
  const sig = String.fromCharCode(buf[0], buf[1], buf[2], buf[3]);
  if (sig === "wOF2") return decodeWoff2Metrics(buf);
  // Raw sfnt: 0x00010000 (truetype), "true"/"typ1", or "OTTO" (CFF).
  return decodeSfntMetrics(buf);
}

/** UIntBase128: big-endian, 7 bits/byte, high bit = continuation, ≤5 bytes. */
function readBase128(buf: Uint8Array, p: number): [number, number] {
  let result = 0;
  for (let i = 0; i < 5; i++) {
    const b = buf[p++];
    result = result * 128 + (b & 0x7f);
    if ((b & 0x80) === 0) return [result, p];
  }
  return [result, p];
}

function decodeWoff2Metrics(buf: Uint8Array): SfntMetrics | null {
  const dv = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
  const numTables = dv.getUint16(12);
  const totalCompressedSize = dv.getUint32(20);
  let p = 48; // fixed woff2 header length

  const dir: { tag: string; streamLen: number }[] = [];
  for (let i = 0; i < numTables; i++) {
    const flags = buf[p++];
    const idx = flags & 0x3f;
    let tag: string;
    if (idx === 0x3f) {
      tag = String.fromCharCode(buf[p], buf[p + 1], buf[p + 2], buf[p + 3]);
      p += 4;
    } else {
      tag = WOFF2_KNOWN_TAGS[idx];
    }
    const transformVersion = (flags >> 6) & 0x3;
    const [origLength, afterOrig] = readBase128(buf, p);
    p = afterOrig;
    // glyf/loca are transformed at version 0 (null transform = version 3); every
    // other table is transformed only at a non-zero version. A transformed table
    // carries a transformLength and occupies THAT many bytes in the stream.
    const transformed =
      tag === "glyf" || tag === "loca"
        ? transformVersion === 0
        : transformVersion !== 0;
    let streamLen = origLength;
    if (transformed) {
      const [tlen, afterTransform] = readBase128(buf, p);
      p = afterTransform;
      streamLen = tlen;
    }
    dir.push({ tag, streamLen });
  }

  const compressed = buf.subarray(p, p + totalCompressedSize);
  let stream: Uint8Array;
  try {
    stream = brotliDecompressSync(compressed);
  } catch {
    return null;
  }

  // Tables are concatenated in directory order with no padding in the stream.
  const tables: Record<string, { offset: number; length: number }> = {};
  let off = 0;
  for (const d of dir) {
    tables[d.tag] = { offset: off, length: d.streamLen };
    off += d.streamLen;
  }
  return readMetricsFromTables(stream, tables);
}

function decodeSfntMetrics(buf: Uint8Array): SfntMetrics | null {
  const dv = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
  const numTables = dv.getUint16(4);
  const tables: Record<string, { offset: number; length: number }> = {};
  let p = 12; // after the sfnt offset table
  for (let i = 0; i < numTables; i++) {
    if (p + 16 > buf.length) break;
    const tag = String.fromCharCode(buf[p], buf[p + 1], buf[p + 2], buf[p + 3]);
    const offset = dv.getUint32(p + 8);
    const length = dv.getUint32(p + 12);
    tables[tag] = { offset, length };
    p += 16;
  }
  return readMetricsFromTables(buf, tables);
}

function readMetricsFromTables(
  data: Uint8Array,
  tables: Record<string, { offset: number; length: number }>,
): SfntMetrics | null {
  const head = tables["head"];
  const hhea = tables["hhea"];
  if (!head || !hhea) return null;
  const dv = new DataView(data.buffer, data.byteOffset, data.byteLength);

  const unitsPerEm = dv.getUint16(head.offset + 18);
  if (!unitsPerEm) return null;

  // hhea ascender/descender/lineGap are the default line-box metrics.
  let ascent = dv.getInt16(hhea.offset + 4);
  let descent = dv.getInt16(hhea.offset + 6);
  let lineGap = dv.getInt16(hhea.offset + 8);
  let xAvgCharWidth = 0;

  const os2 = tables["OS/2"];
  if (os2 && os2.length >= 74) {
    xAvgCharWidth = dv.getInt16(os2.offset + 2);
    const fsSelection = dv.getUint16(os2.offset + 62);
    // USE_TYPO_METRICS (bit 7): prefer the typographic ascender/descender/
    // lineGap — what a conformant browser uses for line layout when set.
    if (fsSelection & 0x80) {
      ascent = dv.getInt16(os2.offset + 68);
      descent = dv.getInt16(os2.offset + 70);
      lineGap = dv.getInt16(os2.offset + 72);
    }
  }
  return { unitsPerEm, ascent, descent, lineGap, xAvgCharWidth };
}

// ---------------------------------------------------------------------------
// Size-adjusted fallback computation
// ---------------------------------------------------------------------------

function fmtPct(v: number): string {
  return `${(Math.abs(v) * 100).toFixed(2)}%`;
}

/** Compute the size-adjusted fallback `@font-face` for a family. The web font's
 *  metrics, divided by the size-adjust factor, become the overrides — so the
 *  local fallback occupies the same line box + average advance as the web font
 *  will, eliminating the swap-in shift. */
export function computeFallbackFace(
  metrics: SfntMetrics,
  family: string,
  category: "sans-serif" | "serif",
): FontFace {
  const fb = FALLBACK_FONTS[category];
  const webAvg =
    metrics.xAvgCharWidth && metrics.unitsPerEm
      ? metrics.xAvgCharWidth / metrics.unitsPerEm
      : 0;
  const fbAvg = fb.xWidthAvg / fb.unitsPerEm;
  const sizeAdjust = webAvg && fbAvg ? webAvg / fbAvg : 1;
  const em = metrics.unitsPerEm * sizeAdjust;
  return {
    family: `${family} Fallback`,
    local: fb.name,
    src: [],
    sizeAdjust: `${(sizeAdjust * 100).toFixed(2)}%`,
    ascentOverride: fmtPct(metrics.ascent / em),
    descentOverride: fmtPct(metrics.descent / em),
    lineGapOverride: fmtPct(metrics.lineGap / em),
  };
}

// ---------------------------------------------------------------------------
// CSS rendering (request-time; resolves URLs against the live prefix)
// ---------------------------------------------------------------------------

/** Render the structured faces + `:root` variables into a CSS string. woff2 URLs
 *  are resolved against `prefix` (the live `public_prefix`) so they match
 *  wherever the assets are served (same-origin or CDN). */
export function renderFontFaceCss(
  fonts: ManifestFonts,
  prefix: string,
): string {
  let css = "";
  for (const f of fonts.faces || []) {
    css += "@font-face{";
    css += `font-family:"${f.family}";`;
    if (f.style) css += `font-style:${f.style};`;
    if (f.weight) css += `font-weight:${f.weight};`;
    if (f.display) css += `font-display:${f.display};`;
    if (f.src && f.src.length) {
      css += `src:${f.src
        .map((s) => `url(${prefix}${s}) format("woff2")`)
        .join(",")};`;
    } else if (f.local) {
      css += `src:local("${f.local}");`;
    }
    if (f.sizeAdjust) css += `size-adjust:${f.sizeAdjust};`;
    if (f.ascentOverride) css += `ascent-override:${f.ascentOverride};`;
    if (f.descentOverride) css += `descent-override:${f.descentOverride};`;
    if (f.lineGapOverride) css += `line-gap-override:${f.lineGapOverride};`;
    if (f.unicodeRange) css += `unicode-range:${f.unicodeRange};`;
    css += "}";
  }
  const vars = Object.entries(fonts.variables || {});
  if (vars.length) {
    css += ":root{";
    for (const [k, v] of vars) css += `${k}:${v};`;
    css += "}";
  }
  return css;
}

// ---------------------------------------------------------------------------
// Google Fonts CSS2 request + parse
// ---------------------------------------------------------------------------

interface FontSpec {
  family: string;
  weights: string[];
  styles: string[];
  subsets: string[];
  display: string;
}

interface FaceBlock {
  subset: string;
  style: string;
  weight: string;
  unicodeRange: string;
  url: string;
}

/** Compare two CSS2 weight tokens ("400", "300..700") ascending by their
 *  leading numeric value (CSS2 requires the axis list sorted ascending). */
function cmpWeight(a: string, b: string): number {
  return parseInt(a, 10) - parseInt(b, 10);
}

/** Build the Google Fonts CSS2 API URL for a spec. */
export function buildGoogleFontsUrl(spec: FontSpec): string {
  const fam = spec.family.trim().replace(/\s+/g, "+");
  const hasItalic = spec.styles.includes("italic");
  let axis: string;
  if (hasItalic) {
    const tuples: string[] = [];
    for (const st of ["normal", "italic"] as const) {
      if (!spec.styles.includes(st)) continue;
      const ital = st === "italic" ? 1 : 0;
      for (const w of spec.weights) tuples.push(`${ital},${w}`);
    }
    tuples.sort((a, b) => {
      const [ai, aw] = a.split(",");
      const [bi, bw] = b.split(",");
      return ai === bi ? cmpWeight(aw, bw) : Number(ai) - Number(bi);
    });
    axis = `:ital,wght@${tuples.join(";")}`;
  } else {
    const sorted = [...spec.weights].sort(cmpWeight);
    axis = `:wght@${sorted.join(";")}`;
  }
  return `https://fonts.googleapis.com/css2?family=${fam}${axis}&display=${spec.display}`;
}

/** Parse a Google Fonts CSS2 response into per-subset face blocks, keeping only
 *  the requested subsets. Each `@font-face` is preceded by a subset comment in
 *  the API output. */
export function parseGoogleFontsCss(
  css: string,
  subsets: string[],
): FaceBlock[] {
  const blocks: FaceBlock[] = [];
  const re = /\/\*\s*([^*]+?)\s*\*\/\s*@font-face\s*\{([^}]*)\}/g;
  for (const m of css.matchAll(re)) {
    const subset = m[1].trim();
    if (subsets.length && !subsets.includes(subset)) continue;
    const body = m[2];
    const style = (body.match(/font-style:\s*([^;]+);/)?.[1] || "normal").trim();
    const weight = (body.match(/font-weight:\s*([^;]+);/)?.[1] || "400").trim();
    const unicodeRange = (
      body.match(/unicode-range:\s*([^;]+);/)?.[1] || ""
    ).trim();
    const urlMatch = body.match(/url\(([^)]+)\)\s*format\(['"]?woff2['"]?\)/);
    if (!urlMatch) continue;
    const url = urlMatch[1].replace(/['"]/g, "");
    blocks.push({ subset, style, weight, unicodeRange, url });
  }
  return blocks;
}

// ---------------------------------------------------------------------------
// Build engine
// ---------------------------------------------------------------------------

interface FontCacheEntry {
  faces: FontFace[];
  metrics: SfntMetrics | null;
}

function fontCategory(font: ManifestFontInput): "sans-serif" | "serif" {
  const fb = font.fallback;
  if (fb && fb.includes("serif") && !fb.includes("sans-serif")) return "serif";
  return "sans-serif";
}

function readFontCache(
  fs: any,
  path: any,
  cacheDir: string,
): FontCacheEntry | null {
  const metaPath = path.join(cacheDir, "meta.json");
  if (!fs.existsSync(metaPath)) return null;
  try {
    const entry = JSON.parse(
      fs.readFileSync(metaPath, "utf8"),
    ) as FontCacheEntry;
    // Verify every referenced woff2 is still present.
    for (const face of entry.faces) {
      for (const fname of face.src) {
        if (!fs.existsSync(path.join(cacheDir, fname))) return null;
      }
    }
    return entry;
  } catch {
    return null;
  }
}

async function fetchAndCacheFont(
  fs: any,
  path: any,
  crypto: any,
  cacheDir: string,
  spec: FontSpec,
): Promise<FontCacheEntry> {
  const url = buildGoogleFontsUrl(spec);
  const res = await fetch(url, { headers: { "User-Agent": MODERN_UA } });
  if (!res.ok) {
    throw new Error(`Google Fonts CSS ${res.status} for ${url}`);
  }
  const css = await res.text();
  const blocks = parseGoogleFontsCss(css, spec.subsets);
  if (blocks.length === 0) {
    throw new Error(
      `no woff2 @font-face for "${spec.family}" (subsets: ${spec.subsets.join(", ")})`,
    );
  }

  fs.mkdirSync(cacheDir, { recursive: true });
  const faces: FontFace[] = [];
  let primaryBuf: Uint8Array | null = null;
  let primaryScore = -Infinity;

  for (const b of blocks) {
    const fontRes = await fetch(b.url);
    if (!fontRes.ok) {
      throw new Error(`woff2 fetch ${fontRes.status} for ${b.url}`);
    }
    const buf = new Uint8Array(await fontRes.arrayBuffer());
    const hash = crypto
      .createHash("sha256")
      .update(buf)
      .digest("hex")
      .slice(0, 12);
    const fname = `font-${hash}.woff2`;
    fs.writeFileSync(path.join(cacheDir, fname), buf);
    faces.push({
      family: spec.family,
      src: [fname],
      weight: b.weight,
      style: b.style,
      unicodeRange: b.unicodeRange,
    });
    // Prefer a normal-style, latin, weight-near-400 face for metrics.
    const score =
      (b.style === "normal" ? 100 : 0) +
      (b.subset === "latin" ? 50 : 0) -
      Math.abs(parseInt(b.weight, 10) - 400) / 100;
    if (score > primaryScore) {
      primaryScore = score;
      primaryBuf = buf;
    }
  }

  const metrics = primaryBuf ? decodeFontMetrics(primaryBuf) : null;
  const entry: FontCacheEntry = { faces, metrics };
  fs.writeFileSync(
    path.join(cacheDir, "meta.json"),
    JSON.stringify(entry),
    "utf8",
  );
  return entry;
}

async function buildOneFont(
  fs: any,
  path: any,
  crypto: any,
  cacheRoot: string,
  outdir: string,
  font: ManifestFontInput,
): Promise<ManifestFonts> {
  const spec: FontSpec = {
    family: font.family,
    weights: font.weights && font.weights.length ? font.weights : ["400"],
    styles: font.styles && font.styles.length ? font.styles : ["normal"],
    subsets: font.subsets && font.subsets.length ? font.subsets : ["latin"],
    display: font.display || "swap",
  };
  const key = crypto
    .createHash("sha256")
    .update(JSON.stringify(spec))
    .digest("hex")
    .slice(0, 16);
  const cacheDir = path.join(cacheRoot, key);

  let cached = readFontCache(fs, path, cacheDir);
  if (!cached) {
    cached = await fetchAndCacheFont(fs, path, crypto, cacheDir, spec);
  }

  const category = fontCategory(font);
  const doPreload = font.preload !== false;
  const faces: FontFace[] = [];
  const preloadFiles: string[] = [];

  // outdir is wiped each build, so re-emit the cached woff2 into it.
  for (const face of cached.faces) {
    const outSrc: string[] = [];
    for (const fname of face.src) {
      fs.copyFileSync(path.join(cacheDir, fname), path.join(outdir, fname));
      outSrc.push(fname);
      if (doPreload) preloadFiles.push(fname);
    }
    faces.push({
      family: font.family,
      src: outSrc,
      weight: face.weight,
      style: face.style,
      unicodeRange: face.unicodeRange,
      display: spec.display,
    });
  }

  const variables: Record<string, string> = {};
  const userFallback =
    font.fallback && font.fallback.length ? font.fallback : [category];
  if ((font.adjust_font_fallback ?? true) && cached.metrics) {
    faces.push(computeFallbackFace(cached.metrics, font.family, category));
    variables[font.variable] = [
      `"${font.family}"`,
      `"${font.family} Fallback"`,
      ...userFallback,
    ].join(", ");
  } else {
    variables[font.variable] = [`"${font.family}"`, ...userFallback].join(", ");
  }

  return { faces, variables, preload: Array.from(new Set(preloadFiles)) };
}

/** Build all declared fonts: fetch + self-host + metrics + structured faces.
 *  Network/parse failures for a single family degrade to a variable-only entry
 *  (so `var(--font-x)` still resolves to the fallback stack) rather than killing
 *  the build. Returns null when nothing was produced. */
export async function buildFonts(
  fs: any,
  path: any,
  cwd: string,
  outdir: string,
  fonts: ManifestFontInput[],
): Promise<ManifestFonts | null> {
  if (!fonts || fonts.length === 0) return null;
  const cryptoMod: any = await import("node:crypto");
  const crypto = cryptoMod.default ?? cryptoMod;
  const cacheRoot = path.join(cwd, ".pylon", "font-cache");

  const result: ManifestFonts = { faces: [], variables: {}, preload: [] };
  for (const font of fonts) {
    try {
      const built = await buildOneFont(
        fs,
        path,
        crypto,
        cacheRoot,
        outdir,
        font,
      );
      result.faces.push(...built.faces);
      Object.assign(result.variables, built.variables);
      result.preload.push(...built.preload);
    } catch (err: any) {
      // eslint-disable-next-line no-console
      console.warn(
        `[pylon ssr] font "${font.family}" failed: ${err?.message ?? err}`,
      );
      const category = fontCategory(font);
      const stack = (
        font.fallback && font.fallback.length ? font.fallback : [category]
      ).join(", ");
      result.variables[font.variable] = `"${font.family}", ${stack}`;
    }
  }

  if (result.faces.length === 0 && Object.keys(result.variables).length === 0) {
    return null;
  }
  return result;
}

/** Read the `fonts` array from the app's `pylon.manifest.json` (written next to
 *  app.ts). Returns [] when absent/unreadable. */
export function readManifestFonts(
  fs: any,
  path: any,
  cwd: string,
): ManifestFontInput[] {
  const manifestPath = path.join(cwd, "pylon.manifest.json");
  if (!fs.existsSync(manifestPath)) return [];
  try {
    const m = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
    return Array.isArray(m.fonts) ? m.fonts : [];
  } catch {
    return [];
  }
}
