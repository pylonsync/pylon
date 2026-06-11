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
import { useMemo, useSyncExternalStore } from "react";

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
