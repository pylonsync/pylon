"use client";

// App updates for apps that set `buildManifest({ offline })`.
//
// A deploy installs a new service worker, and the browser keeps it waiting
// until every tab of the app is closed. An app on the iOS home screen can
// stay open in the background for days, so it runs the old build with no
// sign of the new one. This module tells the page that a new build exists
// and loads it on request:
//
//   - The page knows its own build: the SSR client runtime passes the
//     version and build time from `__PYLON_DATA__` to
//     `registerOfflineWorker({ build })`, which calls `configureAppUpdate`.
//   - The latest build comes from `/_pylon/version` (no-store), checked when
//     the page becomes visible and every five minutes while it is visible.
//   - A waiting service worker is a new build too. The page asks it for its
//     version (`pylon:version`), so a worker with the same version (another
//     machine wrote the same build) does not count.
//   - `apply()` waits until the outbox has no pending writes, tells the
//     waiting worker to take over (`pylon:skip-waiting`), and reloads once
//     when it does.
//   - `moveToLatestBuild()` is the same move, started by the client runtime
//     instead of the app: on a navigation when the server already runs a
//     newer build, and when a file of this page's build failed to load (a
//     route entry or a `dynamic()` chunk the deploy removed). It loads the
//     link's URL in the new build. A failure while offline does not move.
//   - On an iOS home-screen app, a change to the manifest's name, icons,
//     display mode or start URL sets `reinstallRecommended`: iOS reads those
//     only when the app is added.
//
// Under `pylon dev`, during SSR, and in an app without `offline`, the state
// stays empty and `apply()` does nothing.

import { OUTBOX_CHANNEL } from "@pylonsync/sync";
import { peekActiveEngine } from "./engine-registry";

/** The path the runtime serves the latest build's version at. */
export const VERSION_URL = "/_pylon/version";

/** How often a visible page checks for a new build. */
export const CHECK_INTERVAL_MS = 5 * 60 * 1000;

/** How long `apply()` waits for a new worker to install or take over
 *  before it reloads anyway. */
export const WORKER_WAIT_MS = 30 * 1000;

/** How long a waiting worker has to answer `pylon:version`. */
const VERSION_ASK_MS = 1000;

/** A reload that lands on the same build within this window does not
 *  reload again. */
export const RELOAD_GUARD_MS = 60 * 1000;

/** sessionStorage: the build `apply()` reloaded from, and when. */
const RELOAD_KEY = "pylon:app-update:reload";

/** localStorage: the install identity the home-screen app was added with. */
const INSTALL_KEY = "pylon:app-update:install";

/** What `moveToLatestBuild()` did. */
export type BuildMove =
  /** The page is loading the server's build. */
  | "moving"
  /** The server runs this page's build. */
  | "current"
  /** The server did not answer. */
  | "offline"
  /** Writes wait to be sent, and the caller asked not to wait for them. */
  | "pending-writes"
  /** The page reloaded from this build moments ago and still runs it. */
  | "blocked"
  /** No build to compare: `pylon dev`, SSR, or an app without `offline`. */
  | "inert";

export interface MoveOptions {
  /** Load this URL in the new build. Default: reload the current URL. */
  url?: string;
  /** Ask the server for its build first, because a file failed to load.
   *  Default: use what the last check found. */
  recheck?: boolean;
  /** Return `"pending-writes"` instead of waiting for pending writes. */
  skipIfPendingWrites?: boolean;
}

/** One build of the app. */
export interface AppBuild {
  /** The service worker version: a hash of the build. */
  version: string;
  /** When the build was made (ISO 8601), or null when unknown. */
  builtAt: string | null;
}

export interface AppUpdateState {
  /** The build this page runs. Null during SSR and in an app without
   *  `buildManifest({ offline })`. */
  version: string | null;
  builtAt: string | null;
  /** The latest build the server reported. Null until the first check
   *  succeeds. */
  latestVersion: string | null;
  latestBuiltAt: string | null;
  /** A newer build exists. `apply()` loads it. */
  updateReady: boolean;
  /** The app is on the iOS home screen, and its name, icons, display mode
   *  or start URL changed since it was added. The user must remove the app
   *  and add it again to see the change. */
  reinstallRecommended: boolean;
  /** `apply()` is running. */
  applying: boolean;
  /** `apply()` is waiting for pending writes to reach the server. */
  waitingForSync: boolean;
}

/** The parts of `ServiceWorker` this module uses. */
export interface WorkerLike {
  state?: string;
  postMessage(message: unknown, transfer?: unknown[]): void;
  addEventListener?(type: "statechange", listener: () => void): void;
}

