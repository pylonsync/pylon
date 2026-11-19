// Pylon's Next.js-style <Link>. SSR-safe (renders <a> with no JS),
// client-side enhanced for instant navigation + prefetch.
//
// Behavior on the client (post-hydration):
//   - Renders <a href data-pylon-link>. The runtime's global click
//     handler intercepts the click and calls `__pylon.navigate(href)`,
//     which fetches the new SSR HTML, dynamic-imports the matching
//     route entry, and re-renders the React root in place. Layouts
//     that match across the two routes survive reconciliation, so
//     their state persists.
//   - Prefetches in two stages, because the two halves cost very
//     different things. For every rendered link, the runtime
//     modulepreloads the destination's client chunks — static, immutable,
//     cacheable — once the page has painted and the browser is idle
//     (requestIdleCallback), and not at all under Save-Data or on 2G. That
//     covers links off screen, such as a tab bar, so the first tap is warm
//     too, without counting against this page's first paint. For a route
//     that navigates with no page fetch (`navigation = "client"` or an
//     offline route), the runtime also loads its entry then. On intent it
//     fetches the chunks at once and also fetches the page itself, which
//     costs a server render, so that waits for a real signal rather than
//     firing for a screenful of links. Intent is a cursor that stays on the
//     link, a touch that stays down without moving, a mouse press, or focus
//     (see ./linkIntent). A cursor crossing the link, or a touch that starts
//     a scroll, fetches nothing. The runtime also keeps at most two page
//     prefetches waiting on the server at a time.
//   - Modifier keys (cmd/ctrl/shift/alt) or middle-click fall
//     through to the browser's default behavior (open in new tab,
//     etc.) — matches Next.js semantics.
//
// SSR: with no JS or before hydration, this is just an <a href>, so
// the link works regardless. Progressive enhancement.
//
// Disable client-side nav for a specific link with `prefetch={false}`
// (HTML still prefetches) or by setting `target="_blank"`.

import React, { useCallback, useEffect, useRef } from "react";
import { watchLinkIntent } from "./linkIntent";

declare global {
  interface Window {
    __pylon?: {
      prefetch: (
        href: string,
        opts?: {
          /** Also fetch the page payload. Costs a server render, so only on an
           *  intent signal (see ./linkIntent). */
          document?: boolean;
        },
      ) => Promise<void>;
      navigate: (
        href: string,
        opts?: { push?: boolean; replace?: boolean; seed?: unknown },
      ) => Promise<void>;
      /** Current route's dynamic params (read by useParams). A getter on the
       *  runtime side, so it always reflects the latest navigation. */
      readonly params?: Record<string, string>;
      /** Seed the active navigation was started with (read by useRouteSeed).
       *  A getter on the runtime side; null outside an optimistic nav. */
      readonly seed?: unknown;
    };
    /** WeakMap of <Link seed> anchor element → its seed value. Populated by
     *  <Link> and read by the runtime's global click handler so it can pass the
     *  seed to navigate() for an optimistic first paint. */
    __pylonLinkSeeds?: WeakMap<Element, unknown>;
  }
}

export interface LinkProps
  extends Omit<React.AnchorHTMLAttributes<HTMLAnchorElement>, "href"> {
  /** Destination path. Same-origin paths get client-side nav; off-origin paths render as plain <a>. */
  href: string;
  /**
   * Prefetch the destination on viewport entry. Default true.
   * Set `false` to skip prefetch (useful for links the user
   * is unlikely to follow — pagination tail, etc.).
   */
  prefetch?: boolean;
  /**
   * Data you already have for the destination (e.g. the list item you clicked).
   * When set, Pylon paints the destination page's Suspense fallback with this
   * data the instant the link is clicked — before the SSR fetch resolves — so
   * above-the-fold content appears immediately instead of a skeleton or a delay.
   * Read it on the destination page with `useRouteSeed<T>()`.
   *
   * Degrades gracefully: no seed, or a route Pylon can't resolve on the client,
   * simply falls back to normal navigation. Best for detail routes whose page
   * keeps its `serverData` reads inside a `<Suspense>` (so the seeded fallback
   * has somewhere to show).
   */
  seed?: unknown;
  /**
   * A ref to the anchor element. A component that renders through this link,
   * such as a button with a `render` prop, passes one.
   */
  ref?: React.Ref<HTMLAnchorElement>;
  children?: React.ReactNode;
}

export function Link({
  href,
  prefetch = true,
  seed,
  ref: callerRef,
  children,
  ...rest
}: LinkProps) {
  const ref = useRef<HTMLAnchorElement>(null);

  // The anchor goes to both the caller's ref and this component's, which the
  // effects below read.
  const setRef = useCallback(
    (el: HTMLAnchorElement | null) => {
      ref.current = el;
      if (typeof callerRef === "function") {
        const cleanup = callerRef(el);
        return () => {
          ref.current = null;
          if (typeof cleanup === "function") cleanup();
          else callerRef(null);
        };
      }
      if (callerRef) callerRef.current = el;
      return () => {
        ref.current = null;
        if (callerRef) callerRef.current = null;
      };
    },
    [callerRef],
  );

  // Register the seed keyed by this anchor element so the runtime's global
  // click handler can find it and hand it to navigate(). A WeakMap keeps it
  // GC-safe (no leak when the link unmounts) and off the DOM node itself.
  useEffect(() => {
    if (typeof window === "undefined") return;
    const el = ref.current;
    if (!el || seed === undefined) return;
    const reg =
      window.__pylonLinkSeeds ||
      (window.__pylonLinkSeeds = new WeakMap<Element, unknown>());
    reg.set(el, seed);
    return () => {
      reg.delete(el);
    };
  }, [seed]);

  useEffect(() => {
    if (!prefetch || !ref.current) return;
    if (typeof window === "undefined") return;
    // Don't prefetch off-origin / hash-only / non-HTTP links.
    if (
      href.startsWith("http://") ||
      href.startsWith("https://") ||
      href.startsWith("//") ||
      href.startsWith("#") ||
      href.startsWith("mailto:") ||
      href.startsWith("tel:")
    ) {
      return;
    }

    const el = ref.current;
    // Chunks warm once, on sight. The page payload warms on intent — and may
    // re-warm on a later hover, since the runtime expires payloads that have
    // gone stale.
    let warmedChunks = false;
    const warmChunks = () => {
      if (warmedChunks) return;
      warmedChunks = true;
      window.__pylon?.prefetch(href);
    };
    const warmPage = () => {
      warmedChunks = true;
      window.__pylon?.prefetch(href, { document: true });
    };

    // Every rendered link, not only those in view, once the browser is idle.
    const idle =
      typeof window.requestIdleCallback === "function"
        ? window.requestIdleCallback(warmChunks, { timeout: 2000 })
        : window.setTimeout(warmChunks, 200);

    let io: IntersectionObserver | null = null;
    if (typeof IntersectionObserver !== "undefined") {
      io = new IntersectionObserver(
        (entries) => {
          for (const e of entries) {
            if (e.isIntersecting) {
              warmChunks();
              io?.disconnect();
            }
          }
        },
        { rootMargin: "256px" },
      );
      io.observe(el);
    }

    const stopIntent = watchLinkIntent(el, warmPage);

    return () => {
      if (typeof window.cancelIdleCallback === "function") window.cancelIdleCallback(idle);
      else window.clearTimeout(idle);
      io?.disconnect();
      stopIntent();
    };
  }, [href, prefetch]);

  return (
    <a
      ref={setRef}
      href={href}
      data-pylon-link=""
      {...rest}
    >
      {children}
    </a>
  );
}
