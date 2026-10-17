//! The stacking trinkets whose counts and lifetimes the spell data doesn't
//! say (Thistle Tea's `Aura.StackingProc`): Unstable Power and Restless
//! Strength start full and lose a stack a spell or a blow; Ascendance
//! gains one a spell for six spells; Brittle Armor and Mercurial Shield
//! start full and lose one each time their wearer is hit. Each bonus goes
//! with the aura that gave it.

use wow_world_messages::Guid;

use crate::game::data::spells::SpellTemplate;
use crate::game::map::Map;
use crate::game::map::auras::{self, AURA_MOD_DAMAGE_DONE, Auras, Holder};
use crate::game::map::procs::Event;

pub const UNSTABLE_POWER: u32 = 24658;
pub const UNSTABLE_POWER_BONUS: u32 = 24659;
pub const ASCENDANCE: u32 = 28200;
pub const ASCENDANCE_BONUS: u32 = 28204;
pub const RESTLESS_STRENGTH: u32 = 24661;
pub const RESTLESS_STRENGTH_BONUS: u32 = 24662;
pub const BRITTLE_ARMOR: u32 = 24574;
pub const BRITTLE_ARMOR_BONUS: u32 = 24575;
pub const MERCURIAL_SHIELD: u32 = 26463;
pub const MERCURIAL_SHIELD_BONUS: u32 = 26464;
/// The scripts a hit casts: each takes a stack off a bonus.
pub const BRITTLE_ARMOR_HIT: u32 = 24590;
pub const MERCURIAL_SHIELD_HIT: u32 = 26465;

/// A bonus's parent.
fn parent_of(bonus: u32) -> Option<u32> {
    match bonus {
        UNSTABLE_POWER_BONUS => Some(UNSTABLE_POWER),
        ASCENDANCE_BONUS => Some(ASCENDANCE),
        RESTLESS_STRENGTH_BONUS => Some(RESTLESS_STRENGTH),
        BRITTLE_ARMOR_BONUS => Some(BRITTLE_ARMOR),
        MERCURIAL_SHIELD_BONUS => Some(MERCURIAL_SHIELD),
        _ => None,
    }
}

/// The bonus a parent starts with.
fn initial(parent: u32) -> Option<u32> {
    match parent {
        UNSTABLE_POWER => Some(UNSTABLE_POWER_BONUS),
        RESTLESS_STRENGTH => Some(RESTLESS_STRENGTH_BONUS),
        _ => None,
    }
}

fn full_stacks(bonus: u32) -> bool {
    matches!(
        bonus,
        UNSTABLE_POWER_BONUS
            | RESTLESS_STRENGTH_BONUS
            | BRITTLE_ARMOR_BONUS
            | MERCURIAL_SHIELD_BONUS
    )
}

/// A holder about to go on: a bonus whose parent is on starts full;
/// Ascendance holds six charges.
pub fn prepare(existing: &Auras, holder: &mut Holder, s: &SpellTemplate) {
    if holder.spell == ASCENDANCE {
        holder.charges = 6;
    }
    if let Some(parent) = parent_of(holder.spell)
        && full_stacks(holder.spell)
        && existing
            .holders
            .iter()
            .any(|h| h.spell == parent && h.caster == holder.caster)
    {
        holder.stacks = s.stack_amount.max(1);
    }
}

/// A holder went on: a parent that starts with its bonus gives it.
pub fn applied(map: &mut Map, target: Guid, spell: u32, caster: Guid) {
    if let Some(bonus) = initial(spell) {
        crate::game::map::spells::cast_triggered(map, caster, bonus, Some(target));
    }
}

/// A holder came off: a parent's bonus goes with it.
pub fn removed(map: &mut Map, target: Guid, h: &Holder) {
    let bonus = match h.spell {
        UNSTABLE_POWER => UNSTABLE_POWER_BONUS,
        ASCENDANCE => ASCENDANCE_BONUS,
        RESTLESS_STRENGTH => RESTLESS_STRENGTH_BONUS,
        BRITTLE_ARMOR => BRITTLE_ARMOR_BONUS,
        MERCURIAL_SHIELD => MERCURIAL_SHIELD_BONUS,
        _ => return,
    };
    auras::remove_spell(map, target, bonus, Some(h.caster));
}

/// Whether a spell `s` counts for Unstable Power: of the bonus's schools,
/// and doing damage or healing.
fn unstable_power_spell(map: &Map, h: &Holder, s: &SpellTemplate) -> bool {
    let schools = map
        .realm
        .data
        .spells
        .get(h.spell)
        .map(|b| {
            b.effects
                .iter()
                .filter(|e| e.aura == AURA_MOD_DAMAGE_DONE)
                .fold(0u32, |m, e| m | e.misc_value as u32)
        })
        .unwrap_or(0);
    let damage = s.effects.iter().any(|e| {
        matches!(e.effect, 1 | 2 | 7 | 9 | 10 | 17 | 31 | 58 | 62 | 121)
            || matches!(e.aura, 3 | 8 | 53 | 62)
    });
    schools & s.school_mask() != 0 && damage
}

/// A stacking trinket's proc, as Thistle Tea runs it. True when it is
/// handled here (the proc goes no further).
pub fn proc(map: &mut Map, owner: Guid, h: &Holder, e: &Event) -> bool {
    let data = map.realm.data.clone();
    let spell = e.spell.and_then(|id| data.spells.get(id));
    match h.spell {
        UNSTABLE_POWER_BONUS => {
            if spell.is_some_and(|s| unstable_power_spell(map, h, s)) {
                auras::remove_stack(map, owner, UNSTABLE_POWER_BONUS);
            }
            true
        }
        RESTLESS_STRENGTH => {
            auras::remove_stack(map, owner, RESTLESS_STRENGTH_BONUS);
            true
        }
        ASCENDANCE => {
            let skip = spell.is_some_and(|s| {
                s.effects
                    .iter()
                    .any(|x| matches!(x.target_a, 15 | 16 | 22 | 24 | 28))
                    || s.effects[0].effect == 77
            });
            if skip || spell.is_none() {
                return true;
            }
            if h.charges <= 1 {
                auras::remove_spell(map, owner, ASCENDANCE, Some(h.caster));
            } else {
                crate::game::map::spells::cast_triggered(map, owner, ASCENDANCE_BONUS, Some(owner));
                auras::spend_charge(map, owner, ASCENDANCE, h.caster);
            }
            true
        }
        _ => false,
    }
}

/// Whether `s` is one of the scripts a hit casts.
pub fn handles(s: &SpellTemplate) -> bool {
    matches!(s.id, BRITTLE_ARMOR_HIT | MERCURIAL_SHIELD_HIT)
}

/// A hit's script: a stack off its bonus.
pub fn effect(map: &mut Map, caster: Guid, s: &SpellTemplate) {
    let bonus = match s.id {
        BRITTLE_ARMOR_HIT => BRITTLE_ARMOR_BONUS,
        MERCURIAL_SHIELD_HIT => MERCURIAL_SHIELD_BONUS,
        _ => return,
    };
    auras::remove_stack(map, caster, bonus);
}
