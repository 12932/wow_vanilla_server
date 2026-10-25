//! A map's navmesh, from a rustigator bake: paths, ground heights, line of
//! sight, and the zone and area a point is in.
//!
//! Its ADT tiles load as they are first needed, and go again once no one
//! has been near one for a while (`[world] tile_unload_secs`). On the
//! server they are read off the map's thread (`map/tiles.rs`): until a
//! tile is in, a query over it answers as a map with no navmesh would (a
//! path is straight, a line of sight clear, a height unknown), so the map
//! never waits on the disk. Tests load them at once, as they are asked for.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ahash::AHashMap;
use glam::Vec3;
use rustigator_pathfind::PathfindMap;
use rustigator_pathfind::format::NavFile;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use wow_world_messages::vanilla::Vector3d;

/// An ADT tile's side, in yards.
pub const ADT_SIZE: f32 = 533.333_3;
/// How far under a floor its area is asked for.
const AREA_FLOOR_TOLERANCE: f32 = 0.1;
/// How long an update spends taking in tiles read: a busy tile's
/// navmesh takes milliseconds to join the rest, and many may arrive at
/// once (a player logging in, a populated map).
const TAKE_BUDGET: Duration = Duration::from_millis(3);
/// How far a path's line may leave the ground under it before a point on
/// the ground is put in, and how often along it that is looked at: a path
/// is its polygons' corners, joined straight, and a dune or a slope
/// between them (or a goal taken at a victim's height down a slope) would
/// have the creature walk through the ground. mangos's smooth path puts a
/// point every 4 yards at its polygons' heights (`SMOOTH_PATH_STEP_SIZE`).
/// The ground is the floor itself (the bake's geometry), not the
/// navmesh's polygons, which are a quarter yard and more off it; at half a
/// yard, Drek'Thar and his guards walked from the ring of his room into its
/// sunken middle a yard lower at the ring's height, and stood there up to
/// three quarters of a yard in the air (the fight record, decision 106).
const GROUND_TOLERANCE: f32 = 0.15;
/// How far from the navmesh's height under a point its floor is looked
/// for: the floor nearest the navmesh's within this, else the navmesh's.
/// A step's climb: the detail mesh is up to 1.6 yd over the terrain on the
/// valley's steeper slopes (by Tower Point), where at 1.5 a Frostwolf
/// Guardian's walk ended on the navmesh, 1.64 yd in the air (decision 107).
const SURFACE_SNAP: f32 = 2.5;
const GROUND_SAMPLE: f32 = 1.0;
/// How far over or under a point its ground is looked for: as far as a
/// path's ends are taken to the navmesh from.
const GROUND_REACH: f32 = 5.0;
/// The most points a path is given so.
const GROUND_POINTS: usize = 64;

pub struct Nav {
    map: PathfindMap,
    dir: PathBuf,
    name: String,
    /// Tiles asked for: loading, in, or not to be had (not in the bake, or
    /// not read), so a missing one isn't asked for again.
    tiles: AHashMap<(i32, i32), Tile>,
    /// Where tiles read off the map's thread arrive; `None` to read them at
    /// once.
    background: Option<Background>,
    /// The map's time, as of its last update: when a tile was last used.
    now: Instant,
    heights: Vec<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Loading,
    In,
    Absent,
}

#[derive(Debug, Clone, Copy)]
struct Tile {
    state: State,
    used: Instant,
}

/// A tile read: its indices, and its file (`None` if it wasn't to be had).
type Arrival = ((i32, i32), Option<NavFile>);

struct Background {
    tx: mpsc::UnboundedSender<Arrival>,
    rx: mpsc::UnboundedReceiver<Arrival>,
    /// Arrived and not yet taken in.
    arrived: VecDeque<Arrival>,
}

impl std::fmt::Debug for Nav {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Nav({}: {} tiles in, {} loading)",
            self.name,
            self.tiles_in(),
            self.tiles_loading()
        )
    }
}

/// The ADT tile `(x, y)` is in (rustigator's `world_to_adt`).
pub fn tile_of(x: f32, y: f32) -> (i32, i32) {
    let tx = (32.0 - y / ADT_SIZE).floor() as i32;
    let ty = (32.0 - x / ADT_SIZE).floor() as i32;
    (tx.clamp(0, 63), ty.clamp(0, 63))
}

