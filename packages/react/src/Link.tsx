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
//     different things. On viewport entry (IntersectionObserver) the
//     runtime modulepreloads the destination's client chunks — static,
//     immutable, cacheable. On hover/touch it also fetches the page
//     itself, which costs a server render, so that waits for a real
//     signal of intent rather than firing for a screenful of links.
//   - Modifier keys (cmd/ctrl/shift/alt) or middle-click fall
//     through to the browser's default behavior (open in new tab,
//     etc.) — matches Next.js semantics.
//
// SSR: with no JS or before hydration, this is just an <a href>, so
// the link works regardless. Progressive enhancement.
//
// Disable client-side nav for a specific link with `prefetch={false}`
// (HTML still prefetches) or by setting `target="_blank"`.

import React, { useEffect, useRef } from "react";

declare global {
  interface Window {
    __pylon?: {
      prefetch: (
        href: string,
        opts?: {
          /** Also fetch the page payload (costs a server render — hover only). */
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
  children?: React.ReactNode;
}

export function Link({
  href,
  prefetch = true,
  seed,
  children,
  ...rest
}: LinkProps) {
  const ref = useRef<HTMLAnchorElement>(null);

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

    el.addEventListener("mouseenter", warmPage);
    el.addEventListener("touchstart", warmPage, { passive: true });

    return () => {
      io?.disconnect();
      el.removeEventListener("mouseenter", warmPage);
      el.removeEventListener("touchstart", warmPage);
    };
  }, [href, prefetch]);

  return (
    <a
      ref={ref}
      href={href}
      data-pylon-link=""
      {...rest}
    >
      {children}
    </a>
  );
}
