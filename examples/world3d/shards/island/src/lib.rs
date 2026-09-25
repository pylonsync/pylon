//! The island shard: player poses and combat for World3D.
//!
//! Each player is a replicated entity. Clients move their own player
//! (they run the physics against the terrain, which only they have) by
//! sending the distance moved since the last input. The shard applies it
//! within speed budgets and the island bounds: a client cannot teleport,
//! outrun the others, or climb faster than a rocket jump. The shard has no
//! terrain, so it cannot tell a player hovering in the air from one
//! standing on a roof; the height cap bounds that.
//!
//! The shard picks every spawn point (spawns.json, the points the client's
//! worldgen finds for the fixed seed), keeps health, range-checks hits,
//! and limits the damage a shooter deals per second. A player who leaves
//! stays in the world, where others can shoot them, for 5 seconds; one who
//! joins again within a minute comes back where they were, with the health
//! they had.
//!
//! Subscribers are Avatar row ids: `joinIsland` mints the ticket with the
//! caller's avatar id as the subscriber id, and only ticket holders join.
//!
//! Runs inside the Pylon server as a WebAssembly module (see app.ts).
//! `pylon dev` rebuilds it when this file changes.

// The exports exist only in the wasm build; a native build (`cargo test`)
// sees the types as unused.
#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use std::collections::HashMap;
use std::time::Duration;

use pylon_shard_guest::{export_shard, Auth, Replicated, ReplicationConfig, Shard};
use serde::Deserialize;

/// Component: the Avatar row id (UTF-8), for the name and color.
const AVATAR: u8 = 1;
/// Component: heading and pitch, each an i16 LE of radians x 10 000.
const LOOK: u8 = 2;
/// Component: health, one byte, 0 to 100.
const HEALTH: u8 = 3;

/// Ground speed the budget allows, in m/s: sprinting (11.5) with slack for
/// frame timing.
const GROUND_SPEED: f64 = 18.0;
const GROUND_SAVED: f64 = GROUND_SPEED;
/// Sustained climbing speed: the player controller has no slope limit, so a
/// sprint (11.5 m/s) up a steep hill climbs nearly as fast. The most saved
/// up for one climb: a rocket jump (jump plus a grenade's lift) rises about
/// 13 m.
const UP_SPEED: f64 = 12.0;
const UP_SAVED: f64 = 14.0;
/// Falling: gravity is 22 m/s^2 and the tallest drop is about 90 m.
const DOWN_SPEED: f64 = 45.0;
const DOWN_SAVED: f64 = DOWN_SPEED;
/// The island is 512 m across, centered on the origin.
const HALF_WORLD: f32 = 250.0;
const MIN_Y: f32 = -10.0;
/// The terrain peaks near 60 m; add rooftops and a rocket jump.
const MAX_Y: f32 = 90.0;
/// Weapon range (220 m) plus slack for grenade throws and pose lag.
const HIT_RANGE: f64 = 280.0;
const MAX_DAMAGE: u8 = 60;
/// Damage one shooter may deal per second, and at once: the rifle (18 per
/// 105 ms) is about 170 per second, and a grenade can hit several players
/// for up to 55 each (five for 275).
const DAMAGE_PER_SEC: f64 = 200.0;
const DAMAGE_SAVED: f64 = 300.0;
/// A dead player may spawn again after this long.
const RESPAWN_DELAY: Duration = Duration::from_millis(2500);
/// A player who sends nothing for this long leaves.
const IDLE_LIMIT: Duration = Duration::from_secs(15);
/// A player who leaves stays in the world this long, so leaving is no
/// escape from a fight. Joining again in the meantime cancels it.
const LINGER: Duration = Duration::from_secs(5);
/// A player who joins again within this long comes back where they were,
/// with their health.
const REMEMBER_LEFT: Duration = Duration::from_secs(60);

