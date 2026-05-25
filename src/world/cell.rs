//! Per-map registries and shared helpers.
//!
//! Concurrency is per-MAP (cmangos-style): one `MapState` per continent,
//! keyed by [`Map`] in `World::maps_state`. The fine 33.33-yd spatial grid
//! and the coarse 533.33-yd activation grid live in
//! [`crate::world::spatial`]; this module holds the process-wide pacer and
//! player registries that aren't owned by a single map.

use wow_world_base::vanilla::Map;
use wow_world_messages::vanilla::Vector3d;
use wow_world_messages::Guid;

/// Bucket a slab of creatures by their `map`. Used by `World::with_creatures`
/// (and `for_test`) to partition the worlddb's full creature slab into one
/// per-`MapState` bucket at startup. Pure function; drains the input slab.
pub fn partition_creatures(
    slab: slab::Slab<crate::world::world_opcode_handler::creature::Creature>,
) -> ahash::AHashMap<
    Map,
    Vec<crate::world::world_opcode_handler::creature::Creature>,
> {
    let mut out: ahash::AHashMap<
        Map,
        Vec<crate::world::world_opcode_handler::creature::Creature>,
    > = ahash::AHashMap::new();
    for (_, creature) in slab {
        out.entry(creature.map).or_default().push(creature);
    }
    out
}


/// Per-cell pacer state snapshot. Updated by each per-cell tokio
/// task at the end of its tick; read by the `.cells` GM command.
/// Kept here (rather than reaching into `World::cells` from the GM
/// handler) so the handler signature doesn't need to grow another
/// parameter and so reads are O(1) under a brief Mutex lock.
#[derive(Debug, Clone)]
pub struct PacerSnapshot {
    pub current_interval_ms: u64,
    pub slow_ema: f32,
    pub healthy_streak: u32,
    pub last_tick_ms: u64,
}

/// Process-wide map of per-map pacer state. The per-map task writes its
/// entry at the end of every tick; the `.cells` GM command reads it.
/// Locked briefly only — no contention with the hot tick path.
pub static PACER_STATES: std::sync::LazyLock<
    std::sync::Mutex<ahash::AHashMap<Map, PacerSnapshot>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(ahash::AHashMap::new()));

/// Convenience: publish a fresh snapshot for `map`. Called by the per-map
/// task; cheap (allocates only on first call per map).
pub fn publish_pacer_state(map: Map, snap: PacerSnapshot) {
    if let Ok(mut guard) = PACER_STATES.lock() {
        guard.insert(map, snap);
    }
}

/// Process-wide registry of in-world player positions. Maintained by
/// every `MapState::insert_client` / `remove_client` so the GM
/// command parser can answer ".go PlayerName" cross-cell without
/// having to lock every cell's mutex from the GM's cell.
///
/// Entries are best-effort: a player mid-transition (left cell A
/// but not yet admitted to B) may briefly be absent. The `.go`
/// command surfaces this as "Unable to find player 'X'", which is the
/// natural error today as well.
#[derive(Debug, Clone)]
pub struct PlayerRegistryEntry {
    pub guid: Guid,
    pub name: String,
    pub map: Map,
    pub position: Vector3d,
    pub orientation: f32,
}

pub static PLAYER_REGISTRY: std::sync::LazyLock<
    std::sync::Mutex<ahash::AHashMap<Guid, PlayerRegistryEntry>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(ahash::AHashMap::new()));

/// Lowercase-name → guid index. Updated in lockstep with
/// [`PLAYER_REGISTRY`] so `.go PlayerName` (by name) is O(1).
pub static PLAYER_NAME_INDEX: std::sync::LazyLock<
    std::sync::Mutex<ahash::AHashMap<String, Guid>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(ahash::AHashMap::new()));

