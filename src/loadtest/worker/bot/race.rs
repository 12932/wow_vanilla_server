//! Amazing Race helpers: shared waypoint list + per-bot jitter sampling.
//!
//! Two paths to a BB → Gurubashi waypoint list:
//! - `build_race_path()`: rustigator's path search over the bake in the
//!   server's cache directory. A polyline that follows walkable ground,
//!   around buildings and over bridges. Needs `WOW_VANILLA_USE_MAPS` at
//!   compile time, and the bake on disk.
//! - `hardcoded_bb_to_arena()`: the coarse fallback when no maps are
//!   configured. Bots still walk the route, but clip through anything
//!   between waypoints.
//!
//! The race ends at the entrance to Gurubashi Arena, about 1 km north of
//! Booty Bay along the coast. Bots turn back on arrival and run back,
//! looping forever.

use std::sync::Arc;
use wow_world_messages::vanilla::Vector3d;

/// Coarse Booty Bay → Gurubashi Arena waypoint list, with one midpoint on
/// the coast road. The fallback when no maps are configured.
pub fn hardcoded_bb_to_arena() -> Vec<Vector3d> {
    vec![
        BOOTY_BAY,
        // STV coastal road midpoint (north of BB, south of the arena)
        Vector3d { x: -13800.0, y: 200.0, z: 19.2 },
        GURUBASHI_ARENA,
    ]
}

/// Booty Bay's docks, the race start (from `.whereami` on the planks).
pub const BOOTY_BAY: Vector3d = Vector3d { x: -14237.9, y: 262.02, z: 24.75 };
/// Gurubashi Arena's entrance, the race finish.
pub const GURUBASHI_ARENA: Vector3d = Vector3d { x: -13284.747, y: 116.001, z: 24.36 };

/// How far each bot's route is offset from the shared path, at most, on each
/// axis. Small, so bots stay on the path's ground rather than in the bay or
/// the cliffs beside it; enough that they don't run in single file.
const RACE_JITTER_YARDS: f32 = 5.0;

/// What `build_race_path` found.
pub enum RacePath {
    /// A path over the navmesh, and the map the bots read heights from.
    Navmesh(Vec<Vector3d>, Arc<rustigator::vanilla::VanillaMap>),
    /// The loadtest was built without `WOW_VANILLA_USE_MAPS`.
    NoMaps,
}

/// Build the BB → Gurubashi path from the bake in the server's cache
/// directory. Returns the sparse polyline the path search produced, and the
/// map, which the bot drivers share to snap Z to the ground every heartbeat.
///
/// Why no densification: pre-baking Z values at path-build time means one
/// bad height pick poisons the rest of its segment through the lerp's
/// `z_hint` chain. Sampling at the bot's live XY (in
/// `movement.rs::tick_race`) keeps each heartbeat independent.
///
/// An error means maps were configured but the path couldn't be built: the
/// caller should stop rather than race on a made-up route.
pub fn build_race_path() -> Result<RacePath, String> {
    const DATA_PATH: Option<&str> = std::option_env!("WOW_VANILLA_USE_MAPS");
    let Some(data_path) = DATA_PATH else {
        return Ok(RacePath::NoMaps);
    };

    // The server's cache directory, so the loadtest and the server share one
    // bake.
    let output = std::env::temp_dir().join("wow_vanilla_server");
    if !output.exists() {
        return Err(format!(
            "navmesh cache directory {} does not exist; start the server first to bake it",
            output.display(),
        ));
    }

    let threads = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(2)
        .saturating_sub(2)
        .max(1);

    let started = std::time::Instant::now();
    let mut map = rustigator::vanilla::VanillaMap::build_gameobjects_and_map(
        data_path,
        &output,
        rustigator::vanilla::Map::EasternKingdoms,
        threads,
    )
    .map_err(|e| format!("VanillaMap::build_gameobjects_and_map failed: {e:?}"))?;

    // Pre-load every ADT tile in the BB → Gurubashi bounding box: the path
    // search only crosses loaded tiles, and loading from inside a bot's
    // heartbeat would stall it behind tile I/O.
    let (bb_tx, bb_ty) = world_to_adt(BOOTY_BAY.x, BOOTY_BAY.y);
    let (sw_tx, sw_ty) = world_to_adt(GURUBASHI_ARENA.x, GURUBASHI_ARENA.y);
    let (tx_min, tx_max) = (bb_tx.min(sw_tx), bb_tx.max(sw_tx));
    let (ty_min, ty_max) = (bb_ty.min(sw_ty), bb_ty.max(sw_ty));
    let mut loaded = 0;
    let mut failed = 0;
    for tx in tx_min..=tx_max {
        for ty in ty_min..=ty_max {
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

    // The pinned fork's search: steep ground costs four times as much,
    // rather than counting as flat.
    let mut raw_path = Vec::new();
    map.find_path_with(
        BOOTY_BAY,
        GURUBASHI_ARENA,
        &rustigator::raw::PathOptions::reference(false),
        &mut raw_path,
        None,
    )
    .map_err(|e| format!("find_path(BB, Gurubashi) failed: {e:?}"))?;

    let dist: f32 = raw_path
        .windows(2)
        .map(|w| {
            let dx = w[1].x - w[0].x;
            let dy = w[1].y - w[0].y;
            (dx * dx + dy * dy).sqrt()
        })
        .sum();
    let zs: Vec<f32> = raw_path.iter().map(|p| p.z).collect();
    tracing::info!(
        "race path: {} raw waypoints, {:.0} yd total, generated in {:.1}s — Z list: {:?}",
        raw_path.len(),
        dist,
        started.elapsed().as_secs_f32(),
        zs,
    );

    Ok(RacePath::Navmesh(raw_path, Arc::new(map)))
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
/// `[-RACE_JITTER_YARDS, RACE_JITTER_YARDS]`; the bot travels the path
/// offset by a constant `(jx, jy)` for its entire lifetime, so bots
/// spread around the path rather than run in single file.
pub fn jitter_for_slot(slot: u32) -> (f32, f32) {
    // Cheap hash → two floats in [-1, 1], scaled. Splitmix-style, no rand
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
    (next() * RACE_JITTER_YARDS, next() * RACE_JITTER_YARDS)
}
