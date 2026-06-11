// Tests for the Next-style `opengraph-image.png` / `twitter-image.png`
// file convention wired up in `applyAutoSocialImages`.
import { afterEach, describe, expect, test } from "bun:test";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import {
  applyAutoIcons,
  applyAutoSocialImages,
  renderMetadata,
  buildHydrationTail,
  errorDigest,
  resolveOrigin,
  asRouteControl,
  PylonRouteControl,
} from "./ssr-runtime";

describe("resolveOrigin — Host-header allowlist (cache-poisoning fence)", () => {
  const publicUrl = "https://www.notbehind.com";

  test("trusts the Host only when it's the configured public origin", () => {
    expect(resolveOrigin({ host: "www.notbehind.com", publicUrl })).toBe(
      "https://www.notbehind.com",
    );
  });

  test("an attacker Host falls back to the public origin (no poisoning)", () => {
    // The crux: Host: evil.com must NOT produce https://evil.com (which would
    // be baked into og:image + teed into the shared ISR/CDN cache).
    expect(resolveOrigin({ host: "evil.com", publicUrl })).toBe(
      "https://www.notbehind.com",
    );
  });

  test("explicit PYLON_TRUSTED_HOSTS + canonical host are honored", () => {
    expect(
      resolveOrigin({ host: "cdn.notbehind.com", publicUrl, trustedHostsCsv: "cdn.notbehind.com, x.com" }),
    ).toBe("https://cdn.notbehind.com");
    expect(resolveOrigin({ host: "notbehind.com", publicUrl, canonicalHost: "notbehind.com" })).toBe(
      "https://notbehind.com",
    );
  });

  test("loopback is trusted for dev; X-Forwarded-Proto honored only there", () => {
    expect(resolveOrigin({ host: "localhost:4321" })).toBe("http://localhost:4321");
    // Attacker downgrade attempt on an untrusted host is ignored (falls back).
    expect(
      resolveOrigin({ host: "evil.com", forwardedProto: "http", publicUrl }),
    ).toBe("https://www.notbehind.com");
  });

  test("no public origin + untrusted host → empty (relative, never poisoned)", () => {
    expect(resolveOrigin({ host: "evil.com" })).toBe("");
  });
});

// Pull the JSON out of the `__PYLON_DATA__` <script> a hydration tail emits.
function extractPylonData(tail: string): any {
  const m = tail.match(
    /<script id="__PYLON_DATA__" type="application\/json">([\s\S]*?)<\/script>/,
  );
  if (!m) throw new Error("no __PYLON_DATA__ in tail");
  return JSON.parse(m[1]); // JSON.parse natively decodes the < escaping
}

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

    // Host must be allowlisted to be trusted for the absolute OG origin
    // (the cache-poisoning fence). Configure it as the public origin.
    const prevPub = process.env.PYLON_PUBLIC_URL;
    process.env.PYLON_PUBLIC_URL = "https://example.com";
    let md;
    try {
      md = applyAutoSocialImages("app/blog/page", { host: "example.com" }, undefined);
    } finally {
      if (prevPub === undefined) delete process.env.PYLON_PUBLIC_URL;
      else process.env.PYLON_PUBLIC_URL = prevPub;
    }

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

