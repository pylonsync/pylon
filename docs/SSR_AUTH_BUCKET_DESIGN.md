# SSR auth-bucketed caching (PPR Phase 0) — design

Status: proposal · 2026-06-28 · builds on the cache-path hardening in v0.3.298

## Goal

Make the dominant pattern — a public page with a **binary** auth-aware nav
("Sign in" vs "Dashboard") — genuinely CDN-cacheable, with **no flash** and
without forcing the dev to rip auth out of the layout. This is the cheap win that
solves the notbehind case; full PPR (arbitrary per-user holes) is the later,
heavier tool.

The idea: cache **two anonymous variants** keyed on session-cookie *presence*
(not identity) — a "signed-out" shell and a "signed-in" shell — both of which
depend only on the coarse bucket, never on *which* user. Each variant is correct
for its bucket, so there's no flash, and both are shareable within their bucket.

## Why this is safe ONLY with three coupled changes

The v0.3.298 hardening made one invariant load-bearing: **a render that reads the
real `auth` (or headers/cookies) is not shareable, and the hydration tail carries
the request's real identity.** Auth-bucketed caching deliberately caches a
*signed-in* response and replays it to *other* signed-in users — so it is correct
**iff** the cached bytes contain only the bucket, never a specific identity. That
requires all three of:

1. **A coarse `props.session.exists` primitive** that exposes signed-in-or-not
   WITHOUT reading or serializing identity — so a page can render the binary nav
   without tripping `authTouched`/`dynamicTouched`.
2. **A session-presence cache bucket** so the two variants are stored separately.
3. **Tail anonymization for bucketed renders** — the `__PYLON_DATA__` hydration
   blob must serialize an identity-stripped auth (just `{ signedIn: bool }`),
   NEVER `user_id`/`tenant_id`/`roles`. (This is the item the PPR doc flagged as
   "PPR-era"; auth-bucketing needs it too.)

Miss any one → a signed-in user's identity is cached and replayed to another
signed-in user. So this ships as one reviewed unit, not piecemeal.

## The pieces (grounded in current code)

### 1. `props.session.exists`
- Rust (`frontend.rs`): `session_cookie_present(cfg, &request)` already computes
  it for the eligibility gate. Thread it into the SSR render message
  (`serve_via_ssr_rpc` → the runner `msg`) as `session_present: bool`.
- TS (`ssr-runtime.ts`): add `props.session = { exists: msg.session_present }` — a
  plain boolean, NOT wrapped in a touch-proxy and NOT derived from `auth`, so
  reading it does not set `authTouched`/`dynamicTouched`.
- Contract: `session.exists` is a *presence* bit, not auth. It says "a session
  cookie is present", not "valid" and not "who". A present-but-invalid cookie
  reads `true` and renders the signed-in shell whose client JS then resolves the
  real (anonymous) session — acceptable, fail-safe direction, no identity leaked.

### 2. Session-presence cache bucket
- `ssr_cache.rs::cache_key` already takes a `vary` slice; `ssr_cache_vary`
  (`frontend.rs`) currently supplies only the Host bucket. Add a second vary
  tuple `("sess", "1"|"0")` from `session_cookie_present`. Two entries per route.
- Read + write paths both key on it, so a signed-in request can only ever hit /
  populate the signed-in entry.

### 3. Opt-in + verdict
- `export const cache = "auth-bucketed"` on the page (distinct from
  `revalidate`; may compose with it for the TTL).
- New `computeBucketVerdict`: cacheable-bucketed iff opted in, status 200, no
  Set-Cookie, `!forceDynamic`, `!wantsStream`, and **`!authTouched` &&
  `!dynamicTouched`** (reading real auth/headers/cookies still vetoes — only
  `session.exists` is permitted). The existing `computeCacheVerdict` invariants
  carry over unchanged.

### 4. Eligibility (`frontend.rs:698`)
- Today `cacheable_eligible` requires `!session_cookie_present`. Add a parallel
  **bucket-eligible** path: GET + no query + Host-bucketed, **session cookie
  ALLOWED**, that reads/writes the session-keyed entry. A bucketed entry is still
  served from Rust (no Bun) on a hit — unlike PPR, no per-request resume.
- `build_ssr_response_headers`: a bucketed response is shareable *within its
  bucket*. The public Cache-Control must therefore carry `Vary: Cookie` (or a
  Cloudflare cache-key rule on the session cookie) so the CDN buckets too — and
  MUST still be gated by the v0.3.298 `may_share`/`is_non_shared_cc` invariant.
  (Open question O-CDN below.)

