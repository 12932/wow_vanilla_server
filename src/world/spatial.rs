//! Two-level spatial index (cmangos-style) for "visit entities within radius"
//! AOI / aggro / broadcast queries.
//!
//! A map is divided into 533.33 yd **grids** (64×64 per map), each into
//! 33.33 yd **cells** (16×16 per grid → 1024 cells per map axis). Occupants are
//! keyed by the fine 33.33 yd cell; the grid level is kept as a documented
//! constant for parity with cmangos but is not materialized — this server keeps
//! all content resident, so there is no per-grid load/unload to drive.
//!
//! One [`SpatialIndex`] lives on each `MapState` and holds **both** clients and
//! creatures (the slab keys; the slabs themselves live on the `MapState`). Z is
//! ignored — AOI is horizontal-only, matching [`crate::world::aoi::within_aoi`].

use ahash::AHashMap;

/// cmangos grid size (yards): 64 grids per map axis.
pub const GRID_SIZE_YD: f32 = 533.3333;
/// cmangos cell size (yards): 16 cells per grid axis (1024 cells per map axis).
pub const CELL_SIZE_YD: f32 = 33.3333;

/// Fine-cell key for a world `(x, y)`. Floor bucketing matches `within_aoi`.
#[inline]
pub fn cell_coord(x: f32, y: f32) -> (i32, i32) {
    (
        (x / CELL_SIZE_YD).floor() as i32,
        (y / CELL_SIZE_YD).floor() as i32,
    )
}

/// Coarse 533.33-yd grid key for a world `(x, y)` — cmangos's grid layer
/// (16 cells per grid). Used for grid activation: only grids near a player
/// tick their creatures.
#[inline]
pub fn grid_coord(x: f32, y: f32) -> (i32, i32) {
    (
        (x / GRID_SIZE_YD).floor() as i32,
        (y / GRID_SIZE_YD).floor() as i32,
    )
}

/// A slab-keyed occupant of a cell. The slab lives on the owning `MapState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Occupant {
    Client(usize),
    Creature(usize),
}

/// Maps 33.33 yd cells to the occupants currently in them, with reverse indexes
/// for O(1) move/remove. Insert/move/remove are driven from the `MapState`
/// entity lifecycle + movement sites; [`Self::visit`] is the read query.
#[derive(Debug, Default)]
pub struct SpatialIndex {
    cells: AHashMap<(i32, i32), Vec<Occupant>>,
    client_cell_of: AHashMap<usize, (i32, i32)>,
    creature_cell_of: AHashMap<usize, (i32, i32)>,
}

impl SpatialIndex {
    pub fn new() -> Self {
        Self::default()
    }

    fn cell_push(&mut self, cell: (i32, i32), occ: Occupant) {
        self.cells.entry(cell).or_default().push(occ);
    }

    /// Remove `occ` from `cell`'s bucket; drop the bucket when it empties so an
    /// idle map doesn't accumulate empty `Vec`s (mirrors the old `grid_remove`).
    fn cell_remove(&mut self, cell: (i32, i32), occ: Occupant) {
        if let Some(bucket) = self.cells.get_mut(&cell) {
            if let Some(pos) = bucket.iter().position(|&o| o == occ) {
                bucket.swap_remove(pos);
            }
            if bucket.is_empty() {
                self.cells.remove(&cell);
            }
        }
    }

    // ── Clients ──────────────────────────────────────────────────────────
    pub fn insert_client(&mut self, key: usize, x: f32, y: f32) {
        if self.client_cell_of.contains_key(&key) {
            return; // already tracked
        }
        let cell = cell_coord(x, y);
        self.cell_push(cell, Occupant::Client(key));
        self.client_cell_of.insert(key, cell);
    }

    pub fn remove_client(&mut self, key: usize) {
        if let Some(cell) = self.client_cell_of.remove(&key) {
            self.cell_remove(cell, Occupant::Client(key));
        }
    }

