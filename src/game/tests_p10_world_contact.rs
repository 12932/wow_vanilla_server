//! Round 10's tests of a spell's contact, ported from Thistle Tea's
//! `spell_contact_test.exs`: an undetected immune Sap at a creature, an
//! undetected miss at a flagged player, and damage of no initial threat
//! that engages, a killing one still claiming the loot.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh};
use super::tests_p6_wspell::{off, put};
use crate::game::data::GameData;
use crate::game::data::spells::{ATTR_EX2_NO_INITIAL_THREAT, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::{AURA_MOD_STEALTH, AURA_MOD_STUN, AURA_SCHOOL_IMMUNITY};
use crate::game::map::player::{UNIT_FLAG_IN_COMBAT, UNIT_FLAG_PVP};
use crate::game::map::spells::stealth;

/// A Sap (the rogues' family, flag 0x80); a stealth no one sees through;
/// an immunity to every school; 20 fire of no initial threat.
const SAP: u32 = 997_600;
const HIDDEN: u32 = 997_601;
const WARD: u32 = 997_602;
const QUIET_FIRE: u32 = 997_603;
const DISPEL_STEALTH: u32 = 5;

fn data(d: &mut GameData) {
    let mut sap = bare(SAP, 0);
    sap.duration_index = LASTS_5S;
    sap.family_name = 8;
    sap.family_flags = 0x80;
    effect(&mut sap, 0, aura(AURA_MOD_STUN, 0, -50, 6));
    let lasting = |id: u32, kind: u32, misc: i32, amount: i32| {
        let mut s = bare(id, 0);
        s.duration_index = LASTS_LONG;
        effect(&mut s, 0, aura(kind, misc, amount, 1));
        s
    };
    let mut hidden = lasting(HIDDEN, AURA_MOD_STEALTH, 0, 1_000);
    hidden.dispel = DISPEL_STEALTH;
    let mut quiet: SpellTemplate = bare(QUIET_FIRE, 2);
    quiet.attributes[2] |= ATTR_EX2_NO_INITIAL_THREAT;
    effect(&mut quiet, 0, damage(20));
    insert(
        d,
        [
            sap,
            hidden,
            lasting(WARD, AURA_SCHOOL_IMMUNITY, 127, 0),
            quiet,
        ],
    );
}

fn fighting(m: &Map, g: Guid) -> bool {
    m.unit_fields(g)
        .is_some_and(|f| f.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT))
}

fn flagged(m: &Map, g: Guid) -> bool {
    m.unit_fields(g)
        .is_some_and(|f| f.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP))
}

/// Thistle Tea's "an undetected immune Sap leaves both owners peaceful"
/// and "an undetected miss neither flags players nor alerts the pet": a
/// hidden rogue's Sap kept off a warded kobold that doesn't see her puts
/// neither in the fight, gives the kobold no threat on her and leaves it
/// untapped; kept off a flagged player of the other side who doesn't see
/// her, it puts neither in the fight either, but flags her (mangos-classic's
/// `CasterHitTargetWithSpell` engages only one that detects the caster;
/// its `FillTargetMap` flags a player whose spell reaches a flagged
/// player's side, seen or not, where Thistle Tea flags no one).
#[tokio::test]
async fn an_unseen_sap_kept_off_engages_no_one() {
    let realm = fast_realm(data);
    let (_r, rogue, map) = kit::player(&realm, "P10WC1", "Unseen", Race::Human, Class::Rogue).await;
    let (_t, target, _) =
        kit::player(&realm, "P10WC2", "Unaware", Race::Undead, Class::Warrior).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, rogue, 60, &[SAP]);
            kit::ready(m, target, 60, &[]);
            kit::apply(m, rogue, rogue, HIDDEN);
            // A creature.
            let kobold = kit::dummy(m, rogue, 60, 5.0);
            kit::apply(m, kobold, kobold, WARD);
            let unseen = !stealth::sees(m, kobold, rogue);
            let cast = cast_fresh(m, rogue, SAP, at(kobold));
            let c = m.creature(kobold).unwrap();
            let creature = (
                (unseen, cast, kit::has(m, kobold, SAP)),
                (fighting(m, rogue), fighting(m, kobold)),
                c.threat.is_empty(),
                c.tapped_by,
            );
            m.remove_creature(kobold);
            // A flagged player.
            let to = off(m, rogue, 3.0, 0.0);
            put(m, target, to);
            m.players.get_mut(&rogue).unwrap().movement.orientation = 0.0;
            m.players.get_mut(&target).unwrap().movement.orientation = std::f32::consts::PI;
            kit::apply(m, target, target, WARD);
            m.unit_fields_mut(target)
                .unwrap()
                .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP, true);
            let unseen = !stealth::sees(m, target, rogue);
            kit::fresh(m, rogue);
            let cast = cast_fresh(m, rogue, SAP, at(target));
            let player = (
                (unseen, cast, kit::has(m, target, SAP)),
                (fighting(m, rogue), fighting(m, target)),
                flagged(m, rogue),
            );
            (creature, player)
        })
        .await
        .unwrap();
    let (creature, player) = out;
    assert_eq!(creature.0, (true, true, false), "unseen, cast, kept off");
    assert_eq!(creature.1, (false, false), "neither in the fight");
    assert!(creature.2, "no threat");
    assert_eq!(creature.3, None, "untapped");
    assert_eq!(player.0, (true, true, false), "unseen, cast, kept off");
    assert_eq!(player.1, (false, false), "neither in the fight");
    assert!(player.2, "flagged, as mangos has it");
}

/// Thistle Tea's "no-initial-threat damage engages and lethal damage still
/// claims the loot": 20 fire of no initial threat at a kobold of 100
/// health taps it and puts it in the fight with its caster (the damage
/// does, mangos-classic's `DealDamage` with `SetInCombatWithAggressor`);
/// at a kobold of 1 health it kills it, tapped by its caster.
#[tokio::test]
async fn damage_of_no_initial_threat_engages_and_a_kill_is_its_casters() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P10WC3", "Quiet", Class::Mage).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, a, 60, &[QUIET_FIRE]);
            let mut out = Vec::new();
            for health in [100, 1] {
                let kobold = kit::dummy(m, a, 60, 5.0);
                kit::set_health(m, kobold, health);
                kit::fresh(m, a);
                assert!(cast_fresh(m, a, QUIET_FIRE, at(kobold)));
                let c = m.creature(kobold).unwrap();
                out.push((
                    c.alive(),
                    c.tapped_by == Some(a),
                    fighting(m, kobold) || !c.alive(),
                ));
            }
            out
        })
        .await
        .unwrap();
    assert_eq!(out[0], (true, true, true), "tapped, in the fight");
    assert_eq!(out[1], (false, true, true), "killed, the caster's");
}
