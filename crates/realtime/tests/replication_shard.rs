//! Entity replication under the real tick: each subscriber's decoded table
//! matches what it should see, hidden entities never arrive, and a
//! reconnect gets a full baseline.

use std::sync::Arc;
use std::time::{Duration, Instant};

use pylon_realtime::{
    EntityId, FrameKind, InterestArea, InterestConfig, OutboundQueue, Plane, ReplicaTable,
    Replicated, ReplicatedRef, ReplicationConfig, Shard, ShardAuth, ShardConfig, SimState,
    SubscriberId,
};

/// Units walk in a circle. Subscriber "u<N>" watches from unit N. Unit 0 is
/// stealthed: only subscribers named "seer..." see it.
struct Field {
    store: Replicated,
    t: f32,
    interest: bool,
}

const HP: u8 = 1;

impl Field {
    fn new(units: u64, interest: bool) -> Self {
        let mut store = Replicated::new();
        for id in 0..units {
            store.spawn(id, [id as f32 * 3.0, 0.0, 0.0]);
            store.set_component(id, HP, &[100]);
        }
        Self {
            store,
            t: 0.0,
            interest,
        }
    }
}

impl SimState for Field {
    type Input = (EntityId, u8);
    type Snapshot = ();
    type Error = String;

    fn apply_input(
        &mut self,
        _: &SubscriberId,
        (id, hp): Self::Input,
        _: Instant,
    ) -> Result<(), String> {
        if self.store.set_component(id, HP, &[hp]) {
            Ok(())
        } else {
            Err(format!("no unit {id}"))
        }
    }
    fn tick(&mut self, dt: Duration) {
        self.t += dt.as_secs_f32();
        let ids: Vec<EntityId> = self.store.iter().map(|(id, _)| id).collect();
        for id in ids {
            let base = id as f32 * 3.0;
            let a = self.t + id as f32;
            self.store
                .set_pos(id, [base + a.cos() * 2.0, a.sin() * 2.0, 0.0]);
        }
    }
    fn snapshot(&self) {}
    fn interest_config(&self) -> Option<InterestConfig> {
        self.interest.then_some(InterestConfig {
            cell_size: 10.0,
            margin: 1.0,
        })
    }
    fn interest_area(&self, sid: &SubscriberId) -> Option<InterestArea> {
        let n: EntityId = sid
            .as_str()
            .trim_start_matches(|c: char| !c.is_ascii_digit())
            .parse()
            .ok()?;
        let p = self.store.get(n)?.pos;
        Some(InterestArea {
            x: p[0],
            y: p[1],
            radius: 10.0,
        })
    }
    fn can_see(&self, sid: &SubscriberId, entity: EntityId) -> bool {
        entity != 0 || sid.as_str().starts_with("seer")
    }
    fn replicated(&self) -> Option<ReplicatedRef<'_>> {
        Some((&self.store).into())
    }
    fn replication_config(&self) -> ReplicationConfig {
        ReplicationConfig {
            precision: 0.01,
            max_bytes_per_tick: 0,
            plane: Plane::XY,
        }
    }
}

fn shard(units: u64, interest: bool) -> Arc<Shard<Field>> {
    shard_with(
        units,
        interest,
        ShardConfig::default().outbound_queue_frames,
    )
}

fn shard_with(units: u64, interest: bool, queue_frames: usize) -> Arc<Shard<Field>> {
    Shard::new(
        "f",
        Field::new(units, interest),
        ShardConfig {
            tick_rate_hz: 20,
            idle_ticks_before_shutdown: 0,
            outbound_queue_frames: queue_frames,
            ..Default::default()
        },
    )
}

fn join(s: &Arc<Shard<Field>>, sid: &str) -> Arc<OutboundQueue> {
    s.add_queued_subscriber_authorized(SubscriberId::new(sid), &ShardAuth::admin())
        .unwrap()
}

/// Apply every queued frame to `table`; returns whether one was full.
fn drain(q: &OutboundQueue, table: &mut ReplicaTable) -> bool {
    let mut saw_full = false;
    while let Some(f) = q.pop() {
        if f.kind == FrameKind::InputRejected {
            continue;
        }
        assert_eq!(f.kind, FrameKind::Replication);
        saw_full |= table.apply(&f.bytes).unwrap().full;
    }
    saw_full
}

/// The ids a subscriber should have, and their positions and hp.
fn expected(s: &Arc<Shard<Field>>, sid: &str) -> Vec<(EntityId, [f32; 3], Vec<u8>)> {
    s.with_state(|f| {
        let sid = SubscriberId::new(sid);
        let area = f.interest_area(&sid);
        f.store
            .iter()
            .filter(|(id, _)| f.can_see(&sid, *id))
            .filter(|(_, e)| {
                !f.interest
                    || area.is_some_and(|a| {
                        let (dx, dy) = (e.pos[0] - a.x, e.pos[1] - a.y);
                        (dx * dx + dy * dy).sqrt() <= a.radius
                    })
            })
            .map(|(id, e)| (id, e.pos, e.component(HP).unwrap().to_vec()))
            .collect()
    })
}

