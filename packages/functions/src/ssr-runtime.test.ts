// Tests for the Next-style `opengraph-image.png` / `twitter-image.png`
// file convention wired up in `applyAutoSocialImages`.
import { afterEach, describe, expect, test } from "bun:test";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { applyAutoIcons, applyAutoSocialImages, renderMetadata } from "./ssr-runtime";

// `react` isn't a dependency of @pylonsync/functions — the SSR runtime
// imports it dynamically from the host project at render time. For unit
// tests we hand renderMetadata a fake React that records each created
// element's shape, which is all renderMetadata touches.
const FAKE_FRAGMENT = Symbol("Fragment");
const fakeReact = {
  Fragment: FAKE_FRAGMENT,
  createElement: (type: any, props: any, ...children: any[]) => ({
    type,
    props: props ?? {},
    children,
  }),
};

// A minimal PNG: 8-byte signature + an IHDR chunk carrying width/height.
// `readSocialImageMeta` only reads the first 32 bytes, so a full valid
// PNG isn't required to exercise the dimension reader.
function pngHeader(w: number, h: number): Buffer {
  const b = Buffer.alloc(24);
  b.write("\x89PNG\r\n\x1a\n", 0, "latin1");
  b.writeUInt32BE(13, 8); // IHDR length
  b.write("IHDR", 12, "latin1");
  b.writeUInt32BE(w, 16);
  b.writeUInt32BE(h, 20);
  return b;
}

let prevCwd: string | null = null;
const tmpDirs: string[] = [];

function makeApp(): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "pylon-og-"));
  tmpDirs.push(dir);
  prevCwd = process.cwd();
  process.chdir(dir);
  return dir;
}

afterEach(() => {
  if (prevCwd) {
    process.chdir(prevCwd);
    prevCwd = null;
  }
  for (const d of tmpDirs.splice(0)) {
    fs.rmSync(d, { recursive: true, force: true });
  }
});

describe("opengraph-image file convention", () => {
  test("auto-injects og:image + twitter:image with dims from a colocated png", () => {
    const dir = makeApp();
    fs.mkdirSync(path.join(dir, "app", "blog"), { recursive: true });
    fs.writeFileSync(
      path.join(dir, "app", "blog", "opengraph-image.png"),
      pngHeader(1200, 630),
    );

    const md = applyAutoSocialImages(
      "app/blog/page",
      { host: "example.com" },
      undefined,
    );

    expect(md?.openGraph?.image).toContain(
      "https://example.com/_pylon/og?src=app%2Fblog%2Fopengraph-image.png",
    );
    expect(md?.openGraph?.imageType).toBe("image/png");
    expect(md?.openGraph?.imageWidth).toBe(1200);
    expect(md?.openGraph?.imageHeight).toBe(630);
    expect(md?.openGraph?.imageSecureUrl).toBe(md?.openGraph?.image);
    // Twitter falls back to the og image + a large-summary card.
    expect(md?.twitter?.card).toBe("summary_large_image");
    expect(md?.twitter?.image).toContain(
      "/_pylon/og?src=app%2Fblog%2Fopengraph-image.png",
    );
  });

  test("inherits the nearest ancestor image (root app/opengraph-image.png)", () => {
    const dir = makeApp();
    fs.mkdirSync(path.join(dir, "app", "deep", "nested"), { recursive: true });
    fs.writeFileSync(
      path.join(dir, "app", "opengraph-image.png"),
      pngHeader(800, 418),
    );

    const md = applyAutoSocialImages(
      "app/deep/nested/page",
      { host: "x.test" },
      undefined,
    );
    expect(md?.openGraph?.image).toContain(
      "/_pylon/og?src=app%2Fopengraph-image.png",
    );
    expect(md?.openGraph?.imageWidth).toBe(800);
  });

  test("a closer image overrides an ancestor", () => {
    const dir = makeApp();
    fs.mkdirSync(path.join(dir, "app", "blog"), { recursive: true });
    fs.writeFileSync(path.join(dir, "app", "opengraph-image.png"), pngHeader(1, 1));
    fs.writeFileSync(
      path.join(dir, "app", "blog", "opengraph-image.png"),
      pngHeader(1200, 630),
    );
    const md = applyAutoSocialImages("app/blog/page", { host: "x.test" }, undefined);
    expect(md?.openGraph?.image).toContain(
      "/_pylon/og?src=app%2Fblog%2Fopengraph-image.png",
    );
    expect(md?.openGraph?.imageWidth).toBe(1200);
  });

  test("explicit metadata.openGraph.image always wins", () => {
    const dir = makeApp();
    fs.mkdirSync(path.join(dir, "app"), { recursive: true });
    fs.writeFileSync(path.join(dir, "app", "opengraph-image.png"), pngHeader(1, 1));
    const md = applyAutoSocialImages(
      "app/page",
      { host: "x.test" },
      { openGraph: { image: "https://cdn.example/custom.png" } },
    );
    expect(md?.openGraph?.image).toBe("https://cdn.example/custom.png");
  });

  test("no colocated image leaves metadata untouched", () => {
    makeApp(); // empty cwd, no app/ image
    const input = { title: "Hello" };
    const md = applyAutoSocialImages("app/page", { host: "x.test" }, input);
    expect(md).toEqual(input);
  });

  test("localhost host yields an http (non-secure) absolute URL", () => {
    const dir = makeApp();
    fs.mkdirSync(path.join(dir, "app"), { recursive: true });
    fs.writeFileSync(path.join(dir, "app", "opengraph-image.png"), pngHeader(10, 10));
    const md = applyAutoSocialImages("app/page", { host: "localhost:4321" }, undefined);
    expect(md?.openGraph?.image).toContain("http://localhost:4321/_pylon/og?src=");
    expect(md?.openGraph?.imageSecureUrl).toBeUndefined();
  });
});

