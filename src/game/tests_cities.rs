//! Every capital's defence (`cities.rs`, `champions.rs`, `map/elites.rs`):
//! each defended city's alarm bringing its own Elite Guards; Jaina
//! answering Ironforge and Darnassus (on her own map) with their teleport
//! places; one Jaina and one Rexxar at once; Rexxar's run in, fight, words,
//! run off and going; the Horde's Elite Guards' fight. On the guards tests'
//! world (`tests_guards.rs`) with the Elite Guards put in as the loader puts
//! them, and on the real navmesh and data in the extended suite.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::champions::{self, Champion, Place};
use crate::game::cities::{self, CITIES};
use crate::game::data::*;
use crate::game::harness::{self, Client};
use crate::game::jaina;
use crate::game::map::battleground::av_scripts::WHIRLWIND;
use crate::game::map::combat::dist;
use crate::game::map::creature::Motion;
use crate::game::map::{Map, MapKey, ai, event_ai};
use crate::game::tests_elites::{elite_world_with, fire, spell_go};
use crate::game::tests_guards::{
    BAKE, CIVILIAN, called, hardy, heard, in_town, set_alarm, unit, world,
};
use crate::game::tests_p3_combat::{HELPER, START, spawn, template};
use crate::game::tests_p3_entity::on;
use crate::game::tests_p3_entity_objects::until;

/// A civilian of Kalimdor, by the orcs' start, for Darnassus.
const KALIMDOR_CIVILIAN: u32 = 3_812;
/// Where an orc starts, in the Valley of Trials (Jaina's spawn is beside
/// it in the guards tests' world).
const ORC_START: (f32, f32, f32) = (-618.518, -4251.67, 38.718);

/// Every capital's guards and elite, Rexxar and Misha at home beside the
/// orcs' start, a civilian there, and the Elite Guards put in after them.
fn cities_world() -> GameData {
    elite_world_with(|d| {
        for e in [4262, 3296, 14304, 5624, 13839, 3084, 3083, 7980] {
            let mut t = template(e, 7);
            t.flags_extra = 0x400;
            d.creature_templates.insert(e, t);
        }
        for (e, name) in [(champions::REXXAR, "Rexxar"), (champions::MISHA, "Misha")] {
            let mut t = template(e, 7);
            t.name = name.into();
            t.rank = 1;
            t.speed_run = 1.14286;
            d.creature_templates.insert(e, t);
        }
        let mut t = template(KALIMDOR_CIVILIAN, 7);
        t.static_flags1 |= crate::game::map::guards::STATIC_CALLS_GUARDS;
        t.civilian = true;
        t.flags_extra = 0x2;
        d.creature_templates.insert(KALIMDOR_CIVILIAN, t);
        let kalimdor = d.creature_spawns.entry(1).or_default();
        for (guid, entry, dx) in [
            (champions::REXXAR_SPAWN, champions::REXXAR, 6.0),
            (champions::MISHA_SPAWN, champions::MISHA, 8.0),
            (KALIMDOR_CIVILIAN, KALIMDOR_CIVILIAN, -4.0),
        ] {
            kalimdor.push(CreatureSpawn {
                guid,
                entries: vec![entry],
                map: 1,
                x: ORC_START.0 + dx,
                y: ORC_START.1,
                z: ORC_START.2,
                respawn_min: 550,
                respawn_max: 550,
                ..Default::default()
            });
        }
    })
}

/// A realm on [`cities_world`], Rexxar's runs quick (five times his run).
fn realm() -> std::sync::Arc<crate::game::Realm> {
    let mut cfg = harness::test_config();
    cfg.guards.rexxar_run_speed = 5.0;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    crate::game::Realm::new(cfg, store, startup, cities_world(), None)
}

