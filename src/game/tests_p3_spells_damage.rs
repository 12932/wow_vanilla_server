//! Phase 3's spell damage and healing, against the real data when it is
//! here, as Thistle Tea's tests of them have it: what a target's damage
//! taken does to a spell's hit and tick.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::map::spells::outcome::{self, Kind, Landing};
use crate::game::map::{Map, auras, spells};

const FIRE_BLAST: u32 = 2136;
const DAMPEN_MAGIC: u32 = 604;
const CORRUPTION: u32 = 172;
const SINISTER_STRIKE: u32 = 1752;

/// `s`'s direct damage of `base` from `caster` on `target`.
fn hit_of(m: &Map, caster: Guid, target: Guid, spell: u32, base: f32) -> u32 {
    let s = m.realm.data.spells.get(spell).cloned().unwrap();
    outcome::damage(m, caster, target, &s, 0, base, &Landing::default()).amount
}

/// A lasting damage taken aura of `amount` for `schools` on `g`.
fn taken(m: &mut Map, g: Guid, id: u32, schools: i32, amount: i32) {
    let mut s = made_up(id, 6, auras::AURA_MOD_DAMAGE_TAKEN, schools, amount);
    s.duration_index = 21;
    apply_template(m, g, g, &s);
}

#[tokio::test]
#[ignore = "extended"]
async fn a_spells_flat_damage_taken_goes_by_its_coefficient_and_takes_at_most_half() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, mage, map) = player(&realm, "P3D1", "Takenone", Race::Human, Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            let kobold = dummy(m, mage, 60, 5.0);
            let data = m.realm.data.clone();
            let fire_blast = data.spells.get(FIRE_BLAST).cloned().unwrap();
            let direct = outcome::coefficient(m, &fire_blast, 0, Kind::Direct);
            let plain = hit_of(m, mage, kobold, FIRE_BLAST, 100.0);
            // Forty more fire taken: Fire Blast's share of it.
            taken(m, kobold, 990_801, 4, 40);
            let more = hit_of(m, mage, kobold, FIRE_BLAST, 100.0);
            // An ability's is whole.
            taken(m, kobold, 990_802, 1, 40);
            let strike = hit_of(m, mage, kobold, SINISTER_STRIKE, 100.0);
            auras::remove_spell(m, kobold, 990_801, None);
            auras::remove_spell(m, kobold, 990_802, None);
            // Dampen Magic's ten less, by the coefficient.
            apply(m, kobold, kobold, DAMPEN_MAGIC);
            let dampen =
                auras::auras(m, kobold).map_or(0, |a| a.sum_mask(auras::AURA_MOD_DAMAGE_TAKEN, 4));
            let dampened = hit_of(m, mage, kobold, FIRE_BLAST, 100.0);
            auras::remove_spell(m, kobold, DAMPEN_MAGIC, None);
            // A thousand less takes only half.
            taken(m, kobold, 990_803, 4, -1000);
            let halved = hit_of(m, mage, kobold, FIRE_BLAST, 60.0);
            auras::remove_spell(m, kobold, 990_803, None);
            // A tick takes its share by the tick's coefficient.
            let corruption = data.spells.get(CORRUPTION).cloned().unwrap();
            let dot = outcome::coefficient(m, &corruption, 0, Kind::Dot);
            crate::game::map::spells::cast_triggered(m, mage, CORRUPTION, Some(kobold));
            let tick = holder(m, kobold, CORRUPTION)
                .and_then(|h| h.effects.first().map(|x| x.amount))
                .unwrap_or(0);
            taken(m, kobold, 990_804, 32, 40);
            let before = health(m, kobold);
            tick_now(m, kobold, CORRUPTION);
            let ticked = before - health(m, kobold);
            (
                direct, plain, more, strike, dampen, dampened, halved, dot, tick, ticked,
            )
        })
        .await
        .unwrap();
    let (direct, plain, more, strike, dampen, dampened, halved, dot, tick, ticked) = out;
    assert_eq!(plain, 100);
    assert!(
        direct > 0.1 && direct < 0.5,
        "Fire Blast's coefficient: {direct}"
    );
    assert_eq!(
        more,
        (100.0 + 40.0 * direct) as u32,
        "forty by the coefficient"
    );
    assert_eq!(strike, 140, "an ability takes it whole");
    assert!(dampen < 0, "Dampen Magic takes some off");
    assert_eq!(
        dampened,
        (100.0 + dampen as f32 * direct) as u32,
        "by the coefficient"
    );
    assert_eq!(halved, 30, "at most half");
    assert!(tick > 0 && dot > 0.0);
    assert_eq!(
        ticked,
        (tick as f32 + 40.0 * dot) as u32,
        "a tick by its coefficient"
    );
}

