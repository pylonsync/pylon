//! A shooting range with lag compensation: one target circles the middle,
//! and players shoot at the point where they saw it.
//!
//! Params `{ "history": ticks, "speed": units per second }`. With history
//! on (`> 0`), a shot is checked against where the shooter's client drew
//! the target at the tick it was drawing; with history 0, against where the
//! target is when the shot arrives. `pylon bench shard --aim 1` measures the
//! difference (see the README).

use std::time::Duration;

use pylon_shard_guest::{export_shard, LagCompensation, Replicated, ReplicationConfig, Rewind, Shard};
use serde::Deserialize;

/// The target's entity id.
const TARGET: u64 = 1;
/// The target circles this far from the middle.
const RADIUS: f32 = 10.0;
/// A shot within this distance of the target hits.
const HIT_RADIUS: f64 = 0.5;

#[derive(Deserialize)]
struct Params {
    #[serde(default)]
    history: u32,
    #[serde(default = "default_speed")]
    speed: f32,
}

fn default_speed() -> f32 {
    8.0
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Input {
    /// Aimed at the point (x, y).
    Shoot(f64, f64),
}

struct Range {
    store: Replicated,
    history: u32,
    speed: f32,
    angle: f32,
}

impl Range {
    fn check(at: Option<[f64; 3]>, x: f64, y: f64) -> Result<(), String> {
        let Some(p) = at else {
            return Err("miss: the target was not in view".into());
        };
        let d = ((p[0] - x).powi(2) + (p[1] - y).powi(2)).sqrt();
        if d <= HIT_RADIUS {
            Ok(())
        } else {
            Err(format!("miss: {d:.2} units off"))
        }
    }
}

impl Shard for Range {
    type Params = Params;
    type Input = Input;
    type Snapshot = ();

    fn init(_shard_id: &str, params: Params) -> Result<Self, String> {
        if !(params.speed >= 0.0 && params.speed.is_finite()) {
            return Err("speed must be a non-negative number".into());
        }
        let mut store = Replicated::new();
        store.spawn(TARGET, [RADIUS, 0.0, 0.0]);
        Ok(Range {
            store,
            history: params.history.min(60),
            speed: params.speed,
            angle: 0.0,
        })
    }

    /// History off: the target as it is now.
    fn apply_input(&mut self, _subscriber: &str, Input::Shoot(x, y): Input) -> Result<(), String> {
        let now = self.store.get(TARGET).map(|e| e.pos.map(|v| v as f64));
        Self::check(now, x, y)
    }

    /// History on: the target where this player drew it.
    fn apply_input_at(&mut self, _subscriber: &str, Input::Shoot(x, y): Input, rewind: &Rewind) -> Result<(), String> {
        Self::check(rewind.position_at(TARGET), x, y)
    }

    fn tick(&mut self, dt: Duration) {
        self.angle += self.speed / RADIUS * dt.as_secs_f32();
        let (s, c) = self.angle.sin_cos();
        self.store.set_pos(TARGET, [RADIUS * c, RADIUS * s, 0.0]);
    }

    fn snapshot(&self) {}

    fn replicated(&mut self) -> Option<&mut Replicated> {
        Some(&mut self.store)
    }

    fn replication_config(&self) -> ReplicationConfig {
        ReplicationConfig {
            precision: 0.01,
            max_bytes_per_tick: 0,
            y_up: false,
        }
    }

    fn lag_compensation(&self) -> Option<LagCompensation> {
        (self.history > 0).then(|| LagCompensation::new(self.history))
    }
}

export_shard!(Range);
