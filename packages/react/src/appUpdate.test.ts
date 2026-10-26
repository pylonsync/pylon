// The app update state machine: it learns of a new build from
// `/_pylon/version` and from a waiting service worker, waits for pending
// writes before it reloads, reloads once, and never reloads in a loop.

import { expect, test } from "bun:test";
import {
  createAppUpdate,
  RELOAD_GUARD_MS,
  VERSION_URL,
  WORKER_WAIT_MS,
  type AppUpdateEnv,
  type UpdateRegistrationLike,
  type WorkerLike,
} from "./appUpdate";

const BUILD = { version: "v1", builtAt: "2026-10-22T10:00:00.000Z" };

class FakeWorker implements WorkerLike {
  state: string;
  messages: unknown[] = [];
  private listeners: Array<() => void> = [];
  constructor(
    public version: string | null,
    state = "installed",
  ) {
    this.state = state;
  }
  postMessage(message: unknown) {
    this.messages.push(message);
  }
  addEventListener(_type: "statechange", listener: () => void) {
    this.listeners.push(listener);
  }
  setState(state: string) {
    this.state = state;
    for (const l of this.listeners) l();
  }
}

class FakeRegistration implements UpdateRegistrationLike {
  waiting: FakeWorker | null = null;
  installing: FakeWorker | null = null;
  updates = 0;
  onUpdate: (() => void) | null = null;
  private found: Array<() => void> = [];
  async update() {
    this.updates++;
    this.onUpdate?.();
  }
  addEventListener(_type: "updatefound", listener: () => void) {
    this.found.push(listener);
  }
  removeEventListener(_type: "updatefound", listener: () => void) {
    this.found = this.found.filter((l) => l !== listener);
  }
  /** A new worker starts installing. */
  startInstall(version: string) {
    this.installing = new FakeWorker(version, "installing");
    for (const l of this.found) l();
    return this.installing;
  }
  /** The installing worker finishes and waits. */
  finishInstall() {
    const w = this.installing!;
    this.installing = null;
    this.waiting = w;
    w.setState("installed");
  }
}

class MemoryStorage {
  map = new Map<string, string>();
  getItem(k: string) {
    return this.map.get(k) ?? null;
  }
  setItem(k: string, v: string) {
    this.map.set(k, v);
  }
}

function makeEnv(opts: {
  latest?: { version: string; builtAt?: string; install?: string } | "offline";
  registration?: FakeRegistration | null;
  controller?: boolean;
  pending?: number;
  iosStandalone?: boolean;
  session?: MemoryStorage;
  local?: MemoryStorage;
}) {
  let now = 1_000_000;
  let latest = opts.latest ?? { version: "v1" };
  let pending = opts.pending ?? 0;
  const outboxListeners = new Set<() => void>();
  const docListeners = new Set<() => void>();
  const controllerListeners = new Set<() => void>();
  const timers: Array<{ fn: () => void; at: number; id: number }> = [];
  const intervals: Array<{ fn: () => void; id: number }> = [];
  let nextId = 1;
  const fetches: string[] = [];
  const warnings: string[] = [];
  let reloads = 0;
  const reloadUrls: Array<string | undefined> = [];
  const doc = {
    visibilityState: "visible",
    addEventListener: (_t: string, l: () => void) => docListeners.add(l),
    removeEventListener: (_t: string, l: () => void) => docListeners.delete(l),
  };
  const container = {
    controller: opts.controller === false ? null : {},
    getRegistration: async () => opts.registration ?? undefined,
    addEventListener: (_t: string, l: () => void) => controllerListeners.add(l),
    removeEventListener: (_t: string, l: () => void) => controllerListeners.delete(l),
  };
  const env: AppUpdateEnv = {
    serviceWorker: opts.registration === null ? null : container,
    fetch: async (url) => {
      fetches.push(url);
      if (latest === "offline") throw new TypeError("Failed to fetch");
      const body = latest;
      return { ok: true, json: async () => body };
    },
    document: doc,
    setInterval: (fn) => {
      const id = nextId++;
      intervals.push({ fn, id });
      return id;
    },
    clearInterval: (h) => {
      const i = intervals.findIndex((x) => x.id === h);
      if (i >= 0) intervals.splice(i, 1);
    },
    setTimeout: (fn, ms) => {
      const id = nextId++;
      timers.push({ fn, at: now + ms, id });
      return id;
    },
    clearTimeout: (h) => {
      const i = timers.findIndex((x) => x.id === h);
      if (i >= 0) timers.splice(i, 1);
    },
    now: () => now,
    reload: (url) => {
      reloads++;
      reloadUrls.push(url);
    },
    sessionStorage: opts.session ?? new MemoryStorage(),
    localStorage: opts.local ?? new MemoryStorage(),
    iosStandalone: opts.iosStandalone ?? false,
    pendingWrites: () =>
      opts.pending === undefined
        ? null
        : {
            count: () => pending,
            subscribe: (l) => {
              outboxListeners.add(l);
              return () => outboxListeners.delete(l);
            },
          },
    askVersion: async (worker) => (worker as FakeWorker).version,
    warn: (m) => warnings.push(m),
  };
  return {
    env,
    fetches,
    warnings,
    intervals,
    reloads: () => reloads,
    reloadUrls,
    setLatest: (l: typeof latest) => {
      latest = l;
    },
    setPending: (n: number) => {
      pending = n;
      for (const l of [...outboxListeners]) l();
    },
    outboxListeners,
    setVisibility: (v: string) => {
      doc.visibilityState = v;
      for (const l of [...docListeners]) l();
    },
    fireControllerChange: () => {
      for (const l of [...controllerListeners]) l();
    },
    advance: (ms: number) => {
      now += ms;
      for (const t of timers.filter((t) => t.at <= now)) {
        timers.splice(timers.indexOf(t), 1);
        t.fn();
      }
    },
  };
}

