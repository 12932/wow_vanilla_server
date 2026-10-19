//! Round 7, against Thistle Tea's smaller casting checks: no casting on a
//! flight (`mount_test.exs`), a duel only with a player
//! (`duel_cast_validation_test.exs`), a caster-centred area with no ground
//! target (`casting_test.exs`'s Consecration), and nothing resisted of
//! nothing (`spell_resist_test.exs`).

use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh};
use crate::game::data::GameData;
use crate::game::data::spells::SpellEffect;
use crate::game::map::auras::*;
use crate::game::map::spells::{self, Targets, checks, checks::fail, mods::Mods, outcome};

/// A mount; Duel's kind; Consecration's kind (an area of eight yards at
/// its caster's place); a shadow bolt.
const MOUNT: u32 = 997_601;
const DUEL: u32 = 997_602;
const CONSECRATE: u32 = 997_603;
const SHADOW_BOLT: u32 = 997_604;
/// Sap's kind (for the peaceful only) and a stealth.
const SAP: u32 = 997_605;
const STEALTH: u32 = 997_606;
const RADIUS_8: u32 = 996;

fn data(d: &mut GameData) {
    d.spells.radii.insert(RADIUS_8, 8.0);
    let mut mount = bare(MOUNT, 0);
    mount.duration_index = LASTS_LONG;
    effect(&mut mount, 0, aura(AURA_MOUNTED, 0, 0, 1));
    let mut duel = bare(DUEL, 0);
    effect(
        &mut duel,
        0,
        SpellEffect {
            effect: spells::EFFECT_DUEL,
            target_a: 25,
            ..Default::default()
        },
    );
    let mut consecrate = bare(CONSECRATE, 1);
    consecrate.duration_index = LASTS_5S;
    effect(
        &mut consecrate,
        0,
        SpellEffect {
            effect: spells::EFFECT_PERSISTENT_AREA_AURA,
            aura: AURA_PERIODIC_DAMAGE,
            base_points: 7,
            base_dice: 1,
            amplitude: 1_000,
            radius_index: RADIUS_8,
            target_a: 18,
            target_b: 16,
            ..Default::default()
        },
    );
    let mut bolt = bare(SHADOW_BOLT, 5);
    effect(&mut bolt, 0, damage(100));
    let mut sap = bare(SAP, 0);
    sap.attributes[1] |= crate::game::data::spells::ATTR_EX_ONLY_PEACEFUL_TARGETS;
    sap.duration_index = LASTS_5S;
    effect(&mut sap, 0, aura(AURA_MOD_STUN, 0, 0, 6));
    let mut stealth = bare(STEALTH, 0);
    stealth.duration_index = LASTS_LONG;
    stealth.aura_interrupt_flags = INTERRUPT_CAST;
    effect(&mut stealth, 0, aura(AURA_MOD_STEALTH, 0, 5, 1));
    insert(d, [mount, duel, consecrate, bolt, sap, stealth]);
}

/// A player on a flight casts nothing, a mount neither (Thistle Tea's
/// "rejects casts during taxi flights"; mangos's `SPELL_FAILED_NOT_ON_TAXI`);
/// a duel is a player's, not a creature's (its "rejects non-player ...
/// targets").
#[tokio::test]
async fn nothing_is_cast_on_a_flight_and_a_duel_is_only_a_players() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P7CK1", "Flier", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[MOUNT, DUEL]);
            let kobold = kit::dummy(m, a, 60, 3.0);
            let check = |m: &mut crate::game::map::Map, spell: u32, t: &Targets| {
                let s = m.realm.data.spells.get(spell).unwrap().clone();
                checks::check(m, a, &s, t, None, false, &Mods::default())
            };
            let duel = check(m, DUEL, &at(kobold));
            let grounded = check(m, MOUNT, &Targets::default());
            let pos = m.players[&a].position();
            let far = Vector3d {
                x: pos.x + 1000.0,
                ..pos
            };
            m.players.get_mut(&a).unwrap().flight = Some(crate::game::map::npc::Flight {
                walk: crate::game::map::creature::Walk::new(vec![pos, far], 1.0, m.now, 0, true),
                destination: 0,
                mount: 0,
            });
            (duel, grounded, check(m, MOUNT, &Targets::default()))
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        (Some(fail::BAD_TARGETS), None, Some(fail::NOT_ON_TAXI))
    );
}