#[derive(Deserialize)]
struct Params {}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Input {
    /// Enter the island. Ignored when already in.
    Join,
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
    /// Come back after dying, at a spawn point the shard picks.
    Spawn,
    /// Leave the island.
    Leave,
}

struct Player {
    entity: u64,
    pos: [f32; 3],
    health: u8,
    /// Meters the player may still move: ground, up, down.
    moves: [f64; 3],
    /// Damage the player may still deal.
    damage: f64,
    idle: Duration,
    /// Time since health reached 0.
    dead_for: Duration,
    /// Time since the player left, while they linger.
    leaving: Option<Duration>,
}

/// What the shard keeps of a player who left.
struct Departed {
    pos: [f32; 3],
    health: u8,
    dead_for: Duration,
    gone_for: Duration,
}

struct Island {
    store: Replicated,
    players: HashMap<String, Player>,
    departed: HashMap<String, Departed>,
    spawns: Vec<[f32; 3]>,
    next_spawn: usize,
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

/// The share of `wanted` meters that `left` meters of budget allows, in
/// [0, 1]. A budget rounded just below zero allows nothing.
fn allowed(wanted: f64, left: f64) -> f64 {
    let left = left.max(0.0);
    if wanted > left {
        left / wanted
    } else {
        1.0
    }
}

impl Island {
    fn spawn_point(&mut self) -> [f32; 3] {
        let p = self.spawns[self.next_spawn % self.spawns.len()];
        self.next_spawn = self.next_spawn.wrapping_add(1);
        p
    }

    fn join(&mut self, subscriber: &str) {
        if let Some(p) = self.players.get_mut(subscriber) {
            p.leaving = None;
            return;
        }
        let (pos, health, dead_for) = match self.departed.remove(subscriber) {
            Some(d) => (d.pos, d.health, d.dead_for),
            None => (self.spawn_point(), 100, Duration::ZERO),
        };
        let entity = self.next_entity;
        self.next_entity += 1;
        self.store.spawn(entity, pos);
        self.store
            .set_component(entity, AVATAR, subscriber.as_bytes());
        self.store
            .set_component(entity, LOOK, &encode_look(0.0, 0.0));
        self.store.set_component(entity, HEALTH, &[health]);
        self.players.insert(
            subscriber.to_string(),
            Player {
                entity,
                pos,
                health,
                moves: [GROUND_SAVED, UP_SAVED, DOWN_SAVED],
                damage: DAMAGE_SAVED,
                idle: Duration::ZERO,
                dead_for,
                leaving: None,
            },
        );
    }

    /// Start the player's linger; `tick` removes them after it.
    fn leave(&mut self, subscriber: &str) {
        if let Some(p) = self.players.get_mut(subscriber) {
            p.leaving.get_or_insert(Duration::ZERO);
        }
    }

    fn remove(&mut self, subscriber: &str) {
        if let Some(p) = self.players.remove(subscriber) {
            self.store.despawn(p.entity);
            self.departed.insert(
                subscriber.to_string(),
                Departed {
                    pos: p.pos,
                    health: p.health,
                    dead_for: p.dead_for,
                    gone_for: Duration::ZERO,
                },
            );
        }
    }

    /// The player, if they are in and not leaving.
    fn player_mut(&mut self, subscriber: &str) -> Result<&mut Player, String> {
        match self.players.get_mut(subscriber) {
            Some(p) if p.leaving.is_none() => Ok(p),
            Some(_) => Err("left the island".into()),
            None => Err("join the island first".into()),
        }
    }
}

impl Shard for Island {
    type Params = Params;
    type Input = Input;
    type Snapshot = ();

    fn init(_shard_id: &str, _params: Params) -> Result<Self, String> {
        let spawns: Vec<[f32; 3]> = serde_json::from_str(include_str!("../spawns.json"))
            .map_err(|e| format!("spawns.json: {e}"))?;
        if spawns.is_empty() {
            return Err("spawns.json lists no spawn points".into());
        }
        Ok(Island {
            store: Replicated::new(),
            players: HashMap::new(),
            departed: HashMap::new(),
            spawns: spawns.into_iter().map(clamp_pos).collect(),
            next_spawn: 0,
            next_entity: 1,
        })
    }

