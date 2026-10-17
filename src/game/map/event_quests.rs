//! What vmangos's C++ does for event quests (`SpecialFlags` 0x2) that
//! the data and Thistle Tea's scripts leave out.
//!
//! Honoring a Hero (8149, 8150): Uther's and Grom's Tributes, used at the
//! statue, offer the tribute of the user's race, a spell that sets out its
//! wreath and completes the quest (vmangos's `EffectScriptEffect`).

use wow_world_messages::Guid;

use crate::game::data::spells::SpellTemplate;
use crate::game::map::Map;

const EFFECT_SCRIPT_EFFECT: u32 = 77;

const UTHERS_TRIBUTE: u32 = 24194;
const GROMS_TRIBUTE: u32 = 24195;

/// Each tribute's offering, by race (`Race` numbers: 1 human, 2 orc, 3
/// dwarf, 4 night elf, 5 undead, 6 tauren, 7 gnome, 8 troll).
const OFFERINGS: &[(u32, u8, u32)] = &[
    (UTHERS_TRIBUTE, 1, 24105),
    (UTHERS_TRIBUTE, 7, 24106),
    (UTHERS_TRIBUTE, 3, 24107),
    (UTHERS_TRIBUTE, 4, 24108),
    (GROMS_TRIBUTE, 8, 24101),
    (GROMS_TRIBUTE, 6, 24102),
    (GROMS_TRIBUTE, 5, 24103),
    (GROMS_TRIBUTE, 2, 24104),
];

/// Effect `index` of `s`, cast by `caster` at `target`, if it is one of
/// these: whether it was.
pub fn effect(map: &mut Map, caster: Guid, _target: Guid, s: &SpellTemplate, index: usize) -> bool {
    if s.effects[index].effect != EFFECT_SCRIPT_EFFECT
        || !matches!(s.id, UTHERS_TRIBUTE | GROMS_TRIBUTE)
    {
        return false;
    }
    let Some(race) = map.players.get(&caster).map(|p| p.row.race) else {
        return true;
    };
    if let Some((_, _, offering)) = OFFERINGS
        .iter()
        .find(|(tribute, r, _)| *tribute == s.id && *r == race)
    {
        crate::game::map::spells::cast_triggered(map, caster, *offering, Some(caster));
    }
    true
}
