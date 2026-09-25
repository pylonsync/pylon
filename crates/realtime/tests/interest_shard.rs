//! Interest management under the real tick: what each subscriber's frames
//! hold, and when frames are shared.

use std::sync::Arc;
use std::time::{Duration, Instant};

use pylon_realtime::{
    EntityId, EntityPos, FrameKind, InterestArea, InterestConfig, OutboundQueue, Shard, ShardAuth,
    ShardConfig, SimState, SubscriberId, Visibility,
};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
struct Unit {
    id: EntityId,
    x: f32,
    y: f32,
    stealthed: bool,
}

/// Units on a plane. Subscriber "u<N>" controls unit N. Subscribers whose
/// id starts with "seer" see stealthed units.
struct World {
    units: Vec<Unit>,
    radius: f32,
    share: bool,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
struct View {
    ids: Vec<EntityId>,
    entered: Vec<EntityId>,
    left: Vec<EntityId>,
}

impl SimState for World {
    type Input = (EntityId, f32, f32);
    type Snapshot = View;
    type Error = String;

    fn apply_input(
        &mut self,
        _s: &SubscriberId,
        (id, x, y): Self::Input,
        _now: Instant,
    ) -> Result<(), String> {
        let u = self
            .units
            .iter_mut()
            .find(|u| u.id == id)
            .ok_or("no unit")?;
        (u.x, u.y) = (x, y);
        Ok(())
    }
    fn tick(&mut self, _dt: Duration) {}
    fn snapshot(&self) -> View {
        View {
            ids: self.units.iter().map(|u| u.id).collect(),
            entered: vec![],
            left: vec![],
        }
    }
    fn interest_config(&self) -> Option<InterestConfig> {
        Some(InterestConfig {
            cell_size: 20.0,
            margin: 2.0,
        })
    }
    fn entities(&self, out: &mut Vec<EntityPos>) {
        out.extend(self.units.iter().map(|u| EntityPos {
            id: u.id,
            x: u.x,
            y: u.y,
        }));
    }
    fn interest_area(&self, sid: &SubscriberId) -> Option<InterestArea> {
        let n: EntityId = sid
            .as_str()
            .trim_start_matches(|c: char| !c.is_ascii_digit())
            .parse()
            .ok()?;
        let u = self.units.iter().find(|u| u.id == n)?;
        Some(InterestArea {
            x: u.x,
            y: u.y,
            radius: self.radius,
        })
    }
    fn can_see(&self, sid: &SubscriberId, entity: EntityId) -> bool {
        sid.as_str().starts_with("seer")
            || !self.units.iter().any(|u| u.id == entity && u.stealthed)
    }
    fn snapshot_visible(&self, _sid: &SubscriberId, view: &Visibility) -> View {
        View {
            ids: view.visible.clone(),
            entered: view.entered.clone(),
            left: view.left.clone(),
        }
    }
    fn shares_visible_snapshots(&self) -> bool {
        self.share
    }
}

fn unit(id: EntityId, x: f32, y: f32, stealthed: bool) -> Unit {
    Unit {
        id,
        x,
        y,
        stealthed,
    }
}

fn join(shard: &Arc<Shard<World>>, sid: &str) -> Arc<OutboundQueue> {
    shard
        .add_queued_subscriber_authorized(SubscriberId::new(sid), &ShardAuth::admin())
        .unwrap()
}

fn last(q: &OutboundQueue) -> (View, Arc<[u8]>) {
    let mut last = None;
    while let Some(f) = q.pop() {
        if f.kind == FrameKind::Snapshot {
            last = Some(f.bytes);
        }
    }
    let bytes = last.expect("a snapshot frame");
    (serde_json::from_slice(&bytes).unwrap(), bytes)
}

fn world(units: Vec<Unit>, share: bool) -> Arc<Shard<World>> {
    Shard::new(
        "w",
        World {
            units,
            radius: 30.0,
            share,
        },
        ShardConfig {
            idle_ticks_before_shutdown: 0,
            ..Default::default()
        },
    )
}

#[test]
fn a_stealthed_unit_in_range_is_not_sent_to_a_subscriber_the_filter_rejects() {
    let shard = world(
        vec![
            unit(1, 0.0, 0.0, false),
            unit(2, 10.0, 0.0, true), // stealthed, in range
            unit(3, 20.0, 0.0, false),
            unit(4, 500.0, 0.0, false), // far
        ],
        false,
    );
    let plain = join(&shard, "u1");
    let seer = join(&shard, "seer1");
    shard.run_tick();

    let (v, bytes) = last(&plain);
    assert_eq!(v.ids, vec![1, 3]);
    assert!(!String::from_utf8_lossy(&bytes).contains('2'), "{v:?}");
    assert_eq!(last(&seer).0.ids, vec![1, 2, 3]);
}

#[test]
fn entered_and_left_follow_movement_with_hysteresis() {
    let shard = world(
        vec![unit(1, 0.0, 0.0, false), unit(2, 29.0, 0.0, false)],
        false,
    );
    let q = join(&shard, "u1");
    shard.run_tick();
    assert_eq!(last(&q).0.entered, vec![1, 2]);

    // 31 is past the 30 radius but inside the 2 margin: no change.
    shard
        .push_input(SubscriberId::new("u1"), (2, 31.0, 0.0), None)
        .unwrap();
    shard.run_tick();
    let v = last(&q).0;
    assert_eq!((v.ids.clone(), v.left.clone()), (vec![1, 2], vec![]));

    shard
        .push_input(SubscriberId::new("u1"), (2, 33.0, 0.0), None)
        .unwrap();
    shard.run_tick();
    let v = last(&q).0;
    assert_eq!((v.ids, v.left), (vec![1], vec![2]));
}

#[test]
fn subscribers_with_the_same_view_share_one_encoded_frame() {
    let shard = world(
        vec![
            unit(1, 0.0, 0.0, false),
            unit(2, 1.0, 0.0, false),
            unit(3, 400.0, 0.0, false),
        ],
        true,
    );
    let a = join(&shard, "u1");
    let b = join(&shard, "u2");
    let c = join(&shard, "u3");
    shard.run_tick();
    let (va, ba) = last(&a);
    let (vb, bb) = last(&b);
    let (vc, bc) = last(&c);
    assert_eq!(va.ids, vec![1, 2]);
    assert_eq!(va.ids, vb.ids);
    assert!(
        Arc::ptr_eq(&ba, &bb),
        "u1 and u2 see the same units: one encoding"
    );
    assert_eq!(vc.ids, vec![3]);
    assert!(!Arc::ptr_eq(&ba, &bc));
}

#[test]
fn without_an_interest_config_snapshot_for_is_used() {
    struct Plain;
    impl SimState for Plain {
        type Input = ();
        type Snapshot = String;
        type Error = String;
        fn apply_input(&mut self, _: &SubscriberId, _: (), _: Instant) -> Result<(), String> {
            Ok(())
        }
        fn tick(&mut self, _: Duration) {}
        fn snapshot(&self) -> String {
            "all".into()
        }
        fn snapshot_for(&self, sid: &SubscriberId) -> String {
            format!("for {}", sid.as_str())
        }
    }
    let shard = Shard::new(
        "p",
        Plain,
        ShardConfig {
            idle_ticks_before_shutdown: 0,
            ..Default::default()
        },
    );
    let q = shard
        .add_queued_subscriber_authorized(SubscriberId::new("x"), &ShardAuth::admin())
        .unwrap();
    shard.run_tick();
    let f = q.pop().unwrap();
    assert_eq!(&*f.bytes, br#""for x""#);
}