/// Consecration's kind, cast with no ground target, is at its caster's
/// place, and burns what is there (Thistle Tea's "caster-centered
/// persistent aura uses the caster position without a ground target";
/// mangos's `TARGET_LOCATION_CASTER_DEST`).
#[tokio::test]
async fn a_caster_centred_area_is_where_its_caster_stands() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P7CK2", "Consecrator", Class::Paladin).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[CONSECRATE]);
            let kobold = kit::dummy(m, a, 60, 4.0);
            let me = m.players[&a].position();
            cast_fresh(m, a, CONSECRATE, Targets::default());
            let area = m
                .spells
                .areas
                .iter()
                .find(|x| x.spell == CONSECRATE)
                .map(|x| (x.at, x.radius));
            for _ in 0..2 {
                kit::pass(m, 1_000);
            }
            (me, area, 1_000_000 - kit::health(m, kobold))
        })
        .await
        .unwrap();
    let (me, area, burnt) = out;
    let (at, radius) = area.expect("an area");
    assert!(
        crate::game::map::combat::dist(me, at) < 0.01,
        "{me:?} {at:?}"
    );
    assert_eq!(radius, 8.0);
    assert!(burnt >= 8, "it burns: {burnt}");
}

/// Of no damage nothing is resisted, its outcomes rolled or not (Thistle
/// Tea's "returns zero for zero damage").
#[tokio::test]
async fn nothing_is_resisted_of_nothing() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P7CK3", "Resisted", Class::Warlock).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 10, &[]);
            let kobold = kit::dummy(m, a, 60, 3.0);
            m.spells.rolls = Some(true);
            let s = m.realm.data.spells.get(SHADOW_BOLT).unwrap().clone();
            (0..50)
                .map(|_| outcome::partial_resist(m, a, kobold, &s, 0, false))
                .max()
        })
        .await
        .unwrap();
    assert_eq!(out, Some(0));
}

/// Sap's kind refuses a target in a fight before it costs its caster
/// their stealth, and lands on one out of it (mangos-classic's `CheckCast`:
/// `SPELL_ATTR_EX_ONLY_PEACEFUL_TARGETS`, `TARGET_AFFECTING_COMBAT`;
/// Thistle Tea's "peaceful-target requirements reject combat before
/// consuming stealth").
#[tokio::test]
async fn sap_refuses_one_in_a_fight_before_it_costs_stealth() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P7CK4", "Sapper", Class::Rogue).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[SAP]);
            let kobold = kit::dummy(m, a, 60, 3.0);
            kit::apply(m, a, a, STEALTH);
            if let Some(f) = m.unit_fields_mut(kobold) {
                f.set_flag(
                    crate::game::fields::UNIT_FIELD_FLAGS,
                    crate::game::map::player::UNIT_FLAG_IN_COMBAT,
                    true,
                );
            }
            let s = m.realm.data.spells.get(SAP).unwrap().clone();
            let fighting = checks::check(m, a, &s, &at(kobold), None, false, &Mods::default());
            let kept = kit::has(m, a, STEALTH);
            if let Some(f) = m.unit_fields_mut(kobold) {
                f.set_flag(
                    crate::game::fields::UNIT_FIELD_FLAGS,
                    crate::game::map::player::UNIT_FLAG_IN_COMBAT,
                    false,
                );
            }
            cast_fresh(m, a, SAP, at(kobold));
            (fighting, kept, kit::has(m, kobold, SAP))
        })
        .await
        .unwrap();
    assert_eq!(out, (Some(fail::TARGET_AFFECTING_COMBAT), true, true));
}