/// Alice, a human warrior at Northshire, on `realm`.
async fn alice(
    realm: &std::sync::Arc<crate::game::Realm>,
) -> (Client, Guid, crate::game::map::MapHandle) {
    let mut a = Client::connect(realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(ga).unwrap().map();
    on(&map, move |m| hardy(m, ga)).await;
    (a, ga, map)
}

/// Thrall, an orc in the Valley of Trials, by Jaina's and Rexxar's homes.
async fn thrall(
    realm: &std::sync::Arc<crate::game::Realm>,
) -> (Client, Guid, crate::game::map::MapHandle) {
    let mut b = Client::connect(realm, "B");
    let gb = b.enter("Thrall", Race::Orc, Class::Warrior).await;
    let kalimdor = realm.registry.online(gb).unwrap().map();
    assert_eq!(kalimdor.key, MapKey::world(1));
    b.heartbeat(ORC_START.0, ORC_START.1, ORC_START.2).await;
    on(&kalimdor, move |m| hardy(m, gb)).await;
    (b, gb, kalimdor)
}

/// Ironforge's Commons (`game_tele`'s `Ironforge`), inside the mountain.
const THE_COMMONS: (f32, f32, f32) = (-4918.88, -940.406, 501.564);
/// Ironforge's explore bit (`AreaTable.dbc`).
const IRONFORGE_BIT: u32 = 689;

/// Alice, teleported to the Commons with no navmesh nor terrain,
/// is in Ironforge, zone and area, as the server's box has it (the
/// navmesh and terrain read Dun Morogh there): her zone is the one `/who`
/// and her friends see, and the city is explored. Where the guards are
/// in Ironforge, she is too.
#[tokio::test]
async fn a_player_inside_ironforge_is_in_ironforge() {
    let mut d = cities_world();
    for (id, bit, flags, name) in [
        (cities::IRONFORGE, IRONFORGE_BIT, 312, "Ironforge"),
        (1, 119, 64, "Dun Morogh"),
    ] {
        d.areas.insert(
            id,
            AreaInfo {
                id,
                map: 0,
                zone: 0,
                explore_bit: bit,
                flags,
                level: 1,
                name: name.into(),
                team: 0,
            },
        );
    }
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = crate::game::Realm::new(harness::test_config(), store, startup, d, None);
    let (mut a, ga, map) = alice(&realm).await;
    on(&map, move |m| {
        let (x, y, z) = THE_COMMONS;
        m.teleport(ga, m.key, Vector3d { x, y, z }, 0.0);
    })
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
    let explored = a
        .wait_for("SMSG_EXPLORATION_EXPERIENCE", |m| match m {
            ServerOpcodeMessage::SMSG_EXPLORATION_EXPERIENCE(e) => Some(e.area.as_int()),
            _ => None,
        })
        .await;
    assert_eq!(explored, cities::IRONFORGE, "Ironforge explored");
    let seen = on(&map, move |m| {
        let (x, y, z) = THE_COMMONS;
        let town = crate::game::map::world::zone_and_area(m, Vector3d { x, y, z })
            .and_then(|(zone, area)| crate::game::map::guards::town(area, zone));
        let p = &m.players[&ga];
        let word = p
            .fields
            .get_u32(crate::game::fields::PLAYER_EXPLORED_ZONES_1 + (IRONFORGE_BIT / 32) as u16);
        (
            (p.zone, p.area, p.online.zone()),
            word & (1 << (IRONFORGE_BIT % 32)) != 0,
            town,
        )
    })
    .await;
    let ironforge = cities::IRONFORGE;
    assert_eq!(
        seen.0,
        (ironforge, ironforge, ironforge),
        "zone, area, /who"
    );
    assert!(seen.1, "its explore bit set");
    assert_eq!(seen.2, Some(ironforge), "the guards' town");
}

fn place_of(m: &Map, who: Champion) -> Place {
    who.state(&m.realm).lock().place
}

fn visits(m: &Map, who: Champion) -> u32 {
    who.state(&m.realm).lock().visits
}

fn elite_of(city: u32) -> u32 {
    cities::city(city).unwrap().elite.entry
}

/// Each defended capital's alarm, rising through 50, brings two of its own
/// Elite Guards, named and made of its guard, against the civilian's
/// attacker; with Rexxar's cities off, the Horde's bring none.
#[tokio::test]
async fn each_capitals_alarm_brings_its_own_elite_guards() {
    let (_realm, mut a, ga, map) = {
        let realm = realm();
        let (a, ga, map) = alice(&realm).await;
        (realm, a, ga, map)
    };
    let civ = unit(CIVILIAN);
    a.sees(civ).await;
    let seen = on(&map, move |m| {
        let mut seen = Vec::new();
        for city in CITIES {
            in_town(m, civ, city.area);
            set_alarm(m, city.area, 49);
            let helper = spawn(m, ga, HELPER, 20, 6.0);
            ai::taunted(m, civ, helper);
            let came = called(m, city.elite.entry);
            let looks = m.realm.data.creature_template(city.elite.looks).cloned();
            let made: Vec<(String, u32, u32, Guid)> = came
                .iter()
                .map(|e| {
                    let c = m.creature(*e).unwrap();
                    let t = m.realm.data.creature_template(c.entry).unwrap();
                    (t.name.clone(), t.faction, t.display_ids[0], c.combat.victim)
                })
                .collect();
            seen.push((city.area, made, looks, helper));
        }
        seen
    })
    .await;
    for (area, made, looks, helper) in seen {
        let city = cities::city(area).unwrap();
        let looks = looks.unwrap();
        assert_eq!(made.len(), 2, "{}: two at the crossing", city.name);
        for (name, faction, display, victim) in made {
            assert_eq!(name, city.elite.name);
            assert_eq!(
                (faction, display),
                (looks.faction, looks.display_ids[0]),
                "{}: its guard's side and look",
                city.name
            );
            assert_eq!(victim, helper, "{}: against the attacker", city.name);
        }
    }
    let mut cfg = crate::config::GuardsConfig::default();
    cfg.rexxar_cities.clear();
    assert!(cities::defended(&cfg, cities::ORGRIMMAR).is_none());
    assert!(cities::defended(&cfg, cities::STORMWIND).is_some());
    cfg.jaina_cities = vec![cities::ORGRIMMAR];
    assert!(
        cities::defended(&cfg, cities::ORGRIMMAR).is_none(),
        "Orgrimmar is Rexxar's, not hers"
    );
}

/// Ironforge's alarm at 60 brings Jaina, one Jaina: four Ironforge Elite
/// Guards come with her, and her Teleport sends its victim to the Mystic
/// Ward. Called by Stormwind while she is out, she stays; home again,
/// Stormwind's next call brings her there.
#[tokio::test]
async fn jaina_answers_ironforge_with_its_elite_guards_and_her_teleport_to_the_mystic_ward() {
    let realm = realm();
    let (mut a, ga, map) = alice(&realm).await;
    let civ = unit(CIVILIAN);
    a.sees(civ).await;
    on(&map, move |m| {
        in_town(m, civ, cities::IRONFORGE);
        // Two of Ironforge's guards past 40: 60.
        set_alarm(m, cities::IRONFORGE, 58);
        ai::attacked(m, civ, ga);
    })
    .await;
    until(&map, "she comes to Ironforge", |m| {
        matches!(place_of(m, Champion::Jaina), Place::Away { .. })
    })
    .await;
    let (copy, city, company) = on(&map, |m| {
        let Place::Away { copy, city, .. } = place_of(m, Champion::Jaina) else {
            unreachable!()
        };
        let company: Vec<u32> = m
            .guards
            .jaina
            .company
            .iter()
            .map(|e| m.creature(*e).unwrap().entry)
            .collect();
        (copy, city, company)
    })
    .await;
    assert_eq!(city, cities::IRONFORGE);
    assert_eq!(company, [elite_of(cities::IRONFORGE); 4], "her four");
    a.wait_for("her words to Ironforge", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_MESSAGECHAT(c)
            if c.message.contains("Ironforge") || c.message.starts_with("Enough of this"))
        .then_some(())
    })
    .await;
    // Her Teleport: to the Mystic Ward.
    let (at, stand_in) = on(&map, move |m| {
        let tower = m.realm.data.spells.target_positions[&jaina::TELEPORT];
        let stand_in = jaina::stand_in(m, Some(copy), [tower.1, tower.2, tower.3]);
        crate::game::map::spells::cast_triggered(m, copy, jaina::TELEPORT, Some(ga));
        (m.players[&ga].position(), stand_in)
    })
    .await;
    let ward = cities::MYSTIC_WARD.0;
    assert!(dist(at, ward) < 0.1, "{at:?}");
    assert_eq!(stand_in, [ward.x, ward.y, ward.z]);
    // Stormwind calls: she is out, and stays.
    a.heartbeat(START.0, START.1, START.2).await;
    let (still, n) = on(&map, move |m| {
        in_town(m, civ, cities::STORMWIND);
        set_alarm(m, cities::STORMWIND, 70);
        let helper = spawn(m, ga, HELPER, 20, 6.0);
        ai::taunted(m, civ, helper);
        crate::game::map::death::kill(m, helper);
        (place_of(m, Champion::Jaina), visits(m, Champion::Jaina))
    })
    .await;
    assert!(
        matches!(
            still,
            Place::Away {
                city: cities::IRONFORGE,
                ..
            }
        ),
        "one Jaina: {still:?}"
    );
    assert_eq!(n, 1);
    // At peace a minute, she goes home.
    on(&map, move |m| {
        ai::enter_evade(m, copy);
        for e in [elite_of(cities::IRONFORGE), 5595, 68, 1756] {
            for g in called(m, e) {
                ai::enter_evade(m, g);
            }
        }
    })
    .await;
    until(&map, "her fight over", move |m| {
        m.creature(copy)
            .is_some_and(|c| !c.evading && !c.combat.attacking)
    })
    .await;
    on(&map, move |m| {
        m.guards.jaina.calm_since = m.now.checked_sub(Duration::from_secs(61));
    })
    .await;
    let kalimdor = realm.maps.get(&realm, MapKey::world(1));
    until(&kalimdor, "home in Theramore", |m| {
        place_of(m, Champion::Jaina) == Place::Home
    })
    .await;
    // Stormwind's next call brings her there.
    on(&map, move |m| {
        let helper = spawn(m, ga, HELPER, 20, 6.0);
        ai::taunted(m, civ, helper);
    })
    .await;
    until(&map, "called to Stormwind", |m| {
        matches!(
            place_of(m, Champion::Jaina),
            Place::Away {
                city: cities::STORMWIND,
                ..
            }
        ) && visits(m, Champion::Jaina) == 2
    })
    .await;
}

