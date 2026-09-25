//! A small arena shard: players move on a grid.
//!
//! Build: `cargo build -p pylon-shard-guest --example arena --release
//! --target wasm32-unknown-unknown`. The runtime's WASM shard tests and
//! `tools/smoke-wasm-shard.sh` load the result.
//!
//! Besides moves, it takes inputs that exercise the host's failure
//! handling: `panic`, `spin` (never returns), `grow` (allocates until the
//! memory limit), and `burn` (loops a given number of times).

// The exports exist only in the wasm build; a native build (`cargo test`)
// sees the types as unused.
#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use std::time::Duration;

use pylon_shard_guest::{
    export_shard, log, Auth, EntityPos, InterestArea, InterestConfig, Level, Shard, View,
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct Params {
    /// Hide other players' positions from each subscriber.
    #[serde(default)]
    fog: bool,
    #[serde(default = "default_label")]
    label: String,
    /// Turn on the host's interest management with this view radius.
    /// Players named "ghost..." are hidden except from "seer..." subscribers.
    #[serde(default)]
    view_radius: Option<f32>,
}

fn default_label() -> String {
    "arena".into()
}

#[derive(Serialize, Clone)]
struct Player {
    id: String,
    x: i64,
    y: i64,
}

#[derive(Serialize)]
struct Snapshot {
    label: String,
    elapsed_ms: u64,
    players: Vec<Player>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Input {
    Move {
        dx: i64,
        dy: i64,
    },
    Finish,
    Panic,
    Spin,
    Grow,
    Burn {
        iters: u64,
    },
    /// Panics in authorize_input.
    TrapAuth,
}

struct Arena {
    label: String,
    fog: bool,
    view_radius: Option<f32>,
    elapsed: Duration,
    players: Vec<Player>,
    finished: bool,
    hoard: Vec<Vec<u8>>,
}

impl Shard for Arena {
    type Params = Params;
    type Input = Input;
    type Snapshot = Snapshot;

    fn init(shard_id: &str, params: Params) -> Result<Self, String> {
        if params.label.is_empty() {
            return Err("label must not be empty".into());
        }
        log(Level::Info, &format!("arena {shard_id} starting"));
        Ok(Arena {
            label: params.label,
            fog: params.fog,
            view_radius: params.view_radius,
            elapsed: Duration::ZERO,
            players: Vec::new(),
            finished: false,
            hoard: Vec::new(),
        })
    }

    fn apply_input(&mut self, subscriber: &str, input: Input) -> Result<(), String> {
        match input {
            Input::Move { dx, dy } => {
                if dx.abs() > 10 || dy.abs() > 10 {
                    return Err(format!("move ({dx}, {dy}) is too far"));
                }
                match self.players.iter_mut().find(|p| p.id == subscriber) {
                    Some(p) => {
                        p.x += dx;
                        p.y += dy;
                    }
                    None => self.players.push(Player {
                        id: subscriber.to_string(),
                        x: dx,
                        y: dy,
                    }),
                }
            }
            Input::Finish => self.finished = true,
            Input::Panic => panic!("asked to panic"),
            Input::Spin => {
                let mut n: u64 = 0;
                loop {
                    n = std::hint::black_box(n.wrapping_add(1));
                }
            }
            Input::TrapAuth => {}
            Input::Burn { iters } => {
                let mut n: u64 = 0;
                for i in 0..iters {
                    n = std::hint::black_box(n.wrapping_add(i));
                }
                std::hint::black_box(n);
            }
            Input::Grow => loop {
                self.hoard.push(vec![1u8; 1 << 20]);
            },
        }
        Ok(())
    }

    fn tick(&mut self, dt: Duration) {
        self.elapsed += dt;
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            label: self.label.clone(),
            elapsed_ms: self.elapsed.as_millis() as u64,
            players: self.players.clone(),
        }
    }

    fn snapshot_for(&self, subscriber: &str) -> Option<Snapshot> {
        if !self.fog {
            return None;
        }
        let mut snap = self.snapshot();
        snap.players.retain(|p| p.id == subscriber);
        Some(snap)
    }

    fn interest(&self) -> Option<InterestConfig> {
        self.view_radius.map(|r| InterestConfig {
            cell_size: r,
            margin: 1.0,
            shared_snapshots: true,
        })
    }

    fn entities(&self, out: &mut Vec<EntityPos>) {
        // Players are never removed, so the index is a stable id.
        out.extend(self.players.iter().enumerate().map(|(i, p)| EntityPos {
            id: i as u64,
            x: p.x as f32,
            y: p.y as f32,
        }));
    }

    fn interest_area(&self, subscriber: &str) -> Option<InterestArea> {
        let me = self
            .players
            .iter()
            .find(|p| p.id == subscriber.trim_start_matches("seer-"))?;
        Some(InterestArea {
            x: me.x as f32,
            y: me.y as f32,
            radius: self.view_radius?,
        })
    }

    fn can_see(&self, subscriber: &str, entity: u64) -> bool {
        subscriber.starts_with("seer-") || !self.players[entity as usize].id.starts_with("ghost")
    }

    fn snapshot_visible(&self, _subscriber: &str, view: &View<'_>) -> Option<Snapshot> {
        let mut snap = self.snapshot();
        snap.players = view
            .visible
            .iter()
            .map(|&i| self.players[i as usize].clone())
            .collect();
        Some(snap)
    }

    fn is_finished(&self) -> bool {
        self.finished
    }

    fn authorize_input(&self, _subscriber: &str, auth: &Auth, input: &Input) -> Result<(), String> {
        if matches!(input, Input::TrapAuth) {
            panic!("asked to trap in authorize_input");
        }
        if auth.claim("role").and_then(|r| r.as_str()) == Some("spectator") {
            return Err("spectators cannot send inputs".into());
        }
        Ok(())
    }
}

export_shard!(Arena);
