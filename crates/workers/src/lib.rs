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

// KV-backed Cache + R2-backed Files adapters. Only compiled with
// the `workers` feature because they depend on the `worker` crate's
// WASM-only types. Non-WASM builds get the NoopAll stub Cache + Files
// which return typed 503 errors.
#[cfg(feature = "workers")]
pub mod kv_cache;
#[cfg(feature = "workers")]
pub mod pubsub_do;
#[cfg(feature = "workers")]
pub mod queue_jobs;
#[cfg(feature = "workers")]
pub mod r2_files;
#[cfg(feature = "workers")]
pub mod rooms_do;

#[cfg(feature = "workers")]
pub use kv_cache::KvCache;
#[cfg(feature = "workers")]
pub use pubsub_do::{PylonPubSub, WorkersPubSub};
#[cfg(feature = "workers")]
pub use queue_jobs::WorkersJobs;
#[cfg(feature = "workers")]
pub use r2_files::R2Files;
#[cfg(feature = "workers")]
pub use rooms_do::{PylonRoom, WorkersRooms};
