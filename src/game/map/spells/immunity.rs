//! Immunity that purges what it guards against, as Thistle Tea's
//! `Spell.Immunity` has it for a spell marked so (vmangos's
//! `SPELL_ATTR_EX_DISPEL_AURAS_ON_IMMUNITY`): a school, mechanic or dispel
//! immunity going on takes the harmful auras it would have kept off
//! (Blessing of Freedom's roots and snares, Will of the Forsaken's fear).

use wow_world_messages::Guid;

use crate::game::map::Map;
use crate::game::map::auras::{self, Holder};

/// `SPELL_ATTR_EX_DISPEL_AURAS_ON_IMMUNITY`.
const ATTR_EX_PURGES: u32 = 0x8000;

/// Holder `h` went on `target`: what its immunities guard against goes.
pub fn applied(map: &mut Map, target: Guid, h: &Holder) {
    let data = map.realm.data.clone();
    let Some(s) = data.spells.get(h.spell) else {
        return;
    };
    if !s.attr(1, ATTR_EX_PURGES) {
        return;
    }
    let mut schools = 0u32;
    let mut mechanics = 0u64;
    let mut dispels = Vec::new();
    for x in &h.effects {
        match x.kind {
            auras::AURA_SCHOOL_IMMUNITY => schools |= x.misc as u32,
            auras::AURA_MECHANIC_IMMUNITY if x.misc > 0 => {
                mechanics |= 1 << (x.misc as u64 - 1).min(63)
            }
            auras::AURA_MECHANIC_IMMUNITY_MASK => mechanics |= u64::from(x.misc as u32),
            auras::AURA_DISPEL_IMMUNITY if x.misc > 0 => dispels.push(x.misc as u32),
            _ => {}
        }
    }
    if schools == 0 && mechanics == 0 && dispels.is_empty() {
        return;
    }
    let spell = h.spell;
    auras::remove_where(map, target, |o| {
        if o.spell == spell || o.positive || o.passive {
            return false;
        }
        let mechanic = |m: u32| m > 0 && mechanics & (1 << (m - 1).min(63)) != 0;
        let mechanic_hit = mechanic(o.mechanic)
            || data.spells.get(o.spell).is_some_and(|t| {
                t.effects
                    .iter()
                    .any(|e| e.effect != 0 && mechanic(e.mechanic))
            });
        o.school & schools != 0 || mechanic_hit || dispels.contains(&o.dispel)
    });
}
