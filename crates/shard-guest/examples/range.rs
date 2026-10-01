//! Lag compensation: a target walks along x, and players shoot at the point
//! where they saw it.
//!
//! Build: `cargo build -p pylon-shard-guest --example range --release
//! --target wasm32-unknown-unknown`. The runtime's WASM lag compensation
//! tests load it.
//!
//! Params `{"history": ticks, "speed": units per tick}`; history 0 turns lag
//! compensation off. Input `{"shoot": [x, y]}` hits when the target was
//! within 0.5 units of the point: where the shooter drew it at its view
//! tick with history on, where it is now otherwise. A miss comes back as a
//! rejection whose message starts with "miss".

#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use std::time::Duration;

use pylon_shard_guest::{
    export_shard, LagCompensation, Replicated, ReplicationConfig, Rewind, Shard,
};
use serde::Deserialize;

const TARGET: u64 = 1;
const HIT_RADIUS: f64 = 0.5;

#[derive(Deserialize)]
struct Params {
    #[serde(default)]
    history: u32,
    #[serde(default = "default_speed")]
    speed: f32,
}

fn default_speed() -> f32 {
    2.0
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Input {
    Shoot(f64, f64),
}

struct Range {
    store: Replicated,
    history: u32,
    speed: f32,
    x: f32,
}

impl Range {
    fn check(&self, aim: (f64, f64), at: Option<[f64; 3]>) -> Result<(), String> {
        let Some(p) = at else {
            return Err("miss: the target was not in view".into());
        };
        let d = ((p[0] - aim.0).powi(2) + (p[1] - aim.1).powi(2)).sqrt();
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
        let mut store = Replicated::new();
        store.spawn(TARGET, [0.0, 0.0, 0.0]);
        Ok(Range {
            store,
            history: params.history,
            speed: params.speed,
            x: 0.0,
        })
    }

    fn apply_input(&mut self, _subscriber: &str, input: Input) -> Result<(), String> {
        let Input::Shoot(x, y) = input;
        let now = self.store.get(TARGET).map(|e| e.pos.map(|v| v as f64));
        self.check((x, y), now)
    }

    fn apply_input_at(
        &mut self,
        _subscriber: &str,
        input: Input,
        rewind: &Rewind,
    ) -> Result<(), String> {
        let Input::Shoot(x, y) = input;
        self.check((x, y), rewind.position_at(TARGET))
    }

    fn tick(&mut self, _dt: Duration) {
        self.x += self.speed;
        self.store.set_pos(TARGET, [self.x, 0.0, 0.0]);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shot_hits_where_the_rewind_drew_the_target() {
        let mut r = Range::init(
            "r",
            Params {
                history: 10,
                speed: 2.0,
            },
        )
        .unwrap();
        let rewind = Rewind::with_positions(4.0, 6.0, [(TARGET, [8.0, 0.0, 0.0])]);
        assert!(r
            .apply_input_at("p", Input::Shoot(8.2, 0.0), &rewind)
            .is_ok());
        assert!(r
            .apply_input_at("p", Input::Shoot(9.0, 0.0), &rewind)
            .is_err());
    }
}
