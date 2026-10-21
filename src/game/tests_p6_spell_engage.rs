//! Phase 3, round 6, against Thistle Tea's `combat_test.exs`: when a
//! harmful spell puts the creature it got to in the fight with its caster.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh};
use crate::game::data::GameData;
use crate::game::data::spells::SpellTemplate;
use crate::game::map::Map;
use crate::game::map::auras::{
    AURA_MOD_DECREASE_SPEED, AURA_MOD_STEALTH, AURA_MOD_STUN, AURA_REFLECT_SPELLS,
    AURA_SCHOOL_IMMUNITY,
};

/// Harmful auras without damage: a slow, one whose failure breaks stealth
/// and which makes no threat, one of no threat as it lands, a Sap; and on
/// the creature, an immunity to everything and a reflection; on the
/// caster, a stealth no creature sees through.
const SLOW: u32 = 991_020;
const LOUD: u32 = 991_021;
const QUIET: u32 = 991_022;
const SAP: u32 = 991_023;
const WARD: u32 = 991_024;
const MIRROR: u32 = 991_025;
const HIDDEN: u32 = 991_026;
/// The slow as a missile, and a spell of threat outright.
const SLOW_MISSILE: u32 = 991_027;
const TAUNTING: u32 = 991_028;

fn data(d: &mut GameData) {
    let harmful = |id: u32, kind: u32| {
        let mut s = bare(id, 6);
        s.duration_index = LASTS_5S;
        effect(&mut s, 0, aura(kind, 0, -50, 6));
        s
    };
    let mut loud = harmful(LOUD, AURA_MOD_DECREASE_SPEED);
    loud.attributes[1] |= 0x1000 | 0x400;
    let mut quiet = harmful(QUIET, AURA_MOD_DECREASE_SPEED);
    quiet.attributes[2] |= 0x40_0000;
    let mut sap = harmful(SAP, AURA_MOD_STUN);
    sap.school = 0;
    sap.family_name = 8;
    sap.family_flags = 0x80;
    let lasting = |id: u32, kind: u32, misc: i32, amount: i32| {
        let mut s = bare(id, 0);
        s.duration_index = LASTS_LONG;
        effect(&mut s, 0, aura(kind, misc, amount, 1));
        s
    };
    let mut missile = harmful(SLOW_MISSILE, AURA_MOD_DECREASE_SPEED);
    missile.speed = 20.0;
    let mut taunting = bare(TAUNTING, 6);
    effect(
        &mut taunting,
        0,
        crate::game::data::spells::SpellEffect {
            effect: crate::game::map::spells::EFFECT_THREAT,
            base_points: 9,
            base_dice: 1,
            target_a: 6,
            ..Default::default()
        },
    );
    let spells: Vec<SpellTemplate> = vec![
        harmful(SLOW, AURA_MOD_DECREASE_SPEED),
        missile,
        taunting,
        loud,
        quiet,
        sap,
        lasting(WARD, AURA_SCHOOL_IMMUNITY, 127, 0),
        lasting(MIRROR, AURA_REFLECT_SPELLS, 0, 100),
        lasting(HIDDEN, AURA_MOD_STEALTH, 0, 1_000),
    ];
    insert(d, spells);
}

/// `spell` cast by `a` at a fresh kobold, warded or mirrored as `on` has
/// it, `a` hidden or not: whether the kobold is then in the fight with `a`.
fn engages(m: &mut Map, a: Guid, spell: u32, on: Option<u32>, hidden: bool) -> bool {
    engages_as(m, a, spell, on, hidden, false)
}

fn engages_as(
    m: &mut Map,
    a: Guid,
    spell: u32,
    on: Option<u32>,
    hidden: bool,
    triggered: bool,
) -> bool {
    crate::game::map::auras::remove_spell(m, a, HIDDEN, None);
    let kobold = kit::dummy(m, a, 60, 5.0);
    if let Some(x) = on {
        kit::apply(m, kobold, kobold, x);
    }
    if hidden {
        kit::apply(m, a, a, HIDDEN);
    }
    if triggered {
        crate::game::map::spells::cast_by_aura(m, a, spell, Some(kobold));
    } else {
        cast_fresh(m, a, spell, at(kobold));
    }
    m.creature(kobold)
        .is_some_and(|c| c.threat.iter().any(|(g, _)| *g == a))
}

/// A harmful spell that lands puts its creature in the fight if it saw
/// its caster; one kept off (immune) does too, if it saw them, and not if
/// the caster is hidden from it; one reflected doesn't. A spell whose
/// failure breaks stealth fights from hiding, though it makes no threat.
/// A Sap that lands keeps the peace, one kept off provokes a creature that
/// saw it. One of no threat as it lands doesn't fight on landing but does
/// when kept off. A triggered one doesn't fight, landed or kept off, but
/// a missile kept off and a spell of threat outright landing do (Thistle
/// Tea's `Spell.Combat.decide`).
#[tokio::test]
async fn a_harmful_spell_engages_a_creature_that_saw_it_by_its_outcome() {
    let realm = fast_realm(data);
    let (_c, a, map) = kit::player(&realm, "P6GA", "Engager", Race::Human, Class::Rogue).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, a, 60, &[SLOW, LOUD, QUIET, SAP]);
            let plain = (
                engages(m, a, SLOW, None, false),
                engages(m, a, SLOW, Some(WARD), false),
                engages(m, a, SLOW, Some(WARD), true),
                engages(m, a, SLOW, Some(MIRROR), false),
            );
            let loud = engages(m, a, LOUD, Some(WARD), true);
            let sap = (
                engages(m, a, SAP, None, false),
                engages(m, a, SAP, Some(WARD), true),
                engages(m, a, SAP, Some(WARD), false),
            );
            let quiet = (
                engages(m, a, QUIET, None, false),
                engages(m, a, QUIET, Some(WARD), false),
            );
            let triggered = (
                engages_as(m, a, SLOW, None, false, true),
                engages_as(m, a, SLOW, Some(WARD), false, true),
                engages_as(m, a, SLOW_MISSILE, Some(WARD), false, true),
                engages_as(m, a, TAUNTING, None, false, true),
                engages_as(m, a, TAUNTING, Some(WARD), false, true),
            );
            (plain, loud, sap, quiet, triggered)
        })
        .await
        .unwrap();
    let (plain, loud, sap, quiet, triggered) = out;
    assert_eq!(
        triggered,
        (false, false, true, true, false),
        "triggered: landed, kept off, a missile kept off, threat landed and kept off"
    );
    assert_eq!(
        plain,
        (true, true, false, false),
        "landed, immune seen, immune hidden, reflected"
    );
    assert!(loud, "a failure that breaks stealth");
    assert_eq!(
        sap,
        (false, false, true),
        "Sap: landed, kept off unseen, seen"
    );
    assert_eq!(quiet, (false, true), "no threat as it lands");
}
