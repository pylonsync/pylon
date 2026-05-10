# Changelog

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
