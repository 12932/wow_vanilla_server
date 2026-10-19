//! Town guards as mangos-classic calls them (`map/guards.rs`), the towns'
//! alarms and what they escalate to, and Jaina Proudmoore's comings and
//! goings (`jaina.rs`): through the protocol, on a small world of a town's
//! civilian and its guards near Northshire, with Theramore's Jaina on
//! Kalimdor; and on the real navmesh and data, in the extended suite.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::jaina::{self, Place};
use crate::game::map::auras::{self, AURA_MOD_AOE_AVOIDANCE};
use crate::game::map::combat::dist;
use crate::game::map::creature::{MOVEMENT_RANDOM, Motion};
use crate::game::map::guards::{self, Alarm};
use crate::game::map::spells::mods::Mods;
use crate::game::map::spells::outcome::{self, Hit};
use crate::game::map::{Map, MapKey, ai, event_ai};
use crate::game::tests_p3_combat::{HELPER, START, data, faction, setup_with, spawn, template};
use crate::game::tests_p3_entity::on;
use crate::game::tests_p3_entity_objects::until;

/// A civilian of the town that calls the guards, and one of Ironforge.
pub(crate) const CIVILIAN: u32 = 3_810;
const DWARF: u32 = 3_811;
/// Stormwind's guard and its Royal Guard, Ironforge's guard, and Jaina.
pub(crate) const CITY_GUARD: u32 = 68;
pub(crate) const ROYAL_GUARD: u32 = 1756;
const IRONFORGE_GUARD: u32 = 5595;
pub(crate) const STORMWIND: u32 = 1519;
const IRONFORGE: u32 = 1537;
/// An area spell of fire, of the magic class.
const FLAMES: u32 = 990_610;
/// Where an orc starts, in the Valley of Trials: Jaina's spawn is beside
/// it here, so that the orc sees her come and go.
const ORC_START: (f32, f32, f32) = (-618.518, -4251.67, 38.718);
/// Her tower's top in Theramore, her Teleport's place.
const TOWER: (f32, f32, f32) = (-4018.1, -4525.24, 12.0);

pub(crate) fn world() -> GameData {
    let mut d = data();
    for (entry, spawn_x) in [(CIVILIAN, 4.0), (DWARF, -4.0)] {
        let mut t = template(entry, 7);
        t.static_flags1 |= guards::STATIC_CALLS_GUARDS;
        t.civilian = true;
        t.flags_extra = 0x2;
        d.creature_templates.insert(entry, t);
        d.creature_spawns.entry(0).or_default().push(CreatureSpawn {
            guid: entry,
            entries: vec![entry],
            map: 0,
            x: START.0 + spawn_x,
            y: START.1,
            z: START.2,
            respawn_min: 300,
            respawn_max: 300,
            ..Default::default()
        });
    }
    for entry in [CITY_GUARD, ROYAL_GUARD, IRONFORGE_GUARD] {
        let mut t = template(entry, 7);
        t.flags_extra = 0x400;
        // They walk and swim, with the swimming animation, as vmangos's
        // (`inhabit_type` 3, `static_flags1`'s `CAN_SWIM`).
        t.inhabit_type = 3;
        t.static_flags1 |= 0x1000_0000;
        d.creature_templates.insert(entry, t);
    }
    let mut j = template(jaina::JAINA, 7);
    j.name = "Lady Jaina Proudmoore".into();
    j.rank = 3;
    j.static_flags1 |= guards::STATIC_CALLS_GUARDS;
    d.creature_templates.insert(jaina::JAINA, j);
    d.creature_spawns.entry(1).or_default().push(CreatureSpawn {
        guid: jaina::SPAWN,
        entries: vec![jaina::JAINA],
        map: 1,
        x: ORC_START.0 + 3.0,
        y: ORC_START.1,
        z: ORC_START.2,
        respawn_min: 86_400,
        respawn_max: 86_400,
        ..Default::default()
    });
    let spell = |id: u32, name: &str, e: SpellEffect| {
        let mut s = SpellTemplate {
            id,
            name: name.into(),
            equipped_item_class: -1,
            ..Default::default()
        };
        s.effects[0] = e;
        s
    };
    let avoidance = spell(
        guards::AVOIDANCE,
        "Avoidance",
        SpellEffect {
            effect: 6,
            aura: AURA_MOD_AOE_AVOIDANCE,
            base_points: 24,
            base_dice: 1,
            die_sides: 1,
            target_a: 1,
            ..Default::default()
        },
    );
    let flash = spell(
        jaina::TELEPORT_VISUAL,
        "Simple Teleport",
        SpellEffect {
            effect: 46,
            target_a: 1,
            ..Default::default()
        },
    );
    let teleport = spell(
        jaina::TELEPORT,
        "Teleport",
        SpellEffect {
            effect: 5,
            target_a: 6,
            target_b: 17,
            ..Default::default()
        },
    );
    let mut flames = spell(
        FLAMES,
        "Flames",
        SpellEffect {
            effect: 2,
            base_points: 9,
            base_dice: 1,
            die_sides: 1,
            target_a: 15,
            ..Default::default()
        },
    );
    flames.school = 2;
    flames.dmg_class = 1;
    for s in [avoidance, flash, teleport, flames] {
        d.spells.spells.insert(s.id, s);
    }
    d.spells
        .target_positions
        .insert(jaina::TELEPORT, (1, TOWER.0, TOWER.1, TOWER.2, 6.13));
    d
}

pub(crate) fn unit(entry: u32) -> Guid {
    guid::unit(entry, entry)
}

/// Alice, beyond the guards' harm.
pub(crate) fn hardy(m: &mut Map, g: Guid) {
    let f = &mut m.players.get_mut(&g).unwrap().fields;
    f.set_u32(UNIT_FIELD_MAXHEALTH, 100_000_000);
    f.set_u32(UNIT_FIELD_HEALTH, 100_000_000);
}

/// A creature standing in `town` (no navmesh here to say so).
pub(crate) fn in_town(m: &mut Map, g: Guid, town: u32) {
    m.creature_mut(g).unwrap().zone_area = Some((town, town));
}

