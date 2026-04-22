//! Area-of-interest primitives for MMO-scale shards.
//!
//! For small matches, every subscriber sees the same snapshot. For zones
//! with hundreds or thousands of players, snapshots must be filtered so
//! each client receives only what's near them (or otherwise visible).
//!
//! This module provides trait + helpers; actual filtering is done by the
//! user's [`SimState::snapshot_for`] implementation.

use serde::Serialize;

/// Describes what a subscriber can see.
///
/// Attached to a [`Subscriber`](crate::subscriber::Subscriber) and consulted
/// by the sim state when producing per-subscriber snapshots.
pub trait AreaOfInterest: Send + Sync {
    /// Is the given world-space point inside this subscriber's interest area?
    fn contains(&self, x: f32, y: f32, z: f32) -> bool;
}

// ---------------------------------------------------------------------------
// Built-in AOI shapes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SphereAoi {
    pub center: (f32, f32, f32),
    pub radius: f32,
}

impl AreaOfInterest for SphereAoi {
    fn contains(&self, x: f32, y: f32, z: f32) -> bool {
        let dx = x - self.center.0;
        let dy = y - self.center.1;
        let dz = z - self.center.2;
        dx * dx + dy * dy + dz * dz <= self.radius * self.radius
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct BoxAoi {
    pub min: (f32, f32, f32),
    pub max: (f32, f32, f32),
}

impl AreaOfInterest for BoxAoi {
    fn contains(&self, x: f32, y: f32, z: f32) -> bool {
        x >= self.min.0
            && x <= self.max.0
            && y >= self.min.1
            && y <= self.max.1
            && z >= self.min.2
            && z <= self.max.2
    }
}

/// Grid-cell AOI — subscriber sees entities in their cell plus neighbors.
/// Good for large worlds where spatial hashing beats per-entity distance checks.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct GridAoi {
    pub cell_x: i32,
    pub cell_y: i32,
    pub cell_size: f32,
    /// Number of adjacent cells visible (0 = only own cell, 1 = 3x3, 2 = 5x5).
    pub radius_cells: i32,
}

impl AreaOfInterest for GridAoi {
    fn contains(&self, x: f32, _y: f32, z: f32) -> bool {
        // Use x/z as horizontal plane (common for 3D world-floor convention).
        let cx = (x / self.cell_size).floor() as i32;
        let cz = (z / self.cell_size).floor() as i32;
        (cx - self.cell_x).abs() <= self.radius_cells
            && (cz - self.cell_y).abs() <= self.radius_cells
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sphere_contains_center() {
        let aoi = SphereAoi {
            center: (0.0, 0.0, 0.0),
            radius: 10.0,
        };
        assert!(aoi.contains(0.0, 0.0, 0.0));
        assert!(aoi.contains(5.0, 0.0, 0.0));
        assert!(!aoi.contains(11.0, 0.0, 0.0));
    }

    #[test]
    fn box_axis_aligned() {
        let aoi = BoxAoi {
            min: (-5.0, -5.0, -5.0),
            max: (5.0, 5.0, 5.0),
        };
        assert!(aoi.contains(0.0, 0.0, 0.0));
        assert!(aoi.contains(5.0, 5.0, 5.0));
        assert!(!aoi.contains(6.0, 0.0, 0.0));
    }

    #[test]
    fn grid_neighbors() {
        let aoi = GridAoi {
            cell_x: 0,
            cell_y: 0,
            cell_size: 10.0,
            radius_cells: 1,
        };
        // Own cell (0..10 on x).
        assert!(aoi.contains(5.0, 0.0, 5.0));
        // Adjacent cell (10..20).
        assert!(aoi.contains(15.0, 0.0, 5.0));
        // Two cells away.
        assert!(!aoi.contains(25.0, 0.0, 5.0));
    }
}