/// A zone and area the bake names, if it names an area: 0 and the
/// unsigned sentinel `u32::MAX` name none (Thistle Tea's
/// `get_zone_and_area`, which rejects both on Programmer Isle).
pub fn known(ids: Option<(u32, u32)>) -> Option<(u32, u32)> {
    ids.filter(|&(_, area)| area != 0 && area != u32::MAX)
}

fn v(p: Vector3d) -> Vec3 {
    Vec3::new(p.x, p.y, p.z)
}

fn d(p: Vec3) -> Vector3d {
    Vector3d {
        x: p.x,
        y: p.y,
        z: p.z,
    }
}

/// A path's points, as the client's vectors.
struct Points(Vec<Vector3d>);

impl Extend<Vec3> for Points {
    fn extend<I: IntoIterator<Item = Vec3>>(&mut self, iter: I) {
        self.0.extend(iter.into_iter().map(d));
    }
}

/// Tile `(x, y)` of the map `name` in the bake at `dir`, read and decoded;
/// `None` if it isn't to be had.
fn read(dir: &Path, name: &str, (x, y): (i32, i32)) -> Option<NavFile> {
    let _z = crate::zone!("navmesh tile");
    let path = dir.join(format!("{name}_{x}_{y}.nav"));
    let file = std::fs::read(&path)
        .map_err(|e| e.to_string())
        .and_then(|b| NavFile::from_bytes(&b).map_err(|e| e.to_string()));
    match file {
        Ok(f) if (f.adt_x, f.adt_y) == (x, y) => Some(f),
        Ok(_) => {
            warn!("{} is another tile's", path.display());
            None
        }
        Err(e) => {
            debug!("no navmesh tile {}: {e}", path.display());
            None
        }
    }
}

/// The tiles `map`'s header lists, and how many of them have no file in
/// `dir`.
fn missing_tiles(map: &PathfindMap, dir: &Path, name: &str) -> (usize, usize) {
    if !map.has_adts() {
        return (0, 0);
    }
    let listed: Vec<(i32, i32)> = (0..64)
        .flat_map(|x| (0..64).map(move |y| (x, y)))
        .filter(|&(x, y)| map.has_adt(x, y))
        .collect();
    let missing = listed
        .iter()
        .filter(|(x, y)| !dir.join(format!("{name}_{x}_{y}.nav")).is_file())
        .count();
    (listed.len(), missing)
}

impl Nav {
    /// The navmesh for the map called `name` in the bake at `dir`, its
    /// tiles read at once as they are asked for (a dungeon that is one WMO
    /// is read whole, now).
    pub fn open(dir: &Path, name: &str) -> Option<Self> {
        match PathfindMap::new(dir, name) {
            Ok(map) => {
                info!("navmesh for {name} from {}", dir.display());
                let (listed, missing) = missing_tiles(&map, dir, name);
                if missing > 0 {
                    warn!(
                        "navmesh for {name}: {missing} of its {listed} tiles aren't in {} \
                         (a bake copied in part, or a disk that filled): creatures there walk \
                         straight lines; `wvs-setup` builds them again",
                        dir.display()
                    );
                }
                Some(Self {
                    map,
                    dir: dir.to_owned(),
                    name: name.to_owned(),
                    tiles: AHashMap::new(),
                    background: None,
                    now: Instant::now(),
                    heights: Vec::new(),
                })
            }
            Err(e) => {
                warn!("no navmesh for {name} in {}: {e}", dir.display());
                None
            }
        }
    }

    /// From now its tiles are read off the map's thread, and taken in at
    /// [`Nav::receive`].
    pub fn in_background(&mut self) {
        let (tx, rx) = mpsc::unbounded_channel();
        self.background = Some(Background {
            tx,
            rx,
            arrived: VecDeque::new(),
        });
    }

    /// Whether the tile under `(x, y)` can be asked about ([`Nav::ready_tile`]).
    fn ready(&mut self, x: f32, y: f32) -> bool {
        if !x.is_finite() || !y.is_finite() {
            return true;
        }
        self.ready_tile(tile_of(x, y))
    }

