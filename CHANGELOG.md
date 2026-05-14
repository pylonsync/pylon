# Changelog

## [0.3.91](https://github.com/pylonsync/pylon/compare/v0.3.90...v0.3.91) (2026-05-13)


### Breaking

* **storage:** `POST /api/files/upload` (multipart proxy) **removed**. The old endpoint parsed the entire upload body in memory before forwarding to the storage backend — 30MB+ uploads OOM'd the multipart parser. Replaced with a 3-step direct-to-storage flow:

  1. `POST /api/files/init` → `{ uploadUrl, assetId, cdnUrl, expiresAt }`
  2. Client PUTs raw bytes to `uploadUrl` (S3 for Stack0, pylon's `/api/files/local-put/<id>` for local). Bytes never transit pylon for CDN-backed backends.
  3. `POST /api/files/confirm` with `{ assetId }` → `{ id, url, size }` and records ownership.

  The legacy endpoint returns `410 Gone` with a migration hint so old clients see a useful error instead of a 404.

* **storage:** `GET /api/files/<id>` now 302-redirects to the CDN URL for backends that have one (Stack0). Previously it proxied bytes through pylon's process even when a CDN URL was available — a 2x memory hit on every download. Local backend still streams from disk.

* **storage:** new `DELETE /api/files/<assetId>` endpoint. Owner-gated. Routes to the active backend's delete (Stack0's `DELETE /v1/cdn/assets/<id>` or local fs unlink). Pre-0.3.91 there was no way to delete a Stack0 asset from the framework.


### Migration

```ts
// Pre-0.3.91 (multipart proxy, removed):
const form = new FormData();
form.append("file", file);
await fetch("/api/files/upload", { method: "POST", body: form });

// 0.3.91+ (3-step direct-to-storage):
const init = await fetch("/api/files/init", {
  method: "POST",
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify({ filename: file.name, mimeType: file.type, size: file.size }),
}).then((r) => r.json());
await fetch(init.uploadUrl, { method: "PUT", body: file, headers: { "Content-Type": file.type } });
const stored = await fetch("/api/files/confirm", {
  method: "POST",
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify({ assetId: init.assetId }),
}).then((r) => r.json());
```

Persist both `stored.url` (display) and `stored.id` (so you can `DELETE /api/files/<id>` later).


### Closes the Stack0 rollout

Final entry in the v0.3.87 → 0.3.91 Stack0-rollout sequence. v0.3.87 wired the provider, v0.3.88 added `/v1`, v0.3.89 added `projectSlug`, v0.3.90 fixed the confirm Content-Type, v0.3.91 reshapes the API so bytes never proxy through pylon. Yapless's 10MB+ Mac recording uploads now succeed end-to-end without touching pylon's memory.

## [0.3.90](https://github.com/pylonsync/pylon/compare/v0.3.89...v0.3.90) (2026-05-13)


### Bug Fixes

* **storage/stack0:** the upload confirm step (`POST /v1/cdn/upload/<id>/confirm`) used `ureq::Request::call()` which sends no body and no `Content-Type`. Stack0 requires `Content-Type: application/json` on the confirm endpoint and returns `415 Unsupported Media Type` otherwise — uploads got their bytes onto the CDN but the asset stayed half-confirmed and `store()` returned an error to the caller. Fixed by switching to `send_json(serde_json::json!({}))`, matching Stack0's SDK behaviour.


### Closes the Stack0 rollout

Fourth and last fix in the Stack0-rollout sequence: v0.3.87 routed uploads through the provider, v0.3.88 added `/v1` to the base URL, v0.3.89 added `projectSlug` to the init body, v0.3.90 sets `Content-Type` on the confirm step. End-to-end Stack0 uploads now succeed: response includes `"url": "https://cdn.stack0.dev/..."`.

## [0.3.89](https://github.com/pylonsync/pylon/compare/v0.3.88...v0.3.89) (2026-05-13)


### Bug Fixes

