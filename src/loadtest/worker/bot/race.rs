//! Amazing Race helpers: shared waypoint list + per-bot jitter sampling.
//!
//! Two paths to a BB → SW waypoint list:
//! - `build_race_path()` — calls namigator's `find_path` against the
//!   pre-baked nav-mesh cache. Returns a 30+ waypoint polyline that
//!   actually hugs walkable terrain (around buildings, over bridges).
//!   Requires `WOW_VANILLA_USE_MAPS` to have been set at compile time
//!   AND the cache to be present on disk.
//! - `hardcoded_bb_to_sw()` — 5-point fallback. Used when namigator
//!   isn't available (env var unset at build time, cache missing,
//!   find_path errors). Bots still walk the route, but the geometry
//!   is coarse and clips through anything between the waypoints.

use wow_world_messages::vanilla::Vector3d;

/// Hardcoded Booty Bay → Stormwind waypoint list. Coordinates picked by
/// eye from the standard mangos worldmap; close enough to walkable
/// terrain that a bot ignoring geometry won't look obviously wrong from
/// a distance. Will be replaced with `find_path()` output once the
/// namigator cache is built.
pub fn hardcoded_bb_to_sw() -> Vec<Vector3d> {
    vec![
        // Booty Bay docks
        Vector3d { x: -14253.7, y:  290.5, z:   7.4 },
        // Grom'gol Base Camp (entrance)
        Vector3d { x: -12382.0, y: -127.0, z:  46.8 },
        // Duskwood Darkshire crossroads
        Vector3d { x: -10510.0, y: -1278.0, z:  37.8 },
        // Goldshire
        Vector3d { x:  -9461.0, y:    62.0, z:  56.1 },
        // Stormwind Trade District fountain
        Vector3d { x:  -8949.0, y:  -132.0, z:  84.0 },
    ]
}

/// Booty Bay docks — race start. Constant so both `build_race_path()`
/// and `hardcoded_bb_to_sw()` agree on the endpoints. Coordinates
/// hand-picked from in-game (`.whereami` on the docks) so they sit
/// cleanly on the walkable plank surface rather than the namigator
/// mesh under it (which is rocky seabed several yards below).
pub const BOOTY_BAY: Vector3d = Vector3d { x: -14237.9, y: 262.02, z: 24.75 };
/// Stormwind Trade District — race finish.
pub const STORMWIND: Vector3d = Vector3d { x: -8949.0, y: -132.0, z: 84.0 };

