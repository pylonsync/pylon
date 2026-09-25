//! A zone: players with hit points, buffs, and cooldowns move between zones
//! and keep all of it (see `Shard::transfer_out` and `transfer_in` in
//! pylon-shard-guest). `functions/moveZone.ts` moves a player; a zone with
//! `edge` and `next` moves one that walks past the edge on its own.
//!
//! Params: `closed` refuses players coming in; `edge` and `next` make a zone
//! line: a player whose x reaches `edge` is moved to shard `next`.

use std::collections::BTreeMap;
use std::time::Duration;

use pylon_shard_guest::{export_shard, Shard};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct Params {
    #[serde(default)]
    closed: bool,
    #[serde(default)]
    edge: Option<i64>,
    #[serde(default)]
    next: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
struct Buff {
    name: String,
    remaining_ms: u64,
}

/// Everything that moves with the player.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
struct Player {
    x: i64,
    hp: i64,
    buffs: Vec<Buff>,
    /// Ability name to milliseconds until it can be used again.
    cooldowns: BTreeMap<String, u64>,
}

#[derive(Serialize)]
struct Snapshot {
    zone: String,
    players: BTreeMap<String, Player>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Input {
    Join,
    Move { dx: i64 },
    Buff { name: String, ms: u64 },
    Cast { ability: String, cooldown_ms: u64 },
    Hit { damage: i64 },
}

struct Zone {
    id: String,
    params: Params,
    players: BTreeMap<String, Player>,
    leaving: Vec<(String, String)>,
}

fn count_down(ms: &mut u64, dt: Duration) {
    *ms = ms.saturating_sub(dt.as_millis() as u64);
}

impl Shard for Zone {
    type Params = Params;
    type Input = Input;
    type Snapshot = Snapshot;

    fn init(shard_id: &str, params: Params) -> Result<Self, String> {
        Ok(Zone {
            id: shard_id.to_string(),
            params,
            players: BTreeMap::new(),
            leaving: Vec::new(),
        })
    }

    fn apply_input(&mut self, subscriber: &str, input: Input) -> Result<(), String> {
        if let Input::Join = input {
            self.players.entry(subscriber.to_string()).or_insert(Player {
                x: 0,
                hp: 100,
                buffs: Vec::new(),
                cooldowns: BTreeMap::new(),
            });
            return Ok(());
        }
        let p = self
            .players
            .get_mut(subscriber)
            .ok_or("join first")?;
        match input {
            Input::Join => {}
            Input::Move { dx } => p.x += dx.clamp(-10, 10),
            Input::Buff { name, ms } => p.buffs.push(Buff { name, remaining_ms: ms }),
            Input::Cast {
                ability,
                cooldown_ms,
            } => {
                if p.cooldowns.get(&ability).is_some_and(|&ms| ms > 0) {
                    return Err(format!("{ability} is cooling down"));
                }
                p.cooldowns.insert(ability, cooldown_ms);
            }
            Input::Hit { damage } => p.hp -= damage,
        }
        Ok(())
    }

    fn tick(&mut self, dt: Duration) {
        for (sid, p) in &mut self.players {
            for b in &mut p.buffs {
                count_down(&mut b.remaining_ms, dt);
            }
            p.buffs.retain(|b| b.remaining_ms > 0);
            for ms in p.cooldowns.values_mut() {
                count_down(ms, dt);
            }
            if let (Some(edge), Some(next)) = (self.params.edge, &self.params.next) {
                if p.x >= edge && !self.leaving.iter().any(|(s, _)| s == sid) {
                    self.leaving.push((sid.clone(), next.clone()));
                }
            }
        }
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            zone: self.id.clone(),
            players: self.players.clone(),
        }
    }

    fn transfer_out(&mut self, subscriber: &str) -> Result<Option<Vec<u8>>, String> {
        self.leaving.retain(|(s, _)| s != subscriber);
        match self.players.remove(subscriber) {
            Some(p) => serde_json::to_vec(&p).map(Some).map_err(|e| e.to_string()),
            None => Ok(None),
        }
    }

    fn transfer_in(&mut self, subscriber: &str, state: &[u8], returning: bool) -> Result<(), String> {
        // A closed zone keeps newcomers out, not its own players coming back.
        if self.params.closed && !returning {
            return Err(format!("zone {} is closed", self.id));
        }
        let p: Player = serde_json::from_slice(state).map_err(|e| e.to_string())?;
        self.players.insert(subscriber.to_string(), p);
        Ok(())
    }

    fn transfer_requests(&mut self) -> Vec<(String, String)> {
        // Asked once each; a refused player that is still past the edge is
        // asked again next tick.
        std::mem::take(&mut self.leaving)
    }

    fn save(&self) -> Option<Vec<u8>> {
        serde_json::to_vec(&self.players).ok()
    }

    fn restore(shard_id: &str, params: Params, state: &[u8]) -> Result<Self, String> {
        let mut zone = Self::init(shard_id, params)?;
        zone.players = serde_json::from_slice(state).map_err(|e| e.to_string())?;
        Ok(zone)
    }
}

export_shard!(Zone);
