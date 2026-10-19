//! What a pet has of its master's: their spell modifiers (talents such as
//! Improved Imp and Improved Firebolt change the pet's spells, as Thistle
//! Tea's `PetSpellModifiers` projects them), their passive talents aimed
//! at the pet (`APPLY_AREA_AURA_PET`: Unleashed Fury, Ferocity), and the
//! auras `spell_pet_auras` gives the pet for an aura of theirs (Master
//! Demonologist, Spirit Bond). Each is a hidden aura on the pet from its
//! master, marked as inherited by naming the master as its item, and kept
//! in line with the master every second.

use std::time::Duration;

use wow_world_messages::Guid;

use super::*;
use crate::game::map::Map;
use crate::game::map::auras::{AURA_ADD_FLAT_MODIFIER, AURA_ADD_PCT_MODIFIER, Context};

/// How often a pet's inheritance is brought in line.
pub const EVERY: Duration = Duration::from_secs(1);

/// What the pet should have of its master's: each spell and the effects
/// (with their amounts) it gives.
fn wanted(map: &Map, pet: Guid, owner: Guid) -> Vec<(u32, Vec<(usize, i32)>)> {
    let data = &map.realm.data;
    let Some(p) = map.players.get(&owner) else {
        return Vec::new();
    };
    let entry = map.creature(pet).map_or(0, |c| c.entry);
    let level = u32::from(p.level());
    let mut out: Vec<(u32, Vec<(usize, i32)>)> = Vec::new();
    // The master's spell modifiers.
    for h in &p.auras.holders {
        let effects: Vec<(usize, i32)> = h
            .effects
            .iter()
            .filter(|x| matches!(x.kind, AURA_ADD_FLAT_MODIFIER | AURA_ADD_PCT_MODIFIER))
            .map(|x| (x.effect as usize, x.amount * h.stacks.max(1) as i32))
            .collect();
        if !effects.is_empty() && !out.iter().any(|(s, _)| *s == h.spell) {
            out.push((h.spell, effects));
        }
    }
    // Their passives aimed at the pet.
    for s in p.spells.iter().filter_map(|s| data.spells.get(*s)) {
        if !s.passive() {
            continue;
        }
        let effects: Vec<(usize, i32)> = (0..3)
            .filter(|i| {
                s.effects[*i].effect == EFFECT_APPLY_AREA_AURA_PET && s.effects[*i].aura != 0
            })
            .map(|i| (i, crate::game::map::spells::points(s, i, level)))
            .collect();
        if !effects.is_empty() && !out.iter().any(|(x, _)| *x == s.id) {
            out.push((s.id, effects));
        }
    }
    // `spell_pet_auras`.
    for (spell, for_pet, aura) in data.pets.owner_auras.iter().copied() {
        if (for_pet != 0 && for_pet != entry) || !p.auras.has_spell(spell) {
            continue;
        }
        let Some(t) = data.spells.get(aura) else {
            continue;
        };
        let effects: Vec<(usize, i32)> = (0..3)
            .filter(|i| matches!(t.effects[*i].effect, 6 | 35 | 119) && t.effects[*i].aura != 0)
            .map(|i| (i, crate::game::map::spells::points(t, i, level)))
            .collect();
        if !effects.is_empty() && !out.iter().any(|(x, _)| *x == aura) {
            out.push((aura, effects));
        }
    }
    out
}

/// Bring the pet's inherited auras in line with its master.
pub fn sync(map: &mut Map, pet: Guid) {
    let Some(owner) = state(map, pet)
        .map(|s| s.owner)
        .filter(|o| guid::is_player(*o))
    else {
        return;
    };
    let wanted = wanted(map, pet, owner);
    let have: Vec<(u32, Vec<(u8, i32)>)> = crate::game::map::auras::auras(map, pet)
        .map(|a| {
            a.holders
                .iter()
                .filter(|h| h.caster == owner && h.item == Some(owner))
                .map(|h| {
                    (
                        h.spell,
                        h.effects.iter().map(|x| (x.effect, x.amount)).collect(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    // Gone from the master, or changed: off the pet.
    for (spell, effects) in &have {
        let same = wanted.iter().any(|(s, e)| {
            s == spell
                && e.len() == effects.len()
                && e.iter().all(|(i, a)| effects.contains(&(*i as u8, *a)))
        });
        if !same {
            let spell = *spell;
            crate::game::map::auras::remove_where(map, pet, |h| {
                h.spell == spell && h.caster == owner && h.item == Some(owner)
            });
        }
    }
    let data = map.realm.data.clone();
    for (spell, effects) in wanted {
        let present = crate::game::map::auras::auras(map, pet).is_some_and(|a| {
            a.holders
                .iter()
                .any(|h| h.spell == spell && h.caster == owner && h.item == Some(owner))
        });
        if present {
            continue;
        }
        let Some(t) = data.spells.get(spell) else {
            continue;
        };
        let _ = crate::game::map::auras::apply_spell(
            map,
            owner,
            pet,
            t,
            &effects,
            Context {
                item: Some(owner),
                duration: Some(None),
                ..Default::default()
            },
        );
    }
}

/// A modifier of `master`'s (from `spell`, touching the spells of its
/// family whose flags meet `mask`) came or went: their pet's and totems'
/// own permanent passives it touches are put on again, their amounts as
/// the modifiers now make them, one holder each (mangos's
/// `Aura::ReapplyAffectedPassiveAuras` over `CONTROLLED_PET |
/// CONTROLLED_TOTEMS`; Thistle Tea's `PetSpellModifiers.sync`). A dead
/// pet's are too, and it stays dead.
pub fn master_mods_changed(map: &mut Map, master: Guid, spell: u32, mask: u64) {
    use crate::game::map::spells::mods::touches;
    let data = map.realm.data.clone();
    let Some(family) = data.spells.get(spell).map(|t| t.family_name) else {
        return;
    };
    let Some(p) = map.players.get(&master) else {
        return;
    };
    let units: Vec<Guid> = p
        .owned
        .pet
        .into_iter()
        .chain(p.owned.totems.iter().flatten().copied())
        .collect();
    for g in units {
        let affected: Vec<u32> = crate::game::map::auras::auras(map, g)
            .map(|a| {
                a.holders
                    .iter()
                    .filter(|h| h.caster == g && h.passive && h.expires.is_none())
                    .filter(|h| {
                        data.spells
                            .get(h.spell)
                            .is_some_and(|t| t.passive() && touches(family, mask, t))
                    })
                    .map(|h| h.spell)
                    .collect()
            })
            .unwrap_or_default();
        for s in affected {
            crate::game::map::auras::remove_spell(map, g, s, Some(g));
            summon::apply_passive(map, g, s);
        }
    }
}