    /// Whether tile `t` can be asked about: in, or not to be had (as it was
    /// before tiles loaded in the background). One not asked for yet is:
    /// at once, or off the map's thread, when this is false until it
    /// arrives.
    fn ready_tile(&mut self, t: (i32, i32)) -> bool {
        if !self.map.has_adts() {
            return true;
        }
        let now = self.now;
        if let Some(tile) = self.tiles.get_mut(&t) {
            tile.used = now;
            return tile.state != State::Loading;
        }
        if !self.map.has_adt(t.0, t.1) {
            self.tiles.insert(
                t,
                Tile {
                    state: State::Absent,
                    used: now,
                },
            );
            return true;
        }
        match &self.background {
            Some(b) => {
                let (tx, dir, name) = (b.tx.clone(), self.dir.clone(), self.name.clone());
                crate::game::map::tiles::read_into(tx, t, None, move || read(&dir, &name, t));
                self.tiles.insert(
                    t,
                    Tile {
                        state: State::Loading,
                        used: now,
                    },
                );
                false
            }
            None => {
                let file = read(&self.dir, &self.name, t);
                self.take(t, file);
                true
            }
        }
    }

    /// Whether every point's tile can be asked about ([`Nav::ready`]), each
    /// asked for if it isn't in.
    pub fn ready_at(&mut self, points: &[Vector3d]) -> bool {
        points
            .iter()
            .fold(true, |all, p| self.ready(p.x, p.y) & all)
    }

    /// Tile `t` (`tile_of`) is wanted soon: read in the background if it
    /// isn't in, and kept. Reading at once, it is only kept.
    pub fn want(&mut self, t: (i32, i32)) {
        if self.background.is_some() {
            self.ready_tile(t);
        } else {
            self.keep(t);
        }
    }

    /// Tile `t` (`tile_of`) is near someone: kept, if it is in.
    pub fn keep(&mut self, t: (i32, i32)) {
        if let Some(tile) = self.tiles.get_mut(&t) {
            tile.used = self.now;
        }
    }

    fn take(&mut self, t: (i32, i32), file: Option<NavFile>) {
        let state = match file.map(|f| self.map.load_nav_file(f)) {
            Some(Ok(_)) => State::In,
            Some(Err(e)) => {
                warn!("navmesh tile {}_{}_{}: {e}", self.name, t.0, t.1);
                State::Absent
            }
            None => State::Absent,
        };
        let used = self.now;
        self.tiles.insert(t, Tile { state, used });
    }

    /// The map's update: its time, and the tiles read since the last, as
    /// many as [`TAKE_BUDGET`] allows (at least one). How many came in.
    pub fn receive(&mut self, now: Instant) -> usize {
        self.now = now;
        let Some(b) = &mut self.background else {
            return 0;
        };
        b.arrived
            .extend(std::iter::from_fn(|| b.rx.try_recv().ok()));
        let started = Instant::now();
        let mut n = 0;
        while n == 0 || started.elapsed() < TAKE_BUDGET {
            let Some((t, file)) = self.background.as_mut().and_then(|b| b.arrived.pop_front())
            else {
                break;
            };
            n += 1;
            if self
                .tiles
                .get(&t)
                .is_some_and(|tile| tile.state == State::Loading)
            {
                self.take(t, file);
            }
        }
        n
    }

    /// Let go of the tiles in that haven't been used or wanted for `idle`:
    /// how many. One needed again is read again.
    pub fn unload_idle(&mut self, idle: Duration) -> usize {
        self.unload_idle_within(idle, Duration::MAX).0
    }

    /// [`Nav::unload_idle`], as many as `budget` allows (at least one):
    /// how many, and whether idle ones are left. A busy tile takes
    /// milliseconds to drop (and the disk's time, where its idle pages were
    /// swapped out), so the map spreads a sweep over its updates.
    pub fn unload_idle_within(&mut self, idle: Duration, budget: Duration) -> (usize, bool) {
        let now = self.now;
        let mut idle: Vec<(i32, i32)> = self
            .tiles
            .iter()
            .filter(|(_, t)| t.state == State::In && now.saturating_duration_since(t.used) > idle)
            .map(|(k, _)| *k)
            .collect();
        let started = Instant::now();
        let mut n = 0;
        while let Some((x, y)) = idle.pop() {
            let _ = self.map.unload_adt(x, y);
            self.tiles.remove(&(x, y));
            n += 1;
            if started.elapsed() >= budget {
                break;
            }
        }
        (n, !idle.is_empty())
    }

