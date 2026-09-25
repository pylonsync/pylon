//! Interest management: which entities each subscriber sees.
//!
//! A shard whose [`SimState::interest_config`](crate::SimState::interest_config)
//! returns a config gets this each tick:
//!
//! 1. The sim reports its entities' positions ([`SimState::entities`]).
//!    The shard puts them in a uniform grid of `cell_size` cells.
//! 2. For each subscriber with an [`InterestArea`] (a point and a radius,
//!    usually around the entity it controls), the shard reads the cells the
//!    circle touches and keeps the entities within the radius.
//! 3. Hysteresis: an entity already visible stays visible until it is more
//!    than `margin` past the radius, so an entity at the edge does not flicker
//!    in and out.
//! 4. [`SimState::can_see`] can hide an entity in range (stealth, fog).
//! 5. The sim builds the subscriber's snapshot from the result
//!    ([`SimState::snapshot_visible`]): the visible ids, and the ids that
//!    entered and left view since the last tick.
//!
//! Positions are 2D. A 3D world passes its ground plane (x and z).
//!
//! [`SimState::entities`]: crate::SimState::entities
//! [`SimState::can_see`]: crate::SimState::can_see
//! [`SimState::snapshot_visible`]: crate::SimState::snapshot_visible

use std::collections::HashMap;

use crate::subscriber::SubscriberId;

/// A stable entity id, chosen by the simulation.
pub type EntityId = u64;

/// One entity's position this tick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntityPos {
    pub id: EntityId,
    pub x: f32,
    pub y: f32,
}

/// What a subscriber can see: a circle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InterestArea {
    pub x: f32,
    pub y: f32,
    pub radius: f32,
}

/// Grid settings for a shard.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InterestConfig {
    /// Side of a grid cell, in world units. About the view radius works well.
    pub cell_size: f32,
    /// How far past the radius a visible entity may go before it leaves view.
    pub margin: f32,
}

impl Default for InterestConfig {
    fn default() -> Self {
        Self {
            cell_size: 64.0,
            margin: 8.0,
        }
    }
}

/// One subscriber's view this tick.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Visibility {
    /// Visible entity ids, sorted.
    pub visible: Vec<EntityId>,
    /// Ids visible now and not last tick, sorted.
    pub entered: Vec<EntityId>,
    /// Ids visible last tick and not now, sorted.
    pub left: Vec<EntityId>,
}

/// A uniform grid of entity positions, rebuilt each tick.
#[derive(Debug, Default)]
pub struct SpatialGrid {
    cell_size: f32,
    entities: Vec<EntityPos>,
    /// Cell to indexes into `entities`. Cleared, not dropped, between ticks
    /// so the vectors keep their capacity.
    cells: HashMap<(i32, i32), Vec<u32>>,
}

impl SpatialGrid {
    pub fn new(cell_size: f32) -> Self {
        Self {
            cell_size: sanitize_cell(cell_size),
            ..Self::default()
        }
    }

    fn cell_of(&self, x: f32, y: f32) -> (i32, i32) {
        (
            (x / self.cell_size).floor() as i32,
            (y / self.cell_size).floor() as i32,
        )
    }

    /// Replace the grid's contents with `entities`. Entities with a
    /// non-finite position are left out, and of two with one id the first
    /// is kept. The grid holds them sorted by id, so a scan in index order
    /// yields sorted ids.
    pub fn rebuild(&mut self, entities: &[EntityPos]) {
        for cell in self.cells.values_mut() {
            cell.clear();
        }
        self.entities.clear();
        self.entities.extend(
            entities
                .iter()
                .filter(|e| e.x.is_finite() && e.y.is_finite()),
        );
        self.entities.sort_by_key(|e| e.id);
        self.entities.dedup_by_key(|e| e.id);
        for (idx, e) in self.entities.iter().enumerate() {
            let cell = (
                (e.x / self.cell_size).floor() as i32,
                (e.y / self.cell_size).floor() as i32,
            );
            self.cells.entry(cell).or_default().push(idx as u32);
        }
        // Drop cells that stayed empty, so a moving crowd does not leave a
        // trail of empty vectors behind it.
        if self.cells.len() > 4 * self.entities.len().max(64) {
            self.cells.retain(|_, v| !v.is_empty());
        }
    }

    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// Call `visit` for every entity in a cell the circle at (`x`, `y`)
    /// with radius `r` touches. Entities outside the circle may be visited.
    pub fn for_each_near(&self, x: f32, y: f32, r: f32, mut visit: impl FnMut(&EntityPos)) {
        self.for_each_index_near(x, y, r, |i| visit(&self.entities[i]));
    }

