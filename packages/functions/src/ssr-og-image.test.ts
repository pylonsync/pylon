// Tests for the dynamic OG image convention (app/**/opengraph-image.tsx →
// PNG, Next.js `next/og` parity): the render engine (`renderOgImage`, the
// Satori→resvg pipeline) plus `handleOgImageRoute` end-to-end — import a temp
// module, render, and emit the response_start/chunk/done protocol with an
// image content-type. Fixtures live UNDER this package dir so their
// `import React from "react"` / `@pylonsync/react` resolve via workspace
// walk-up (an os.tmpdir() fixture has no node_modules to reach).

import { afterEach, describe, expect, test } from "bun:test";
import * as React from "react";
import * as fs from "node:fs";
import * as path from "node:path";
import { fileURLToPath } from "node:url";
import { renderOgImage } from "./ssr-og-runtime";
import { handleOgImageRoute, type RenderRouteMessage } from "./ssr-runtime";

const PNG_MAGIC = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
const isPng = (b: Uint8Array): boolean =>
  Buffer.from(b.subarray(0, 8)).equals(PNG_MAGIC);
// PNG IHDR: 8-byte signature, then len(4)+"IHDR"(4)+width(4 BE)+height(4 BE).
const pngSize = (b: Uint8Array): { w: number; h: number } => {
  const buf = Buffer.from(b);
  return { w: buf.readUInt32BE(16), h: buf.readUInt32BE(20) };
};

describe("renderOgImage (Satori → resvg pipeline)", () => {
  test("renders a React element to a PNG of the requested size", async () => {
    const el = React.createElement(
      "div",
      {
        style: {
          display: "flex",
          width: "100%",
          height: "100%",
          alignItems: "center",
          justifyContent: "center",
          fontFamily: "Inter",
          fontSize: 48,
          color: "#0f172a",
          background: "#ffffff",
        },
      },
      "Hello OG",
    );
    const png = await renderOgImage(el, { width: 600, height: 400 });
    expect(isPng(png)).toBe(true);
    expect(png.byteLength).toBeGreaterThan(1000);
    expect(pngSize(png)).toEqual({ w: 600, h: 400 });
  });

  test("defaults to 1200×630 when no size is given", async () => {
    const el = React.createElement(
      "div",
      { style: { display: "flex", fontFamily: "Inter", fontSize: 40 } },
      "Default size",
    );
    const png = await renderOgImage(el);
    expect(pngSize(png)).toEqual({ w: 1200, h: 630 });
  });
});

describe("handleOgImageRoute (end-to-end import + render)", () => {
  const baseDir = path.dirname(fileURLToPath(import.meta.url));
  const tmpdirs: string[] = [];
  const prevCwd = process.cwd();
  afterEach(() => {
    process.chdir(prevCwd);
    for (const d of tmpdirs.splice(0)) {
      try {
        fs.rmSync(d, { recursive: true, force: true });
      } catch {
        /* best effort */
      }
    }
  });

  function fixture(src: string): void {
    const dir = fs.mkdtempSync(path.join(baseDir, "og-fixture-"));
    tmpdirs.push(dir);
    fs.mkdirSync(path.join(dir, "app"), { recursive: true });
    fs.writeFileSync(path.join(dir, "app", "opengraph-image.tsx"), src);
    process.chdir(dir);
  }

  const collect = async (): Promise<any[]> => {
    const sent: any[] = [];
    const msg = {
      component: "app/opengraph-image",
      call_id: "og1",
      params: {},
      search_params: {},
    } as unknown as RenderRouteMessage;
    await handleOgImageRoute(msg, (m) => sent.push(m));
    return sent;
  };

  test("ImageResponse default export → 200 image/png with the rendered bytes", async () => {
    // The fixture returns the exact shape `new ImageResponse(el, opts)`
    // produces — the `__pylonImageResponse` brand + `element`/`options` — so
    // this asserts the runtime's structural contract without pulling the
    // whole @pylonsync/react barrel into a temp module. The real
    // `ImageResponse` integration is covered by the live acme e2e.
    fixture(
      `import React from "react";
       export default function OG() {
         return {
           __pylonImageResponse: true,
           element: React.createElement("div",
             { style: { display: "flex", width: "100%", height: "100%", fontSize: 40, fontFamily: "Inter" } },
             "Branded"),
           options: { width: 500, height: 300, headers: { "cache-control": "public, max-age=60" } },
         };
       }`,
    );
    const sent = await collect();
    const start = sent.find((m) => m.type === "response_start");
    const chunk = sent.find((m) => m.type === "render_chunk");
    expect(start.status).toBe(200);
    expect(start.headers["content-type"]).toBe("image/png");
    // options.headers override the default cache-control.
    expect(start.headers["cache-control"]).toBe("public, max-age=60");
    const png = new Uint8Array(Buffer.from(chunk.data, "base64"));
    expect(isPng(png)).toBe(true);
    expect(pngSize(png)).toEqual({ w: 500, h: 300 });
    expect(sent.some((m) => m.type === "render_done")).toBe(true);
  });

  test("bare React element + `export const size` also renders", async () => {
    fixture(
      `import React from "react";
       export const size = { width: 640, height: 360 };
       export default function OG() {
         return React.createElement("div",
           { style: { display: "flex", fontSize: 32, fontFamily: "Inter" } }, "Bare");
       }`,
    );
    const sent = await collect();
    const start = sent.find((m) => m.type === "response_start");
    const png = new Uint8Array(
      Buffer.from(sent.find((m) => m.type === "render_chunk").data, "base64"),
    );
    expect(start.status).toBe(200);
    expect(start.headers["content-type"]).toBe("image/png");
    // default cache-control applies when the module set none.
    expect(start.headers["cache-control"]).toContain("public");
    expect(pngSize(png)).toEqual({ w: 640, h: 360 });
  });

  test("a throwing opengraph-image surfaces as 500 text/plain (no wedge)", async () => {
    fixture(
      `export default function OG() { throw new Error("boom"); }`,
    );
    const sent = await collect();
    const start = sent.find((m) => m.type === "response_start");
    const body = Buffer.from(
      sent.find((m) => m.type === "render_chunk").data,
      "base64",
    ).toString("utf8");
    expect(start.status).toBe(500);
    expect(start.headers["content-type"]).toContain("text/plain");
    expect(body).toContain("boom");
    expect(sent.some((m) => m.type === "render_done")).toBe(true);
  });
});
