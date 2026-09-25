//! The island shard: player poses and combat for World3D.
//!
//! Each player is a replicated entity. Clients move their own player
//! (they run the physics against the terrain, which only they have) by
//! sending the distance moved since the last input; the shard applies it
//! within a speed budget and the island bounds, so a client cannot
//! teleport or outrun the others. Hits are range-checked and damage is
//! capped here, and health lives here.
//!
//! Subscribers are Avatar row ids: `joinIsland` mints the ticket with the
//! caller's avatar id as the subscriber id.
//!
//! Runs inside the Pylon server as a WebAssembly module (see app.ts).
//! `pylon dev` rebuilds it when this file changes.

// The exports exist only in the wasm build; a native build (`cargo test`)
// sees the types as unused.
#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use std::collections::HashMap;
use std::time::Duration;

use pylon_shard_guest::{export_shard, Replicated, ReplicationConfig, Shard};
use serde::Deserialize;

/// Component: the Avatar row id (UTF-8), for the name and color.
const AVATAR: u8 = 1;
/// Component: heading and pitch, each an i16 LE of radians x 10 000.
const LOOK: u8 = 2;
/// Component: health, one byte, 0 to 100.
const HEALTH: u8 = 3;

/// Ground speed the budget allows, in m/s: sprinting (11.5) with slack for
/// frame timing.
const GROUND_SPEED: f32 = 18.0;
/// Vertical speed the budget allows, in m/s (a long fall).
const VERTICAL_SPEED: f32 = 45.0;
/// The budget saves up at most this many seconds of movement, so inputs
/// that arrive together after a network stall still apply.
const BUDGET_SECS: f32 = 1.0;
/// The island is 512 m across, centered on the origin.
const HALF_WORLD: f32 = 250.0;
const MIN_Y: f32 = -10.0;
const MAX_Y: f32 = 200.0;
/// Weapon range (220 m) plus slack for grenade throws and pose lag.
const HIT_RANGE: f32 = 280.0;
const MAX_DAMAGE: u8 = 60;
/// A dead player may spawn again after this long.
const RESPAWN_DELAY: Duration = Duration::from_millis(2500);
/// A player who sends nothing for this long leaves.
const IDLE_LIMIT: Duration = Duration::from_secs(15);

#[derive(Deserialize)]
struct Params {}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Input {
    /// Enter the island at a spawn point the client picked on the terrain.
    /// Ignored when already in.
    Join { x: f32, y: f32, z: f32 },
    /// Moved this far since the last move, now looking this way. A move
    /// with no distance keeps the player in.
    Move {
        dx: f32,
        dy: f32,
        dz: f32,
        heading: f32,
        pitch: f32,
    },
    /// Shot entity `target` for `damage`.
    Hit { target: u64, damage: u8 },
    /// Come back after dying, at a spawn point the client picked.
    Spawn { x: f32, y: f32, z: f32 },
    /// Leave the island.
    Leave,
}

struct Player {
    entity: u64,
    pos: [f32; 3],
    health: u8,
    /// Meters of movement the player may still use, ground and vertical.
    budget: [f32; 2],
    idle: Duration,
    /// Time since health reached 0.
    dead_for: Duration,
}

struct Island {
    store: Replicated,
    players: HashMap<String, Player>,
    next_entity: u64,
}

fn clamp_pos(p: [f32; 3]) -> [f32; 3] {
    [
        p[0].clamp(-HALF_WORLD, HALF_WORLD),
        p[1].clamp(MIN_Y, MAX_Y),
        p[2].clamp(-HALF_WORLD, HALF_WORLD),
    ]
}

fn finite(values: &[f32]) -> Result<(), String> {
    if values.iter().all(|v| v.is_finite()) {
        Ok(())
    } else {
        Err("numbers must be finite".into())
    }
}

/// Heading and pitch as the LOOK component.
fn encode_look(heading: f32, pitch: f32) -> [u8; 4] {
    let wrap = |a: f32| {
        let a = a.rem_euclid(std::f32::consts::TAU);
        if a > std::f32::consts::PI {
            a - std::f32::consts::TAU
        } else {
            a
        }
    };
    let h = (wrap(heading) * 10_000.0).round() as i16;
    let p = (pitch.clamp(-1.55, 1.55) * 10_000.0).round() as i16;
    let (h, p) = (h.to_le_bytes(), p.to_le_bytes());
    [h[0], h[1], p[0], p[1]]
}