* **storage/stack0:** Stack0's `/v1/cdn/upload` requires a `projectSlug` field on the request body — pylon omitted it, so every upload was rejected with `400 Bad Request` (no useful error from Stack0 about what was missing). Adds `PYLON_STACK0_PROJECT_SLUG` env var, includes the value in the upload-init body, and validates the var at server boot.
* **storage/stack0:** Boot-time validation: `PYLON_FILES_PROVIDER=stack0` now refuses to start the server when `PYLON_STACK0_API_KEY` or `PYLON_STACK0_PROJECT_SLUG` is missing. The earlier silent fallback to local storage masked the misconfiguration until end users hit upload failures.


### Migration

Apps already on Stack0 must add `PYLON_STACK0_PROJECT_SLUG=<your-slug>` to their environment alongside the existing `PYLON_STACK0_API_KEY`. Pylon will refuse to boot otherwise.

Third in the v0.3.87 → 0.3.88 → 0.3.89 Stack0-rollout sequence. v0.3.87 routed uploads through the provider, v0.3.88 fixed the missing `/v1` prefix, v0.3.89 adds the required `projectSlug`. Voice clone / avatar uploads work end-to-end from this release.

## [0.3.81](https://github.com/pylonsync/pylon/compare/v0.3.80...v0.3.81) (2026-05-12)


### Features

* **feature-flags:** new `@pylonsync/feature-flags` package (at `packages/plugins/feature-flags/`) — local-eval flags with boolean + multivariate variants, percentage rollouts, 11 targeting predicate ops, JSON payloads per variant, deterministic FNV-1a bucketing. Framework has no equivalent.
* **webhooks:** new `@pylonsync/webhooks` package (at `packages/plugins/webhooks/`) — Svix-compatible outbound webhook delivery (HMAC-SHA256 signed, replay protection, secret-rotation overlap, exponential-backoff retries). Framework has no outbound delivery.


### Project structure

* New `packages/plugins/` subdirectory for optional TS extension packages. Keeps framework core (sdk, functions, sync), client bindings (react, next, swift), and CLI tooling visually separate from opt-in plugins. The `@pylonsync/stripe` package also moved here.


### Refactoring

* **plugin:** remove orphaned Rust plugin shells from `crates/plugin/src/builtin/`. Most were type-only sketches that never got wired into the runtime or SDK. The genuinely useful ones (cache, tenant_scope, rate_limit, ai_proxy, csrf, email SMTP, file_storage, net_guard, search) stay. The truly-dead ones get TS replacements only where the framework didn't already cover the use case — see Features above. Deletes 21 files, ~5000 LOC.


### Documentation

* **plugins:** new docs pages under `/plugins/*` for stripe, organizations, feature-flags, webhooks. Each page covers install, config, manifest fragment, wrapper-file pattern, env vars, lifecycle hooks, security notes. Overview restructured to split TS packages from runtime Rust hooks.


### Withdrawn (don't use these — they duplicated framework functionality)

* ~~`@pylonsync/api-keys`~~ — Framework already provides `/api/auth/api-keys` (mint/list/revoke + bearer-auth verification). Removed.
* ~~`@pylonsync/two-factor`~~ — Framework already provides `/api/auth/totp/*` (enroll/verify/disable + backup codes). Removed.
* ~~`@pylonsync/email`~~ — Framework already supports SendGrid/Resend/Stack0/webhook via `PYLON_EMAIL_PROVIDER` env + `ctx.email.send()`. Removed.
* ~~`@pylonsync/audit-log`~~ — Framework already provides `AuditAction`/`AuditEvent` + `/api/auth/audit{,/tenant}` routes. Removed.
* ~~`@pylonsync/organizations`~~ — Framework already provides Org/OrgMember/OrgInvite as manifest entities + `/api/auth/orgs/*` routes. The plugin's "permission system" was just a thin TS wrapper over the policy DSL's existing `auth.hasRole(...)`. The "teams" feature inside it had no current consumer and will come back as a focused `@pylonsync/teams` package when one materializes. Removed.

## [0.3.80](https://github.com/pylonsync/pylon/compare/v0.3.79...v0.3.80) (2026-05-11)


### Features