    /// Re-seat a client to the cell matching its current position. No-op if the
    /// cell is unchanged; treats an untracked key as an insert.
    pub fn move_client(&mut self, key: usize, x: f32, y: f32) {
        let new_cell = cell_coord(x, y);
        match self.client_cell_of.get(&key).copied() {
            Some(old) if old == new_cell => {}
            Some(old) => {
                self.cell_remove(old, Occupant::Client(key));
                self.cell_push(new_cell, Occupant::Client(key));
                self.client_cell_of.insert(key, new_cell);
            }
            None => {
                self.cell_push(new_cell, Occupant::Client(key));
                self.client_cell_of.insert(key, new_cell);
            }
        }
    }

    // ── Creatures ────────────────────────────────────────────────────────
    pub fn insert_creature(&mut self, key: usize, x: f32, y: f32) {
        if self.creature_cell_of.contains_key(&key) {
            return;
        }
        let cell = cell_coord(x, y);
        self.cell_push(cell, Occupant::Creature(key));
        self.creature_cell_of.insert(key, cell);
    }

    pub fn remove_creature(&mut self, key: usize) {
        if let Some(cell) = self.creature_cell_of.remove(&key) {
            self.cell_remove(cell, Occupant::Creature(key));
        }
    }

    pub fn move_creature(&mut self, key: usize, x: f32, y: f32) {
        let new_cell = cell_coord(x, y);
        match self.creature_cell_of.get(&key).copied() {
            Some(old) if old == new_cell => {}
            Some(old) => {
                self.cell_remove(old, Occupant::Creature(key));
                self.cell_push(new_cell, Occupant::Creature(key));
                self.creature_cell_of.insert(key, new_cell);
            }
            None => {
                self.cell_push(new_cell, Occupant::Creature(key));
                self.creature_cell_of.insert(key, new_cell);
            }
        }
    }

    pub fn is_creature_tracked(&self, key: usize) -> bool {
        self.creature_cell_of.contains_key(&key)
    }

    pub fn is_client_tracked(&self, key: usize) -> bool {
        self.client_cell_of.contains_key(&key)
    }

    /// Number of clients / creatures currently tracked. Used by the
    /// index↔slab membership invariant test.
    pub fn tracked_client_count(&self) -> usize {
        self.client_cell_of.len()
    }

    pub fn tracked_creature_count(&self) -> usize {
        self.creature_cell_of.len()
    }

    /// cmangos `Cell::Visit`: hand every occupant in the cell window spanning
    /// `[center-radius, center+radius]` to `f`. The window is derived from the
    /// actual `radius`, so it never truncates AOI regardless of cell size — the
    /// caller is responsible for the precise squared-distance filter (the index
    /// only narrows candidates).
    pub fn visit(&self, center_x: f32, center_y: f32, radius: f32, mut f: impl FnMut(Occupant)) {
        let (lo_x, lo_y) = cell_coord(center_x - radius, center_y - radius);
        let (hi_x, hi_y) = cell_coord(center_x + radius, center_y + radius);
        for gx in lo_x..=hi_x {
            for gy in lo_y..=hi_y {
                if let Some(bucket) = self.cells.get(&(gx, gy)) {
                    for &occ in bucket {
                        f(occ);
                    }
                }
            }
        }
    }

    /// Number of non-empty cells — for telemetry (`.cells` GM command).
    pub fn occupied_cell_count(&self) -> usize {
        self.cells.len()
    }

    /// Return capacity sized for an earlier peak (e.g. a brief sim spike).
    /// Called from `MapState::shrink_periodic`; safe at any time since the
    /// reverse indexes hold no in-flight state beyond current membership.
    pub fn shrink_to_fit(&mut self) {
        self.cells.shrink_to_fit();
        self.client_cell_of.shrink_to_fit();
        self.creature_cell_of.shrink_to_fit();
        for bucket in self.cells.values_mut() {
            bucket.shrink_to_fit();
        }
    }

