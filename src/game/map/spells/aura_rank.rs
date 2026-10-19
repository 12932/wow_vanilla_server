//! Ranked buffs and their recipients' levels, as Thistle Tea's `AuraRank`
//! has it: a rank of a buff goes on no one more than ten levels below its
//! spell level. Cast at someone lower, the highest rank that may is cast
//! instead; a group buff passes over whoever is too low.

use wow_world_messages::Guid;

use crate::game::data::GameData;
use crate::game::data::spells::SpellTemplate;
use crate::game::map::Map;
use crate::game::map::auras;

use super::{EFFECT_APPLY_AREA_AURA_PARTY, EFFECT_APPLY_AURA, Targets};

/// `SPELL_ATTR_EX2_ALLOW_LOW_LEVEL_BUFF`.
const ATTR_EX2_ALLOW_LOW_LEVEL_BUFF: u32 = 0x8;

/// The friendly targets a buff is aimed at: a friend, a party member,
/// the party, the raid's class, friends about the caster or a place.
const FRIENDLY: [u32; 11] = [20, 21, 30, 31, 33, 34, 35, 37, 45, 57, 61];

/// The lowest level `s` may go on, or 0.
pub fn minimum_level(data: &GameData, s: &SpellTemplate) -> u32 {
    if s.passive()
        || s.attr(2, ATTR_EX2_ALLOW_LOW_LEVEL_BUFF)
        || !data.spells.ranks.get(&s.id).is_some_and(|r| r.rank > 0)
        || !auras::positive(s)
    {
        return 0;
    }
    let buff = s.effects.iter().any(|e| {
        e.effect == EFFECT_APPLY_AREA_AURA_PARTY
            || (e.effect == EFFECT_APPLY_AURA && e.aura != 0 && FRIENDLY.contains(&e.target_a))
    });
    if buff {
        s.spell_level.saturating_sub(10)
    } else {
        0
    }
}

/// The rank of `s` a unit of `level` may have: it, the highest before it
/// that may, or none.
pub fn for_level(data: &GameData, s: &SpellTemplate, level: u32) -> Option<u32> {
    if level >= minimum_level(data, s) {
        return Some(s.id);
    }
    let mut id = data.spells.ranks.get(&s.id).map_or(0, |r| r.prev);
    for _ in 0..20 {
        let r = data.spells.get(id)?;
        if r.spell_level <= level + 10 {
            return Some(id);
        }
        id = data.spells.ranks.get(&id).map_or(0, |r| r.prev);
    }
    None
}

/// The rank a player casts at `t`: lower if the one asked for is too high
/// for them, and none if no rank may go on them.
pub fn chosen(map: &Map, caster: Guid, s: &SpellTemplate, t: &Targets) -> Option<u32> {
    // Cast on oneself (the client names no unit), by one's own level:
    // mangos's `SpellCastTargets` takes the caster for a self cast, whose
    // level `SelectAuraRankForLevel` goes by.
    let target = t.unit.unwrap_or(caster);
    let Some(level) = map.unit_view(target).map(|u| u.level) else {
        return Some(s.id);
    };
    for_level(&map.realm.data, s, level)
}

/// A group buff's recipients, less those too low for its rank (the caster
/// and the one it was cast at excepted).
pub fn recipients(
    map: &Map,
    caster: Guid,
    s: &SpellTemplate,
    t: &Targets,
    resolved: Vec<(Guid, [bool; 3])>,
) -> Vec<(Guid, [bool; 3])> {
    let minimum = minimum_level(&map.realm.data, s);
    if minimum == 0
        || s.effects
            .iter()
            .any(|e| e.effect == EFFECT_APPLY_AREA_AURA_PARTY)
    {
        return resolved;
    }
    resolved
        .into_iter()
        .filter(|(g, _)| {
            *g == caster
                || Some(*g) == t.unit
                || map.unit_view(*g).is_none_or(|u| u.level >= minimum)
        })
        .collect()
}
