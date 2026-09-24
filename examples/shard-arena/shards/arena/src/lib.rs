//! The arena shard: each player is a dot that glides to where they click.
//!
//! Runs inside the Pylon server as a WebAssembly module (see app.ts).
//! `pylon dev` rebuilds it when this file changes.

use std::time::Duration;

use pylon_shard_guest::{export_shard, Shard};
use serde::{Deserialize, Serialize};

/// Units per second a dot moves.
const SPEED: f32 = 240.0;
/// A player who sends nothing for this long leaves the arena.
const IDLE_LIMIT: Duration = Duration::from_secs(60);

#[derive(Deserialize)]
struct Params {
    width: f32,
    height: f32,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Input {
    /// Enter the arena, or stay in it.
    Join,
    MoveTo {
        x: f32,
        y: f32,
    },
}

#[derive(Serialize, Clone)]
struct Player {
    id: String,
    x: f32,
    y: f32,
    tx: f32,
    ty: f32,
    hue: u16,
    #[serde(skip)]
    idle: Duration,
}

#[derive(Serialize)]
struct Snapshot {
    width: f32,
    height: f32,
    players: Vec<Player>,
}

struct Arena {
    width: f32,
    height: f32,
    players: Vec<Player>,
}

/// A stable color per player, from the subscriber id.
fn hue_for(id: &str) -> u16 {
    let mut h: u32 = 2166136261;
    for b in id.bytes() {
        h = (h ^ b as u32).wrapping_mul(16777619);
    }
    (h % 360) as u16
}

impl Arena {
    fn player(&mut self, id: &str) -> &mut Player {
        if let Some(i) = self.players.iter().position(|p| p.id == id) {
            return &mut self.players[i];
        }
        // New players start in the middle.
        let (x, y) = (self.width / 2.0, self.height / 2.0);
        self.players.push(Player {
            id: id.to_string(),
            x,
            y,
            tx: x,
            ty: y,
            hue: hue_for(id),
            idle: Duration::ZERO,
        });
        self.players.last_mut().expect("just pushed")
    }
}

impl Shard for Arena {
    type Params = Params;
    type Input = Input;
    type Snapshot = Snapshot;

    fn init(_shard_id: &str, params: Params) -> Result<Self, String> {
        if !(params.width > 0.0 && params.height > 0.0) {
            return Err("width and height must be positive".into());
        }
        Ok(Arena {
            width: params.width,
            height: params.height,
            players: Vec::new(),
        })
    }

    fn apply_input(&mut self, subscriber: &str, input: Input) -> Result<(), String> {
        let (w, h) = (self.width, self.height);
        let p = self.player(subscriber);
        p.idle = Duration::ZERO;
        if let Input::MoveTo { x, y } = input {
            if !(0.0..=w).contains(&x) || !(0.0..=h).contains(&y) {
                return Err(format!("({x}, {y}) is outside the arena"));
            }
            p.tx = x;
            p.ty = y;
        }
        Ok(())
    }

    fn tick(&mut self, dt: Duration) {
        let step = SPEED * dt.as_secs_f32();
        for p in &mut self.players {
            let (dx, dy) = (p.tx - p.x, p.ty - p.y);
            let dist = (dx * dx + dy * dy).sqrt();
            if dist <= step {
                p.x = p.tx;
                p.y = p.ty;
            } else {
                p.x += dx / dist * step;
                p.y += dy / dist * step;
            }
            p.idle += dt;
        }
        self.players.retain(|p| p.idle < IDLE_LIMIT);
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            width: self.width,
            height: self.height,
            players: self.players.clone(),
        }
    }
}

export_shard!(Arena);