    /// Only a ticket from joinIsland admits a player: its subscriber id is
    /// the caller's Avatar row id.
    fn authorize_subscribe(&self, _subscriber: &str, auth: &Auth) -> Result<(), String> {
        if auth.ticket.is_some() {
            Ok(())
        } else {
            Err("join through joinIsland".into())
        }
    }

    fn apply_input(&mut self, subscriber: &str, input: Input) -> Result<(), String> {
        if let Some(p) = self.players.get_mut(subscriber) {
            p.idle = Duration::ZERO;
        }
        match input {
            Input::Join => self.join(subscriber),
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
                // In f64: no finite f32 overflows when squared.
                let (dx, dy, dz) = (dx as f64, dy as f64, dz as f64);
                let ground = dx.hypot(dz);
                let g = allowed(ground, p.moves[0]);
                let vertical = if dy >= 0.0 { 1 } else { 2 };
                let v = allowed(dy.abs(), p.moves[vertical]);
                p.moves[0] = (p.moves[0] - ground * g).max(0.0);
                p.moves[vertical] = (p.moves[vertical] - dy.abs() * v).max(0.0);
                let next = clamp_pos([
                    (p.pos[0] as f64 + dx * g) as f32,
                    (p.pos[1] as f64 + dy * v) as f32,
                    (p.pos[2] as f64 + dz * g) as f32,
                ]);
                if !next.iter().all(|c| c.is_finite()) {
                    return Err("the move does not add up".into());
                }
                p.pos = next;
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
                let damage = damage.clamp(1, MAX_DAMAGE);
                if (damage as f64) > shooter.damage {
                    return Err("firing faster than the weapons allow".into());
                }
                let (from, own) = (shooter.pos, shooter.entity);
                if target == own {
                    return Ok(());
                }
                let Some(victim) = self.players.values().find(|p| p.entity == target) else {
                    return Err(format!("no player {target}"));
                };
                let d = [0, 1, 2].map(|i| victim.pos[i] as f64 - from[i] as f64);
                // A NaN distance fails too.
                let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                if dist.is_nan() || dist > HIT_RANGE {
                    return Err("target out of weapon range".into());
                }
                if victim.health == 0 {
                    return Ok(());
                }
                self.player_mut(subscriber)?.damage -= damage as f64;
                let victim = self
                    .players
                    .values_mut()
                    .find(|p| p.entity == target)
                    .expect("found above");
                victim.health = victim.health.saturating_sub(damage);
                victim.dead_for = Duration::ZERO;
                let (entity, health) = (victim.entity, victim.health);
                self.store.set_component(entity, HEALTH, &[health]);
            }
            Input::Spawn => {
                let p = self.player_mut(subscriber)?;
                if p.health > 0 {
                    return Err("only the dead respawn".into());
                }
                if p.dead_for < RESPAWN_DELAY {
                    return Err("too soon to respawn".into());
                }
                let pos = self.spawn_point();
                let p = self.player_mut(subscriber)?;
                p.health = 100;
                p.pos = pos;
                p.moves = [GROUND_SAVED, UP_SAVED, DOWN_SAVED];
                let entity = p.entity;
                self.store.set_pos(entity, pos);
                self.store.set_component(entity, HEALTH, &[100]);
            }
            Input::Leave => self.leave(subscriber),
        }
        Ok(())
    }