impl Island {
    fn join(&mut self, subscriber: &str, pos: [f32; 3]) {
        if self.players.contains_key(subscriber) {
            return;
        }
        let entity = self.next_entity;
        self.next_entity += 1;
        let pos = clamp_pos(pos);
        self.store.spawn(entity, pos);
        self.store
            .set_component(entity, AVATAR, subscriber.as_bytes());
        self.store.set_component(entity, LOOK, &encode_look(0.0, 0.0));
        self.store.set_component(entity, HEALTH, &[100]);
        self.players.insert(
            subscriber.to_string(),
            Player {
                entity,
                pos,
                health: 100,
                budget: [GROUND_SPEED * BUDGET_SECS, VERTICAL_SPEED * BUDGET_SECS],
                idle: Duration::ZERO,
                dead_for: Duration::ZERO,
            },
        );
    }

    fn leave(&mut self, subscriber: &str) {
        if let Some(p) = self.players.remove(subscriber) {
            self.store.despawn(p.entity);
        }
    }

    fn player_mut(&mut self, subscriber: &str) -> Result<&mut Player, String> {
        self.players
            .get_mut(subscriber)
            .ok_or_else(|| "join the island first".to_string())
    }
}

impl Shard for Island {
    type Params = Params;
    type Input = Input;
    type Snapshot = ();

    fn init(_shard_id: &str, _params: Params) -> Result<Self, String> {
        Ok(Island {
            store: Replicated::new(),
            players: HashMap::new(),
            next_entity: 1,
        })
    }

    fn apply_input(&mut self, subscriber: &str, input: Input) -> Result<(), String> {
        if let Some(p) = self.players.get_mut(subscriber) {
            p.idle = Duration::ZERO;
        }
        match input {
            Input::Join { x, y, z } => {
                finite(&[x, y, z])?;
                self.join(subscriber, [x, y, z]);
            }
            Input::Move {
                dx,
                dy,
                dz,
                heading,
                pitch,
            } => {
                finite(&[dx, dy, dz, heading, pitch])?;
                let p = self.player_mut(subscriber)?;
                if p.health == 0 {
                    // The dead do not move; the client stops sending moves.
                    return Ok(());
                }
                // Spend the budget; a move past it goes as far as it allows.
                let ground = (dx * dx + dz * dz).sqrt();
                let g = if ground > p.budget[0] {
                    p.budget[0] / ground
                } else {
                    1.0
                };
                let v = if dy.abs() > p.budget[1] {
                    p.budget[1] / dy.abs()
                } else {
                    1.0
                };
                p.budget[0] -= ground * g;
                p.budget[1] -= dy.abs() * v;
                p.pos = clamp_pos([p.pos[0] + dx * g, p.pos[1] + dy * v, p.pos[2] + dz * g]);
                let (entity, pos) = (p.entity, p.pos);
                self.store.set_pos(entity, pos);
                self.store
                    .set_component(entity, LOOK, &encode_look(heading, pitch));
            }
            Input::Hit { target, damage } => {
                let shooter = self.player_mut(subscriber)?;
                if shooter.health == 0 {
                    return Err("the dead cannot shoot".into());
                }
                let (from, own) = (shooter.pos, shooter.entity);
                if target == own {
                    return Ok(());
                }
                let Some(victim) = self.players.values_mut().find(|p| p.entity == target) else {
                    return Err(format!("no player {target}"));
                };
                let d = [
                    victim.pos[0] - from[0],
                    victim.pos[1] - from[1],
                    victim.pos[2] - from[2],
                ];
                if (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() > HIT_RANGE {
                    return Err("target out of weapon range".into());
                }
                if victim.health == 0 {
                    return Ok(());
                }
                victim.health = victim.health.saturating_sub(damage.clamp(1, MAX_DAMAGE));
                victim.dead_for = Duration::ZERO;
                let (entity, health) = (victim.entity, victim.health);
                self.store.set_component(entity, HEALTH, &[health]);
            }
            Input::Spawn { x, y, z } => {
                finite(&[x, y, z])?;
                let p = self.player_mut(subscriber)?;
                if p.health > 0 {
                    return Err("only the dead respawn".into());
                }
                if p.dead_for < RESPAWN_DELAY {
                    return Err("too soon to respawn".into());
                }
                p.health = 100;
                p.pos = clamp_pos([x, y, z]);
                p.budget = [GROUND_SPEED * BUDGET_SECS, VERTICAL_SPEED * BUDGET_SECS];
                let (entity, pos) = (p.entity, p.pos);
                self.store.set_pos(entity, pos);
                self.store.set_component(entity, HEALTH, &[100]);
            }
            Input::Leave => self.leave(subscriber),
        }
        Ok(())
    }

    fn tick(&mut self, dt: Duration) {
        let secs = dt.as_secs_f32();
        let mut gone = Vec::new();
        for (id, p) in &mut self.players {
            p.budget[0] = (p.budget[0] + GROUND_SPEED * secs).min(GROUND_SPEED * BUDGET_SECS);
            p.budget[1] = (p.budget[1] + VERTICAL_SPEED * secs).min(VERTICAL_SPEED * BUDGET_SECS);
            p.idle += dt;
            if p.health == 0 {
                p.dead_for += dt;
            }
            if p.idle >= IDLE_LIMIT {
                gone.push(id.clone());
            }
        }
        for id in gone {
            self.leave(&id);
        }
    }

    fn snapshot(&self) {}

    fn replicated(&mut self) -> Option<&mut Replicated> {
        Some(&mut self.store)
    }

    fn replication_config(&self) -> ReplicationConfig {
        ReplicationConfig {
            precision: 0.01,
            max_bytes_per_tick: 0,
            y_up: true,
        }
    }
}

export_shard!(Island);

#[cfg(test)]
mod tests {
    use super::*;

