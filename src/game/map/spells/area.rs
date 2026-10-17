//! `spell_area`, as Thistle Tea's `Spell.Area` and `SpellAreas` have it: a
//! spell with rows there is cast only in one of their zones or areas, and
//! only by whom a row names; one marked autocast is had there without
//! casting. Whoever stops matching (they left, finished the quest, lost the
//! aura) loses the spell's auras.

use wow_world_messages::Guid;

use crate::game::data::GameData;
use crate::game::data::spells::SpellArea;
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras;
use crate::game::map::death::Death;

/// Whether row `a` holds for player `g` where they are now.
fn matches(map: &Map, g: Guid, a: &SpellArea) -> bool {
    let Some(p) = map.players.get(&g) else {
        return false;
    };
    if a.area != 0 && a.area != p.zone && a.area != p.area {
        return false;
    }
    let race = p.fields.get_byte(UNIT_FIELD_BYTES_0, 0);
    let gender = p.fields.get_byte(UNIT_FIELD_BYTES_0, 2);
    if a.gender != 2 && a.gender != gender {
        return false;
    }
    if a.race_mask != 0 && (race == 0 || a.race_mask & (1 << (race - 1)) == 0) {
        return false;
    }
    let rewarded = |q: u32| p.journal.rewarded.contains(&q);
    if a.quest_start != 0
        && !rewarded(a.quest_start)
        && !(a.quest_start_active && p.journal.has(a.quest_start))
    {
        return false;
    }
    if a.quest_end != 0 && rewarded(a.quest_end) {
        return false;
    }
    if a.aura_spell != 0 {
        let has = auras::auras(map, g).is_some_and(|x| x.has_spell(a.aura_spell.unsigned_abs()));
        if has != (a.aura_spell > 0) {
            return false;
        }
    }
    true
}

/// Whether `g` may cast (or keep) `spell` here. Only players are held to
/// it.
pub fn allows(map: &Map, g: Guid, spell: u32) -> bool {
    let Some(rules) = map.realm.data.spells.areas.get(&spell) else {
        return true;
    };
    !map.players.contains_key(&g) || rules.iter().any(|a| matches(map, g, a))
}

/// The area a spell's cast failure names.
pub fn required(data: &GameData, spell: u32) -> u32 {
    data.spells
        .areas
        .get(&spell)
        .and_then(|r| r.iter().find(|a| a.area > 0))
        .map_or(0, |a| a.area)
}

/// Bring player `g`'s area spells in line with where they are: the auras
/// of those that no longer hold come off, and autocast ones that do go
/// on.
pub fn reconcile(map: &mut Map, g: Guid) {
    let data = map.realm.data.clone();
    if data.spells.areas.is_empty() || !map.players.contains_key(&g) {
        return;
    }
    remove_invalid(map, g);
    if map.players.get(&g).is_none_or(|p| p.death != Death::Alive) {
        return;
    }
    let mut autocast: Vec<u32> = data
        .spells
        .areas
        .iter()
        .filter(|(_, rules)| rules.iter().any(|a| a.autocast))
        .map(|(id, _)| *id)
        .collect();
    autocast.sort_unstable();
    for id in autocast {
        let had = auras::auras(map, g).is_some_and(|a| a.has_spell(id));
        let holds = data.spells.areas[&id]
            .iter()
            .any(|a| a.autocast && matches(map, g, a));
        if !had && holds {
            super::cast_triggered(map, g, id, Some(g));
        }
    }
    remove_invalid(map, g);
}

/// Take off `g`'s auras of area spells that no longer hold; again while
/// that takes another's reason away.
fn remove_invalid(map: &mut Map, g: Guid) {
    for _ in 0..4 {
        let gone: Vec<u32> = auras::auras(map, g)
            .map(|a| {
                a.holders
                    .iter()
                    .map(|h| h.spell)
                    .filter(|s| map.realm.data.spells.areas.contains_key(s))
                    .collect()
            })
            .unwrap_or_default();
        let gone: Vec<u32> = gone.into_iter().filter(|s| !allows(map, g, *s)).collect();
        if gone.is_empty() {
            return;
        }
        for spell in gone {
            auras::remove_spell(map, g, spell, None);
        }
    }
}