    fn tick(&mut self, dt: Duration) {
        let secs = dt.as_secs_f64();
        let mut idle = Vec::new();
        let mut gone = Vec::new();
        for (id, p) in &mut self.players {
            let earn = |left: f64, rate: f64, cap: f64| (left + rate * secs).min(cap);
            p.moves[0] = earn(p.moves[0], GROUND_SPEED, GROUND_SAVED);
            p.moves[1] = earn(p.moves[1], UP_SPEED, UP_SAVED);
            p.moves[2] = earn(p.moves[2], DOWN_SPEED, DOWN_SAVED);
            p.damage = earn(p.damage, DAMAGE_PER_SEC, DAMAGE_SAVED);
            p.idle += dt;
            if p.health == 0 {
                p.dead_for += dt;
            }
            if let Some(t) = &mut p.leaving {
                *t += dt;
                if *t >= LINGER {
                    gone.push(id.clone());
                }
            } else if p.idle >= IDLE_LIMIT {
                idle.push(id.clone());
            }
        }
        for id in idle {
            self.leave(&id);
        }
        for id in gone {
            self.remove(&id);
        }
        self.departed.retain(|_, d| {
            d.gone_for += dt;
            if d.health == 0 {
                d.dead_for += dt;
            }
            d.gone_for < REMEMBER_LEFT
        });
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

    fn mv(dx: f32, dy: f32, dz: f32) -> Input {
        Input::Move {
            dx,
            dy,
            dz,
            heading: 0.0,
            pitch: 0.0,
        }
    }

    fn pos(s: &Island, sub: &str) -> [f32; 3] {
        s.store.get(s.players[sub].entity).unwrap().pos
    }

    fn ticks(s: &mut Island, n: u32) {
        for _ in 0..n {
            s.tick(TICK);
        }
    }

    /// Joins `sub` and puts them at `at`.
    fn place(s: &mut Island, sub: &str, at: [f32; 3]) {
        s.apply_input(sub, Input::Join).unwrap();
        s.players.get_mut(sub).unwrap().pos = at;
    }

    #[test]
    fn players_join_at_the_shards_spawn_points_and_leave() {
        let mut s = island();
        s.apply_input("a", Input::Join).unwrap();
        s.apply_input("b", Input::Join).unwrap();
        assert_eq!(pos(&s, "a"), s.spawns[0]);
        assert_eq!(pos(&s, "b"), s.spawns[1]);
        let e = s.players["a"].entity;
        assert_eq!(s.store.get(e).unwrap().component(AVATAR), Some(&b"a"[..]));
        assert_eq!(s.store.get(e).unwrap().component(HEALTH), Some(&[100][..]));
        s.apply_input("a", Input::Leave).unwrap();
        assert!(s.apply_input("a", mv(1.0, 0.0, 0.0)).is_err());
        // It lingers, then goes.
        assert_eq!(s.store.len(), 2);
        ticks(&mut s, (LINGER.as_millis() / TICK.as_millis()) as u32);
        assert_eq!(s.store.len(), 1);
    }

    #[test]
    fn only_ticket_holders_subscribe() {
        let s = island();
        let signed_in = Auth {
            user_id: Some("a".into()),
            ..Default::default()
        };
        assert!(s.authorize_subscribe("a", &signed_in).is_err());
        let with_ticket = Auth {
            ticket: Some(pylon_shard_guest::Ticket {
                shard: "island-main".into(),
                sid: "a".into(),
                user_id: Some("u".into()),
                exp: u64::MAX,
                claims: serde_json::Value::Null,
            }),
            ..Default::default()
        };
        assert!(s.authorize_subscribe("a", &with_ticket).is_ok());
    }

    #[test]
    fn movement_is_held_to_the_speed_budget() {
        let mut s = island();
        place(&mut s, "a", [0.0, 0.0, 0.0]);
        // A 100 m jump goes only as far as the saved budget.
        s.apply_input("a", mv(100.0, 0.0, 0.0)).unwrap();
        assert_eq!(pos(&s, "a")[0], GROUND_SAVED as f32);
        // The budget is spent; one tick earns one tick of movement.
        s.tick(TICK);
        s.apply_input("a", mv(5.0, 0.0, 0.0)).unwrap();
        let x = pos(&s, "a")[0];
        assert!(
            (x - (GROUND_SAVED + GROUND_SPEED * 0.05) as f32).abs() < 1e-3,
            "{x}"
        );
        // Sprinting at 20 inputs a second stays within it.
        for _ in 0..100 {
            s.tick(TICK);
            s.apply_input("a", mv(11.5 / 20.0, 0.0, 0.0)).unwrap();
        }
        let x2 = pos(&s, "a")[0];
        assert!((x2 - x - 57.5).abs() < 1e-2, "{x2}");
    }

    #[test]
    fn a_huge_move_neither_overflows_nor_frees_the_next_one() {
        let mut s = island();
        place(&mut s, "a", [0.0, 0.0, 0.0]);
        s.apply_input("a", mv(1e20, 1e20, 1e20)).unwrap();
        s.apply_input("a", mv(f32::MAX, -f32::MAX, f32::MAX))
            .unwrap();
        // The budgets are spent, not NaN: the next move goes nowhere.
        assert!(s.players["a"].moves.iter().all(|m| m.is_finite()));
        let before = pos(&s, "a");
        s.apply_input("a", mv(240.0, 0.0, 0.0)).unwrap();
        assert_eq!(pos(&s, "a"), before);
        let p = pos(&s, "a");
        assert!(p[0].hypot(p[2]) <= (GROUND_SAVED + 1e-3) as f32, "{p:?}");
        assert!(p[1] <= UP_SAVED as f32 + 1e-3, "{p:?}");
    }

    #[test]
    fn climbing_is_slow_and_the_height_is_capped() {
        let mut s = island();
        place(&mut s, "a", [0.0, 0.0, 0.0]);
        // Straight up for 10 s at 45 m/s: at most the saved climb plus the
        // climbing speed.
        for _ in 0..200 {
            s.apply_input("a", mv(0.0, 45.0 / 20.0, 0.0)).unwrap();
            s.tick(TICK);
        }
        let y = pos(&s, "a")[1];
        assert!(y <= (UP_SAVED + UP_SPEED * 10.0) as f32 + 0.5, "{y}");
        for _ in 0..600 {
            s.apply_input("a", mv(0.0, 45.0 / 20.0, 0.0)).unwrap();
            s.tick(TICK);
        }
        assert_eq!(pos(&s, "a")[1], MAX_Y);
        // Falling is fast.
        s.apply_input("a", mv(0.0, -40.0, 0.0)).unwrap();
        assert_eq!(pos(&s, "a")[1], MAX_Y - 40.0);
    }

    #[test]
    fn positions_stay_on_the_island_and_numbers_must_be_finite() {
        let mut s = island();
        place(&mut s, "a", [HALF_WORLD - 1.0, 0.0, 0.0]);
        s.apply_input("a", mv(10.0, 0.0, 0.0)).unwrap();
        assert_eq!(pos(&s, "a")[0], HALF_WORLD);
        assert!(s.apply_input("a", mv(f32::NAN, 0.0, 0.0)).is_err());
        assert!(s.apply_input("a", mv(0.0, f32::INFINITY, 0.0)).is_err());
    }

    #[test]
    fn hits_are_range_checked_capped_and_kill() {
        let mut s = island();
        place(&mut s, "a", [0.0, 0.0, 0.0]);
        place(&mut s, "b", [10.0, 0.0, 0.0]);
        place(&mut s, "far", [0.0, 0.0, 300.0]);
        let (b, far) = (s.players["b"].entity, s.players["far"].entity);
        assert!(s
            .apply_input(
                "a",
                Input::Hit {
                    target: far,
                    damage: 10
                }
            )
            .is_err());
        s.apply_input(
            "a",
            Input::Hit {
                target: b,
                damage: 255,
            },
        )
        .unwrap();
        assert_eq!(s.players["b"].health, 100 - MAX_DAMAGE);
        ticks(&mut s, 10);
        s.apply_input(
            "a",
            Input::Hit {
                target: b,
                damage: 60,
            },
        )
        .unwrap();
        assert_eq!(s.players["b"].health, 0);
        assert_eq!(s.store.get(b).unwrap().component(HEALTH), Some(&[0][..]));
        // The dead neither shoot nor move.
        let a = s.players["a"].entity;
        assert!(s
            .apply_input(
                "b",
                Input::Hit {
                    target: a,
                    damage: 10
                }
            )
            .is_err());
        let before = pos(&s, "b");
        s.apply_input("b", mv(1.0, 0.0, 0.0)).unwrap();
        assert_eq!(pos(&s, "b"), before);
    }

    #[test]
    fn a_shooter_cannot_deal_more_than_the_weapons_allow() {
        let mut s = island();
        place(&mut s, "a", [0.0, 0.0, 0.0]);
        let targets: Vec<u64> = (0..16)
            .map(|i| {
                let id = format!("t{i}");
                place(&mut s, &id, [i as f32, 0.0, 5.0]);
                s.players[&id].entity
            })
            .collect();
        // Everything at once, in one tick: the saved 300 damage, no more.
        let mut dealt = 0;
        for &t in &targets {
            for _ in 0..2 {
                if s.apply_input(
                    "a",
                    Input::Hit {
                        target: t,
                        damage: 60,
                    },
                )
                .is_ok()
                {
                    dealt += 60;
                }
            }
        }
        assert_eq!(dealt, 300);
        let dead = s.players.values().filter(|p| p.health == 0).count();
        assert_eq!(dead, 2);
        // A second of rifle fire (about 10 shots of 18) goes through.
        ticks(&mut s, 20);
        let t = targets[9];
        let mut hits = 0;
        for _ in 0..10 {
            if s.apply_input(
                "a",
                Input::Hit {
                    target: t,
                    damage: 18,
                },
            )
            .is_ok()
            {
                hits += 1;
            }
            ticks(&mut s, 2);
        }
        assert_eq!(hits, 10);
        assert_eq!(s.players["t9"].health, 0);
    }

    #[test]
    fn only_the_dead_respawn_only_after_the_delay_at_a_spawn_point() {
        let mut s = island();
        place(&mut s, "a", [0.0, 0.0, 0.0]);
        place(&mut s, "b", [1.0, 0.0, 0.0]);
        assert!(s.apply_input("b", Input::Spawn).is_err());
        s.players.get_mut("b").unwrap().health = 0;
        assert!(s.apply_input("b", Input::Spawn).is_err());
        ticks(&mut s, 50);
        s.apply_input("b", Input::Spawn).unwrap();
        assert!(s.spawns.contains(&pos(&s, "b")));
        assert_eq!(s.players["b"].health, 100);
    }

    #[test]
    fn leaving_is_no_escape_and_joining_again_neither_heals_nor_moves_a_player() {
        let linger = (LINGER.as_millis() / TICK.as_millis()) as u32;
        let mut s = island();
        place(&mut s, "a", [-235.0, 5.0, 235.0]);
        place(&mut s, "b", [-240.0, 5.0, 240.0]);
        let b = s.players["b"].entity;
        // B leaves under fire: still there to be shot for the linger.
        s.apply_input("b", Input::Leave).unwrap();
        s.apply_input(
            "a",
            Input::Hit {
                target: b,
                damage: 60,
            },
        )
        .unwrap();
        ticks(&mut s, 10);
        s.apply_input(
            "a",
            Input::Hit {
                target: b,
                damage: 60,
            },
        )
        .unwrap();
        assert_eq!(s.players["b"].health, 0);
        // Leaving players neither move nor shoot.
        assert!(s.apply_input("b", mv(1.0, 0.0, 0.0)).is_err());
        ticks(&mut s, linger);
        assert!(!s.players.contains_key("b"));
        // Within a minute: back where they were, still dead.
        s.apply_input("b", Input::Join).unwrap();
        assert_eq!(pos(&s, "b"), [-240.0, 5.0, 240.0]);
        assert_eq!(s.players["b"].health, 0);
        // Joining again during the linger cancels it.
        s.players.get_mut("b").unwrap().health = 70;
        s.apply_input("b", Input::Leave).unwrap();
        s.apply_input("b", Input::Join).unwrap();
        ticks(&mut s, linger);
        assert_eq!(s.players["b"].health, 70);
        // The idle kick is a leave too.
        s.apply_input("a", Input::Leave).unwrap();
        ticks(
            &mut s,
            (IDLE_LIMIT.as_millis() / TICK.as_millis()) as u32 + linger,
        );
        assert!(!s.players.contains_key("b"));
        s.apply_input("b", Input::Join).unwrap();
        assert_eq!(s.players["b"].health, 70);
        // After a minute away the record is gone: a fresh spawn.
        s.apply_input("b", Input::Leave).unwrap();
        ticks(
            &mut s,
            linger + (REMEMBER_LEFT.as_millis() / TICK.as_millis()) as u32 + 1,
        );
        assert!(s.departed.is_empty());
        s.apply_input("b", Input::Join).unwrap();
        assert!(s.spawns.contains(&pos(&s, "b")));
        assert_eq!(s.players["b"].health, 100);
    }

    #[test]
    fn a_budget_spent_to_rounding_error_moves_nothing_and_never_nan() {
        let mut s = island();
        place(&mut s, "a", [0.0, 5.0, 0.0]);
        // Spend each budget with moves that do not divide evenly.
        s.apply_input("a", mv(23.47, 16.51, 23.47)).unwrap();
        for m in [
            mv(0.0, 1.0, 0.0),
            mv(1.0, 0.0, 0.0),
            mv(0.0, -0.0, 0.0),
            mv(0.3, 0.7, -0.2),
        ] {
            s.apply_input("a", m).unwrap();
            assert!(
                pos(&s, "a").iter().all(|c| c.is_finite()),
                "{:?}",
                pos(&s, "a")
            );
            assert!(s.players["a"]
                .moves
                .iter()
                .all(|m| m.is_finite() && *m >= 0.0));
        }
        // Many uneven moves over many ticks stay finite.
        for i in 0..2000 {
            s.apply_input("a", mv(0.37 + (i % 7) as f32, 0.53, -0.29))
                .unwrap();
            s.apply_input("a", mv(0.0, -0.11, 0.0)).unwrap();
            if i % 3 == 0 {
                s.tick(TICK);
            }
            assert!(pos(&s, "a").iter().all(|c| c.is_finite()));
        }
        // A player far away cannot be hit.
        place(&mut s, "t", [250.0, 0.0, 250.0]);
        let t = s.players["t"].entity;
        s.players.get_mut("a").unwrap().pos = [-250.0, 90.0, -250.0];
        assert!(s
            .apply_input(
                "a",
                Input::Hit {
                    target: t,
                    damage: 10
                }
            )
            .is_err());
    }

    #[test]
    fn a_quiet_player_leaves() {
        let mut s = island();
        s.apply_input("a", Input::Join).unwrap();
        ticks(
            &mut s,
            (IDLE_LIMIT.as_millis() / TICK.as_millis() - 1) as u32,
        );
        assert_eq!(s.store.len(), 1);
        s.apply_input("a", mv(0.0, 0.0, 0.0)).unwrap();
        ticks(&mut s, (IDLE_LIMIT.as_millis() / TICK.as_millis()) as u32);
        // Kicked: it lingers, then goes.
        assert!(s.apply_input("a", mv(1.0, 0.0, 0.0)).is_err());
        ticks(&mut s, (LINGER.as_millis() / TICK.as_millis()) as u32);
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
