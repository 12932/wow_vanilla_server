//! Phase 3's combat tests, part nine (Thistle Tea's
//! `attack_damage_taken_test.exs` and `elemental_combat_test.exs`): what a
//! victim's melee and ranged damage-taken auras make of a blow, before
//! armor and crits; and an elemental's swings, of its school.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::{self, *};
use crate::game::map::combat::{self, Hand, Outcome};
use crate::game::tests_p3_combat::{self as base, *};

/// A fire elemental of level 20.
const ELEMENTAL: u32 = 3730;

fn data() -> GameData {
    let mut d = base::data();
    let mut t = template(ELEMENTAL, 4);
    t.damage_school = 2;
    d.creature_templates.insert(ELEMENTAL, t);
    d
}

/// A blow of `attacker`'s `hand` at `victim`, its damage before armor.
fn raw(m: &Map, attacker: Guid, victim: Guid, hand: Hand) -> (u32, u32) {
    combat::weapon_damage(m, attacker, victim, hand, &mut rand::rng())
}

/// `g` hits for exactly `damage` with each hand.
fn hits_for(m: &mut Map, g: Guid, damage: f32) {
    let f = m.unit_fields_mut(g).unwrap();
    for (lo, hi) in [
        (UNIT_FIELD_MINDAMAGE, UNIT_FIELD_MAXDAMAGE),
        (UNIT_FIELD_MINRANGEDDAMAGE, UNIT_FIELD_MAXRANGEDDAMAGE),
    ] {
        f.set_f32(lo, damage);
        f.set_f32(hi, damage);
    }
}

/// An aura of `kind` and `misc` holding `amount`, `stacks` deep.
fn stacked(m: &mut Map, g: Guid, id: u32, kind: u32, misc: i32, amount: i32, stacks: u32) {
    aura_of(m, g, id, kind, misc, amount);
    if let Some(a) = auras::auras_mut(m, g)
        && let Some(h) = a.holders.iter_mut().find(|h| h.spell == id)
    {
        h.stacks = stacks;
    }
}

#[tokio::test]
async fn melee_and_ranged_damage_taken_add_their_flats_then_multiply_their_shares() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 1.0);
            let mut out = Vec::new();
            // Ten less twice for blows, thirty more for shots.
            stacked(m, ga, 90_300, AURA_MOD_MELEE_DAMAGE_TAKEN, 0, -10, 2);
            stacked(m, ga, 90_301, AURA_MOD_RANGED_DAMAGE_TAKEN, 0, 30, 1);
            for (damage, hand) in [
                (100.0, Hand::Main),
                (100.0, Hand::Ranged),
                (10.0, Hand::Main),
            ] {
                hits_for(m, brute, damage);
                out.push(raw(m, brute, ga, hand).0);
            }
            auras::remove_spell(m, ga, 90_300, None);
            auras::remove_spell(m, ga, 90_301, None);
            // Twenty less, then a fifth less and half more: whole again;
            // and all of a shot off.
            stacked(m, ga, 90_302, AURA_MOD_MELEE_DAMAGE_TAKEN, 0, -20, 1);
            stacked(m, ga, 90_303, AURA_MOD_MELEE_DAMAGE_TAKEN_PCT, 0, -10, 2);
            stacked(m, ga, 90_304, AURA_MOD_MELEE_DAMAGE_TAKEN_PCT, 0, 50, 1);
            stacked(m, ga, 90_305, AURA_MOD_RANGED_DAMAGE_TAKEN_PCT, 0, -100, 1);
            hits_for(m, brute, 120.0);
            out.push(raw(m, brute, ga, Hand::Main).0);
            out.push(raw(m, brute, ga, Hand::Ranged).0);
            out
        })
        .await
        .unwrap();
    assert_eq!(r, vec![80, 130, 0, 120, 0]);
}

