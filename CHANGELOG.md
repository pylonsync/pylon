# Changelog

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