/** The parts of `ServiceWorkerRegistration` this module uses. */
export interface UpdateRegistrationLike {
  waiting?: WorkerLike | null;
  installing?: WorkerLike | null;
  update?(): Promise<unknown>;
  addEventListener?(type: "updatefound", listener: () => void): void;
  removeEventListener?(type: "updatefound", listener: () => void): void;
}

/** The parts of `ServiceWorkerContainer` this module uses. */
export interface UpdateContainerLike {
  controller?: unknown;
  getRegistration?(scope?: string): Promise<UpdateRegistrationLike | undefined>;
  addEventListener?(type: "controllerchange", listener: () => void): void;
  removeEventListener?(type: "controllerchange", listener: () => void): void;
}

interface StorageLike {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

/** The pending writes `apply()` waits for. */
export interface PendingWrites {
  count(): number;
  subscribe(listener: () => void): () => void;
}

/** Everything the updater touches outside itself. Tests pass their own;
 *  the browser defaults come from `browserEnv`. */
export interface AppUpdateEnv {
  serviceWorker: UpdateContainerLike | null;
  fetch(url: string, init: RequestInit): Promise<{ ok: boolean; json(): Promise<unknown> }>;
  document: {
    visibilityState: string;
    addEventListener(type: "visibilitychange", listener: () => void): void;
    removeEventListener(type: "visibilitychange", listener: () => void): void;
  } | null;
  setInterval(fn: () => void, ms: number): unknown;
  clearInterval(handle: unknown): void;
  setTimeout(fn: () => void, ms: number): unknown;
  clearTimeout(handle: unknown): void;
  now(): number;
  /** Reload the page, or load `url` when given. */
  reload(url?: string): void;
  sessionStorage: StorageLike | null;
  localStorage: StorageLike | null;
  /** The page runs as an iOS home-screen app (`navigator.standalone`). */
  iosStandalone: boolean;
  /** The sync engine's pending writes, or null when the app has none. */
  pendingWrites(): PendingWrites | null;
  /** Asks a waiting worker for its version. Null when it does not answer. */
  askVersion(worker: WorkerLike): Promise<string | null>;
  warn(message: string): void;
}

const INERT: AppUpdateState = Object.freeze({
  version: null,
  builtAt: null,
  latestVersion: null,
  latestBuiltAt: null,
  updateReady: false,
  reinstallRecommended: false,
  applying: false,
  waitingForSync: false,
}) as AppUpdateState;

/** The state during SSR and before the client runtime configured a build. */
export const INERT_APP_UPDATE_STATE: AppUpdateState = INERT;

interface VersionReport {
  version: string;
  builtAt: string | null;
  install: string | null;
}

function readJson(storage: StorageLike | null, key: string): any {
  try {
    const raw = storage?.getItem(key);
    return raw ? JSON.parse(raw) : null;
  } catch {
    return null;
  }
}

function writeItem(storage: StorageLike | null, key: string, value: string): void {
  try {
    storage?.setItem(key, value);
  } catch {
    // Storage full or blocked: the guard and the install record are
    // best-effort.
  }
}

/** Checks for new builds and loads them. One per page; see `appUpdate`. */
export class AppUpdater {
  private build: AppBuild | null = null;
  private production = false;
  private envFactory: () => AppUpdateEnv | null;
  private env: AppUpdateEnv | null = null;
  private listeners = new Set<() => void>();
  private snapshot: AppUpdateState = INERT;

  private latest: VersionReport | null = null;
  private waitingVersion: string | null = null;
  private reinstall = false;
  private applying = false;
  private waitingForSync = false;

  private registration: UpdateRegistrationLike | null = null;
  private running = false;
  private interval: unknown = null;
  private checking: Promise<void> | null = null;
  private applyRun: Promise<boolean> | null = null;
  private reloading = false;

  constructor(envFactory: () => AppUpdateEnv | null) {
    this.envFactory = envFactory;
  }

  /** Set the build this page runs. `production` is false under `pylon
   *  dev`, which leaves the updater inert. */
  configure(build: AppBuild | null, production: boolean): void {
    this.build = build;
    this.production = production;
    this.emit();
    if (this.listeners.size > 0) this.start();
  }

  getSnapshot = (): AppUpdateState => this.snapshot;

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    if (this.listeners.size === 1) this.start();
    return () => {
      this.listeners.delete(listener);
      if (this.listeners.size === 0) this.stop();
    };
  };

