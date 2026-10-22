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

/// An object in a cell, with what visibility's quick path needs of it
/// without looking it up (`visibility::refresh_dirty`): where its
/// visibility was last worked out (its anchor), and whether it is plain
/// (who sees it, and whom it sees, turns on distance alone: see
/// `visibility::plain_now`). An object is plain only once its own refresh
/// has said so, and its anchor is set then.
#[derive(Debug, Clone, Copy)]
pub struct Entry {
    pub guid: Guid,
    pub ax: f32,
    pub ay: f32,
    pub plain: bool,
    /// Its place (from 1) in the refreshes under way, 0 if none.
    pub batch: u32,
}

/// A cell's objects, and how many of them are players and how many not
/// plain.
#[derive(Debug, Default)]
pub struct Cell {
    entries: SmallVec<[Entry; 8]>,
    players: u32,
    unplain: u32,
}

impl Cell {
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn players(&self) -> u32 {
        self.players
    }

    pub fn unplain(&self) -> u32 {
        self.unplain
    }

    fn push(&mut self, e: Entry) {
        self.players += u32::from(crate::game::guid::is_player(e.guid));
        self.unplain += u32::from(!e.plain);
        self.entries.push(e);
    }

    fn take(&mut self, guid: Guid) -> Option<Entry> {
        let i = self.entries.iter().position(|e| e.guid == guid)?;
        let e = self.entries.swap_remove(i);
        self.players -= u32::from(crate::game::guid::is_player(e.guid));
        self.unplain -= u32::from(!e.plain);
        Some(e)
    }
}

#[derive(Debug, Default)]
pub struct Grid {
    cells: AHashMap<CellKey, Cell>,
}

impl Grid {
    pub fn new() -> Self {
        Self::default()
    }

    /// `guid` comes into `cell`: not plain, until its refresh.
    pub fn insert(&mut self, guid: Guid, cell: CellKey) {
        self.cells.entry(cell).or_default().push(Entry {
            guid,
            ax: f32::NAN,
            ay: f32::NAN,
            plain: false,
            batch: 0,
        });
    }

    pub fn remove(&mut self, guid: Guid, cell: CellKey) {
        if let Some(c) = self.cells.get_mut(&cell) {
            c.take(guid);
            if c.entries.is_empty() {
                self.cells.remove(&cell);
            }
        }
    }

    /// Move `guid` between cells, with what is known of it; a no-op within
    /// one. One not in `from` comes into `to` as new.
    pub fn relocate(&mut self, guid: Guid, from: CellKey, to: CellKey) {
        if from == to {
            return;
        }
        let taken = self.cells.get_mut(&from).and_then(|c| c.take(guid));
        if self.cells.get(&from).is_some_and(|c| c.entries.is_empty()) {
            self.cells.remove(&from);
        }
        match taken {
            Some(e) => self.cells.entry(to).or_default().push(e),
            None => self.insert(guid, to),
        }
    }

    /// `guid`'s entry in `cell`, if it is there.
    pub fn entry(&self, guid: Guid, cell: CellKey) -> Option<&Entry> {
        self.cells
            .get(&cell)
            .and_then(|c| c.entries.iter().find(|e| e.guid == guid))
    }

    /// Change `guid`'s entry in `cell`, if it is there, its cell's count of
    /// those not plain kept.
    pub fn update(&mut self, guid: Guid, cell: CellKey, f: impl FnOnce(&mut Entry)) {
        let Some(c) = self.cells.get_mut(&cell) else {
            return;
        };
        let Some(e) = c.entries.iter_mut().find(|e| e.guid == guid) else {
            return;
        };
        let was = e.plain;
        f(e);
        match (was, e.plain) {
            (true, false) => c.unplain += 1,
            (false, true) => c.unplain -= 1,
            _ => {}
        }
    }

    /// Take nothing as plain, until each object's refresh says so again.
    pub fn unplain_all(&mut self) {
        for c in self.cells.values_mut() {
            for e in &mut c.entries {
                e.plain = false;
            }
            c.unplain = c.entries.len() as u32;
        }
    }

    /// The cells of [`Grid::around`], with their keys.
    pub fn cells_around(
        &self,
        x: f32,
        y: f32,
        radius: f32,
    ) -> impl Iterator<Item = (CellKey, &Cell)> + '_ {
        let (cx, cy) = cell_of(x, y);
        let r = (radius / CELL_YARDS).ceil() as i32;
        (cx - r..=cx + r).flat_map(move |ix| {
            (cy - r..=cy + r)
                .filter_map(move |iy| self.cells.get(&(ix, iy)).map(|c| ((ix, iy), c)))
        })
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
                    .flat_map(|c| c.entries.iter().map(|e| e.guid))
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
                        .flat_map(|c| c.entries.iter().map(|e| e.guid))
                })
        })
    }

    pub fn cell_count(&self) -> usize {
        self.cells.len()
    }
}

/// The farthest a point in `cell` is from `(x, y)`, squared.
pub fn farthest2(cell: CellKey, x: f32, y: f32) -> f32 {
    let far = |v: f32, i: i32| {
        let low = i as f32 * CELL_YARDS;
        (v - low).abs().max((low + CELL_YARDS - v).abs())
    };
    let (dx, dy) = (far(x, cell.0), far(y, cell.1));
    dx * dx + dy * dy
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
