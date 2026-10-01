//! Lag compensation under the real tick: an input's view tick reaches the
//! simulation clamped, with a view of what that subscriber was sent.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pylon_realtime::{
    EntityId, FrameKind, LagCompensation, OutboundQueue, Plane, ReplicaTable, Replicated,
    ReplicatedRef, ReplicationConfig, Rewind, Shard, ShardAuth, ShardConfig, SimState,
    SubscriberId,
};

/// What `apply_input_at` saw for one input.
#[derive(Debug, Clone, PartialEq)]
struct Seen {
    tick: Option<f64>,
    target: Option<[f64; 3]>,
    near: Vec<EntityId>,
}

/// Entity 1 walks along x one unit a tick; entities 2-40 stand far away.
/// Each input asks where entity 1 was.
struct Range {
    store: Replicated,
    ticks: u64,
    lag: Option<LagCompensation>,
    budget: usize,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl SimState for Range {
    type Input = u8;
    type Snapshot = ();
    type Error = String;

    fn apply_input(&mut self, _: &SubscriberId, _: u8, _: Instant) -> Result<(), String> {
        self.seen.lock().unwrap().push(Seen {
            tick: None,
            target: None,
            near: Vec::new(),
        });
        Ok(())
    }

    fn apply_input_at(
        &mut self,
        sid: &SubscriberId,
        input: u8,
        now: Instant,
        view: Option<&Rewind>,
    ) -> Result<(), String> {
        let Some(view) = view else {
            return self.apply_input(sid, input, now);
        };
        self.seen.lock().unwrap().push(Seen {
            tick: Some(view.tick()),
            target: view.position_at(1),
            near: view
                .entities_near([0.0, 0.0, 0.0], 50.0)
                .into_iter()
                .map(|(id, _)| id)
                .collect(),
        });
        Ok(())
    }

    fn tick(&mut self, _: Duration) {
        self.ticks += 1;
        self.store.set_pos(1, [self.ticks as f32, 0.0, 0.0]);
        // The far entities move too, so a byte budget has to choose.
        for id in 2..=40u64 {
            self.store
                .set_pos(id, [1000.0 + id as f32, self.ticks as f32, 0.0]);
        }
    }
    fn snapshot(&self) {}
    fn replicated(&self) -> Option<ReplicatedRef<'_>> {
        Some((&self.store).into())
    }
    fn replication_config(&self) -> ReplicationConfig {
        ReplicationConfig {
            precision: 0.01,
            max_bytes_per_tick: self.budget,
            plane: Plane::XY,
        }
    }
    fn lag_compensation(&self) -> Option<LagCompensation> {
        self.lag
    }
}

fn shard(
    lag: Option<LagCompensation>,
    budget: usize,
) -> (Arc<Shard<Range>>, Arc<Mutex<Vec<Seen>>>) {
    let mut store = Replicated::new();
    store.spawn(1, [0.0, 0.0, 0.0]);
    for id in 2..=40u64 {
        store.spawn(id, [1000.0 + id as f32, 0.0, 0.0]);
    }
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = Shard::new(
        "range",
        Range {
            store,
            ticks: 0,
            lag,
            budget,
            seen: Arc::clone(&seen),
        },
        ShardConfig {
            tick_rate_hz: 20,
            fixed_timestep: true,
            idle_ticks_before_shutdown: 0,
            ..Default::default()
        },
    );
    (s, seen)
}

fn join(s: &Arc<Shard<Range>>, sid: &str) -> Arc<OutboundQueue> {
    s.add_queued_subscriber_authorized(SubscriberId::new(sid), &ShardAuth::admin())
        .unwrap()
}

fn drain(q: &OutboundQueue, table: &mut ReplicaTable) {
    while let Some(f) = q.pop() {
        if f.kind == FrameKind::Replication {
            table.apply(&f.bytes).unwrap();
        }
    }
}