    /// The zone and area at a point; where the point says nothing, those
    /// of the floor nearest it, as Thistle Tea looks.
    pub fn zone_and_area(&mut self, p: Vector3d) -> Option<(u32, u32)> {
        if !self.ready(p.x, p.y) {
            return None;
        }
        if let Some(ids) = known(self.map.get_zone_and_area(p.x, p.y, p.z).ok()) {
            return Some(ids);
        }
        self.map.find_heights(p.x, p.y, &mut self.heights).ok()?;
        let mut floors = self.heights.clone();
        floors.sort_by(|a, b| (a - p.z).abs().total_cmp(&(b - p.z).abs()));
        floors.into_iter().find_map(|h| {
            known(
                self.map
                    .get_zone_and_area(p.x, p.y, h - AREA_FLOOR_TOLERANCE)
                    .ok(),
            )
        })
    }

    /// A walkable path from `from` to `to`, its first point `from`;
    /// straight while a tile of its ends is loading. The navmesh's first
    /// point is `from` put on its polygon, which at a polygon's edge (a
    /// tile's) is the edge's own height: it is `from` itself, where the
    /// walker is, as mangos's `MoveSplineInit::Launch` puts its real
    /// position first. Before, a Frostwolf Legionnaire set off on a tile's
    /// edge from 2.8 yd in the air (decision 107).
    pub fn path(&mut self, from: Vector3d, to: Vector3d) -> Option<Vec<Vector3d>> {
        if !self.ready_at(&[from, to]) {
            return Some(vec![from, to]);
        }
        let mut points = Points(Vec::new());
        self.map
            .find_path(v(from), v(to), false, &mut points)
            .ok()?;
        if points.0.len() < 2 {
            return None;
        }
        points.0[0] = from;
        Some(self.grounded(&points.0))
    }

    /// [`Nav::path`] as the navmesh answers it, not put on the ground: for
    /// the probe of what that costs.
    #[cfg(test)]
    pub fn path_as_found(&mut self, from: Vector3d, to: Vector3d) -> Option<Vec<Vector3d>> {
        let mut points = Points(Vec::new());
        self.map
            .find_path(v(from), v(to), false, &mut points)
            .ok()?;
        (points.0.len() >= 2).then_some(points.0)
    }

    /// The floor under a point: the navmesh's polygon there, within
    /// [`GROUND_REACH`] of it, and the surface of the bake's geometry (its
    /// terrain's, WMOs' and doodads' upward faces) nearest that polygon's
    /// height within [`SURFACE_SNAP`]; else the polygon's height. Off the
    /// navmesh (a straight walk where it has no way), the surface nearest
    /// the point itself within [`SURFACE_SNAP`].
    fn floor_at(&mut self, p: Vector3d) -> Option<f32> {
        if !p.x.is_finite() || !p.y.is_finite() || !p.z.is_finite() {
            return None;
        }
        let detail = self.ground_near(p);
        let near = detail.unwrap_or(p.z);
        if self.map.find_heights(p.x, p.y, &mut self.heights).is_ok()
            && let Some(z) = self
                .heights
                .iter()
                .copied()
                .filter(|h| (h - near).abs() <= SURFACE_SNAP)
                .min_by(|a, b| (a - near).abs().total_cmp(&(b - near).abs()))
        {
            return Some(z);
        }
        detail
    }

    /// The straight way from `from` to `to`, for a walk the navmesh has no
    /// way for (a goal off it, or on a polygon it joins to nothing): on the ground as
    /// a path is ([`Nav::grounded`]), its end on its floor and points put
    /// in where the line leaves the ground; straight while a tile of its
    /// ends is loading. Before, it was the line itself: a chase down a
    /// Tanaris dune's face, to a goal under a doodad's overhang, went
    /// through the dune and ended at its victim's height, a yard off the
    /// slope (decision 107).
    pub fn straight(&mut self, from: Vector3d, to: Vector3d) -> Vec<Vector3d> {
        if !self.ready_at(&[from, to]) {
            return vec![from, to];
        }
        self.grounded(&[from, to])
    }

