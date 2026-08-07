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
  isSafeRedirect,
  asRouteControl,
  PylonRouteControl,
  finalizeHeaders,
  escapeScriptJson,
  makeResponseController,
  makeReadTrackingProxy,
  makeRevocableReadTrackingProxy,
  jsonClone,
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

  test("off-loopback never honors X-Forwarded-Proto (no downgrade poisoning)", () => {
    // Even on a TRUSTED host, a client-supplied proto must not change the
    // absolute origin — it feeds og:image/canonical and is cache-keyed only by
    // host, so honoring `http` would downgrade the cached URL for everyone.
    // Off-loopback is ALWAYS https.
    expect(
      resolveOrigin({ host: "www.notbehind.com", publicUrl, forwardedProto: "http" }),
    ).toBe("https://www.notbehind.com");
    expect(
      resolveOrigin({
        host: "www.notbehind.com",
        publicUrl,
        forwardedProto: "javascript:alert(1)",
      }),
    ).toBe("https://www.notbehind.com");
    // Loopback (dev) may be http; an explicit https there is honored.
    expect(resolveOrigin({ host: "localhost:4321", forwardedProto: "http" })).toBe(
      "http://localhost:4321",
    );
    expect(resolveOrigin({ host: "localhost:4321", forwardedProto: "https" })).toBe(
      "https://localhost:4321",
    );
  });
});

