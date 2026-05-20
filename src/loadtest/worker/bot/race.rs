//! Amazing Race helpers: shared waypoint list + per-bot jitter sampling.
//!
//! Two paths to a BB → Gurubashi waypoint list:
//! - `build_race_path()` — calls namigator's `find_path` against the
//!   pre-baked nav-mesh cache. Returns a polyline that actually hugs
//!   walkable terrain (around buildings, over bridges). Requires
//!   `WOW_VANILLA_USE_MAPS` to have been set at compile time AND the
//!   cache to be present on disk.
//! - `hardcoded_bb_to_arena()` — coarse fallback. Used when namigator
//!   isn't available (env var unset at build time, cache missing,
//!   find_path errors). Bots still walk the route, but the geometry
//!   clips through anything between waypoints.
//!
//! The race endpoint is the entrance to Gurubashi Arena (~1.6 km
//! north of Booty Bay along the coast). Bots reverse direction on
//! arrival and run back, looping forever.

use std::sync::{Arc, Mutex};
use wow_world_messages::vanilla::Vector3d;

/// Coarse Booty Bay → Gurubashi Arena waypoint list. Two midpoints
/// hand-picked from in-game eye-balling so the straight-line
/// interpolation doesn't cut through any major terrain feature.
/// Used as a fallback when namigator is unavailable.
pub fn hardcoded_bb_to_arena() -> Vec<Vector3d> {
    vec![
        BOOTY_BAY,
        // STV coastal road midpoint (north of BB, south of the arena)
        Vector3d { x: -13800.0, y: 200.0, z: 30.0 },
        GURUBASHI_ARENA,
    ]
}

/// Booty Bay docks — race start. Constant so both `build_race_path()`
/// and `hardcoded_bb_to_arena()` agree on the endpoints. Coordinates
/// hand-picked from in-game (`.whereami` on the docks) so they sit
/// cleanly on the walkable plank surface rather than the namigator
/// mesh under it (which is rocky seabed several yards below).
pub const BOOTY_BAY: Vector3d = Vector3d { x: -14237.9, y: 262.02, z: 24.75 };
/// Gurubashi Arena entrance — race finish. ~1.6 km north of Booty
/// Bay along the coast road. Replaces the earlier Stormwind
/// endpoint, which kept failing `find_path` with `UnknownPath`
/// (probably a missing-bridge gap on the STV → Westfall corridor).
pub const GURUBASHI_ARENA: Vector3d = Vector3d { x: -13284.747, y: 116.001, z: 24.36 };

/// Build the BB → Gurubashi path from the pre-baked navmesh cache.
/// Returns the SPARSE polyline as namigator's `find_path` produced
/// it (no densification) PLUS a shared `Arc<Mutex<VanillaMap>>` the
/// bot driver uses to snap Z to actual ground every heartbeat.
///
/// Why no densification: pre-baking Z values at path-build time
/// means any single bad `find_heights` pick poisons all subsequent
/// ticks in that segment via the lerp `z_hint` chain. Per-tick
/// sampling at the bot's live XY (in `movement.rs::tick_race`)
/// avoids the drift problem — each heartbeat is independent.
///
/// Returns `Err` if:
/// - `WOW_VANILLA_USE_MAPS` was unset at compile time
/// - The cache directory doesn't exist
/// - `find_path` returns `UnknownPath`
///
/// Caller's responsibility to fall back to `hardcoded_bb_to_arena()`
/// on error (in which case bots run without per-tick Z sampling
/// and use the driver's path-Z lerp).
pub fn build_race_path() -> Result<
    (
        Vec<Vector3d>,
        Arc<Mutex<namigator::vanilla::VanillaMap>>,
    ),
    String,
> {
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

    // Pre-load every ADT tile in the BB → Gurubashi bounding box.
    // `find_path` refuses to cross unloaded tiles, and the per-tick
    // `find_heights` runtime calls also need tiles already loaded
    // — lazy-loading from inside a hot path would block the bot's
    // heartbeat behind tile I/O.
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

    let raw_path: Vec<Vector3d> = map
        .find_path(BOOTY_BAY, GURUBASHI_ARENA)
        .map_err(|e| format!("find_path(BB, Gurubashi) failed: {e:?}"))?
        .to_vec();

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

    Ok((raw_path, Arc::new(Mutex::new(map))))
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