fn send(s: &Arc<Shard<Range>>, sid: &str, view_tick: Option<f64>) {
    s.push_input_authorized_at(
        SubscriberId::new(sid),
        0,
        Some(1),
        view_tick,
        &ShardAuth::admin(),
    )
    .unwrap();
}

#[test]
fn an_input_sees_where_its_subscriber_drew_the_target_at_its_view_tick() {
    let (s, seen) = shard(Some(LagCompensation::new(10)), 0);
    let q = join(&s, "p1");
    let mut table = ReplicaTable::new();
    for _ in 0..20 {
        s.run_tick();
        drain(&q, &mut table);
    }
    // Frames went out for ticks 1-20; entity 1 was at x = tick after each.
    // A view 2.5 ticks back is between two of them.
    send(&s, "p1", Some(17.5));
    s.run_tick();
    let got = seen.lock().unwrap().pop().unwrap();
    assert_eq!(got.tick, Some(17.5));
    let p = 0.01f32 as f64;
    let x = got.target.unwrap()[0];
    let want = 1700.0 * p + (1800.0 * p - 1700.0 * p) * 0.5;
    assert!((x - want).abs() < 1e-9, "{x} vs {want}");
    assert_eq!(got.near, vec![1], "only entity 1 is near the origin");
}

#[test]
fn the_view_tick_is_clamped_to_the_history_and_the_newest_tick() {
    let (s, seen) = shard(Some(LagCompensation::new(10)), 0);
    let q = join(&s, "p1");
    let mut table = ReplicaTable::new();
    for _ in 0..30 {
        s.run_tick();
        drain(&q, &mut table);
    }
    // The newest frames are for tick 30; the history reaches tick 20.
    send(&s, "p1", Some(3.0));
    send(&s, "p1", Some(99.0));
    send(&s, "p1", Some(f64::NAN));
    s.run_tick();
    let seen = seen.lock().unwrap();
    let n = seen.len();
    assert_eq!(seen[n - 3].tick, Some(20.0));
    assert_eq!(seen[n - 2].tick, Some(30.0));
    // Not a number: the input applies without a view.
    assert_eq!(seen[n - 1].tick, None);
}

#[test]
fn the_round_trip_bounds_how_far_back_a_view_reaches() {
    let (s, seen) = shard(Some(LagCompensation::new(20)), 0);
    let q = join(&s, "p1");
    let mut table = ReplicaTable::new();
    for _ in 0..30 {
        s.run_tick();
        drain(&q, &mut table);
    }
    // 50 ms round trip + 250 ms allowance = 6 ticks at 50 ms.
    q.record_rtt(Duration::from_millis(50));
    send(&s, "p1", Some(15.0));
    s.run_tick();
    assert_eq!(seen.lock().unwrap().pop().unwrap().tick, Some(24.0));
}

#[test]
fn a_deferred_update_rewinds_to_what_the_subscriber_was_sent() {
    // A budget that fits a few updates a tick: the far entities fall behind,
    // and each subscriber's view follows its own frames.
    let (s, seen) = shard(Some(LagCompensation::new(10)), 40);
    let q = join(&s, "p1");
    let mut table = ReplicaTable::new();
    for _ in 0..20 {
        s.run_tick();
        drain(&q, &mut table);
    }
    send(&s, "p1", Some(20.0));
    s.run_tick();
    let got = seen.lock().unwrap().pop().unwrap();
    assert_eq!(got.tick, Some(20.0));
    // At the newest tick, the view places entity 1 where the client's table
    // has it, which the budget may hold behind the store.
    let e = &table.entities[&1];
    assert_eq!(got.target, Some(e.q.map(|v| v as f64 * 0.01f32 as f64)));
}

#[test]
fn without_lag_compensation_inputs_apply_as_before() {
    let (s, seen) = shard(None, 0);
    let q = join(&s, "p1");
    let mut table = ReplicaTable::new();
    s.run_tick();
    drain(&q, &mut table);
    send(&s, "p1", Some(1.0));
    s.run_tick();
    assert_eq!(seen.lock().unwrap().pop().unwrap().tick, None);
}