    /// Occupants currently bucketed in `cell`, if any. Used by the promote
    /// window-scan to enumerate the creatures in one specific cell.
    pub fn cell_occupants(&self, cell: (i32, i32)) -> Option<&[Occupant]> {
        self.cells.get(&cell).map(Vec::as_slice)
    }

    /// Iterate every non-empty cell with its `(cell_x, cell_y)` and occupants.
    /// Used to project the per-tick `Sync` AoI snapshot from the persistent
    /// membership without re-scanning the slabs.
    pub fn iter_cells(&self) -> impl Iterator<Item = ((i32, i32), &[Occupant])> {
        self.cells.iter().map(|(&k, v)| (k, v.as_slice()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_coord_buckets_by_cell_size() {
        assert_eq!(cell_coord(0.0, 0.0), (0, 0));
        // Just under one cell stays in cell 0; at/over rolls to cell 1.
        assert_eq!(cell_coord(CELL_SIZE_YD - 0.1, 0.0), (0, 0));
        assert_eq!(cell_coord(CELL_SIZE_YD + 0.1, 0.0), (1, 0));
        // Floor handles negatives (no rounding toward zero).
        assert_eq!(cell_coord(-0.1, -0.1), (-1, -1));
    }

    #[test]
    fn visit_finds_within_radius_excludes_beyond() {
        // Mimic an AOI caller: index narrows to a cell window, then the caller
        // applies the precise distance filter. An occupant at 199 yd must be
        // found within a 200 yd query; one at 201 yd must not.
        let mut idx = SpatialIndex::new();
        let radius = 200.0f32;
        let (cx, cy) = (1000.0f32, 1000.0f32);
        idx.insert_creature(1, cx + 199.0, cy); // inside
        idx.insert_creature(2, cx + 201.0, cy); // outside

        let mut found = Vec::new();
        idx.visit(cx, cy, radius, |occ| {
            if let Occupant::Creature(k) = occ {
                // caller's precise filter using the true position
                let px = if k == 1 { cx + 199.0 } else { cx + 201.0 };
                let dx = px - cx;
                if dx * dx <= radius * radius {
                    found.push(k);
                }
            }
        });
        assert!(found.contains(&1), "199 yd occupant should be in range");
        assert!(!found.contains(&2), "201 yd occupant should be out of range");
    }

    #[test]
    fn visit_window_includes_occupant_at_exactly_radius() {
        // No false negatives: an occupant exactly at the radius must be in the
        // visited cell window (the caller's <= filter then keeps it).
        let mut idx = SpatialIndex::new();
        idx.insert_client(7, 200.0, 0.0);
        let mut seen = false;
        idx.visit(0.0, 0.0, 200.0, |occ| {
            if occ == Occupant::Client(7) {
                seen = true;
            }
        });
        assert!(seen, "occupant at exactly the radius must be visited");
    }

    #[test]
    fn insert_move_remove_keep_buckets_consistent() {
        let mut idx = SpatialIndex::new();
        idx.insert_creature(1, 10.0, 10.0); // cell (0,0)
        idx.insert_client(2, 10.0, 10.0); // same cell
        assert_eq!(idx.occupied_cell_count(), 1);

        // Move creature far away → new cell, old bucket still holds the client.
        idx.move_creature(1, 2000.0, 2000.0);
        assert_eq!(idx.occupied_cell_count(), 2);

        // Remove the client → its (now sole) bucket is dropped.
        idx.remove_client(2);
        assert_eq!(idx.occupied_cell_count(), 1);

        // Remove the creature → index empty.
        idx.remove_creature(1);
        assert_eq!(idx.occupied_cell_count(), 0);
    }

    #[test]
    fn move_to_same_cell_is_noop() {
        let mut idx = SpatialIndex::new();
        idx.insert_creature(1, 10.0, 10.0);
        idx.move_creature(1, 12.0, 12.0); // same 33yd cell
        assert_eq!(idx.occupied_cell_count(), 1);
        let mut count = 0;
        idx.visit(10.0, 10.0, 1.0, |_| count += 1);
        assert_eq!(count, 1, "no duplicate after same-cell move");
    }
}