    const TICK: Duration = Duration::from_millis(50);

    fn island() -> Island {
        Island::init("island", Params {}).unwrap()
    }

    fn mv(dx: f32, dz: f32) -> Input {
        Input::Move {
            dx,
            dy: 0.0,
            dz,
            heading: 0.0,
            pitch: 0.0,
        }
    }

    fn pos(s: &Island, sub: &str) -> [f32; 3] {
        s.store.get(s.players[sub].entity).unwrap().pos
    }

    #[test]
    fn a_player_joins_moves_and_leaves() {
        let mut s = island();
        s.apply_input("a", Input::Join { x: 1.0, y: 2.0, z: 3.0 })
            .unwrap();
        s.apply_input("a", mv(1.0, -1.0)).unwrap();
        assert_eq!(pos(&s, "a"), [2.0, 2.0, 2.0]);
        let e = s.players["a"].entity;
        assert_eq!(s.store.get(e).unwrap().component(AVATAR), Some(&b"a"[..]));
        assert_eq!(s.store.get(e).unwrap().component(HEALTH), Some(&[100][..]));
        s.apply_input("a", Input::Leave).unwrap();
        assert!(s.store.is_empty());
        assert!(s.apply_input("a", mv(1.0, 0.0)).is_err());
    }

    #[test]
    fn movement_is_held_to_the_speed_budget() {
        let mut s = island();
        s.apply_input("a", Input::Join { x: 0.0, y: 0.0, z: 0.0 })
            .unwrap();
        // A 100 m jump goes only as far as one second of budget.
        s.apply_input("a", mv(100.0, 0.0)).unwrap();
        assert_eq!(pos(&s, "a")[0], GROUND_SPEED * BUDGET_SECS);
        // The budget is spent; one tick earns one tick of movement.
        s.tick(TICK);
        s.apply_input("a", mv(5.0, 0.0)).unwrap();
        let x = pos(&s, "a")[0];
        assert!((x - (GROUND_SPEED + GROUND_SPEED * 0.05)).abs() < 1e-3, "{x}");
        // Sprinting at 20 inputs a second stays within it.
        for _ in 0..100 {
            s.tick(TICK);
            s.apply_input("a", mv(11.5 / 20.0, 0.0)).unwrap();
        }
        let x2 = pos(&s, "a")[0];
        assert!((x2 - x - 57.5).abs() < 1e-2, "{x2}");
    }

