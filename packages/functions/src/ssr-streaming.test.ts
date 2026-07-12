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
  asRouteControl,
  buildHydrationTail,
  computeBucketVerdict,
  computeCacheVerdict,
  computeRevalidateSecs,
  computeWantsStream,
  describeCacheVerdict,
  diffCommittedResponse,
  isDevMode,
  PylonRouteControl,
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
    sessionTouched: false,
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
    expect(computeCacheVerdict({ ...base, sessionTouched: true })).toBe(false); // read session.exists (anon cache)
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
                    sessionTouched: false,
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

describe("computeBucketVerdict (PPR Phase 0 — session-presence shared cache)", () => {
  const base = {
    bucketOptIn: true,
    revalidateSecs: 60 as number | null,
    forceDynamic: false,
    authTouched: false,
    dynamicTouched: false,
    cookieCount: 0,
    strictPolicies: false,
    wantsStream: false,
    status: 200,
  };

  test("a clean opted-in bucketed 200 is bucketable", () => {
    expect(computeBucketVerdict(base)).toBe(true);
  });

  test("sessionTouched is NOT a veto here (the whole point of a bucket)", () => {
    // computeBucketVerdict has no sessionTouched field — reading session.exists
    // is allowed. This test documents the contract by construction: the same
    // base (which a bucket render reaches with sessionTouched=true) buckets.
    expect(computeBucketVerdict(base)).toBe(true);
  });

  test("no opt-in → never buckets, even when otherwise clean", () => {
    expect(computeBucketVerdict({ ...base, bucketOptIn: false })).toBe(false);
  });

  test("every identity/safety veto flips it to non-bucketable (fail-closed)", () => {
    expect(computeBucketVerdict({ ...base, revalidateSecs: null })).toBe(false); // no TTL
    expect(computeBucketVerdict({ ...base, forceDynamic: true })).toBe(false);
    expect(computeBucketVerdict({ ...base, authTouched: true })).toBe(false); // read real auth
    expect(computeBucketVerdict({ ...base, dynamicTouched: true })).toBe(false); // headers/cookies
    expect(computeBucketVerdict({ ...base, cookieCount: 1 })).toBe(false); // set a cookie
    expect(computeBucketVerdict({ ...base, strictPolicies: true })).toBe(false); // auth-filtered reads
    expect(computeBucketVerdict({ ...base, wantsStream: true })).toBe(false); // streaming head
    expect(computeBucketVerdict({ ...base, status: 404 })).toBe(false);
    expect(computeBucketVerdict({ ...base, status: 307 })).toBe(false);
  });

  test("LOAD-BEARING: authTouched ALWAYS vetoes regardless of opt-in", () => {
    // The proof that a bucketable body is identity-free: any output that depends
    // on identity must read props.auth, which sets authTouched. So authTouched
    // ⟹ !bucketable over the full cross-product.
    const bools = [false, true];
    for (const bucketOptIn of bools)
      for (const dynamicTouched of bools)
        for (const strictPolicies of bools)
          for (const wantsStream of bools)
            for (const cookieCount of [0, 1])
              for (const status of [200, 404]) {
                const v = computeBucketVerdict({
                  bucketOptIn,
                  revalidateSecs: 60,
                  forceDynamic: false,
                  authTouched: true,
                  dynamicTouched,
                  cookieCount,
                  strictPolicies,
                  wantsStream,
                  status,
                });
                expect(v).toBe(false);
              }
  });
});

