//! A zone: players with hit points, buffs, and cooldowns move between zones
//! and keep all of it (see `Shard::transfer_out` and `transfer_in` in
//! pylon-shard-guest). `functions/moveZone.ts` moves a player; a zone with
//! `edge` and `next` moves one that walks past the edge on its own.
//!
//! Params: `closed` refuses players coming in; `edge` and `next` make a zone
//! line: a player whose x reaches `edge` is moved to shard `next`.
//!
//! The app's data (see `Shard::calls` and `writes`): a player's `Character`
//! row loads when it joins (`functions/loadCharacter.ts`); `loot` grants an
//! `Item` through `functions/grantItem.ts`, once per key, and the item shows
//! only after that mutation commits; x is written back every few seconds.
//! A zone restored from its last save loads its characters' items again:
//! a grant can have committed after that save.
//! `functions/gmHeal.ts` heals a player through `ctx.shards.send`.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use pylon_shard_guest::{export_shard, Auth, Call, Outgoing, Shard, Target, Write};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct Params {
    #[serde(default)]
    closed: bool,
    #[serde(default)]
    edge: Option<i64>,
    #[serde(default)]
    next: Option<String>,
    /// A group for messages (`shout` with `to: "group:<name>"`).
    #[serde(default)]
    group: Option<String>,
    /// Test hook: grant results wait for a `release` input, so every save
    /// holds the grant as sent (tools/smoke-shard-cluster.sh).
    #[serde(default)]
    hold_results: bool,
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
    /// True once the character row has loaded.
    #[serde(default)]
    loaded: bool,
    /// The character row's id.
    #[serde(default)]
    character: String,
    /// Items whose grant committed, by Item row id.
    #[serde(default)]
    items: BTreeMap<String, String>,
    /// The number of the character's next grant (its key), from the row.
    #[serde(default)]
    next_grant: u64,
    /// Milliseconds until a failed load is tried again.
    #[serde(default)]
    load_retry_ms: u64,
}

/// A grant sent to `grantItem` and not answered yet. Saved, so a zone that
/// restarts sends it again under the same key.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
struct Grant {
    player: String,
    character: String,
    item: String,
    /// Test hook: how long the mutation waits before it writes.
    #[serde(default)]
    delay_ms: u64,
    /// Restored from a save: its result is a replay.
    #[serde(default)]
    restored: bool,
}

/// What a zone saves.
#[derive(Serialize, Deserialize, Default)]
struct Saved {
    #[serde(default)]
    players: BTreeMap<String, Player>,
    /// Grants in flight, by key.
    #[serde(default)]
    grants: BTreeMap<String, Grant>,
}

