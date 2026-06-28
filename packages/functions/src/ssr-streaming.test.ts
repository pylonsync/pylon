/**
 * #278 Stage 2 — progressive streaming harness.
 *
 * Two layers:
 *  1. PURE verdict logic (computeWantsStream / computeRevalidateSecs /
 *     computeCacheVerdict / diffCommittedResponse) — the security-critical
 *     "is this render cacheable / should it stream" gate, tested directly
 *     including the leak-class invariant `cacheable ⟹ !wantsStream`.
 *  2. The actual React streaming MECHANISM through react-dom/server.browser —
 *     proving the buffered (allReady) path emits clean inline HTML, and the
 *     streamed path flushes the shell + Suspense fallback first then reveals.
 *
 * Hydration (the prod-killer "stuck on fallback" check) lives in the sibling
 * ssr-hydration.test.ts (it mutates DOM globals, so it's isolated).
 */
import { describe, expect, test } from "bun:test";
import React, { Suspense, use } from "react";
import { renderToReadableStream } from "react-dom/server.browser";
import {
  buildHydrationTail,
  computeCacheVerdict,
  computeRevalidateSecs,
  computeWantsStream,
  diffCommittedResponse,
  isDevMode,
} from "./ssr-runtime";

// ---------------------------------------------------------------------------
// isDevMode — must match the Rust host's is_dev_mode() exactly
// ---------------------------------------------------------------------------

describe("isDevMode (PYLON_DEV_MODE parity with Rust host)", () => {
  const orig = process.env.PYLON_DEV_MODE;
  const set = (v: string | undefined) => {
    if (v === undefined) delete process.env.PYLON_DEV_MODE;
    else process.env.PYLON_DEV_MODE = v;
  };

  test("ONLY '1' / 'true' (case-insensitive) are dev; 'false'/'0'/unset are NOT", () => {
    try {
      set("1");
      expect(isDevMode()).toBe(true);
      set("true");
      expect(isDevMode()).toBe(true);
      set("TRUE");
      expect(isDevMode()).toBe(true);
      // The bug this guards: a prod machine explicitly set PYLON_DEV_MODE=false,
      // and `if (process.env.PYLON_DEV_MODE)` treated the truthy STRING "false"
      // as dev → injected the live-reload script → EventSource 404-looped on
      // /_pylon/dev/live in prod.
      set("false");
      expect(isDevMode()).toBe(false);
      set("0");
      expect(isDevMode()).toBe(false);
      set("");
      expect(isDevMode()).toBe(false);
      set(undefined);
      expect(isDevMode()).toBe(false);
    } finally {
      set(orig);
    }
  });
});

// ---------------------------------------------------------------------------
// Pure verdict logic
// ---------------------------------------------------------------------------

describe("computeWantsStream", () => {
  test("loading.tsx OR streaming=true opts in; nothing else does", () => {
    expect(computeWantsStream(true, {})).toBe(true); // loading.tsx
    expect(computeWantsStream(false, { streaming: true })).toBe(true); // opt-in
    expect(computeWantsStream(true, { streaming: true })).toBe(true);
    expect(computeWantsStream(false, {})).toBe(false); // default = buffered
    expect(computeWantsStream(false, { streaming: false })).toBe(false);
    expect(computeWantsStream(false, { streaming: 1 as any })).toBe(false); // strict ===
    expect(computeWantsStream(false, null)).toBe(false);
  });
});

describe("computeRevalidateSecs", () => {
  test("revalidate>0 → floor; force-static → a year; else null", () => {
    expect(computeRevalidateSecs({ revalidate: 60 })).toBe(60);
    expect(computeRevalidateSecs({ revalidate: 12.9 })).toBe(12);
    expect(computeRevalidateSecs({ revalidate: 0 })).toBeNull();
    expect(computeRevalidateSecs({ revalidate: -5 })).toBeNull();
    expect(computeRevalidateSecs({ dynamic: "force-static" })).toBe(31536000);
    expect(computeRevalidateSecs({ dynamic: "force-dynamic" })).toBeNull();
    expect(computeRevalidateSecs({})).toBeNull();
    expect(computeRevalidateSecs(null)).toBeNull();
  });
});