* **stripe:** new `@pylonsync/stripe` package — declarative billing for Pylon apps. Replaces the ~400 lines of per-app Stripe boilerplate (customer creation, checkout, billing portal, webhook signature + event routing, plan derivation, subscription state) with one config block: `stripe({ referenceType, plans, hooks })` returns a manifest fragment + handler factories. Auto-derives URL allowlist from `PYLON_PUBLIC_URL` (closes the "hardcoded yapless.com vs getyapless.com" bug class), wires the canonical `Subscription` entity with tenant-scoped policies, fires lifecycle hooks (`onSubscriptionActivate`/`Update`/`Cancel`, `onInvoice`, `onEvent`, `onCustomerCreate`), and ships idempotent webhook upsert via `_pylonStripeUpsertSubscription`. Built-in double-trial guard, RBAC via `authorizeReference` (default: org owners + admins, or self for `referenceType: 'user'`), constant-time signature verification with replay-window check + multi-secret rotation support. 13 tests cover signature verification + URL allowlist edge cases.

## [0.3.79](https://github.com/pylonsync/pylon/compare/v0.3.78...v0.3.79) (2026-05-11)


### Bug Fixes

* **auth/oauth:** `PYLON_OAUTH_<provider>_REDIRECT` now falls back to `{PYLON_PUBLIC_URL}/api/auth/callback/<provider>` before the old `http://localhost:3000/...` default. Every production deploy that set `PYLON_PUBLIC_URL` (the typical case — Pylon Cloud sets it automatically) but forgot the per-provider `_REDIRECT` env shipped Google with `redirect_uri=http://localhost:3000/...` and got `redirect_uri_mismatch` at the IdP. Localhost only kicks in now when neither env is set (local dev). Applies to all builtins (google/github/microsoft/apple/discord/etc.) AND generic OIDC providers (Auth0/Okta/Keycloak/etc.).

## [0.3.78](https://github.com/pylonsync/pylon/compare/v0.3.77...v0.3.78) (2026-05-11)


### Features

* **runtime/logger:** Optional Tinybird request-log shipper. When `PYLON_TINYBIRD_TOKEN` + `PYLON_PROJECT_ID` are set in env, every completed HTTP request emits one NDJSON row to a configured Tinybird datasource (defaults to `request_log`). Disabled at runtime if either env is unset — zero cost for standalone deployments. Designed for Pylon Cloud's per-project Logs page: cloud sets the envs on each customer Fly machine at provision time, the shipper batches up to 100 events / 2s and POSTs to `{host}/v0/events?name=request_log`. Bounded `mpsc::sync_channel` (capacity 2048) drops on backpressure rather than blocking the request hot path. Optional envs: `PYLON_TINYBIRD_HOST` (defaults to `https://api.tinybird.co`), `PYLON_DEPLOYMENT_ID`, `PYLON_REGION` (falls back to `FLY_REGION`), `PYLON_TINYBIRD_DATASOURCE` (defaults to `request_log`).

## [0.3.77](https://github.com/pylonsync/pylon/compare/v0.3.76...v0.3.77) (2026-05-11)


### Bug Fixes

* **runtime/jobs:** `JobQueue::restore_from` now also re-populates the in-memory dead-letter queue from disk on startup, not just pending/running/retrying. Dead-letter rows were always persisted by `try_enqueue_job` → `JobStore::save` (the SQLite table has them with `status='dead'`), but `restore_from` skipped them, so `/api/jobs/dead` returned `[]` after every restart even though the rows were still on disk. Operators lost visibility into failed jobs at the exact moment they most needed it (right after a deploy that may have caused the failures). Fix re-pushes dead rows into the in-memory VecDeque in chronological order and threads them into the next-id calculation so a restored `job_999` doesn't get clobbered by a fresh enqueue. Regression test covers the round-trip.

## [0.3.76](https://github.com/pylonsync/pylon/compare/v0.3.75...v0.3.76) (2026-05-11)


### Bug Fixes

