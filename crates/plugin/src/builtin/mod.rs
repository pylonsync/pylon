//! Builtin plugin modules — only the ones actually wired into the
//! runtime survive here. The dead plugin modules in this directory
//! were design sketches that never got connected to the SDK or the
//! runtime's plugin registry. They've moved to TS packages that
//! ship the same surface as declarative config:
//!
//!   - `stripe`            → `@pylonsync/stripe` (declarative billing)
//!   - `organizations`     → `@pylonsync/organizations` (perms + teams)
//!   - `totp`              → `@pylonsync/two-factor`
//!   - `api_keys`          → `@pylonsync/api-keys`
//!   - `audit_log`         → `@pylonsync/audit-log`
//!   - `feature_flags`     → `@pylonsync/feature-flags`
//!   - `webhooks`          → `@pylonsync/webhooks` (outbound delivery)
//!
//! What stays as Rust plugins (each is actually wired into the
//! runtime or auth layer):
//!
//!   - `cache` + `cache_client`  — wired by the cache subsystem
//!   - `tenant_scope`            — auto-registered for tenantId entities
//!   - `rate_limit`              — auto-registered with prod/dev limits
//!   - `ai_proxy`                — server.rs registers when configured
//!   - `csrf`                    — server.rs registers from env
//!   - `email`                   — server.rs wires the SMTP path
//!   - `file_storage`            — used by runtime tests + apps
//!   - `search`                  — used by storage/runtime search

pub mod ai_proxy;
pub mod cache;
pub mod cache_client;
pub mod csrf;
pub mod email;
pub mod file_storage;
pub mod net_guard;
pub mod rate_limit;
pub mod search;
pub mod tenant_scope;
