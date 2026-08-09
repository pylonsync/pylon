/**
 * `serverData.fn(name, args)` — SSR pages calling query functions with the
 * page's auth context. Pins the pieces that must not drift:
 *
 *  - the cache key shape ("fn:" + name + ":" + stableStringify(args ?? {})),
 *    which the client hydration shim replays from ssrData — a mismatch means
 *    hydration re-suspends forever;
 *  - promise caching per (name, args) so React 19 `use()` gets the SAME
 *    promise on the post-suspense re-render;
 *  - resolved values recorded into the valueCache (the ssrData blob);
 *  - the onFnCall notification that lets the render veto shared caching for
 *    identity-carrying requests;
 *  - the client runtime template carrying the matching `fn` shim.
 */
import { describe, expect, test } from "bun:test";
import { makeServerData } from "./ssr-runtime";

const noopReader = {};

describe("makeServerData.fn", () => {
  test("caches per (name, args) and records into valueCache with the wire key", async () => {
    const valueCache: Record<string, any> = {};
    const calls: Array<[string, any]> = [];
    const sd = makeServerData(noopReader, valueCache, async (name, args) => {
      calls.push([name, args]);
      return { rows: [name, args?.x ?? null] };
    });

    const p1 = sd.fn("getPublicSchedule", { x: 1 });
    const p2 = sd.fn("getPublicSchedule", { x: 1 });
    expect(p1).toBe(p2); // use() re-render must see the SAME promise

    const v = await p1;
    expect(v).toEqual({ rows: ["getPublicSchedule", 1] });
    expect(calls.length).toBe(1); // deduped

    // Distinct args → distinct call, distinct cache entry.
    await sd.fn("getPublicSchedule", { x: 2 });
    expect(calls.length).toBe(2);

    // No args normalizes to {} in the key.
    await sd.fn("getSpeakers");
    expect(Object.keys(valueCache).sort()).toEqual([
      'fn:getPublicSchedule:{"x":1}',
      'fn:getPublicSchedule:{"x":2}',
      "fn:getSpeakers:{}",
    ]);
  });

  test("onFnCall fires once per unique call (the cache-veto hook)", async () => {
    let touches = 0;
    const sd = makeServerData(
      noopReader,
      {},
      async () => null,
      () => {
        touches++;
      },
    );
    sd.fn("a");
    sd.fn("a"); // cached — no second touch
    sd.fn("b");
    expect(touches).toBe(2);
  });

  test("no callFn → no fn method (older host pairing)", () => {
    const sd = makeServerData(noopReader, {});
    expect(sd.fn).toBeUndefined();
  });
});

describe("client runtime shim parity", () => {
  test("CLIENT_RUNTIME_SOURCE replays fn results with the identical key", async () => {
    const src = await Bun.file(
      new URL("./ssr-client-bundler.ts", import.meta.url).pathname,
    ).text();
    // Both client stand-ins (hydration replay + optimistic-nav pending) must
    // build the exact server-side key. String-level guard against drift.
    const keyExpr = '"fn:" + name + ":" + stableStringify(args ?? {})';
    const count = src.split(keyExpr).length - 1;
    expect(count).toBeGreaterThanOrEqual(2);
  });
});