const tick = () => new Promise((r) => setTimeout(r, 0));
async function settle() {
  for (let i = 0; i < 10; i++) await tick();
}

test("the same build on the server: no update", async () => {
  const f = makeEnv({ latest: { version: "v1" }, registration: new FakeRegistration() });
  const u = createAppUpdate({ build: BUILD, production: true, env: f.env });
  const off = u.subscribe(() => {});
  await u.check();
  const s = u.getSnapshot();
  expect(s.version).toBe("v1");
  expect(s.builtAt).toBe(BUILD.builtAt);
  expect(s.latestVersion).toBe("v1");
  expect(s.updateReady).toBe(false);
  expect(f.fetches[0]).toBe(VERSION_URL);
  off();
});

test("a new build on the server is ready, and the browser fetches its worker", async () => {
  const reg = new FakeRegistration();
  const f = makeEnv({
    latest: { version: "v2", builtAt: "2026-10-22T11:00:00.000Z" },
    registration: reg,
  });
  const u = createAppUpdate({ build: BUILD, production: true, env: f.env });
  await u.check();
  expect(u.getSnapshot().updateReady).toBe(true);
  expect(u.getSnapshot().latestVersion).toBe("v2");
  expect(u.getSnapshot().latestBuiltAt).toBe("2026-10-22T11:00:00.000Z");
  expect(reg.updates).toBe(1);
});

test("a waiting worker with a new version is ready; one with the same version is not", async () => {
  const reg = new FakeRegistration();
  reg.waiting = new FakeWorker("v1");
  const f = makeEnv({ latest: "offline", registration: reg });
  const u = createAppUpdate({ build: BUILD, production: true, env: f.env });
  await u.check();
  expect(u.getSnapshot().updateReady).toBe(false);

  reg.waiting = new FakeWorker("v2");
  await u.check();
  expect(u.getSnapshot().updateReady).toBe(true);
});

test("a worker that finishes installing makes the update ready without a new check", async () => {
  const reg = new FakeRegistration();
  const f = makeEnv({ latest: "offline", registration: reg });
  const u = createAppUpdate({ build: BUILD, production: true, env: f.env });
  let notified = 0;
  u.subscribe(() => notified++);
  await settle();
  expect(u.getSnapshot().updateReady).toBe(false);
  reg.startInstall("v2");
  reg.finishInstall();
  await settle();
  expect(u.getSnapshot().updateReady).toBe(true);
  expect(notified).toBeGreaterThan(0);
});