/// Made-up spells that ignore their caster's modifiers: a nature bolt, a
/// heal and a nature damage over time, each of 100.
fn unmodified_spells(d: &mut crate::game::data::GameData) {
    const IGNORE: u32 = crate::game::data::spells::ATTR_EX3_IGNORE_CASTER_MODIFIERS;
    let mut bolt = made_up(990_810, 2, 0, 0, 100);
    bolt.school = 3;
    bolt.dmg_class = 1;
    bolt.range_index = 4;
    bolt.effects[0].target_a = 6;
    bolt.attributes[3] = IGNORE;
    let mut heal = made_up(990_811, 10, 0, 0, 100);
    heal.school = 1;
    heal.dmg_class = 1;
    heal.range_index = 4;
    heal.effects[0].target_a = 21;
    heal.attributes[3] = IGNORE;
    let mut dot = made_up(990_812, 6, auras::AURA_PERIODIC_DAMAGE, 0, 100);
    dot.school = 3;
    dot.dmg_class = 1;
    dot.range_index = 4;
    dot.duration_index = 28;
    dot.effects[0].target_a = 6;
    dot.effects[0].amplitude = 1000;
    dot.attributes[3] = IGNORE;
    for s in [bolt, heal, dot] {
        d.spells.spells.insert(s.id, s);
    }
}

#[tokio::test]
#[ignore = "extended"]
async fn a_spell_that_ignores_its_casters_modifiers_takes_no_power_percent_or_damage_crit() {
    if !have_data() {
        return;
    }
    let realm = realm_plus(unmodified_spells);
    let (_c, mage, map) = player(&realm, "P3D2", "Plainone", Race::Human, Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            let kobold = dummy(m, mage, 60, 5.0);
            // Nature power and twice the damage, healing and twice it, and
            // every spell a crit.
            let lasting = |m: &mut Map, id: u32, kind: u32, misc: i32, amount: i32| {
                let mut s = made_up(id, 6, kind, misc, amount);
                s.duration_index = 21;
                apply_template(m, mage, mage, &s);
            };
            lasting(m, 990_820, auras::AURA_MOD_DAMAGE_DONE, 8, 500);
            lasting(m, 990_821, auras::AURA_MOD_DAMAGE_PERCENT_DONE, 8, 100);
            lasting(m, 990_822, auras::AURA_MOD_HEALING_DONE, -1, 1000);
            lasting(m, 990_823, auras::AURA_MOD_HEALING_DONE_PERCENT, 0, 100);
            lasting(m, 990_824, auras::AURA_MOD_SPELL_CRIT_CHANCE, 0, 100);
            let before = health(m, kobold);
            crate::game::map::spells::cast_triggered(m, mage, 990_810, Some(kobold));
            let bolt = before - health(m, kobold);
            set_health(m, mage, 100);
            crate::game::map::spells::cast_triggered(m, mage, 990_811, Some(mage));
            let healed = health(m, mage) - 100;
            crate::game::map::spells::cast_triggered(m, mage, 990_812, Some(kobold));
            let tick = holder(m, kobold, 990_812).and_then(|h| h.effects.first().map(|x| x.amount));
            (bolt, healed, tick)
        })
        .await
        .unwrap();
    let (bolt, healed, tick) = out;
    assert_eq!(bolt, 100, "no power, no percent, no crit");
    assert_eq!(healed, 150, "a heal's crit stays, without power or percent");
    assert_eq!(tick, Some(100), "a tick held without them");
}

const BLIZZARD: u32 = 10;
const RAIN_OF_FIRE: u32 = 5740;
const CONSECRATION: u32 = 26573;

