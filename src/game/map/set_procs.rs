//! The tier sets' and relics' procs mangos-classic does in code
//! (`HandleDummyAuraProc`, `HandleProcTriggerSpellAuraProc` and the Vanish
//! script), the data saying only a dummy aura or a placeholder trigger:
//! Garments of the Oracle's heal, Vestments of Faith's Armor of Faith,
//! Dreamwalker's and the Earthfury's mana back, the Idol of Longevity and
//! the Totem of Flowing Water, The Ten Storms' Lightning Shield, Nightslayer's
//! Clean Escape and Cheat Death's low-health check.

use wow_world_messages::Guid;

use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::{self, Holder};
use crate::game::map::proc_scripts::Outcome;
use crate::game::map::procs::Event;
use crate::game::map::spells::{cast_triggered, scripts::cast_with_points};

/// Garments of the Oracle: a heal heals the priest a tenth of it.
pub const ORACLE: u32 = 26169;
pub const ORACLE_HEAL: u32 = 26170;
/// Vestments of Faith: a Greater Heal's crit gives its target Armor of Faith.
pub const VESTMENTS_OF_FAITH: u32 = 28809;
pub const ARMOR_OF_FAITH: u32 = 28810;
/// Dreamwalker Raiment: a Healing Touch's crit gives back 30% of its cost.
pub const DREAMWALKER: u32 = 28719;
pub const DREAMWALKER_MANA: u32 = 28742;
/// Idol of Longevity: Healing Touch gives mana back.
pub const IDOL_OF_LONGEVITY: u32 = 28847;
pub const IDOL_OF_LONGEVITY_MANA: u32 = 28848;
/// Totem of Flowing Water: Lesser Healing Wave gives mana back.
pub const TOTEM_OF_FLOWING_WATER: u32 = 28849;
pub const TOTEM_OF_FLOWING_WATER_MANA: u32 = 28850;
/// The Ten Storms: Healing Wave and Lesser Healing Wave may give the healed
/// a Lightning Shield, whose charges strike their attackers.
pub const TEN_STORMS: u32 = 23551;
pub const TEN_STORMS_SHIELD: u32 = 23552;
pub const TEN_STORMS_STRIKE: u32 = 27635;
/// The Earthfury: Healing Wave and Lesser Healing Wave may give back 35% of
/// their cost.
pub const MANA_SURGE: u32 = 23572;
pub const MANA_SURGE_MANA: u32 = 23571;
/// Nightslayer: Vanish heals the rogue.
pub const CLEAN_ESCAPE: u32 = 23582;
pub const CLEAN_ESCAPE_HEAL: u32 = 23583;
/// Cheat Death: more healing taken once a blow takes its bearer below 20%.
pub const CHEAT_DEATH: u32 = 28845;

/// The triggering spell's base mana cost (mangos's `spellInfo->manaCost`).
fn base_cost(map: &Map, e: &Event) -> Option<u32> {
    e.spell
        .and_then(|s| map.realm.data.spells.get(s))
        .map(|s| s.mana_cost)
}

/// A set's or relic's proc of `h` (on `owner`, against `other`), if it is
/// one: what came of it. `None` for any other holder.
pub fn fire(map: &mut Map, owner: Guid, other: Guid, h: &Holder, e: &Event) -> Option<Outcome> {
    match h.spell {
        ORACLE => {
            let heal = (e.damage * 10 / 100) as i32;
            if heal > 0 {
                cast_with_points(map, owner, ORACLE_HEAL, owner, [Some(heal), None, None]);
            }
            Some(Outcome::Done)
        }
        // On the healed (mangos's `pVictim`).
        VESTMENTS_OF_FAITH => {
            cast_triggered(map, owner, ARMOR_OF_FAITH, Some(other));
            Some(Outcome::Done)
        }
        DREAMWALKER => {
            let back = base_cost(map, e).unwrap_or(0) * 30 / 100;
            if back > 0 {
                cast_with_points(
                    map,
                    owner,
                    DREAMWALKER_MANA,
                    owner,
                    [Some(back as i32), None, None],
                );
            }
            Some(Outcome::Done)
        }
        IDOL_OF_LONGEVITY => {
            cast_triggered(map, owner, IDOL_OF_LONGEVITY_MANA, Some(owner));
            Some(Outcome::Done)
        }
        TOTEM_OF_FLOWING_WATER => {
            cast_triggered(map, owner, TOTEM_OF_FLOWING_WATER_MANA, Some(owner));
            Some(Outcome::Done)
        }
        // The shield on the one healed (mangos's `pVictim`).
        TEN_STORMS => {
            cast_triggered(map, owner, TEN_STORMS_SHIELD, Some(other));
            Some(Outcome::Done)
        }
        // Its charges strike the attacker (its trigger, 26545, a dummy).
        TEN_STORMS_SHIELD => {
            cast_triggered(map, owner, TEN_STORMS_STRIKE, Some(other));
            Some(Outcome::Done)
        }
        MANA_SURGE => {
            let back = base_cost(map, e).unwrap_or(0) * 35 / 100;
            if back > 0 {
                cast_with_points(
                    map,
                    owner,
                    MANA_SURGE_MANA,
                    owner,
                    [Some(back as i32), None, None],
                );
            }
            Some(Outcome::Done)
        }
        // Only the blow that takes the bearer from 20% or more below it, and
        // leaves them alive (mangos's "when your health drops below 20%",
        // its health read before the blow): else nothing, and no charge.
        CHEAT_DEATH => {
            let (health, max) = map.unit_fields(owner).map_or((0, 0), |f| {
                (
                    f.get_u32(UNIT_FIELD_HEALTH),
                    f.get_u32(UNIT_FIELD_MAXHEALTH),
                )
            });
            let fifth = max / 5;
            let before = health + e.damage;
            if health == 0 || health >= fifth || before < fifth {
                return Some(Outcome::DoneOwnCharges);
            }
            // Its own trigger, as the data has it.
            None
        }
        _ => None,
    }
}

/// After a rogue's Vanish: Nightslayer's heal (mangos-classic's
/// `VanishRogue`, on the set's dummy aura).
pub fn vanished(map: &mut Map, rogue: Guid) {
    if auras::auras(map, rogue).is_some_and(|a| a.has_spell(CLEAN_ESCAPE)) {
        cast_triggered(map, rogue, CLEAN_ESCAPE_HEAL, Some(rogue));
    }
}