test("checks when the page becomes visible and on the interval, only while visible", async () => {
  const f = makeEnv({ latest: { version: "v1" }, registration: new FakeRegistration() });
  const u = createAppUpdate({ build: BUILD, production: true, env: f.env });
  const off = u.subscribe(() => {});
  await settle();
  expect(f.fetches).toHaveLength(1);

  f.setVisibility("hidden");
  f.intervals[0].fn();
  await settle();
  expect(f.fetches).toHaveLength(1);

  f.setLatest({ version: "v2" });
  f.setVisibility("visible");
  await settle();
  expect(f.fetches).toHaveLength(2);
  expect(u.getSnapshot().updateReady).toBe(true);

  f.intervals[0].fn();
  await settle();
  expect(f.fetches).toHaveLength(3);

  // The last subscriber leaving stops the checks.
  off();
  expect(f.intervals).toHaveLength(0);
  f.setVisibility("visible");
  await settle();
  expect(f.fetches).toHaveLength(3);
});

test("apply waits for pending writes, activates the waiting worker, and reloads once", async () => {
  const reg = new FakeRegistration();
  reg.waiting = new FakeWorker("v2");
  const f = makeEnv({ latest: { version: "v2" }, registration: reg, pending: 2 });
  const u = createAppUpdate({ build: BUILD, production: true, env: f.env });
  await u.check();

  const done = u.apply();
  await settle();
  expect(u.getSnapshot().applying).toBe(true);
  expect(u.getSnapshot().waitingForSync).toBe(true);
  expect(reg.waiting!.messages).toEqual([]);

  f.setPending(1);
  await settle();
  expect(reg.waiting!.messages).toEqual([]);

  f.setPending(0);
  await settle();
  expect(u.getSnapshot().waitingForSync).toBe(false);
  expect(reg.waiting!.messages).toEqual([{ type: "pylon:skip-waiting" }]);
  expect(f.reloads()).toBe(0);

  f.fireControllerChange();
  await done;
  expect(f.reloads()).toBe(1);
  // A second controllerchange (or a second apply) does not reload again.
  f.fireControllerChange();
  await settle();
  expect(f.reloads()).toBe(1);
  expect(u.getSnapshot().applying).toBe(false);
});

test("apply with a new build the worker has not installed yet waits for the install", async () => {
  const reg = new FakeRegistration();
  reg.onUpdate = () => reg.startInstall("v2");
  const f = makeEnv({ latest: { version: "v2" }, registration: reg });
  const u = createAppUpdate({ build: BUILD, production: true, env: f.env });
  await u.check();
  // check() already asked the browser to fetch the new worker.
  expect(reg.installing).not.toBeNull();

  const done = u.apply();
  await settle();
  expect(f.reloads()).toBe(0);
  reg.finishInstall();
  await settle();
  expect(reg.waiting!.messages).toContainEqual({ type: "pylon:skip-waiting" });
  f.fireControllerChange();
  await done;
  expect(f.reloads()).toBe(1);
});

test("apply reloads anyway when the worker never takes over", async () => {
  const reg = new FakeRegistration();
  reg.waiting = new FakeWorker("v2");
  const f = makeEnv({ latest: { version: "v2" }, registration: reg });
  const u = createAppUpdate({ build: BUILD, production: true, env: f.env });
  const done = u.apply();
  await settle();
  expect(f.reloads()).toBe(0);
  f.advance(WORKER_WAIT_MS);
  await done;
  expect(f.reloads()).toBe(1);
});

test("without service workers, apply reloads after the writes are sent", async () => {
  const f = makeEnv({ latest: { version: "v2" }, registration: null, pending: 1 });
  const u = createAppUpdate({ build: BUILD, production: true, env: f.env });
  await u.check();
  expect(u.getSnapshot().updateReady).toBe(true);
  const done = u.apply();
  await settle();
  expect(f.reloads()).toBe(0);
  f.setPending(0);
  await done;
  expect(f.reloads()).toBe(1);
});

