//! Loot.

use wow_world_messages::Guid;

use crate::game::fields::UNIT_DYNAMIC_FLAGS;
use crate::game::map::Map;

/// `UNIT_DYNAMIC_FLAGS`: there is loot on the corpse.
pub const DYNFLAG_LOOTABLE: u32 = 0x1;

/// A creature died: its loot is rolled for its tapper.
pub fn creature_died(map: &mut Map, creature: Guid, _tapper: Guid) {
    let _ = (map, creature, UNIT_DYNAMIC_FLAGS);
}