/// rev5-social 11: a champion whose farewell was lost with their city's
/// map (started again as they said it, and their time home with it) is
/// brought home by home, 30 s past when they were due, where nothing
/// brought them and no call wanted them again until a restart. A farewell
/// under way is left to the city.
#[tokio::test]
async fn a_champion_whose_farewell_was_lost_with_the_citys_map_comes_home() {
    let realm = realm();
    let kalimdor = realm.maps.get(&realm, MapKey::world(1));
    let spawn = guid_of_spawn(&kalimdor, champions::JAINA_SPAWN).await;
    let set = |due: Duration| {
        move |m: &mut Map| {
            let mut s = Champion::Jaina.state(&m.realm).lock();
            s.place = Place::Returning;
            s.home_by = std::time::Instant::now().checked_sub(due);
        }
    };
    // Due a second ago: the city's to see home yet; home keeps her away.
    on(&kalimdor, set(Duration::from_secs(1))).await;
    until(&kalimdor, "her spawn kept away", move |m| {
        m.creature(spawn).is_none_or(|c| !c.alive())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(
        on(&kalimdor, |m| place_of(m, Champion::Jaina)).await,
        Place::Returning
    );
    // Due a minute ago, lost: home brings her back.
    on(&kalimdor, set(Duration::from_secs(60))).await;
    until(&kalimdor, "home in Theramore", move |m| {
        place_of(m, Champion::Jaina) == Place::Home && m.creature(spawn).is_some_and(|c| c.alive())
    })
    .await;
}

/// Darnassus is on Theramore's map: its alarm at 60 brings Jaina from home
/// to it on the same map, with four Darnassus Elite Sentinels, and her
/// Teleport sends its victim into the Temple of the Moon.
#[tokio::test]
async fn jaina_answers_darnassus_on_her_own_map_and_teleports_to_the_temple_of_the_moon() {
    let realm = realm();
    let (_b, gb, kalimdor) = thrall(&realm).await;
    let civ = unit(KALIMDOR_CIVILIAN);
    on(&kalimdor, move |m| {
        in_town(m, civ, cities::DARNASSUS);
        set_alarm(m, cities::DARNASSUS, 58);
        ai::attacked(m, civ, gb);
    })
    .await;
    until(&kalimdor, "she comes to Darnassus", |m| {
        matches!(
            place_of(m, Champion::Jaina),
            Place::Away {
                city: cities::DARNASSUS,
                ..
            }
        )
    })
    .await;
    let (copy, company, home) = on(&kalimdor, |m| {
        let copy = champions::copy(m, Champion::Jaina).unwrap();
        let company: Vec<u32> = m
            .guards
            .jaina
            .company
            .iter()
            .map(|e| m.creature(*e).unwrap().entry)
            .collect();
        let home = crate::game::map::scripts::creature_of_spawn(m, jaina::SPAWN)
            .and_then(|g| m.creature(g))
            .map(|c| c.alive());
        (copy, company, home)
    })
    .await;
    assert_eq!(company, [elite_of(cities::DARNASSUS); 4]);
    assert_eq!(home, Some(false), "gone from Theramore");
    let at = on(&kalimdor, move |m| {
        crate::game::map::spells::cast_triggered(m, copy, jaina::TELEPORT, Some(gb));
        m.players[&gb].position()
    })
    .await;
    assert!(dist(at, cities::TEMPLE_OF_THE_MOON.0) < 0.1, "{at:?}");
}

/// Rexxar, the Horde's champion: the Undercity's alarm at 60 calls him; in
/// Desolace he says so and runs off 40 yards at his raised speed, Misha
/// after him, and is gone; he comes into the Undercity out of sight (past
/// 100 yards) and runs in, Misha and four Undercity Elite Deathguards with
/// him, says his words at the fight, his run his own again, and fights the
/// civilian's attacker. One Rexxar: Orgrimmar's call while he is out goes
/// unanswered. At peace a minute, he says he must go, runs off 40 yards
/// quickly, and is gone with his company, home in Desolace again.
#[tokio::test]
async fn rexxar_runs_in_fights_says_he_must_go_runs_off_and_is_gone() {
    let realm = realm();
    let (mut a, ga, map) = alice(&realm).await;
    let (mut b, _gb, kalimdor) = thrall(&realm).await;
    let civ = unit(CIVILIAN);
    a.sees(civ).await;
    let rexxar_home = guid_of_spawn(&kalimdor, champions::REXXAR_SPAWN).await;
    b.sees(rexxar_home).await;
    let mult = 5.0;
    let own = 7.0 * 1.14286;
    // Two Royal Dreadguards from 57: 59; the call that takes it past 60 is
    // against a creature of Alice's side.
    let helper = on(&map, move |m| {
        in_town(m, civ, cities::UNDERCITY);
        set_alarm(m, cities::UNDERCITY, 57);
        ai::attacked(m, civ, ga);
        let helper = spawn(m, ga, HELPER, 20, 6.0);
        hardy_creature(m, helper);
        ai::taunted(m, civ, helper);
        helper
    })
    .await;
    // Desolace sees him off, running.
    b.wait_for("his words to Desolace", |m| {
        heard(
            m,
            &[
                "Undercity is under attack. Misha, with me!",
                "The Horde calls. Come, Misha.",
            ],
        )
        .then_some(())
    })
    .await;
    let running = on(&kalimdor, move |m| {
        m.creature(rexxar_home).map(|c| {
            let to = match &c.motion {
                Motion::Moving(w) => Some(dist(w.destination(), c.position)),
                Motion::Idle { .. } => None,
            };
            (c.speeds.run, to)
        })
    })
    .await;
    let (speed, to) = running.expect("at home still");
    assert!((speed - own * mult).abs() < 0.1, "{speed}");
    assert!(to.is_some_and(|d| (30.0..=41.0).contains(&d)), "{to:?}");
    b.forgets(rexxar_home).await;
    // The Undercity: he comes out of sight and runs in.
    until(&map, "he comes", |m| {
        matches!(place_of(m, Champion::Rexxar), Place::Away { .. })
    })
    .await;
    let (copy, from, to, speed, company) = on(&map, |m| {
        let copy = champions::copy(m, Champion::Rexxar).unwrap();
        let coming = m.guards.rexxar.coming.clone();
        let company: Vec<u32> = m
            .guards
            .rexxar
            .company
            .iter()
            .map(|e| m.creature(*e).unwrap().entry)
            .collect();
        (
            copy,
            coming.as_ref().map(|c| c.from),
            coming.as_ref().map(|c| c.to),
            m.creature(copy).unwrap().speeds.run,
            company,
        )
    })
    .await;
    let (from, to) = (from.expect("on his way"), to.unwrap());
    assert!(dist(from, to) >= 100.0, "out of sight: {}", dist(from, to));
    assert!(
        (speed - own * mult).abs() < 0.1,
        "running in quick: {speed}"
    );
    let mut expected = vec![elite_of(cities::UNDERCITY); 4];
    expected.push(champions::MISHA);
    assert_eq!(company, expected, "four Elite Deathguards and Misha");
    // One Rexxar: Orgrimmar calls while he is out.
    let (still, n) = on(&map, move |m| {
        in_town(m, civ, cities::ORGRIMMAR);
        set_alarm(m, cities::ORGRIMMAR, 70);
        ai::taunted(m, civ, ga);
        in_town(m, civ, cities::UNDERCITY);
        (place_of(m, Champion::Rexxar), visits(m, Champion::Rexxar))
    })
    .await;
    assert!(
        matches!(
            still,
            Place::Away {
                city: cities::UNDERCITY,
                ..
            }
        ),
        "{still:?}"
    );
    assert_eq!(n, 1);
    a.wait_for("his words at the fight", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_MESSAGECHAT(c)
            if c.message.contains("Undercity") || c.message.starts_with("You picked"))
        .then_some(())
    })
    .await;
    until(&map, "he fights the attacker at his own run", move |m| {
        m.creature(copy).is_some_and(|c| {
            c.combat.attacking && c.combat.victim == helper && (c.speeds.run - own).abs() < 0.1
        }) && m.guards.rexxar.coming.is_none()
    })
    .await;
    // The fight over, at peace a minute: "I must go", a quick run, gone.
    on(&map, move |m| {
        crate::game::map::death::kill(m, helper);
        ai::enter_evade(m, copy);
        for e in [elite_of(cities::UNDERCITY), champions::MISHA, 13839, 5624] {
            for g in called(m, e) {
                ai::enter_evade(m, g);
            }
        }
    })
    .await;
    until(&map, "his fight over", move |m| {
        m.creature(copy)
            .is_some_and(|c| !c.evading && !c.combat.attacking)
    })
    .await;
    a.clear();
    on(&map, move |m| {
        m.guards.rexxar.calm_since = m.now.checked_sub(Duration::from_secs(61));
    })
    .await;
    a.wait_for("I must go", |m| {
        heard(m, &["I must go.", "I must go. Come, Misha."]).then_some(())
    })
    .await;
    let off = on(&map, move |m| {
        m.creature(copy).map(|c| {
            let to = match &c.motion {
                Motion::Moving(w) => Some(dist(w.destination(), c.position)),
                Motion::Idle { .. } => None,
            };
            (c.speeds.run, to, place_of(m, Champion::Rexxar))
        })
    })
    .await;
    let (speed, to, place) = off.expect("still here, running");
    assert!((speed - own * mult).abs() < 0.1, "quick: {speed}");
    assert!(to.is_some_and(|d| (30.0..=41.0).contains(&d)), "{to:?}");
    assert_eq!(place, Place::Returning);
    a.forgets(copy).await;
    until(&map, "gone with his company", |m| {
        called(m, elite_of(cities::UNDERCITY)).is_empty()
            && called(m, champions::MISHA).is_empty()
            && m.guards.rexxar.company.is_empty()
    })
    .await;
    until(&kalimdor, "home in Desolace, Misha too", |m| {
        let alive = |s: u32| {
            crate::game::map::scripts::creature_of_spawn(m, s)
                .and_then(|g| m.creature(g))
                .is_some_and(|c| c.alive())
        };
        place_of(m, Champion::Rexxar) == Place::Home
            && alive(champions::REXXAR_SPAWN)
            && alive(champions::MISHA_SPAWN)
    })
    .await;
    let back = on(&kalimdor, |m| {
        let g = crate::game::map::scripts::creature_of_spawn(m, champions::REXXAR_SPAWN).unwrap();
        m.creature(g).unwrap().speeds.run
    })
    .await;
    assert!((back - own).abs() < 0.1, "his own run at home: {back}");
}

