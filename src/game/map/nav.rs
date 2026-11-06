//! A map's navmesh, from a rustigator bake: paths, ground heights, line of
//! sight, and the zone and area a point is in.
//!
//! Its ADT tiles load as they are first needed, and go again once no one
//! has been near one for a while (`[world] tile_unload_secs`). On the
//! server they are read off the map's thread (`map/tiles.rs`): until a
//! tile is in, a query over it answers as a map with no navmesh would (a
//! path is straight, a line of sight clear, a height unknown), so the map
//! never waits on the disk. A tile is made ready there too (rustigator's
//! `PreparedCell`: its navmesh linked within, its collision built), so the
//! map only links it to its neighbours, and one let go is freed there.
//! Tests load them at once, as they are asked for.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ahash::AHashMap;
use glam::Vec3;
use rustigator_pathfind::format::NavFile;
use rustigator_pathfind::{GroundOptions, PathfindMap, PreparedCell, QueryFilter, UnloadedCell};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use wow_world_messages::vanilla::Vector3d;

/// An ADT tile's side, in yards.
pub const ADT_SIZE: f32 = 533.333_3;
/// How far under a floor its area is asked for.
const AREA_FLOOR_TOLERANCE: f32 = 0.1;
/// How long an update spends taking in tiles read (at least one): many may
/// arrive at once (a player logging in, a populated map). Prepared on the
/// readers, a city's tile takes about 0.4 ms to join the rest (at most 1),
/// an ordinary one under 0.1, so this is several cities' tiles; before
/// round two of rustigator's performance it was one, at 3.6 ms.
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
/// rustigator's `put_on_ground` does the work, with these
/// ([`ground_options`]).
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
/// How far across a point its navmesh polygon is looked for.
const GROUND_ACROSS: f32 = 0.25;
/// The most points a path is given so.
const GROUND_POINTS: usize = 64;

/// How a path is put on the ground: the constants above, each half of a
/// line looked at again after a point is put in (`sample_once` off: on, a
/// winding chase's line is off the ground at up to a tenth of its looks).
fn ground_options() -> GroundOptions {
    GroundOptions {
        tolerance: GROUND_TOLERANCE,
        sample: GROUND_SAMPLE,
        reach: GROUND_REACH,
        across: GROUND_ACROSS,
        snap: SURFACE_SNAP,
        max_points: GROUND_POINTS,
        filter: QueryFilter::any(),
        sample_once: false,
    }
}
/// How often along a way its height is put to the terrain's
/// ([`Nav::into_terrain`]): the terrain's triangles are 2 to 4 yards across.
const TERRAIN_SAMPLE: f32 = 0.5;
/// How far under the terrain's surface a point is still on it: a floor's
/// own height, to the float.
const TERRAIN_SLACK: f32 = 0.05;

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
    /// A path as the navmesh finds it, and on the ground: kept, so a path
    /// allocates only its answer.
    found: Vec<Vec3>,
    grounded: Vec<Vec3>,
    /// The collision models of the map's game objects, by display
    /// (`obstacles`), read at its start.
    pub obstacle_models: AHashMap<u32, std::sync::Arc<crate::game::map::obstacles::Model>>,
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

/// A tile read and made ready: its indices, and the cell (`None` if it
/// wasn't to be had).
type Arrival = ((i32, i32), Option<PreparedCell>);

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

/// How far over or under a goal a path's end may be and still reach it:
/// a step's climb.
const REACH_RISE: f32 = 2.5;