/// The guards of `entry` called, in the order they came.
pub(crate) fn called(m: &Map, entry: u32) -> Vec<Guid> {
    let mut out: Vec<(u32, Guid)> = m
        .creatures
        .iter()
        .filter(|(_, c)| c.entry == entry && c.spawn_guid.is_none())
        .map(|(_, c)| (guid::counter(c.guid), c.guid))
        .collect();
    out.sort_unstable();
    out.into_iter().map(|(_, g)| g).collect()
}

pub(crate) fn set_alarm(m: &mut Map, town: u32, guards: u32) {
    let now = m.now;
    m.guards.alarms.insert(
        town,
        Alarm {
            guards,
            last_call: now,
        },
    );
}

pub(crate) fn avoidance(m: &Map, g: Guid) -> i32 {
    auras::auras(m, g).map_or(0, |a| a.sum(AURA_MOD_AOE_AVOIDANCE))
}

/// mangos's `AttackStart` and `HandleSpawnGuard`: a civilian calls a guard
/// each time it takes a victim, a new one or another (by a taunt, or as
/// its threat turns), with no charges and no cooldown; each comes within
/// 30 yards of it and fights its attacker; the town's alarm counts them.
#[tokio::test]
async fn a_civilian_calls_a_guard_for_each_victim_it_takes() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let civ = unit(CIVILIAN);
    a.sees(civ).await;
    let (first, after_switches, helper) = on(&map, move |m| {
        hardy(m, ga);
        in_town(m, civ, STORMWIND);
        ai::attacked(m, civ, ga);
        let first = called(m, CITY_GUARD);
        let helper = spawn(m, ga, HELPER, 20, 6.0);
        for i in 0..12 {
            ai::taunted(m, civ, if i % 2 == 0 { helper } else { ga });
        }
        (first, called(m, CITY_GUARD), helper)
    })
    .await;
    assert_eq!(first.len(), 1, "a guard for its first victim");
    assert_eq!(after_switches.len(), 13, "and one for each other it took");
    let seen = on(&map, move |m| {
        let at = m.creature(civ).unwrap().position;
        let guards = called(m, CITY_GUARD);
        let places: Vec<f32> = guards
            .iter()
            .map(|g| dist(m.creature(*g).unwrap().position, at))
            .collect();
        let victims: Vec<Guid> = guards
            .iter()
            .map(|g| m.creature(*g).unwrap().combat.victim)
            .collect();
        (places, victims, guards::alarm(m, STORMWIND))
    })
    .await;
    let (places, victims, alarm) = seen;
    assert!(
        places.iter().all(|d| *d <= guards::SUMMON_RADIUS),
        "{places:?}"
    );
    // Each fights whom the civilian took as it came.
    let expected: Vec<Guid> = std::iter::once(ga)
        .chain((0..12).map(|i| if i % 2 == 0 { helper } else { ga }))
        .collect();
    assert_eq!(victims, expected);
    assert_eq!(alarm, 13);
    // Its threat turning to another, it calls again (the fight's own turn).
    on(&map, move |m| ai::add_threat(m, civ, helper, 1.0e9)).await;
    until(&map, "a guard for the threat's turn", move |m| {
        called(m, CITY_GUARD).len() == 14
    })
    .await;
    let last = on(&map, move |m| {
        let g = *called(m, CITY_GUARD).last().unwrap();
        m.creature(g).unwrap().combat.victim
    })
    .await;
    assert_eq!(last, helper);
}

/// The user's two changes to mangos: a guard goes five minutes after its
/// fight (not 30 seconds), and at once when it dies; out of a fight, it
/// wanders about where it came.
#[tokio::test]
async fn a_guard_goes_five_minutes_out_of_combat_or_at_death_and_wanders_meanwhile() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let civ = unit(CIVILIAN);
    a.sees(civ).await;
    let (kept, gone, died) = on(&map, move |m| {
        hardy(m, ga);
        in_town(m, civ, STORMWIND);
        ai::attacked(m, civ, ga);
        let helper = spawn(m, ga, HELPER, 20, 6.0);
        ai::taunted(m, civ, helper);
        ai::taunted(m, civ, ga);
        let [g1, g2, _g3] = called(m, CITY_GUARD)[..] else {
            panic!("three guards");
        };
        // Out of its fight, its five minutes begin.
        ai::enter_evade(m, g1);
        let start = m.now;
        let due = event_ai::scripted(m, g1).and_then(|s| s.despawn_at);
        assert_eq!(due, Some(start + Duration::from_secs(300)));
        m.now = start + Duration::from_secs(299);
        event_ai::update_despawns(m);
        let kept = m.creature(g1).is_some();
        m.now = start + Duration::from_secs(301);
        event_ai::update_despawns(m);
        let gone = m.creature(g1).is_none();
        m.now = start;
        ai::die(m, g2, ga);
        (kept, gone, m.creature(g2).is_none())
    })
    .await;
    assert!(kept, "still there a second short of five minutes");
    assert!(gone, "gone at five minutes");
    assert!(died, "gone at its death");
    let g3 = on(&map, move |m| {
        let g = called(m, CITY_GUARD)[0];
        ai::enter_evade(m, g);
        let c = m.creature(g).unwrap();
        assert_eq!(c.movement_type, MOVEMENT_RANDOM);
        assert_eq!(c.wander_distance, 10.0);
        g
    })
    .await;
    until(&map, "it wanders about where it came", move |m| {
        m.creature(g3).is_some_and(|c| match &c.motion {
            Motion::Moving(w) => !c.evading && dist(w.destination(), c.home) <= 10.0 + 0.01,
            Motion::Idle { .. } => false,
        })
    })
    .await;
}

