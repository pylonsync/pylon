//! Route modules. Each module owns one slice of the `/api/*` surface and
//! exposes a `handle(...) -> Option<(u16, String)>` function. The
//! top-level `route_inner` in `lib.rs` invokes them in order; the first
//! `Some(...)` short-circuits the dispatch.
//!
//! Splitting strategy mirrors the security-review groupings: routes
//! that share an auth model and a threat surface live together so a
//! reviewer can audit one file instead of scrolling through 6000+
//! lines of unrelated handlers.

pub mod auth;