describe("buildHydrationTail — boundary hydration (#279) + strip (#270)", () => {
  const manifestRoute = { file: "app__error-x.js", imports: [], css: [] };

  test("error boundary serializes {message,digest}; raw error/stack/cookies NEVER cross the wire", () => {
    const tail = buildHydrationTail({
      component: "app/error",
      layouts: ["app/layout"],
      props: {
        url: "/boom",
        auth: { user_id: "u1", is_admin: false, tenant_id: null, roles: [] },
        // live, non-serializable + sensitive handles that MUST be stripped:
        error: new Error("DB exploded at secretHost:5432"),
        serverData: { get() {} },
        response: { setStatus() {} },
        reset: () => {},
        headers: { cookie: "pylon_session=SUPERSECRET" },
        cookies: { pylon_session: "SUPERSECRET" },
      },
      ssrData: {},
      manifestRoute,
      publicPrefix: "/_pylon/build/",
      manifestErr: null,
      kind: "error",
      errorForClient: { message: "Something went wrong", digest: "deadbeef" },
    });
    const data = extractPylonData(tail);
    expect(data.kind).toBe("error");
    expect(data.component).toBe("app/error");
    // The client error.tsx gets ONLY the safe projection.
    expect(data.props.error).toEqual({
      message: "Something went wrong",
      digest: "deadbeef",
    });
    // Live handles stripped; headers/cookies emptied (shape preserved).
    expect(data.props.serverData).toBeUndefined();
    expect(data.props.response).toBeUndefined();
    expect(data.props.reset).toBeUndefined();
    expect(data.props.headers).toEqual({});
    expect(data.props.cookies).toEqual({});
    // The session cookie + the raw error message/stack are nowhere in the blob.
    expect(tail).not.toContain("SUPERSECRET");
    expect(tail).not.toContain("secretHost");
    expect(tail).not.toContain("stack");
    // The per-boundary entry script is appended.
    expect(tail).toContain('src="/_pylon/build/app__error-x.js"');
  });

  test("not-found boundary carries kind but no error/reset", () => {
    const tail = buildHydrationTail({
      component: "app/not-found",
      layouts: [],
      props: { url: "/missing", auth: {}, response: {}, serverData: {} },
      ssrData: {},
      manifestRoute,
      publicPrefix: "/_pylon/build/",
      manifestErr: null,
      kind: "not-found",
    });
    const data = extractPylonData(tail);
    expect(data.kind).toBe("not-found");
    expect(data.props.error).toBeUndefined();
    expect(data.props.reset).toBeUndefined();
  });

  test("a page (no kind) hydrates without a kind field", () => {
    const tail = buildHydrationTail({
      component: "app/page",
      layouts: ["app/layout"],
      props: { url: "/", auth: {}, response: {}, serverData: {} },
      ssrData: { "list:Note": [] },
      manifestRoute,
      publicPrefix: "/_pylon/build/",
      manifestErr: null,
    });
    const data = extractPylonData(tail);
    expect(data.kind).toBeUndefined();
    expect(data.ssrData).toEqual({ "list:Note": [] });
  });

  test("no manifest entry → hydration-disabled warning, not an entry script", () => {
    const tail = buildHydrationTail({
      component: "app/page",
      layouts: [],
      props: { url: "/" },
      ssrData: {},
      manifestRoute: null,
      publicPrefix: "/_pylon/build/",
      manifestErr: "manifest crashed",
    });
    expect(tail).toContain("hydration disabled");
    expect(tail).not.toContain('type="module" src=');
  });

  test("errorDigest is deterministic, stack-free, 8 hex chars", () => {
    const e = new Error("boom");
    const d1 = errorDigest(e);
    const d2 = errorDigest(e);
    expect(d1).toBe(d2);
    expect(d1).toMatch(/^[0-9a-f]{8}$/);
    // A different error yields a different digest.
    expect(errorDigest(new Error("other"))).not.toBe(d1);
  });
});

describe("asRouteControl — route-control normalization (redirect/notFound)", () => {
  test("passes the framework's own PylonRouteControl straight through", () => {
    const redirect = new PylonRouteControl("redirect");
    redirect.url = "/login";
    redirect.redirectStatus = 302;
    expect(asRouteControl(redirect)).toBe(redirect);

    const nf = new PylonRouteControl("notFound");
    expect(asRouteControl(nf)).toBe(nf);
  });

  test("recognizes @pylonsync/react's branded notFound() error by digest", () => {
    // The cross-package contract: `notFound()` from @pylonsync/react throws an
    // error stamped `digest === "PYLON_NOT_FOUND"`. The runtime duck-types on
    // that brand (no import of the React class) and turns it into a notFound
    // control → a real 404 + nearest not-found.tsx. If this regresses, a
    // server page calling notFound() would 500 instead of 404.
    const reactNotFound = Object.assign(new Error("PYLON_NOT_FOUND"), {
      digest: "PYLON_NOT_FOUND",
    });
    const ctrl = asRouteControl(reactNotFound);
    expect(ctrl).not.toBeNull();
    expect(ctrl?.kind).toBe("notFound");
  });

  test("does NOT swallow ordinary errors as a 404 (fails open is forbidden)", () => {
    // The critical safety property: a real render error must fall through to
    // the error.tsx / 500 path, never be silently masked as a not-found.
    expect(asRouteControl(new Error("boom"))).toBeNull();
    expect(asRouteControl(new TypeError("nope"))).toBeNull();
    expect(asRouteControl({ digest: "SOME_OTHER_DIGEST" })).toBeNull();
    expect(asRouteControl({ digest: 42 })).toBeNull();
    expect(asRouteControl(null)).toBeNull();
    expect(asRouteControl(undefined)).toBeNull();
    expect(asRouteControl("PYLON_NOT_FOUND")).toBeNull(); // a bare string, not an error
  });
});