/// The user, after the raid: "the guards run back to their spawn point
/// and stand still." A called guard that has fought (run after its foe,
/// who has died), evaded and come home walks again about where it came,
/// within `[guards] wander_yards` (10), as a spawn with random movement
/// does after its evade (mangos's motion master back at its default).
#[tokio::test]
async fn a_guard_that_fought_evaded_and_came_home_wanders_again() {
    let (realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let civ = unit(CIVILIAN);
    a.sees(civ).await;
    // Bob watches: a creature no one sees doesn't move.
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    b.heartbeat(START.0, START.1, START.2).await;
    let g = on(&map, move |m| {
        hardy(m, ga);
        in_town(m, civ, STORMWIND);
        ai::attacked(m, civ, ga);
        called(m, CITY_GUARD)[0]
    })
    .await;
    b.sees(g).await;
    let (wander, home) = on(&map, move |m| {
        let c = m.creature(g).unwrap();
        (c.wander_distance, c.home)
    })
    .await;
    assert_eq!(wander, 10.0, "the default");
    // Alice runs off, 25 yards past where it came, the guard after her;
    // she dies, and it goes home.
    let away = if home.x > START.0 { -25.0 } else { 25.0 };
    a.heartbeat(home.x + away, home.y, home.z).await;
    until(&map, "it runs after her", move |m| {
        m.creature(g)
            .is_some_and(|c| dist(c.position, c.home) > 10.0)
    })
    .await;
    on(&map, move |m| {
        crate::game::map::death::kill(m, ga);
        // Nobody left to fight: it gives up, as it would on its next look.
        if !m.creature(g).unwrap().evading {
            ai::enter_evade(m, g);
        }
    })
    .await;
    until(&map, "it is home", move |m| {
        m.creature(g)
            .is_some_and(|c| !c.evading && dist(c.position, c.home) < 1.0)
    })
    .await;
    until(
        &map,
        "it walks again within 10 yards of where it came",
        move |m| {
            m.creature(g).is_some_and(|c| match &c.motion {
                Motion::Moving(w) => {
                    !c.evading
                        && dist(w.destination(), c.home) > 0.1
                        && dist(w.destination(), c.home) <= 10.0 + 0.01
                }
                Motion::Idle { .. } => false,
            })
        },
    )
    .await;
}

/// A town's alarm counts its guards and halves for each five minutes
/// without a call, so that a later raid starts it afresh.
#[tokio::test]
async fn a_towns_alarm_counts_its_guards_and_cools() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let civ = unit(CIVILIAN);
    a.sees(civ).await;
    let seen = on(&map, move |m| {
        hardy(m, ga);
        in_town(m, civ, STORMWIND);
        set_alarm(m, STORMWIND, 64);
        let start = m.now;
        let mut seen = vec![guards::alarm(m, STORMWIND)];
        m.now = start + Duration::from_secs(299);
        seen.push(guards::alarm(m, STORMWIND));
        m.now = start + Duration::from_secs(300);
        seen.push(guards::alarm(m, STORMWIND));
        m.now = start + Duration::from_secs(900);
        seen.push(guards::alarm(m, STORMWIND));
        // A call now counts from what is left.
        ai::attacked(m, civ, ga);
        seen.push(guards::alarm(m, STORMWIND));
        m.now = start + Duration::from_secs(900 + 299);
        seen.push(guards::alarm(m, STORMWIND));
        m.now = start;
        seen
    })
    .await;
    assert_eq!(seen, [64, 64, 32, 8, 9, 9]);
}

/// Past 20 guards in the alarm a guard comes with Avoidance, 5% more for
/// each ten more, to 50%; it keeps it through an evade, and an area spell
/// misses it as often as that.
#[tokio::test]
async fn guards_past_the_threshold_come_with_avoidance_and_area_spells_miss_them() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Mage).await;
    let civ = unit(CIVILIAN);
    a.sees(civ).await;
    let seen = on(&map, move |m| {
        hardy(m, ga);
        m.players
            .get_mut(&ga)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_LEVEL, 20);
        in_town(m, civ, STORMWIND);
        let helper = spawn(m, ga, HELPER, 20, 6.0);
        ai::attacked(m, civ, ga);
        let mut amounts = Vec::new();
        for (i, alarm) in [19, 20, 29, 30, 65, 110, 1000].into_iter().enumerate() {
            set_alarm(m, STORMWIND, alarm);
            ai::taunted(m, civ, if i % 2 == 0 { helper } else { ga });
            let newest = [CITY_GUARD, ROYAL_GUARD]
                .into_iter()
                .flat_map(|e| called(m, e))
                .max_by_key(|g| guid::counter(*g))
                .unwrap();
            amounts.push(avoidance(m, newest));
        }
        let plain = called(m, CITY_GUARD)[0];
        let shielded = *called(m, ROYAL_GUARD).last().unwrap();
        ai::enter_evade(m, shielded);
        (amounts, plain, shielded)
    })
    .await;
    let (amounts, plain, shielded) = seen;
    assert_eq!(amounts, [0, 5, 5, 10, 25, 50, 50]);
    until(&map, "its Avoidance back after its evade", move |m| {
        m.creature(shielded).is_some_and(|c| !c.evading) && avoidance(m, shielded) == 50
    })
    .await;
    let (chances, hits) = on(&map, move |m| {
        let data = m.realm.data.clone();
        let flames = data.spells.get(FLAMES).unwrap();
        let chance = |m: &Map, g| outcome::spell_hit_chance(m, ga, g, flames, &Mods::default());
        let chances = (chance(m, plain), chance(m, shielded));
        m.spells.rolls = Some(true);
        let mut hits = (0, 0);
        for _ in 0..4000 {
            if outcome::roll_hit(m, ga, plain, flames, &Mods::default()) == Hit::Hit {
                hits.0 += 1;
            }
            if outcome::roll_hit(m, ga, shielded, flames, &Mods::default()) == Hit::Hit {
                hits.1 += 1;
            }
        }
        m.spells.rolls = None;
        (chances, hits)
    })
    .await;
    assert_eq!(chances, (9_600, 4_600), "half the area's hits gone");
    // 96% and 46% of 4000, within five standard deviations.
    assert!((3_760..=3_950).contains(&hits.0), "{hits:?}");
    assert!((1_680..=2_000).contains(&hits.1), "{hits:?}");
}

