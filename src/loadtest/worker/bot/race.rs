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

/// Build the BB → Gurubashi path from the pre-baked navmesh cache. Reads the
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
/// Caller's responsibility to fall back to `hardcoded_bb_to_arena()` on
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
    let (sw_tx, sw_ty) = world_to_adt(GURUBASHI_ARENA.x, GURUBASHI_ARENA.y);
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

    let raw_path: Vec<Vector3d> = map
        .find_path(BOOTY_BAY, GURUBASHI_ARENA)
        .map_err(|e| format!("find_path(BB, Gurubashi) failed: {e:?}"))?
        .to_vec();

    // Densify the Detour string-pulled path so the bot driver's
    // straight-line XY interpolation between consecutive waypoints
    // doesn't cut through terrain (the raw path has 10-30+ yd gaps).
    // For each (prev → next) segment we sample every
    // `DENSE_SPACING_YD` yards and raycast the actual ground Z via
    // `find_height` — same convention the server uses in
    // `pathfinding_maps.rs::ground_height`.
    //
    // The first sample is `BOOTY_BAY` (our hand-picked dock coords),
    // not `raw_path[0]` — namigator snaps the BB input to the rocky
    // seabed under the docks, and we want bots spawning on the
    // visible plank surface instead.
    const DENSE_SPACING_YD: f32 = 10.0;
    let mut dense: Vec<Vector3d> = Vec::with_capacity(raw_path.len() * 30);
    dense.push(BOOTY_BAY);
    let mut prev = BOOTY_BAY;
    let mut fallbacks = 0_usize;
    for &next in raw_path.iter().skip(1) {
        let dx = next.x - prev.x;
        let dy = next.y - prev.y;
        let len = (dx * dx + dy * dy).sqrt();
        if len < DENSE_SPACING_YD {
            dense.push(next);
            prev = next;
            continue;
        }
        let steps = (len / DENSE_SPACING_YD).ceil() as usize;
        for k in 1..=steps {
            let t = k as f32 / steps as f32;
            let x = prev.x + dx * t;
            let y = prev.y + dy * t;
            let z_hint = prev.z + (next.z - prev.z) * t;
            // `find_heights` returns every navmesh-Z candidate in the
            // (x,y) column — for outdoor STV that's typically just
            // the forest floor, but anywhere with multi-level
            // structures (a bridge, a building, an arena rim) it can
            // include rooftops too. Pick the candidate closest to
            // `z_hint` (the lerp between adjacent waypoint Z values)
            // so we lock onto the ground rather than landing on a
            // structure above. `find_height` (singular) is a poor
            // fit here — it does a tiny 1-yd `findNearestPoly`
            // search around start.z that misses when start.z is
            // off-mesh even slightly.
            let z = match map.find_heights(x, y) {
                Ok(heights) if !heights.is_empty() => {
                    heights
                        .iter()
                        .copied()
                        .min_by(|a, b| {
                            (a - z_hint)
                                .abs()
                                .partial_cmp(&(b - z_hint).abs())
                                .unwrap_or(std::cmp::Ordering::Equal)
                        })
                        .unwrap_or(z_hint)
                }
                _ => {
                    fallbacks += 1;
                    z_hint
                }
            };
            dense.push(Vector3d { x, y, z });
        }
        prev = next;
    }

    let dist: f32 = dense
        .windows(2)
        .map(|w| {
            let dx = w[1].x - w[0].x;
            let dy = w[1].y - w[0].y;
            (dx * dx + dy * dy).sqrt()
        })
        .sum();
    tracing::info!(
        "race path: {} raw waypoints, densified to {} at {} yd spacing ({} find_height fallbacks), {:.0} yd total, generated in {:.1}s",
        raw_path.len(),
        dense.len(),
        DENSE_SPACING_YD,
        fallbacks,
        dist,
        started.elapsed().as_secs_f32(),
    );

    Ok(dense)
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