    /// The navmesh's ground under a point (its detail mesh's height),
    /// within [`GROUND_REACH`] of it. A point on a polygon's edge (a path's
    /// corner always is) is over none of them, to the float: its ground is
    /// that of the nearest point of the nearest polygon's edge, as
    /// Detour's `closestPointOnPoly` has it. Before, a corner had none, and
    /// kept the polygon's own height, up to 1.2 yd over the valley's slopes
    /// and round its rocks (decision 107).
    fn ground_near(&self, p: Vector3d) -> Option<f32> {
        if !p.x.is_finite() || !p.y.is_finite() || !p.z.is_finite() {
            return None;
        }
        let nav = self.map.navmesh();
        let at = [-p.y, p.z, -p.x];
        let filter = rustigator_pathfind::navmesh::QueryFilter::any();
        let (poly, nearest) = nav.find_nearest_poly(at, [0.25, GROUND_REACH, 0.25], &filter);
        if poly.is_null() {
            return None;
        }
        nav.poly_height(poly, at)
            .or(Some(nearest[1]))
            .filter(|z| (z - p.z).abs() <= GROUND_REACH)
    }

    /// A path's points on the ground: each after the first (where the
    /// walker is) at the ground under it if it is farther from it than
    /// [`GROUND_TOLERANCE`] (one on the ground is left alone), and, where the line between two
    /// leaves the ground by more than [`GROUND_TOLERANCE`], a point on the
    /// ground put in where it leaves it most, until none does.
    fn grounded(&mut self, path: &[Vector3d]) -> Vec<Vector3d> {
        let mut out = Vec::with_capacity(path.len());
        out.push(path[0]);
        let mut budget = GROUND_POINTS;
        for &p in &path[1..] {
            let mut b = p;
            if let Some(z) = self.floor_at(b)
                && (z - b.z).abs() > GROUND_TOLERANCE
            {
                b.z = z;
            }
            let a = out[out.len() - 1];
            // A point where the last is (the navmesh's corner at the walker,
            // both now on the same floor) is left out: a walk's next point
            // is somewhere to go.
            if (a.x - b.x).hypot(a.y - b.y) < 0.01 && (a.z - b.z).abs() < 0.01 {
                continue;
            }
            self.fill(a, b, &mut out, &mut budget);
            out.push(b);
        }
        if out.len() < 2 {
            out.push(path[path.len() - 1]);
        }
        out
    }

    /// The points on the ground between `a` and `b` their line needs.
    fn fill(&mut self, a: Vector3d, b: Vector3d, out: &mut Vec<Vector3d>, budget: &mut usize) {
        let planar = (b.x - a.x).hypot(b.y - a.y);
        let steps = (planar / GROUND_SAMPLE).ceil() as usize;
        if steps < 2 || *budget == 0 {
            return;
        }
        let mut worst: Option<(f32, Vector3d)> = None;
        for i in 1..steps {
            let f = i as f32 / steps as f32;
            let p = Vector3d {
                x: a.x + (b.x - a.x) * f,
                y: a.y + (b.y - a.y) * f,
                z: a.z + (b.z - a.z) * f,
            };
            if let Some(z) = self.floor_at(p) {
                let off = (z - p.z).abs();
                if off > GROUND_TOLERANCE && worst.is_none_or(|(o, _)| off > o) {
                    worst = Some((off, Vector3d { z, ..p }));
                }
            }
        }
        if let Some((_, m)) = worst {
            *budget -= 1;
            self.fill(a, m, out, budget);
            out.push(m);
            self.fill(m, b, out, budget);
        }
    }

    /// A random walkable point within `radius` of `center`.
    pub fn random_point(&mut self, center: Vector3d, radius: f32) -> Option<Vector3d> {
        if !self.ready(center.x, center.y) {
            return None;
        }
        self.map
            .find_random_point_around_circle(v(center), radius)
            .ok()
            .map(d)
    }

    /// Where a walk over the navmesh from `from` toward `to` stops
    /// (Thistle Tea's `walk_hit_position`); `None` off the navmesh.
    pub fn walk_hit(&mut self, from: Vector3d, to: Vector3d) -> Option<Vector3d> {
        if !self.ready_at(&[from, to]) {
            return None;
        }
        self.map
            .walk_hit_position(v(from), v(to))
            .ok()
            .flatten()
            .map(d)
    }

    /// The heights at `(x, y)`, into `self.heights`; false if none.
    fn heights_at(&mut self, x: f32, y: f32) -> bool {
        self.ready(x, y) && self.map.find_heights(x, y, &mut self.heights).is_ok()
    }