/// Past 40 in the alarm, a call brings two of the capital's elite:
/// Stormwind's Royal Guard; Ironforge, with none, sends two of its guards.
#[tokio::test]
async fn past_the_second_threshold_a_call_brings_two_of_the_elite() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let (civ, dwarf) = (unit(CIVILIAN), unit(DWARF));
    a.sees(dwarf).await;
    let seen = on(&map, move |m| {
        hardy(m, ga);
        in_town(m, civ, STORMWIND);
        in_town(m, dwarf, IRONFORGE);
        set_alarm(m, STORMWIND, 39);
        ai::attacked(m, civ, ga);
        let below = (called(m, CITY_GUARD).len(), called(m, ROYAL_GUARD).len());
        let helper = spawn(m, ga, HELPER, 20, 6.0);
        ai::taunted(m, civ, helper);
        let royal = called(m, ROYAL_GUARD)
            .iter()
            .map(|g| m.creature(*g).unwrap().combat.victim)
            .collect::<Vec<_>>();
        set_alarm(m, IRONFORGE, 40);
        ai::attacked(m, dwarf, ga);
        (
            below,
            royal,
            guards::alarm(m, STORMWIND),
            called(m, IRONFORGE_GUARD).len(),
            guards::alarm(m, IRONFORGE),
            helper,
        )
    })
    .await;
    let (below, royal, stormwind, ironforge, ironforge_alarm, helper) = seen;
    assert_eq!(below, (1, 0), "one guard below the threshold");
    assert_eq!(royal, [helper, helper], "two Royal Guards past it");
    assert_eq!(stormwind, 42);
    assert_eq!(ironforge, 2, "Ironforge's two guards");
    assert_eq!(ironforge_alarm, 42);
}

/// Thrall, an orc in the Valley of Trials, beside whom Jaina's spawn is.
async fn orc(
    realm: &std::sync::Arc<crate::game::Realm>,
) -> (Client, Guid, crate::game::map::MapHandle) {
    let mut b = Client::connect(realm, "B");
    let gb = b.enter("Thrall", Race::Orc, Class::Warrior).await;
    let kalimdor = realm.registry.online(gb).unwrap().map();
    assert_eq!(kalimdor.key, MapKey::world(1));
    b.heartbeat(ORC_START.0, ORC_START.1, ORC_START.2).await;
    b.sees(guid::unit(jaina::JAINA, jaina::SPAWN)).await;
    (b, gb, kalimdor)
}

pub(crate) fn heard(m: &ServerOpcodeMessage, lines: &[&str]) -> bool {
    matches!(m, ServerOpcodeMessage::SMSG_MESSAGECHAT(c) if lines.iter().any(|l| c.message == *l))
}

/// Whether a packet is the teleport's flash on a Jaina.
fn flash(m: &ServerOpcodeMessage) -> bool {
    matches!(m, ServerOpcodeMessage::SMSG_SPELL_GO(s)
        if s.spell == jaina::TELEPORT_VISUAL && guid::entry(s.caster) == jaina::JAINA)
}

pub(crate) fn place(m: &Map) -> Place {
    m.realm.social.jaina.lock().place
}

/// Jaina's spawn in Theramore: whether it is alive.
fn home_alive(m: &Map) -> Option<bool> {
    crate::game::map::scripts::creature_of_spawn(m, jaina::SPAWN)
        .and_then(|g| m.creature(g))
        .map(|c| c.alive())
}

const LEAVING: &[&str] = &[
    "I have urgent matters to tend to. Tervosh, Theramore is in your hands until I return.",
    "Stormwind calls for aid. I will not be long.",
    "Forgive me, I am needed elsewhere. Keep watch while I am gone.",
];
const ARRIVING: &[&str] = &[
    "You dare bring war to the heart of Stormwind? Not while I stand!",
    "Stand fast, Stormwind! Help has come.",
    "Enough of this! Leave this city, or I will see you leave it.",
];
pub(crate) const GOING: &[&str] = &[
    "The city is safe for now. Theramore needs me.",
    "Stormwind can look after itself again. I must return home.",
];