* **runtime/scheduler:** P0 — `ctx.scheduler.runAfter` / `runAt` now propagates the scheduling caller's identity (user_id, is_admin, tenant_id) to the dispatched job instead of running every scheduled callback with anonymous auth. Apps whose internal mutations reject anonymous direct callers (a common defense against HTTP smuggling) silently failed every scheduled handler — pylon-cloud's `provisionMachine` hit this on first cloud-side create, dead-lettering after 3 retries on `UNAUTHENTICATED`. New `JobAuth` field on `Job` carries the identity through the in-memory queue + the SQLite job store (schema migrated on open; existing rows deserialize as `auth = None` → anonymous, matching pre-fix behavior). Framework cron jobs (`pylon.cache.cleanup`, `pylon.rooms.cleanup`) and dead-letter retries keep the anonymous default. The schedule-time smuggle gate (added in 0.3.73) already enforces chain-of-custody — only admin/internal callers can enqueue internal:true targets — so admin-scheduled jobs running as admin is consistent, not a new escalation surface. Two regression tests cover the round-trip + the back-compat anonymous default.

## [0.3.75](https://github.com/pylonsync/pylon/compare/v0.3.74...v0.3.75) (2026-05-11)


### Features

* **sdk:** `auth({ org: { entity, memberEntity, inviteEntity, disabled } })` config now flows through the TS SDK to the manifest. Rust side accepted it in v0.3.74 but the SDK helper hadn't been updated — apps had to hand-write the snake_case JSON. Now `auth({ org: { disabled: true } })` reads cleanly in TS for apps that want to keep their own org flow without the framework's routes adding parallel write paths. Same camelCase → snake_case translation pattern as the rest of `auth({...})`.

## [0.3.74](https://github.com/pylonsync/pylon/compare/v0.3.73...v0.3.74) (2026-05-11)


### Breaking changes

* **auth/org:** Org / OrgMember / OrgInvite are now **manifest entities** instead of hardcoded Rust structs. Apps that use the framework's `/api/auth/orgs/*` surface must declare `Org`, `OrgMember`, and `OrgInvite` entities in their schema with the required fields (see docs). The previous SQLite + Postgres `OrgBackend` impls are deleted — org / member / invite data now flows through the same DataStore as every other entity. **Migration**: pylon-cloud and other apps with their own `Organization` / `OrgMember` entities point the framework at their names via `auth.org.{ entity, member_entity, invite_entity }` in the manifest. Apps that want to keep custom flow can set `auth.org.disabled = true` and the framework's routes return 501. No data migration tool ships — if you have org data in the old SQLite/PG tables, write a one-time copy into your new entities before upgrading.


### Features

* **auth/org:** Apps customize the Org / OrgMember / OrgInvite schema freely. Add `logo`, `industry`, `billingEmail`, `plan`, `title`, `department`, anything else — the framework reads only the required fields it manages and leaves the rest alone. Same `/api/auth/orgs/*` HTTP surface; the underlying schema is now app-owned.

## [0.3.73](https://github.com/pylonsync/pylon/compare/v0.3.72...v0.3.73) (2026-05-10)


### Bug Fixes

* **router,plugin:** P1 — `POST /api/crdt/<entity>/<row>` now (a) requires the materialized row to exist before merging the LoroDoc patch (returns 404 `ROW_NOT_FOUND` instead of accumulating CRDT sidecar state for phantom rows) and (b) runs the plugin chain's `before_update` / `after_update` hooks so TenantScopePlugin / audit_log / validation observe the write. The v0.3.70 `HookEnforcingDataStore` fix wrapped `ctx.db.update` but not this binary CRDT path, leaving relation mutations and audit trails to silently skip the chain. Caught in the 2026-05-10 codex pass-3 audit.
* **auth,runtime:** P3 — WS handshake now uses the same bearer-resolution chain HTTP does (admin token / API key / JWT / session). New shared helper `pylon_auth::resolve_bearer_token`. New `pylon_runtime::ws::WsAuth` bundle carries the four pieces (sessions, API-key store, admin token, JWT secret + issuer) through the WS upgrade. Before this fix the WS subprotocol auth resolved ONLY session tokens — admin tokens / API keys / JWT bearers that worked over HTTP silently failed over WS, and revocation / admin-promotion semantics diverged. Five regression tests on the shared resolver cover admin / wrong-admin / bad-API-key / none / JWT-misconfigured paths. Caught in the 2026-05-10 codex pass-3 audit.

