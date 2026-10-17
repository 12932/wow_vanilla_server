//! Phase 3's spell damage and healing, against the real data when it is
//! here, as Thistle Tea's tests of them have it: what a target's damage
//! taken does to a spell's hit and tick.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::map::spells::outcome::{self, Kind, Landing};
use crate::game::map::{Map, auras};

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
            lasting(m, 990_822, auras::AURA_MOD_HEALING_DONE, 0, 1000);
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
    let mut at = Vec::new();
    let mut gone = 0;
    while gone < ms {
        let before = health(m, kobold);
        pass(m, 50);
        gone += 50;
        if health(m, kobold) < before {
            at.push(gone);
        }
    }
    at
}

#[tokio::test]
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
            (blizzard, rain, consecration)
        })
        .await
        .unwrap();
    let (blizzard, rain, consecration) = out;
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
