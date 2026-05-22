use ahash::{AHashMap, AHashSet};
use rustigator::raw::{build_bvh, build_map, bvh_files_exist, map_files_exist};
use rustigator::vanilla::{Map, VanillaMap};
use tracing::{info, warn};

/// Maps we build navmeshes for when `WOW_VANILLA_USE_MAPS` is set.
/// EasternKingdoms + Kalimdor cover all the worlddb spawns; DevelopmentLand
/// is retained for compatibility with the original test workflow.
const BUILD_MAPS: &[Map] = &[
    Map::DevelopmentLand,
    Map::EasternKingdoms,
    Map::Kalimdor,
];

#[derive(Debug)]
pub struct PathfindingMaps {
    maps: AHashMap<Map, VanillaMap>,
    /// Tracks which `(map, tile_x, tile_y)` ADTs we've already attempted to
    /// load so we don't retry-on-miss on every height query.
    attempted_adts: AHashSet<(Map, i32, i32)>,
}

impl Default for PathfindingMaps {
    fn default() -> Self {
        Self::new()
    }
}

impl PathfindingMaps {
    pub fn new() -> Self {
        let maps = if let Some(data_path) = std::option_env!("WOW_VANILLA_USE_MAPS") {
            let output = std::env::temp_dir().join("wow_vanilla_server");
            info!(
                "Building maps for pathfind from '{data_path}' into '{}'. This may take a while.",
                output
                    .to_str()
                    .expect("temp dir path should be valid UTF-8 on supported platforms"),
            );
            let threads = {
                let t = std::thread::available_parallelism()
                    .expect("available_parallelism failed; cannot pick thread count")
                    .get() as u32;
                let t = t.saturating_sub(2);
                if t == 0 { 1 } else { t }
            };

            if !bvh_files_exist(&output)
                .expect("failed to probe BVH output directory; check filesystem permissions")
            {
                info!("Building gameobjects.");
                build_bvh(data_path, &output, threads)
                    .expect("build_bvh failed; check WOW_VANILLA_USE_MAPS path and client data");
                info!("Gameobjects built.");
            } else {
                info!("Gameobjects already built.");
            }

            let mut m = AHashMap::new();
            for &map in BUILD_MAPS {
                match build_one(map, data_path, &output, threads) {
                    Ok(v) => {
                        m.insert(map, v);
                    }
                    Err(e) => {
                        warn!("Failed to set up map {map}: {e}; terrain queries will return None for this map.");
                    }
                }
            }
            info!("Finished setting up maps");
            m
        } else {
            info!("Not using maps for pathfind.");
            AHashMap::new()
        };

        Self {
            maps,
            attempted_adts: AHashSet::new(),
        }
    }

    pub fn get(&self, map: &Map) -> Option<&VanillaMap> {
        self.maps.get(map)
    }

    /// Number of `(map, tile_x, tile_y)` ADT tiles we've attempted to load
    /// since startup. Each loaded ADT keeps its mesh data resident, so this
    /// is a useful proxy for "how much of the pathfinding cache has warmed
    /// up" — published as a Tracy plot for memory-growth diagnosis.
    pub fn attempted_adt_count(&self) -> usize {
        self.attempted_adts.len()
    }

    /// Snap a world XY to the nearest ground Z. Lazily loads the ADT that
    /// contains the point. Returns None if maps aren't configured for this
    /// continent or the lookup fails (e.g. outside the navmesh).
    ///
    /// Uses `find_heights` (plural) and picks the candidate closest
    /// to `z_hint`. The singular `find_height` looks like a downward
    /// raycast but is actually a `findNearestPoly` with hardcoded
    /// 1-yd extents around the input position — passing
    /// `z = z_hint + 50` to "ray from above" fails because the search
    /// cube is 50 yd above the ground. `find_heights` returns every
    /// navmesh-Z value in the column regardless of start altitude,
    /// and picking closest-to-hint locks onto the floor in
    /// multi-level structures.
    pub fn ground_height(&mut self, map: Map, x: f32, y: f32, z_hint: f32) -> Option<f32> {
        let vmap = self.maps.get_mut(&map)?;
        let (tx, ty) = world_to_adt(x, y);
        if !self.attempted_adts.contains(&(map, tx, ty)) {
            self.attempted_adts.insert((map, tx, ty));
            if let Err(e) = vmap.load_adt(tx, ty) {
                warn!("load_adt({map}, {tx}, {ty}) failed: {e}");
                return None;
            }
        }
        let heights = vmap.find_heights(x, y).ok()?;
        if heights.is_empty() {
            return None;
        }
        heights
            .iter()
            .copied()
            .min_by(|a, b| {
                (a - z_hint)
                    .abs()
                    .partial_cmp(&(b - z_hint).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }
}

fn build_one(
    map: Map,
    data_path: &str,
    output: &std::path::Path,
    threads: u32,
) -> Result<VanillaMap, rustigator::RustigatorError> {
    if !map_files_exist(output, map.directory_name())? {
        info!("Building map {map} ({})", map.directory_name());
        build_map(data_path, output, map.directory_name(), "", threads)?;
        info!("Finished building {map} ({})", map.directory_name());
    } else {
        info!("{map} ({}) already built.", map.directory_name());
    }
    VanillaMap::build_gameobjects_and_map(data_path, output, map, threads)
}

/// Vanilla WoW divides the world into 64×64 ADT tiles. Each tile is 533.33333
/// yards on a side, with the origin at the center of the world.
fn world_to_adt(x: f32, y: f32) -> (i32, i32) {
    const ADT_SIZE: f32 = 533.333_3;
    const HALF_GRID: f32 = 32.0;
    // Mangos convention: tile_x derived from world.y, tile_y from world.x.
    let tx = (HALF_GRID - y / ADT_SIZE).floor() as i32;
    let ty = (HALF_GRID - x / ADT_SIZE).floor() as i32;
    (tx.clamp(0, 63), ty.clamp(0, 63))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_origin_maps_to_center_tile() {
        // World origin (0,0) sits at the boundary between tiles 31 and 32 on
        // both axes; floor() picks tile 32.
        assert_eq!(world_to_adt(0.0, 0.0), (32, 32));
    }

    #[test]
    fn world_to_adt_clamps_extreme_positive() {
        // Far north-west of the world map — should clamp to (0, 0).
        let (tx, ty) = world_to_adt(50_000.0, 50_000.0);
        assert_eq!((tx, ty), (0, 0));
    }

    #[test]
    fn world_to_adt_clamps_extreme_negative() {
        // Far south-east — should clamp to (63, 63).
        let (tx, ty) = world_to_adt(-50_000.0, -50_000.0);
        assert_eq!((tx, ty), (63, 63));
    }

    #[test]
    fn stormwind_gate_lives_on_known_tiles() {
        // The Stormwind south gate is approx (-9083, 419) on Map 0.
        // It should resolve to a valid in-bounds ADT tile.
        let (tx, ty) = world_to_adt(-9083.0, 419.0);
        assert!((0..=63).contains(&tx), "tx out of range: {tx}");
        assert!((0..=63).contains(&ty), "ty out of range: {ty}");
    }
}
