//! Bugs found while mapping wvs's tests against Thistle Tea's, the fourth
//! batch: the weather's seasons, sounds and changes; a scripted stay, a
//! dead master's guardian, extra attacks in a cast, pets and civilians, an
//! event's looks in a fight; and equipment: Dual Wield, proficiency from
//! the spells learned, the skill and spell an item asks for, gear not learned
//! coming off on login; a profession's skill as it is learned.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::scripts::ScriptStep;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
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

const PEASANT: u32 = 3040;

/// The pets' world, with a guardian a hunter summons for a minute, and a
/// civilian of the wolves' faction.
fn pets_data() -> crate::game::data::GameData {
    let mut d = tests_pets::data();
    d.creature_templates.insert(
        GUARDIAN,
        tests_pets::template(GUARDIAN, "Dragonling", 35, 1),
    );
    let mut peasant = tests_pets::template(PEASANT, "Peasant", 14, 1);
    peasant.civilian = true;
    d.creature_templates.insert(PEASANT, peasant);
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
    let realm = harness::realm_with(pets_data());
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

const LONG_CAST: u32 = 9200;

/// Extra attacks a proc owes swing while a spell is cast, and the cast
/// goes on, as vmangos's `AttackerStateUpdate` (no cast check for an extra
/// swing) and Thistle Tea's `extra_attacks_step` (before its casting)
/// have it; the regular swings wait for the cast.
#[tokio::test]
async fn extra_attacks_swing_through_a_cast() {
    let mut d = tests_pets::data();
    let none = crate::game::data::spells::SpellEffect::default();
    let mut s = tests_pets::spell(
        LONG_CAST,
        "Long Cast",
        [tests_pets::effect(3, 0, 1), none, none],
    );
    s.casting_time_index = 90;
    d.spells.cast_times.insert(90, 10_000);
    d.spells.spells.insert(LONG_CAST, s);
    let realm = harness::realm_with(d);
    let (mut a, ga, map) =
        tests_pets::enter(&realm, "A", "Warrior", Race::Dwarf, Class::Warrior).await;
    let wolf = tests_pets::place(&map, ga, tests_pets::WOLF, 0x70_0501, 1.5).await;
    a.sees(wolf).await;
    map.query(move |m| {
        if let Some(c) = m.creature_mut(wolf) {
            c.fields.set_u32(UNIT_FIELD_MAXHEALTH, 1_000_000);
            c.fields.set_u32(UNIT_FIELD_HEALTH, 1_000_000);
        }
    })
    .await
    .unwrap();
    tests_pets::learn(&map, ga, &[LONG_CAST]);
    a.send(CMSG_ATTACKSWING { guid: wolf }).await;
    tests_pets::cast(&a, LONG_CAST, None).await;
    until(&map, "the cast under way", move |m| {
        m.caster(ga).is_some_and(|c| c.casting.is_some())
    })
    .await;
    let swings_held = map
        .query(move |m| {
            let now = m.now;
            let p = m.players.get_mut(&ga).unwrap();
            p.combat.next_swing = Some(now);
            p.combat.extra_attacks = 2;
            fighting(m, ga, wolf)
        })
        .await
        .unwrap();
    assert!(swings_held, "attacking the wolf");
    until(&map, "the extra attacks swung", move |m| {
        m.players[&ga].combat.extra_attacks == 0
    })
    .await;
    let (casting, swing_waits) = map
        .query(move |m| {
            let now = m.now;
            (
                m.caster(ga).is_some_and(|c| c.casting.is_some()),
                m.players[&ga].combat.next_swing.is_some_and(|t| t <= now),
            )
        })
        .await
        .unwrap();
    assert!(casting, "the cast goes on");
    assert!(swing_waits, "the regular swing waits for the cast");
}

/// An aggressive pet looking for a fight passes over a civilian, whose
/// death would be a dishonorable kill, as Thistle Tea's
/// `Acquisition.civilian_allowed?` has it; it takes on a wolf of the same
/// faction.
#[tokio::test]
async fn a_pet_spares_civilians() {
    let realm = harness::realm_with(pets_data());
    let (mut h, gh, map) =
        tests_pets::enter(&realm, "H", "Hunter", Race::Dwarf, Class::Hunter).await;
    tests_pets::learn(&map, gh, &[tests_pets::TAME_BEAST]);
    let boar = tests_pets::place(&map, gh, tests_pets::BOAR, 0x70_0601, 4.0).await;
    h.sees(boar).await;
    tests_pets::cast(&h, tests_pets::TAME_BEAST, Some(boar)).await;
    let pet = tests_pets::pet_bar(&mut h).await;
    assert!(!pet.is_zero(), "the tamed boar");
    map.query(move |m| {
        if let Some(s) = pets::state_mut(m, pet) {
            s.react = pets::REACT_AGGRESSIVE;
        }
    })
    .await
    .unwrap();
    let peasant = tests_pets::place(&map, gh, PEASANT, 0x70_0602, 8.0).await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    let spared = map
        .query(move |m| !m.combat(pet).is_some_and(|c| c.attacking))
        .await
        .unwrap();
    assert!(spared, "the pet leaves the peasant be");
    map.query(move |m| m.remove_creature(peasant))
        .await
        .unwrap();
    let wolf = tests_pets::place(&map, gh, tests_pets::WOLF, 0x70_0603, 8.0).await;
    until(&map, "the pet at the wolf", move |m| fighting(m, pet, wolf)).await;
}

/// A game event starting changes no creature under its foes: one in a
/// fight keeps its looks, and its fight, and takes the event's when the
/// fight is over (Thistle Tea changes the living creature in place).
#[tokio::test]
async fn an_event_changes_no_creature_in_a_fight() {
    const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
    let mut d = tests_pets::data();
    d.creature_spawns.insert(
        0,
        vec![crate::game::data::CreatureSpawn {
            guid: 77,
            entries: vec![tests_pets::WOLF],
            map: 0,
            x: START.0 + 3.0,
            y: START.1,
            z: START.2,
            respawn_min: 60,
            respawn_max: 60,
            ..Default::default()
        }],
    );
    d.world.event_creatures.insert(
        77,
        vec![crate::game::data::world::EventCreature {
            event: 9,
            display: 4321,
            ..Default::default()
        }],
    );
    let realm = harness::realm_with(d);
    let (mut a, ga, map) =
        tests_pets::enter(&realm, "A", "Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let wolf = crate::game::guid::unit(tests_pets::WOLF, 77);
    a.sees(wolf).await;
    let display = move |m: &Map| {
        m.creatures
            .iter()
            .find(|(_, c)| c.spawn_guid == Some(77))
            .map(|(_, c)| c.fields.get_u32(UNIT_FIELD_DISPLAYID))
    };
    let before = map.query(move |m| display(m)).await.unwrap();
    let on: ahash::AHashSet<u32> = [9].into_iter().collect();
    *realm.social.events.lock() = on.clone();
    let (fights, worn) = map
        .query(move |m| {
            if let Some(c) = m.creature_mut(wolf) {
                c.fields.set_u32(UNIT_FIELD_MAXHEALTH, 1_000_000);
                c.fields.set_u32(UNIT_FIELD_HEALTH, 1_000_000);
            }
            crate::game::map::ai::attacked(m, wolf, ga);
            crate::game::events::apply(m, &on);
            (
                m.creature(wolf)
                    .is_some_and(|c| c.combat.attacking && c.combat.victim == ga),
                display(m),
            )
        })
        .await
        .unwrap();
    assert!(fights, "the wolf fights on");
    assert_eq!(worn, before, "in its own looks");
    // Its fight over, it takes the event's.
    map.query(move |m| crate::game::map::ai::enter_evade(m, wolf))
        .await
        .unwrap();
    until(&map, "the event's looks", move |m| display(m) == Some(4321)).await;
}

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| crate::game::harness::real_data("db"));

fn have_data() -> bool {
    DB.as_path().join("vmangos.sqlite").exists()
}

fn real_realm() -> std::sync::Arc<crate::game::Realm> {
    harness::realm_with(crate::game::data::load::load(DB.as_path()))
}

const BAG_PLAYER: u8 = crate::game::store::BAG_PLAYER;
const SLOT_MAIN_HAND: u8 = 15;
const SLOT_OFF_HAND: u8 = 16;
const SLOT_CHEST: u8 = 4;
const SHORTSWORD: u32 = 2131;
const WORN_SHORTSWORD: u32 = 25;
const PEARL_SPEAR: u32 = 1406;
const LIGHT_MAIL: u32 = 2392;
const FISHING_POLE: u32 = 6256;
const TIGER_GOGGLES: u32 = 4368;
const DIMENSIONAL_RIPPER: u32 = 18984;
const DUAL_WIELD: u32 = 674;
const POLEARMS: u32 = 200;
const MAIL: u32 = 8737;
const FISHING: u32 = 7620;
const ENGINEERING: u32 = 4036;
const GOBLIN_ENGINEER: u32 = 20222;

/// `g` at `level`, knowing `learn` too.
fn level(m: &mut Map, g: Guid, level: u8, learn: &[u32]) {
    let data = m.realm.data.clone();
    let p = m.players.get_mut(&g).unwrap();
    crate::game::map::stats::set_level(p, &data, level);
    crate::game::map::skills::level_up(p);
    for s in learn {
        crate::game::map::spells::learn(m, g, *s);
    }
}

/// Where `g`'s first `entry` not worn is.
fn carried(m: &Map, g: Guid, entry: u32) -> Option<(u8, u8)> {
    m.players[&g]
        .items
        .iter()
        .find(|i| i.entry == entry && !(i.bag == BAG_PLAYER && i.slot < 19))
        .map(|i| (i.bag, i.slot))
}

/// The entry worn in equipment `slot`.
fn worn(m: &Map, g: Guid, slot: u8) -> Option<u32> {
    let p = &m.players[&g];
    p.item_at(BAG_PLAYER, slot).map(|i| p.items[i].entry)
}

/// Give `g` an `entry` and try to wear it in `slot`: what the client is
/// told of a failure, if any, and what is worn there after.
async fn wear(
    c: &mut Client,
    map: &MapHandle,
    g: Guid,
    entry: u32,
    slot: u8,
) -> (Option<String>, Option<u32>) {
    let from = map
        .query(move |m| {
            if carried(m, g, entry).is_none() {
                crate::game::map::inventory::add_item(m, g, entry, 1, false).unwrap();
            }
            carried(m, g, entry).unwrap()
        })
        .await
        .unwrap();
    c.clear();
    c.send(CMSG_SWAP_ITEM {
        destination_bag: BAG_PLAYER,
        destionation_slot: slot,
        source_bag: from.0,
        source_slot: from.1,
    })
    .await;
    let failed = c
        .gets(Duration::from_millis(300), |m| match m {
            ServerOpcodeMessage::SMSG_INVENTORY_CHANGE_FAILURE(f) => {
                Some(format!("{:?}", f.result))
            }
            _ => None,
        })
        .await;
    let now = map.query(move |m| worn(m, g, slot)).await.unwrap();
    (failed, now)
}

fn refused(r: &(Option<String>, Option<u32>), why: &str) -> bool {
    r.0.as_deref().is_some_and(|f| f.starts_with(why))
}

/// A one-hander goes in the off hand only with Dual Wield learned, as
/// vmangos and Thistle Tea have it, not at level 20; without it, equipped
/// as it comes, it goes to the main hand.
#[tokio::test]
async fn a_weapon_in_the_off_hand_takes_dual_wield() {
    if !have_data() {
        return;
    }
    let realm = real_realm();
    let (mut a, g, map) =
        tests_pets::enter(&realm, "A", "Warrior", Race::Human, Class::Warrior).await;
    map.query(move |m| level(m, g, 20, &[])).await.unwrap();
    let r = wear(&mut a, &map, g, SHORTSWORD, SLOT_OFF_HAND).await;
    assert!(refused(&r, "CantDualWield"), "{r:?}");
    // The shield off, and the sword equipped as it comes: the main hand.
    let at = map
        .query(move |m| {
            let p = m.players.get_mut(&g).unwrap();
            let free = p.free_carry_slot(&m.realm.data.clone()).unwrap();
            crate::game::map::inventory::move_item(m, g, (BAG_PLAYER, SLOT_OFF_HAND), free);
            carried(m, g, SHORTSWORD).unwrap()
        })
        .await
        .unwrap();
    a.send(CMSG_AUTOEQUIP_ITEM {
        source_bag: at.0,
        source_slot: at.1,
    })
    .await;
    until(&map, "the sword in the main hand", move |m| {
        worn(m, g, SLOT_MAIN_HAND) == Some(SHORTSWORD)
    })
    .await;
    assert_eq!(
        map.query(move |m| worn(m, g, SLOT_OFF_HAND)).await.unwrap(),
        None
    );
    // Dual Wield learned, the old sword goes in the off hand.
    map.query(move |m| crate::game::map::spells::learn(m, g, DUAL_WIELD))
        .await
        .unwrap();
    let r = wear(&mut a, &map, g, WORN_SHORTSWORD, SLOT_OFF_HAND).await;
    assert!(
        refused(&r, "ItemDoesntGoToSlot"),
        "a main-hand weapon doesn't go there: {r:?}"
    );
    let r = wear(&mut a, &map, g, SHORTSWORD, SLOT_OFF_HAND).await;
    assert_eq!(r, (None, Some(SHORTSWORD)));
}

/// Weapon and armor proficiency comes from the spells learned, as Thistle
/// Tea's and mangos's have it, not by class and level: a warrior learns
/// Polearms, a hunter of 40 Mail, and anyone Fishing for a fishing pole.
#[tokio::test]
async fn proficiency_is_learned_not_had_by_level() {
    if !have_data() {
        return;
    }
    let realm = real_realm();
    let (mut a, g, map) =
        tests_pets::enter(&realm, "A", "Warrior", Race::Human, Class::Warrior).await;
    map.query(move |m| level(m, g, 20, &[])).await.unwrap();
    let r = wear(&mut a, &map, g, PEARL_SPEAR, SLOT_MAIN_HAND).await;
    assert!(refused(&r, "NoRequiredProficiency"), "{r:?}");
    let r = wear(&mut a, &map, g, FISHING_POLE, SLOT_MAIN_HAND).await;
    assert!(refused(&r, "NoRequiredProficiency"), "{r:?}");
    map.query(move |m| level(m, g, 20, &[POLEARMS, FISHING]))
        .await
        .unwrap();
    let r = wear(&mut a, &map, g, PEARL_SPEAR, SLOT_MAIN_HAND).await;
    assert_eq!(r, (None, Some(PEARL_SPEAR)));
    let r = wear(&mut a, &map, g, FISHING_POLE, SLOT_MAIN_HAND).await;
    assert_eq!(r, (None, Some(FISHING_POLE)));
    let (mut h, gh, map) =
        tests_pets::enter(&realm, "H", "Hunter", Race::Dwarf, Class::Hunter).await;
    map.query(move |m| level(m, gh, 40, &[])).await.unwrap();
    let r = wear(&mut h, &map, gh, LIGHT_MAIL, SLOT_CHEST).await;
    assert!(refused(&r, "NoRequiredProficiency"), "{r:?}");
    map.query(move |m| level(m, gh, 40, &[MAIL])).await.unwrap();
    let r = wear(&mut h, &map, gh, LIGHT_MAIL, SLOT_CHEST).await;
    assert_eq!(r, (None, Some(LIGHT_MAIL)));
}

/// Gear that asks for a skill at a rank, or a spell, is worn only with
/// them, as mangos's `CanUseItem` and Thistle Tea's `Proficiency` have it:
/// engineer's goggles want Engineering at 100 (short of it, mangos's "not
/// skilled enough"), a goblin engineer's ripper the Goblin Engineer too.
#[tokio::test]
async fn gear_asks_for_its_skill_and_spell() {
    if !have_data() {
        return;
    }
    let realm = real_realm();
    let (mut a, g, map) =
        tests_pets::enter(&realm, "A", "Warrior", Race::Human, Class::Warrior).await;
    let r = wear(&mut a, &map, g, TIGER_GOGGLES, 0).await;
    assert!(refused(&r, "NoRequiredProficiency"), "{r:?}");
    map.query(move |m| level(m, g, 1, &[ENGINEERING]))
        .await
        .unwrap();
    let r = wear(&mut a, &map, g, TIGER_GOGGLES, 0).await;
    assert!(refused(&r, "CantEquipSkill"), "{r:?}");
    map.query(move |m| {
        let p = m.players.get_mut(&g).unwrap();
        crate::game::map::skills::learn_skill(p, 202, 4);
        crate::game::map::skills::set_value(p, 202, 300);
    })
    .await
    .unwrap();
    let r = wear(&mut a, &map, g, TIGER_GOGGLES, 0).await;
    assert_eq!(r, (None, Some(TIGER_GOGGLES)));
    let r = wear(&mut a, &map, g, DIMENSIONAL_RIPPER, 12).await;
    assert!(refused(&r, "NoRequiredProficiency"), "{r:?}");
    map.query(move |m| level(m, g, 1, &[GOBLIN_ENGINEER]))
        .await
        .unwrap();
    let r = wear(&mut a, &map, g, DIMENSIONAL_RIPPER, 12).await;
    assert_eq!(r, (None, Some(DIMENSIONAL_RIPPER)));
}

/// What every race and class starts with, it can wear with the spells it
/// starts with (the dwarven mage, who has no start, aside).
#[test]
fn every_start_wears_its_gear() {
    if !have_data() {
        return;
    }
    let data = crate::game::data::load::load(DB.as_path());
    let mut checked = 0;
    for ((race, class), start) in &data.start {
        if start.spells.is_empty() {
            continue;
        }
        let prof = crate::game::map::inventory::Proficiency::from_spells(&data, &start.spells);
        for (entry, _, _) in &start.items {
            let Some(t) = data.item(*entry) else {
                continue;
            };
            assert!(
                prof.covers(t),
                "race {race} class {class}: {} ({entry})",
                t.name
            );
            checked += 1;
        }
        // None starts dual wielding.
        assert!(!prof.dual_wield, "race {race} class {class}");
    }
    assert!(checked > 100, "{checked}");
}

/// A character made when proficiency came by class and level, wearing
/// what it hasn't learned, finds it in its bags on logging in, or, with
/// no room, in its mail, as mangos's `_LoadInventory` takes off what it
/// can't equip.
#[tokio::test]
async fn gear_not_learned_comes_off_on_login() {
    if !have_data() {
        return;
    }
    let realm = real_realm();
    let (mut h, g, map) =
        tests_pets::enter(&realm, "H", "Hunter", Race::Dwarf, Class::Hunter).await;
    // Mail on, at 40, as the old rules let a hunter.
    let put_on = move |m: &mut Map| {
        level(m, g, 40, &[]);
        if let Some(i) = m.players[&g].item_at(BAG_PLAYER, SLOT_CHEST) {
            crate::game::map::inventory::take_item(m, g, i, 1);
        }
        crate::game::map::inventory::add_item(m, g, LIGHT_MAIL, 1, false).unwrap();
        let p = m.players.get_mut(&g).unwrap();
        let i = p.items.iter().position(|i| i.entry == LIGHT_MAIL).unwrap();
        p.items[i].bag = BAG_PLAYER;
        p.items[i].slot = SLOT_CHEST;
    };
    map.query(put_on).await.unwrap();
    let relog = async |h: &mut Client| {
        h.send(CMSG_LOGOUT_REQUEST {}).await;
        h.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
        })
        .await;
        h.clear();
        h.login(g).await;
    };
    relog(&mut h).await;
    let map = realm.registry.online(g).unwrap().map();
    let (off, bagged) = map
        .query(move |m| (worn(m, g, SLOT_CHEST), carried(m, g, LIGHT_MAIL)))
        .await
        .unwrap();
    assert_eq!(off, None, "off");
    assert!(bagged.is_some(), "into the bags");
    // On again, with the bags full: by mail.
    map.query(move |m| {
        crate::game::map::inventory::take_item(
            m,
            g,
            m.players[&g]
                .item_by_guid(
                    m.players[&g]
                        .items
                        .iter()
                        .find(|i| i.entry == LIGHT_MAIL)
                        .unwrap()
                        .guid,
                )
                .unwrap(),
            1,
        );
        put_on(m);
        while crate::game::map::inventory::add_item(m, g, SHORTSWORD, 1, false).is_ok() {}
    })
    .await
    .unwrap();
    relog(&mut h).await;
    let map = realm.registry.online(g).unwrap().map();
    let still = map
        .query(move |m| m.players[&g].count_of(LIGHT_MAIL))
        .await
        .unwrap();
    assert_eq!(still, 0, "not on, nor carried");
    let counter = crate::game::guid::counter(g);
    let mailed =
        realm.social.mail.lock().mails.values().any(|m| {
            m.receiver == counter && m.item.as_ref().is_some_and(|i| i.entry == LIGHT_MAIL)
        });
    assert!(mailed, "mailed to itself");
}

/// A profession's rank learned gives its skill at once, at the step its
/// points say, as on login (mangos's `addSpell` learns a spell's skill
/// with it): it is no passive, so it waited for the next login.
#[tokio::test]
async fn a_profession_learned_gives_its_skill_at_once() {
    if !have_data() {
        return;
    }
    let realm = real_realm();
    let (_a, g, map) = tests_pets::enter(&realm, "A", "Warrior", Race::Human, Class::Warrior).await;
    let (engineering, fishing) = map
        .query(move |m| {
            level(m, g, 10, &[ENGINEERING, FISHING]);
            let p = &m.players[&g];
            let skill = |s| {
                crate::game::map::skills::value(p, s).zip(crate::game::map::skills::max_value(p, s))
            };
            (skill(202), skill(356))
        })
        .await
        .unwrap();
    assert_eq!(engineering, Some((1, 75)));
    assert_eq!(fishing, Some((1, 75)));
}