## [0.3.72](https://github.com/pylonsync/pylon/compare/v0.3.71...v0.3.72) (2026-05-10)


### Bug Fixes

* **runtime,policy:** P0 — WS + SSE change-event broadcasts now run a per-client tenant filter. Each connected client's `AuthContext` is captured at registration time and stored on the shard's client entry; before the shard worker forwards a change event, it calls `policy.check_entity_read(entity, &client.auth, &row)` and skips clients that fail. Without this filter every connected client received every change event from every tenant — a cross-tenant data leak via subscription. New types `ws::WsClient` (carries auth) + `ws::BroadcastJob::Change` / `BroadcastJob::Plain` enum + `sse::SseClient` (carries auth) + `sse::SseJob`. Both hubs now require an `Arc<PolicyEngine>` at construction. `WsHub::new(policy)` and `SseHub::new(policy)` are breaking-API in their signatures (no in-tree callers outside the runtime + benches; bench/test sites updated). Caught in the 2026-05-10 codex pass-3 audit.
* **runtime:** P0 (cont.) — dedicated SSE port now authenticates incoming connections by parsing the initial HTTP request for `Authorization: Bearer`, the framework session cookie (`PYLON_COOKIE_NAME` / default `pylon_session`), or `?token=` query param. Resolves through the existing `SessionStore`. Rejects unauthenticated callers with `401 AUTH_REQUIRED` outside dev mode unless the operator opts in via `PYLON_SSE_PORT_ACKNOWLEDGE_UNAUTH=1`. Browsers using `EventSource` carry the session cookie automatically on same-origin; native clients use bearer or `?token=`.
* **functions:** P1 — `ctx.scheduler.runAfter / runAt` now refuses to enqueue an `internal:true` target when the calling function is itself public (non-internal) AND the caller's auth is non-admin. Without this gate any public action that exposed scheduler parameters became an internal-fn smuggling proxy: a non-admin caller could `runAfter(0, "rollupUsage", {...})` to run an internal cron with arbitrary args, and the dispatched job would execute under anonymous auth context. New `ScheduleCallerInfo { caller_internal, caller_is_admin }` flows through the schedule hook. `FnRunner::call_with_caller_internal` exposes the propagation; `FnOpsImpl::call` wires `def.internal` at every top-level + nested mutation path. Internal cron self-perpetuation (`internal -> internal scheduling`) and admin contexts continue to work. Caught in the 2026-05-10 codex pass-3 audit.

### Deferred to 0.3.73

* **router (P1):** `/api/crdt/<entity>/<row>` binary CRDT update path bypasses plugin hooks and doesn't verify row tenant before merging the LoroDoc patch. Real gap but requires a logged-in caller crafting binary CRDT update messages with another tenant's row id.
* **runtime (P3):** WS bearer-subprotocol auth resolves only session tokens, while HTTP bearer additionally accepts admin tokens / API keys / JWT. Revocation + admin-promotion semantics diverge. Needs a shared resolver across the WS upgrade + HTTP routes.

## [0.3.71](https://github.com/pylonsync/pylon/compare/v0.3.70...v0.3.71) (2026-05-10)


### Bug Fixes