  /** Ask the server for the latest build now. */
  check = (): Promise<void> => {
    const env = this.activeEnv();
    if (!env) return Promise.resolve();
    if (!this.checking) {
      this.checking = this.runCheck(env).finally(() => {
        this.checking = null;
      });
    }
    return this.checking;
  };

  /**
   * Load the newest build: wait until the outbox has no pending writes,
   * let the waiting service worker take over, and reload the page once.
   * Does nothing under `pylon dev`, during SSR, or right after a reload
   * that did not change the build (so a page that calls `apply()` on its
   * own cannot reload in a loop).
   */
  apply = (): Promise<void> => this.startApply().then(() => {});

  /**
   * Move to the server's build when it is not this page's: `apply()`, then
   * load `url` (or reload). Resolves with what it did; on `"moving"` the
   * page is about to unload. Never moves while the server cannot be reached,
   * so an offline page keeps its screen.
   */
  moveToLatestBuild = async (options: MoveOptions = {}): Promise<BuildMove> => {
    const env = this.activeEnv();
    if (!env) return "inert";
    if (this.reloading) return "moving";
    if (options.recheck) {
      this.latest = null;
      await this.check();
    }
    const latest = this.latest?.version ?? null;
    if (latest === null) return "offline";
    // The server runs this page's build, but its worker still waits behind
    // an older one, which answers with the older build's cached files (the
    // build manifest among them, from a worker of an earlier Pylon). Let it take
    // over.
    const ownWorkerWaits = this.waitingVersion === this.build!.version;
    if (latest === this.build!.version && !ownWorkerWaits) return "current";
    if (options.skipIfPendingWrites && (env.pendingWrites()?.count() ?? 0) > 0) {
      return "pending-writes";
    }
    return (await this.startApply(options.url)) ? "moving" : "blocked";
  };

  /** Run `apply()` once at a time; resolves with whether the page reloads. */
  private startApply(url?: string): Promise<boolean> {
    const env = this.activeEnv();
    if (!env) return Promise.resolve(false);
    if (!this.applyRun) {
      this.applying = true;
      this.emit();
      this.applyRun = this.runApply(env, url).finally(() => {
        this.applyRun = null;
        this.applying = false;
        this.waitingForSync = false;
        this.emit();
      });
    }
    return this.applyRun;
  };

  private activeEnv(): AppUpdateEnv | null {
    if (!this.build || !this.production) return null;
    if (!this.env) this.env = this.envFactory();
    return this.env;
  }

  private start(): void {
    const env = this.activeEnv();
    if (!env || this.running) return;
    this.running = true;
    env.document?.addEventListener("visibilitychange", this.onVisibility);
    this.interval = env.setInterval(() => {
      if (!env.document || env.document.visibilityState === "visible") void this.check();
    }, CHECK_INTERVAL_MS);
    void this.check();
  }

  private stop(): void {
    if (!this.running || !this.env) return;
    this.running = false;
    this.env.document?.removeEventListener("visibilitychange", this.onVisibility);
    this.env.clearInterval(this.interval);
    this.interval = null;
  }

  private onVisibility = (): void => {
    if (this.env?.document?.visibilityState === "visible") void this.check();
  };

  private emit(): void {
    const build = this.production ? this.build : null;
    const current = build?.version ?? null;
    const latestVersion = build ? (this.latest?.version ?? null) : null;
    const next: AppUpdateState = build
      ? {
          version: build.version,
          builtAt: build.builtAt,
          latestVersion,
          latestBuiltAt: this.latest?.builtAt ?? null,
          updateReady:
            (latestVersion !== null && latestVersion !== current) ||
            (this.waitingVersion !== null && this.waitingVersion !== current),
          reinstallRecommended: this.reinstall,
          applying: this.applying,
          waitingForSync: this.waitingForSync,
        }
      : INERT;
    const prev = this.snapshot;
    const same = (Object.keys(next) as Array<keyof AppUpdateState>).every(
      (k) => prev[k] === next[k],
    );
    if (same) return;
    this.snapshot = next;
    for (const listener of this.listeners) listener();
  }