/// The times (ms after the cast) `kobold` was hurt while `ms` pass in
/// steps of 50.
fn hurt_at(m: &mut Map, kobold: Guid, ms: u64) -> Vec<u64> {
    hurt_by(m, kobold, ms, 50)
}

/// The times `kobold` was hurt while `ms` pass in steps of `step`.
fn hurt_by(m: &mut Map, kobold: Guid, ms: u64, step: u64) -> Vec<u64> {
    let mut at = Vec::new();
    let mut gone = 0;
    while gone < ms {
        let before = health(m, kobold);
        pass(m, step);
        gone += step;
        if health(m, kobold) < before {
            at.push(gone);
        }
    }
    at
}

#[tokio::test]
#[ignore = "extended"]
async fn a_ground_spell_ticks_a_period_in_and_has_its_last_tick_as_it_ends() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, mage, map) = player(&realm, "P3D3", "Groundone", Race::Human, Class::Mage).await;
    let (_d, pally, _) = player(&realm, "P3D4", "Groundtwo", Race::Human, Class::Paladin).await;
    let (_e, lock, _) = player(&realm, "P3D5", "Groundthree", Race::Human, Class::Warlock).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[BLIZZARD]);
            ready(m, pally, 60, &[CONSECRATION]);
            ready(m, lock, 60, &[RAIN_OF_FIRE]);
            let kobold = dummy(m, mage, 60, 10.0);
            let at = m.position_of(kobold);
            let ground = |m: &mut Map, caster: Guid, spell: u32| {
                let t = crate::game::map::spells::Targets {
                    dest: at,
                    ..Default::default()
                };
                assert!(cast_fresh(m, caster, spell, t), "{spell} cast");
                hurt_at(m, kobold, 10_000)
            };
            let blizzard = ground(m, mage, BLIZZARD);
            // Updates 55 ms apart: still eight.
            let t = crate::game::map::spells::Targets {
                dest: at,
                ..Default::default()
            };
            assert!(cast_fresh(m, mage, BLIZZARD, t));
            let late = hurt_by(m, kobold, 10_000, 55).len();
            let rain = ground(m, lock, RAIN_OF_FIRE);
            // Consecration is about the paladin.
            let p = m.position_of(kobold).unwrap();
            if let Some(x) = m.players.get_mut(&pally) {
                x.movement.position = p;
            }
            assert!(cast_fresh(
                m,
                pally,
                CONSECRATION,
                crate::game::map::spells::Targets::default()
            ));
            let consecration = hurt_at(m, kobold, 10_000);
            (blizzard, late, rain, consecration)
        })
        .await
        .unwrap();
    let (blizzard, late, rain, consecration) = out;
    assert_eq!(late, 8, "eight on late updates too");
    assert_eq!(
        blizzard,
        (1..=8).map(|s| s * 1000).collect::<Vec<_>>(),
        "Blizzard's eight waves, a second apart"
    );
    assert_eq!(rain, vec![2000, 4000, 6000, 8000], "Rain of Fire's four");
    assert_eq!(
        consecration,
        (1..=8).map(|s| s * 1000).collect::<Vec<_>>(),
        "Consecration's eight, the first a second in"
    );
}

const DRAIN_LIFE: u32 = 689;

#[tokio::test]
#[ignore = "extended"]
async fn a_leech_heals_no_more_than_the_health_it_took() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, lock, map) = player(&realm, "P3D6", "Leechone", Race::Human, Class::Warlock).await;
    let out = map
        .query(move |m| {
            ready(m, lock, 60, &[]);
            let kobold = dummy(m, lock, 60, 5.0);
            crate::game::map::spells::cast_triggered(m, lock, DRAIN_LIFE, Some(kobold));
            let tick = holder(m, kobold, DRAIN_LIFE)
                .and_then(|h| h.effects.first().map(|x| x.amount))
                .unwrap_or(0) as u32;
            // A whole tick, healed whole.
            set_health(m, lock, 100);
            tick_now(m, kobold, DRAIN_LIFE);
            let whole = health(m, lock) - 100;
            // A kobold of two health gives two.
            set_health(m, lock, 100);
            set_health(m, kobold, 2);
            tick_now(m, kobold, DRAIN_LIFE);
            let little = health(m, lock) - 100;
            (tick, whole, little)
        })
        .await
        .unwrap();
    let (tick, whole, little) = out;
    assert!(tick > 2);
    assert_eq!(whole, tick, "a whole tick healed whole");
    assert_eq!(little, 2, "no more than the kobold had");
}

