//! The frontier shard: a large open zone, the shape of an MMO's realm-war
//! frontier. Each player is an entity in a `Replicated` store. The host
//! sends each subscriber only the entities within its view radius (interest
//! management), as spawn, update, and despawn frames within a byte budget.
//!
//! It exists to load-test that path: `pylon bench shard --join joinFrontier`
//! (see README). `arena` sends every subscriber the full snapshot instead.

// The exports exist only in the wasm build; a native build (`cargo test`)
// sees the types as unused.
#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use std::collections::HashMap;
use std::time::Duration;

use pylon_shard_guest::{
    export_shard, InterestArea, InterestConfig, Replicated, ReplicationConfig, Shard,
};
use serde::Deserialize;

/// Units per second a player runs.
const SPEED: f32 = 60.0;
/// A player who sends nothing for this long leaves the zone.
const IDLE_LIMIT: Duration = Duration::from_secs(60);
/// Component id of hit points (two bytes, little-endian).
const HP: u8 = 1;
/// Hit points lost per `hit`.
const HIT_DAMAGE: u16 = 7;
const MAX_HP: u16 = 1000;

#[derive(Deserialize)]
struct Params {
    width: f32,
    height: f32,
    /// How far a player sees, in world units.
    view_radius: f32,
    /// Byte budget per subscriber per tick for updates. 0 = no limit.
    #[serde(default)]
    max_bytes_per_tick: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Input {
    /// Enter the zone, or stay in it.
    Join,
    /// Run toward a point.
    MoveTo { x: f32, y: f32 },
    /// Hit the nearest other player in view. A hit changes the target's hp
    /// component, so it replicates to every subscriber that sees the target.
    Hit,
}

struct Player {
    entity: u64,
    target: [f32; 2],
    hp: u16,
    idle: Duration,
}

struct Frontier {
    width: f32,
    height: f32,
    view_radius: f32,
    max_bytes_per_tick: u32,
    store: Replicated,
    players: HashMap<String, Player>,
    next_entity: u64,
}

/// A spawn point from the subscriber id, so players start spread over the
/// zone and a restart puts them back in the same place.
fn spawn_point(id: &str, width: f32, height: f32) -> [f32; 2] {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in id.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    let x = (h & 0xffff) as f32 / 65535.0 * width;
    let y = ((h >> 16) & 0xffff) as f32 / 65535.0 * height;
    [x, y]
}

impl Frontier {
    fn player(&mut self, id: &str) -> &mut Player {
        if !self.players.contains_key(id) {
            let entity = self.next_entity;
            self.next_entity += 1;
            let [x, y] = spawn_point(id, self.width, self.height);
            self.store.spawn(entity, [x, y, 0.0]);
            self.store
                .set_component(entity, HP, &MAX_HP.to_le_bytes());
            self.players.insert(
                id.to_string(),
                Player {
                    entity,
                    target: [x, y],
                    hp: MAX_HP,
                    idle: Duration::ZERO,
                },
            );
        }
        self.players.get_mut(id).expect("just inserted")
    }

    fn position(&self, entity: u64) -> Option<[f32; 2]> {
        self.store.get(entity).map(|e| [e.pos[0], e.pos[1]])
    }

    /// The nearest other player within view of `id`, by entity.
    fn nearest_in_view(&self, id: &str) -> Option<String> {
        let me = self.players.get(id)?;
        let [x, y] = self.position(me.entity)?;
        let r2 = self.view_radius * self.view_radius;
        self.players
            .iter()
            .filter(|(other, _)| other.as_str() != id)
            .filter_map(|(other, p)| {
                let [ox, oy] = self.position(p.entity)?;
                let d2 = (ox - x).powi(2) + (oy - y).powi(2);
                (d2 <= r2).then(|| (d2, other.clone()))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, other)| other)
    }
}

impl Shard for Frontier {
    type Params = Params;
    type Input = Input;
    type Snapshot = ();

    fn init(_shard_id: &str, params: Params) -> Result<Self, String> {
        if !(params.width > 0.0 && params.height > 0.0 && params.view_radius > 0.0) {
            return Err("width, height, and view_radius must be positive".into());
        }
        Ok(Frontier {
            width: params.width,
            height: params.height,
            view_radius: params.view_radius,
            max_bytes_per_tick: params.max_bytes_per_tick,
            store: Replicated::new(),
            players: HashMap::new(),
            next_entity: 0,
        })
    }

    fn apply_input(&mut self, subscriber: &str, input: Input) -> Result<(), String> {
        let (w, h) = (self.width, self.height);
        let p = self.player(subscriber);
        p.idle = Duration::ZERO;
        match input {
            Input::Join => {}
            Input::MoveTo { x, y } => {
                if !(0.0..=w).contains(&x) || !(0.0..=h).contains(&y) {
                    return Err(format!("({x}, {y}) is outside the zone"));
                }
                p.target = [x, y];
            }
            Input::Hit => {
                let Some(target) = self.nearest_in_view(subscriber) else {
                    return Ok(());
                };
                let t = self.players.get_mut(&target).expect("found above");
                // Wrap to full so a long run keeps changing the component.
                t.hp = match t.hp.checked_sub(HIT_DAMAGE) {
                    Some(hp) if hp > 0 => hp,
                    _ => MAX_HP,
                };
                let (entity, hp) = (t.entity, t.hp);
                self.store.set_component(entity, HP, &hp.to_le_bytes());
            }
        }
        Ok(())
    }

    fn tick(&mut self, dt: Duration) {
        let step = SPEED * dt.as_secs_f32();
        for p in self.players.values_mut() {
            p.idle += dt;
            let Some(e) = self.store.get(p.entity) else {
                continue;
            };
            let [x, y] = [e.pos[0], e.pos[1]];
            let (dx, dy) = (p.target[0] - x, p.target[1] - y);
            let dist = (dx * dx + dy * dy).sqrt();
            if dist == 0.0 {
                continue;
            }
            let next = if dist <= step {
                p.target
            } else {
                [x + dx / dist * step, y + dy / dist * step]
            };
            self.store.set_pos(p.entity, [next[0], next[1], 0.0]);
        }
        let store = &mut self.store;
        self.players.retain(|_, p| {
            let keep = p.idle < IDLE_LIMIT;
            if !keep {
                store.despawn(p.entity);
            }
            keep
        });
    }

    fn snapshot(&self) {}

    fn interest(&self) -> Option<InterestConfig> {
        Some(InterestConfig {
            cell_size: self.view_radius,
            margin: self.view_radius * 0.1,
            shared_snapshots: false,
        })
    }

    fn interest_area(&self, subscriber: &str) -> Option<InterestArea> {
        let p = self.players.get(subscriber)?;
        let [x, y] = self.position(p.entity)?;
        Some(InterestArea {
            x,
            y,
            radius: self.view_radius,
        })
    }

    fn replicated(&mut self) -> Option<&mut Replicated> {
        Some(&mut self.store)
    }

    fn replication_config(&self) -> ReplicationConfig {
        ReplicationConfig {
            precision: 0.05,
            max_bytes_per_tick: self.max_bytes_per_tick,
            y_up: false,
        }
    }
}

export_shard!(Frontier);
