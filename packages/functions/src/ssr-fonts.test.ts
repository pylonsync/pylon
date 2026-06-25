// Tests for the self-hosted font engine: sfnt metrics parsing, size-adjust
// math, CSS rendering, and the Google Fonts CSS2 request/parse helpers.

import { describe, expect, test } from "bun:test";
import {
  buildGoogleFontsUrl,
  computeFallbackFace,
  decodeFontMetrics,
  parseGoogleFontsCss,
  renderFontFaceCss,
  type ManifestFonts,
} from "./ssr-fonts";

// ---------------------------------------------------------------------------
// Synthesize a minimal valid raw sfnt (ttf) with head/hhea/OS-2 tables so we
// can assert the byte-offset metrics parser against known values.
// ---------------------------------------------------------------------------
function makeSfnt(opts: {
  unitsPerEm: number;
  hheaAscent: number;
  hheaDescent: number;
  hheaLineGap: number;
  xAvgCharWidth: number;
  useTypoMetrics: boolean;
  sTypoAscender: number;
  sTypoDescender: number;
  sTypoLineGap: number;
}): Uint8Array {
  const HEAD_LEN = 54;
  const HHEA_LEN = 36;
  const OS2_LEN = 96;
  const dirStart = 12;
  const numTables = 3;
  const dataStart = dirStart + numTables * 16; // 60
  const headOff = dataStart;
  const hheaOff = headOff + HEAD_LEN;
  const os2Off = hheaOff + HHEA_LEN;
  const total = os2Off + OS2_LEN;

  const buf = new Uint8Array(total);
  const dv = new DataView(buf.buffer);

  // sfnt offset table
  dv.setUint32(0, 0x00010000); // version 1.0 (truetype)
  dv.setUint16(4, numTables);

  // table directory (tag, checksum, offset, length). Tags sorted alphabetically
  // isn't required by our parser, which indexes by tag.
  function writeRecord(i: number, tag: string, off: number, len: number) {
    const p = dirStart + i * 16;
    buf[p] = tag.charCodeAt(0);
    buf[p + 1] = tag.charCodeAt(1);
    buf[p + 2] = tag.charCodeAt(2);
    buf[p + 3] = tag.charCodeAt(3);
    dv.setUint32(p + 4, 0); // checksum (unused by parser)
    dv.setUint32(p + 8, off);
    dv.setUint32(p + 12, len);
  }
  writeRecord(0, "head", headOff, HEAD_LEN);
  writeRecord(1, "hhea", hheaOff, HHEA_LEN);
  writeRecord(2, "OS/2", os2Off, OS2_LEN);

  // head: unitsPerEm @ +18
  dv.setUint16(headOff + 18, opts.unitsPerEm);

  // hhea: ascender @ +4, descender @ +6, lineGap @ +8
  dv.setInt16(hheaOff + 4, opts.hheaAscent);
  dv.setInt16(hheaOff + 6, opts.hheaDescent);
  dv.setInt16(hheaOff + 8, opts.hheaLineGap);

  // OS/2: xAvgCharWidth @ +2, fsSelection @ +62, sTypo* @ +68/+70/+72
  dv.setInt16(os2Off + 2, opts.xAvgCharWidth);
  dv.setUint16(os2Off + 62, opts.useTypoMetrics ? 0x80 : 0x00);
  dv.setInt16(os2Off + 68, opts.sTypoAscender);
  dv.setInt16(os2Off + 70, opts.sTypoDescender);
  dv.setInt16(os2Off + 72, opts.sTypoLineGap);

  return buf;
}

describe("decodeFontMetrics — raw sfnt", () => {
  test("reads unitsPerEm + hhea metrics + xAvgCharWidth", () => {
    const buf = makeSfnt({
      unitsPerEm: 1000,
      hheaAscent: 950,
      hheaDescent: -250,
      hheaLineGap: 0,
      xAvgCharWidth: 500,
      useTypoMetrics: false,
      sTypoAscender: 800,
      sTypoDescender: -200,
      sTypoLineGap: 10,
    });
    const m = decodeFontMetrics(buf);
    expect(m).not.toBeNull();
    expect(m!.unitsPerEm).toBe(1000);
    // USE_TYPO_METRICS off → hhea ascender/descender/lineGap.
    expect(m!.ascent).toBe(950);
    expect(m!.descent).toBe(-250);
    expect(m!.lineGap).toBe(0);
    expect(m!.xAvgCharWidth).toBe(500);
  });

  test("prefers OS/2 sTypo* when USE_TYPO_METRICS is set", () => {
    const buf = makeSfnt({
      unitsPerEm: 2048,
      hheaAscent: 1900,
      hheaDescent: -500,
      hheaLineGap: 0,
      xAvgCharWidth: 1000,
      useTypoMetrics: true,
      sTypoAscender: 1600,
      sTypoDescender: -400,
      sTypoLineGap: 90,
    });
    const m = decodeFontMetrics(buf);
    expect(m!.ascent).toBe(1600);
    expect(m!.descent).toBe(-400);
    expect(m!.lineGap).toBe(90);
  });

  test("returns null for a too-short / unknown buffer", () => {
    expect(decodeFontMetrics(new Uint8Array(4))).toBeNull();
  });
});

