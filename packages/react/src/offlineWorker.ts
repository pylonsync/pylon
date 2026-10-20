// Registers the offline service worker (`/pylon-sw.js`) for apps that set
// `buildManifest({ offline })`. The SSR client runtime calls this once per
// page load; apps do not call it themselves.
//
// The worker is served by the Pylon runtime and generated per build (see
// packages/functions/src/offline-sw.ts). It is never registered under
// `pylon dev`: a worker serves cached build files first, which would hide
// every edit until the next install.

const WORKER_URL = "/pylon-sw.js";

export interface OfflineWorkerOptions {
  /** The app set `buildManifest({ offline })`. */
  optedIn: boolean;
  /** Not `pylon dev`. */
  production: boolean;
  /** Whether a user is signed in on this page. The worker refetches the
   *  pages it keeps when this changes, because each one carries the
   *  signed-in bit. */
  signedIn?: boolean;
  /** Defaults to the global `navigator`. */
  navigator?: { serviceWorker?: ServiceWorkerContainerLike };
  /** Resolves when the page has loaded, so the worker's downloads do not
   *  compete with the page's own. Defaults to the window `load` event. */
  whenLoaded?: () => Promise<void>;
}

/** The parts of `ServiceWorkerContainer` this module uses. */
export interface ServiceWorkerContainerLike {
  register(url: string, options?: { scope?: string }): Promise<unknown>;
  getRegistrations?(): Promise<ReadonlyArray<RegistrationLike>>;
  ready?: Promise<{ active?: { postMessage(message: unknown): void } | null }>;
}

interface RegistrationLike {
  active?: { scriptURL: string } | null;
  waiting?: { scriptURL: string } | null;
  installing?: { scriptURL: string } | null;
  unregister(): Promise<boolean>;
}

function defaultWhenLoaded(): Promise<void> {
  if (typeof window === "undefined" || typeof document === "undefined") {
    return Promise.resolve();
  }
  if (document.readyState === "complete") return Promise.resolve();
  return new Promise((resolve) =>
    window.addEventListener("load", () => resolve(), { once: true }),
  );
}

function isPylonWorker(reg: RegistrationLike): boolean {
  const worker = reg.active ?? reg.waiting ?? reg.installing;
  if (!worker) return false;
  try {
    return new URL(worker.scriptURL).pathname === WORKER_URL;
  } catch {
    return false;
  }
}

/**
 * Register the worker when the app opted in and this is a production page.
 * Otherwise remove a Pylon worker an earlier build left on this origin (an
 * app that turned offline off, or `pylon dev` after `pylon start`). Workers
 * the app registered itself are left alone. Never throws: an app works
 * online without its worker.
 */
export async function registerOfflineWorker(
  opts: OfflineWorkerOptions,
): Promise<unknown | null> {
  const nav =
    opts.navigator ??
    (typeof navigator !== "undefined" ? (navigator as any) : undefined);
  const container = nav?.serviceWorker as ServiceWorkerContainerLike | undefined;
  if (!container) return null;
  try {
    if (!opts.optedIn || !opts.production) {
      const regs = (await container.getRegistrations?.()) ?? [];
      await Promise.all(regs.filter(isPylonWorker).map((r) => r.unregister()));
      return null;
    }
    await (opts.whenLoaded ?? defaultWhenLoaded)();
    const registration = await container.register(WORKER_URL, { scope: "/" });
    if (typeof opts.signedIn === "boolean" && container.ready) {
      const ready = await container.ready;
      ready.active?.postMessage({ type: "pylon:session", signedIn: opts.signedIn });
    }
    return registration;
  } catch (err) {
    // eslint-disable-next-line no-console
    console.warn("[pylon offline] service worker registration failed:", err);
    return null;
  }
}
