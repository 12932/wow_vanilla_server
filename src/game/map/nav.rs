//! A map's navmesh, from a rustigator bake: paths, ground heights, line of
//! sight, and the zone and area a point is in. Tiles load as they are
//! first needed.

use std::path::Path;

use ahash::AHashSet;
use rustigator::raw::PathfindMap;
use tracing::{info, warn};
use wow_world_messages::vanilla::Vector3d;

/// An ADT tile's side, in yards.
const ADT_SIZE: f32 = 533.333_3;
/// How far under a floor its area is asked for.
const AREA_FLOOR_TOLERANCE: f32 = 0.1;

pub struct Nav {
    map: PathfindMap,
    /// Tiles tried, loaded or not, so a missing one isn't retried.
    tried: AHashSet<(i32, i32)>,
}

impl std::fmt::Debug for Nav {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Nav({} tiles tried)", self.tried.len())
    }
}

fn tile_of(x: f32, y: f32) -> (i32, i32) {
    let tx = (32.0 - y / ADT_SIZE).floor() as i32;
    let ty = (32.0 - x / ADT_SIZE).floor() as i32;
    (tx.clamp(0, 63), ty.clamp(0, 63))
}

impl Nav {
    /// The navmesh for the map called `name` in the bake at `dir`.
    pub fn open(dir: &Path, name: &str) -> Option<Self> {
        match PathfindMap::new(dir, name) {
            Ok(map) => {
                info!("navmesh for {name} from {}", dir.display());
                Some(Self {
                    map,
                    tried: AHashSet::new(),
                })
            }
            Err(e) => {
                warn!("no navmesh for {name} in {}: {e}", dir.display());
                None
            }
        }
    }

    fn ensure(&mut self, x: f32, y: f32) {
        if !x.is_finite() || !y.is_finite() {
            return;
        }
        let tile = tile_of(x, y);
        if self.tried.insert(tile) {
            let _ = self.map.ensure_adt_loaded_at(x, y);
        }
    }

    /// The zone and area at a point; where the point says nothing, those
    /// of the floor nearest it, as Thistle Tea looks.
    pub fn zone_and_area(&mut self, p: Vector3d) -> Option<(u32, u32)> {
        self.ensure(p.x, p.y);
        let known =
            |ids: Option<(u32, u32)>| ids.filter(|&(_, area)| area != 0 && area != u32::MAX);
        if let Some(ids) = known(self.map.get_zone_and_area(p.x, p.y, p.z).ok()) {
            return Some(ids);
        }
        let mut floors = self.map.find_heights(p.x, p.y).ok()?.to_vec();
        floors.sort_by(|a, b| (a - p.z).abs().total_cmp(&(b - p.z).abs()));
        floors.into_iter().find_map(|h| {
            known(
                self.map
                    .get_zone_and_area(p.x, p.y, h - AREA_FLOOR_TOLERANCE)
                    .ok(),
            )
        })
    }

    /// A walkable path from `from` to `to`, its first point `from`.
    pub fn path(&mut self, from: Vector3d, to: Vector3d) -> Option<Vec<Vector3d>> {
        self.ensure(from.x, from.y);
        self.ensure(to.x, to.y);
        let path = self.map.find_path(from, to).ok()?;
        if path.len() < 2 {
            return None;
        }
        Some(path.to_vec())
    }

    /// A random walkable point within `radius` of `center`.
    pub fn random_point(&mut self, center: Vector3d, radius: f32) -> Option<Vector3d> {
        self.ensure(center.x, center.y);
        self.map
            .find_random_point_around_circle(center, radius)
            .ok()
    }

    /// Where a walk over the navmesh from `from` toward `to` stops
    /// (Thistle Tea's `walk_hit_position`); `None` off the navmesh.
    pub fn walk_hit(&mut self, from: Vector3d, to: Vector3d) -> Option<Vector3d> {
        self.ensure(from.x, from.y);
        self.ensure(to.x, to.y);
        self.map.walk_hit_position(from, to).ok().flatten()
    }

    /// The ground nearest `z_hint` at `(x, y)`.
    pub fn ground(&mut self, x: f32, y: f32, z_hint: f32) -> Option<f32> {
        self.ensure(x, y);
        let heights = self.map.find_heights(x, y).ok()?;
        heights
            .iter()
            .copied()
            .min_by(|a, b| (a - z_hint).abs().total_cmp(&(b - z_hint).abs()))
    }

    /// The liquid a WMO holds at a point: its entry (1 water, 2 ocean, 3
    /// magma, 4 slime, 21 Naxxramas slime), its surface and the floor under
    /// it.
    pub fn wmo_liquid(&mut self, p: Vector3d) -> Option<(u8, f32, f32)> {
        self.ensure(p.x, p.y);
        let s = self.map.wmo_liquid(p.x, p.y, p.z).ok()??;
        Some((s.entry, s.surface, s.floor))
    }

    /// The navmesh's liquid surface nearest the point, of whatever kind.
    pub fn liquid_surface(&mut self, p: Vector3d) -> Option<f32> {
        self.ensure(p.x, p.y);
        self.map.liquid_surface(p.x, p.y, p.z).ok().flatten()
    }

    pub fn line_of_sight(&mut self, from: Vector3d, to: Vector3d) -> bool {
        self.ensure(from.x, from.y);
        self.ensure(to.x, to.y);
        self.map.line_of_sight(from, to).unwrap_or(true)
    }

    /// Line of sight over the tiles already loaded (where units are), for
    /// a reader that can't load more.
    pub fn sight(&self, from: Vector3d, to: Vector3d) -> bool {
        self.map.line_of_sight(from, to).unwrap_or(true)
    }

    /// Whether a point is outdoors, from the floor below it; `None` where
    /// the bake can't say.
    pub fn outdoors(&mut self, p: Vector3d) -> Option<bool> {
        self.ensure(p.x, p.y);
        self.map.is_outdoors(p.x, p.y, p.z).ok().flatten()
    }

    pub fn tiles_tried(&self) -> usize {
        self.tried.len()
    }
}
