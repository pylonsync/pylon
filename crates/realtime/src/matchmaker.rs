//! Matchmaker — queue players and create shards when criteria are met.
//!
//! The matchmaker is transport-agnostic and game-agnostic: it takes a queue
//! of "I want to play" entries, runs a user-supplied matching function on
//! each tick, and creates shards for formed matches.
//!
//! # Flow
//!
//! 1. Players `enqueue(player_id, criteria)` — queued with a timestamp.
//! 2. On every match tick, the [`MatchFn`] examines the queue and returns
//!    any valid groupings (each group = a future match).
//! 3. The matchmaker removes those players from the queue, creates a new
//!    shard in the registry, stores per-player "match ready" assignments,
//!    and returns the shard IDs.
//! 4. Players poll `status(player_id)` (or subscribe to notifications) to
//!    discover their assigned shard ID, then connect via WebSocket.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::registry::ShardRegistry;
use crate::shard::{Shard, ShardConfig, SimState};

// ---------------------------------------------------------------------------
// Queue entries
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedPlayer {
    pub player_id: String,
    pub criteria: serde_json::Value,
    pub joined_at_secs: u64,
}

impl QueuedPlayer {
    pub fn new(player_id: impl Into<String>, criteria: serde_json::Value) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Self {
            player_id: player_id.into(),
            criteria,
            joined_at_secs: now,
        }
    }
}

// ---------------------------------------------------------------------------
// Match assignments — the result of a successful match
// ---------------------------------------------------------------------------

/// The shard ID a player has been assigned to after matching.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchAssignment {
    pub player_id: String,
    pub shard_id: String,
    pub assigned_at_secs: u64,
}

/// Per-player status returned by [`Matchmaker::status`].
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PlayerStatus {
    Idle,
    Queued { position: usize, waiting_secs: u64 },
    Matched { shard_id: String },
}

// ---------------------------------------------------------------------------
// MatchFn — user-provided match logic
// ---------------------------------------------------------------------------

/// User-provided matching logic.
///
/// Given the current queue (in join order), return zero or more groups.
/// Each group is a `Vec<QueuedPlayer>` representing one match. Players
/// returned from this function are removed from the queue.
pub trait MatchFn: Send + Sync + 'static {
    fn try_match(&self, queue: &[QueuedPlayer]) -> Vec<Vec<QueuedPlayer>>;
}

impl<F> MatchFn for F
where
    F: Fn(&[QueuedPlayer]) -> Vec<Vec<QueuedPlayer>> + Send + Sync + 'static,
{
    fn try_match(&self, queue: &[QueuedPlayer]) -> Vec<Vec<QueuedPlayer>> {
        self(queue)
    }
}

/// Simple fixed-size matching: group the first N players whose criteria's
/// `"mode"` field matches. Swap in your own for skill-based, party-based,
/// or region-based matching.
pub fn fixed_size_match(group_size: usize) -> impl MatchFn {
    move |queue: &[QueuedPlayer]| -> Vec<Vec<QueuedPlayer>> {
        if group_size == 0 {
            return vec![];
        }
        let mut by_mode: HashMap<String, Vec<QueuedPlayer>> = HashMap::new();
        for p in queue {
            let mode = p
                .criteria
                .get("mode")
                .and_then(|v| v.as_str())
                .unwrap_or("default")
                .to_string();
            by_mode.entry(mode).or_default().push(p.clone());
        }
        let mut matches = Vec::new();
        for (_mode, mut players) in by_mode {
            while players.len() >= group_size {
                let group: Vec<_> = players.drain(..group_size).collect();
                matches.push(group);
            }
        }
        matches
    }
}

// ---------------------------------------------------------------------------
// Shard factory — turns a group of players into initial sim state
// ---------------------------------------------------------------------------

pub trait ShardFactory<S: SimState>: Send + Sync + 'static {
    fn build(&self, players: &[QueuedPlayer]) -> S;
}

impl<S: SimState, F> ShardFactory<S> for F
where
    F: Fn(&[QueuedPlayer]) -> S + Send + Sync + 'static,
{
    fn build(&self, players: &[QueuedPlayer]) -> S {
        self(players)
    }
}

// ---------------------------------------------------------------------------
// MatchmakerConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct MatchmakerConfig {
    /// How often to run the match function. Default: every 1s.
    pub tick_interval: Duration,
    /// Discard queue entries older than this. 0 = never. Default: 60s.
    pub max_wait: Duration,
    /// ShardConfig for newly-created match shards.
    pub shard_config: ShardConfig,
    /// Optional prefix for generated shard IDs (default: `"match_"`).
    pub shard_id_prefix: String,
}