    /// The ground nearest `z_hint` at `(x, y)`.
    pub fn ground(&mut self, x: f32, y: f32, z_hint: f32) -> Option<f32> {
        if !self.heights_at(x, y) {
            return None;
        }
        self.heights
            .iter()
            .copied()
            .min_by(|a, b| (a - z_hint).abs().total_cmp(&(b - z_hint).abs()))
    }

    /// The highest ground at `(x, y)` no higher than `z`.
    pub fn floor_under(&mut self, x: f32, y: f32, z: f32) -> Option<f32> {
        if !self.heights_at(x, y) {
            return None;
        }
        self.heights
            .iter()
            .copied()
            .filter(|h| *h <= z)
            .max_by(|a, b| a.total_cmp(b))
    }

    /// The liquid a WMO holds at a point: its entry (1 water, 2 ocean, 3
    /// magma, 4 slime, 21 Naxxramas slime), its surface and the floor under
    /// it.
    pub fn wmo_liquid(&mut self, p: Vector3d) -> Option<(u8, f32, f32)> {
        if !self.ready(p.x, p.y) {
            return None;
        }
        let s = self.map.wmo_liquid(v(p)).ok()??;
        Some((s.entry, s.surface, s.floor))
    }

    /// The navmesh's liquid surface nearest the point, of whatever kind.
    pub fn liquid_surface(&mut self, p: Vector3d) -> Option<f32> {
        if !self.ready(p.x, p.y) {
            return None;
        }
        self.map.liquid_surface(p.x, p.y, p.z).ok().flatten()
    }

    /// Whether `to` can be seen from `from`; clear while a tile of its ends
    /// is loading.
    pub fn line_of_sight(&mut self, from: Vector3d, to: Vector3d) -> bool {
        if !self.ready_at(&[from, to]) {
            return true;
        }
        self.sight(from, to)
    }

    /// Line of sight over the tiles already loaded (where units are), for
    /// a reader that can't load more.
    pub fn sight(&self, from: Vector3d, to: Vector3d) -> bool {
        self.map
            .line_of_sight(v(from), v(to), false)
            .unwrap_or(true)
    }

    /// Whether a point is outdoors, from the floor below it; `None` where
    /// the bake can't say.
    pub fn outdoors(&mut self, p: Vector3d) -> Option<bool> {
        if !self.ready(p.x, p.y) {
            return None;
        }
        self.map.is_outdoors(v(p)).ok().flatten()
    }

    /// Tiles asked for, loaded or not.
    pub fn tiles_tried(&self) -> usize {
        self.tiles.len()
    }

    /// Whether tile `t` (`tile_of`) is in.
    pub fn has_tile(&self, t: (i32, i32)) -> bool {
        self.tiles.get(&t).is_some_and(|t| t.state == State::In)
    }

    /// Tiles in now.
    pub fn tiles_in(&self) -> usize {
        self.tiles.values().filter(|t| t.state == State::In).count()
    }

    /// Tiles being read.
    pub fn tiles_loading(&self) -> usize {
        self.tiles
            .values()
            .filter(|t| t.state == State::Loading)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// rev5-data 8: a bake whose header lists tiles that aren't there is
    /// counted, for the warning at its open.
    #[test]
    #[ignore = "extended"]
    fn the_tiles_a_bake_lacks_are_counted() {
        let real = crate::game::harness::real_data("nav");
        let dir = std::env::temp_dir().join(format!("wvs-nav-part-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let name = "AhnQirajTemple";
        for f in [format!("{name}.map"), format!("{name}.build.json")] {
            let _ = std::fs::copy(real.join(&f), dir.join(&f));
        }
        let whole = PathfindMap::new(&real, name).unwrap();
        let (listed, missing) = missing_tiles(&whole, &real, name);
        assert!(listed > 2, "{listed}");
        assert_eq!(missing, 0, "the whole bake");
        // Two tiles of it.
        for (x, y) in [(26, 46), (26, 47)] {
            let f = format!("{name}_{x}_{y}.nav");
            std::fs::copy(real.join(&f), dir.join(&f)).unwrap();
        }
        let part = PathfindMap::new(&dir, name).unwrap();
        assert_eq!(missing_tiles(&part, &dir, name), (listed, listed - 2));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
