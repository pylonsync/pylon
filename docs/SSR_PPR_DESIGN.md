# SSR Partial Prerendering (PPR) — design

Status: proposal · Author: design pass 2026-06-28 · Owner: TBD

> Reviewed by codex (2026-06-28, verdict "needs-changes"); findings folded in below
> — React API semantics corrected, prelude-anonymity widened to headers/metadata, the
> uncached hydration tail added (§5.1), the PPR shell verdict + read path made
> explicit, and a React feature-probe made the Phase-1 gate. Each P0 is now reflected
> in the design or the open questions.

## 1. Problem

Pylon's anonymous output cache (#277) is **all-or-nothing**. A render is cacheable
only if `computeCacheVerdict` passes every condition — crucially `!authTouched`,
where `authTouched` is **one page-global flag** flipped by *any* read of
`props.auth` (`ssr-runtime.ts:1799-1805`). Because the root `layout.tsx` is part of
every render, a single `Boolean(auth.user_id)` for an auth-aware nav taints the
**entire app**: every page falls through to `Cache-Control: no-cache`
(`frontend.rs:2274`), every request re-runs the full Bun SSR render, and Cloudflare
marks it `DYNAMIC`.

This is the dominant real-world pattern (public marketing page + "Sign in / Dashboard"
nav in the layout). Live example: `notbehind.com` — `app/layout.tsx:23` reads
`auth.user_id`, no page opts into caching, so the whole site is uncached.

The current escape hatch (move auth out of the layout, resolve it client-side) is a
workaround: it forces an architecture contortion and a hydration flash. Next.js
solves this with **Partial Prerendering**: a static shell is prerendered with
dynamic "holes" (the auth-dependent regions) streamed in per request. Auth stays in
the layout, no flash, the static frame is cached. Pylon does not support this today.

## 2. Current architecture (what we're extending)

Render flow (`ssr-runtime.ts`, `handleSsr`):

1. `props` is built with `auth: authProxy` (Proxy → `authTouched`), `response`
   controller, `serverData` (React 19 `use()` + `<Suspense>` data over the RPC pipe).
2. `wantsStream = computeWantsStream(!!Loading, mod)` — static, from module exports
   (`loading.tsx` or `export const streaming = true`). Knowable before any await.
3. The layout chain wraps the page into one element (`buildLayoutTree`).
4. **One** `renderToReadableStream(element)` pass (`ssr-runtime.ts:1893`).
5. Buffered path: `await stream.allReady` (`:1946`) → full document, then headers.
   Streaming path: flush shell + Suspense fallbacks progressively.
6. `cacheable = computeCacheVerdict({revalidateSecs, forceDynamic, authTouched,
   cookieCount, strictPolicies, wantsStream, status})` (`:1977`). **INVARIANT:
   cacheable ⟹ !wantsStream** — streaming commits the head before `authTouched` is
   final, so a streamed render can never be cached.
7. On `cacheable`, emit internal `x-pylon-cacheable`; the host turns it into
   `public, s-maxage=N, stale-while-revalidate=N` and tees the full body into the
   ISR cache; absence → fail-closed `no-cache`.

ISR cache (`ssr_cache.rs`): stores the **full HTML body** + status + headers, keyed
`SHA-256(route_path, pathname, vary)`. `cache_key` accepts arbitrary vary tuples, but
the host wires only the **Host bucket** today (`ssr_cache_vary`, `frontend.rs:1722-1727`),
and the fast path is gated to `GET` + **empty query** + **no session cookie**
(`frontend.rs:698-704`) — i.e. query strings bypass the cache and authed requests never
hit it. Build-namespaced (`PYLON_ARTIFACT_ID`), mem-LRU over disk, TTL =
`revalidate_secs`, stale-while-revalidate. A cookie-anonymous hit serves from Rust with
**no Bun round-trip**. (This cookie gate is exactly what PPR must change — see §6.)

The gaps for PPR: (a) single-pass full-tree render, no shell/hole split; (b)
page-global `authTouched`; (c) cache XOR stream; (d) cache stores a whole body, not a
prelude + postponed state. No use of React's `prerender`/`resume`/`postpone`.