describe("icon / apple-icon / favicon file convention", () => {
  test("auto-wires <link rel=icon> + apple-touch-icon (relative URL + sizes)", () => {
    const dir = makeApp();
    fs.mkdirSync(path.join(dir, "app"), { recursive: true });
    fs.writeFileSync(path.join(dir, "app", "icon.png"), pngHeader(512, 512));
    fs.writeFileSync(path.join(dir, "app", "apple-icon.png"), pngHeader(180, 180));

    const md = applyAutoIcons("app/page", undefined);
    expect(md?.icons?.icon?.url).toContain("/_pylon/og?src=app%2Ficon.png");
    expect(md?.icons?.icon?.url.startsWith("/")).toBe(true); // relative
    expect(md?.icons?.icon?.type).toBe("image/png");
    expect(md?.icons?.icon?.sizes).toBe("512x512");
    expect(md?.icons?.apple?.url).toContain("/_pylon/og?src=app%2Fapple-icon.png");
    expect(md?.icons?.apple?.sizes).toBe("180x180");
  });

  test("svg icon gets sizes=any; inherits from a parent folder", () => {
    const dir = makeApp();
    fs.mkdirSync(path.join(dir, "app", "blog"), { recursive: true });
    fs.writeFileSync(path.join(dir, "app", "icon.svg"), "<svg/>");
    const md = applyAutoIcons("app/blog/page", undefined);
    expect(md?.icons?.icon?.url).toContain("/_pylon/og?src=app%2Ficon.svg");
    expect(md?.icons?.icon?.type).toBe("image/svg+xml");
    expect(md?.icons?.icon?.sizes).toBe("any");
  });

  test("favicon.ico is the icon fallback", () => {
    const dir = makeApp();
    fs.mkdirSync(path.join(dir, "app"), { recursive: true });
    fs.writeFileSync(path.join(dir, "app", "favicon.ico"), Buffer.alloc(8));
    const md = applyAutoIcons("app/page", undefined);
    expect(md?.icons?.icon?.url).toContain("/_pylon/og?src=app%2Ffavicon.ico");
    expect(md?.icons?.icon?.type).toBe("image/x-icon");
    expect(md?.icons?.icon?.sizes).toBeUndefined(); // .ico is multi-size
  });

  test("explicit metadata.icons wins; no file → untouched", () => {
    const dir = makeApp();
    fs.mkdirSync(path.join(dir, "app"), { recursive: true });
    fs.writeFileSync(path.join(dir, "app", "icon.png"), pngHeader(1, 1));
    const explicit = { icons: { icon: { url: "/custom.ico" } } };
    expect(applyAutoIcons("app/page", explicit)?.icons?.icon?.url).toBe("/custom.ico");

    makeApp(); // fresh empty cwd
    const input = { title: "T" };
    expect(applyAutoIcons("app/page", input)).toEqual(input);
  });
});

describe("renderMetadata head-tag marking (client-nav sync)", () => {
  test("every <meta>/<link> carries data-pylon-meta; <title> does not", () => {
    const frag = renderMetadata(fakeReact, {
      title: "Hello",
      description: "A page",
      canonical: "https://x.test/p",
      openGraph: { title: "OG", image: "https://x.test/og.png" },
      twitter: { card: "summary" },
      icons: { icon: { url: "/icon.png" } },
    });
    expect(frag.type).toBe(FAKE_FRAGMENT);
    const kids: any[] = frag.children;
    const metaLink = kids.filter((k) => k.type === "meta" || k.type === "link");
    const titles = kids.filter((k) => k.type === "title");
    expect(metaLink.length).toBeGreaterThan(0);
    // The marker is what the client runtime swaps on navigation — without it,
    // SEO/social tags go stale on client-side nav. Every meta/link must carry
    // it; <title> must NOT (the client syncs document.title separately).
    for (const el of metaLink) {
      expect(el.props["data-pylon-meta"]).toBe("");
    }
    expect(titles.length).toBe(1);
    expect(titles[0].props["data-pylon-meta"]).toBeUndefined();
    expect(titles[0].children).toEqual(["Hello"]);
  });

  test("returns null when there's nothing to emit", () => {
    expect(renderMetadata(fakeReact, undefined)).toBeNull();
    expect(renderMetadata(fakeReact, {})).toBeNull();
  });
});