/// Stormwind's alarm reaching 60, a call brings Jaina: she says so in
/// Theramore and teleports out; she comes into Stormwind by the fight with
/// the teleport's flash, seen after her create, says so, and fights the
/// attacker. She is in one place at a time; at peace a minute, she says
/// farewell and is home again with the flash, to be called again.
#[tokio::test]
async fn jaina_comes_to_stormwind_at_the_third_threshold_and_goes_home_after() {
    let (realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let civ = unit(CIVILIAN);
    a.sees(civ).await;
    let (mut b, _gb, kalimdor) = orc(&realm).await;
    let below = on(&map, move |m| {
        hardy(m, ga);
        in_town(m, civ, STORMWIND);
        // Two Royal Guards past 40: 59.
        set_alarm(m, STORMWIND, 57);
        ai::attacked(m, civ, ga);
        (guards::alarm(m, STORMWIND), place(m))
    })
    .await;
    assert_eq!(below, (59, Place::Home), "59: not yet");
    a.clear();
    // The call that takes it past 60 is against a creature of Alice's side.
    let helper = on(&map, move |m| {
        let helper = spawn(m, ga, HELPER, 20, 6.0);
        ai::taunted(m, civ, helper);
        helper
    })
    .await;
    // Theramore sees her off.
    b.wait_for("her farewell to Theramore", |m| {
        heard(m, LEAVING).then_some(())
    })
    .await;
    b.wait_for("her flash out", |m| flash(m).then_some(()))
        .await;
    b.forgets(guid::unit(jaina::JAINA, jaina::SPAWN)).await;
    // Stormwind sees her come: her create, then the flash.
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    let copy = loop {
        let sent: Vec<ServerOpcodeMessage> = a
            .captured()
            .iter()
            .filter_map(|f| ServerOpcodeMessage::read_unencrypted(&f[..]).ok())
            .collect();
        let created = sent.iter().enumerate().find_map(|(i, m)| {
            harness::creates(m)
                .into_iter()
                .find(|g| guid::entry(*g) == jaina::JAINA)
                .map(|g| (i, g))
        });
        let flashed = sent.iter().position(flash);
        if let (Some((created, g)), Some(flashed)) = (created, flashed) {
            assert!(created < flashed, "her flash before her create");
            break g;
        }
        assert!(tokio::time::Instant::now() < deadline, "she never came");
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    a.wait_for("her words to Stormwind", |m| {
        heard(m, ARRIVING).then_some(())
    })
    .await;
    let (there, fights) = on(&map, move |m| {
        let c = m.creature(copy).unwrap();
        (place(m), c.combat.attacking && c.combat.victim == helper)
    })
    .await;
    assert_eq!(
        there,
        Place::Away {
            map: MapKey::world(0),
            copy
        }
    );
    assert!(fights, "she fights the civilian's attacker");
    assert_eq!(
        on(&kalimdor, |m| home_alive(m)).await,
        Some(false),
        "gone from Theramore"
    );
    // At peace a minute, she goes home.
    on(&map, move |m| {
        ai::enter_evade(m, copy);
        for g in called(m, CITY_GUARD) {
            ai::enter_evade(m, g);
        }
    })
    .await;
    until(&map, "her walk home done", move |m| {
        m.creature(copy)
            .is_some_and(|c| !c.evading && !c.combat.attacking)
    })
    .await;
    on(&map, move |m| {
        m.guards.jaina.calm_since = m.now.checked_sub(Duration::from_secs(61));
    })
    .await;
    a.wait_for("her farewell to Stormwind", |m| {
        heard(m, GOING).then_some(())
    })
    .await;
    a.forgets(copy).await;
    until(&kalimdor, "home again", |m| {
        place(m) == Place::Home && home_alive(m) == Some(true)
    })
    .await;
    b.wait_for("her flash home", |m| flash(m).then_some(()))
        .await;
    assert!(on(&map, move |m| m.creature(copy).is_none()).await);
    // Still alarmed, Stormwind calls her again.
    on(&map, move |m| {
        let helper = spawn(m, ga, HELPER, 20, 6.0);
        ai::taunted(m, civ, helper);
    })
    .await;
    until(&map, "called again", |m| {
        matches!(place(m), Place::Away { .. }) && m.realm.social.jaina.lock().visits == 2
    })
    .await;
}

/// The Alliance's side (Theramore's, Stormwind's): the Horde's enemy.
pub(crate) const ALLIANCE: u32 = 960_200;
/// An orc player's faction template.
pub(crate) const ORCS: u32 = 2;

/// A Horde raider in Stormwind: an orc, brought from the Valley of Trials
/// to `at` in the Eastern Kingdoms, beyond harm.
pub(crate) async fn raider(
    realm: &std::sync::Arc<crate::game::Realm>,
    name: &str,
    at: (f32, f32, f32),
) -> (Client, Guid) {
    let mut c = Client::connect(realm, name);
    let g = c.enter(name, Race::Orc, Class::Warrior).await;
    c.say(&format!(".go xyz {} {} {} 0", at.0, at.1, at.2))
        .await;
    c.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    c.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    c.sees(g).await;
    let map = realm.maps.get(realm, MapKey::world(0));
    on(&map, move |m| hardy(m, g)).await;
    (c, g)
}

/// The user's raid: Jaina came to Stormwind "but she just stands still and
/// doesn't attack anything, even though there are 80 horde in front of
/// her." Her template takes no one on sight (`flags_extra` 0x2, so
/// `Reaction::Defensive`), and the raider whose fight called her had died
/// as she came. In Stormwind she takes on a Horde player within her reach,
/// and when it dies, the next; at home in Theramore she keeps her no-aggro
/// (Thrall, beside her there, is left alone).
#[tokio::test]
async fn jaina_in_stormwind_takes_on_the_horde_though_the_one_she_came_for_died() {
    let mut d = world();
    d.faction_templates.extend([
        (ALLIANCE, faction(ALLIANCE, 0x1, 0x4)),
        (ORCS, faction(ORCS, 0x4, 0x1)),
    ]);
    // Her real template's flag and reach; the city's people and guards of
    // her side, and as before taking no one on sight (their reach), so
    // that only she can take the raiders on.
    let j = d.creature_templates.get_mut(&jaina::JAINA).unwrap();
    j.flags_extra |= 0x2;
    j.detection_range = 20.0;
    j.faction = ALLIANCE;
    for e in [CIVILIAN, CITY_GUARD, ROYAL_GUARD] {
        d.creature_templates.get_mut(&e).unwrap().faction = ALLIANCE;
    }
    let (realm, mut a, ga, map) = setup_with(d, Class::Warrior).await;
    let civ = unit(CIVILIAN);
    a.sees(civ).await;
    on(&map, move |m| hardy(m, ga)).await;
    // At home she lets Thrall be.
    let (_b, gb, kalimdor) = orc(&realm).await;
    let start = on(&kalimdor, |m| m.stats.updates).await;
    until(&kalimdor, "her looks about her at home", move |m| {
        m.stats.updates > start + 20
    })
    .await;
    let at_home = on(&kalimdor, move |m| {
        let g = crate::game::map::scripts::creature_of_spawn(m, jaina::SPAWN).unwrap();
        let c = m.creature(g).unwrap();
        (
            crate::game::map::assistance::reaction(m, g),
            c.combat.attacking && c.combat.victim == gb,
        )
    })
    .await;
    assert_eq!(
        at_home,
        (crate::game::map::assistance::Reaction::Defensive, false),
        "no aggro at home"
    );
    // The raiders: the one whose fight calls her, and two more.
    let (_h0, foe) = raider(&realm, "Foe", (START.0 - 3.0, START.1, START.2)).await;
    let (_h1, g1) = raider(&realm, "Grunt", (START.0 - 3.0, START.1 + 2.0, START.2)).await;
    let (_h2, g2) = raider(&realm, "Peon", (START.0 - 3.0, START.1 - 2.0, START.2)).await;
    let called = on(&map, move |m| {
        in_town(m, civ, STORMWIND);
        // Two Royal Guards from 58: 60, and she is called.
        set_alarm(m, STORMWIND, 58);
        ai::attacked(m, civ, foe);
        // The one she came for dies before she comes.
        crate::game::map::death::kill(m, foe);
        place(m)
    })
    .await;
    assert_ne!(called, Place::Home, "she is called");
    until(&map, "she comes", |m| {
        matches!(place(m), Place::Away { .. })
    })
    .await;
    let copy = on(&map, |m| match place(m) {
        Place::Away { copy, .. } => copy,
        _ => unreachable!(),
    })
    .await;
    let fights = move |m: &Map, g: Guid| {
        m.creature(copy)
            .is_some_and(|c| c.combat.attacking && c.combat.victim == g)
    };
    until(&map, "she takes on a raider before her", move |m| {
        fights(m, g1) || fights(m, g2)
    })
    .await;
    let (first, next) = on(&map, move |m| {
        let first = if fights(m, g1) { g1 } else { g2 };
        crate::game::map::death::kill(m, first);
        (first, if first == g1 { g2 } else { g1 })
    })
    .await;
    assert_ne!(first, next);
    until(&map, "and the next after that one dies", move |m| {
        fights(m, next)
    })
    .await;
    let home = on(&kalimdor, |m| {
        crate::game::map::scripts::creature_of_spawn(m, jaina::SPAWN)
            .map(|g| crate::game::map::assistance::reaction(m, g))
    })
    .await;
    assert_eq!(
        home,
        Some(crate::game::map::assistance::Reaction::Defensive),
        "her home self keeps her no-aggro"
    );
}

/// Dead in Stormwind, her corpse lies there, and she is back in Theramore
/// after her respawn time (a day), to be called by a later raid.
#[tokio::test]
async fn jaina_dead_in_stormwind_is_back_home_after_her_respawn_time() {
    let (realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let civ = unit(CIVILIAN);
    a.sees(civ).await;
    let kalimdor = realm.maps.get(&realm, MapKey::world(1));
    on(&map, move |m| {
        hardy(m, ga);
        in_town(m, civ, STORMWIND);
        set_alarm(m, STORMWIND, 70);
        ai::attacked(m, civ, ga);
    })
    .await;
    until(&map, "she came", |m| matches!(place(m), Place::Away { .. })).await;
    let copy = on(&map, |m| match place(m) {
        Place::Away { copy, .. } => copy,
        _ => unreachable!(),
    })
    .await;
    let lies = on(&map, move |m| {
        ai::die(m, copy, ga);
        (place(m), m.creature(copy).map(|c| c.alive()))
    })
    .await;
    assert_eq!(
        lies,
        (Place::Home, Some(false)),
        "her corpse lies in Stormwind"
    );
    until(&kalimdor, "her respawn put at a day", |m| {
        let now = m.now;
        crate::game::map::scripts::creature_of_spawn(m, jaina::SPAWN)
            .and_then(|g| m.creature(g))
            .is_some_and(|c| match c.life {
                crate::game::map::creature::Life::Dead { respawn_at, .. } => {
                    let left = respawn_at.saturating_duration_since(now).as_secs();
                    (86_300..=86_400).contains(&left)
                }
                _ => false,
            })
    })
    .await;
    // A day on, she is home.
    on(&kalimdor, |m| {
        let now = m.now;
        let g = crate::game::map::scripts::creature_of_spawn(m, jaina::SPAWN).unwrap();
        if let crate::game::map::creature::Life::Dead { respawn_at, .. } =
            &mut m.creature_mut(g).unwrap().life
        {
            *respawn_at = now;
        }
    })
    .await;
    until(&kalimdor, "alive at home", |m| home_alive(m) == Some(true)).await;
    // A later raid calls her again.
    on(&map, move |m| {
        let helper = spawn(m, ga, HELPER, 20, 6.0);
        ai::taunted(m, civ, helper);
    })
    .await;
    until(&map, "called by the next raid", |m| {
        matches!(place(m), Place::Away { .. }) && m.realm.social.jaina.lock().visits == 2
    })
    .await;
}

/// In Stormwind her Teleport sends its victim to the Stockade's door, not
/// across the sea to her tower, and her 40-yard rule is about that door.
#[tokio::test]
async fn jainas_teleport_in_stormwind_sends_its_victim_to_the_stockade() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let (at, stand_in, at_home) = on(&map, move |m| {
        hardy(m, ga);
        let copy = spawn(m, ga, jaina::JAINA, 20, 5.0);
        let other = spawn(m, ga, PACK_MAGE, 20, 5.0);
        m.realm.social.jaina.lock().place = Place::Away { map: m.key, copy };
        let tower = [TOWER.0, TOWER.1, TOWER.2];
        let stand_in = (
            jaina::stand_in(m, Some(copy), tower),
            jaina::stand_in(m, Some(other), tower),
        );
        crate::game::map::spells::cast_triggered(m, copy, jaina::TELEPORT, Some(ga));
        let at = m.players[&ga].position();
        m.realm.social.jaina.lock().place = Place::Home;
        (at, stand_in, jaina::teleport_to(m, copy, jaina::TELEPORT))
    })
    .await;
    let door = Vector3d {
        x: -8766.11,
        y: 845.499,
        z: 87.9952,
    };
    assert!(dist(at, door) < 0.1, "{at:?}");
    assert_eq!(stand_in.0, [door.x, door.y, door.z]);
    assert_eq!(
        stand_in.1,
        [TOWER.0, TOWER.1, TOWER.2],
        "another's is its own"
    );
    assert_eq!(at_home, None, "at home, her tower's");
}

/// A creature of the pack that isn't Jaina.
const PACK_MAGE: u32 = crate::game::tests_p3_combat::BRUTE;

// The real world, in the extended suite.

pub(crate) static BAKE: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| harness::real_data("nav"));

/// On the real navmesh, a civilian in Stormwind's Trade District calls a
/// Stormwind City Guard to a point a path from it reaches, within 30
/// yards; the town is known from the navmesh's area.
#[tokio::test]
#[ignore = "extended"]
async fn on_the_navmesh_a_stormwind_civilian_calls_a_guard_it_can_reach() {
    let bake = BAKE.as_path();
    if !bake.join("Azeroth.map").exists() {
        return;
    }
    // Before Stormwind's auction house.
    const TRADE: (f32, f32, f32) = (-8816.0, 660.0, 96.0);
    let mut d = world();
    if let Some(s) = d.creature_spawns.get_mut(&0) {
        for s in s.iter_mut().filter(|s| s.guid == CIVILIAN) {
            (s.x, s.y, s.z) = TRADE;
        }
    }
    let mut cfg = harness::test_config();
    cfg.nav_dir = Some(BAKE.clone());
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = crate::game::Realm::new(cfg, store, startup, d, None);
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = on(&map, move |m| {
        let civ = unit(CIVILIAN);
        // On the ground there.
        let ground = m
            .nav
            .as_mut()
            .and_then(|n| n.ground(TRADE.0, TRADE.1, TRADE.2))
            .expect("Stormwind's ground");
        let at = Vector3d {
            x: TRADE.0,
            y: TRADE.1,
            z: ground,
        };
        m.creature_mut(civ).unwrap().position = at;
        let town = crate::game::map::world::zone_and_area(m, at);
        let t = m.realm.data.creature_template(HELPER).cloned().unwrap();
        let mut foes = Vec::new();
        for i in 0..2 {
            let c = crate::game::map::creature::Creature::new(
                guid::unit(HELPER, 0x80_0100 + i),
                &t,
                20,
                Vector3d {
                    x: at.x + 3.0,
                    y: at.y + i as f32,
                    ..at
                },
                0.0,
            )
            .with_base();
            foes.push(m.add_creature(c));
        }
        ai::attacked(m, civ, foes[0]);
        for i in 0..9 {
            ai::taunted(m, civ, foes[(i + 1) % 2]);
        }
        let mut out = Vec::new();
        for g in called(m, CITY_GUARD) {
            let p = m.creature(g).unwrap().position;
            let path = m.nav.as_mut().and_then(|n| n.path(at, p));
            let reached = path
                .and_then(|p| p.last().copied())
                .is_some_and(|e| dist(e, p) <= 1.0);
            let victim = m.creature(g).unwrap().combat.victim;
            // On the ground's plane, as mangos's `GetRandomPoint` measures
            // it (the Trade District slopes: in three, 30.4 once).
            let flat = ((p.x - at.x).powi(2) + (p.y - at.y).powi(2)).sqrt();
            out.push((flat, reached, foes.contains(&victim)));
        }
        (town, out)
    })
    .await;
    let (town, out) = out;
    let (zone, area) = town.expect("the navmesh knows where it is");
    assert_eq!(guards::town(area, zone), Some(STORMWIND), "{zone} {area}");
    assert!(!out.is_empty(), "a guard came");
    for (d, reached, fights) in out {
        assert!(d <= guards::SUMMON_RADIUS, "{d}");
        assert!(reached, "a path reaches it");
        assert!(fights);
    }
}

/// On the real navmesh: Jaina, fighting in Stormwind, fights on; drawn out
/// of it (to Goldshire, say), she goes back to where she came.
#[tokio::test]
#[ignore = "extended"]
async fn jaina_is_leashed_to_stormwind() {
    let bake = BAKE.as_path();
    if !bake.join("Azeroth.map").exists() {
        return;
    }
    const TRADE: (f32, f32, f32) = (-8816.0, 660.0, 96.0);
    const GOLDSHIRE: (f32, f32, f32) = (-9464.0, 62.0, 56.0);
    let mut cfg = harness::test_config();
    cfg.nav_dir = Some(BAKE.clone());
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = crate::game::Realm::new(cfg, store, startup, world(), None);
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = on(&map, |m| {
        let data = m.realm.data.clone();
        let at = |m: &mut Map, (x, y, z): (f32, f32, f32)| {
            let z = m.nav.as_mut().and_then(|n| n.ground(x, y, z)).unwrap_or(z);
            Vector3d { x, y, z }
        };
        let (trade, goldshire) = (at(m, TRADE), at(m, GOLDSHIRE));
        let make = |m: &mut Map, entry: u32, p: Vector3d| {
            let t = data.creature_template(entry).cloned().unwrap();
            let counter = m.next_creature_counter;
            m.next_creature_counter += 1;
            let c = crate::game::map::creature::Creature::new(
                guid::unit(entry, counter | 0x80_0000),
                &t,
                20,
                p,
                0.0,
            )
            .with_base();
            m.add_creature(c)
        };
        let mut seen = Vec::new();
        for p in [trade, goldshire] {
            let copy = make(m, jaina::JAINA, p);
            let foe = make(m, HELPER, Vector3d { x: p.x + 2.0, ..p });
            m.realm.social.jaina.lock().place = Place::Away { map: m.key, copy };
            ai::attacked(m, copy, foe);
            m.guards.jaina.look_at = None;
            jaina::update(m);
            seen.push(m.creature(copy).unwrap().evading);
        }
        m.realm.social.jaina.lock().place = Place::Home;
        seen
    })
    .await;
    assert_eq!(
        out,
        [false, true],
        "in Stormwind she fights on; beyond it, she goes back"
    );
}

/// The real data: what the guards and Jaina are made of is there as the
/// code takes it. The civilians and Jaina call the guards and the guards
/// don't; every guard the table names is a creature; the elite are; her
/// spawn, her script (Thistle Tea's, imported) and gossip, her spells, and
/// the Stockade's exit.
#[test]
#[ignore = "extended"]
fn the_real_data_has_the_guards_and_jaina() {
    let Some(d) = crate::game::data::load::real_data() else {
        return;
    };
    let calls = |e: u32| {
        d.creature_template(e)
            .is_some_and(|t| t.static_flags1 & guards::STATIC_CALLS_GUARDS != 0)
    };
    assert!(calls(jaina::JAINA), "Jaina calls the guards");
    assert!(!calls(CITY_GUARD) && !calls(ROYAL_GUARD), "guards don't");
    let civilians = d
        .creature_templates
        .values()
        .filter(|t| t.static_flags1 & guards::STATIC_CALLS_GUARDS != 0)
        .count();
    assert!(civilians > 1000, "{civilians}");
    for town in [1519, 1537, 1657, 1637, 1638, 1497, 87, 513, 362, 221] {
        let (a, h) = guards::guards_of(town).unwrap();
        for e in [a, h].into_iter().filter(|e| *e != 0 && *e != u32::MAX) {
            assert!(d.creature_template(e).is_some(), "{town}'s {e}");
        }
    }
    for e in [1756, 14304, 13839, 3083, 3210, 3211, 3213, 3214] {
        let t = d.creature_template(e).unwrap_or_else(|| panic!("{e}"));
        assert!(t.level_max >= 55, "{e}");
    }
    let spawn = d
        .creature_spawns
        .get(&1)
        .and_then(|v| v.iter().find(|s| s.guid == jaina::SPAWN))
        .expect("her spawn");
    assert_eq!(spawn.entries, [jaina::JAINA]);
    assert_eq!((spawn.respawn_min, spawn.respawn_max), (86_400, 86_400));
    assert!((spawn.x + 3749.69).abs() < 0.1 && (spawn.y + 4454.15).abs() < 0.1);
    let events = &d.scripts.ai_events[&jaina::JAINA];
    assert_eq!(events.len(), 3, "her aggro and her two timers");
    assert!(
        events[1..]
            .iter()
            .all(|e| e.flags & crate::game::data::scripts::EF_NOT_CASTING != 0),
        "her timers wait while she casts"
    );
    let menu = d.creature_template(jaina::JAINA).unwrap().gossip_menu_id;
    assert_eq!(d.npc.gossip_menus[&menu].texts.len(), 2);
    let tower = d.spells.target_positions[&jaina::TELEPORT];
    assert_eq!(tower.0, 1);
    assert!((tower.1 - TOWER.0).abs() < 0.1 && (tower.2 - TOWER.1).abs() < 0.1);
    let avoidance = d.spells.get(guards::AVOIDANCE).unwrap();
    assert_eq!(avoidance.effects[0].aura, AURA_MOD_AOE_AVOIDANCE);
    assert!(d.spells.get(jaina::TELEPORT_VISUAL).is_some());
    let exit = d.area_triggers[&503].teleport.as_ref().unwrap();
    assert_eq!(exit.map, 0);
    assert!((exit.x + 8766.11).abs() < 0.1 && (exit.y - 845.499).abs() < 0.1);
}

/// On the real navmesh: a called guard whose fight ends where it came (its
/// foe died beside it there, so its walk home is none) walks again about
/// that spot within 10 yards, as one that walked home does.
#[tokio::test]
#[ignore = "extended"]
async fn on_the_navmesh_a_guard_that_fought_where_it_came_wanders_again() {
    let bake = BAKE.as_path();
    if !bake.join("Azeroth.map").exists() {
        return;
    }
    const TRADE: (f32, f32, f32) = (-8816.0, 660.0, 96.0);
    let mut d = world();
    if let Some(s) = d.creature_spawns.get_mut(&0) {
        for s in s.iter_mut().filter(|s| s.guid == CIVILIAN) {
            (s.x, s.y, s.z) = TRADE;
        }
    }
    let mut cfg = harness::test_config();
    cfg.nav_dir = Some(BAKE.clone());
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = crate::game::Realm::new(cfg, store, startup, d, None);
    let map = realm.maps.get(&realm, MapKey::world(0));
    // Alice watches from beside the auction house.
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.say(&format!(".go {} {} {}", TRADE.0 + 10.0, TRADE.1, TRADE.2))
        .await;
    a.wait_for("MSG_MOVE_TELEPORT_ACK", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: ga,
        movement_counter: a.teleport_order(),
        time: Default::default(),
    })
    .await;
    let civ = unit(CIVILIAN);
    a.sees(civ).await;
    let g = on(&map, move |m| {
        hardy(m, ga);
        let ground = m
            .nav
            .as_mut()
            .and_then(|n| n.ground(TRADE.0, TRADE.1, TRADE.2))
            .expect("Stormwind's ground");
        let at = Vector3d {
            x: TRADE.0,
            y: TRADE.1,
            z: ground,
        };
        m.creature_mut(civ).unwrap().position = at;
        let t = m.realm.data.creature_template(HELPER).cloned().unwrap();
        let foe = m.add_creature(
            crate::game::map::creature::Creature::new(
                guid::unit(HELPER, 0x80_0100),
                &t,
                20,
                Vector3d {
                    x: at.x + 3.0,
                    ..at
                },
                0.0,
            )
            .with_base(),
        );
        ai::attacked(m, civ, foe);
        let g = called(m, CITY_GUARD)[0];
        assert_eq!(m.creature(g).unwrap().combat.victim, foe, "it fights");
        // The fight comes to it: its foe beside the spot it came to, it
        // standing there as a chase leaves it; its foe dies.
        let home = m.creature(g).unwrap().home;
        let c = m.creature_mut(g).unwrap();
        c.position = home;
        c.motion = Motion::Idle { until: None };
        m.creature_mut(foe).unwrap().position = Vector3d {
            x: home.x + 1.0,
            ..home
        };
        ai::die(m, foe, g);
        g
    })
    .await;
    a.sees(g).await;
    until(&map, "its fight over", move |m| {
        m.creature(g)
            .is_some_and(|c| !c.combat.attacking && !c.evading)
    })
    .await;
    until(
        &map,
        "it walks again within 10 yards of where it came",
        move |m| {
            m.creature(g).is_some_and(|c| match &c.motion {
                Motion::Moving(w) => {
                    !c.evading
                        && dist(w.destination(), c.home) > 0.1
                        && dist(w.destination(), c.home) <= 10.0 + 0.01
                }
                Motion::Idle { .. } => false,
            })
        },
    )
    .await;
}