impl Default for MatchmakerConfig {
    fn default() -> Self {
        Self {
            tick_interval: Duration::from_secs(1),
            max_wait: Duration::from_secs(60),
            shard_config: ShardConfig::default(),
            shard_id_prefix: "match_".into(),
        }
    }
}

// ---------------------------------------------------------------------------
// Matchmaker
// ---------------------------------------------------------------------------

/// Queues players and creates shards when matches form.
///
/// Generic over the game's [`SimState`]. Instantiated once per game type.
pub struct Matchmaker<S: SimState> {
    queue: Mutex<Vec<QueuedPlayer>>,
    /// Assignments by player ID. Cleared after the player reads their status
    /// via [`ack_assignment`].
    assignments: Mutex<HashMap<String, MatchAssignment>>,
    registry: Arc<ShardRegistry<S>>,
    match_fn: Box<dyn MatchFn>,
    shard_factory: Box<dyn ShardFactory<S>>,
    config: MatchmakerConfig,
    counter: AtomicU64,
    handle: Mutex<Option<JoinHandle<()>>>,
    running: Arc<std::sync::atomic::AtomicBool>,
}

impl<S: SimState> Matchmaker<S> {
    pub fn new(
        registry: Arc<ShardRegistry<S>>,
        match_fn: impl MatchFn,
        shard_factory: impl ShardFactory<S>,
        config: MatchmakerConfig,
    ) -> Arc<Self> {
        Arc::new(Self {
            queue: Mutex::new(Vec::new()),
            assignments: Mutex::new(HashMap::new()),
            registry,
            match_fn: Box::new(match_fn),
            shard_factory: Box::new(shard_factory),
            config,
            counter: AtomicU64::new(0),
            handle: Mutex::new(None),
            running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }

    /// Spawn the background match loop. Can be called multiple times safely.
    pub fn start(self: &Arc<Self>) {
        use std::sync::atomic::Ordering::SeqCst;
        if self.running.swap(true, SeqCst) {
            return;
        }
        let me = Arc::clone(self);
        let handle = std::thread::Builder::new()
            .name("matchmaker".into())
            .spawn(move || {
                while me.running.load(SeqCst) {
                    me.run_once();
                    std::thread::sleep(me.config.tick_interval);
                }
            })
            .expect("failed to spawn matchmaker thread");
        *self.handle.lock().unwrap() = Some(handle);
    }

    pub fn stop(&self) {
        self.running
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }

    /// Enqueue a player with matching criteria.
    pub fn enqueue(&self, player_id: impl Into<String>, criteria: serde_json::Value) {
        let player = QueuedPlayer::new(player_id, criteria);
        let id = player.player_id.clone();
        let mut q = self.queue.lock().unwrap();
        // De-dup: replace any existing entry for this player.
        q.retain(|p| p.player_id != id);
        q.push(player);
    }

    /// Remove a player from the queue.
    pub fn dequeue(&self, player_id: &str) -> bool {
        let mut q = self.queue.lock().unwrap();
        let before = q.len();
        q.retain(|p| p.player_id != player_id);
        before != q.len()
    }

    /// Look up a player's current status.
    pub fn status(&self, player_id: &str) -> PlayerStatus {
        if let Some(a) = self.assignments.lock().unwrap().get(player_id) {
            return PlayerStatus::Matched {
                shard_id: a.shard_id.clone(),
            };
        }
        let q = self.queue.lock().unwrap();
        match q.iter().position(|p| p.player_id == player_id) {
            Some(pos) => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                PlayerStatus::Queued {
                    position: pos,
                    waiting_secs: now.saturating_sub(q[pos].joined_at_secs),
                }
            }
            None => PlayerStatus::Idle,
        }
    }

    /// Acknowledge an assignment; clears it from the pending map.
    /// Players typically do this after connecting to their shard.
    pub fn ack_assignment(&self, player_id: &str) -> Option<MatchAssignment> {
        self.assignments.lock().unwrap().remove(player_id)
    }

    /// Current queue depth.
    pub fn queue_len(&self) -> usize {
        self.queue.lock().unwrap().len()
    }

    /// One pass of the match loop. Public so tests and manual cadences can invoke it.
    pub fn run_once(&self) {
        // Expire stale entries.
        if self.config.max_wait.as_secs() > 0 {
            let cutoff = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                .saturating_sub(self.config.max_wait.as_secs());
            let mut q = self.queue.lock().unwrap();
            q.retain(|p| p.joined_at_secs >= cutoff);
        }

        // Run match function.
        let queue_snapshot = self.queue.lock().unwrap().clone();
        let matches = self.match_fn.try_match(&queue_snapshot);
        if matches.is_empty() {
            return;
        }

        // Remove matched players from the queue.
        let matched_ids: std::collections::HashSet<String> = matches
            .iter()
            .flatten()
            .map(|p| p.player_id.clone())
            .collect();
        {
            let mut q = self.queue.lock().unwrap();
            q.retain(|p| !matched_ids.contains(&p.player_id));
        }

        // Create a shard per match and store per-player assignments.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut assignments = self.assignments.lock().unwrap();
        for group in matches {
            let match_no = self.counter.fetch_add(1, Ordering::Relaxed) + 1;
            let shard_id = format!("{}{:x}_{}", self.config.shard_id_prefix, now, match_no);
            let state = self.shard_factory.build(&group);
            let shard = Shard::new(shard_id.clone(), state, self.config.shard_config.clone());
            self.registry.insert(shard);
            for p in group {
                assignments.insert(
                    p.player_id.clone(),
                    MatchAssignment {
                        player_id: p.player_id,
                        shard_id: shard_id.clone(),
                        assigned_at_secs: now,
                    },
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subscriber::SubscriberId;
    use std::time::Instant;

    struct Game {
        players: Vec<String>,
    }

    impl SimState for Game {
        type Input = String;
        type Snapshot = Vec<String>;
        type Error = String;
        fn apply_input(
            &mut self,
            _s: &SubscriberId,
            _input: Self::Input,
            _now: Instant,
        ) -> Result<(), Self::Error> {
            Ok(())
        }
        fn tick(&mut self, _dt: Duration) {}
        fn snapshot(&self) -> Self::Snapshot {
            self.players.clone()
        }
    }

    #[test]
    fn matchmaker_creates_shard_when_group_forms() {
        let reg: Arc<ShardRegistry<Game>> = Arc::new(ShardRegistry::new());
        let mm = Matchmaker::new(
            Arc::clone(&reg),
            fixed_size_match(2),
            |players: &[QueuedPlayer]| Game {
                players: players.iter().map(|p| p.player_id.clone()).collect(),
            },
            MatchmakerConfig {
                shard_config: ShardConfig {
                    tick_rate_hz: 10,
                    ..Default::default()
                },
                ..Default::default()
            },
        );

        mm.enqueue("alice", serde_json::json!({"mode": "ranked"}));
        mm.enqueue("bob", serde_json::json!({"mode": "ranked"}));
        mm.enqueue("charlie", serde_json::json!({"mode": "casual"}));

        assert_eq!(mm.queue_len(), 3);
        mm.run_once();

        // alice + bob should be matched, charlie still queued.
        assert_eq!(mm.queue_len(), 1);
        match mm.status("alice") {
            PlayerStatus::Matched { shard_id } => {
                assert!(shard_id.starts_with("match_"));
                assert!(reg.get(&shard_id).is_some());
            }
            _ => panic!("expected alice to be matched"),
        }
        assert!(matches!(mm.status("charlie"), PlayerStatus::Queued { .. }));
    }

    #[test]
    fn dequeue_removes_player() {
        let reg: Arc<ShardRegistry<Game>> = Arc::new(ShardRegistry::new());
        let mm = Matchmaker::new(
            Arc::clone(&reg),
            fixed_size_match(2),
            |_players: &[QueuedPlayer]| Game { players: vec![] },
            MatchmakerConfig::default(),
        );

        mm.enqueue("alice", serde_json::json!({}));
        assert_eq!(mm.queue_len(), 1);
        assert!(mm.dequeue("alice"));
        assert_eq!(mm.queue_len(), 0);
        assert!(matches!(mm.status("alice"), PlayerStatus::Idle));
    }

    #[test]
    fn ack_assignment_clears_pending() {
        let reg: Arc<ShardRegistry<Game>> = Arc::new(ShardRegistry::new());
        let mm = Matchmaker::new(
            Arc::clone(&reg),
            fixed_size_match(2),
            |players: &[QueuedPlayer]| Game {
                players: players.iter().map(|p| p.player_id.clone()).collect(),
            },
            MatchmakerConfig {
                shard_config: ShardConfig {
                    tick_rate_hz: 10,
                    ..Default::default()
                },
                ..Default::default()
            },
        );

        mm.enqueue("a", serde_json::json!({}));
        mm.enqueue("b", serde_json::json!({}));
        mm.run_once();

        let ack = mm.ack_assignment("a").unwrap();
        assert!(ack.shard_id.starts_with("match_"));
        // Second ack returns None (already consumed).
        assert!(mm.ack_assignment("a").is_none());
    }
}
