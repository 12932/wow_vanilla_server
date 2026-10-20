//! A map's spatial grid: which objects are in each 50-yard cell, so the
//! objects near a point are found by looking at a few cells.

use ahash::AHashMap;
use smallvec::SmallVec;
use wow_world_messages::Guid;

pub const CELL_YARDS: f32 = 50.0;

/// The world is about 34,000 yards across; positions beyond this are
/// clamped so a bad coordinate can't overflow cell arithmetic.
const LIMIT: f32 = 20_000.0;

pub type CellKey = (i32, i32);

/// A coordinate as the grid takes it: within the limit, a bad one at 0.
fn clamp(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(-LIMIT, LIMIT)
    } else {
        0.0
    }
}

pub fn cell_of(x: f32, y: f32) -> CellKey {
    (
        (clamp(x) / CELL_YARDS).floor() as i32,
        (clamp(y) / CELL_YARDS).floor() as i32,
    )
}

#[derive(Debug, Default)]
pub struct Grid {
    cells: AHashMap<CellKey, SmallVec<[Guid; 8]>>,
}

impl Grid {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, guid: Guid, cell: CellKey) {
        self.cells.entry(cell).or_default().push(guid);
    }

    pub fn remove(&mut self, guid: Guid, cell: CellKey) {
        if let Some(c) = self.cells.get_mut(&cell) {
            if let Some(i) = c.iter().position(|g| *g == guid) {
                c.swap_remove(i);
            }
            if c.is_empty() {
                self.cells.remove(&cell);
            }
        }
    }

    /// Move `guid` between cells; a no-op within one.
    pub fn relocate(&mut self, guid: Guid, from: CellKey, to: CellKey) {
        if from != to {
            self.remove(guid, from);
            self.insert(guid, to);
        }
    }

    /// Every object in the cells a disc of `radius` around `(x, y)`
    /// touches. Callers check the exact distance.
    pub fn around(&self, x: f32, y: f32, radius: f32) -> impl Iterator<Item = Guid> + '_ {
        let (cx, cy) = cell_of(x, y);
        let r = (radius / CELL_YARDS).ceil() as i32;
        (cx - r..=cx + r).flat_map(move |ix| {
            (cy - r..=cy + r).flat_map(move |iy| {
                self.cells
                    .get(&(ix, iy))
                    .into_iter()
                    .flat_map(|c| c.iter().copied())
            })
        })
    }

    /// [`Grid::around`], less the cells wholly outside the disc: a wide
    /// disc's square has corners of a third of its cells (a flier's sight).
    /// Every object within `radius` is among them still; callers check the
    /// exact distance.
    pub fn within(&self, x: f32, y: f32, radius: f32) -> impl Iterator<Item = Guid> + '_ {
        let (cx, cy) = cell_of(x, y);
        let (x, y) = (clamp(x), clamp(y));
        let r = (radius / CELL_YARDS).ceil() as i32;
        // How far `v` is from the cell `i` spans, along one axis.
        let off = |v: f32, i: i32| {
            let low = i as f32 * CELL_YARDS;
            v.clamp(low, low + CELL_YARDS) - v
        };
        (cx - r..=cx + r).flat_map(move |ix| {
            let dx = off(x, ix);
            (cy - r..=cy + r)
                .filter(move |&iy| {
                    let dy = off(y, iy);
                    dx * dx + dy * dy <= radius * radius
                })
                .flat_map(move |iy| {
                    self.cells
                        .get(&(ix, iy))
                        .into_iter()
                        .flat_map(|c| c.iter().copied())
                })
        })
    }

    pub fn cell_count(&self) -> usize {
        self.cells.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn objects_near_a_point_are_found_and_far_ones_are_not() {
        let mut g = Grid::new();
        let near = Guid::new(1);
        let far = Guid::new(2);
        g.insert(near, cell_of(10.0, 10.0));
        g.insert(far, cell_of(1000.0, 10.0));
        let found: Vec<Guid> = g.around(0.0, 0.0, 100.0).collect();
        assert_eq!(found, vec![near]);
        g.relocate(near, cell_of(10.0, 10.0), cell_of(990.0, 10.0));
        assert_eq!(g.around(0.0, 0.0, 100.0).count(), 0);
        assert_eq!(g.around(1000.0, 0.0, 100.0).count(), 2);
    }

    /// A flier's window, the disc's cells: every object within the radius
    /// is found, as the square finds it, and the square's far corners are
    /// left out.
    #[test]
    fn the_disc_finds_all_within_it_and_leaves_the_corners() {
        let mut g = Grid::new();
        let mut n = 0;
        for ix in -20..=20 {
            for iy in -20..=20 {
                n += 1;
                let (x, y) = (ix as f32 * 23.0 + 3.0, iy as f32 * 23.0 - 7.0);
                g.insert(Guid::new(n), cell_of(x, y));
            }
        }
        let at = |guid: Guid| {
            let i = guid.guid() as i32 - 1;
            (
                (i / 41 - 20) as f32 * 23.0 + 3.0,
                (i % 41 - 20) as f32 * 23.0 - 7.0,
            )
        };
        let (cx, cy, radius) = (12.0, -31.0, 410.0);
        let in_disc = |guid: &Guid| {
            let (x, y) = at(*guid);
            (x - cx).powi(2) + (y - cy).powi(2) <= radius * radius
        };
        let square: Vec<Guid> = g.around(cx, cy, radius).collect();
        let disc: Vec<Guid> = g.within(cx, cy, radius).collect();
        assert!(disc.iter().all(|o| square.contains(o)));
        let wanted: Vec<Guid> = square.iter().copied().filter(in_disc).collect();
        assert!(!wanted.is_empty());
        assert!(
            wanted.iter().all(|o| disc.contains(o)),
            "one within the disc missed"
        );
        assert!(disc.len() < square.len(), "no corner left out");
        // A corner of the square, far past the disc: left out.
        let corner = square.iter().copied().find(|o| {
            let (x, y) = at(*o);
            (x - cx).abs() > 400.0 && (y - cy).abs() > 400.0
        });
        assert!(corner.is_some_and(|o| !disc.contains(&o)));
    }

    #[test]
    fn a_bad_coordinate_lands_in_a_cell() {
        assert_eq!(cell_of(f32::NAN, f32::INFINITY), (0, 0));
        assert_eq!(cell_of(-1e30, 0.0), (-400, 0));
    }
}
