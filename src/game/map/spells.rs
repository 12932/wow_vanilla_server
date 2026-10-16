//! Spells.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::SMSG_LEARNED_SPELL;

use crate::game::data::GameData;
use crate::game::map::Map;

/// The spell learning `spell` gives: a trainer's spell may be one that
/// teaches another.
pub fn learns(_data: &GameData, spell: u32) -> u32 {
    spell
}

/// `player` learns `spell`.
pub fn learn(map: &mut Map, player: Guid, spell: u32) {
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    if spell == 0 || p.spells.contains(&spell) {
        return;
    }
    p.spells.push(spell);
    p.outbound.send(&SMSG_LEARNED_SPELL { id: spell });
}
