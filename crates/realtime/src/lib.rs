//! pylon-realtime — tick-driven, sharded, stateful simulations.
//!
//! Provides a general-purpose [`Shard`] abstraction for any workload that is
//! contention-limited on a single entity: game matches, MMO zones, RTS rooms,
//! FPS lobbies, but also hot auction lots, live collab docs, or bid-heavy
//! listings.
//!
//! # Core ideas
//!
//! - **One Shard = one authoritative state + one tick loop.** Each shard owns
//!   its own lock, state, inputs, and subscribers. Shards run independently —
//!   no shared write path.
//!
//! - **Tick-driven.** [`TickLoop`] wakes the shard at a fixed rate (e.g.
//!   60 Hz). On every tick, the shard drains its input queue, advances
//!   simulation time, and broadcasts a snapshot to subscribers.
//!
//! - **Transport-agnostic.** The shard itself doesn't know about HTTP or
//!   WebSocket. Transports implement the [`SnapshotSink`] trait and the
//!   server layer wires them in.
//!
//! - **Binary by default.** Snapshots encode through a pluggable format
//!   (JSON for debugging, bincode / MessagePack for production).
//!
//! # Mapping to game genres
//!
//! - **Turn-based** (chess, card games): `tick_rate_hz = 0`, inputs drive
//!   ticks directly.
//! - **RTS** (lockstep): `tick_rate_hz = 10–30`, inputs ack'd with tick
//!   numbers for synchronized execution.
//! - **MMO** (zone-based): each zone is a shard; implement
//!   [`SimState::snapshot_for`] to filter by area-of-interest.
//! - **FPS** (authoritative server): `tick_rate_hz = 60`, add input
//!   sequence numbers so the client can reconcile.

pub mod aoi;
pub mod dyn_shard;
pub mod matchmaker;
pub mod persistence;
pub mod prediction;
pub mod registry;
pub mod replay;
pub mod shard;
pub mod snapshot;
pub mod subscriber;
pub mod tick;
pub mod transport;

pub use aoi::AreaOfInterest;
pub use dyn_shard::{DynShard, DynShardRegistry};
pub use matchmaker::{
    fixed_size_match, MatchAssignment, MatchFn, Matchmaker, MatchmakerConfig, PlayerStatus,
    QueuedPlayer, ShardFactory,
};
pub use persistence::{persist_every_ticks, restore_or_init};
pub use prediction::{InputAck, Reconciliation};
pub use registry::ShardRegistry;
pub use replay::{replay, ReplayEntry, ReplayLog};
pub use shard::{Shard, ShardAuth, ShardConfig, ShardError, SimState};
pub use snapshot::{encode_snapshot, SnapshotFormat};
pub use subscriber::{SnapshotSink, Subscriber, SubscriberId};
pub use tick::TickLoop;
pub use transport::{sse_info, websocket_info, webtransport_info, ShardTransport, TransportInfo};