describe("computeCacheVerdict (the #277 leak-class gate)", () => {
  const base = {
    revalidateSecs: 60 as number | null,
    forceDynamic: false,
    authTouched: false,
    dynamicTouched: false,
    cookieCount: 0,
    strictPolicies: false,
    wantsStream: false,
    status: 200,
  };

  test("a clean opted-in buffered 200 is cacheable", () => {
    expect(computeCacheVerdict(base)).toBe(true);
  });

  test("every single veto flips it to non-cacheable (fail-closed)", () => {
    expect(computeCacheVerdict({ ...base, revalidateSecs: null })).toBe(false); // no opt-in
    expect(computeCacheVerdict({ ...base, forceDynamic: true })).toBe(false);
    expect(computeCacheVerdict({ ...base, authTouched: true })).toBe(false); // read auth
    expect(computeCacheVerdict({ ...base, dynamicTouched: true })).toBe(false); // read headers/cookies
    expect(computeCacheVerdict({ ...base, cookieCount: 1 })).toBe(false); // set a cookie
    expect(computeCacheVerdict({ ...base, strictPolicies: true })).toBe(false);
    expect(computeCacheVerdict({ ...base, wantsStream: true })).toBe(false); // STREAMING
    expect(computeCacheVerdict({ ...base, status: 404 })).toBe(false);
    expect(computeCacheVerdict({ ...base, status: 307 })).toBe(false);
  });

  test("INVARIANT: cacheable ⟹ !wantsStream over the full cross-product", () => {
    const bools = [false, true];
    const statuses = [200, 201, 302, 404, 500];
    const revs: (number | null)[] = [null, 0 as any, 60, 31536000];
    let checked = 0;
    for (const forceDynamic of bools)
      for (const authTouched of bools)
        for (const strictPolicies of bools)
          for (const wantsStream of bools)
            for (const cookieCount of [0, 1])
              for (const status of statuses)
                for (const revalidateSecs of revs) {
                  const c = computeCacheVerdict({
                    revalidateSecs,
                    forceDynamic,
                    authTouched,
                    dynamicTouched: false,
                    cookieCount,
                    strictPolicies,
                    wantsStream,
                    status,
                  });
                  // The load-bearing security invariant: a streaming render is
                  // NEVER cacheable (its head commits before auth/cookies/status
                  // are final).
                  if (c) expect(wantsStream).toBe(false);
                  checked++;
                }
    expect(checked).toBe(640); // 2^4 (bools) × 2 (cookies) × 5 (status) × 4 (revs)
  });
});

describe("diffCommittedResponse (#278 late-response.* drop detector)", () => {
  const snap = (over: any = {}) => ({
    status: 200,
    cookies: ["sid=committed"],
    headerKeys: ["x-base"],
    ...over,
  });

  test("no change → null", () => {
    const r = diffCommittedResponse(snap(), {
      status: 200,
      cookies: ["sid=committed"],
      headers: { "x-base": "1" },
    });
    expect(r).toBeNull();
  });

  test("a late Set-Cookie from a suspended subtree is reported", () => {
    const r = diffCommittedResponse(snap(), {
      status: 200,
      cookies: ["sid=committed", "flash=late"],
      headers: { "x-base": "1" },
    });
    expect(r).not.toBeNull();
    expect(r!.droppedCookies).toEqual(["flash=late"]);
  });

  test("a late status change + new header are reported", () => {
    const r = diffCommittedResponse(snap(), {
      status: 201,
      cookies: ["sid=committed"],
      headers: { "x-base": "1", "x-late": "2" },
    });
    expect(r!.statusChanged).toBe(true);
    expect(r!.newHeaderKeys).toEqual(["x-late"]);
  });
});

// ---------------------------------------------------------------------------
// React streaming mechanism (real renderToReadableStream)
// ---------------------------------------------------------------------------

function makeDeferred<T>() {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => (resolve = r));
  return { promise, resolve };
}