/// Build the BB → SW path from the pre-baked navmesh cache. Reads the
/// same env var + cache directory the server uses
/// (`pathfinding_maps.rs:33`), so once the server has finished its
/// first-boot bake the loadtest sees the cached output instantly.
///
/// Returns `Err` if:
/// - `WOW_VANILLA_USE_MAPS` was unset at compile time (no namigator)
/// - The nav-mesh cache directory doesn't exist or is missing files
/// - `find_path` returns `UnknownPath` (the corridor isn't connected
///   on the mesh — e.g. a bridge that's M2-only and got skipped by
///   the bake)
///
/// Caller's responsibility to fall back to `hardcoded_bb_to_sw()` on
/// error and log the failure for the operator.
pub fn build_race_path() -> Result<Vec<Vector3d>, String> {
    const DATA_PATH: Option<&str> = std::option_env!("WOW_VANILLA_USE_MAPS");
    let data_path = DATA_PATH
        .ok_or_else(|| "WOW_VANILLA_USE_MAPS was unset at compile time".to_string())?;

    // Match the server's cache directory convention so the loadtest
    // and server share one bake artifact.
    let output = std::env::temp_dir().join("wow_vanilla_server");
    if !output.exists() {
        return Err(format!(
            "navmesh cache directory {} does not exist; start the server first to bake it",
            output.display(),
        ));
    }

    // Threads = available_parallelism - 2, same as the server. Doesn't
    // matter much here — if the cache is already baked, `build_*`
    // short-circuit on the `*_files_exist` checks.
    let threads = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(2)
        .saturating_sub(2)
        .max(1);

    let started = std::time::Instant::now();
    let mut map = namigator::vanilla::VanillaMap::build_gameobjects_and_map(
        data_path,
        &output,
        namigator::vanilla::Map::EasternKingdoms,
        threads,
    )
    .map_err(|e| format!("VanillaMap::build_gameobjects_and_map failed: {e:?}"))?;

    // Pre-load every ADT tile in the BB→SW bounding box. `find_path`
    // refuses to cross unloaded tiles (`FailedToLoadAdt`), and lazy-
    // loading from inside the path search isn't an option. The
    // bounding box is ~10–12 tiles tall × 1–2 wide on Eastern
    // Kingdoms, so ~20 loads up front.
    let (bb_tx, bb_ty) = world_to_adt(BOOTY_BAY.x, BOOTY_BAY.y);
    let (sw_tx, sw_ty) = world_to_adt(STORMWIND.x, STORMWIND.y);
    let (tx_min, tx_max) = (bb_tx.min(sw_tx), bb_tx.max(sw_tx));
    let (ty_min, ty_max) = (bb_ty.min(sw_ty), bb_ty.max(sw_ty));
    let mut loaded = 0;
    let mut failed = 0;
    for tx in tx_min..=tx_max {
        for ty in ty_min..=ty_max {
            // load_adt returns Err for tiles outside the mesh — okay,
            // a corridor that grazes empty terrain is fine.
            match map.load_adt(tx, ty) {
                Ok(_) => loaded += 1,
                Err(_) => failed += 1,
            }
        }
    }
    tracing::info!(
        "race path: loaded {loaded} ADT tiles ({failed} skipped) in {:.1}s",
        started.elapsed().as_secs_f32(),
    );

    let path: Vec<Vector3d> = map
        .find_path(BOOTY_BAY, STORMWIND)
        .map_err(|e| format!("find_path(BB, SW) failed: {e:?}"))?
        .to_vec();

    let dist: f32 = path
        .windows(2)
        .map(|w| {
            let dx = w[1].x - w[0].x;
            let dy = w[1].y - w[0].y;
            (dx * dx + dy * dy).sqrt()
        })
        .sum();
    tracing::info!(
        "race path: {} waypoints, {:.0} yd total, generated in {:.1}s",
        path.len(),
        dist,
        started.elapsed().as_secs_f32(),
    );

    Ok(path)
}

/// World XY → ADT tile (tx, ty). Mangos convention: tile_x derives
/// from world.y, tile_y from world.x. Mirrors `world_to_adt` in
/// `src/world/world/pathfinding_maps.rs`.
fn world_to_adt(x: f32, y: f32) -> (i32, i32) {
    const ADT_SIZE: f32 = 533.333_3;
    const HALF_GRID: f32 = 32.0;
    let tx = (HALF_GRID - y / ADT_SIZE).floor() as i32;
    let ty = (HALF_GRID - x / ADT_SIZE).floor() as i32;
    (tx.clamp(0, 63), ty.clamp(0, 63))
}

/// Per-bot lateral offset applied to every waypoint. Deterministic in
/// the slot index so re-runs are reproducible. Both axes sampled in
/// `[-50, 50] yd`; the bot will travel through the corridor offset by
/// a constant `(jx, jy)` for its entire lifetime. Keeps each bot
/// inside a tube around the base path rather than crowding everyone
/// onto a single waypoint string.
pub fn jitter_for_slot(slot: u32) -> (f32, f32) {
    // Cheap hash → two floats in [-50, 50]. Splitmix-style, no rand
    // dependency needed (rand is already pulled in by movement.rs but
    // keeping this deterministic-by-slot is more useful for debugging
    // than fresh randomness).
    let mut z = slot.wrapping_mul(0x9E37_79B9).wrapping_add(1);
    let mut next = || {
        z = z.wrapping_add(0x9E37_79B9);
        let mut h = z;
        h ^= h >> 16;
        h = h.wrapping_mul(0x85eb_ca6b);
        h ^= h >> 13;
        h = h.wrapping_mul(0xc2b2_ae35);
        h ^= h >> 16;
        // Map to [-1.0, 1.0).
        (h as f32 / u32::MAX as f32) * 2.0 - 1.0
    };
    (next() * 50.0, next() * 50.0)
}