  private async runCheck(env: AppUpdateEnv): Promise<void> {
    try {
      const res = await env.fetch(VERSION_URL, {
        cache: "no-store",
        credentials: "same-origin",
      });
      if (res.ok) {
        const body: any = await res.json();
        if (body && typeof body.version === "string") {
          this.latest = {
            version: body.version,
            builtAt: typeof body.builtAt === "string" ? body.builtAt : null,
            install: typeof body.install === "string" ? body.install : null,
          };
        }
      }
    } catch {
      // Offline, or the server is restarting: the next check tries again.
    }
    const reg = await this.watchRegistration(env);
    if (
      reg &&
      this.latest &&
      this.latest.version !== this.build?.version &&
      !reg.waiting &&
      !reg.installing
    ) {
      // The server has a new build the browser has not fetched the worker
      // for yet. Fetch it now rather than on the next navigation.
      try {
        await reg.update?.();
      } catch {
        // The worker script failed to download; the next check tries again.
      }
    }
    await this.readWaiting(env);
    this.readInstall(env);
    this.emit();
  }

  /** The Pylon worker's registration, with an `updatefound` listener that
   *  records a new worker once it is installed. */
  private async watchRegistration(env: AppUpdateEnv): Promise<UpdateRegistrationLike | null> {
    if (this.registration) return this.registration;
    const container = env.serviceWorker;
    if (!container?.getRegistration) return null;
    let reg: UpdateRegistrationLike | undefined;
    try {
      reg = await container.getRegistration("/");
    } catch {
      return null;
    }
    if (!reg) return null;
    this.registration = reg;
    reg.addEventListener?.("updatefound", () => {
      const worker = reg!.installing;
      worker?.addEventListener?.("statechange", () => {
        if (worker.state === "installed") {
          void this.readWaiting(env).then(() => this.emit());
        }
      });
    });
    return reg;
  }

  private async readWaiting(env: AppUpdateEnv): Promise<void> {
    const waiting = this.registration?.waiting;
    if (!waiting) {
      this.waitingVersion = null;
      return;
    }
    const version = await env.askVersion(waiting);
    // A worker that does not answer is still a different script: count it
    // as a new build.
    this.waitingVersion = version ?? "unknown";
  }

  private readInstall(env: AppUpdateEnv): void {
    const latest = this.latest;
    if (!env.iosStandalone || !latest?.install) {
      this.reinstall = false;
      return;
    }
    const stored = readJson(env.localStorage, INSTALL_KEY);
    if (!stored || typeof stored.install !== "string") {
      // First check since the app was added (iOS gives each home-screen app
      // its own storage, so adding it again clears this). Record the
      // identity of the build it runs, not a newer one.
      if (latest.version === this.build?.version) {
        writeItem(env.localStorage, INSTALL_KEY, JSON.stringify({ install: latest.install }));
      }
      this.reinstall = false;
      return;
    }
    this.reinstall = stored.install !== latest.install;
  }

  private async runApply(env: AppUpdateEnv, url?: string): Promise<boolean> {
    const from = this.build!.version;
    const last = readJson(env.sessionStorage, RELOAD_KEY);
    if (
      last &&
      last.from === from &&
      typeof last.at === "number" &&
      env.now() - last.at < RELOAD_GUARD_MS
    ) {
      env.warn(
        `[pylon] apply() reloaded ${Math.round((env.now() - last.at) / 1000)}s ago ` +
          `and the page still runs build ${from}. Not reloading again.`,
      );
      return false;
    }

    await this.drainOutbox(env);

    const reg = await this.watchRegistration(env);
    if (reg && !reg.waiting) {
      const behind = this.latest !== null && this.latest.version !== from;
      if (reg.installing || behind) {
        if (!reg.installing) {
          try {
            await reg.update?.();
          } catch {
            // Reload without the new worker: navigations are network
            // first, so the page still gets the new build.
          }
        }
        await this.waitForInstalled(env, reg);
      }
    }

    writeItem(env.sessionStorage, RELOAD_KEY, JSON.stringify({ from, at: env.now() }));

    const waiting = reg?.waiting;
    const container = env.serviceWorker;
    if (waiting && container?.controller) {
      await new Promise<void>((resolve) => {
        let timer: unknown = null;
        const done = () => {
          container.removeEventListener?.("controllerchange", done);
          env.clearTimeout(timer);
          resolve();
        };
        container.addEventListener?.("controllerchange", done);
        timer = env.setTimeout(done, WORKER_WAIT_MS);
        waiting.postMessage({ type: "pylon:skip-waiting" });
      });
    } else if (waiting) {
      waiting.postMessage({ type: "pylon:skip-waiting" });
    }
    this.reload(env, url);
    return true;
  }

  private reload(env: AppUpdateEnv, url?: string): void {
    if (this.reloading) return;
    this.reloading = true;
    env.reload(url);
  }

