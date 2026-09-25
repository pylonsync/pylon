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
//!   WebSocket. A network transport gives each client an [`OutboundQueue`]
//!   and drains it from its own thread or task; the tick thread only
//!   enqueues, so a slow client never stalls the shard. In-process
//!   consumers can use a [`SnapshotSink`] instead.
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
//! - **MMO** (zone-based): each zone is a shard; return an
//!   [`InterestConfig`] from [`SimState::interest_config`] so the shard sends
//!   each subscriber only the entities near it (see [`interest`]).
//! - **FPS** (authoritative server): `tick_rate_hz = 60`, add input
//!   sequence numbers so the client can reconcile.

pub mod aoi;
pub mod dyn_shard;
pub mod interest;
pub mod matchmaker;
pub mod outbound;
pub mod persistence;
pub mod prediction;
pub mod raw;
pub mod registry;
pub mod replay;
pub mod shard;
pub mod snapshot;
pub mod subscriber;
pub mod tick;
pub mod ticket;
pub mod transport;
pub mod wire;

pub use aoi::AreaOfInterest;
pub use dyn_shard::{DynShard, DynShardRegistry};
pub use interest::{
    EntityId, EntityPos, InterestArea, InterestConfig, InterestManager, SpatialGrid, Visibility,
};
pub use matchmaker::{
    fixed_size_match, MatchAssignment, MatchFn, Matchmaker, MatchmakerConfig, PlayerStatus,
    QueuedPlayer, ShardFactory,
};
pub use outbound::{Frame, FrameKind, OutboundConfig, OutboundQueue, PushOutcome};
pub use persistence::{persist_every_ticks, restore_or_init};
pub use prediction::{InputAck, Reconciliation};
pub use raw::{RawInput, RawSnapshot};
pub use registry::ShardRegistry;
pub use replay::{replay, replay_to, ReplayEntry, ReplayLog};
pub use shard::{Shard, ShardAuth, ShardConfig, ShardError, SimState};
pub use snapshot::{encode_snapshot, EncodeSnapshot, SnapshotFormat};
pub use subscriber::{SnapshotSink, Subscriber, SubscriberId};
pub use tick::TickLoop;
pub use ticket::{sign_ticket, verify_ticket, ShardTicket, TicketError};
pub use transport::{sse_info, websocket_info, webtransport_info, ShardTransport, TransportInfo};
pub use wire::{InputEnvelope, InputRejection, ShardInput};