#[tokio::test]
#[ignore = "extended"]
async fn a_damage_over_time_keeps_its_cadence_through_late_updates_and_its_last_tick() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, lock, map) = player(&realm, "P3D7", "Cadenceone", Race::Human, Class::Warlock).await;
    let out = map
        .query(move |m| {
            ready(m, lock, 60, &[]);
            let kobold = dummy(m, lock, 60, 5.0);
            crate::game::map::spells::cast_triggered(m, lock, CORRUPTION, Some(kobold));
            let full = holder(m, kobold, CORRUPTION)
                .and_then(|h| h.duration)
                .unwrap_or(0);
            // Updates 55 ms apart, each a little late for a tick.
            let mut ticks = 0;
            let mut gone = 0;
            while gone < u64::from(full) + 500 {
                let before = health(m, kobold);
                pass(m, 55);
                gone += 55;
                if health(m, kobold) < before {
                    ticks += 1;
                }
            }
            // Removed, it ticks no more.
            crate::game::map::spells::cast_triggered(m, lock, CORRUPTION, Some(kobold));
            pass(m, 1_000);
            auras::remove_spell(m, kobold, CORRUPTION, None);
            let before = health(m, kobold);
            pass(m, 5_000);
            (full, ticks, health(m, kobold) == before)
        })
        .await
        .unwrap();
    let (full, ticks, quiet) = out;
    assert_eq!(full % 3000, 0, "Corruption's ticks three seconds apart");
    assert_eq!(ticks, full / 3000, "every tick, the last too");
    assert!(quiet, "removed, it ticks no more");
}

const FIREBALL: u32 = 133;
const FROSTBOLT: u32 = 116;
const LAY_ON_HANDS: u32 = 633;
const CONJURE_WATER: u32 = 5504;
const CONJURED_WATER: u32 = 5350;
const RESURRECTION: u32 = 2006;
const SEAL_OF_RIGHTEOUSNESS_STRIKE: u32 = 25742;
const AMPLIFY_MAGIC: u32 = 1008;
const LESSER_HEAL: u32 = 2050;
const COUNTERSPELL: u32 = 2139;

