// Instant client navigation, run in the real client runtime: the fixture
// (ssr-client-nav-runtime.fixture.ts) hydrates a page in happy-dom and taps
// through it, in its own process. This checks what it saw:
//
//   - /fast (the page exports navigation = "client") and /offline (an offline
//     route) render with no page fetch;
//   - /slow fetches its page, as before;
//   - back and forward add no history entries and fetch only /slow;
//   - every navigation resets the scroll, like a fetched one;
//   - after a deploy, a click moves the page to the new build once instead
//     of a full page load of the old build per click, and an offline page
//     keeps its screen;
//   - a build manifest of another build (an older service worker's cached
//     copy) is not used.

import { beforeAll, describe, expect, test } from "bun:test";
import path from "node:path";

let report: Record<string, any> = {};

async function runFixture(mode?: string): Promise<Record<string, any>> {
  const args = [process.execPath, path.join(import.meta.dir, "ssr-client-nav-runtime.fixture.ts")];
  if (mode) args.push(mode);
  const proc = Bun.spawn(args, {
    cwd: path.join(import.meta.dir, ".."),
    stdout: "pipe",
    stderr: "pipe",
  });
  const [out, err] = await Promise.all([new Response(proc.stdout).text(), new Response(proc.stderr).text()]);
  await proc.exited;
  const line = out.split("\n").find((l) => l.startsWith("REPORT "));
  if (!line) throw new Error(`fixture printed no report:\n${out}\n${err}`);
  return JSON.parse(line.slice("REPORT ".length));
}

beforeAll(async () => {
  const [main, stale] = await Promise.all([runFixture(), runFixture("stale-manifest")]);
  report = { ...main, staleManifest: stale.staleManifest };
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

describe("a deploy while the page is open", () => {
  const early = (url: string) => ({ url, skipIfPendingWrites: true });
  const recheck = (url: string) => ({ url, recheck: true });

  test("before the deploy, an entry that fails to load still gets one full page load", () => {
    const { sameBuildFailure } = report.deploy;
    expect(sameBuildFailure.moves).toEqual([early("/reports"), recheck("/reports")]);
    expect(sameBuildFailure.fullLoads).toEqual(["/reports"]);
  });

  test("a page route served by the new build moves the page once, with no full load", () => {
    const { pageRoute } = report.deploy;
    expect(pageRoute.pageFetches).toEqual(["/reports"]);
    expect(pageRoute.moves).toEqual([early("/reports"), recheck("/reports")]);
    expect(pageRoute.fullLoads).toEqual([]);
  });

  test("a client route whose entry the deploy removed moves the page, with no page fetch", () => {
    const { clientRoute } = report.deploy;
    expect(clientRoute.moves).toEqual([early("/quick"), recheck("/quick")]);
    expect(clientRoute.pageFetches).toEqual([]);
    expect(clientRoute.fullLoads).toEqual([]);
  });

  test("when the last check saw the new build, the click moves at once", () => {
    const { knownAhead } = report.deploy;
    expect(knownAhead.moves).toEqual([early("/reports")]);
    expect(knownAhead.pageFetches).toEqual([]);
    expect(knownAhead.fullLoads).toEqual([]);
  });

  test("offline, a route that cannot load keeps the current screen", () => {
    const { offline } = report.deploy;
    expect(offline.fullLoads).toEqual([]);
    expect(offline.heading).toBe("slow /slow");
  });

  test("a page that already moved and is still on the old build gets one full load, not a loop", () => {
    const { blocked } = report.deploy;
    expect(blocked.moves).toEqual([early("/reports"), recheck("/reports")]);
    expect(blocked.fullLoads).toEqual(["/reports"]);
  });

  test("another build's manifest is not used: no route files of that build load", () => {
    const stale = report.staleManifest;
    expect(stale.warnedAboutManifest).toBe(true);
    expect(stale.preloads.filter((h: string) => h.includes("-v0"))).toEqual([]);
    expect(stale.moves).toEqual([early("/reports"), recheck("/reports")]);
    expect(stale.fullLoads).toEqual([]);
  });
});