test("a reload that lands on the same build does not reload again", async () => {
  const session = new MemoryStorage();
  // First page: apply reloads.
  const first = makeEnv({ latest: { version: "v2" }, registration: null, session });
  const u1 = createAppUpdate({ build: BUILD, production: true, env: first.env });
  await u1.apply();
  expect(first.reloads()).toBe(1);

  // The reloaded page still runs v1 (a stale CDN, say), and the app calls
  // apply() on its own whenever updateReady is true.
  const second = makeEnv({ latest: { version: "v2" }, registration: null, session });
  const u2 = createAppUpdate({ build: BUILD, production: true, env: second.env });
  await u2.check();
  expect(u2.getSnapshot().updateReady).toBe(true);
  await u2.apply();
  expect(second.reloads()).toBe(0);
  expect(second.warnings[0]).toContain("Not reloading again");

  // After the guard window, apply works again.
  second.advance(RELOAD_GUARD_MS);
  const u3 = createAppUpdate({ build: BUILD, production: true, env: second.env });
  await u3.apply();
  expect(second.reloads()).toBe(1);
});

test("a reload that reached the new build may apply the next one at once", async () => {
  const session = new MemoryStorage();
  const first = makeEnv({ latest: { version: "v2" }, registration: null, session });
  await createAppUpdate({ build: BUILD, production: true, env: first.env }).apply();
  const second = makeEnv({ latest: { version: "v3" }, registration: null, session });
  const u = createAppUpdate({
    build: { version: "v2", builtAt: null },
    production: true,
    env: second.env,
  });
  await u.apply();
  expect(second.reloads()).toBe(1);
});

test("dev, SSR and apps without offline are inert", async () => {
  const f = makeEnv({ latest: { version: "v2" }, registration: new FakeRegistration() });
  for (const u of [
    createAppUpdate({ build: BUILD, production: false, env: f.env }),
    createAppUpdate({ build: null, production: true, env: f.env }),
    createAppUpdate({ build: BUILD, production: true, env: null }),
  ]) {
    const off = u.subscribe(() => {});
    await u.check();
    await u.apply();
    expect(u.getSnapshot().updateReady).toBe(false);
    off();
  }
  expect(f.fetches).toHaveLength(0);
  expect(f.reloads()).toBe(0);
});

test("an iOS home-screen app recommends adding it again when the install identity changed", async () => {
  const local = new MemoryStorage();
  const f = makeEnv({
    latest: { version: "v1", install: "i1" },
    registration: new FakeRegistration(),
    iosStandalone: true,
    local,
  });
  const u = createAppUpdate({ build: BUILD, production: true, env: f.env });
  await u.check();
  // The first check records what the app was added with.
  expect(u.getSnapshot().reinstallRecommended).toBe(false);

  f.setLatest({ version: "v2", install: "i1" });
  await u.check();
  expect(u.getSnapshot().reinstallRecommended).toBe(false);

  f.setLatest({ version: "v3", install: "i2" });
  await u.check();
  expect(u.getSnapshot().reinstallRecommended).toBe(true);

  // In a browser tab the same change means nothing.
  const tab = makeEnv({
    latest: { version: "v3", install: "i2" },
    registration: new FakeRegistration(),
    local,
  });
  const inTab = createAppUpdate({ build: BUILD, production: true, env: tab.env });
  await inTab.check();
  expect(inTab.getSnapshot().reinstallRecommended).toBe(false);
});

test("the first check does not record a newer build's install identity", async () => {
  const local = new MemoryStorage();
  const f = makeEnv({
    latest: { version: "v2", install: "i2" },
    registration: new FakeRegistration(),
    iosStandalone: true,
    local,
  });
  const u = createAppUpdate({ build: BUILD, production: true, env: f.env });
  await u.check();
  expect(local.map.size).toBe(0);
  expect(u.getSnapshot().reinstallRecommended).toBe(false);
});

// ---------------------------------------------------------------------------
// moveToLatestBuild: the client runtime moves an open page to a new deploy.
// Veritas, after a deploy: the new worker waited for every tab to close, and
// each click in an open tab became a full page load of the old build.
// ---------------------------------------------------------------------------

test("after a deploy, a click moves the page once: the waiting worker takes over and the link loads", async () => {
  const reg = new FakeRegistration();
  reg.waiting = new FakeWorker("v2");
  const f = makeEnv({ latest: { version: "v2" }, registration: reg, pending: 0 });
  const u = createAppUpdate({ build: BUILD, production: true, env: f.env });
  await u.check();

  const move = u.moveToLatestBuild({ url: "/leads", skipIfPendingWrites: true });
  await settle();
  expect(reg.waiting!.messages).toEqual([{ type: "pylon:skip-waiting" }]);
  f.fireControllerChange();
  expect(await move).toBe("moving");
  expect(f.reloadUrls).toEqual(["/leads"]);

  // More clicks before the page unloads: no second load.
  expect(await u.moveToLatestBuild({ url: "/map" })).toBe("moving");
  expect(f.reloads()).toBe(1);
});