async fn guid_of_spawn(map: &crate::game::map::MapHandle, spawn: u32) -> Guid {
    on(map, move |m| {
        crate::game::map::scripts::creature_of_spawn(m, spawn).expect("the spawn")
    })
    .await
}

fn hardy_creature(m: &mut Map, g: Guid) {
    let f = &mut m.creature_mut(g).unwrap().fields;
    f.set_u32(crate::game::fields::UNIT_FIELD_MAXHEALTH, 100_000_000);
    f.set_u32(crate::game::fields::UNIT_FIELD_HEALTH, 100_000_000);
}

/// The Horde's Elite Guards fight as Stormwind's: the warmaster's six
/// events, their Charge at whom the Elite Guard's pick goes, and their
/// Whirlwind cast in a fight; each its own guard's look, and its weapons.
#[tokio::test]
async fn the_horde_elite_guards_fight_as_stormwinds() {
    let realm = realm();
    let (mut a, ga, map) = alice(&realm).await;
    for city in CITIES.iter().filter(|c| c.champion == Champion::Rexxar) {
        let entry = city.elite.entry;
        let (e, events, weapons, i) = on(&map, move |m| {
            let at = m.players[&ga].position();
            let e = crate::game::map::scripts::summon(
                m,
                ga,
                entry,
                Vector3d {
                    x: at.x + 3.0,
                    ..at
                },
                std::f32::consts::PI,
                None,
            )
            .unwrap();
            ai::attacked(m, e, ga);
            let data = &m.realm.data.scripts;
            let events = &data.ai_events[&entry];
            let whirlwind = events
                .iter()
                .position(|ev| {
                    data.ai_scripts
                        .get(&ev.actions[0])
                        .and_then(|s| s.first())
                        .is_some_and(|s| s.command == 15 && s.datalong[0] == WHIRLWIND)
                })
                .expect("its Whirlwind");
            let f = &m.creature(e).unwrap().fields;
            let weapons: Vec<u32> = (0..3)
                .map(|i| f.get_u32(crate::game::fields::UNIT_VIRTUAL_ITEM_SLOT_DISPLAY + i))
                .collect();
            (e, events.len(), weapons, whirlwind)
        })
        .await;
        assert_eq!(events, 6, "{}: the warmaster's fight", city.name);
        let displays: Vec<u32> = city
            .elite
            .weapons
            .iter()
            .map(|w| realm.data.item(*w).map_or(0, |i| i.display_id))
            .collect();
        assert_eq!(weapons, displays, "{}: its weapons", city.name);
        a.sees(e).await;
        on(&map, move |m| {
            if let Some(s) = event_ai::scripted_mut(m, e) {
                s.next_tick = None;
            }
            fire(m, e, i);
        })
        .await;
        a.wait_for("its Whirlwind", move |m| {
            spell_go(m, WHIRLWIND, e).then_some(())
        })
        .await;
        on(&map, move |m| {
            crate::game::map::scripts::remove_for_good(m, e)
        })
        .await;
    }
}