describe("reserved x-pylon-* header namespace (cache-proof forgery fence)", () => {
  test("response.setHeader() rejects the reserved x-pylon-* namespace", () => {
    const state = {
      status: 200,
      headers: {} as Record<string, string>,
      cookies: [] as string[],
    };
    const res = makeResponseController(state);
    // Forging the #277 cache proof from userland must throw, not silently set it.
    expect(() => res.setHeader("x-pylon-cacheable", "300")).toThrow(/reserved/i);
    expect(() => res.setHeader("X-Pylon-Anything", "1")).toThrow(/reserved/i);
    // An ordinary header still works.
    res.setHeader("x-custom", "ok");
    expect(state.headers["x-custom"]).toBe("ok");
  });

  test("finalizeHeaders: x-pylon-* survives ONLY from the trusted internal channel", () => {
    // The #277 proof must come from the 3rd `internal` arg. A page-set header
    // (state.headers) OR a route-handler header (the 2nd `extra` arg, which
    // ssr-form-runtime fills from user-returned headers) is stripped — so
    // userland can't forge the host-side cache verdict through ANY path.
    const state = {
      status: 200,
      headers: { "x-pylon-cacheable": "999", "x-keep": "yes" } as Record<string, string>,
      cookies: [] as string[],
    };
    const out = finalizeHeaders(
      state,
      { "x-pylon-cacheable": "888", "x-extra": "e" }, // untrusted extra → stripped
      { "x-pylon-cacheable": "60" }, // trusted internal → kept
    );
    expect(out["x-pylon-cacheable"]).toBe("60"); // only the trusted value
    expect(out["x-keep"]).toBe("yes");
    expect(out["x-extra"]).toBe("e"); // a non-reserved extra header still merges

    // No internal proof → NO x-pylon-* survives, from page headers OR extra.
    const out2 = finalizeHeaders(
      { status: 200, headers: { "x-pylon-cacheable": "999" }, cookies: [] as string[] },
      { "x-pylon-cacheable": "777" },
    );
    expect(out2["x-pylon-cacheable"]).toBeUndefined();
  });

  test("finalizeHeaders: 3xx open-redirect guard on setHeader('location') / returned headers", () => {
    const st = (status: number, location: string) => ({
      status,
      headers: { location } as Record<string, string>,
      cookies: [] as string[],
    });

    // An off-site absolute Location on a 3xx (set via setHeader or a route
    // handler's returned headers) is refused — the same rule redirect() applies.
    expect(() => finalizeHeaders(st(302, "https://evil.example/steal"))).toThrow(
      /open redirect/i,
    );
    // Protocol-relative `//host` is the classic bypass — also refused.
    expect(() => finalizeHeaders(st(307, "//evil.example"))).toThrow(/open redirect/i);
    // A backslash variant that browsers normalize cross-origin — refused.
    expect(() => finalizeHeaders(st(303, "/\\evil.example"))).toThrow(/open redirect/i);

    // A same-site relative path is fine and passes through unchanged.
    expect(finalizeHeaders(st(302, "/dashboard"))["location"]).toBe("/dashboard");

    // Case-insensitive header name is still caught (host lowercases, but guard
    // must not depend on that).
    expect(() =>
      finalizeHeaders({
        status: 302,
        headers: { Location: "https://evil.example" } as Record<string, string>,
        cookies: [],
      }),
    ).toThrow(/open redirect/i);

    // NON-3xx status → `location` is just a header, not a redirect: no guard.
    expect(
      finalizeHeaders(st(200, "https://evil.example"))["location"],
    ).toBe("https://evil.example");

    // A raw `route.ts` GET returns its OWN status via the `effectiveStatus`
    // arg — an off-site Location there is still refused even though
    // `state.status` is 200.
    expect(() =>
      finalizeHeaders(
        { status: 200, headers: {}, cookies: [] },
        { location: "https://evil.example" },
        undefined,
        302,
      ),
    ).toThrow(/open redirect/i);
  });

  test("makeReadTrackingProxy trips on get / in / Object.keys / descriptor / spread", () => {
    const probes: Array<(o: any) => unknown> = [
      (o) => o.host,
      (o) => "host" in o,
      (o) => Object.keys(o),
      (o) => Object.getOwnPropertyDescriptor(o, "host"),
      (o) => ({ ...o }),
    ];
    for (const probe of probes) {
      let touched = false;
      const p = makeReadTrackingProxy({ host: "x" }, () => {
        touched = true;
      });
      probe(p);
      expect(touched).toBe(true); // a bare `get` trap would miss in/keys
    }
    // No observation → never touched.
    let t = false;
    makeReadTrackingProxy({ host: "x" }, () => {
      t = true;
    });
    expect(t).toBe(false);
  });

  test("revocable proxy throws after revoke (stale module-stashed props fence)", () => {
    // P0 (codex 2026-06-28): a page that stashes `props` (or `props.auth`) in
    // module-level state and reads it on a LATER render must not silently read a
    // prior request's identity without tripping THIS render's read-tracking. The
    // render path revokes each per-request proxy when the render ends, so any
    // retained reference throws on access — fail-closed.
    const { proxy, revoke } = makeRevocableReadTrackingProxy(
      { user_id: "alice" },
      () => {},
    );
    expect((proxy as any).user_id).toBe("alice"); // live during the render
    revoke();
    // A stashed reference, read on a later render:
    expect(() => (proxy as any).user_id).toThrow();
    expect(() => "user_id" in proxy).toThrow();
    expect(() => Object.keys(proxy)).toThrow();
  });

  test("jsonClone snapshot is independent of later source mutation (bucket params fence)", () => {
    // The bucket-tail snapshot (bucketTailBase) is jsonClone'd at render START.
    // The defense rests on the clone being decoupled from the live params object:
    // a page mutating a NESTED field afterwards (props.searchParams.leak =
    // props.auth) can't reach the already-captured snapshot.
    const source: any = { id: "a", nested: { keep: 1 } };
    const snap = jsonClone(source);
    // Simulate the page smuggling identity in after the snapshot was taken.
    source.leak = { user_id: "alice" };
    source.nested.keep = 999;
    expect(snap).toEqual({ id: "a", nested: { keep: 1 } });
    expect((snap as any).leak).toBeUndefined();
    // And it strips non-JSON values (a proxy aliased in would serialize as its
    // target via JSON, but a function/symbol is dropped entirely).
    const stripped = jsonClone({ ok: "v", fn: () => 1, sym: Symbol("x") } as any);
    expect(stripped).toEqual({ ok: "v" });
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
  // Only the FIRST call records the restore target. A second makeApp()
  // in the same test would otherwise capture the previous temp dir,
  // and afterEach would chdir back into it right before deleting it —
  // leaving the whole process on a deleted cwd, which breaks every
  // later test file that spawns a subprocess.
  if (prevCwd === null) prevCwd = process.cwd();
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

/**
 * Relative social URLs. Facebook and Slack resolve them; Twitter drops
 * the image and ships a grey box, so a card looks fine everywhere the
 * author checked and broken where it mattered.
 */
describe("absolutizing author-supplied social URLs", () => {
  const withOrigin = <T,>(fn: () => T): T => {
    const prev = process.env.PYLON_PUBLIC_URL;
    process.env.PYLON_PUBLIC_URL = "https://www.example.com";
    try {
      return fn();
    } finally {
      if (prev === undefined) delete process.env.PYLON_PUBLIC_URL;
      else process.env.PYLON_PUBLIC_URL = prev;
    }
  };
  const apply = (md: any, url = "/") =>
    withOrigin(() =>
      applyAutoSocialImages("app/page", { host: "www.example.com" }, md, url),
    );

  test("a root-relative og:image becomes absolute", () => {
    const md = apply({
      openGraph: { image: "/marketing/og.png" },
      twitter: { card: "summary_large_image", image: "/marketing/og.png" },
    });
    expect(md?.openGraph?.image).toBe("https://www.example.com/marketing/og.png");
    expect(md?.twitter?.image).toBe("https://www.example.com/marketing/og.png");
  });

  test("an https origin also fills og:image:secure_url", () => {
    const md = apply({ openGraph: { image: "/og.png" } });
    expect(md?.openGraph?.imageSecureUrl).toBe("https://www.example.com/og.png");
  });

  test("an absolute URL is left exactly as authored", () => {
    const md = apply({
      openGraph: { image: "https://cdn.other.test/a.png" },
      twitter: { image: "https://cdn.other.test/a.png" },
    });
    expect(md?.openGraph?.image).toBe("https://cdn.other.test/a.png");
    expect(md?.twitter?.image).toBe("https://cdn.other.test/a.png");
  });

  test("a data: URL is left alone", () => {
    const md = apply({ openGraph: { image: "data:image/png;base64,iVBOR" } });
    expect(md?.openGraph?.image).toBe("data:image/png;base64,iVBOR");
  });

  test("a protocol-relative URL picks up the origin's scheme", () => {
    const md = apply({ openGraph: { image: "//cdn.other.test/a.png" } });
    expect(md?.openGraph?.image).toBe("https://cdn.other.test/a.png");
  });

  test("a path-relative URL resolves against the request path", () => {
    const md = apply({ openGraph: { image: "og.png" } }, "/blog/post-1");
    expect(md?.openGraph?.image).toBe("https://www.example.com/blog/og.png");
  });

  test("og:url and canonical are absolutized too", () => {
    const md = apply({
      canonical: "/pricing",
      openGraph: { image: "/og.png", url: "/pricing" },
    });
    expect(md?.canonical).toBe("https://www.example.com/pricing");
    expect(md?.openGraph?.url).toBe("https://www.example.com/pricing");
  });

  test("every entry in an images list is absolutized", () => {
    const md = apply({
      openGraph: { images: [{ url: "/a.png" }, { url: "https://x.test/b.png" }] },
    });
    expect(md?.openGraph?.images?.[0]?.url).toBe("https://www.example.com/a.png");
    expect(md?.openGraph?.images?.[1]?.url).toBe("https://x.test/b.png");
  });

  test("metadata with no social URLs is untouched", () => {
    const md = apply({ title: "Hi" });
    expect(md?.title).toBe("Hi");
    expect(md?.openGraph).toBeUndefined();
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

  test("emits og:site_name when openGraph.siteName is set", () => {
    const frag = renderMetadata(fakeReact, {
      openGraph: { title: "OG", siteName: "Pylon" },
    });
    const metas: any[] = frag.children.filter((k: any) => k.type === "meta");
    const siteName = metas.find((m) => m.props.property === "og:site_name");
    expect(siteName).toBeDefined();
    expect(siteName.props.content).toBe("Pylon");
    // It's a head tag like the rest, so it must carry the nav-swap marker.
    expect(siteName.props["data-pylon-meta"]).toBe("");
  });

  test("omits og:site_name when not provided", () => {
    const frag = renderMetadata(fakeReact, { openGraph: { title: "OG" } });
    const metas: any[] = frag.children.filter((k: any) => k.type === "meta");
    expect(metas.some((m) => m.props.property === "og:site_name")).toBe(false);
  });

  test("returns null when there's nothing to emit", () => {
    expect(renderMetadata(fakeReact, undefined)).toBeNull();
    expect(renderMetadata(fakeReact, {})).toBeNull();
  });

  test("emits JSON-LD as an escaped application/ld+json script", () => {
    const frag = renderMetadata(fakeReact, {
      jsonLd: {
        "@context": "https://schema.org",
        "@type": "Organization",
        name: "Pylon </script><x>&y",
      },
    });
    const scripts: any[] = frag.children.filter((k: any) => k.type === "script");
    expect(scripts.length).toBe(1);
    expect(scripts[0].props.type).toBe("application/ld+json");
    expect(scripts[0].props["data-pylon-meta"]).toBe("");
    const body = scripts[0].children[0] as string;
    // Breakout chars must be \u-escaped — the payload can't contain a literal
    // `</script>`, `<`, `>`, or `&`.
    expect(body).not.toContain("</script>");
    expect(body).not.toMatch(/[<>&]/);
    expect(body).toContain("\\u003c");
    // …and it's still valid structured data once a parser decodes it.
    const parsed = JSON.parse(body);
    expect(parsed["@type"]).toBe("Organization");
    expect(parsed.name).toBe("Pylon </script><x>&y");
  });

  test("JSON-LD array emits one script per item", () => {
    const frag = renderMetadata(fakeReact, {
      jsonLd: [{ "@type": "A" }, { "@type": "B" }],
    });
    const scripts: any[] = frag.children.filter((k: any) => k.type === "script");
    expect(scripts.map((s) => JSON.parse(s.children[0])["@type"])).toEqual(["A", "B"]);
  });

  test("emits the extended SEO/social tags", () => {
    const frag = renderMetadata(fakeReact, {
      authors: ["Ada", "Grace"],
      themeColor: "#0b5fff",
      openGraph: {
        locale: "en_US",
        images: [{ url: "https://x.test/a.png", width: 1200, height: 630, alt: "A" }],
        article: { author: "Ada", publishedTime: "2026-01-01", tags: ["ai", "ssr"] },
      },
      twitter: { card: "summary", site: "@pylon", creator: "@ada", imageAlt: "card" },
      alternates: {
        languages: { "en-US": "https://x.test/en", "fr-FR": "https://x.test/fr" },
      },
    });
    const metas: any[] = frag.children.filter((k: any) => k.type === "meta");
    const links: any[] = frag.children.filter((k: any) => k.type === "link");
    const find = (sel: (m: any) => boolean) => metas.find(sel);

    expect(
      metas.filter((m) => m.props.name === "author").map((m) => m.props.content),
    ).toEqual(["Ada", "Grace"]);
    expect(find((m) => m.props.name === "theme-color")?.props.content).toBe("#0b5fff");
    expect(find((m) => m.props.property === "og:locale")?.props.content).toBe("en_US");
    expect(
      find((m) => m.props.property === "og:image" && m.props.content === "https://x.test/a.png"),
    ).toBeDefined();
    expect(find((m) => m.props.property === "article:author")?.props.content).toBe("Ada");
    expect(find((m) => m.props.property === "article:published_time")?.props.content).toBe(
      "2026-01-01",
    );
    expect(
      metas.filter((m) => m.props.property === "article:tag").map((m) => m.props.content),
    ).toEqual(["ai", "ssr"]);
    expect(find((m) => m.props.name === "twitter:site")?.props.content).toBe("@pylon");
    expect(find((m) => m.props.name === "twitter:creator")?.props.content).toBe("@ada");
    expect(find((m) => m.props.name === "twitter:image:alt")?.props.content).toBe("card");

    const alts = links.filter((l) => l.props.rel === "alternate");
    expect(alts.map((l) => [l.props.hrefLang, l.props.href])).toEqual([
      ["en-US", "https://x.test/en"],
      ["fr-FR", "https://x.test/fr"],
    ]);
    // Every emitted meta/link still carries the nav-swap marker.
    for (const el of [...metas, ...links]) {
      expect(el.props["data-pylon-meta"]).toBe("");
    }
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

describe("isSafeRedirect — open-redirect guard for response.redirect()", () => {
  const trusted = {
    publicUrl: "https://app.example.com",
    trustedHostsCsv: "checkout.stripe.com, other.example.com",
  };

  test("allows same-site relative paths", () => {
    expect(isSafeRedirect("/", trusted)).toBe(true);
    expect(isSafeRedirect("/dashboard", trusted)).toBe(true);
    expect(isSafeRedirect("/a/b?x=1#h", trusted)).toBe(true);
    // %2F in a path stays a path segment (browsers don't change origin on it).
    expect(isSafeRedirect("/%2F%2Fevil.com", trusted)).toBe(true);
  });

  test("rejects the classic open-redirect vectors", () => {
    expect(isSafeRedirect("//evil.com", trusted)).toBe(false); // protocol-relative
    expect(isSafeRedirect("/\\evil.com", trusted)).toBe(false); // backslash trick
    expect(isSafeRedirect("\\/evil.com", trusted)).toBe(false);
    expect(isSafeRedirect("https://evil.com", trusted)).toBe(false); // other origin
    expect(isSafeRedirect("https://evil.com/path", trusted)).toBe(false);
    expect(isSafeRedirect("javascript:alert(1)", trusted)).toBe(false);
    expect(isSafeRedirect("data:text/html,x", trusted)).toBe(false);
    expect(isSafeRedirect("dashboard", trusted)).toBe(false); // bare-relative → reject
  });

  test("allows absolute URLs to a trusted host (public origin / PYLON_TRUSTED_HOSTS / loopback)", () => {
    expect(isSafeRedirect("https://app.example.com/next", trusted)).toBe(true);
    expect(isSafeRedirect("https://checkout.stripe.com/pay/abc", trusted)).toBe(true);
    expect(isSafeRedirect("http://localhost:3000/x", trusted)).toBe(true);
    expect(isSafeRedirect("http://127.0.0.1/x", trusted)).toBe(true);
  });

  test("with no trusted config, only relative paths + loopback are allowed", () => {
    expect(isSafeRedirect("/ok", {})).toBe(true);
    expect(isSafeRedirect("http://localhost/x", {})).toBe(true);
    expect(isSafeRedirect("https://app.example.com/x", {})).toBe(false);
  });
});

describe("script-escaping + secure cookies", () => {
  test("escapeScriptJson neutralizes a </script> breakout", () => {
    const out = escapeScriptJson(JSON.stringify("</script><script>alert(1)</script>"));
    expect(out).not.toContain("<");
    expect(out).toContain("\\u003c");
  });

  test("buildHydrationTail fallback warn script escapes manifestErr", () => {
    const tail = buildHydrationTail({
      component: "app/x/page",
      layouts: [],
      props: {},
      ssrData: {},
      manifestRoute: null,
      publicPrefix: "/_pylon/build/",
      manifestErr: 'no entry for "</script><script>evil</script>"',
    });
    const warn = tail.slice(tail.indexOf("console.warn"));
    // The executable fallback must escape the error — no raw injected tag.
    expect(warn).not.toContain("<script>evil");
    expect(warn).toContain("\\u003c");
  });

  test("serializeCookie: Secure defaults + SameSite=None forces Secure", () => {
    const mk = () => {
      const s = { status: 200, headers: {}, cookies: [] as string[] };
      return { s, r: makeResponseController(s) };
    };
    const prev = process.env.PYLON_DEV_MODE;
    try {
      // Prod (dev off): Secure defaults ON.
      delete process.env.PYLON_DEV_MODE;
      {
        const { s, r } = mk();
        r.setCookie("a", "1");
        expect(s.cookies[0]).toContain("; Secure");
        expect(s.cookies[0]).toContain("; HttpOnly");
      }
      // Explicit secure:false (non-None) wins even in prod.
      {
        const { s, r } = mk();
        r.setCookie("a", "1", { secure: false });
        expect(s.cookies[0]).not.toContain("; Secure");
      }
      // Dev: Secure defaults OFF so http://localhost keeps the cookie.
      process.env.PYLON_DEV_MODE = "1";
      {
        const { s, r } = mk();
        r.setCookie("a", "1");
        expect(s.cookies[0]).not.toContain("; Secure");
      }
      // SameSite=None forces Secure even in dev (browsers drop it otherwise).
      {
        const { s, r } = mk();
        r.setCookie("a", "1", { sameSite: "none" });
        expect(s.cookies[0]).toContain("; Secure");
        expect(s.cookies[0]).toContain("SameSite=None");
      }
    } finally {
      if (prev === undefined) delete process.env.PYLON_DEV_MODE;
      else process.env.PYLON_DEV_MODE = prev;
    }
  });
});
