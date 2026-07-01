import { describe, expect, it } from "bun:test";
import { matchRoute, type MatchableManifest } from "./ssr-route-match";

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
