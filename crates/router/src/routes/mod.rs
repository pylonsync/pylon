//! Route modules. Each module owns one slice of the `/api/*` surface and
//! exposes a `handle(...) -> Option<(u16, String)>` function. The
//! top-level `route_inner` in `lib.rs` invokes them in order; the first
//! `Some(...)` short-circuits the dispatch.
//!
//! Splitting strategy mirrors the security-review groupings: routes
//! that share an auth model and a threat surface live together so a
//! reviewer can audit one file instead of scrolling through 6000+
//! lines of unrelated handlers.

pub mod actions;
pub mod admin_data;
pub mod ai;
// Auth + auth_admin pull in pylon_auth modules (saml, captcha,
// oidc_provider, etc.) that are wasm-gated because their deps
// (samael, openssl, ureq, ring) don't cross-compile to wasm32.
// The Workers target (pylon-workers) compiles without these
// routes; OAuth/SAML/SCIM endpoints aren't reachable on Workers
// for that reason — documented in pylon-workers/src/handler.rs.
#[cfg(not(target_arch = "wasm32"))]
pub mod auth;
#[cfg(not(target_arch = "wasm32"))]
pub mod auth_admin;
pub mod crdt;
pub mod entities;
pub mod files;
pub mod functions;
pub mod infra;
pub mod links;
pub mod ops_admin;
pub mod queries;
pub mod rooms;
pub mod search;
pub mod shards;
pub mod sync;
pub mod vector;
pub mod workflows;