// The real world, in the extended suite.

/// Where each capital's raid is (peon's hotspots): its square.
const SQUARES: &[(u32, (f32, f32, f32))] = &[
    (cities::STORMWIND, (-8833.38, 628.628, 94.0066)),
    (cities::IRONFORGE, (-4918.88, -940.406, 501.564)),
    (cities::DARNASSUS, (9949.56, 2284.21, 1341.4)),
    (cities::ORGRIMMAR, (1629.36, -4373.39, 31.2564)),
    (cities::UNDERCITY, (1584.07, 241.987, -52.1534)),
    (cities::THUNDER_BLUFF, (-1277.37, 124.804, 131.287)),
];

/// On the real navmesh: each capital's square is on it and in the city
/// (Ironforge too, inside its mountain, where the bake reads Dun Morogh);
/// Jaina's teleport places are on it, in their cities; and Rexxar finds a
/// start out of sight of each of his cities' squares, at about its height,
/// that a walk from it reaches.
#[tokio::test]
#[ignore = "extended"]
async fn each_citys_spots_are_on_the_navmesh() {
    let bake = BAKE.as_path();
    if !bake.join("Azeroth.map").exists() {
        return;
    }
    let mut cfg = harness::test_config();
    cfg.nav_dir = Some(BAKE.clone());
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = crate::game::Realm::new(cfg, store, startup, world(), None);
    for &(area, (x, y, z)) in SQUARES {
        let city = cities::city(area).unwrap();
        let map = realm.maps.get(&realm, MapKey::world(city.map));
        let out = on(&map, move |m| {
            let city = cities::city(area).unwrap();
            let ground = m.nav.as_mut().and_then(|n| n.ground(x, y, z));
            let square = Vector3d {
                x,
                y,
                z: ground.unwrap_or(z),
            };
            let town = crate::game::map::world::zone_and_area(m, square)
                .and_then(|(zone, a)| crate::game::map::guards::town(a, zone));
            let prison = jaina::prison(m, area).map(|(_, p, _)| {
                let ground = m.nav.as_mut().and_then(|n| n.ground(p.x, p.y, p.z));
                let place = crate::game::map::world::zone_and_area(m, p)
                    .and_then(|(zone, a)| crate::game::map::guards::town(a, zone));
                (ground.map(|g| (g - p.z).abs()), place)
            });
            let start = (city.champion == Champion::Rexxar).then(|| {
                // As he comes: to a point near the fight a walk reaches.
                let fight =
                    crate::game::map::guards::reachable_point(m, square, 8.0).unwrap_or(square);
                let p = champions::out_of_sight(m, fight, 110.0);
                let path = m.nav.as_mut().and_then(|n| n.path(p, fight));
                let reached = path
                    .and_then(|p| p.last().copied())
                    .is_some_and(|e| crate::game::map::nav::reaches(e, fight, 3.0));
                (dist(p, fight), (p.z - fight.z).abs(), reached)
            });
            (ground, town, prison, start)
        })
        .await;
        let (ground, town, prison, start) = out;
        assert!(
            ground.is_some_and(|g| (g - z).abs() < 2.0),
            "{}: its square on the navmesh: {ground:?}",
            city.name
        );
        assert_eq!(town, Some(area), "{}: its square is the city's", city.name);
        if let Some((off, place)) = prison {
            assert!(
                off.is_some_and(|d| d < 1.0),
                "{}: her teleport's place on the navmesh: {off:?}",
                city.name
            );
            assert_eq!(place, Some(area), "{}: and in the city", city.name);
        }
        if let Some((far, rise, reached)) = start {
            assert!(far >= 100.0, "{}: out of sight: {far}", city.name);
            assert!(rise <= 20.0, "{}: about as high: {rise}", city.name);
            assert!(reached, "{}: a walk reaches the square", city.name);
        }
    }
}

