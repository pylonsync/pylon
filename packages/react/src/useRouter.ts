// Client navigation hooks for Pylon SSR pages — useRouter / useSearchParams
// / usePathname. They drive (and read) the same client runtime that <Link>
// uses: `window.__pylon.navigate` for programmatic nav, and the
// `pylon:navigation` event (dispatched by the runtime after every nav) +
// native `popstate` for reactivity.
//
// SSR note: useSearchParams / usePathname are CLIENT-reactive. During the
// server render (and the matching first hydration pass) they return defaults
// (empty params / "/") so there's never a hydration mismatch — React's
// useSyncExternalStore uses the server snapshot for both, then re-renders
// with the live value. For SSR-time access to the URL, read the `url` /
// `searchParams` PROPS the runtime already hands every page (see PageProps);
// the hooks exist for deep children that need to react to client navigation
// without prop-drilling.
import { use, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";

// `Window.__pylon` is globally augmented in ./Link (same package, ambient).

function subscribe(onChange: () => void): () => void {
  // React Native has a `window` global with no event API; treat it
  // like the server and never subscribe.
  if (
    typeof window === "undefined" ||
    typeof window.addEventListener !== "function"
  ) {
    return () => {};
  }
  window.addEventListener("popstate", onChange);
  window.addEventListener("pylon:navigation", onChange);
  return () => {
    window.removeEventListener("popstate", onChange);
    window.removeEventListener("pylon:navigation", onChange);
  };
}

// useSyncExternalStore compares snapshots with Object.is, so the getSnapshot
// must return a STABLE reference until the underlying value changes — a fresh
// URLSearchParams every call would loop forever. Cache by the raw search
// string.
let cachedSearch: string | null = null;
let cachedParams = new URLSearchParams();
function searchClientSnapshot(): URLSearchParams {
  const s = typeof window !== "undefined" ? window.location.search : "";
  if (s !== cachedSearch) {
    cachedSearch = s;
    cachedParams = new URLSearchParams(s);
  }
  return cachedParams;
}
const EMPTY_PARAMS = new URLSearchParams();
function searchServerSnapshot(): URLSearchParams {
  return EMPTY_PARAMS;
}

/**
 * The current query string as a reactive `URLSearchParams`. Re-renders on
 * client navigation. Returns empty params during SSR / first hydration —
 * use the `searchParams` page prop for server-side values.
 *
 * ```tsx
 * const params = useSearchParams();
 * const tab = params.get("tab") ?? "overview";
 * ```
 */
export function useSearchParams(): URLSearchParams {
  return useSyncExternalStore(
    subscribe,
    searchClientSnapshot,
    searchServerSnapshot,
  );
}

function pathClientSnapshot(): string {
  return typeof window !== "undefined" ? window.location.pathname : "/";
}
function pathServerSnapshot(): string {
  return "/";
}

/**
 * The current pathname (no query/hash), reactive to client navigation.
 * Returns "/" during SSR / first hydration — use the `url` page prop for
 * server-side values.
 */
export function usePathname(): string {
  return useSyncExternalStore(subscribe, pathClientSnapshot, pathServerSnapshot);
}

// The current route's dynamic params, stashed on `window.__pylon.params` by the
// SSR client runtime at hydration + on every nav. A stable object reference
// between navs (the runtime mints a fresh one per route), which
// useSyncExternalStore requires.
const EMPTY_OBJ: Record<string, string> = {};
function paramsClientSnapshot(): Record<string, string> {
  return (
    (typeof window !== "undefined" && window.__pylon?.params) || EMPTY_OBJ
  );
}
function paramsServerSnapshot(): Record<string, string> {
  return EMPTY_OBJ;
}

/**
 * The current route's dynamic params — e.g. `/dashboard/[projectId]` →
 * `{ projectId: "p_1" }`. Reactive to client navigation, so a deep child gets
 * the new params after a `<Link>` click without prop-drilling. Returns `{}`
 * during SSR / first hydration — use the `params` page prop for server-side
 * values. Drop-in for Next's `useParams`.
 *
 * ```tsx
 * const { projectId } = useParams<{ projectId: string }>();
 * ```
 */
export function useParams<
  T extends Record<string, string> = Record<string, string>,
>(): T {
  return useSyncExternalStore(
    subscribe,
    paramsClientSnapshot,
    paramsServerSnapshot,
  ) as T;
}

// The seed for the active navigation, stashed on `window.__pylon.seed` by the
// runtime when a <Link seed> is clicked. A stable reference for the whole nav
// (set once at nav start), which useSyncExternalStore requires. Null otherwise.
function seedClientSnapshot(): unknown {
  return (typeof window !== "undefined" && window.__pylon?.seed) || null;
}
function seedServerSnapshot(): unknown {
  return null;
}

/**
 * The seed passed to the `<Link seed>` that started the current navigation, for
 * an instant optimistic first paint. Returns the seed while the destination's
 * data is still loading, then `null` once the real server render lands (and on
 * hard loads / seedless navs). Use it as the page's Suspense fallback so
 * above-the-fold content shows immediately instead of a skeleton:
 *
 * ```tsx
 * export default function Page({ params, serverData }: PageProps<{ slug: string }>) {
 *   const seed = useRouteSeed<Product>();
 *   return (
 *     <Suspense fallback={seed ? <ProductView product={seed} pending /> : <Skeleton />}>
 *       <ProductDetail serverData={serverData} slug={params.slug} />
 *     </Suspense>
 *   );
 * }
 * ```
 */
export function useRouteSeed<T = unknown>(): T | null {
  return useSyncExternalStore(
    subscribe,
    seedClientSnapshot,
    seedServerSnapshot,
  ) as T | null;
}

/**
 * Load a page's primary data with an optimistic first paint from `<Link seed>`.
 * This is the flash-free way to consume a seed — prefer it over reading
 * `useRouteSeed()` into a Suspense fallback (a fallback→content transition
 * remounts, which flickers).
 *
 * - Hard load / SSR (no seed): suspends on `loader()`, so the server renders +
 *   streams the real content and the client hydrates from the pre-fulfilled
 *   serverData cache. Wrap the calling component in `<Suspense>`.
 * - Optimistic client nav (a `<Link seed>` was clicked): returns the seed
 *   immediately as real content — NO Suspense fallback — then swaps to the
 *   resolved `loader()` value IN PLACE (same component instance), so there is no
 *   remount between the seed and the authoritative row. No flash.
 *
 * `deps` drive the background reload — pass the values `loader` closes over
 * (e.g. `[serverData, slug]`). Key the component by its dynamic route param if
 * it serves both seeded and hard-loaded requests, so each navigation mounts a
 * fresh instance in the right mode.
 *
 * ```tsx
 * function Detail({ serverData, slug }: { serverData: ServerData; slug: string }) {
 *   const product = useRouteData(() => loadProduct(serverData, slug), [serverData, slug]);
 *   return product ? <ProductView product={product} /> : <NotFound />;
 * }
 * ```
 */
export function useRouteData<T>(
  loader: () => Promise<T> | T,
  deps: readonly unknown[],
): T | null {
  const seed = useRouteSeed<T>();
  // Fix the mode at first render so the hook calls below stay stable even if the
  // seed changes on a later same-instance nav (rules of hooks). `use()` may be
  // called conditionally, but useState/useEffect may not — hence the ref.
  const optimistic = useRef(seed != null).current;
  const [data, setData] = useState<T | null>(optimistic ? seed : null);
  useEffect(() => {
    if (!optimistic) return;
    let live = true;
    Promise.resolve(loader()).then((v) => {
      if (live && v != null) setData(v as T);
    });
    return () => {
      live = false;
    };
    // loader is intentionally excluded; `deps` are the reload trigger.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);
  // Hooks run in both modes so their order never changes.
  const committed = useRef<LoaderEntry | null>(null);
  const source = optimistic ? "" : loader.toString();
  const entry = optimistic
    ? null
    : committed.current && sameLoader(committed.current, source, deps)
      ? committed.current
      : loaderEntry(source, deps, loader);
  useEffect(() => {
    if (!entry) return;
    // Once mounted, the ref keeps the promise; the shared cache only has to
    // bridge renders that suspended before the first commit.
    committed.current = entry;
    releaseLoaderEntry(entry);
  });
  if (entry) {
    // SSR + hard load + non-seeded nav: suspend so the server streams the real
    // content and the client hydrates it synchronously from the pre-fulfilled
    // serverData cache (matches the server HTML — no mismatch). `use()` must
    // get the SAME promise on every attempt: a render that suspends before its
    // first commit keeps no state, so the promise lives in a shared cache
    // keyed by the loader's source and `deps`.
    return use(entry.promise) as T;
  }
  return data ?? seed;
}

interface LoaderEntry {
  source: string;
  deps: readonly unknown[];
  promise: Promise<unknown>;
}

/** Promises for loaders whose component has not committed yet. Bounded so
 *  renders that suspend and are then abandoned cannot grow it forever. */
const pendingLoaders: LoaderEntry[] = [];
const PENDING_LOADERS_MAX = 64;

function sameLoader(e: LoaderEntry, source: string, deps: readonly unknown[]): boolean {
  if (e.source !== source || e.deps.length !== deps.length) return false;
  for (let i = 0; i < deps.length; i++) {
    if (!Object.is(e.deps[i], deps[i])) return false;
  }
  return true;
}

/** Server renders: entries keyed by the page's `serverData` found in
 *  `deps`. The SSR runtime builds a new `serverData` for every request,
 *  so a render's retries share one promise and nothing is shared between
 *  requests. Loaders whose deps hold no `serverData` are not cached on
 *  the server (a module-level object in deps would be shared by every
 *  request). */
const serverLoaders = new WeakMap<object, LoaderEntry[]>();

/** True for the SSR `serverData` object (see `ServerData` in ./ssr). */
function isServerData(d: unknown): d is object {
  if (d === null || typeof d !== "object") return false;
  const o = d as Record<string, unknown>;
  return (
    typeof o.get === "function" &&
    typeof o.list === "function" &&
    typeof o.queryGraph === "function" &&
    typeof o.paginate === "function"
  );
}

/** How long a rejected load stays cached. React re-renders the suspended
 *  component once the promise rejects; that render must see the same
 *  rejected promise so the error reaches the error boundary. A retry
 *  after this window runs the loader again. */
const REJECTED_ENTRY_TTL_MS = 1_000;

function makeEntry(source: string, deps: readonly unknown[], loader: () => unknown): LoaderEntry {
  const value = loader();
  let promise: Promise<unknown>;
  if (value != null && typeof (value as { then?: unknown }).then === "function") {
    promise = value as Promise<unknown>;
  } else {
    promise = Promise.resolve(value);
    // React reads `status` / `value` on a thenable to skip suspending.
    Object.assign(promise, { status: "fulfilled", value });
  }
  return { source, deps: [...deps], promise };
}

/** Find or create the cached promise for this loader call. A synchronous
 *  loader result becomes an already-fulfilled thenable, which `use()` reads
 *  without suspending. */
function loaderEntry(
  source: string,
  deps: readonly unknown[],
  loader: () => unknown,
): LoaderEntry {
  if (typeof window === "undefined") {
    // The module is shared by every request; the key must include the
    // request (see `serverLoaders`).
    const scope = deps.find(isServerData);
    if (!scope) return makeEntry(source, deps, loader);
    let list = serverLoaders.get(scope);
    if (!list) {
      list = [];
      serverLoaders.set(scope, list);
    }
    const hit = list.find((e) => sameLoader(e, source, deps));
    if (hit) return hit;
    const entry = makeEntry(source, deps, loader);
    list.push(entry);
    const scoped = list;
    entry.promise.then(undefined, () => {
      setTimeout(() => {
        const i = scoped.indexOf(entry);
        if (i !== -1) scoped.splice(i, 1);
      }, REJECTED_ENTRY_TTL_MS);
    });
    return entry;
  }
  const hit = pendingLoaders.find((e) => sameLoader(e, source, deps));
  if (hit) return hit;
  const entry = makeEntry(source, deps, loader);
  pendingLoaders.push(entry);
  // A failed load must not stay cached forever: an error-boundary retry
  // has to run the loader again.
  entry.promise.then(undefined, () => {
    setTimeout(() => releaseLoaderEntry(entry), REJECTED_ENTRY_TTL_MS);
  });
  if (pendingLoaders.length > PENDING_LOADERS_MAX) pendingLoaders.shift();
  return entry;
}

function releaseLoaderEntry(entry: LoaderEntry): void {
  const i = pendingLoaders.indexOf(entry);
  if (i !== -1) pendingLoaders.splice(i, 1);
}

/**
 * Client-side redirect — replaces the current history entry with `href`.
 * Drop-in for Next's `redirect` when called from a client component
 * (effect/handler). For a redirect decided during a server render, use the
 * `response.redirect()` API on the page's `PageProps` instead.
 */
export function redirect(href: string): void {
  if (typeof window !== "undefined") {
    void window.__pylon?.navigate(href, { replace: true });
  }
}

/** Error thrown by {@link notFound}; the SSR not-found boundary renders it. */
export class NotFoundError extends Error {
  readonly digest = "PYLON_NOT_FOUND";
  constructor() {
    super("PYLON_NOT_FOUND");
    this.name = "NotFoundError";
  }
}

/**
 * Render the nearest `not-found.tsx` boundary from a client component by
 * throwing — drop-in for Next's `notFound`. For a 404 decided during a server
 * render, prefer `response.notFound()` on the page's `PageProps` so the
 * response carries a real 404 status.
 */
export function notFound(): never {
  throw new NotFoundError();
}

/** Imperative navigation handle (Next-style `useRouter`). */
export interface PylonRouter {
  /** Navigate to `href`, pushing a new history entry. */
  push(href: string): void;
  /** Navigate to `href`, replacing the current history entry. */
  replace(href: string): void;
  /** Go back one history entry. */
  back(): void;
  /** Go forward one history entry. */
  forward(): void;
  /** Re-fetch + re-render the current route (fresh server data). */
  refresh(): void;
  /** Warm the SSR HTML + chunks for `href` ahead of a navigation. */
  prefetch(href: string): void;
}

/**
 * Programmatic client navigation. Methods are no-ops before hydration /
 * during SSR (there's no client runtime yet), so they're safe to call from
 * effects and event handlers.
 *
 * ```tsx
 * const router = useRouter();
 * <button onClick={() => router.push("/dashboard")}>Go</button>
 * ```
 */
export function useRouter(): PylonRouter {
  return useMemo<PylonRouter>(
    () => ({
      push(href) {
        void window.__pylon?.navigate(href, { push: true });
      },
      replace(href) {
        void window.__pylon?.navigate(href, { replace: true });
      },
      back() {
        if (typeof window !== "undefined") window.history.back();
      },
      forward() {
        if (typeof window !== "undefined") window.history.forward();
      },
      refresh() {
        if (typeof window === "undefined") return;
        void window.__pylon?.navigate(
          window.location.pathname + window.location.search,
          { replace: true },
        );
      },
      prefetch(href) {
        void window.__pylon?.prefetch(href);
      },
    }),
    [],
  );
}

/** Test hooks for the useRouteData promise cache. */
export const __routeDataCacheInternals = {
  lookup: loaderEntry,
  size: (): number => pendingLoaders.length,
  clear: (): void => {
    pendingLoaders.length = 0;
  },
};