    /// Like `for_each_near`, with the entity's index (entities are in id
    /// order).
    fn for_each_index_near(&self, x: f32, y: f32, r: f32, mut visit: impl FnMut(usize)) {
        if !(x.is_finite() && y.is_finite() && r.is_finite()) || r < 0.0 {
            return;
        }
        let (x0, y0) = self.cell_of(x - r, y - r);
        let (x1, y1) = self.cell_of(x + r, y + r);
        // A radius far larger than the world would walk millions of empty
        // cells; scan the entities directly instead.
        let span = (x1 as i128 - x0 as i128 + 1) * (y1 as i128 - y0 as i128 + 1);
        if span > self.cells.len() as i128 {
            for i in 0..self.entities.len() {
                visit(i);
            }
            return;
        }
        for cx in x0..=x1 {
            for cy in y0..=y1 {
                if let Some(cell) = self.cells.get(&(cx, cy)) {
                    for &i in cell {
                        visit(i as usize);
                    }
                }
            }
        }
    }
}

fn sanitize_cell(cell_size: f32) -> f32 {
    if cell_size.is_finite() && cell_size > 0.0 {
        cell_size
    } else {
        InterestConfig::default().cell_size
    }
}

/// Computes each subscriber's [`Visibility`] from a [`SpatialGrid`] and
/// keeps last tick's views for hysteresis and the entered/left lists.
#[derive(Debug, Default)]
pub struct InterestManager {
    config: InterestConfig,
    grid: SpatialGrid,
    views: HashMap<SubscriberId, Visibility>,
    scratch: Vec<EntityId>,
    /// One bit per grid entity: in view this update.
    bits: Vec<u64>,
}

impl InterestManager {
    pub fn new(config: InterestConfig) -> Self {
        Self {
            config,
            grid: SpatialGrid::new(config.cell_size),
            views: HashMap::new(),
            scratch: Vec::new(),
            bits: Vec::new(),
        }
    }

    pub fn config(&self) -> InterestConfig {
        self.config
    }

    /// Use new settings from the next tick on.
    pub fn set_config(&mut self, config: InterestConfig) {
        if config.cell_size != self.config.cell_size {
            self.grid = SpatialGrid::new(config.cell_size);
        }
        self.config = config;
    }

    pub fn grid(&self) -> &SpatialGrid {
        &self.grid
    }

    /// Rebuild the grid from this tick's entities.
    pub fn rebuild(&mut self, entities: &[EntityPos]) {
        self.grid.rebuild(entities);
    }

    /// Recompute one subscriber's view. `area` None sees nothing. `filter`
    /// gets the sorted ids in range and may remove some (stealth, fog).
    pub fn update(
        &mut self,
        subscriber: &SubscriberId,
        area: Option<InterestArea>,
        filter: impl FnOnce(&mut Vec<EntityId>),
    ) -> &Visibility {
        let margin = self.config.margin.max(0.0);
        let view = self.views.entry(subscriber.clone()).or_default();
        let next = &mut self.scratch;
        next.clear();
        if let Some(area) = area.filter(|a| a.radius.is_finite() && a.radius >= 0.0) {
            let r2 = area.radius * area.radius;
            let keep = area.radius + margin;
            let keep2 = keep * keep;
            let prev = &view.visible;
            let entities = &self.grid.entities;
            let bits = &mut self.bits;
            bits.clear();
            bits.resize(entities.len().div_ceil(64), 0);
            self.grid.for_each_index_near(area.x, area.y, keep, |i| {
                let e = &entities[i];
                let (dx, dy) = (e.x - area.x, e.y - area.y);
                let d2 = dx * dx + dy * dy;
                if d2 <= r2 || (d2 <= keep2 && prev.binary_search(&e.id).is_ok()) {
                    bits[i / 64] |= 1 << (i % 64);
                }
            });
            // Entities are in id order, so reading the bits in order gives
            // sorted ids with no sort.
            for (w, word) in bits.iter().enumerate() {
                let mut word = *word;
                while word != 0 {
                    let b = word.trailing_zeros() as usize;
                    next.push(entities[w * 64 + b].id);
                    word &= word - 1;
                }
            }
            filter(next);
            // A filter should only remove ids. Keep the list sorted and
            // unique if it did more.
            if !next.windows(2).all(|w| w[0] < w[1]) {
                next.sort_unstable();
                next.dedup();
            }
        }
        diff_into(&view.visible, next, &mut view.entered, &mut view.left);
        std::mem::swap(&mut view.visible, next);
        view
    }

    /// The view computed by the last `update` for this subscriber.
    pub fn view(&self, subscriber: &SubscriberId) -> Option<&Visibility> {
        self.views.get(subscriber)
    }

    /// Drop views of subscribers not in `keep` (they disconnected).
    pub fn retain(&mut self, mut keep: impl FnMut(&SubscriberId) -> bool) {
        self.views.retain(|id, _| keep(id));
    }
}