/// The real data: each capital's Elite Guard is made of its guard as the
/// database has it (its look and side), with weapons the data has; and
/// Rexxar and Misha are at home in Desolace, his spells his template's.
#[test]
#[ignore = "extended"]
fn the_real_data_has_every_capitals_elite_guard_and_rexxar() {
    let Some(d) = crate::game::data::load::real_data() else {
        return;
    };
    for city in CITIES {
        let t = d
            .creature_template(city.elite.entry)
            .unwrap_or_else(|| panic!("{}'s Elite Guard", city.name));
        let looks = d.creature_template(city.elite.looks).unwrap();
        assert_eq!(t.name, city.elite.name);
        assert_eq!(
            (t.display_ids, t.faction, t.level_max, t.rank),
            (looks.display_ids, looks.faction, 62, 1),
            "{}",
            city.name
        );
        assert_eq!(d.scripts.ai_events[&city.elite.entry].len(), 6);
        for w in city.elite.weapons.iter().filter(|w| **w != 0) {
            let i = d.item(*w).unwrap_or_else(|| panic!("{}: {w}", city.name));
            assert_eq!(i.class, 2, "{}: {w} a weapon", city.name);
        }
    }
    let rexxar = d.creature_template(champions::REXXAR).expect("Rexxar");
    assert_eq!(rexxar.name, "Rexxar");
    let spells = &d.spells.creature_spells[&rexxar.spell_list_id];
    let ids: Vec<u32> = spells.iter().map(|s| s.spell).collect();
    assert_eq!(
        ids,
        [3391, 18813, 17963, 15976],
        "Thrash, Knock Away, Sundering Cleave, Puncture"
    );
    let spawns = &d.creature_spawns[&1];
    for (spawn, entry) in [
        (champions::REXXAR_SPAWN, champions::REXXAR),
        (champions::MISHA_SPAWN, champions::MISHA),
    ] {
        let s = spawns.iter().find(|s| s.guid == spawn).expect("the spawn");
        assert_eq!(s.entries, [entry]);
    }
}