/// Whether a path's end at `end` reaches `goal`: within `yards` of it over
/// the ground, and a step's climb of its height. A path's end is on the
/// floor, and the goal may be at the navmesh's height, a yard and more
/// over it, or on a ledge beside it: Orgrimmar's square is a platform 1.45
/// yd over the floor where the navmesh's way to it ends, 2.7 yd off, so a
/// path to it ended 3.05 yd from it, Rexxar's start found none (decision
/// 107).
pub fn reaches(end: Vector3d, goal: Vector3d, yards: f32) -> bool {
    (end.x - goal.x).hypot(end.y - goal.y) <= yards && (end.z - goal.z).abs() <= REACH_RISE
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
                    found: Vec::new(),
                    grounded: Vec::new(),
                    obstacle_models: AHashMap::new(),
                })
            }
            Err(e) => {
                warn!("no navmesh for {name} in {}: {e}", dir.display());
                None
            }
        }
    }

    /// The bake's directory.
    pub fn dir(&self) -> &Path {
        &self.dir
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
                // Its collision is built here too: a city's takes up to 70
                // ms, which the map's thread paid at the first query
                // over it.
                crate::game::map::tiles::read_into(tx, t, None, move || {
                    read(&dir, &name, t).map(PreparedCell::new)
                });
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
                // At once, its collision is built as it is first needed,
                // as rustigator's `load_nav_file` leaves it.
                let cell = read(&self.dir, &self.name, t).map(PreparedCell::without_collision);
                self.take(t, cell);
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

    fn take(&mut self, t: (i32, i32), cell: Option<PreparedCell>) {
        let _z = crate::zone!("navmesh take");
        let state = match cell.map(|c| self.map.load_prepared(c)) {
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
    /// how many, and whether idle ones are left. Taking a tile out of the
    /// navmesh is about 0.02 ms; what it held is freed on a reader
    /// ([`Nav::free`]), where a busy one's thousands of allocations (and
    /// the disk's time, where its idle pages were swapped out) cost the map
    /// nothing.
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
            let held = {
                let _z = crate::zone!("navmesh unload");
                self.map.take_adt(x, y)
            };
            if let Some(held) = held {
                self.free(held);
            }
            self.tiles.remove(&(x, y));
            n += 1;
            if started.elapsed() >= budget {
                break;
            }
        }
        (n, !idle.is_empty())
    }

    /// What a tile let go held, freed: on a reader, where tiles are read
    /// off the map's thread; else here.
    fn free(&self, held: UnloadedCell) {
        if self.background.is_some() {
            crate::game::map::tiles::read(move || drop(held));
        } else {
            drop(held);
        }
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
        // Nearest first; of two as near, the higher (the sort is stable).
        self.heights
            .sort_by(|a, b| (a - p.z).abs().total_cmp(&(b - p.z).abs()));
        let map = &self.map;
        self.heights.iter().find_map(|&h| {
            known(
                map.get_zone_and_area(p.x, p.y, h - AREA_FLOOR_TOLERANCE)
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
        let mut found = std::mem::take(&mut self.found);
        found.clear();
        let ok = self
            .map
            .find_path(v(from), v(to), false, &mut found)
            .is_ok()
            && found.len() >= 2;
        let path = ok.then(|| {
            found[0] = v(from);
            self.grounded(&found)
        });
        self.found = found;
        path
    }

    /// [`Nav::path`] as the navmesh answers it, not put on the ground: for
    /// the probe of what that costs.
    #[cfg(test)]
    pub fn path_as_found(&mut self, from: Vector3d, to: Vector3d) -> Option<Vec<Vector3d>> {
        let mut points = Vec::new();
        self.map
            .find_path(v(from), v(to), false, &mut points)
            .ok()?;
        (points.len() >= 2).then(|| points.into_iter().map(d).collect())
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
        self.grounded(&[v(from), v(to)])
    }

    /// A path's points on the ground (rustigator's `put_on_ground`, with
    /// [`ground_options`]): the first as it is (where the walker is); each
    /// other at the floor under it if it is farther from it than
    /// [`GROUND_TOLERANCE`] (one on the ground is left alone), one where the
    /// last is left out (a walk's next point is somewhere to go); and, where
    /// the line between two leaves the ground by more than
    /// [`GROUND_TOLERANCE`], a point on the ground put in where it leaves it
    /// most, until none does.
    ///
    /// The floor under a point is the navmesh's polygon there, within
    /// [`GROUND_REACH`] of it (a point on a polygon's edge, as a path's
    /// corner always is, takes the nearest point of the nearest polygon's
    /// edge, as Detour's `closestPointOnPoly` has it: before, a corner kept
    /// the polygon's own height, up to 1.2 yd over the valley's slopes and
    /// round its rocks, decision 107), and the surface of the bake's
    /// geometry (its terrain's, WMOs' and doodads' upward faces) nearest
    /// that polygon's height within [`SURFACE_SNAP`]; else the polygon's
    /// height. Off the navmesh (a straight walk where it has no way), the
    /// surface nearest the point itself within [`SURFACE_SNAP`].
    fn grounded(&mut self, path: &[Vec3]) -> Vec<Vector3d> {
        self.map
            .put_on_ground(path, &ground_options(), &mut self.grounded);
        self.grounded.iter().copied().map(d).collect()
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

    /// The ground nearest `z_hint` at `(x, y)` (of two as near, the
    /// higher), of the [`Nav::surfaces`] there: looked for within a few
    /// yards of the hint first, so a column's other floors aren't walked
    /// (rustigator's `nearest_height`).
    pub fn ground(&mut self, x: f32, y: f32, z_hint: f32) -> Option<f32> {
        if !self.ready(x, y) {
            return None;
        }
        self.map.nearest_height(x, y, z_hint).ok().flatten()
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

    /// Every surface at `(x, y)`, top to bottom: the terrain's and the
    /// upward faces of the bake's WMOs and doodads (rustigator's
    /// `find_heights`, which leaves ceilings out). `None` while its tile is
    /// loading, or where there is none.
    pub fn surfaces(&mut self, x: f32, y: f32) -> Option<&[f32]> {
        self.heights_at(x, y).then_some(&self.heights[..])
    }

    /// Whether the bake's WMOs or doodads (M2s: trees, rocks, fences)
    /// stand across the segment from `from` to `to`, met from either side;
    /// true while a tile under it is loading. rustigator's line of sight
    /// takes a triangle that faces the ray alone (namigator's), so a wall
    /// seen from behind, or a doodad's inside, hides nothing: a ray each way
    /// meets every triangle the segment crosses, as mangos-classic's VMaps
    /// do (`IntersectTriangle` takes either face). Terrain is no part of it.
    pub fn crossed(&mut self, from: Vector3d, to: Vector3d) -> bool {
        self.crossed_by(from, to, true)
    }

    /// [`Nav::crossed`] by the WMOs alone (`doodads` false), or by them and
    /// the doodads.
    pub fn crossed_by(&mut self, from: Vector3d, to: Vector3d, doodads: bool) -> bool {
        let corners = [
            from,
            to,
            Vector3d {
                x: from.x,
                y: to.y,
                z: from.z,
            },
            Vector3d {
                x: to.x,
                y: from.y,
                z: from.z,
            },
        ];
        if !self.ready_at(&corners) {
            return true;
        }
        // One walk of the models, a triangle of either facing in the way.
        !self
            .map
            .line_of_sight_two_sided(v(from), v(to), doodads)
            .unwrap_or(true)
    }

    /// The terrain's height at `(x, y)`, its heightmap's alone (the
    /// navmesh's heights mix the terrain's with the models' surfaces, and a
    /// way that goes into the ground is found on the terrain's alone,
    /// [`Nav::into_terrain`]): `None` over a hole in it (a cave's mouth, a
    /// building's cellar), where there is none, or while its tile is
    /// loading.
    pub fn terrain_height(&mut self, x: f32, y: f32) -> Option<f32> {
        if !x.is_finite() || !y.is_finite() || !self.ready(x, y) {
            return None;
        }
        self.map.terrain_height(x, y)
    }

    /// Whether the segment from `from` to `to` goes into the terrain or out
    /// of it (a hill's face, a slope it cuts under), looked at every
    /// [`TERRAIN_SAMPLE`]: from one side of the terrain's surface to the
    /// other between two looks, both over the terrain and not a hole. A way
    /// under the terrain all along (a cellar, a cave) crosses nothing, and
    /// one through a hole (a cave's mouth) neither. True while a tile under
    /// it is loading.
    pub fn into_terrain(&mut self, from: Vector3d, to: Vector3d) -> bool {
        if !self.ready_at(&[from, to]) {
            return true;
        }
        let planar = (to.x - from.x).hypot(to.y - from.y);
        let n = ((planar / TERRAIN_SAMPLE).ceil() as usize).max(1);
        let mut last: Option<bool> = None;
        for i in 0..=n {
            let f = i as f32 / n as f32;
            let p = Vector3d {
                x: from.x + (to.x - from.x) * f,
                y: from.y + (to.y - from.y) * f,
                z: from.z + (to.z - from.z) * f,
            };
            let under = self
                .terrain_height(p.x, p.y)
                .map(|h| p.z < h - TERRAIN_SLACK);
            match (last, under) {
                (Some(a), Some(b)) if a != b => return true,
                _ => {}
            }
            last = under;
        }
        false
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

    /// The terrain's height is rustigator's heightmap's, where wvs kept a
    /// copy of each tile's terrain beside it (150 KB a tile): to the bit
    /// the copy's, at once and read off the map's thread, over Stormwind's,
    /// Elwynn's and Undercity's tiles and their holes.
    #[test]
    #[ignore = "extended"]
    fn the_terrain_height_is_the_tiles_own() {
        let real = crate::game::harness::real_data("nav");
        let tiles = [(30, 48), (31, 48), (32, 49), (31, 29)];
        let mut at_once = Nav::open(&real, "Azeroth").unwrap();
        let mut off = Nav::open(&real, "Azeroth").unwrap();
        off.in_background();
        for &t in &tiles {
            off.want(t);
        }
        let deadline = Instant::now() + Duration::from_secs(60);
        while off.tiles_in() < tiles.len() {
            assert!(Instant::now() < deadline, "the tiles within 60 s");
            std::thread::sleep(Duration::from_millis(10));
            off.receive(Instant::now());
        }
        const N: usize = 300;
        let (mut heights, mut holes) = (0, 0);
        for &t in &tiles {
            let file = read(&real, "Azeroth", t).expect("the tile");
            let ylo = (31 - t.0) as f32 * ADT_SIZE;
            let xlo = (31 - t.1) as f32 * ADT_SIZE;
            for i in 0..N {
                for j in 0..N {
                    let x = xlo + (i as f32 + 0.5) * ADT_SIZE / N as f32;
                    let y = ylo + (j as f32 + 0.5) * ADT_SIZE / N as f32;
                    let copy = file.terrain.height_at(t.0, t.1, x, y).map(f32::to_bits);
                    assert_eq!(
                        at_once.terrain_height(x, y).map(f32::to_bits),
                        copy,
                        "{x} {y}"
                    );
                    assert_eq!(off.terrain_height(x, y).map(f32::to_bits), copy, "{x} {y}");
                    if copy.is_some() {
                        heights += 1;
                    } else {
                        holes += 1;
                    }
                }
            }
        }
        assert!(
            heights > 300_000 && holes > 100,
            "{heights} heights, {holes} holes"
        );
    }

    /// What a tile costs the map's thread, taking it in and letting it go,
    /// and the reader preparing it: each of 11 busy tiles (the cities,
    /// rustigator's sample) with its eight neighbours in, and the memory of
    /// the 99, their collision built. Run by hand (the extended suite's
    /// filter leaves the probes out):
    /// `cargo nextest run --lib --profile extended --run-ignored all
    /// --ignore-default-filter probe_a_tiles_cost --no-capture`.
    #[test]
    #[ignore = "a probe, run by hand"]
    fn probe_a_tiles_cost() {
        let real = crate::game::harness::real_data("nav");
        let rss = || {
            let s = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
            let pages: f64 = s
                .split_whitespace()
                .nth(1)
                .and_then(|p| p.parse().ok())
                .unwrap_or(0.0);
            pages * 4096.0 / 1e6
        };
        let busy: [(&str, (i32, i32)); 11] = [
            ("Azeroth", (30, 48)),
            ("Azeroth", (33, 41)),
            ("Azeroth", (31, 29)),
            ("Azeroth", (31, 28)),
            ("Kalimdor", (40, 28)),
            ("Kalimdor", (40, 29)),
            ("Kalimdor", (31, 34)),
            ("Kalimdor", (27, 13)),
            ("Azeroth", (35, 52)),
            ("Azeroth", (34, 44)),
            ("Kalimdor", (28, 47)),
        ];
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        let (mut take, mut take_near, mut prepare) = (Vec::new(), Vec::new(), Vec::new());
        let (mut unload, mut free) = (Vec::new(), Vec::new());
        let before = rss();
        let mut navs = Vec::new();
        for (name, (cx, cy)) in busy {
            let mut nav = Nav::open(&real, name).expect("the bake");
            for x in cx - 1..=cx + 1 {
                for y in cy - 1..=cy + 1 {
                    if (x, y) != (cx, cy) && nav.map.has_adt(x, y) {
                        let cell = read(&nav.dir, &nav.name, (x, y)).map(PreparedCell::new);
                        let t = Instant::now();
                        nav.take((x, y), cell);
                        take_near.push(ms(t.elapsed()));
                    }
                }
            }
            let t = Instant::now();
            let cell = read(&nav.dir, &nav.name, (cx, cy)).map(PreparedCell::new);
            prepare.push(ms(t.elapsed()));
            let t = Instant::now();
            nav.take((cx, cy), cell);
            take.push(ms(t.elapsed()));
            navs.push(nav);
        }
        let held = rss() - before;
        for ((_, (cx, cy)), nav) in busy.iter().zip(&mut navs) {
            let t = Instant::now();
            let cell = nav.map.take_adt(*cx, *cy);
            unload.push(ms(t.elapsed()));
            let t = Instant::now();
            drop(cell);
            free.push(ms(t.elapsed()));
        }
        let q = |v: &mut Vec<f64>| {
            v.sort_by(f64::total_cmp);
            format!(
                "p50 {:.3} max {:.3} ms (of {})",
                v[v.len() / 2],
                v[v.len() - 1],
                v.len()
            )
        };
        println!("taken in, a busy tile: {}", q(&mut take));
        println!("taken in, its neighbours: {}", q(&mut take_near));
        println!("let go, a busy tile: {}", q(&mut unload));
        println!(
            "read and prepared on a reader, a busy tile: {}",
            q(&mut prepare)
        );
        println!("freed on a reader, a busy tile: {}", q(&mut free));
        println!("the 99 tiles in, their collision built: {held:.0} MB");
    }
}
