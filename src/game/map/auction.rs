//! The auction house.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::AuctionHouse;

use crate::game::fields::UNIT_FIELD_FACTIONTEMPLATE;
use crate::game::map::Map;

/// Which house an auctioneer is of, by its faction, as mangos's
/// `GetAuctionHouseEntry` picks it: the cities' own (Stormwind's, the
/// dwarves', Darnassus's, Undercity's, Thunder Bluff's, the orcs'), the
/// goblins' of Booty Bay, Gadgetzan and Everlook; else the Alliance's
/// (Stormwind's), the Horde's or the goblins' by its side.
pub fn house_of(map: &Map, npc: Guid) -> AuctionHouse {
    let Some(c) = map.creature(npc) else {
        return AuctionHouse::Goblin;
    };
    let faction = c.fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE);
    match faction {
        12 => AuctionHouse::Stormwind,
        29 => AuctionHouse::Horde,
        55 | 534 => AuctionHouse::Alliance,
        68 => AuctionHouse::Undercity,
        80 => AuctionHouse::Darnassus,
        104 => AuctionHouse::ThunderBluff,
        120 | 474 | 855 => AuctionHouse::Goblin,
        _ => match map.realm.data.faction_templates.get(&faction) {
            Some(f) if f.ours & 0x2 != 0 => AuctionHouse::Stormwind,
            Some(f) if f.ours & 0x4 != 0 => AuctionHouse::Horde,
            _ => AuctionHouse::Goblin,
        },
    }
}
