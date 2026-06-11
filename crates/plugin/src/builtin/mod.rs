//! Builtin plugin modules. Every module here is actually wired into
//! the runtime — modules that weren't wired anywhere have been
//! deleted as dead code. What lives elsewhere:
//!
//!   - File storage      → `pylon_storage::files` (LocalFileStorage,
//!                          Stack0FileStorage; FileOpsAdapter in
//!                          datastore.rs exposes it through the
//!                          router's FileOps trait)
//!   - Full-text search  → `pylon_storage::{search, pg_search,
//!                          search_query, search_maintenance}` —
//!                          FTS5 + roaring-bitmap facets natively
//!   - Email transport   → `pylon_auth::email::HttpEmailTransport` +
//!                          `EmailAdapter` in datastore.rs (SendGrid,
//!                          Resend, Stack0, generic webhook)
//!   - Audit log         → `pylon_auth::audit` + `audit_backend` in
//!                          runtime + `/api/auth/audit{,/tenant}`
//!                          routes
//!   - API keys / TOTP / orgs / sessions — `pylon_auth::*` +
//!                          backends in runtime + `/api/auth/*` routes
//!
//! Plugin TS packages live under `packages/plugins/` for things the
//! framework binary doesn't already cover (currently stripe,
//! feature-flags, outbound webhooks).

pub mod ai_proxy;
pub mod cache;
pub mod cache_client;
pub mod csrf;
pub mod owner_stamp;
pub mod rate_limit;
pub mod tenant_scope;