fn assert_table(table: &ReplicaTable, want: &[(EntityId, [f32; 3], Vec<u8>)], margin_ok: bool) {
    for (id, pos, hp) in want {
        let got = table
            .pos(*id)
            .unwrap_or_else(|| panic!("entity {id} missing"));
        for i in 0..3 {
            assert!((got[i] - pos[i]).abs() <= 0.01, "{id}: {got:?} vs {pos:?}");
        }
        assert_eq!(&table.entities[id].components[&HP], hp);
    }
    if !margin_ok {
        assert_eq!(table.entities.len(), want.len());
    }
}

#[test]
fn every_subscriber_table_matches_the_store_without_interest() {
    let s = shard(6, false);
    let q = join(&s, "u1");
    let mut table = ReplicaTable::new();
    for tick in 0..40 {
        if tick == 20 {
            s.push_input(SubscriberId::new("u1"), (3, 42), None)
                .unwrap();
        }
        s.run_tick();
        drain(&q, &mut table);
        assert_table(&table, &expected(&s, "u1"), false);
    }
    assert_eq!(table.entities[&3].components[&HP], vec![42]);
    // The stealthed unit never reached a plain subscriber.
    assert!(!table.entities.contains_key(&0));
}

#[test]
fn interest_views_and_the_stealth_filter_shape_each_table() {
    let s = shard(30, true);
    let plain = join(&s, "u5");
    let seer = join(&s, "seer1");
    let (mut tp, mut ts) = (ReplicaTable::new(), ReplicaTable::new());
    let mut seer_saw_zero = false;
    for _ in 0..60 {
        s.run_tick();
        drain(&plain, &mut tp);
        drain(&seer, &mut ts);
        // Hysteresis keeps entities up to 1 unit past the radius, so the
        // tables may hold a few extra; every expected entity must be there.
        assert_table(&tp, &expected(&s, "u5"), true);
        assert_table(&ts, &expected(&s, "seer1"), true);
        assert!(
            !tp.entities.contains_key(&0),
            "the plain subscriber got the stealthed unit"
        );
        seer_saw_zero |= ts.entities.contains_key(&0);
        // Far units are not in the plain table.
        assert!(!tp.entities.contains_key(&29));
    }
    assert!(seer_saw_zero, "the seer never saw the stealthed unit");
}

#[test]
fn a_reconnect_gets_a_full_baseline() {
    let s = shard(4, false);
    let first = join(&s, "u1");
    let mut table = ReplicaTable::new();
    s.run_tick();
    assert!(drain(&first, &mut table));
    s.run_tick();
    assert!(!drain(&first, &mut table));
    assert!(s.remove_queued_subscriber(&first));
    let again = join(&s, "u1");
    let mut fresh = ReplicaTable::new();
    s.run_tick();
    assert!(
        drain(&again, &mut fresh),
        "the new connection's first frame is full"
    );
    assert_table(&fresh, &expected(&s, "u1"), false);
}

#[test]
fn a_full_queue_never_ends_up_with_a_delta_after_a_dropped_baseline() {
    // Two frames of room. The first (full) frame stays unread.
    let s = shard_with(3, false, 2);
    let q = join(&s, "u1");
    s.run_tick();
    // Next tick: an input fails (a rejection frame fills the queue) and
    // every unit moves (a delta would follow and push the baseline out).
    s.push_input(SubscriberId::new("u1"), (99, 1), Some(1))
        .unwrap();
    s.run_tick();
    // Whatever survived applies cleanly and the table matches the store.
    let mut table = ReplicaTable::new();
    drain(&q, &mut table);
    assert_table(&table, &expected(&s, "u1"), false);
    // And later deltas keep applying.
    for _ in 0..5 {
        s.run_tick();
        drain(&q, &mut table);
    }
    assert_table(&table, &expected(&s, "u1"), false);
}

#[test]
fn stats_count_the_full_frame_sent_for_a_refused_delta_and_not_the_delta() {
    // A queue of one frame that is never read: tick 1 queues the full
    // baseline; tick 2's delta finds the queue full, is refused, and a full
    // frame replaces the baseline.
    let s = shard_with(20, false, 1);
    let q = join(&s, "u1");
    s.run_tick();
    let first = s.stats().bytes_total;
    assert!(first > 0);
    s.run_tick();
    let second = s.stats().bytes_total - first;
    let queued = q.pop().expect("the full frame");
    assert_eq!(queued.kind, FrameKind::Replication);
    assert!(q.pop().is_none());
    assert_eq!(second, queued.bytes.len() as u64);
    assert_eq!(s.stats().dropped_frames_total, 1);
    // A closed queue takes nothing, and nothing is counted.
    q.close();
    s.run_tick();
    assert_eq!(s.stats().bytes_total, first + second);
}
