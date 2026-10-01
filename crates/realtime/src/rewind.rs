//! Lag compensation: checking a player's action against what that player
//! saw.
//!
//! A client draws other entities in the past: its interpolation delay
//! (about 100 ms) plus the time frames take to reach it. An aimed shot is
//! aimed at that picture. With lag compensation on, the client stamps each
//! input with the tick it was drawing (`view_tick` in the input envelope),
//! the shard keeps what it sent each subscriber over the last ticks
//! ([`pylon_replication::ViewHistory`]), and the simulation gets a [`Rewind`]
//! with the input: where that subscriber drew each entity at that tick.
//!
//! The shard clamps the view tick, so a client cannot shoot further into
//! the past than its connection explains:
//!
//! - not after the newest tick sent;
//! - not before the newest tick minus [`LagCompensation::history_ticks`];
//! - once the connection's round trip is known, not before the newest tick
//!   minus the round trip and [`LagCompensation::max_view_delay_ms`].

use std::sync::Arc;
use std::time::Duration;

use pylon_replication::{EntityId, ViewHistory};

/// Lag compensation settings for a replicating shard (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LagCompensation {
    /// Ticks of what each subscriber was sent to keep: how far back a view
    /// can reach. 10 ticks at 20 Hz is 500 ms.
    pub history_ticks: u32,
    /// How far behind the shard a client may draw beyond its round trip:
    /// its interpolation delay plus jitter. Default 250 ms.
    pub max_view_delay_ms: u32,
}

impl LagCompensation {
    /// Keep `history_ticks` ticks, with the default view delay allowance.
    pub fn new(history_ticks: u32) -> Self {
        Self {
            history_ticks,
            max_view_delay_ms: 250,
        }
    }
}

/// What one subscriber drew at the tick it acted on, for one input.
#[derive(Debug, Clone)]
pub struct Rewind {
    tick: f64,
    now_tick: f64,
    history: Option<Arc<ViewHistory>>,
}

impl Rewind {
    /// A view of `history` at `tick` (already clamped), for an input applied
    /// on tick `now_tick`.
    pub fn new(tick: f64, now_tick: f64, history: Option<Arc<ViewHistory>>) -> Self {
        Self {
            tick,
            now_tick,
            history,
        }
    }

    /// The tick (fractional, clamped) the subscriber was drawing.
    pub fn tick(&self) -> f64 {
        self.tick
    }

    /// The tick the input applies on: the one the simulation advances to
    /// next.
    pub fn now_tick(&self) -> f64 {
        self.now_tick
    }

    /// How many ticks (fractional) the subscriber's picture was behind the
    /// shard: for a projectile, how far to advance it to catch up.
    pub fn ticks_behind(&self) -> f64 {
        self.now_tick - self.tick
    }

    /// Where the subscriber drew `id` at the view tick, or None when it did
    /// not draw it then (out of its view, not spawned yet, or gone).
    pub fn position_at(&self, id: EntityId) -> Option<[f64; 3]> {
        self.history.as_ref()?.position_at(id, self.tick)
    }

    /// The entities the subscriber drew within `radius` of `center` (3D
    /// distance) at the view tick, with their positions then, in id order.
    pub fn entities_near(&self, center: [f64; 3], radius: f64) -> Vec<(EntityId, [f64; 3])> {
        match &self.history {
            Some(h) => h.entities_near(center, radius, self.tick),
            None => Vec::new(),
        }
    }
}

/// Clamp a client's view tick (see the module docs). `newest` is the newest
/// tick frames went out for; `tick_ms` the shard's tick length. None for a
/// view tick that is not a finite number.
pub fn clamp_view_tick(
    view_tick: f64,
    newest: u64,
    lag: &LagCompensation,
    rtt: Option<Duration>,
    tick_ms: f64,
) -> Option<f64> {
    if !view_tick.is_finite() {
        return None;
    }
    let hi = newest as f64;
    let mut lo = hi - lag.history_ticks as f64;
    if let Some(rtt) = rtt {
        if tick_ms > 0.0 && tick_ms.is_finite() {
            let behind_ms = rtt.as_secs_f64() * 1000.0 + lag.max_view_delay_ms as f64;
            lo = lo.max(hi - behind_ms / tick_ms);
        }
    }
    Some(view_tick.clamp(lo.min(hi), hi))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_view_tick_is_clamped_at_both_ends() {
        let lag = LagCompensation::new(10);
        // In range: unchanged.
        assert_eq!(clamp_view_tick(96.5, 100, &lag, None, 50.0), Some(96.5));
        // Not after the newest tick sent.
        assert_eq!(clamp_view_tick(140.0, 100, &lag, None, 50.0), Some(100.0));
        // Not before the history.
        assert_eq!(clamp_view_tick(20.0, 100, &lag, None, 50.0), Some(90.0));
        // A 100 ms round trip and a 250 ms allowance at 50 ms ticks: 7 ticks.
        let rtt = Some(Duration::from_millis(100));
        assert_eq!(clamp_view_tick(91.0, 100, &lag, rtt, 50.0), Some(93.0));
        assert_eq!(clamp_view_tick(95.0, 100, &lag, rtt, 50.0), Some(95.0));
        // A long round trip leaves the history bound.
        let slow = Some(Duration::from_secs(2));
        assert_eq!(clamp_view_tick(50.0, 100, &lag, slow, 50.0), Some(90.0));
        // Not a number: no view.
        assert_eq!(clamp_view_tick(f64::NAN, 100, &lag, None, 50.0), None);
        assert_eq!(clamp_view_tick(f64::INFINITY, 100, &lag, None, 50.0), None);
        // Early ticks: the bound goes below zero, the value stays in range.
        assert_eq!(clamp_view_tick(1.5, 3, &lag, None, 50.0), Some(1.5));
    }

    #[test]
    fn a_view_without_history_sees_nothing() {
        let v = Rewind::new(5.0, 8.0, None);
        assert_eq!(v.tick(), 5.0);
        assert_eq!(v.ticks_behind(), 3.0);
        assert_eq!(v.position_at(1), None);
        assert!(v.entities_near([0.0; 3], 100.0).is_empty());
    }
}
