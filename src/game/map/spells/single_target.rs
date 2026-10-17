//! Auras one caster holds on only one target at a time, as Thistle Tea's
//! `SingleTarget` (vmangos's single-target aura flag) keeps them: a
//! Polymorph, Sap, Banish, Hunter's Mark, Fear or Shackle on a new target
//! ends the caster's last one of its kind (the same family and icon, or
//! both judgements, or both polymorphs), and they end when their caster
//! leaves the world. Its death ends only Hunter's Mark.

use wow_world_messages::Guid;

use crate::game::data::spells::{self as sd, SpellTemplate};
use crate::game::map::Map;
use crate::game::map::auras::{self, Exclusive, Holder};

fn limited(s: &SpellTemplate) -> bool {
    s.custom(sd::CUSTOM_SINGLE_TARGET_AURA)
}

fn kind_of(s: &SpellTemplate, h: &Holder, other: &SpellTemplate) -> bool {
    (s.family_name == other.family_name && s.spell_icon == other.spell_icon)
        || matches!(
            (auras::exclusive(s, None), h.exclusive),
            (
                Some(Exclusive::PaladinJudgement),
                Some(Exclusive::PaladinJudgement)
            ) | (
                Some(Exclusive::MagePolymorph),
                Some(Exclusive::MagePolymorph)
            )
        )
}

fn units(map: &mut Map) -> Vec<Guid> {
    let mut units: Vec<Guid> = map.players.keys().copied().collect();
    // Of the creatures, only those with auras can have one to end.
    units.extend(map.with_auras_where(|c| !c.auras.is_empty(), |_, c| c.guid));
    units
}

/// `caster` put `s` on `target`: its others of that kind elsewhere end.
pub fn applied(map: &mut Map, caster: Guid, target: Guid, s: &SpellTemplate) {
    if !limited(s) {
        return;
    }
    let data = map.realm.data.clone();
    let gone: Vec<(Guid, u32)> = units(map)
        .into_iter()
        .filter(|g| *g != target)
        .flat_map(|g| {
            auras::auras(map, g)
                .map(|a| {
                    a.holders
                        .iter()
                        .filter(|h| h.caster == caster)
                        .filter(|h| {
                            data.spells
                                .get(h.spell)
                                .is_some_and(|t| limited(t) && kind_of(s, h, t))
                        })
                        .map(|h| (g, h.spell))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
        .collect();
    for (g, spell) in gone {
        auras::remove_spell(map, g, spell, Some(caster));
    }
}

/// `caster` left the world: its single-target auras on others end
/// (mangos's `RemoveNotOwnTrackedTargetAuras`, Thistle Tea's
/// `SingleTarget.detach`).
pub fn caster_gone(map: &mut Map, caster: Guid) {
    end_held(map, caster, false);
}

/// `caster` died: of its single-target auras on others only Hunter's Mark
/// (a stalking aura) ends, as Thistle Tea's `SingleTargetAuras.caster_died`
/// has it; mangos ends none. A Polymorph, Fear or Sap runs its course.
pub fn caster_died(map: &mut Map, caster: Guid) {
    end_held(map, caster, true);
}

fn end_held(map: &mut Map, caster: Guid, stalking_only: bool) {
    let data = map.realm.data.clone();
    let gone: Vec<(Guid, u32)> = units(map)
        .into_iter()
        .filter(|g| *g != caster)
        .flat_map(|g| {
            auras::auras(map, g)
                .map(|a| {
                    a.holders
                        .iter()
                        .filter(|h| {
                            h.caster == caster
                                && data.spells.get(h.spell).is_some_and(limited)
                                && (!stalking_only || h.has(auras::AURA_MOD_STALKED))
                        })
                        .map(|h| (g, h.spell))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
        .collect();
    for (g, spell) in gone {
        auras::remove_spell(map, g, spell, Some(caster));
    }
}
