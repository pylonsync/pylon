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
function harness(opts: { ttlMs?: number; max?: number; maxInFlight?: number } = {}) {
  let clock = 1000;
  const calls: string[] = [];
  let resolveNext: ((v: string | null) => void) | null = null;
  const cache = createNavPayloadCache({
    now: () => clock,
    ttlMs: opts.ttlMs,
    max: opts.max,
    maxInFlight: opts.maxInFlight,
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
    const h = harness({ max: 2, maxInFlight: 3 });
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

  // The incident behind the in-flight cap: every card in a list was a link,
  // a scroll through it warmed 26 pages in a minute, and the renders queued
  // behind each other on the server for 25 to 38 seconds each.
  describe("in-flight cap", () => {
    /** A cache whose fetches answer only when the test settles them. */
    function slowHarness(maxInFlight?: number) {
      const calls: string[] = [];
      const settle = new Map<string, (v: string | null) => void>();
      const cache = createNavPayloadCache({
        now: () => 0,
        maxInFlight,
        fetchPage: (target) => {
          calls.push(target);
          return new Promise<string | null>((res) => settle.set(target, res));
        },
      });
      return { cache, calls, settle: (t: string) => settle.get(t)!(`<html>${t}</html>`) };
    }

    it("sends at most two prefetches at a time by default", () => {
      const h = slowHarness();
      for (let i = 0; i < 26; i++) h.cache.prefetch(`/property/${i}`);
      expect(h.calls).toEqual(["/property/0", "/property/1"]);
    });

    it("skips a prefetch over the cap, and the click fetches the page itself", () => {
      const h = slowHarness(1);
      h.cache.prefetch("/a");
      h.cache.prefetch("/b");
      expect(h.calls).toEqual(["/a"]);
      // null tells navigate() to open its own request.
      expect(h.cache.take("/b")).toBeNull();
    });

    it("sends again once a prefetch has answered", async () => {
      const h = slowHarness(1);
      h.cache.prefetch("/a");
      h.settle("/a");
      await h.cache.take("/a");
      h.cache.prefetch("/b");
      expect(h.calls).toEqual(["/a", "/b"]);
    });

    it("counts a request the server is still rendering after its entry is gone", () => {
      const h = slowHarness(1);
      h.cache.prefetch("/a");
      // Taken by a click, then the navigation commits: no entry is left, but
      // the render is still running.
      h.cache.take("/a");
      h.cache.clear();
      h.cache.prefetch("/b");
      expect(h.calls).toEqual(["/a"]);
    });

    it("frees the slot when the fetch fails", async () => {
      let fail = true;
      const calls: string[] = [];
      const cache = createNavPayloadCache({
        now: () => 0,
        maxInFlight: 1,
        fetchPage: (t) => {
          calls.push(t);
          return fail ? Promise.reject(new Error("offline")) : Promise.resolve("ok");
        },
      });
      cache.prefetch("/a");
      await cache.take("/a");
      fail = false;
      cache.prefetch("/b");
      expect(calls).toEqual(["/a", "/b"]);
    });
  });
});