test("a page that a move left on the old build does not move again", async () => {
  const session = new MemoryStorage();
  const first = makeEnv({ latest: { version: "v2" }, registration: null, session });
  const u1 = createAppUpdate({ build: BUILD, production: true, env: first.env });
  expect(await u1.moveToLatestBuild({ url: "/leads", recheck: true })).toBe("moving");

  // The load came back on v1 (the server answered with the old build). The
  // next click must not load again: that is the reload per click.
  const second = makeEnv({ latest: { version: "v2" }, registration: null, session });
  const u2 = createAppUpdate({ build: BUILD, production: true, env: second.env });
  expect(await u2.moveToLatestBuild({ url: "/map", recheck: true })).toBe("blocked");
  expect(await u2.moveToLatestBuild({ url: "/map", recheck: true })).toBe("blocked");
  expect(second.reloads()).toBe(0);
});

test("a click with unsent writes does not wait for them; a failed file load does", async () => {
  const reg = new FakeRegistration();
  reg.waiting = new FakeWorker("v2");
  const f = makeEnv({ latest: { version: "v2" }, registration: reg, pending: 1 });
  const u = createAppUpdate({ build: BUILD, production: true, env: f.env });
  await u.check();

  expect(await u.moveToLatestBuild({ url: "/leads", skipIfPendingWrites: true })).toBe(
    "pending-writes",
  );
  expect(reg.waiting!.messages).toEqual([]);

  // The old build cannot show the next screen: wait for the writes, then move.
  const move = u.moveToLatestBuild({ url: "/map", recheck: true });
  await settle();
  expect(u.getSnapshot().waitingForSync).toBe(true);
  expect(reg.waiting!.messages).toEqual([]);
  f.setPending(0);
  await settle();
  expect(reg.waiting!.messages).toEqual([{ type: "pylon:skip-waiting" }]);
  f.fireControllerChange();
  expect(await move).toBe("moving");
  expect(f.reloadUrls).toEqual(["/map"]);
});

test("a failed file load while offline keeps the page", async () => {
  const reg = new FakeRegistration();
  const f = makeEnv({ latest: { version: "v2" }, registration: reg });
  const u = createAppUpdate({ build: BUILD, production: true, env: f.env });
  await u.check();
  f.setLatest("offline");
  expect(await u.moveToLatestBuild({ recheck: true })).toBe("offline");
  expect(f.reloads()).toBe(0);
});

test("a page of the new build under the old worker lets its own worker take over", async () => {
  // The tab loaded build v2 from the network, but the v1 worker still
  // controls it and answers the build manifest from its cache.
  const reg = new FakeRegistration();
  reg.waiting = new FakeWorker("v2");
  const f = makeEnv({ latest: { version: "v2" }, registration: reg });
  const u = createAppUpdate({
    build: { version: "v2", builtAt: null },
    production: true,
    env: f.env,
  });
  const move = u.moveToLatestBuild({ url: "/leads", recheck: true });
  await settle();
  expect(reg.waiting!.messages).toEqual([{ type: "pylon:skip-waiting" }]);
  f.fireControllerChange();
  expect(await move).toBe("moving");
  expect(f.reloadUrls).toEqual(["/leads"]);
});

test("a failed file load with the server on this build does not move", async () => {
  const f = makeEnv({ latest: { version: "v1" }, registration: new FakeRegistration() });
  const u = createAppUpdate({ build: BUILD, production: true, env: f.env });
  expect(await u.moveToLatestBuild({ recheck: true })).toBe("current");
  expect(f.reloads()).toBe(0);
});

test("an app without offline does not move", async () => {
  const f = makeEnv({ latest: { version: "v2" }, registration: null });
  const u = createAppUpdate({ build: BUILD, production: false, env: f.env });
  expect(await u.moveToLatestBuild({ recheck: true })).toBe("inert");
  expect(f.fetches).toHaveLength(0);
});