  private drainOutbox(env: AppUpdateEnv): Promise<void> {
    const writes = env.pendingWrites();
    if (!writes || writes.count() === 0) return Promise.resolve();
    this.waitingForSync = true;
    this.emit();
    return new Promise((resolve) => {
      let unsubscribe: (() => void) | null = null;
      let drained = false;
      const onChange = () => {
        if (drained || writes.count() > 0) return;
        drained = true;
        unsubscribe?.();
        this.waitingForSync = false;
        this.emit();
        resolve();
      };
      unsubscribe = writes.subscribe(onChange);
      // The last write may have landed between the count and the subscribe.
      onChange();
    });
  }

  private waitForInstalled(env: AppUpdateEnv, reg: UpdateRegistrationLike): Promise<void> {
    return new Promise((resolve) => {
      let finished = false;
      let timer: unknown = null;
      const finish = () => {
        if (finished) return;
        finished = true;
        env.clearTimeout(timer);
        reg.removeEventListener?.("updatefound", onFound);
        resolve();
      };
      const watch = (worker: WorkerLike | null | undefined) => {
        worker?.addEventListener?.("statechange", () => {
          if (worker.state !== "installing") finish();
        });
      };
      const onFound = () => watch(reg.installing);
      if (reg.waiting) return finish();
      timer = env.setTimeout(finish, WORKER_WAIT_MS);
      reg.addEventListener?.("updatefound", onFound);
      watch(reg.installing);
    });
  }
}

function safeStorage(get: () => StorageLike): StorageLike | null {
  try {
    return get();
  } catch {
    return null;
  }
}

/** The browser environment, or null outside a browser. */
export function browserEnv(): AppUpdateEnv | null {
  if (typeof window === "undefined" || typeof document === "undefined") return null;
  const nav: any = typeof navigator !== "undefined" ? navigator : {};
  return {
    serviceWorker: nav.serviceWorker ?? null,
    fetch: (url, init) => fetch(url, init),
    document,
    setInterval: (fn, ms) => setInterval(fn, ms),
    clearInterval: (h) => clearInterval(h as any),
    setTimeout: (fn, ms) => setTimeout(fn, ms),
    clearTimeout: (h) => clearTimeout(h as any),
    now: () => Date.now(),
    reload: (url) => (url ? window.location.assign(url) : window.location.reload()),
    sessionStorage: safeStorage(() => window.sessionStorage),
    localStorage: safeStorage(() => window.localStorage),
    iosStandalone: nav.standalone === true,
    pendingWrites: () => {
      const engine = peekActiveEngine();
      if (!engine) return null;
      return {
        count: () => engine.outbox().pending.length,
        subscribe: (listener) => engine.store.subscribe(listener, OUTBOX_CHANNEL),
      };
    },
    askVersion: (worker) =>
      new Promise((resolve) => {
        if (typeof MessageChannel === "undefined") return resolve(null);
        const channel = new MessageChannel();
        const timer = setTimeout(() => {
          channel.port1.close();
          resolve(null);
        }, VERSION_ASK_MS);
        channel.port1.onmessage = (event) => {
          clearTimeout(timer);
          channel.port1.close();
          const version = event.data?.version;
          resolve(typeof version === "string" ? version : null);
        };
        worker.postMessage({ type: "pylon:version" }, [channel.port2]);
      }),
    // eslint-disable-next-line no-console
    warn: (message) => console.warn(message),
  };
}

/** The page's updater. `useAppUpdate()` reads it. */
export const appUpdate = new AppUpdater(browserEnv);

/**
 * Set the build this page runs. The SSR client runtime calls it through
 * `registerOfflineWorker({ build })`; apps do not call it.
 */
export function configureAppUpdate(build: AppBuild | null, production: boolean): void {
  appUpdate.configure(build, production);
}

/**
 * Move this page to the server's build when it runs another one. The SSR
 * client runtime calls it on navigation and when a route entry fails to
 * load; `dynamic()` calls it when a chunk fails to load. See
 * `AppUpdater.moveToLatestBuild`.
 */
export function moveToLatestBuild(options?: MoveOptions): Promise<BuildMove> {
  return appUpdate.moveToLatestBuild(options);
}

/** An updater with its own environment, for tests and for apps that
 *  render outside the SSR client runtime. */
export function createAppUpdate(options: {
  build: AppBuild | null;
  production: boolean;
  env: AppUpdateEnv | null;
}): AppUpdater {
  const updater = new AppUpdater(() => options.env);
  updater.configure(options.build, options.production);
  return updater;
}
