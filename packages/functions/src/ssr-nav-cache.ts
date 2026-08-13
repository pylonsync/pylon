// Prefetched page payloads for client-side navigation.
//
// `<Link>` warms a destination on hover; the click then consumes what the
// hover fetched instead of opening its own request. Before this existed the
// prefetch was a `<link rel="prefetch">` that could never be reused — SSR
// pages are sent "private, no-store", so nothing was storable, and an
// `as="document"` prefetch only feeds real navigations anyway, not `fetch()`.
// Every prefetch was therefore a full server render, executed and discarded,
// and the click still paid full price.
//
// So the cache lives HERE, in memory, for the length of the tab: honoring
// no-store means never handing the response to a disk cache, and it means a
// reload always re-renders.
//
// Kept in its own module (not inlined in CLIENT_RUNTIME_SOURCE) so its
// expiry, eviction and single-use rules are unit-testable — the same reason
// client-boundary and route-match were pulled out. The bundler stages a copy
// next to the generated runtime, which imports it as "./nav-cache".

export interface NavPayloadCacheDeps {
  /** Fetch a page, resolving to its HTML or null when it shouldn't be reused. */
  fetchPage: (target: string) => Promise<string | null>;
  /** Injected so tests drive expiry without sleeping. */
  now: () => number;
  /** Long enough to cover hover-to-click, short enough that nobody reads a
   *  page rendered from data this old. */
  ttlMs?: number;
  /** Cap on retained payloads; the oldest is evicted first. Pages are whole
   *  HTML documents, so this bounds memory on a link-dense page. */
  max?: number;
}

export interface NavPayloadCache {
  /** Warm `target`, unless a fresh entry is already present or in flight. */
  prefetch(target: string): void;
  /** Hand over `target`'s payload, or null. Single-use: a prefetch
   *  accelerates the NEXT click, and holding it past that would serve
   *  navigations from an increasingly stale render. */
  take(target: string): Promise<string | null> | null;
  /** Drop everything — called when a navigation commits, since entries were
   *  rendered against the page the user just left. */
  clear(): void;
  /** Retained entry count (tests + diagnostics). */
  size(): number;
}

export function createNavPayloadCache(
  deps: NavPayloadCacheDeps,
): NavPayloadCache {
  const { fetchPage, now } = deps;
  const ttlMs = deps.ttlMs ?? 15000;
  const max = deps.max ?? 8;
  // Holds the in-flight PROMISE, not just the resolved text, so a click that
  // lands mid-prefetch joins that request rather than starting a second one —
  // the common case, since hover-to-click is usually shorter than the render.
  const entries = new Map<string, { at: number; promise: Promise<string | null> }>();

  return {
    prefetch(target: string): void {
      const hit = entries.get(target);
      if (hit && now() - hit.at < ttlMs) return;
      // Re-warming an expired target: drop it first so the refreshed entry
      // re-enters at the END of the insertion order. Map.set on an existing
      // key keeps its original position, which would make a repeatedly
      // re-warmed target the first one evicted. No test covers this: a
      // re-warm only happens after expiry, so everything ahead of it in the
      // order has expired too and evicting it costs nothing today. Keeping
      // the LRU order honest anyway, so this stays true if the TTL does not.
      entries.delete(target);
      while (entries.size >= max) {
        const oldest = entries.keys().next();
        if (oldest.done) break;
        entries.delete(oldest.value);
      }
      // A rejected fetch must not surface as an unhandled rejection when
      // nobody ends up clicking; resolve to null and let the click refetch.
      const promise = fetchPage(target).catch(() => null);
      entries.set(target, { at: now(), promise });
    },

    take(target: string): Promise<string | null> | null {
      const hit = entries.get(target);
      if (!hit) return null;
      entries.delete(target);
      if (now() - hit.at >= ttlMs) return null;
      return hit.promise;
    },

    clear(): void {
      entries.clear();
    },

    size(): number {
      return entries.size;
    },
  };
}
