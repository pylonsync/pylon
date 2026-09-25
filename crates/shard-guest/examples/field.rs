//! A replicating shard: units walk in circles in a `Replicated` store and
//! the host sends each subscriber spawn, update, and despawn frames.
//!
//! Build: `cargo build -p pylon-shard-guest --example field --release
//! --target wasm32-unknown-unknown`. The runtime's WASM replication tests
//! load it.
//!
//! Subscriber "u<N>" watches from unit N. Unit 0 is stealthed: only
//! subscribers named "seer..." see it. Input `{"hp": [id, value]}` sets a
//! unit's hp component (id 1); `{"despawn": id}` removes a unit.

// The exports exist only in the wasm build; a native build (`cargo test`)
// sees the types as unused.
#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use std::time::Duration;

use pylon_shard_guest::{
    export_shard, InterestArea, InterestConfig, Replicated, ReplicationConfig, Shard,
};
use serde::Deserialize;

const HP: u8 = 1;

#[derive(Deserialize)]
struct Params {
    units: u64,
    /// Interest radius; none replicates every unit to everyone.
    #[serde(default)]
    radius: Option<f32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Input {
    Hp(u64, u8),
    Despawn(u64),
    /// Start over with a new store: `n` units, shifted 1000 units right.
    Reset(u64),
    /// Spawn `n` more units.
    Flood(u64),
}

struct Field {
    store: Replicated,
    t: f32,
    radius: Option<f32>,
    /// Added to every unit's x (a reset moves the field).
    shift: f32,
}

impl Shard for Field {
    type Params = Params;
    type Input = Input;
    type Snapshot = ();

    fn init(_shard_id: &str, params: Params) -> Result<Self, String> {
        let mut store = Replicated::new();
        for id in 0..params.units {
            store.spawn(id, [id as f32 * 3.0, 0.0, 0.0]);
            store.set_component(id, HP, &[100]);
        }
        Ok(Field {
            store,
            t: 0.0,
            radius: params.radius,
            shift: 0.0,
        })
    }

    fn apply_input(&mut self, _subscriber: &str, input: Input) -> Result<(), String> {
        match input {
            Input::Hp(id, hp) => {
                if !self.store.set_component(id, HP, &[hp]) {
                    return Err(format!("no unit {id}"));
                }
            }
            Input::Despawn(id) => {
                self.store.despawn(id);
            }
            Input::Reset(n) => {
                let mut store = Replicated::new();
                for id in 0..n {
                    store.spawn(id, [1000.0 + id as f32 * 3.0, 0.0, 0.0]);
                    store.set_component(id, HP, &[50]);
                }
                self.store = store;
                self.shift = 1000.0;
            }
            Input::Flood(n) => {
                let start = self.store.iter().map(|(id, _)| id + 1).max().unwrap_or(0);
                for id in start..start + n {
                    self.store.spawn(id, [0.0; 3]);
                }
            }
        }
        Ok(())
    }

    fn tick(&mut self, dt: Duration) {
        self.t += dt.as_secs_f32();
        let ids: Vec<u64> = self.store.iter().map(|(id, _)| id).collect();
        for id in ids {
            let a = self.t + id as f32;
            let base = self.shift + id as f32 * 3.0;
            self.store
                .set_pos(id, [base + a.cos() * 2.0, a.sin() * 2.0, 0.0]);
        }
    }

    fn snapshot(&self) {}

    fn interest(&self) -> Option<InterestConfig> {
        self.radius.map(|r| InterestConfig {
            cell_size: r,
            margin: 1.0,
            shared_snapshots: false,
        })
    }

    fn interest_area(&self, subscriber: &str) -> Option<InterestArea> {
        let n: u64 = subscriber
            .trim_start_matches(|c: char| !c.is_ascii_digit())
            .parse()
            .ok()?;
        let p = self.store.get(n)?.pos;
        Some(InterestArea {
            x: p[0],
            y: p[1],
            radius: self.radius.unwrap_or(f32::MAX),
        })
    }

    fn can_see(&self, subscriber: &str, entity: u64) -> bool {
        entity != 0 || subscriber.starts_with("seer")
    }

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
}

export_shard!(Field);
