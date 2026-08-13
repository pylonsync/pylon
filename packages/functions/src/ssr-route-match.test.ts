import { describe, expect, it } from "bun:test";
import {
  matchRoute,
  prefetchTargets,
  type MatchableManifest,
} from "./ssr-route-match";

const manifest: MatchableManifest = {
  routes: {
    "app/page": { path: "/" },
    "app/account/page": { path: "/account" },
    "app/checkout/page": { path: "/checkout" },
    "app/p/[slug]/page": { path: "/p/[slug]" },
    "app/orders/[id]/page": { path: "/orders/[id]" },
    "app/orders/new/page": { path: "/orders/new" },
    "app/docs/[...rest]/page": { path: "/docs/[...rest]" },
    // Boundary modules ship without a `path` and must never match.
    "app/p/[slug]/not-found": {},
  },
};

describe("matchRoute", () => {
  it("matches the index route", () => {
    expect(matchRoute(manifest, "/")).toEqual({
      component: "app/page",
      params: {},
    });
  });

  it("matches a static route", () => {
    expect(matchRoute(manifest, "/account")).toEqual({
      component: "app/account/page",
      params: {},
    });
  });

  it("captures a dynamic segment and decodes it", () => {
    expect(matchRoute(manifest, "/p/blue-runner-9f3a2b")).toEqual({
      component: "app/p/[slug]/page",
      params: { slug: "blue-runner-9f3a2b" },
    });
    expect(matchRoute(manifest, "/p/a%20b")).toEqual({
      component: "app/p/[slug]/page",
      params: { slug: "a b" },
    });
  });

  it("prefers a static route over a dynamic one at the same depth", () => {
    expect(matchRoute(manifest, "/orders/new")).toEqual({
      component: "app/orders/new/page",
      params: {},
    });
    expect(matchRoute(manifest, "/orders/o_123")).toEqual({
      component: "app/orders/[id]/page",
      params: { id: "o_123" },
    });
  });

  it("matches a catch-all across the remaining segments", () => {
    expect(matchRoute(manifest, "/docs/guide/getting-started")).toEqual({
      component: "app/docs/[...rest]/page",
      params: { rest: "guide/getting-started" },
    });
  });

  it("ignores trailing slashes", () => {
    expect(matchRoute(manifest, "/account/")).toEqual({
      component: "app/account/page",
      params: {},
    });
  });

  it("returns null when nothing matches", () => {
    expect(matchRoute(manifest, "/does/not/exist")).toBeNull();
  });

  it("never matches a boundary module (no path)", () => {
    // /p/anything resolves to the page, never the not-found boundary.
    const m = matchRoute(manifest, "/p/anything");
    expect(m?.component).toBe("app/p/[slug]/page");
  });

  it("is null-safe on an empty/absent manifest", () => {
    expect(matchRoute(null, "/")).toBeNull();
    expect(matchRoute({ routes: {} }, "/")).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// prefetchTargets — what <Link prefetch> warms.
//
// Regression: prefetch emitted the page payload and the shared chunks but
// never the destination's OWN entry chunk, so every click paid a full
// round-trip (126-281ms measured across a 12-link sidebar) before the route
// could render. The payload it did prefetch was the half that was already fast.
// ---------------------------------------------------------------------------

const chunked: MatchableManifest = {
  routes: {
    "app/page": {
      path: "/",
      file: "client-entry-app__page-a1.js",
      imports: ["chunks/shared-1.js"],
    },
    "app/speakers/page": {
      path: "/speakers",
      file: "client-entry-app__speakers__page-b2.js",
      imports: ["chunks/shared-1.js", "chunks/editor-2.js"],
    },
    "app/events/[id]/page": {
      path: "/events/[id]",
      file: "client-entry-app__events____id____page-c3.js",
      imports: ["chunks/shared-1.js"],
    },
    // A boundary module: no path, so it never matches a href.
    "app/not-found": { file: "client-entry-app__not_found-d4.js", imports: [] },
  },
};

describe("prefetchTargets", () => {
  it("names the destination's OWN entry chunk", () => {
    // The whole point: without `file`, the click still blocks on this fetch.
    expect(prefetchTargets(chunked, "/speakers").file).toBe(
      "client-entry-app__speakers__page-b2.js",
    );
  });

  it("includes chunks belonging to the destination alone", () => {
    // editor-2 is imported only by /speakers. It has to be in the warm set
    // whether it's reached as the destination's own import or via the
    // all-routes union.
    expect(prefetchTargets(chunked, "/speakers").imports).toContain(
      "chunks/editor-2.js",
    );
  });

  it("warms the shared chunks every route needs", () => {
    expect(prefetchTargets(chunked, "/").imports).toContain("chunks/shared-1.js");
  });

  it("resolves a dynamic route to its entry", () => {
    expect(prefetchTargets(chunked, "/events/42").file).toBe(
      "client-entry-app__events____id____page-c3.js",
    );
  });

  it("yields no entry for an href that matches no page route", () => {
    // e.g. an API path or a route this build doesn't serve. Shared chunks are
    // still worth warming; the caller skips an empty `file`.
    const t = prefetchTargets(chunked, "/api/webhooks/stripe");
    expect(t.file).toBe("");
    expect(t.imports).toContain("chunks/shared-1.js");
  });

  it("never resolves to a boundary module's entry", () => {
    expect(prefetchTargets(chunked, "/not-found").file).toBe("");
  });

  it("deduplicates chunks shared across routes", () => {
    const imports = prefetchTargets(chunked, "/speakers").imports;
    expect(imports.filter((i) => i === "chunks/shared-1.js")).toHaveLength(1);
  });

  it("is null-safe on an absent or empty manifest", () => {
    expect(prefetchTargets(null, "/")).toEqual({
      file: "",
      imports: [],

    });
    expect(prefetchTargets({ routes: {} }, "/")).toEqual({
      file: "",
      imports: [],

    });
  });

  it("tolerates a manifest whose routes carry no chunk fields", () => {
    // Older build output — must degrade, not throw.
    expect(prefetchTargets({ routes: { "app/page": { path: "/" } } }, "/")).toEqual(
      { file: "", imports: [] },
    );
  });
});