function Rows({ p }: { p: Promise<string[]> }) {
  const rows = use(p);
  return React.createElement(
    "ul",
    { id: "rows" },
    rows.map((r) => React.createElement("li", { key: r }, r)),
  );
}

// A page with an inner <Suspense> reading async data — the /notes shape.
function Page({ p }: { p: Promise<string[]> }) {
  return React.createElement(
    "div",
    { id: "app" },
    React.createElement("h1", { id: "shell" }, "Shell"),
    React.createElement(
      Suspense,
      { fallback: React.createElement("p", { id: "fallback" }, "Loading…") },
      React.createElement(Rows, { p }),
    ),
  );
}

const dec = new TextDecoder();

describe("react streaming mechanism", () => {
  test("BUFFERED (await allReady, then drain) → clean inline, no reveal scripts", async () => {
    // Mirrors the runtime's `if (!wantsStream) await stream.allReady` path.
    const d = makeDeferred<string[]>();
    setTimeout(() => d.resolve(["a", "b"]), 5);
    const stream = await renderToReadableStream(
      React.createElement(Page, { p: d.promise }),
    );
    await (stream as any).allReady;
    const reader = stream.getReader();
    let html = "";
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      html += dec.decode(value);
    }
    // Resolved rows inline; NO fallback, NO pending marker, NO $RC reveal /
    // <template>. This is the byte-identical buffered contract today's prod
    // traffic rides — the regression lock.
    expect(html).toContain("<li>a</li>");
    expect(html).toContain("<li>b</li>");
    expect(html).not.toContain("Loading…");
    expect(html).not.toContain("<!--$?-->"); // no PENDING boundary marker
    expect(html).not.toContain("<template");
    expect(/\$RC|completeBoundary/.test(html)).toBe(false);
  });

  test("STREAMING (drain progressively) → shell+fallback first, rows+reveal later", async () => {
    const d = makeDeferred<string[]>();
    const stream = await renderToReadableStream(
      React.createElement(Page, { p: d.promise }),
    );
    const reader = stream.getReader();
    // First flush = the shell with the fallback (data still pending).
    const first = await reader.read();
    const firstHtml = dec.decode(first.value);
    expect(firstHtml).toContain("Shell");
    expect(firstHtml).toContain("Loading…"); // fallback streamed
    expect(firstHtml).not.toContain("<li>a</li>"); // rows NOT here yet
    expect(firstHtml).toContain("<template"); // boundary placeholder
    // Now the data resolves → React reveals the boundary in a later chunk.
    d.resolve(["a", "b"]);
    let rest = "";
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      rest += dec.decode(value);
    }
    expect(rest).toContain("<li>a</li>");
    expect(rest).toContain("<li>b</li>");
    expect(/\$RC|completeBoundary|<template/.test(rest)).toBe(true); // reveal
  });
});

describe("hydration tail ordering (#278: data blob before entry script)", () => {
  test("__PYLON_DATA__ carries ssrData and the entry <script type=module> is LAST", () => {
    const ssrData = { 'list:["Note"]': [{ id: "1", body: "hi" }] };
    const tail = buildHydrationTail({
      component: "app/notes/page",
      layouts: [],
      props: { url: "/notes", params: {}, searchParams: {} },
      ssrData,
      manifestRoute: { file: "notes.js", imports: ["chunk.js"], css: [] },
      publicPrefix: "/_pylon/build/",
      manifestErr: null,
    });
    const dataIdx = tail.indexOf('id="__PYLON_DATA__"');
    const entryIdx = tail.indexOf("notes.js");
    expect(dataIdx).toBeGreaterThanOrEqual(0);
    expect(entryIdx).toBeGreaterThanOrEqual(0);
    // The entry script MUST come after the data blob so hydrateRoot (which the
    // entry triggers) sees a fully-seeded ssrData — the whole reason multi-
    // boundary streaming hydrates cleanly without inline patch scripts.
    expect(entryIdx).toBeGreaterThan(dataIdx);
    // ssrData round-trips into the blob.
    expect(tail).toContain("Note");
    expect(tail).toContain('"body":"hi"');
  });
});