describe("computeFallbackFace — size-adjust math", () => {
  test("computes size-adjust + overrides against Arial", () => {
    // unitsPerEm 1000, xAvgCharWidth 500 → webAvg 0.5.
    // Arial xWidthAvg/em = 904/2048 = 0.4414062500.
    // sizeAdjust = 0.5 / 0.44140625 = 1.13274336...  → "113.27%"
    const face = computeFallbackFace(
      {
        unitsPerEm: 1000,
        ascent: 800,
        descent: -200,
        lineGap: 0,
        xAvgCharWidth: 500,
      },
      "Geist",
      "sans-serif",
    );
    expect(face.family).toBe("Geist Fallback");
    expect(face.local).toBe("Arial");
    expect(face.src).toEqual([]);
    expect(face.sizeAdjust).toBe("113.27%");
    // em = 1000 * 1.13274336 = 1132.7436; ascent 800/em ≈ 0.70625 → ~70.62%
    expect(parseFloat(face.ascentOverride!)).toBeCloseTo(70.62, 1);
    // descent is negative; override is the positive magnitude. 200/em ≈ 17.66%
    expect(parseFloat(face.descentOverride!)).toBeCloseTo(17.66, 1);
    expect(face.lineGapOverride).toBe("0.00%");
  });

  test("falls back to size-adjust 100% when avg widths are missing", () => {
    const face = computeFallbackFace(
      { unitsPerEm: 1000, ascent: 800, descent: -200, lineGap: 0, xAvgCharWidth: 0 },
      "Mono",
      "sans-serif",
    );
    expect(face.sizeAdjust).toBe("100.00%");
  });

  test("serif category uses Times New Roman as the local fallback", () => {
    const face = computeFallbackFace(
      { unitsPerEm: 2048, ascent: 1500, descent: -400, lineGap: 0, xAvgCharWidth: 900 },
      "Lora",
      "serif",
    );
    expect(face.local).toBe("Times New Roman");
  });
});

describe("renderFontFaceCss", () => {
  const fonts: ManifestFonts = {
    faces: [
      {
        family: "Geist",
        src: ["font-abc123.woff2"],
        weight: "400",
        style: "normal",
        display: "swap",
        unicodeRange: "U+0000-00FF",
      },
      {
        family: "Geist Fallback",
        src: [],
        local: "Arial",
        sizeAdjust: "113.27%",
        ascentOverride: "70.62%",
        descentOverride: "17.66%",
        lineGapOverride: "0.00%",
      },
    ],
    variables: { "--font-sans": '"Geist", "Geist Fallback", sans-serif' },
    preload: ["font-abc123.woff2"],
  };

  test("resolves woff2 URLs against the given prefix", () => {
    const css = renderFontFaceCss(fonts, "/_pylon/build/");
    expect(css).toContain(
      'src:url(/_pylon/build/font-abc123.woff2) format("woff2")',
    );
  });

  test("uses an absolute CDN prefix verbatim", () => {
    const css = renderFontFaceCss(fonts, "https://cdn.example.com/b/");
    expect(css).toContain("url(https://cdn.example.com/b/font-abc123.woff2)");
  });

  test("emits the size-adjusted fallback face with local() + overrides", () => {
    const css = renderFontFaceCss(fonts, "/_pylon/build/");
    expect(css).toContain('font-family:"Geist Fallback"');
    expect(css).toContain('src:local("Arial")');
    expect(css).toContain("size-adjust:113.27%");
    expect(css).toContain("ascent-override:70.62%");
    expect(css).toContain("descent-override:17.66%");
    expect(css).toContain("line-gap-override:0.00%");
  });

  test("emits the :root variable", () => {
    const css = renderFontFaceCss(fonts, "/_pylon/build/");
    expect(css).toContain(
      ':root{--font-sans:"Geist", "Geist Fallback", sans-serif;}',
    );
  });
});

describe("buildGoogleFontsUrl", () => {
  test("single axis, weights sorted ascending", () => {
    const url = buildGoogleFontsUrl({
      family: "Geist",
      weights: ["700", "400"],
      styles: ["normal"],
      subsets: ["latin"],
      display: "swap",
    });
    expect(url).toBe(
      "https://fonts.googleapis.com/css2?family=Geist:wght@400;700&display=swap",
    );
  });

  test("encodes spaces in the family name", () => {
    const url = buildGoogleFontsUrl({
      family: "Open Sans",
      weights: ["400"],
      styles: ["normal"],
      subsets: ["latin"],
      display: "swap",
    });
    expect(url).toContain("family=Open+Sans:wght@400");
  });

  test("italic uses the ital,wght axis with sorted tuples", () => {
    const url = buildGoogleFontsUrl({
      family: "Inter",
      weights: ["400", "700"],
      styles: ["normal", "italic"],
      subsets: ["latin"],
      display: "swap",
    });
    expect(url).toContain("Inter:ital,wght@0,400;0,700;1,400;1,700");
  });
});

describe("parseGoogleFontsCss", () => {
  const css = `
/* cyrillic */
@font-face {
  font-family: 'Geist';
  font-style: normal;
  font-weight: 400;
  font-display: swap;
  src: url(https://fonts.gstatic.com/s/geist/v1/cyr.woff2) format('woff2');
  unicode-range: U+0301, U+0400-045F;
}
/* latin */
@font-face {
  font-family: 'Geist';
  font-style: normal;
  font-weight: 400;
  font-display: swap;
  src: url(https://fonts.gstatic.com/s/geist/v1/latin.woff2) format('woff2');
  unicode-range: U+0000-00FF;
}
`;

  test("keeps only the requested subsets", () => {
    const blocks = parseGoogleFontsCss(css, ["latin"]);
    expect(blocks.length).toBe(1);
    expect(blocks[0].subset).toBe("latin");
    expect(blocks[0].url).toBe(
      "https://fonts.gstatic.com/s/geist/v1/latin.woff2",
    );
    expect(blocks[0].weight).toBe("400");
    expect(blocks[0].style).toBe("normal");
    expect(blocks[0].unicodeRange).toBe("U+0000-00FF");
  });

  test("returns every block when no subset filter is given", () => {
    const blocks = parseGoogleFontsCss(css, []);
    expect(blocks.length).toBe(2);
  });
});
