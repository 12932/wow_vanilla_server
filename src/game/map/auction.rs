//! The auction house.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::AuctionHouse;

use crate::game::fields::UNIT_FIELD_FACTIONTEMPLATE;
use crate::game::map::Map;

/// Which house an auctioneer is of, by its faction: the Alliance's, the
/// Horde's, or the goblins'.
pub fn house_of(map: &Map, npc: Guid) -> AuctionHouse {
    let Some(c) = map.creature(npc) else {
        return AuctionHouse::Goblin;
    };
    let faction = c.fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE);
    match map.realm.data.faction_templates.get(&faction) {
        Some(f) if f.ours & 0x2 != 0 => AuctionHouse::Alliance,
        Some(f) if f.ours & 0x4 != 0 => AuctionHouse::Horde,
        _ => AuctionHouse::Goblin,
    }
}
