//! Two class spells' scripts of mangos-classic's `OnCheckCast` and
//! `OnApply`: Curse of Doom, not at players and with a Doomguard now and
//! then at its victim's death (`CurseOfDoom`); Divine Intervention, not at
//! a flag's carrier (`DivineIntervention`).

use wow_world_messages::Guid;

use crate::game::data::spells::SpellTemplate;
use crate::game::guid;
use crate::game::map::Map;

const CURSE_OF_DOOM: u32 = 603;
/// Curse of Doom's Doomguard.
const DOOMGUARD: u32 = 18662;
const DIVINE_INTERVENTION: u32 = 19752;
/// Warsong Gulch's flags, carried.
const CARRYING: [u32; 2] = [23333, 23335];
/// `SPELL_FAILED_BAD_TARGETS`, `SPELL_FAILED_TARGET_AURASTATE`.
const BAD_TARGETS: u8 = 0x0a;
const TARGET_AURASTATE: u8 = 0x67;

/// Whether `s` at `target` is refused by its script.
pub fn check(map: &Map, s: &SpellTemplate, target: Guid) -> Option<u8> {
    match s.id {
        CURSE_OF_DOOM if guid::is_player(target) => Some(BAD_TARGETS),
        DIVINE_INTERVENTION
            if crate::game::map::auras::auras(map, target)
                .is_some_and(|a| CARRYING.iter().any(|f| a.has_spell(*f))) =>
        {
            Some(TARGET_AURASTATE)
        }
        _ => None,
    }
}

/// The casters of the Curses of Doom on `g`, as it dies.
pub fn doomed(map: &Map, g: Guid) -> Vec<Guid> {
    crate::game::map::auras::auras(map, g)
        .map(|a| {
            a.holders
                .iter()
                .filter(|h| h.spell == CURSE_OF_DOOM)
                .map(|h| h.caster)
                .collect()
        })
        .unwrap_or_default()
}

/// Each of `casters` whose curse took its victim to its death summons a
/// Doomguard, by mangos's `urand(0, 100) > 95` (5 in 101).
pub fn doom(map: &mut Map, casters: &[Guid]) {
    for c in casters {
        if super::outcome::roll(map) < 495 && map.unit_view(*c).is_some_and(|u| u.alive) {
            super::cast_triggered(map, *c, DOOMGUARD, None);
        }
    }
}