    #[test]
    fn positions_stay_on_the_island_and_numbers_must_be_finite() {
        let mut s = island();
        s.apply_input("a", Input::Join { x: 9999.0, y: -50.0, z: 0.0 })
            .unwrap();
        assert_eq!(pos(&s, "a"), [HALF_WORLD, MIN_Y, 0.0]);
        assert!(s.apply_input("a", mv(f32::NAN, 0.0)).is_err());
        assert!(s
            .apply_input("b", Input::Join { x: f32::INFINITY, y: 0.0, z: 0.0 })
            .is_err());
    }

    #[test]
    fn hits_are_range_checked_capped_and_kill() {
        let mut s = island();
        s.apply_input("a", Input::Join { x: 0.0, y: 0.0, z: 0.0 })
            .unwrap();
        s.apply_input("b", Input::Join { x: 10.0, y: 0.0, z: 0.0 })
            .unwrap();
        s.apply_input("far", Input::Join { x: 0.0, y: 0.0, z: 0.0 })
            .unwrap();
        let (b, far) = (s.players["b"].entity, s.players["far"].entity);
        s.players.get_mut("far").unwrap().pos = [0.0, 0.0, 300.0];
        assert!(s.apply_input("a", Input::Hit { target: far, damage: 10 }).is_err());
        s.apply_input("a", Input::Hit { target: b, damage: 255 })
            .unwrap();
        assert_eq!(s.players["b"].health, 100 - MAX_DAMAGE);
        s.apply_input("a", Input::Hit { target: b, damage: 60 })
            .unwrap();
        assert_eq!(s.players["b"].health, 0);
        assert_eq!(s.store.get(b).unwrap().component(HEALTH), Some(&[0][..]));
        // The dead neither shoot nor move.
        let a = s.players["a"].entity;
        assert!(s.apply_input("b", Input::Hit { target: a, damage: 10 }).is_err());
        let before = pos(&s, "b");
        s.apply_input("b", mv(1.0, 0.0)).unwrap();
        assert_eq!(pos(&s, "b"), before);
    }

    #[test]
    fn only_the_dead_respawn_and_only_after_the_delay() {
        let mut s = island();
        s.apply_input("a", Input::Join { x: 0.0, y: 0.0, z: 0.0 })
            .unwrap();
        s.apply_input("b", Input::Join { x: 1.0, y: 0.0, z: 0.0 })
            .unwrap();
        let spawn = Input::Spawn { x: 50.0, y: 5.0, z: 50.0 };
        assert!(s.apply_input("b", spawn.clone()).is_err());
        let b = s.players["b"].entity;
        s.apply_input("a", Input::Hit { target: b, damage: 60 })
            .unwrap();
        s.apply_input("a", Input::Hit { target: b, damage: 60 })
            .unwrap();
        assert!(s.apply_input("b", spawn.clone()).is_err());
        for _ in 0..50 {
            s.tick(TICK);
        }
        s.apply_input("b", spawn).unwrap();
        assert_eq!(pos(&s, "b"), [50.0, 5.0, 50.0]);
        assert_eq!(s.players["b"].health, 100);
    }

    #[test]
    fn a_quiet_player_leaves() {
        let mut s = island();
        s.apply_input("a", Input::Join { x: 0.0, y: 0.0, z: 0.0 })
            .unwrap();
        for _ in 0..(IDLE_LIMIT.as_millis() / TICK.as_millis() - 1) {
            s.tick(TICK);
        }
        assert_eq!(s.store.len(), 1);
        s.apply_input("a", mv(0.0, 0.0)).unwrap();
        for _ in 0..(IDLE_LIMIT.as_millis() / TICK.as_millis()) {
            s.tick(TICK);
        }
        assert!(s.store.is_empty());
    }

    #[test]
    fn look_wraps_the_heading_and_clamps_the_pitch() {
        let decode = |b: [u8; 4]| {
            (
                i16::from_le_bytes([b[0], b[1]]) as f32 / 10_000.0,
                i16::from_le_bytes([b[2], b[3]]) as f32 / 10_000.0,
            )
        };
        let (h, p) = decode(encode_look(7.0, 3.0));
        assert!((h - (7.0 - std::f32::consts::TAU)).abs() < 1e-3);
        assert!((p - 1.55).abs() < 1e-3);
    }
}