/// `entered` = next − prev, `left` = prev − next. Both inputs sorted.
fn diff_into(
    prev: &[EntityId],
    next: &[EntityId],
    entered: &mut Vec<EntityId>,
    left: &mut Vec<EntityId>,
) {
    entered.clear();
    left.clear();
    let (mut i, mut j) = (0, 0);
    while i < prev.len() && j < next.len() {
        match prev[i].cmp(&next[j]) {
            std::cmp::Ordering::Less => {
                left.push(prev[i]);
                i += 1;
            }
            std::cmp::Ordering::Greater => {
                entered.push(next[j]);
                j += 1;
            }
            std::cmp::Ordering::Equal => {
                i += 1;
                j += 1;
            }
        }
    }
    left.extend_from_slice(&prev[i..]);
    entered.extend_from_slice(&next[j..]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(id: EntityId, x: f32, y: f32) -> EntityPos {
        EntityPos { id, x, y }
    }

    fn sid(s: &str) -> SubscriberId {
        SubscriberId::new(s)
    }

    fn area(x: f32, y: f32, radius: f32) -> Option<InterestArea> {
        Some(InterestArea { x, y, radius })
    }

    #[test]
    fn sees_entities_within_the_radius_across_cells() {
        let mut m = InterestManager::new(InterestConfig {
            cell_size: 10.0,
            margin: 0.0,
        });
        m.rebuild(&[
            e(1, 0.0, 0.0),
            e(2, 9.0, 0.0),
            e(3, 11.0, 0.0),
            e(4, -9.5, 0.0),
            e(5, 50.0, 50.0),
        ]);
        let v = m.update(&sid("a"), area(0.0, 0.0, 10.0), |_| {});
        assert_eq!(v.visible, vec![1, 2, 4]);
        assert_eq!(v.entered, vec![1, 2, 4]);
        assert!(v.left.is_empty());
    }

    #[test]
    fn hysteresis_keeps_an_entity_until_it_passes_the_margin() {
        let mut m = InterestManager::new(InterestConfig {
            cell_size: 10.0,
            margin: 2.0,
        });
        let me = sid("a");
        // Outside the radius: not seen.
        m.rebuild(&[e(7, 11.0, 0.0)]);
        assert!(m
            .update(&me, area(0.0, 0.0, 10.0), |_| {})
            .visible
            .is_empty());
        // Inside: seen.
        m.rebuild(&[e(7, 9.0, 0.0)]);
        assert_eq!(m.update(&me, area(0.0, 0.0, 10.0), |_| {}).entered, vec![7]);
        // Past the radius but within the margin: still seen, no churn.
        m.rebuild(&[e(7, 11.5, 0.0)]);
        let v = m.update(&me, area(0.0, 0.0, 10.0), |_| {});
        assert_eq!((v.visible.clone(), v.left.clone()), (vec![7], vec![]));
        // Past the margin: leaves.
        m.rebuild(&[e(7, 12.5, 0.0)]);
        let v = m.update(&me, area(0.0, 0.0, 10.0), |_| {});
        assert!(v.visible.is_empty());
        assert_eq!(v.left, vec![7]);
    }

    #[test]
    fn the_filter_hides_an_entity_in_range() {
        let mut m = InterestManager::new(InterestConfig::default());
        m.rebuild(&[e(1, 1.0, 1.0), e(2, 2.0, 2.0)]);
        let v = m.update(&sid("a"), area(0.0, 0.0, 50.0), |ids| {
            ids.retain(|id| *id != 2)
        });
        assert_eq!(v.visible, vec![1]);
        // The same entity is visible to someone the filter admits.
        let v = m.update(&sid("b"), area(0.0, 0.0, 50.0), |_| {});
        assert_eq!(v.visible, vec![1, 2]);
    }

    #[test]
    fn no_area_sees_nothing_and_despawned_entities_leave() {
        let mut m = InterestManager::new(InterestConfig::default());
        let me = sid("a");
        m.rebuild(&[e(1, 1.0, 1.0)]);
        assert_eq!(m.update(&me, area(0.0, 0.0, 10.0), |_| {}).visible, vec![1]);
        m.rebuild(&[]);
        assert_eq!(m.update(&me, area(0.0, 0.0, 10.0), |_| {}).left, vec![1]);
        m.rebuild(&[e(1, 1.0, 1.0)]);
        assert!(m.update(&me, None, |_| {}).visible.is_empty());
    }

    #[test]
    fn bad_positions_and_huge_radii_do_not_panic_or_hang() {
        let mut m = InterestManager::new(InterestConfig {
            cell_size: 1.0,
            margin: 0.0,
        });
        m.rebuild(&[
            e(1, f32::NAN, 0.0),
            e(2, f32::INFINITY, 0.0),
            e(3, 5.0, 5.0),
        ]);
        assert_eq!(m.grid().len(), 1);
        assert_eq!(
            m.update(&sid("a"), area(0.0, 0.0, 1.0e30), |_| {}).visible,
            vec![3]
        );
        assert!(m
            .update(&sid("a"), area(f32::NAN, 0.0, 10.0), |_| {})
            .visible
            .is_empty());
        assert!(m
            .update(&sid("a"), area(0.0, 0.0, -1.0), |_| {})
            .visible
            .is_empty());
    }

    #[test]
    fn diff_lists_entered_and_left() {
        let (mut en, mut le) = (Vec::new(), Vec::new());
        diff_into(&[1, 3, 5, 9], &[2, 3, 9, 10], &mut en, &mut le);
        assert_eq!(en, vec![2, 10]);
        assert_eq!(le, vec![1, 5]);
    }
}