* **runtime,plugin:** P2 — `HookEnforcingDataStore.link/unlink` now runs the plugin chain (TenantScopePlugin, audit_log, validation) instead of forwarding straight to the inner store. Previously a TS-mutation `ctx.db.link("Doc", id, "tenantOwner", "other-tenant")` skipped the chain entirely, undoing v0.3.70's "every TS mutation path" claim for relation mutations. SQLite `TxStore.link/unlink` also now records sync change events so subscribers see relation flips, matching the Postgres path. Caught in the 2026-05-10 codex pass-3 audit.
* **router:** P2 — `/api/ai/stream` now per-user rate limited (default 30/hour, configurable via `PYLON_AI_RATE_LIMIT_MAX` / `PYLON_AI_RATE_LIMIT_WINDOW`) and refuses client model overrides unless the operator opts in via `PYLON_AI_MODELS_ALLOWED` (comma-separated allowlist). Without these gates a logged-in user could request the most expensive model the provider key supports and burn shared spend. Admins skip both gates so internal tooling isn't blocked. Caught in the 2026-05-10 codex pass-3 audit.
* **router:** P3 — `GET /api/openapi.json` now requires admin/dev-mode. The full spec includes batch/transact + entity surface, useful reconnaissance for attackers probing app shape. Public clients use the typed SDK and don't need it. Returns 404 outside dev for non-admin callers (rather than 403, to avoid confirming the route exists). Caught in the 2026-05-10 codex pass-3 audit.
* **runtime:** P0 partial mitigation — the dedicated SSE port (port+2) currently broadcasts every change event to every connected client with no auth and no per-client tenant filter. Pylon Cloud does NOT expose this port, so the cloud surface is unaffected, but self-hosted deploys that bind it on a public interface leak cross-tenant row data. Stop-gap added: loud boot-time warning, plus `PYLON_SSE_PORT_DISABLE=1` kill switch for deploys that don't need the SSE fallback transport. Full per-client tenant filter + cookie/bearer auth gate ships in v0.3.72. Caught in the 2026-05-10 codex pass-3 audit.

## [0.3.70](https://github.com/pylonsync/pylon/compare/v0.3.69...v0.3.70) (2026-05-10)


### Bug Fixes

