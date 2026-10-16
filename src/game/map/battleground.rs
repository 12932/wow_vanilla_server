//! Battlegrounds.

use wow_world_messages::Guid;

use crate::game::guid;
use crate::game::map::Map;

/// The map of the battleground a battlemaster sends players to.
pub fn map_for_battlemaster(map: &Map, npc: Guid) -> u32 {
    match map.realm.data.battlemasters.get(&guid::entry(npc)) {
        Some(1) => 30,
        Some(2) => 489,
        Some(3) => 529,
        _ => 489,
    }
}