## 3. React 19 primitives

PPR is buildable on react-dom **without** adopting full RSC/Flight (Pylon stays
single-pass-SSR + hydrated islands). The exact API names/semantics below MUST be
verified against the pinned React before any code (the lock currently resolves
react-dom **19.2.7**; see O1) — codex flagged that the first draft named non-existent
exports and overstated the execution model. Corrected:

- `prerender(element, opts)` from `react-dom/static` → `{ prelude: ReadableStream,
  postponed: PostponedState | null }`. A `<Suspense>` boundary that is still pending
  when prerender finishes is **excluded** from the prelude (its fallback goes into the
  prelude) and recorded in `postponed`. `postponed` is `null` when everything resolved
  (fully static → today's plain cache path, no resume).
- `resume(element, postponed, opts)` from `react-dom/server` (Web-streams; the
  Node-stream twin is `resumeToPipeableStream`) → returns a `ReadableStream` for the
  remaining work. **Resume restarts from the root and SKIPS fully-prerendered
  subtrees** — it does not literally "render only the holes." The win is that
  completed shell subtrees are skipped, so the per-request cost is dominated by the
  dynamic boundaries, not a full re-render. (Earlier drafts claimed "shell components
  don't re-run" — too strong.)
- **Making a region a hole.** A per-request read (auth/cookies/…) must leave its
  enclosing `<Suspense>` *pending during prerender* so the boundary postpones. The
  robust, no-unstable-API mechanism: in prerender mode the read returns a
  **never-resolving promise** (suspends), so the boundary never completes and
  `prerender` postpones it. `React.unstable_postpone(reason)` does this more directly
  **iff** the pinned React exposes it — but it is NOT a documented-stable API, so the
  design must NOT depend on it; treat it as an optimization gated behind the feature
  probe. Either way, a dynamic read must be **inside a `<Suspense>`**; a read in the
  synchronous shell postpones the whole document → no shell → fail-closed to
  fully-dynamic.

## 4. Design

### 4.1 Two phases

**Prerender (cache miss):**

1. Build the element tree as today, but with **prerender-mode proxies over EVERY
   per-request input** — not just auth. The current runtime threads `props.auth`
   (`ssr-runtime.ts:1813`), `props.cookies` (`:1812`), `props.headers` (`:1811`, incl.
   `Host`), `props.url`/`searchParams`, and `response` into props. In prerender mode a
   read of any of them must **suspend** (never-resolving promise, per §3) so its
   enclosing `<Suspense>` postpones — a read inside a boundary becomes a hole; a read
   in the synchronous shell postpones the whole doc → fail-closed to fully-dynamic, no
   leak. codex P0: the first draft only proxied auth/cookies, leaving `headers.host`
   et al. free to poison the prelude.
2. **`generateMetadata` is the sharp edge.** It runs *before* React render and outside
   any Suspense (`ssr-runtime.ts:1826-1828`), and auto social-image/icon metadata reads
   request headers (`:1832`). Metadata can't be a "hole." So: run `generateMetadata`
   under the same suspend-proxies; if it touches any per-request input, **disable PPR
   for that route/request** (fall back to live dynamic). Only metadata that is a pure
   function of static params may enter the cached prelude.
3. `const { prelude, postponed } = await prerender(element, {...})`. Buffer `prelude`.
   If `postponed == null` → fully static; store as today's plain body (the existing
   #277 path, served Rust-only). Else store `{ prelude, postponed, status, headers }`.
4. Serve this request by immediately resuming (§4.2) so the miss isn't blocking.

**Single-flight (open work, not free today).** codex P1: the existing
`try_claim_write` (`ssr_cache.rs:441-464`) dedupes only the final *disk write* — losers
skip writing (`frontend.rs:1792-1796`) *after* already rendering live. So a cache-miss
herd still all prerender. If prerender cost makes that unacceptable, add render-level
single-flight *before* invoking Bun (claim → others await the winner's prelude). Until
then, do not claim the miss is single-flighted.

**Resume (every request, including the miss above):**

1. Read cached `{ prelude, postponed }` (mem/disk, `ssr_cache.rs`).
2. Build the element tree with the **real** props (real `auth`, `cookies`, headers).
3. `const holes = await resume(element, postponed, {...})` (Web-streams; §3).
4. Write the cached `prelude` bytes immediately (great TTFB), then pipe `holes`, then
   the **per-request hydration tail** (§5.1). Resume markers in the prelude tell the
   client where each boundary lands; hydration runs once post-EOF as today.

The shell is rendered once and cached; the holes are computed per request and **never
cached**. Resume restarts from the root but **skips the fully-prerendered subtrees**,
so the per-request cost is dominated by the dynamic boundaries rather than a full
re-render (it is *not* "only the holes execute" — see §3).

### 4.2 The dynamic-boundary contract

A region is a hole iff it (a) reads `auth`/`cookies` (or another per-request source)
**and** (b) is wrapped in `<Suspense>`. Authoring options:

- Explicit: `<Suspense fallback={<AnonNav/>}><AuthNav/></Suspense>` where `AuthNav`
  reads `auth`. Prelude caches `AnonNav`; per request the real `AuthNav` streams in.
- Sugar: a framework `<Dynamic fallback={…}>` = `<Suspense>` + a dev-time lint that
  warns if a child reads auth outside it.

If auth is read **outside** any Suspense (e.g. directly in `layout.tsx`'s body, like
notbehind today) → whole-doc postpone → no PPR benefit, falls back to fully-dynamic.
This is the safe default and the migration signal: "wrap your auth nav to get PPR."

### 4.3 Opt-in

PPR is opt-in and composes with `revalidate`:

```ts
export const revalidate = 300;   // shell freshness (existing #277 export)
export const ppr = true;         // NEW: allow shell/hole split for this route
```

`ppr` without `revalidate` is an error (no shell TTL). Un-annotated pages keep the
**exact** current buffered/streaming/cache code paths — zero behavior change.

**PPR shell verdict (codex P1).** PPR does NOT discard the other gates in
`computeCacheVerdict` (`ssr-runtime.ts:1485-1502`); it only stops treating an auth/cookie
read as a blanket veto *when that read happened inside a postponed boundary*. The shell
is cacheable iff: opted in (`ppr` + `revalidate`), **`!forceDynamic`**, **status 200**,
**no `Set-Cookie` emitted in the shell** (a shell cookie ⇒ personalized prelude ⇒
`no-store`, the absolute veto at `frontend.rs:2269`), strict-policy behavior preserved
(under `PYLON_STRICT_FN_POLICIES` serverData reads are auth-filtered, so a shell
`serverData` read is not shareable — keep vetoing it), and **no per-request read escaped
to the synchronous shell** (any such read postpones root → fully-dynamic). Only
in-boundary auth/cookie/header reads are permitted, and they become holes. Encode this
as a `computePprShellVerdict` sibling, pure + unit-tested for the same leak class.

## 5. Cache storage changes (`ssr_cache.rs`)

`Meta` gains `postponed: Option<String>` (serialized `PostponedState`) and a
`kind: Static | Ppr` discriminator. `CacheEntry`/`MemEntry` carry the prelude as
`body` plus the optional `postponed` blob. `get`/`put` signatures grow a `postponed`
parameter. Key, namespacing, TTL, mem-LRU, disk sweep, and `wipe_stale_namespaces`
are unchanged — the prelude is anonymous, so the **same** anonymity key applies. A
`Static` entry (postponed `None`) is served byte-for-byte as today (Rust-only, no
Bun); a `Ppr` entry triggers the resume round-trip (§4.1).

**Only the prelude is cached — never the hydration tail.**

### 5.1 The hydration tail is per-request (codex P0)

After React EOF, Pylon appends a `__PYLON_DATA__` blob (`ssr-runtime.ts:2144`) built by
`buildHydrationTail`, which strips `headers`/`cookies`/`serverData`/`response` but
**keeps the rest of `props` — including `auth`** (`:1140-1155`, `restProps`) — plus the
resolved `ssrData`. If we cached that tail with the prelude, an authed user would
hydrate against a frozen *anonymous* (or worse, another user's) `auth` and `ssrData` →
either a leak or a hydration mismatch that blanks the dynamic boundary.

So the tail is **never cached**. The cache holds the anonymous prelude only; the tail is
emitted per request, *after* `resume`, carrying the request's real `auth` and the
`ssrData` resolved by the holes. Required tests: (a) the cached prelude bytes are
byte-identical across two different identities; (b) the per-request tail's `auth` and
the hole content differ between those identities; (c) the cache contains exactly one
entry (the prelude) regardless of how many identities hit it.

## 6. Host/runtime integration

- `ssr-runtime.ts`: add `prerenderRoute()` and `resumeRoute()` beside the current
  render. Reuse `computeRevalidateSecs`; add `pprEnabled = mod.ppr === true`. The
  prerender-mode proxies live next to the existing `authProxy` (`:1799`). The internal
  proof becomes `x-pylon-cacheable` (static, as today) **or** `x-pylon-ppr` (carries
  the revalidate secs; host stores prelude+postponed and resumes on hit).
- `frontend.rs`: needs a **separate PPR eligibility + read path** distinct from the
  static fast path. codex P1: today `cacheable_eligible` (`:698-704`) requires `GET` +
  empty query + **`!session_cookie_present`** — so it deliberately never serves a
  cached page to an authed request. PPR is the opposite: it must serve the cached
  *shell* to authed requests and resume the holes with their real auth. So add a PPR
  path that is `GET` + Host-bucketed but **session-cookie allowed**, looks up a `Ppr`
  entry, and — instead of returning bytes — calls the runner's resume entrypoint with
  the request's auth/cookies + the cached `postponed`, then streams prelude + holes +
  per-request tail (§5.1). **A `Ppr` entry is never served Rust-only** (it always
  resumes via Bun); only `Static` entries keep the Rust-only fast path. The public
  `Cache-Control` for a PPR response is **`private, no-store`** (it's personalized —
  the caching is *inside* Pylon, not at the CDN; see §8). Strip internal proof headers
  as today (`:1828`).

## 7. Security invariants (the leak class — must be tests, not prose)

1. **Prelude anonymity.** Prerender uses the postpone-proxies, so no `auth`/`cookies`
   value can enter the prelude. Test: a page that reads `auth.userId` inside a hole →
   the cached prelude bytes contain the fallback, never the id; the id appears only in
   the per-request resume output.
2. **Holes never cached.** The resume output is streamed, never teed to `ssr_cache`.
   Test: two requests with different `auth` to the same PPR route → identical prelude,
   different hole content; cache has one entry (the prelude).
3. **Postpone-outside-Suspense fails closed.** Auth read in the synchronous shell →
   whole-doc postpone → fully-dynamic `no-store`, never a partial cache. Test.
4. **Cookie/status from a hole is dropped** (head committed with the prelude — same
   limitation as today's streaming, `ssr-runtime.ts:1906-1917`). Lint + a loud warn;
   `response.*` must run in the shell.
5. **No poisoning.** Prerender runs with a canonical host + empty auth/cookies; the
   `#347` host-bucket vary still applies. Test that `Host:` can't enter the prelude.

## 8. Honest scope / non-goals

- **PPR is an origin optimization, not generic CDN caching.** The combined response
  (cached shell + per-request holes) is personalized, so Cloudflare still sees
  `DYNAMIC`. What PPR buys: the shell renders once (not per request), TTFB is the
  prelude flush, auth stays in the layout, and there's no flash. It does **not** make
  `cf-cache-status` go `HIT` for a personalized page.
- **For a coarse, binary dynamic region (signed-in vs not — notbehind's actual
  case), the simpler tool is auth-bucketed variant caching**: render two *anonymous*
  variants keyed on session-cookie presence (via the new `props.session.exists`
  primitive that never reads/serializes identity — see Phase 0), cache both (Pylon ISR
  + a Cloudflare Cache Rule keyed on the cookie). That gets a true CDN `HIT`, no flash,
  no PPR machinery — but only works when the authed variant is user-agnostic (no
  username/avatar). PPR is the general tool for when the personalized region is richer
  than a bucket. The two compose (a route can be variant-cached *and* internally PPR'd).
- Not RSC. No server→client Flight payload; this is react-dom prerender/resume only.

## 9. Backward compatibility & fallbacks

- Opt-in only (`ppr = true`). Every existing page is byte-identical.
- If the resolved React lacks `prerender`/`resume` → log once, treat `ppr` as a no-op
  (page renders as today). Never 500 on a missing primitive.
- A prerender that postpones at root → fully-dynamic fallback (no shell), same as a
  non-PPR auth page today.
- A resume failure → re-render the route live (full render), never serve a broken
  shell. The ISR entry is best-effort exactly like §`ssr_cache.rs` today.

## 10. Testing

Pure/unit (mirror the existing `computeCacheVerdict` style): the §7 leak-class tests;
prerender→postpone classification; `Static` vs `Ppr` entry selection; resume merges
prelude+holes in order. Integration (real Bun runner + tiny_http, like the dev-live
SSE test): PPR route → first request prerenders+resumes, second serves cached prelude
+ fresh holes; two identities → shared prelude, distinct holes; auth-in-shell →
no-store. Each non-vacuous (assert it fails without the split).

## 11. Phased rollout

1. **Phase 0 — auth-bucketed variant caching** (ships the notbehind win cheaply;
   no renderer change): add a `cookie-presence` vary bucket + opt-in `export const
   cache = "auth-bucketed"`. **codex P2:** this can NOT be built on `props.auth` — any
   `auth.user_id` read flips `authTouched` and vetoes the cache (`ssr-runtime.ts:1799,
   1497`). It needs a NEW coarse primitive, e.g. `props.session.exists` (a boolean
   derived purely from session-cookie presence) that does **not** resolve or serialize
   identity and does **not** trip `authTouched`. The page reads that for the binary nav;
   the cache keys on the same bucket. Only variants that never read real `auth` are
   cacheable. Solves binary-nav pages with a true CDN `HIT`. *Recommended first step.*
2. **Phase 1 — prerender/resume plumbing**: `ssr_cache.rs` postponed storage; runtime
   `prerenderRoute`/`resumeRoute`; the §7 tests. Behind `ppr = true`, dev-only.
3. **Phase 2 — host resume path** in `frontend.rs`; `<Dynamic>` sugar + lint; enable
   on one real route (notbehind) and verify TTFB + no-flash + anonymity in a browser.
4. **Phase 3 — docs/skill**: the boundary contract, the PPR-vs-variant decision, the
   "don't read auth in the shell" rule.

## 12. Open questions

- **O1 — RESOLVED ✅ (probed react-dom 19.2.7, 2026-06-28).** `prerender` is exported
  from `react-dom/static` (+ `.browser`/`.edge`); `resume` is exported from
  `react-dom/server.browser` and `.edge` — the exact Web-streams entry points Pylon
  uses for `renderToReadableStream`. So PPR is buildable on the pinned React. Caveat:
  `React.unstable_postpone` is **undefined** on stable 19.2.7, so holes MUST be created
  via the suspend-during-prerender mechanism (§3), never `unstable_postpone`. The first
  draft's `resumeToReadableStream` name was wrong (it's `resume`). Re-probe if the React
  floor changes.
- **O2**: do we auto-wrap a known auth-nav slot in `<Suspense>`, or require the dev to?
  (Auto is seamless but magic; explicit is predictable.)
- **O3**: resume cost per request vs. just rendering live for cheap shells — measure
  the crossover; PPR may not pay off for tiny pages.
- **O4**: interaction with `serverData`/`use()` already using Suspense — make sure a
  data boundary and an auth hole compose without double-postpone.
```
