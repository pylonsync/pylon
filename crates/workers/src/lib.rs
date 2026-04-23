//! Cloudflare Workers adapter for pylon.
//!
//! # Architecture
//!
//! ```text
//!   Browser ──► Cloudflare Worker
//!                 ├─ pylon_router::route()   — platform-agnostic routing
//!                 ├─ D1DataStore              — D1 SQL execution (SQLite)
//!                 └─ Durable Object rooms      — WebSocket (future)
//! ```
//!
//! # Build (requires `workers` feature)
//!
//! ```sh
//! cargo install worker-build
//! worker-build --release --features workers
//! # or: wrangler deploy
//! ```

pub mod d1_store;
pub mod durable_object;
pub mod noop_adapters;

pub use d1_store::{D1DataStore, D1Executor};
pub use durable_object::{
    do_websocket_sink, persist_to_do_storage, register_do_subscriber, restore_from_do_storage,
    DoStorage, DoSubscriberHandle, DURABLE_OBJECT_TEMPLATE_JS,
};
pub use noop_adapters::NoopAll;

#[cfg(feature = "workers")]
pub mod handler;
