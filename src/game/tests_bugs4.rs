//! Bugs found while mapping wvs's tests against Thistle Tea's, the fourth
//! batch: the weather's seasons, sounds and changes; a scripted stay, a
//! dead master's guardian.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::scripts::ScriptStep;
use crate::game::fields::*;
use crate::game::harness;
use crate::game::map::pets;
use crate::game::map::{Map, MapHandle};
use crate::game::tests_pets;
use crate::game::weather::{self, Rolls};

/// A change's rolls.
fn rolls(change: u32, kind: u32, radical: u32, intensity: u32, grade: f32) -> Rolls {
    Rolls {
        change,
        kind,
        radical,
        intensity,
        grade,
    }
}

fn day0(month: u32, day: u32) -> u32 {
    use chrono::Datelike;
    chrono::NaiveDate::from_ymd_opt(2026, month, day)
        .unwrap()
        .ordinal0()
}

/// Spring starts on 19 March, not 1 March, as mangos's and Thistle Tea's
/// day-of-year seasons have it: 0 spring, 1 summer, 2 fall, 3 winter.
#[test]
fn the_seasons_turn_on_vanillas_days() {
    for (month, day, season) in [
        (1, 1, 3),
        (3, 1, 3),
        (3, 18, 3),
        (3, 19, 0),
        (6, 17, 0),
        (6, 18, 1),
        (9, 16, 1),
        (9, 17, 2),
        (12, 16, 2),
        (12, 17, 3),
        (12, 31, 3),
    ] {
        assert_eq!(weather::season(day0(month, day)), season, "{day}/{month}");
    }
}

/// The sound of rain, snow and a storm: none under 0.3, light under 0.6,
/// medium under 0.9, heavy above, as vmangos and Thistle Tea have it; not
/// the grade's third.
#[test]
fn the_weather_sounds_by_vanillas_thresholds() {
    for (kind, base) in [(1u8, 8533u32), (2, 8536), (3, 8556)] {
        for (grade, sound) in [
            (0.2999f32, 0),
            (0.3, base),
            (0.5999, base),
            (0.6, base + 1),
            (0.8999, base + 1),
            (0.9, base + 2),
            (0.9999, base + 2),
        ] {
            assert_eq!(weather::sound(kind, grade), sound, "{kind} at {grade}");
        }
    }
    assert_eq!(weather::sound(0, 0.0), 0);
}

/// Weather gets better or worse by a third, or changes all at once, as
/// mangos's `ReGenerate` and Thistle Tea's `Weather.advance` have it.
#[test]
fn the_weather_changes_gradually_and_suddenly() {
    let rain = [100, 0, 0];
    // Three in ten, no change.
    assert_eq!(
        weather::advance(1, 0.7, rain, rolls(29, 1, 0, 0, 1.0)),
        (1, 0.7)
    );
    // Better by a third, and worse.
    let (kind, grade) = weather::advance(1, 0.7, rain, rolls(30, 1, 0, 0, 1.0));
    assert_eq!(kind, 1);
    assert!((grade - 0.366_666_66).abs() < 1e-6, "{grade}");
    assert_eq!(
        weather::advance(1, 0.7, rain, rolls(60, 1, 0, 0, 1.0)),
        (1, 0.9999)
    );
    // Light weather getting better gives way to the season's: here fair.
    assert_eq!(
        weather::advance(1, 0.1, [0, 0, 0], rolls(30, 1, 0, 0, 1.0)),
        (0, 0.0)
    );
    // New weather by the chances, light.
    for (kind, expected) in [
        (1, 1),
        (20, 1),
        (21, 2),
        (50, 2),
        (51, 3),
        (60, 3),
        (61, 0),
        (100, 0),
    ] {
        let (k, grade) = weather::advance(0, 0.0, [20, 30, 10], rolls(60, kind, 0, 0, 1.0));
        assert_eq!(k, expected, "a roll of {kind}");
        assert_eq!(grade, if expected == 0 { 0.0 } else { 0.3333 });
    }
    // Radically: light goes heavy; heavy goes light half the time, or
    // gives way to new weather.
    let snow = [0, 100, 0];
    assert_eq!(
        weather::advance(1, 0.2, snow, rolls(90, 1, 0, 0, 1.0)),
        (1, 0.9999)
    );
    let (kind, grade) = weather::advance(3, 0.9, snow, rolls(90, 1, 49, 0, 1.0));
    assert_eq!(kind, 3);
    assert!((grade - 0.233_333_3).abs() < 1e-6, "{grade}");
    assert_eq!(
        weather::advance(1, 0.9, snow, rolls(90, 1, 50, 50, 1.0)),
        (2, 0.9999)
    );
    // New weather on a radical change is medium or heavy.
    assert_eq!(
        weather::advance(0, 0.0, snow, rolls(90, 1, 0, 49, 0.0)),
        (2, 0.3334)
    );
}

