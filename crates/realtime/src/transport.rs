//! Transport abstraction — the contract between shards and any wire protocol.
//!
//! The shard layer is transport-agnostic: it receives inputs, advances
//! simulation, and emits snapshots via [`SnapshotSink`]s. *How* those
//! snapshots reach the client (WebSocket, WebTransport, WebRTC, raw TCP,
//! a Durable Object's hibernating socket, whatever) is the job of a
//! [`ShardTransport`] adapter.
//!
//! # Existing transports
//!
//! - **WebSocket** (see `crates/runtime/src/shard_ws.rs`) — bidirectional,
//!   tungstenite-based. Works everywhere. Default for self-hosted.
//! - **SSE + HTTP POST** (see `crates/runtime/src/server.rs` `/connect` path) —
//!   no bidirectional frames, but works through any HTTP proxy and has
//!   zero client-side setup.
//! - **Durable Object WebSocket** (see `crates/workers/src/durable_object.rs`) —
//!   DO-hosted socket with hibernation.
//!
//! # Extending
//!
//! Write a module that:
//!
//! 1. Accepts an incoming connection via your transport's server API.
//! 2. Resolves `shard_id`, `subscriber_id`, and auth from the handshake.
//! 3. Builds a [`SnapshotSink`] that writes to your transport's outbound path.
//! 4. Calls [`DynShard::add_subscriber_authorized`] with the sink.
//! 5. Reads incoming messages and calls [`DynShard::push_input_json_authorized`].
//! 6. On disconnect, calls [`DynShard::remove_subscriber`].
//!
//! The [`ShardTransport`] trait below captures this shape so multiple
//! transports can be started from the same server entry point.

use std::sync::Arc;

use crate::dyn_shard::DynShardRegistry;

/// A pluggable transport that accepts client connections and routes them
/// to shards.
///
/// Transports are started once at server boot. They typically own a
/// `TcpListener` / `QuicEndpoint` / similar and spawn per-connection tasks.
/// The [`DynShardRegistry`] lookup lets them route to the right shard.
pub trait ShardTransport: Send + Sync {
    /// Human-readable name (for logs / `/health`).
    fn name(&self) -> &'static str;

    /// Start accepting connections. Blocking — spawn in a thread/task.
    ///
    /// The default `pylon` server spawns one thread per transport. A
    /// Workers deployment doesn't need this — transports there are
    /// per-request handlers driven by the Workers fetch event loop.
    fn serve(&self, registry: Arc<dyn DynShardRegistry>);

    /// Gracefully stop accepting new connections.
    ///
    /// Existing connections stay open; transports are expected to close
    /// them on `Drop`.
    fn shutdown(&self);
}

// ---------------------------------------------------------------------------
// Transport metadata — what a transport advertises to clients
// ---------------------------------------------------------------------------

/// Describes a running transport so clients can discover connection options.
///
/// Exposed via `GET /api/shards/transports` when wired in.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TransportInfo {
    pub name: String,
    pub scheme: String,
    pub port: u16,
    /// Query-string params clients should include (e.g. `["shard", "sid"]`).
    pub required_params: Vec<String>,
    /// Wire format used for snapshots (`"binary_tick_json"`, etc.).
    pub snapshot_framing: String,
}

// ---------------------------------------------------------------------------
// Common info constants for the built-in transports
// ---------------------------------------------------------------------------

/// Info for the built-in WebSocket transport.
pub fn websocket_info(port: u16, secure: bool) -> TransportInfo {
    TransportInfo {
        name: "websocket".into(),
        scheme: if secure { "wss" } else { "ws" }.into(),
        port,
        required_params: vec!["shard".into(), "sid".into()],
        snapshot_framing: "binary_tick_json".into(),
    }
}

/// Info for the built-in SSE + POST transport.
pub fn sse_info(port: u16, secure: bool) -> TransportInfo {
    TransportInfo {
        name: "sse".into(),
        scheme: if secure { "https" } else { "http" }.into(),
        port,
        required_params: vec!["sid".into()],
        snapshot_framing: "sse_event".into(),
    }
}

/// Info for a hypothetical WebTransport integration.
/// Emitted only when the user has enabled a WebTransport transport.
pub fn webtransport_info(port: u16, secure: bool) -> TransportInfo {
    TransportInfo {
        name: "webtransport".into(),
        scheme: if secure { "https" } else { "http" }.into(),
        port,
        required_params: vec!["shard".into(), "sid".into()],
        snapshot_framing: "datagram_tick_binary".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_info_defaults() {
        let info = websocket_info(4324, false);
        assert_eq!(info.name, "websocket");
        assert_eq!(info.scheme, "ws");
        assert_eq!(info.port, 4324);
    }

    #[test]
    fn websocket_info_secure() {
        let info = websocket_info(4324, true);
        assert_eq!(info.scheme, "wss");
    }

    #[test]
    fn transport_info_serializes() {
        let info = sse_info(4322, false);
        let s = serde_json::to_string(&info).unwrap();
        assert!(s.contains("\"name\":\"sse\""));
        assert!(s.contains("\"scheme\":\"http\""));
    }
}