#[tokio::test]
#[ignore = "extended"]
async fn spells_do_what_their_effects_say() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, mage, map) = player(&realm, "P3D8", "Effectone", Race::Human, Class::Mage).await;
    let (_d, pally, _) = player(&realm, "P3D9", "Effecttwo", Race::Human, Class::Paladin).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            ready(m, pally, 60, &[]);
            let kobold = dummy(m, mage, 60, 10.0);
            let data = m.realm.data.clone();
            let s = |id: u32| data.spells.get(id).cloned().unwrap();
            // A fire reflect sends a Fireball back, not a Frostbolt.
            let mut reflect = made_up(990_980, 6, auras::AURA_REFLECT_SPELLS_SCHOOL, 4, 100);
            reflect.duration_index = 21;
            apply_template(m, kobold, kobold, &reflect);
            let (k, me) = (health(m, kobold), health(m, mage));
            crate::game::map::spells::cast_triggered(m, mage, FIREBALL, Some(kobold));
            let reflected = (health(m, kobold) == k, health(m, mage) < me);
            crate::game::map::spells::cast_triggered(m, mage, FROSTBOLT, Some(kobold));
            let frost = health(m, kobold) < k;
            auras::remove_spell(m, kobold, 990_980, None);
            // A heal's crit is half more.
            let heal = s(LESSER_HEAL);
            let plain = outcome::healing(m, pally, pally, &heal, 0, 100.0, &Landing::default());
            let crit = outcome::healing(
                m,
                pally,
                pally,
                &heal,
                0,
                100.0,
                &Landing {
                    crit: true,
                    ..Default::default()
                },
            );
            // Amplify Magic's healing by the heal's coefficient.
            apply(m, mage, pally, AMPLIFY_MAGIC);
            let amplified = outcome::healing(m, pally, pally, &heal, 0, 100.0, &Landing::default());
            let bonus =
                auras::auras(m, pally).map_or(0, |a| a.sum_mask(auras::AURA_MOD_HEALING, 2));
            let coefficient = outcome::coefficient(m, &heal, 0, Kind::Direct);
            auras::remove_spell(m, pally, AMPLIFY_MAGIC, None);
            // Lay on Hands: the caster's whole health.
            let max = m
                .unit_fields(pally)
                .map_or(0, |f| f.get_u32(crate::game::fields::UNIT_FIELD_MAXHEALTH));
            set_health(m, mage, 100);
            if let Some(f) = m.unit_fields_mut(mage) {
                f.set_u32(crate::game::fields::UNIT_FIELD_MAXHEALTH, 100_000);
            }
            crate::game::map::spells::cast_triggered(m, pally, LAY_ON_HANDS, Some(mage));
            let laid = health(m, mage) - 100;
            // A spell that can't crit doesn't, at a sure crit.
            let mut sure = made_up(990_981, 6, auras::AURA_MOD_SPELL_CRIT_CHANCE, 0, 100);
            sure.duration_index = 21;
            apply_template(m, pally, pally, &sure);
            let mods = crate::game::map::spells::mods::Mods::default();
            let crits = (
                outcome::roll_crit(m, pally, kobold, &s(FIREBALL), &mods),
                outcome::roll_crit(m, pally, kobold, &s(SEAL_OF_RIGHTEOUSNESS_STRIKE), &mods),
            );
            // Shadow power does nothing for a Fireball.
            let mut shadow = made_up(990_982, 6, auras::AURA_MOD_DAMAGE_DONE, 32, 100);
            shadow.duration_index = 21;
            apply_template(m, mage, mage, &shadow);
            let fire =
                outcome::damage(m, mage, kobold, &s(FIREBALL), 0, 100.0, &Landing::default())
                    .amount;
            // A Fireball that kills leaves no burn on the dead.
            set_health(m, kobold, 5);
            crate::game::map::spells::cast_triggered(m, mage, FIREBALL, Some(kobold));
            let killed = (
                m.creature(kobold).is_none_or(|c| !c.alive()),
                has(m, kobold, FIREBALL),
            );
            // Resurrection does nothing for a dead creature.
            crate::game::map::spells::cast_triggered(m, pally, RESURRECTION, Some(kobold));
            let raised = m.creature(kobold).is_some_and(|c| c.alive());
            // Conjure Water makes two.
            let had = m.players[&mage].carried_count_of(CONJURED_WATER);
            crate::game::map::spells::cast_triggered(m, mage, CONJURE_WATER, None);
            let water = m.players[&mage].carried_count_of(CONJURED_WATER) - had;
            // Counterspell locks the school out for its ten seconds.
            let other = dummy(m, mage, 60, 8.0);
            crate::game::map::spells::cast(m, other, FIREBALL, at(mage), None);
            crate::game::map::spells::cast_triggered(m, mage, COUNTERSPELL, Some(other));
            let lock = m.caster(other).map(|c| {
                c.school_lockout
                    .iter()
                    .map(|(mask, t)| (*mask, t.saturating_duration_since(m.now)))
                    .collect::<Vec<_>>()
            });
            (
                reflected,
                frost,
                plain,
                crit,
                amplified,
                bonus,
                coefficient,
                max,
                laid,
                crits,
                fire,
                killed,
                raised,
                water,
                lock,
            )
        })
        .await
        .unwrap();
    let (
        reflected,
        frost,
        plain,
        crit,
        amplified,
        bonus,
        coefficient,
        max,
        laid,
        crits,
        fire,
        killed,
        raised,
        water,
        lock,
    ) = out;
    assert_eq!(
        reflected,
        (true, true),
        "the Fireball comes back at the mage"
    );
    assert!(frost, "a Frostbolt isn't reflected");
    assert_eq!(crit, plain * 3 / 2, "a heal's crit is half more");
    assert!(bonus > 0);
    assert_eq!(
        amplified,
        (100.0 + bonus as f32 * coefficient) as u32,
        "Amplify Magic by the coefficient"
    );
    assert_eq!(laid, max, "Lay on Hands heals the paladin's health");
    assert_eq!(
        crits,
        (true, false),
        "Seal of Righteousness's strike can't crit"
    );
    assert_eq!(fire, 100, "shadow power is nothing to a Fireball");
    assert_eq!(killed, (true, false), "a killing Fireball leaves no burn");
    assert!(!raised, "Resurrection is for players");
    // Two at its own level, two more a level to its thirteenth.
    assert_eq!(water, 20, "twenty waters at sixty");
    assert_eq!(
        lock,
        Some(vec![(4, Duration::from_secs(10))]),
        "fire locked out ten seconds"
    );
}