#[derive(Serialize)]
struct Snapshot {
    zone: String,
    players: BTreeMap<String, Player>,
    /// The last shouts heard from other zones.
    heard: Vec<String>,
    /// Grant results applied to grants restored from a save.
    replayed: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Input {
    Join,
    Move {
        dx: i64,
    },
    Buff {
        name: String,
        ms: u64,
    },
    Cast {
        ability: String,
        cooldown_ms: u64,
    },
    Hit {
        damage: i64,
    },
    /// Send `text` to other zones: `to` is "all" (default),
    /// "group:<name>", or "shard:<id>".
    Shout {
        text: String,
        #[serde(default)]
        to: Option<String>,
    },
    /// Grant `item` (through the `grantItem` mutation).
    Loot {
        item: String,
        #[serde(default)]
        delay_ms: u64,
    },
    /// From the server only (`ctx.shards.send`): heal `who` by `hp`.
    Heal {
        who: String,
        hp: i64,
    },
    /// Apply the grant results `hold_results` kept.
    Release,
}

struct Zone {
    id: String,
    params: Params,
    players: BTreeMap<String, Player>,
    leaving: Vec<(String, String)>,
    heard: std::collections::VecDeque<String>,
    shouts: Vec<Outgoing>,
    /// Players past the edge whose move was asked for. Asked again only
    /// after they step back, so a refused move is not retried every tick.
    asked: BTreeSet<String>,
    /// Grants in flight, by key.
    grants: BTreeMap<String, Grant>,
    /// Players whose x changed since the last write.
    moved: BTreeSet<String>,
    /// Grant results waiting for `release` (with `hold_results`), by key.
    held: BTreeMap<String, Result<serde_json::Value, String>>,
    replayed: u64,
}

/// The key of a character load: a query, so it is not stored.
fn load_key(player: &str) -> String {
    format!("load:{player}")
}

fn count_down(ms: &mut u64, dt: Duration) {
    *ms = ms.saturating_sub(dt.as_millis() as u64);
}

impl Zone {
    /// Apply a grant's result: its item on `Ok`, the next key on
    /// `KEY_REUSED`, a log line on a refusal.
    fn apply_grant_result(&mut self, key: &str, result: Result<serde_json::Value, String>) {
        let Some(grant) = self.grants.remove(key) else {
            // Answered already (a result can arrive twice).
            return;
        };
        let restored = grant.restored;
        match result {
            Ok(done) => {
                // By row id: a reload may have shown it already.
                if let (Some(p), Some(id)) =
                    (self.players.get_mut(&grant.player), done["itemId"].as_str())
                {
                    p.items.insert(id.to_string(), grant.item);
                }
                if restored {
                    self.replayed += 1;
                }
            }
            // The key was used for another grant: a restored zone counted
            // from an older row. Send this grant again under the next key.
            Err(e) if e.starts_with("KEY_REUSED") => {
                if let Some(p) = self.players.get_mut(&grant.player) {
                    let key = format!("loot:{}:{}", p.character, p.next_grant);
                    p.next_grant += 1;
                    self.grants.insert(key, grant);
                }
            }
            // The function refused (the host tries database failures
            // again itself): the mutation rolled back, so no item exists.
            Err(e) => pylon_shard_guest::log(
                pylon_shard_guest::Level::Warn,
                &format!("grant {key} failed: {e}"),
            ),
        }
    }
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
            heard: Default::default(),
            shouts: Vec::new(),
            asked: Default::default(),
            grants: BTreeMap::new(),
            moved: BTreeSet::new(),
            held: BTreeMap::new(),
            replayed: 0,
        })
    }

    fn apply_input(&mut self, subscriber: &str, input: Input) -> Result<(), String> {
        if let Input::Join = input {
            // The empty id is the server's (ctx.shards.send), not a player.
            if subscriber.is_empty() {
                return Err("the server cannot join".into());
            }
            self.players
                .entry(subscriber.to_string())
                .or_insert(Player {
                    x: 0,
                    hp: 100,
                    buffs: Vec::new(),
                    cooldowns: BTreeMap::new(),
                    loaded: false,
                    character: String::new(),
                    items: BTreeMap::new(),
                    next_grant: 0,
                    load_retry_ms: 0,
                });
            return Ok(());
        }
        if let Input::Release = input {
            for (key, result) in std::mem::take(&mut self.held) {
                self.apply_grant_result(&key, result);
            }
            return Ok(());
        }
        if let Input::Heal { who, hp } = &input {
            if !subscriber.is_empty() {
                return Err("heal comes from the server only".into());
            }
            let p = self.players.get_mut(who).ok_or("no such player")?;
            p.hp += hp;
            return Ok(());
        }
        let p = self.players.get_mut(subscriber).ok_or("join first")?;
        match input {
            Input::Join | Input::Heal { .. } | Input::Release => {}
            Input::Move { dx } => {
                // x comes from the character row: no move before it loads.
                if !p.loaded {
                    return Err("the character has not loaded yet".into());
                }
                p.x += dx.clamp(-10, 10);
                self.moved.insert(subscriber.to_string());
            }
            Input::Loot { item, delay_ms } => {
                if !p.loaded {
                    return Err("the character has not loaded yet".into());
                }
                // The character's grant number makes the key: unique for
                // good, and the same when a restarted zone sends it again.
                let key = format!("loot:{}:{}", p.character, p.next_grant);
                p.next_grant += 1;
                self.grants.insert(
                    key,
                    Grant {
                        player: subscriber.to_string(),
                        character: p.character.clone(),
                        item,
                        delay_ms,
                        restored: false,
                    },
                );
            }
            Input::Buff { name, ms } => p.buffs.push(Buff {
                name,
                remaining_ms: ms,
            }),
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
            Input::Shout { text, to } => {
                let to = match to.as_deref() {
                    None | Some("all") => Target::All,
                    Some(t) => match t.split_once(':') {
                        Some(("group", name)) => Target::Group(name.to_string()),
                        Some(("shard", id)) => Target::Shard(id.to_string()),
                        _ => return Err(format!("cannot shout to {t}")),
                    },
                };
                self.shouts.push(Outgoing {
                    to,
                    topic: "shout".into(),
                    data: format!("{subscriber}: {text}").into_bytes(),
                });
            }
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
            count_down(&mut p.load_retry_ms, dt);
            if let (Some(edge), Some(next)) = (self.params.edge, &self.params.next) {
                if p.x < edge {
                    self.asked.remove(sid);
                } else if self.asked.insert(sid.clone()) {
                    self.leaving.push((sid.clone(), next.clone()));
                }
            }
        }
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            zone: self.id.clone(),
            players: self.players.clone(),
            heard: self.heard.iter().cloned().collect(),
            replayed: self.replayed,
        }
    }

    fn transfer_out(&mut self, subscriber: &str) -> Result<Option<Vec<u8>>, String> {
        // Its result comes back here: the player waits for it.
        if self.grants.values().any(|g| g.player == subscriber) {
            return Err("a grant for the player is in flight".into());
        }
        self.leaving.retain(|(s, _)| s != subscriber);
        self.asked.remove(subscriber);
        match self.players.remove(subscriber) {
            Some(p) => serde_json::to_vec(&p).map(Some).map_err(|e| e.to_string()),
            None => Ok(None),
        }
    }

    fn transfer_in(
        &mut self,
        subscriber: &str,
        state: &[u8],
        _auth: &Auth,
        returning: bool,
    ) -> Result<(), String> {
        // A closed zone keeps newcomers out, not its own players coming back.
        if self.params.closed && !returning {
            return Err(format!("zone {} is closed", self.id));
        }
        let p: Player = serde_json::from_slice(state).map_err(|e| e.to_string())?;
        if returning {
            // Back from a refused move: not asked again until it steps back.
            self.asked.insert(subscriber.to_string());
        }
        self.players.insert(subscriber.to_string(), p);
        Ok(())
    }

    fn transfer_requests(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.leaving)
    }

    fn groups(&self) -> Vec<String> {
        self.params.group.iter().cloned().collect()
    }

    fn outbox(&mut self) -> Vec<Outgoing> {
        std::mem::take(&mut self.shouts)
    }

    fn on_message(&mut self, from: &str, topic: &str, data: &[u8]) {
        if topic == "shout" {
            if self.heard.len() == 10 {
                self.heard.pop_front();
            }
            self.heard
                .push_back(format!("{from}> {}", String::from_utf8_lossy(data)));
        }
    }

    fn calls(&mut self) -> Vec<Call> {
        // Every call waiting, every tick: the host skips the ones in
        // flight, and one that a crash lost goes out again.
        let loads = self
            .players
            .iter()
            .filter(|(_, p)| !p.loaded && p.load_retry_ms == 0)
            .map(|(sid, _)| Call {
                key: load_key(sid),
                function: "loadCharacter".into(),
                args: serde_json::json!({ "userId": sid }),
            });
        // A grant whose result waits for `release` is answered already.
        let grants = self
            .grants
            .iter()
            .filter(|(key, _)| !self.held.contains_key(*key))
            .map(|(key, g)| Call {
                key: key.clone(),
                function: "grantItem".into(),
                args: serde_json::json!({
                    "characterId": g.character,
                    "item": g.item,
                    "key": key,
                    "delayMs": g.delay_ms,
                }),
            });
        loads.chain(grants).collect()
    }

    fn on_call_result(&mut self, key: &str, result: Result<serde_json::Value, String>) {
        if let Some(sid) = key.strip_prefix("load:") {
            let Some(p) = self.players.get_mut(sid) else {
                return;
            };
            match result {
                Ok(row) if !p.loaded => {
                    p.loaded = true;
                    // The first load places the player; a reload after a
                    // restore keeps the saved x, newer than the row's.
                    if p.character.is_empty() {
                        p.x = row["x"].as_i64().unwrap_or(p.x);
                    }
                    p.character = row["id"].as_str().unwrap_or_default().to_string();
                    p.next_grant = p.next_grant.max(row["nextGrant"].as_u64().unwrap_or(0));
                    for item in row["items"].as_array().into_iter().flatten() {
                        if let (Some(id), Some(name)) = (item["id"].as_str(), item["name"].as_str())
                        {
                            p.items.insert(id.to_string(), name.to_string());
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    // The host tries database failures again itself; this
                    // is the function's answer (no character, say).
                    p.load_retry_ms = 5_000;
                    pylon_shard_guest::log(
                        pylon_shard_guest::Level::Warn,
                        &format!("loading {sid} failed: {e}"),
                    );
                }
            }
            return;
        }
        if self.params.hold_results {
            self.held.insert(key.to_string(), result);
            return;
        }
        self.apply_grant_result(key, result);
    }

    fn writes(&mut self) -> Vec<Write> {
        std::mem::take(&mut self.moved)
            .into_iter()
            .filter_map(|sid| {
                let p = self.players.get(&sid).filter(|p| p.loaded)?;
                let mut set = serde_json::Map::new();
                set.insert("x".into(), p.x.into());
                Some(Write {
                    entity: "Character".into(),
                    id: p.character.clone(),
                    set,
                })
            })
            .collect()
    }

    fn save(&self) -> Option<Vec<u8>> {
        serde_json::to_vec(&Saved {
            players: self.players.clone(),
            grants: self.grants.clone(),
        })
        .ok()
    }

    fn restore(shard_id: &str, params: Params, state: &[u8]) -> Result<Self, String> {
        let mut zone = Self::init(shard_id, params)?;
        let saved: Saved = serde_json::from_slice(state).map_err(|e| e.to_string())?;
        zone.players = saved.players;
        // Load every character again: a grant can have committed after
        // the save this state is from.
        for p in zone.players.values_mut() {
            p.loaded = false;
        }
        zone.grants = saved.grants;
        for g in zone.grants.values_mut() {
            g.restored = true;
        }
        Ok(zone)
    }
}

export_shard!(Zone);
