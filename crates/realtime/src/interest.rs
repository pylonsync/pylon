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
use std::hash::Hash;

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

    /// The cell index of a coordinate. Insertion and lookup both use this
    /// (f64, so they agree for any cell size). `as i32` saturates, so
    /// coordinates far outside the grid clamp.
    fn cell_of(&self, v: f64) -> i32 {
        (v / self.cell_size as f64).floor() as i32
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
            let cell = (self.cell_of(e.x as f64), self.cell_of(e.y as f64));
            self.cells.entry(cell).or_default().push(idx as u32);
        }
        // Drop cells that stayed empty, and trim cells whose buffer is far
        // larger than their contents: a crowd moving cell to cell must not
        // leave a trail of large, empty buffers behind it.
        self.cells.retain(|_, v| {
            if v.capacity() > 64 && v.capacity() > 4 * v.len() {
                v.shrink_to(v.len() * 2);
            }
            !v.is_empty()
        });
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
        self.for_each_index_near(x as f64, y as f64, r as f64, |i| visit(&self.entities[i]));
    }

    /// Like `for_each_near`, with the entity's index (entities are in id
    /// order).
    ///
    /// Takes f64 so the bounds do not overflow for large finite f32 inputs.
    fn for_each_index_near(&self, x: f64, y: f64, r: f64, mut visit: impl FnMut(usize)) {
        if !(x.is_finite() && y.is_finite() && r.is_finite()) || r < 0.0 {
            return;
        }
        let (x0, y0) = (self.cell_of(x - r), self.cell_of(y - r));
        let (x1, y1) = (self.cell_of(x + r), self.cell_of(y + r));
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

/// Computes each view's [`Visibility`] from a [`SpatialGrid`] and keeps
/// last tick's views for hysteresis and the entered/left lists.
///
/// Views are keyed by `K`. A shard keys them by subscription (one per
/// connection), not by subscriber id: two connections with one id, or a
/// reconnect, each get their own history, so each sees `entered` for every
/// entity it has not been told about.
#[derive(Debug)]
pub struct InterestManager<K = u64> {
    config: InterestConfig,
    grid: SpatialGrid,
    views: HashMap<K, Visibility>,
    scratch: Vec<EntityId>,
    /// One bit per grid entity: in view this update.
    bits: Vec<u64>,
}

impl<K: Hash + Eq + Clone> InterestManager<K> {
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
        key: &K,
        area: Option<InterestArea>,
        filter: impl FnOnce(&mut Vec<EntityId>),
    ) -> &Visibility {
        let margin = if self.config.margin.is_finite() {
            self.config.margin.max(0.0) as f64
        } else {
            0.0
        };
        let view = self.views.entry(key.clone()).or_default();
        let next = &mut self.scratch;
        next.clear();
        let area = area.filter(|a| {
            a.x.is_finite() && a.y.is_finite() && a.radius.is_finite() && a.radius >= 0.0
        });
        if let Some(area) = area {
            // f64: squares of large finite f32 values overflow f32 to
            // infinity, which would admit entities at any distance.
            let (ax, ay, radius) = (area.x as f64, area.y as f64, area.radius as f64);
            let r2 = radius * radius;
            let keep = radius + margin;
            let keep2 = keep * keep;
            let prev = &view.visible;
            let entities = &self.grid.entities;
            let bits = &mut self.bits;
            bits.clear();
            bits.resize(entities.len().div_ceil(64), 0);
            self.grid.for_each_index_near(ax, ay, keep, |i| {
                let e = &entities[i];
                let (dx, dy) = (e.x as f64 - ax, e.y as f64 - ay);
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

    /// The view computed by the last `update` for this key.
    pub fn view(&self, key: &K) -> Option<&Visibility> {
        self.views.get(key)
    }

    /// Drop views whose key `keep` rejects (the connection closed).
    pub fn retain(&mut self, mut keep: impl FnMut(&K) -> bool) {
        self.views.retain(|k, _| keep(k));
    }
}

impl<K: Hash + Eq + Clone> Default for InterestManager<K> {
    fn default() -> Self {
        Self::new(InterestConfig::default())
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

    fn sid(s: &str) -> String {
        s.to_string()
    }

    fn area(x: f32, y: f32, radius: f32) -> Option<InterestArea> {
        Some(InterestArea { x, y, radius })
    }

    #[test]
    fn sees_entities_within_the_radius_across_cells() {
        let mut m: InterestManager<String> = InterestManager::new(InterestConfig {
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
        let mut m: InterestManager<String> = InterestManager::new(InterestConfig {
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
        let mut m: InterestManager<String> = InterestManager::new(InterestConfig::default());
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
        let mut m: InterestManager<String> = InterestManager::new(InterestConfig::default());
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
        let mut m: InterestManager<String> = InterestManager::new(InterestConfig {
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
    fn huge_finite_coordinates_do_not_admit_distant_entities() {
        let mut m: InterestManager<String> = InterestManager::new(InterestConfig {
            cell_size: 10.0,
            margin: 0.0,
        });
        m.rebuild(&[e(1, 2.0e20, 0.0), e(2, 0.5e20, 0.0)]);
        let v = m.update(&sid("a"), area(0.0, 0.0, 1.0e20), |_| {});
        assert_eq!(v.visible, vec![2]);
        // radius + margin past f32::MAX still finds entities in range.
        let mut m: InterestManager<String> = InterestManager::new(InterestConfig {
            cell_size: 10.0,
            margin: f32::MAX,
        });
        m.rebuild(&[e(1, 1.0, 1.0)]);
        assert_eq!(
            m.update(&sid("a"), area(0.0, 0.0, f32::MAX), |_| {})
                .visible,
            vec![1]
        );
    }

    #[test]
    fn a_crowd_moving_across_cells_leaves_no_empty_buffers() {
        let mut g = SpatialGrid::new(1.0);
        for step in 0..50 {
            let crowd: Vec<EntityPos> = (0..500).map(|i| e(i, step as f32 * 3.0, 0.5)).collect();
            g.rebuild(&crowd);
        }
        assert_eq!(g.cells.len(), 1);
    }

    #[test]
    fn an_entity_is_visible_at_its_own_position_with_a_fractional_cell() {
        // 1.0 / 0.1f32 is 9.99999 in f32 and 10.000000149 in f64: insertion
        // and lookup must use the same arithmetic.
        let mut m: InterestManager<String> = InterestManager::new(InterestConfig {
            cell_size: 0.1,
            margin: 0.0,
        });
        m.rebuild(&[e(1, 1.0, 0.0), e(2, 0.3, 0.7)]);
        assert_eq!(
            m.update(&sid("a"), area(1.0, 0.0, 0.0), |_| {}).visible,
            vec![1]
        );
        assert_eq!(
            m.update(&sid("b"), area(0.3, 0.7, 0.0), |_| {}).visible,
            vec![2]
        );
    }

    #[test]
    fn diff_lists_entered_and_left() {
        let (mut en, mut le) = (Vec::new(), Vec::new());
        diff_into(&[1, 3, 5, 9], &[2, 3, 9, 10], &mut en, &mut le);
        assert_eq!(en, vec![2, 10]);
        assert_eq!(le, vec![1, 5]);
    }
}