const DEATHS_DOOR: u32 = 23127;
const DIVINE_SHIELD: u32 = 642;
const CURE_DISEASE: u32 = 528;

#[tokio::test]
#[ignore = "extended"]
async fn a_percent_damage_tick_takes_its_share_of_the_health_there_is_room_for() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, priest, map) = player(&realm, "P3D10", "Doorone", Race::Human, Class::Priest).await;
    let out = map
        .query(move |m| {
            ready(m, priest, 60, &[]);
            let kobold = dummy(m, priest, 60, 5.0);
            // Its base too, which its auras' changes derive from.
            let set_max = |m: &mut Map, g: Guid, max: u32| {
                if let Some(c) = m.creature_mut(g) {
                    c.base.max_health = max;
                    c.fields
                        .set_u32(crate::game::fields::UNIT_FIELD_MAXHEALTH, max);
                    c.fields
                        .set_u32(crate::game::fields::UNIT_FIELD_HEALTH, max);
                }
            };
            let tick = |m: &mut Map, g: Guid| {
                let max = m
                    .unit_fields(g)
                    .map_or(0, |f| f.get_u32(crate::game::fields::UNIT_FIELD_MAXHEALTH));
                set_health(m, g, max);
                let before = health(m, g);
                tick_now(m, g, DEATHS_DOOR);
                before - health(m, g)
            };
            set_max(m, kobold, 1000);
            apply(m, priest, kobold, DEATHS_DOOR);
            let first = tick(m, kobold);
            // Of the health it has at the tick.
            set_max(m, kobold, 2007);
            let grown = tick(m, kobold);
            // Two casters', each its own.
            let other = dummy(m, priest, 60, 6.0);
            apply(m, other, kobold, DEATHS_DOOR);
            set_max(m, kobold, 1000);
            let both = tick(m, kobold);
            auras::remove_spell(m, kobold, DEATHS_DOOR, Some(other));
            // A Divine Shield takes it, and keeps it off.
            apply(m, kobold, kobold, DIVINE_SHIELD);
            let purged = !has(m, kobold, DEATHS_DOOR);
            apply(m, priest, kobold, DEATHS_DOOR);
            let shielded = (has(m, kobold, DEATHS_DOOR), tick(m, kobold));
            auras::remove_spell(m, kobold, DIVINE_SHIELD, None);
            apply(m, priest, kobold, DEATHS_DOOR);
            // Half taken: half.
            let mut half = made_up(990_997, 6, auras::AURA_MOD_DAMAGE_PERCENT_TAKEN, 127, -50);
            half.duration_index = 21;
            apply_template(m, kobold, kobold, &half);
            let halved = tick(m, kobold);
            auras::remove_spell(m, kobold, 990_997, None);
            // Cured, it ticks no more.
            spells::cast_triggered(m, kobold, CURE_DISEASE, Some(kobold));
            let cured = (has(m, kobold, DEATHS_DOOR), tick(m, kobold));
            (first, grown, both, purged, shielded, halved, cured)
        })
        .await
        .unwrap();
    let (first, grown, both, purged, shielded, halved, cured) = out;
    assert!(purged, "Divine Shield takes it");
    assert_eq!(first, 50, "five in a hundred of 1000");
    assert_eq!(grown, 100, "of the 2007 there is then, truncated");
    assert_eq!(both, 100, "two casters', each its own");
    assert_eq!(shielded, (false, 0), "and keeps it off");
    assert_eq!(halved, 25, "half of it taken");
    assert_eq!(cured, (false, 0), "cured, no more");
}