/// Register or refresh a player's entry in both indexes. Called by
/// `MapState::insert_client` after the slab insert succeeds.
pub fn register_player(entry: PlayerRegistryEntry) {
    let name_lc = entry.name.to_lowercase();
    let guid = entry.guid;
    if let Ok(mut reg) = PLAYER_REGISTRY.lock() {
        reg.insert(guid, entry);
    }
    if let Ok(mut idx) = PLAYER_NAME_INDEX.lock() {
        idx.insert(name_lc, guid);
    }
}

/// Drop a player from both indexes. Called by
/// `MapState::remove_client`. No-op if the player isn't registered
/// (e.g. if the caller forgot to register them on insert — defensive,
/// not load-bearing).
pub fn unregister_player(guid: Guid) {
    let name_lc = {
        if let Ok(mut reg) = PLAYER_REGISTRY.lock() {
            reg.remove(&guid).map(|e| e.name.to_lowercase())
        } else {
            None
        }
    };
    if let Some(name_lc) = name_lc
        && let Ok(mut idx) = PLAYER_NAME_INDEX.lock()
    {
        idx.remove(&name_lc);
    }
}

/// Look up a player's position by guid. Used by `.go` (no args, with
/// selected target) when the target isn't in the GM's local cell.
pub fn lookup_player_position(guid: Guid) -> Option<(Map, Vector3d, f32)> {
    let reg = PLAYER_REGISTRY.lock().ok()?;
    reg.get(&guid).map(|e| (e.map, e.position, e.orientation))
}

/// Look up a player's position by case-insensitive name. Used by
/// `.go PlayerName`.
pub fn lookup_player_position_by_name(name: &str) -> Option<(Map, Vector3d, f32)> {
    let name_lc = name.to_lowercase();
    let guid = {
        let idx = PLAYER_NAME_INDEX.lock().ok()?;
        idx.get(&name_lc).copied()?
    };
    lookup_player_position(guid)
}

#[cfg(test)]
mod tests {
    use super::*;





    // ── Tests for partition_creatures (Stage 5 partition TDD) ──

    use crate::world::world_opcode_handler::creature::Creature;
    use slab::Slab;
    use wow_world_messages::Guid;

    /// Build a `Creature` at the given map + (x, y) position with a
    /// unique guid. Used by the partition tests below.
    fn make_creature_at(guid_int: u64, map: Map, x: f32, y: f32) -> Creature {
        let mut c = Creature::new(format!("test_{guid_int}"), Guid::new(guid_int));
        c.map = map;
        c.info.position = Vector3d { x, y, z: 0.0 };
        c
    }



    #[test]
    fn partition_creatures_preserves_empty_input() {
        let slab: Slab<Creature> = Slab::new();
        let buckets = partition_creatures(slab);
        assert!(buckets.is_empty());
    }

    #[test]
    fn partition_creatures_groups_multiple_in_same_cell() {
        // Two creatures in the same 1000-yd cell should land in one
        // bucket of size 2; confirms `entry(...).or_default().push(...)`.
        let mut slab: Slab<Creature> = Slab::new();
        slab.insert(make_creature_at(1, Map::EasternKingdoms, 100.0, 100.0));
        slab.insert(make_creature_at(2, Map::EasternKingdoms, 900.0, 900.0));

        let buckets = partition_creatures(slab);

        assert_eq!(buckets.len(), 1);
        assert_eq!(buckets.get(&Map::EasternKingdoms).map(Vec::len), Some(2));
    }

    #[test]
    fn partition_creatures_separates_by_map() {
        // Same (x, y) on different maps must produce different
        // CellKeys.
        let mut slab: Slab<Creature> = Slab::new();
        slab.insert(make_creature_at(1, Map::EasternKingdoms, 100.0, 100.0));
        slab.insert(make_creature_at(2, Map::Kalimdor, 100.0, 100.0));

        let buckets = partition_creatures(slab);

        assert_eq!(buckets.len(), 2);
        assert!(buckets.contains_key(&Map::EasternKingdoms));
        assert!(buckets.contains_key(&Map::Kalimdor));
    }
}
