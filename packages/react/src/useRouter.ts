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
  if (typeof window === "undefined") return () => {};
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
  if (!optimistic) {
    // SSR + hard load + non-seeded nav: suspend so the server streams the real
    // content and the client hydrates it synchronously from the pre-fulfilled
    // serverData cache (matches the server HTML — no mismatch).
    return use(Promise.resolve(loader())) as T;
  }
  return data ?? seed;
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