* **runtime,plugin:** P1 — `ctx.db.insert/update/delete` from a TS function handler now fires the same plugin chain (TenantScopePlugin, validation, audit_log, webhooks, ...) as the entity-API path. Previously TS-mutation writes bypassed the chain entirely, so a non-admin caller could pass `{tenantId: "other"}` directly through `ctx.db.insert("Doc", {...})` and plant rows in another tenant's space, even though TenantScopePlugin's `before_insert` was correctly rejecting the same payload on `POST /api/entities/Doc`. New `HookEnforcingDataStore` wrapper installed across both SQLite (`TxStore`) and Postgres (`PgBufferedTxStore`) mutation paths, plus the two nested-mutation paths (`action -> ctx.runMutation`). Three regression tests prove the gate fires: cross-tenant insert rejected, same-tenant insert stamped, admin override allowed. Caught in the 2026-05-10 codex pass-2 audit.
* **runtime:** P2 — SSE fast path `/api/fn/:name` for `Accept: text/event-stream` callers now returns `404 FN_NOT_FOUND` for internal-and-non-admin requests (matching the router's non-streaming path) instead of `403 FN_INTERNAL`, which leaked which functions exist by name. Also switched anonymous rate-limit identity from a global `"anon"` bucket to the per-peer-IP key the router uses, so one bad actor can't lock everyone else out of the SSE endpoint. Caught in the 2026-05-10 codex pass-2 audit.
* **release:** regenerate `bun.lock` so workspace versions track the published package.json versions. Stale lockfile entries from 0.3.63 caused `bun publish` to rewrite `workspace:*` deps to `0.3.63` instead of the current version — `@pylonsync/react@0.3.64` through `@pylonsync/react@0.3.69` all shipped with frozen `@pylonsync/sdk: 0.3.63` + `@pylonsync/sync: 0.3.63` nested deps. Apps that pulled `@pylonsync/react` at any of those versions got the SDK at 0.3.63 nested under react, missing every SDK fix from 0.3.64 forward (Files API owner gate, PYLON_SECRET fail-loud, advisoryLock primitive). The 0.3.70 publish ships react/next/react-native with correctly-rewritten 0.3.70 internal deps. Caught in the 2026-05-10 codex pass-2 audit.

## [0.3.69](https://github.com/pylonsync/pylon/compare/v0.3.68...v0.3.69) (2026-05-10)


### Features

* **functions:** new `ctx.db.advisoryLock(key: string): Promise<void>` primitive for closing TOCTOU windows on quota / uniqueness checks. Backed by `pg_advisory_xact_lock` on Postgres (held until the mutation tx commits or rolls back); noop on SQLite where writers are already serialized at the connection level. Application code can write `await ctx.db.advisoryLock(\`org_count:\${userId}\`); /* count + insert */` and trust that two concurrent mutations on the same key serialize without manual transaction-isolation tuning. Wired through DataStore trait → PgTxStore → PgBufferedTxStore → DbOp protocol → TS DbWriter interface. Three regression tests on the key-pair hash (deterministic, distinct keys hash differently, empty key doesn't panic). Caught in the 2026-05-09 codex security audit.

## [0.3.68](https://github.com/pylonsync/pylon/compare/v0.3.67...v0.3.68) (2026-05-09)


### Bug Fixes

* **router,storage:** P1 — `GET /api/files/:id` now requires the requester to be the file's owner (or `is_admin`). Previously any authenticated identity could fetch any file by id; file IDs are timestamp-based so they're enumerable. New uploads stamp a `FileOwner { user_id, tenant_id }` sidecar at upload time; the router's GET handler returns 404 for non-owner / non-admin reads (404 not 403, to avoid leaking which IDs exist in another user's space). Files predating this change have no sidecar and remain readable as a one-release transition (logged as legacy on every read). Stack0 backend opts out via `requires_owner_check() = false` since CDN-level auth handles access there. New regression tests: `rejects_other_users_file_id`, `allows_owner_and_admin`, `legacy_file_with_no_owner_is_readable` in `pylon-runtime`, plus `local_owner_round_trip` and `local_owner_rejects_traversal` in `pylon-storage`.
* **auth:** P2 — `seal_secret` no longer silently downgrades to `plain:` envelopes when `PYLON_SECRET` (or the legacy `PYLON_SSO_ENCRYPTION_KEY` alias) is set but unparseable. The previous `sso_encryption_key()` returned `None` for both "unset" and "set-but-malformed" — operators believed encryption was on while their values were persisted in cleartext. The new public `resolve_sso_encryption_key() -> Result<Option<[u8; 32]>, String>` distinguishes the two cases; `seal_secret` / `unseal_secret` propagate the parse error; the runtime panics at boot if PYLON_SECRET is set but invalid (refuses to start with a misconfigured key). Six regression tests added covering unset, empty, malformed, valid hex round-trip, legacy-alias errors, and PYLON_SECRET-takes-precedence ordering.

## [0.3.67](https://github.com/pylonsync/pylon/compare/v0.3.66...v0.3.67) (2026-05-09)


### Bug Fixes

* **auth:** P0 — `auth.user.adminField` and `PYLON_ADMIN_EMAILS` no longer promote API-key contexts to `is_admin`. A leaked or scoped `pk.*` token for an admin-allowlisted user previously escalated to full admin on every privileged route. Caught in the 2026-05-09 codex security audit.
* **auth:** P0 — `POST /api/auth/native-session` rejects API-key-authed callers (403 `API_KEY_AUTH_FORBIDDEN`). The endpoint mints a real SessionStore entry that bypasses session-only routes; allowing API-key auth to mint sessions was a privilege-escalation path. Regression test added.
* **runtime:** P1 — streaming `/api/fn/:name` (SSE fast path) now enforces `def.internal && !is_admin` like the non-streaming router path. The fast path previously bypassed the gate, letting any caller invoke an internal function by setting `Accept: text/event-stream`.
* **plugin:** P1 — `TenantScopePlugin` now overrides `before_insert` (was a no-op default) so registering the plugin actually stamps + validates `tenantId` on every insert. Non-admin callers can no longer override the tenant id to plant rows in another tenant's space; admin contexts retain explicit-override capability for migration tooling. Two regression tests added.


### Features

* **auth:** `POST /api/auth/native-session` for desktop / native-app handoff. Cookie-session-gated; mints a real SessionStore token that works on both HTTP `Authorization: Bearer <token>` AND the WebSocket `bearer.<token>` subprotocol. Closes the gap where JWTs from `/api/auth/jwt` passed HTTP but the SyncEngine's WS reconnect-looped with `unauthorized: bearer token required`. No admin token needed — the cookie itself proves identity, and we mint a session for the same user only (no privilege escalation). New canonical endpoint for the `/auth/desktop` redirect pattern.
* **docs:** add `docs/auth/sessions.md` covering when to use `/api/auth/native-session` vs `/api/auth/jwt`, the desktop-handoff flow, and SDK guidance.

## [0.3.65](https://github.com/pylonsync/pylon/compare/v0.3.64...v0.3.65) (2026-05-09)


### Features

* **auth:** rename `PYLON_SSO_ENCRYPTION_KEY` → `PYLON_SECRET`. Matches the `<framework>_SECRET` convention used by better-auth, NextAuth, Auth.js. The old name remains a legacy alias — existing deployments don't need to update env vars to upgrade. Boot-time warning + doc references now use the new name.

## [0.3.64](https://github.com/pylonsync/pylon/compare/v0.3.63...v0.3.64) (2026-05-09)


### Features

* **auth:** PYLON_ADMIN_EMAILS env-var allowlist for declarative admin promotion. Comma-separated list of verified email addresses; matched users get auth.is_admin lifted on every request. When `auth.user.adminField` is configured, the User row's flag is persisted on first match so removing the email from the env doesn't demote — revoke explicitly.


### Bug Fixes

* **storage:** plan_from_snapshot now detects index shape drift (columns / unique / partial-WHERE predicate) and emits RemoveIndex + AddIndex to recreate. Auto-heals indexes created by older Pylon binaries that silently dropped a WHERE clause.
* **storage:** plan_from_snapshot now drops indexes that were removed from the manifest. The previous version only iterated over manifest indexes, so dropping an index from schema left it in the DB forever.
* **storage:** Postgres RemoveIndex SQL now correctly prefixes index names with `<entity>_` to match AddIndex's namespace. Previously emitted bare logical names that no-op'd against the prefixed DB names.
* **storage:** SQLite gains a RemoveIndex handler (was previously rejected as `SQLITE_OP_UNSUPPORTED`).
* **runtime:** WS / SSE / RESP / shard-WS / cache-server bind sites now use dual-stack `[::]:port` with v4 fallback. Fixes macOS clients connecting to `localhost` over IPv6 (`::1`) hitting connection-refused on v4-only listeners.

## [0.2.0](https://github.com/pylonsync/pylon/compare/v0.1.0...v0.2.0) (2026-04-24)


### Features

* **cli:** add 'pylon start' — production server command ([a39483e](https://github.com/pylonsync/pylon/commit/a39483e1e8dc7bd96fecd9ff8417cc0f7384e24b))
* **cli:** auto-restart pylon dev on functions/ changes via self-exec ([0247568](https://github.com/pylonsync/pylon/commit/024756853e7660df4bcbee0364bb390e72644fb2))


### Bug Fixes

* **docker:** broken [@pylonsync](https://github.com/pylonsync) symlinks + stale [@pylon](https://github.com/pylon) runtime lookup ([21644f5](https://github.com/pylonsync/pylon/commit/21644f5da25bfae03dcaec3669f5af15637b1767))
* **fly:** drop PYLON_DEV_MODE=false override that fought the Dockerfile default ([6ad04ea](https://github.com/pylonsync/pylon/commit/6ad04ea3bcdf1a2befa484da3faa3bf94ab90b32))
* remove pre-rebrand APIs (AgentDBProvider, @pylon/*, v.money, shard(), etc.) from marketing site; fix pylon-plugin api_keys stale prefix-length test ([7cd4d45](https://github.com/pylonsync/pylon/commit/7cd4d458d43391ea8e63b2863dff5e54f34bdf28))
* **studio:** derive base URL from request Host/X-Forwarded-Proto instead of hardcoded localhost ([704559c](https://github.com/pylonsync/pylon/commit/704559c7506733523083d6e780da306b36738513))
