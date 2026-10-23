// Instant client navigation, run in the real client runtime: the fixture
// (ssr-client-nav-runtime.fixture.ts) hydrates a page in happy-dom and taps
// through it, in its own process. This checks what it saw:
//
//   - /fast (the page exports navigation = "client") and /offline (an offline
//     route) render with no page fetch;
//   - /slow fetches its page, as before;
//   - back and forward add no history entries and fetch only /slow;
//   - every navigation resets the scroll, like a fetched one.

import { beforeAll, describe, expect, test } from "bun:test";
import path from "node:path";

let report: Record<string, any> = {};

beforeAll(async () => {
  const proc = Bun.spawn([process.execPath, path.join(import.meta.dir, "ssr-client-nav-runtime.fixture.ts")], {
    cwd: path.join(import.meta.dir, ".."),
    stdout: "pipe",
    stderr: "pipe",
  });
  const [out, err] = await Promise.all([new Response(proc.stdout).text(), new Response(proc.stderr).text()]);
  await proc.exited;
  const line = out.split("\n").find((l) => l.startsWith("REPORT "));
  if (!line) throw new Error(`fixture printed no report:\n${out}\n${err}`);
  report = JSON.parse(line.slice("REPORT ".length));
}, 60_000);

describe("instant client navigation", () => {
  test("the page hydrated", () => {
    expect(report.hydrated.heading).toBe("home /");
  });

  test('a click on a route with navigation = "client" makes no page fetch', () => {
    const { fast, hydrated } = report;
    expect(fast.heading).toBe("fast /fast");
    expect(fast.pathname).toBe("/fast");
    expect(fast.pageFetches).toEqual([]);
    expect(fast.historyLength).toBe(hydrated.historyLength + 1);
    expect(fast.scrolls).toEqual([0]);
    // The title comes from the page's static metadata.
    expect(fast.title).toBe("Fast");
  });

  test("a click on an offline route makes no page fetch", () => {
    expect(report.offline.heading).toBe("offline /offline");
    expect(report.offline.pageFetches).toEqual([]);
  });

  test("a route that did not opt in still fetches its page", () => {
    const { slow } = report;
    expect(slow.heading).toBe("slow /slow");
    expect(slow.pageFetches).toEqual(["/slow"]);
    expect(slow.pathname).toBe("/slow");
    expect(slow.scrolls).toEqual([0]);
    expect(slow.title).toBe("Slow");
  });

  test("back and forward re-render without new history entries, fetching only /slow", () => {
    const { slow, back1, back2, forward1, forward2 } = report;
    expect([back1.pathname, back1.heading]).toEqual(["/offline", "offline /offline"]);
    expect([back2.pathname, back2.heading]).toEqual(["/fast", "fast /fast"]);
    expect([forward1.pathname, forward1.heading]).toEqual(["/offline", "offline /offline"]);
    expect([forward2.pathname, forward2.heading]).toEqual(["/slow", "slow /slow"]);
    expect([back1, back2, forward1].flatMap((o) => o.pageFetches)).toEqual([]);
    expect(forward2.pageFetches).toEqual(["/slow"]);
    for (const o of [back1, back2, forward1, forward2]) {
      expect(o.historyLength).toBe(slow.historyLength);
    }
  });

  test("prefetch on sight waits for idle and skips Save-Data; intent fetches at once", () => {
    const { sightBeforeIdle, sightAfterIdle, saveDataSight, intent } = report.prefetch;
    expect(sightBeforeIdle).toBe(0);
    expect(sightAfterIdle).toBeGreaterThan(0);
    expect(saveDataSight).toBe(0);
    expect(intent).toBeGreaterThan(0);
  });
});
