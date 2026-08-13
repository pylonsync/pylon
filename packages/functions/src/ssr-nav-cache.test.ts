// Regression tests for the client-navigation payload cache.
//
// The bug this exists to fix: `<Link>` prefetched pages with a
// `<link rel="prefetch">` that could never be reused. Measured on a real app,
// the same URL was fetched twice — 16742 bytes both times, full transferSize
// on the second — because SSR pages are sent "private, no-store" and an
// as="document" prefetch only feeds real navigations, not fetch(). So each
// prefetch was a full server render, thrown away, and the click still paid.

import { describe, expect, it } from "bun:test";
import { createNavPayloadCache } from "./ssr-nav-cache";

/** A cache with a controllable clock and a fetch that records its calls. */
function harness(opts: { ttlMs?: number; max?: number } = {}) {
  let clock = 1000;
  const calls: string[] = [];
  let resolveNext: ((v: string | null) => void) | null = null;
  const cache = createNavPayloadCache({
    now: () => clock,
    ttlMs: opts.ttlMs,
    max: opts.max,
    fetchPage: (target) => {
      calls.push(target);
      return new Promise<string | null>((res) => {
        resolveNext = res;
        // Default: resolve immediately on the microtask queue.
        queueMicrotask(() => res(`<html>${target}</html>`));
      });
    },
  });
  return {
    cache,
    calls,
    advance: (ms: number) => {
      clock += ms;
    },
    settleWith: (v: string | null) => resolveNext?.(v),
  };
}

describe("nav payload cache", () => {
  it("hands the prefetched payload to the click", async () => {
    const h = harness();
    h.cache.prefetch("/a");
    const taken = h.cache.take("/a");
    expect(taken).not.toBeNull();
    expect(await taken!).toBe("<html>/a</html>");
    // The whole point: the click opened no request of its own.
    expect(h.calls).toEqual(["/a"]);
  });

  it("does not re-fetch a target already in flight", () => {
    const h = harness();
    h.cache.prefetch("/a");
    h.cache.prefetch("/a");
    h.cache.prefetch("/a");
    expect(h.calls).toEqual(["/a"]);
  });

  it("lets a click join a prefetch that hasn't landed yet", async () => {
    // Hover-to-click is usually shorter than the render, so this is the
    // common path, not an edge case.
    const h = harness();
    h.cache.prefetch("/slow");
    const taken = h.cache.take("/slow");
    h.settleWith("<html>late</html>");
    expect(await taken!).toBe("<html>late</html>");
    expect(h.calls).toEqual(["/slow"]);
  });

  it("is single-use, so a second navigation re-renders", () => {
    const h = harness();
    h.cache.prefetch("/a");
    expect(h.cache.take("/a")).not.toBeNull();
    expect(h.cache.take("/a")).toBeNull();
  });

  it("refuses a payload past its TTL", () => {
    const h = harness({ ttlMs: 15000 });
    h.cache.prefetch("/a");
    h.advance(15000);
    expect(h.cache.take("/a")).toBeNull();
  });

  it("still serves a payload just inside its TTL", async () => {
    const h = harness({ ttlMs: 15000 });
    h.cache.prefetch("/a");
    h.advance(14999);
    expect(await h.cache.take("/a")!).toBe("<html>/a</html>");
  });

  it("re-warms an expired target instead of holding the stale one", async () => {
    const h = harness({ ttlMs: 100 });
    h.cache.prefetch("/a");
    h.advance(200);
    h.cache.prefetch("/a");
    expect(h.calls).toEqual(["/a", "/a"]);
    expect(await h.cache.take("/a")!).toBe("<html>/a</html>");
  });

  it("evicts the oldest entry at the cap", () => {
    const h = harness({ max: 2 });
    h.cache.prefetch("/a");
    h.cache.prefetch("/b");
    h.cache.prefetch("/c");
    expect(h.cache.size()).toBe(2);
    expect(h.cache.take("/a")).toBeNull();
    expect(h.cache.take("/c")).not.toBeNull();
  });

  it("drops everything when a navigation commits", () => {
    // Entries were rendered against the page the user just left.
    const h = harness();
    h.cache.prefetch("/a");
    h.cache.prefetch("/b");
    h.cache.clear();
    expect(h.cache.size()).toBe(0);
    expect(h.cache.take("/a")).toBeNull();
  });

  it("yields null when the fetch fails, so the click refetches", async () => {
    const calls: string[] = [];
    const cache = createNavPayloadCache({
      now: () => 0,
      fetchPage: (t) => {
        calls.push(t);
        return Promise.reject(new Error("offline"));
      },
    });
    cache.prefetch("/a");
    // A rejection must not escape as an unhandled rejection when nobody clicks.
    expect(await cache.take("/a")!).toBeNull();
    expect(calls).toEqual(["/a"]);
  });

  it("caches nothing for a response the fetch rejected as unusable", async () => {
    // fetchPage returns null for a non-200 or a redirect — the URL that
    // answered isn't the one navigate() would commit.
    const cache = createNavPayloadCache({
      now: () => 0,
      fetchPage: () => Promise.resolve(null),
    });
    cache.prefetch("/a");
    expect(await cache.take("/a")!).toBeNull();
  });
});