const GUARDIAN: u32 = 3030;
const SUMMON_GUARDIAN: u32 = 9050;

/// The pets' world, with a guardian a hunter summons for a minute.
fn guardian_data() -> crate::game::data::GameData {
    let mut d = tests_pets::data();
    d.creature_templates.insert(
        GUARDIAN,
        tests_pets::template(GUARDIAN, "Dragonling", 35, 1),
    );
    let mut e = tests_pets::effect(42, 0, 32);
    e.misc_value = GUARDIAN as i32;
    let none = crate::game::data::spells::SpellEffect::default();
    let mut s = tests_pets::spell(SUMMON_GUARDIAN, "Summon Dragonling", [e, none, none]);
    s.duration_index = 4;
    d.spells.spells.insert(SUMMON_GUARDIAN, s);
    d
}

/// Wait until `check` holds on the map.
async fn until(map: &MapHandle, what: &str, check: impl Fn(&Map) -> bool + Send + Clone + 'static) {
    for _ in 0..150 {
        let c = check.clone();
        if map.query(move |m| c(m)).await.unwrap() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("never: {what}");
}

fn fighting(m: &Map, g: Guid, victim: Guid) -> bool {
    m.combat(g)
        .is_some_and(|c| c.attacking && c.victim == victim)
}

/// Script command 88's stay leaves a pet in its fight, as vmangos's
/// `HandlePetCommand` and Thistle Tea's `PetBT.command(:stay)` have it: it
/// halts where it is, and its attack goes on.
#[tokio::test]
async fn a_scripts_stay_keeps_a_pet_in_its_fight() {
    let (_realm, _h, _gh, map, pet, wolf) = tests_pets::tamed().await;
    let (sent, kept, command) = map
        .query(move |m| {
            pets::attack_target(m, pet, wolf);
            let sent = fighting(m, pet, wolf);
            let stay = ScriptStep {
                command: 88,
                datalong: [u32::from(pets::COMMAND_STAY), 0, 0, 0],
                ..Default::default()
            };
            crate::game::map::scripts::execute(m, &stay, pet, Guid::zero());
            (
                sent,
                fighting(m, pet, wolf),
                pets::state(m, pet).map(|s| s.command),
            )
        })
        .await
        .unwrap();
    assert!(sent, "sent at the wolf");
    assert!(kept, "staying, it fights on");
    assert_eq!(command, Some(pets::COMMAND_STAY));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        map.query(move |m| fighting(m, pet, wolf)).await.unwrap(),
        "and goes on fighting"
    );
}

/// A dead master's guardian fights on until its fight ends, as Thistle
/// Tea's `Guardian.lifetime` keeps it, and only then goes.
#[tokio::test]
async fn a_dead_masters_guardian_goes_when_its_fight_ends() {
    let realm = harness::realm_with(guardian_data());
    let (h, gh, map) = tests_pets::enter(&realm, "H", "Hunter", Race::Dwarf, Class::Hunter).await;
    tests_pets::learn(&map, gh, &[SUMMON_GUARDIAN]);
    tests_pets::cast(&h, SUMMON_GUARDIAN, None).await;
    until(&map, "a guardian", move |m| {
        m.players[&gh].owned.guardians.len() == 1
    })
    .await;
    let guardian = map
        .query(move |m| m.players[&gh].owned.guardians[0])
        .await
        .unwrap();
    let wolf = tests_pets::place(&map, gh, tests_pets::WOLF, 0x70_0401, 3.0).await;
    map.query(move |m| {
        if let Some(c) = m.creature_mut(wolf) {
            c.fields.set_u32(UNIT_FIELD_MAXHEALTH, 1_000_000);
            c.fields.set_u32(UNIT_FIELD_HEALTH, 1_000_000);
        }
        pets::attack_target(m, guardian, wolf);
        crate::game::map::combat::engage(m, guardian, wolf);
        crate::game::map::death::kill(m, gh);
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let kept = map
        .query(move |m| m.creature(guardian).is_some() && fighting(m, guardian, wolf))
        .await
        .unwrap();
    assert!(kept, "it fights on over its master's body");
    // Its fight over, it goes.
    map.query(move |m| {
        crate::game::map::combat::kill(m, guardian, wolf);
        crate::game::map::combat::leave_combat(m, guardian);
    })
    .await
    .unwrap();
    until(&map, "the guardian gone", move |m| {
        m.creature(guardian).is_none()
    })
    .await;
}