### 5. Tail anonymization
- `buildHydrationTail` (`ssr-runtime.ts:~1140`) currently keeps `auth` in
  `restProps`. For a bucketed render, replace `props.auth` in the serialized
  payload with `{ signedIn: session.exists }` — no `user_id`/`tenant`/`roles`.
  The client then resolves the real session client-side (it already has the
  cookie) without a flash (the shell already shows the correct binary state).

## Security invariants (must be tests)
1. A bucketed cache entry's bytes contain NO `user_id`/`tenant_id`/`roles` —
   for ANY signed-in user. Two different signed-in identities → byte-identical
   signed-in entry.
2. Reading real `auth`/headers/cookies in a bucketed page → veto (falls back to
   per-request, never cached). Only `session.exists` is allowed.
3. The signed-in entry is never served to a signed-out request and vice-versa
   (bucket key on both read and write).
4. A present-but-invalid session cookie never causes a real identity to be
   cached (it renders the signed-in *shell*; identity is resolved client-side).

## Open questions
- **O-CDN**: `Vary: Cookie` is too coarse for a generic CDN (varies on the whole
  cookie value → no sharing). Cloudflare needs a custom cache key on a *normalized
  cookie-presence*, which is dashboard config, not origin. Decide: ship Pylon-ISR
  bucketing (origin-fast) now + document the Cloudflare cache-key rule, vs. emit a
  normalized `Vary` the CDN can use. Pylon ISR bucketing alone is the safe MVP.
- **O-INVALID**: should `session.exists` reflect cookie *presence* (cheap, chosen)
  or *validity* (requires a session-store lookup → cost + an identity read that
  would taint)? Presence is the right call; document the "invalid cookie → signed-
  in shell, client corrects" behavior.
- **O-OPTIN**: is `cache = "auth-bucketed"` the right surface, or fold into
  `revalidate` + an auto-detected `session.exists` read?

## Rollout
1. `props.session.exists` plumbing + test (no caching behavior yet).
2. Tail anonymization for bucketed renders + invariant tests (the leak class).
3. Bucket vary + bucket verdict + eligibility; Pylon-ISR bucketing only.
4. codex adversarial review (the leak class) BEFORE enabling on any app.
5. CDN cache-key rule (Cloudflare) + browser-verify on notbehind.
6. Docs/skill: the `session.exists` pattern + when to use bucketing vs PPR.

## Implementation status (2026-06-28)
Steps 1–4 SHIPPED (unreleased, default-OFF). Resolved open questions:
- **O-INVALID resolved the OTHER way**: `session.exists` reflects RESOLVED auth
  (`auth.user_id.is_some()`), not raw cookie presence — an expired/invalid cookie
  maps to the signed-OUT bucket. The read path resolves only for cookie-bearing
  requests (`session_authenticated`), so the cost is a local store lookup only
  when a cookie is present. This is what makes read-key and write-key agree.
- **O-CDN**: Pylon-ISR bucketing ships first (origin render-skip). A bucket
  response is browser-`private`/`no-store` by default; it advertises
  `public, s-maxage` ONLY when `PYLON_SSR_BUCKET_CDN=1` tells the host the CDN
  keys its cache on session-cookie presence (a Cloudflare cache-key rule). Flag
  default OFF → no shared cache can mis-serve across buckets before the rule is in.

### Security review (codex, 3 rounds — each found real holes, all fixed)
- Signed-in request must not write the SHARED anon lane (`cache_write_plan` gates
  the anon proof on `cacheable_eligible`; the bucket lane is identity-free).
- The hydration tail of a bucketed render serializes a strict ALLOWLIST
  (url/params/searchParams + `{ signedIn }`), from an immutable pre-render
  snapshot (`bucketTailBase`) — a page can't smuggle identity via a top-level or
  nested prop mutation. Per-request proxies are revocable + revoked after render.
- A bucket render's shareability is governed only by `bucket_shareable`, so a
  page-set `public` Cache-Control can't escape the bucket policy.

### Purity contract (the residual, documented)
An `auth-bucketed` page MUST be a pure function of its props. It MUST NOT stash
request-derived data (auth/cookies/headers, or values computed from them) in
module/global scope and render it on a later request. This is the same constraint
every SSR framework draws (Next.js: "don't put request data in module scope");
it is undefendable by the framework — and a page that renders another request's
data already has an app-level IDOR independent of caching. The framework catches
every *prop-mediated* vector; module-global stashing is the author's contract.