#[tokio::test]
async fn damage_taken_comes_off_before_armor_and_the_crit_and_none_stays_none() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 1.0);
            let health = |m: &Map| m.unit_fields(ga).unwrap().get_u32(UNIT_FIELD_HEALTH);
            let full = |m: &mut Map| {
                let f = m.unit_fields_mut(ga).unwrap();
                f.set_u32(UNIT_FIELD_MAXHEALTH, 10_000);
                f.set_u32(UNIT_FIELD_HEALTH, 10_000);
            };
            aura_of(m, ga, 90_310, AURA_MOD_MELEE_DAMAGE_TAKEN, 0, -30);
            m.unit_fields_mut(ga)
                .unwrap()
                .set_u32(UNIT_FIELD_RESISTANCES, 3_000);
            hits_for(m, brute, 100.0);
            full(m);
            forced_swing(m, brute, ga, Outcome::Crit);
            let crit = 10_000 - health(m);
            let expected = combat::armor_mitigate(m, brute, ga, 70) * 2;
            // All of it off: nothing, crit or crushing.
            aura_of(m, ga, 90_311, AURA_MOD_MELEE_DAMAGE_TAKEN, 0, -1000);
            let mut none = Vec::new();
            for o in [
                Outcome::Hit,
                Outcome::Crit,
                Outcome::Crushing,
                Outcome::Glancing,
            ] {
                full(m);
                forced_swing(m, brute, ga, o);
                none.push(10_000 - health(m));
            }
            (crit, expected, none)
        })
        .await
        .unwrap();
    assert_eq!(r.0, r.1);
    assert_eq!(r.2, vec![0; 4]);
}

#[tokio::test]
async fn an_elementals_swings_are_of_its_school_take_its_schools_bonuses_and_skip_armor() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let fire = spawn(m, ga, ELEMENTAL, 20, 1.0);
            let brute = spawn(m, ga, BRUTE, 20, 1.0);
            let schools = (
                raw(m, fire, ga, Hand::Main).1,
                raw(m, fire, ga, Hand::Off).1,
                raw(m, brute, ga, Hand::Main).1,
            );
            // Twenty more fire, half again of fire, a hundred more
            // physical: (100 + 20) * 1.5.
            aura_of(m, fire, 90_320, AURA_MOD_DAMAGE_DONE, 4, 20);
            aura_of(m, fire, 90_321, AURA_MOD_DAMAGE_PERCENT_DONE, 4, 50);
            aura_of(m, fire, 90_322, AURA_MOD_DAMAGE_DONE, 1, 100);
            hits_for(m, fire, 100.0);
            let bonused = raw(m, fire, ga, Hand::Main).0;
            // Armor stops only the physical.
            m.unit_fields_mut(ga)
                .unwrap()
                .set_u32(UNIT_FIELD_RESISTANCES, 3_000);
            let armored = (
                combat::mitigate(m, fire, ga, 100, 4),
                combat::mitigate(m, brute, ga, 100, 1),
            );
            (schools, bonused, armored)
        })
        .await
        .unwrap();
    assert_eq!(r.0, (4, 4, 1));
    assert_eq!(r.1, 180);
    assert_eq!(r.2.0, 100);
    assert!(r.2.1 < 100);
}

#[tokio::test]
async fn an_elementals_swing_passes_other_wards_and_its_schools_immunity_stops_it_whole() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let fire = spawn(m, ga, ELEMENTAL, 20, 1.0);
            hits_for(m, fire, 100.0);
            let health = |m: &Map| m.unit_fields(ga).unwrap().get_u32(UNIT_FIELD_HEALTH);
            let full = |m: &mut Map| {
                let f = m.unit_fields_mut(ga).unwrap();
                f.set_u32(UNIT_FIELD_MAXHEALTH, 1_000);
                f.set_u32(UNIT_FIELD_HEALTH, 1_000);
            };
            // A nature ward: no help against fire.
            aura_of(m, ga, 90_330, AURA_SCHOOL_ABSORB, 8, 500);
            full(m);
            forced_swing(m, fire, ga, Outcome::Hit);
            let warded = (
                1_000 - health(m),
                auras::auras(m, ga)
                    .unwrap()
                    .holders
                    .iter()
                    .find(|h| h.spell == 90_330)
                    .map(|h| h.effects[0].left),
            );
            // Immune to fire, with a fire ward too: nothing, the ward
            // untouched.
            aura_of(m, ga, 90_331, AURA_SCHOOL_ABSORB, 4, 500);
            aura_of(m, ga, 90_332, AURA_SCHOOL_IMMUNITY, 4, 1);
            full(m);
            forced_swing(m, fire, ga, Outcome::Hit);
            let immune = (
                1_000 - health(m),
                auras::auras(m, ga)
                    .unwrap()
                    .holders
                    .iter()
                    .find(|h| h.spell == 90_331)
                    .map(|h| h.effects[0].left),
            );
            (warded, immune)
        })
        .await
        .unwrap();
    assert_eq!(r.0, (100, Some(500)));
    assert_eq!(r.1, (0, Some(500)));
}