describe("describeCacheVerdict (dev HUD — why isn't my page cached?)", () => {
  const base = {
    bucketOptIn: false,
    cacheable: false,
    bucketable: false,
    revalidateSecs: null as number | null,
    authTouched: false,
    dynamicTouched: false,
    sessionTouched: false,
    cookieCount: 0,
    strictPolicies: false,
    wantsStream: false,
    forceDynamic: false,
    status: 200,
  };

  test("cacheable / bucketed verdicts carry the TTL", () => {
    expect(describeCacheVerdict({ ...base, cacheable: true, revalidateSecs: 60 })).toEqual({
      verdict: "cacheable",
      secs: 60,
      reason: "anonymous shared cache",
    });
    const b = describeCacheVerdict({
      ...base,
      bucketOptIn: true,
      bucketable: true,
      revalidateSecs: 30,
    });
    expect(b.verdict).toBe("bucketed");
    expect(b.secs).toBe(30);
  });

  test("dynamic verdict reports the SINGLE actionable reason", () => {
    const why = (over: any) =>
      describeCacheVerdict({ ...base, ...over }).reason;
    // Not opted in at all.
    expect(why({})).toContain("not opted in");
    // Opted in (revalidate set) but each veto wins in priority order.
    const opted = { revalidateSecs: 60 };
    expect(why({ ...opted, status: 404 })).toContain("status 404");
    expect(why({ ...opted, forceDynamic: true })).toContain("force-dynamic");
    expect(why({ ...opted, wantsStream: true })).toContain("streaming");
    expect(why({ ...opted, cookieCount: 1 })).toContain("set a cookie");
    expect(why({ ...opted, strictPolicies: true })).toContain("STRICT");
    expect(why({ ...opted, authTouched: true })).toContain("props.auth");
    expect(why({ ...opted, dynamicTouched: true })).toContain("headers/cookies");
    // Reading session on a non-bucket page → the actionable hint to opt into buckets.
    expect(why({ ...opted, sessionTouched: true })).toContain("auth-bucketed");
    // …but on a bucket-opted page that didn't bucket for another reason, no hint.
    expect(why({ bucketOptIn: true, revalidateSecs: 60, sessionTouched: true })).not.toContain(
      "auth-bucketed",
    );
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

// ---------------------------------------------------------------------------
// onError gating: a redirect()/notFound() is only "ignored" once the HTTP head
// is committed. The runtime tracks a `headCommitted` flag and only logs the
// scary "[ssr] … was ignored — head already sent" warning when it's true.
// Before that (every BUFFERED render, and a streaming render pre-commit) the
// control signal rejects the render and the outer catch turns it into the real
// 3xx/404 — so warning there would fire on every CORRECT synchronous-shell
// auth-guard redirect. These tests lock the React behavior that makes the gate
// correct, using the real PylonRouteControl + asRouteControl classifier.
// ---------------------------------------------------------------------------

function makeRedirect(url: string): PylonRouteControl {
  const e = new PylonRouteControl("redirect");
  e.url = url;
  e.redirectStatus = 307;
  return e;
}

describe("SSR onError gating (redirect only 'ignored' after head commit)", () => {
  test("BUFFERED shell redirect: render REJECTS (→ outer catch emits 307); no warning fires pre-commit", async () => {
    // The auth-guard shape: response.redirect() throws a PylonRouteControl in
    // the synchronous shell, before any await/<Suspense>.
    function GuardedPage(): any {
      throw makeRedirect("/login");
    }
    let headCommitted = false;
    const warnings: string[] = [];
    let rejectedWith: PylonRouteControl | null = null;
    try {
      const stream = await renderToReadableStream(
        React.createElement(GuardedPage, {}),
        {
          onError(err: unknown) {
            const ctrl = asRouteControl(err);
            // Mirror the runtime's gate exactly: only warn once head is out.
            if (ctrl && headCommitted) warnings.push(ctrl.kind);
          },
        },
      );
      // Buffered path: the runtime awaits allReady, THEN sends response_start
      // (commits the head). A shell throw never gets here.
      await (stream as any).allReady;
      headCommitted = true;
    } catch (err) {
      // This is the runtime's outer catch — where the real 307 is emitted.
      rejectedWith = asRouteControl(err);
    }
    expect(rejectedWith?.kind).toBe("redirect"); // honored, not lost
    expect(rejectedWith?.url).toBe("/login");
    expect(headCommitted).toBe(false); // head never committed → NOT ignored
    expect(warnings).toEqual([]); // ← the fix: no false warning on a correct redirect
  });

  test("STREAMING below-<Suspense> redirect: onError fires AFTER head commit → warns", async () => {
    const d = makeDeferred<string[]>();
    // Throws a redirect from BELOW the boundary, only once its data resolves —
    // i.e. after the shell (and head) have already been flushed.
    function LateRedirect({ p }: { p: Promise<string[]> }): any {
      use(p);
      throw makeRedirect("/login");
    }
    function StreamPage({ p }: { p: Promise<string[]> }) {
      return React.createElement(
        "div",
        null,
        React.createElement("h1", null, "Shell"),
        React.createElement(
          Suspense,
          { fallback: React.createElement("p", null, "L") },
          React.createElement(LateRedirect, { p }),
        ),
      );
    }
    let headCommitted = false;
    const warnings: string[] = [];
    const stream = await renderToReadableStream(
      React.createElement(StreamPage, { p: d.promise }),
      {
        onError(err: unknown) {
          const ctrl = asRouteControl(err);
          if (ctrl && headCommitted) warnings.push(ctrl.kind);
        },
      },
    );
    const reader = stream.getReader();
    await reader.read(); // first flush = shell + fallback → head is on the wire
    headCommitted = true; // runtime sends response_start at this point
    d.resolve(["x"]); // resolve → LateRedirect throws below the committed boundary
    try {
      for (;;) {
        const { done } = await reader.read();
        if (done) break;
      }
    } catch {
      /* the stream may error after the late throw; irrelevant here */
    }
    expect(warnings).toEqual(["redirect"]); // genuinely ignored → correctly warned
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

// ---------------------------------------------------------------------------
// PPR Phase 0 — bucketed hydration tail must be IDENTITY-FREE
// ---------------------------------------------------------------------------

// Pull the __PYLON_DATA__ payload back out of a tail. JSON.parse natively
// decodes the unicode escapes (< etc.) buildHydrationTail applies.
function parsePylonData(tail: string): any {
  const m = tail.match(
    /<script id="__PYLON_DATA__" type="application\/json">([\s\S]*?)<\/script>/,
  );
  if (!m) throw new Error("no __PYLON_DATA__ blob in tail");
  return JSON.parse(m[1]);
}

describe("bucketed hydration tail (PPR Phase 0 leak class)", () => {
  // Two DIFFERENT signed-in identities. A non-bucketed tail serializes each
  // verbatim; a bucketed tail MUST collapse both to `{ signedIn: true }`.
  const alice = {
    user_id: "user_alice",
    tenant_id: "tenant_1",
    roles: ["admin"],
    email: "alice@x.com",
  };
  const bob = {
    user_id: "user_bob",
    tenant_id: "tenant_2",
    roles: ["member"],
    email: "bob@y.com",
  };
  const baseArgs = (auth: any) => ({
    component: "app/page",
    layouts: [],
    props: {
      url: "/",
      params: {},
      searchParams: {},
      auth,
      session: { exists: true },
    },
    ssrData: {},
    manifestRoute: { file: "index.js", imports: [], css: [] },
    publicPrefix: "/_pylon/build/",
    manifestErr: null,
  });

  test("non-bucketed tail keeps real auth (unchanged legacy behavior)", () => {
    const tail = buildHydrationTail(baseArgs(alice));
    expect(parsePylonData(tail).props.auth).toEqual(alice);
  });

  test("bucketed signed-in tail carries ONLY { signedIn: true }", () => {
    const tail = buildHydrationTail({
      ...baseArgs(alice),
      bucketAuth: { signedIn: true },
    });
    const data = parsePylonData(tail);
    expect(data.props.auth).toEqual({ signedIn: true });
    expect(data.props.session).toEqual({ exists: true });
    // No identity escapes — not in props, not anywhere in the rendered tail.
    expect(tail).not.toContain("user_alice");
    expect(tail).not.toContain("tenant_1");
    expect(tail).not.toContain("alice@x.com");
    expect(tail).not.toContain("admin");
  });

  test("bucketed signed-out tail carries ONLY { signedIn: false }", () => {
    const tail = buildHydrationTail({
      ...baseArgs(null),
      props: {
        url: "/",
        params: {},
        searchParams: {},
        auth: null,
        session: { exists: false },
      },
      bucketAuth: { signedIn: false },
    });
    const data = parsePylonData(tail);
    expect(data.props.auth).toEqual({ signedIn: false });
    expect(data.props.session).toEqual({ exists: false });
  });

  test("LEAK INVARIANT: two identities → byte-identical bucketed tail", () => {
    // The load-bearing property. If these two tails differed by a single byte,
    // a shared cache entry would replay alice's identity to bob (or vice-versa).
    const aTail = buildHydrationTail({
      ...baseArgs(alice),
      bucketAuth: { signedIn: true },
    });
    const bTail = buildHydrationTail({
      ...baseArgs(bob),
      bucketAuth: { signedIn: true },
    });
    expect(aTail).toBe(bTail);
  });

  test("bucketed tail DROPS page-added props (alias-identity fence, codex #4)", () => {
    // A page can alias identity onto a custom key without tripping read-tracking
    // (`props.leak = props.auth` — props is a plain object, no proxy trap). A
    // bucketed (SHARED) tail must therefore serialize a strict ALLOWLIST of
    // framework props, never a spread — else `leak` would carry one user's
    // identity into every signed-in visitor's cached page.
    const tail = buildHydrationTail({
      ...baseArgs(alice),
      props: {
        url: "/",
        params: { id: "x" },
        searchParams: {},
        auth: alice,
        session: { exists: true },
        // Page-aliased identity + a copied cookie bag.
        leak: { uid: alice.user_id },
        sneaky: alice.email,
      },
      bucketAuth: { signedIn: true },
    });
    const data = parsePylonData(tail);
    expect(data.props.auth).toEqual({ signedIn: true });
    // Allowlisted framework fields survive…
    expect(data.props.url).toBe("/");
    expect(data.props.params).toEqual({ id: "x" });
    // …but the page-added keys are GONE, and no identity is anywhere in the tail.
    expect(data.props.leak).toBeUndefined();
    expect(data.props.sneaky).toBeUndefined();
    expect(tail).not.toContain("user_alice");
    expect(tail).not.toContain("alice@x.com");
  });

  test("NON-bucket tail still preserves page-added props (no behavior change)", () => {
    // The allowlist applies ONLY to bucketed renders. A normal (non-shared)
    // render keeps serializing all props so hydration matches the server tree.
    const tail = buildHydrationTail({
      ...baseArgs(alice),
      props: { url: "/", params: {}, searchParams: {}, auth: alice, custom: 42 },
    });
    const data = parsePylonData(tail);
    expect(data.props.custom).toBe(42);
    expect(data.props.auth).toEqual(alice);
  });
});
