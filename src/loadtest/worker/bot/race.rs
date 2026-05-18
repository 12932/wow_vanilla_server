//! Amazing Race helpers: shared waypoint list + per-bot jitter sampling.
//!
//! The race path is currently hardcoded — a 4-point polyline through the
//! Eastern Kingdoms south-to-north corridor. The plan calls for replacing
//! this with `namigator::vanilla::VanillaMap::find_path` output once the
//! nav-mesh cache has been baked; until then, the hardcoded path is
//! enough to wire up Race mode end-to-end and verify the driver behavior
//! in-game.

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
