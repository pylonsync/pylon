# Changelog

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