/// On the real navmesh and terrain, which read Dun Morogh inside the
/// mountain and the Gates of Ironforge (809) down the gate's hall: a
/// player anywhere in Ironforge, from the hall in to its wards, is in the
/// city, zone and area; out on the gate's steps (`game_tele`'s
/// `GatesOfIronforge`) and in Kharanos, still in Dun Morogh.
#[tokio::test]
#[ignore = "extended"]
async fn on_the_real_navmesh_a_player_inside_ironforge_is_in_ironforge() {
    if !BAKE.as_path().join("Azeroth.map").exists() {
        return;
    }
    let mut cfg = harness::test_config();
    cfg.nav_dir = Some(BAKE.clone());
    cfg.terrain_dir = Some(harness::real_data("terrain"));
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = crate::game::Realm::new(cfg, store, startup, cities_world(), None);
    let (_a, ga, map) = alice(&realm).await;
    let ironforge = cities::IRONFORGE;
    let inside = [
        ("the Commons", THE_COMMONS),
        ("the Great Forge", (-4792.95, -1119.12, 498.806)),
        ("the Military Ward", (-4968.86, -1225.49, 501.669)),
        ("Tinker Town", (-4831.62, -1279.2, 501.869)),
        ("the Mystic Ward", (-4662.07, -952.721, 500.378)),
        ("the gate's hall", (-4990.0, -858.0, 500.7)),
        ("just in the gate", (-5016.0, -846.0, 497.0)),
    ];
    let outside = [
        ("the gate's steps", (-5040.5, -814.817, 495.128)),
        ("Kharanos", (-5597.31, -483.398, 396.981)),
    ];
    let places = inside
        .map(|(name, at)| (name, at, Some(ironforge)))
        .into_iter()
        .chain(outside.map(|(name, at)| (name, at, None)));
    for (name, (x, y, z), want) in places {
        let (zone, area) = on(&map, move |m| {
            let z = m.nav.as_mut().and_then(|n| n.ground(x, y, z)).unwrap_or(z);
            let p = m.players.get_mut(&ga).unwrap();
            p.movement.position = Vector3d { x, y, z };
            crate::game::map::world::update_zone(m, ga);
            (m.players[&ga].zone, m.players[&ga].area)
        })
        .await;
        match want {
            Some(city) => assert_eq!((zone, area), (city, city), "{name}"),
            None => assert_eq!(zone, 1, "{name}: Dun Morogh ({area})"),
        }
    }
}
