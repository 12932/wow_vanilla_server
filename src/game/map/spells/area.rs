//! `spell_area` for the spells module: the rules and their reconciling are
//! [`crate::game::map::spell_areas`]'s; a cast asks through here, and a
//! refusal names the area the spell wants.

use wow_world_messages::Guid;

use crate::game::data::GameData;
use crate::game::map::Map;

/// Whether `g` may cast (or keep) `spell` here.
pub fn allows(map: &Map, g: Guid, spell: u32) -> bool {
    crate::game::map::spell_areas::allowed(map, g, spell)
}

/// The area a spell's first row names, for the refusal's message.
pub fn required(data: &GameData, spell: u32) -> u32 {
    data.scripts
        .spell_areas
        .iter()
        .find(|r| r.spell == spell)
        .map_or(0, |r| r.area)
}

/// The player's area auras in line with where they are and what they've done.
pub fn reconcile(map: &mut Map, g: Guid) {
    crate::game::map::spell_areas::reconcile(map, g);
}
