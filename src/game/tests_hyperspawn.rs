//! Hyperspawn (`[hyperspawn]`, `map/hyperspawn.rs`): in a starting area,
//! the faster a pocket's creatures die, the faster they come back, and its
//! quest objects by the rate they are looted. Each test runs its kills in
//! one query, moving the map's clock itself (`m.now`), so none waits.
//! The default areas are checked against the real world's navmesh at the
//! end (extended).

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::creature::Life;
use crate::game::map::{Map, MapKey, ai, hyperspawn, professions};

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const KOBOLD: u32 = 2100;
const ELITE: u32 = 2101;
const RARE: u32 = 2102;
const NAMED: u32 = 2103;
const GIVER: u32 = 2104;
const CHEST: u32 = 2110;
const QUEST_ITEM: u32 = 2120;
const NORTHSHIRE: u32 = 9;
const ELWYNN: u32 = 12;
/// The kobolds' spawns, 1 to 8; the exempt ones' after.
const KOBOLDS: std::ops::RangeInclusive<u32> = 1..=8;
const ELITES: [u32; 2] = [20, 21];
const RARES: [u32; 2] = [22, 23];
const THE_NAMED: u32 = 24;
const GIVERS: [u32; 2] = [25, 26];
const CHESTS: std::ops::RangeInclusive<u32> = 1..=4;
const FIVE_MINUTES: Duration = Duration::from_secs(300);
const THREE_MINUTES: Duration = Duration::from_secs(180);

/// One pocket east of the start (the 50-yard square from x -8950 and y
/// -150): eight kobolds, two elites, two rares, a named one and two quest
/// givers, all at five minutes; four quest chests at three.
fn data() -> GameData {
    let mut d = GameData::empty();
    let template = |entry, name: &str, rank, npc_flags| CreatureTemplate {
        entry,
        name: name.into(),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: 2,
        level_max: 2,
        faction: 32,
        unit_class: 1,
        health_multiplier: 1.0,
        damage_multiplier: 1.0,
        base_attack_time: 2000,
        rank,
        npc_flags,
        ..Default::default()
    };
    for (entry, name, rank, flags) in [
        (KOBOLD, "Kobold", 0, 0),
        (ELITE, "Elite", 1, 0),
        (RARE, "Rare", 4, 0),
        (NAMED, "Garrick", 0, 0),
        (GIVER, "Giver", 0, 2),
    ] {
        d.creature_templates
            .insert(entry, template(entry, name, rank, flags));
    }
    let spawn = |guid: u32, entry| CreatureSpawn {
        guid,
        entries: vec![entry],
        map: 0,
        x: START.0 + 5.0 + guid as f32,
        y: START.1 + 5.0,
        z: START.2,
        respawn_min: 300,
        respawn_max: 300,
        ..Default::default()
    };
    let mut spawns: Vec<CreatureSpawn> = KOBOLDS.map(|g| spawn(g, KOBOLD)).collect();
    spawns.extend(ELITES.map(|g| spawn(g, ELITE)));
    spawns.extend(RARES.map(|g| spawn(g, RARE)));
    spawns.push(spawn(THE_NAMED, NAMED));
    spawns.extend(GIVERS.map(|g| spawn(g, GIVER)));
    d.creature_spawns.insert(0, spawns);
    let mut data = [0u32; 24];
    data[1] = CHEST;
    d.gameobject_templates.insert(
        CHEST,
        GameObjectTemplate {
            entry: CHEST,
            kind: 3,
            display_id: 1,
            name: "Milly's Harvest".into(),
            size: 1.0,
            data,
            ..Default::default()
        },
    );
    d.loot.gameobject.insert(
        CHEST,
        vec![LootEntry {
            item: QUEST_ITEM,
            chance: -100.0,
            min: 1,
            max: 1,
            ..Default::default()
        }],
    );
    d.gameobject_spawns.insert(
        0,
        CHESTS
            .map(|g| GameObjectSpawn {
                guid: g,
                entry: CHEST,
                map: 0,
                x: START.0 + 10.0 + 3.0 * g as f32,
                y: START.1 + 12.0,
                z: START.2,
                rotation: [0.0, 0.0, 0.0, 1.0],
                respawn_min: 180,
                respawn_max: 180,
                state: 1,
                ..Default::default()
            })
            .collect(),
    );
    d.areas.insert(
        NORTHSHIRE,
        AreaInfo {
            id: NORTHSHIRE,
            map: 0,
            zone: ELWYNN,
            name: "Northshire Valley".into(),
            ..Default::default()
        },
    );
    d
}

fn realm() -> Arc<Realm> {
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(harness::test_config(), store, startup, data(), None)
}

/// A realm, a player at the start (the killer), and its map.
async fn world() -> (Arc<Realm>, Client, Guid, crate::game::map::MapHandle) {
    let realm = realm();
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.sees(guid::unit(KOBOLD, 1)).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    (realm, a, g, map)
}

/// The creature of spawn `s`, as it is now.
fn of_spawn(m: &Map, s: u32) -> Guid {
    m.creatures
        .iter()
        .find(|(_, c)| c.spawn_guid == Some(s))
        .map(|(_, c)| c.guid)
        .expect("the spawn's creature")
}

/// Every creature and object of the map in `area` (the tests have no
/// navmesh: what hyperspawn falls back on).
fn place_in(m: &mut Map, area: u32) {
    for (_, c) in m.creatures.iter_mut() {
        c.zone_area = Some((ELWYNN, area));
    }
    for (_, o) in m.gameobjects.iter_mut() {
        o.zone_area = Some((ELWYNN, area));
    }
}

/// Spawn `s` killed by `by`: how long until it comes back.
fn kill(m: &mut Map, s: u32, by: Guid) -> Duration {
    let g = of_spawn(m, s);
    ai::die(m, g, by);
    match m.creature(g).map(|c| c.life) {
        Some(Life::Dead { respawn_at, .. }) => respawn_at - m.now,
        _ => panic!("dead"),
    }
}

/// Every dead creature back now, in `area`.
fn bring_back(m: &mut Map, area: u32) {
    let now = m.now;
    for (_, c) in m.creatures.iter_mut() {
        if let Life::Dead { respawn_at, .. } = &mut c.life {
            *respawn_at = now;
        }
    }
    ai::update_dead(m);
    place_in(m, area);
}

/// The kobolds killed `rounds` times over, each brought back at once:
/// their respawns, in order.
fn rounds(m: &mut Map, by: Guid, area: u32, rounds: usize) -> Vec<Duration> {
    let mut out = Vec::new();
    for _ in 0..rounds {
        for s in KOBOLDS {
            out.push(kill(m, s, by));
        }
        bring_back(m, area);
    }
    out
}

#[tokio::test]
async fn a_quiet_pocket_respawns_at_its_normal_time() {
    let (_realm, _a, g, map) = world().await;
    let times = map
        .query(move |m| {
            place_in(m, NORTHSHIRE);
            // A kill a minute: 0.5 a minute in the window's score, a
            // sixteenth a spawn, under the quiet rate.
            let mut times = Vec::new();
            for s in [1, 2, 3] {
                times.push(kill(m, s, g));
                m.now += Duration::from_secs(60);
            }
            times
        })
        .await
        .unwrap();
    assert_eq!(times, vec![FIVE_MINUTES; 3]);
}

#[tokio::test]
async fn a_hot_pocket_respawns_at_its_floor() {
    let (_realm, _a, g, map) = world().await;
    let times = map
        .query(move |m| {
            place_in(m, NORTHSHIRE);
            rounds(m, g, NORTHSHIRE, 10)
        })
        .await
        .unwrap();
    assert_eq!(times[0], FIVE_MINUTES, "the first kill is quiet");
    // Faster with each kill; then at the floor.
    assert!(times.windows(2).all(|w| w[1] <= w[0]), "{times:?}");
    assert!(times[20] < Duration::from_secs(30), "{times:?}");
    assert_eq!(*times.last().unwrap(), Duration::from_secs(2));
}

#[tokio::test]
async fn a_creature_outside_the_areas_is_unchanged() {
    let (_realm, _a, g, map) = world().await;
    let times = map
        .query(move |m| {
            place_in(m, ELWYNN);
            rounds(m, g, ELWYNN, 10)
        })
        .await
        .unwrap();
    assert!(times.iter().all(|t| *t == FIVE_MINUTES), "{times:?}");
}

#[tokio::test]
async fn elites_rares_named_and_quest_givers_are_exempt() {
    let (_realm, _a, g, map) = world().await;
    let (times, before, after) = map
        .query(move |m| {
            place_in(m, NORTHSHIRE);
            rounds(m, g, NORTHSHIRE, 10);
            let at = m.creature(of_spawn(m, 1)).unwrap().home;
            let before = hyperspawn::reading(m, at).1;
            let mut times = Vec::new();
            for s in ELITES
                .into_iter()
                .chain(RARES)
                .chain([THE_NAMED])
                .chain(GIVERS)
            {
                times.push(kill(m, s, g));
            }
            (times, before, hyperspawn::reading(m, at).1)
        })
        .await
        .unwrap();
    assert_eq!(before.factor, 150.0, "the pocket is at its most");
    assert_eq!(before.spawns, 8, "only the kobolds count");
    assert!(times.iter().all(|t| *t == FIVE_MINUTES), "{times:?}");
    assert_eq!(before, after, "their kills don't count");
}

#[tokio::test]
async fn decay_brings_the_factor_back_down() {
    let (_realm, _a, g, map) = world().await;
    let (factors, quiet) = map
        .query(move |m| {
            place_in(m, NORTHSHIRE);
            rounds(m, g, NORTHSHIRE, 10);
            let at = m.creature(of_spawn(m, 1)).unwrap().home;
            let mut factors = Vec::new();
            for _ in 0..6 {
                factors.push(hyperspawn::reading(m, at).1.factor);
                m.now += Duration::from_secs(120);
            }
            m.now += Duration::from_secs(1200);
            (factors, kill(m, 1, g))
        })
        .await
        .unwrap();
    assert_eq!(factors[0], 150.0);
    // 80 kills: after two windows, 80/e² ≈ 10.8 in the score, 5.4 a
    // minute, 0.68 a spawn: 1 + 0.48 × 40.
    assert!((factors[2] - 20.1).abs() < 0.2, "{factors:?}");
    assert!(factors[..4].windows(2).all(|w| w[1] < w[0]), "{factors:?}");
    assert!(factors.windows(2).all(|w| w[1] <= w[0]), "{factors:?}");
    assert_eq!(factors[5], 1.0, "{factors:?}");
    assert_eq!(quiet, FIVE_MINUTES, "quiet again");
}

/// A hastened kobold whose corpse has loot on it waits for its looter;
/// looted clean, it comes back.
#[tokio::test]
async fn a_hastened_corpse_with_loot_holds_its_respawn() {
    let (_realm, _a, g, map) = world().await;
    let (waited, back) = map
        .query(move |m| {
            place_in(m, NORTHSHIRE);
            rounds(m, g, NORTHSHIRE, 10);
            assert_eq!(kill(m, 1, g), Duration::from_secs(2));
            let kobold = of_spawn(m, 1);
            m.creature_mut(kobold).unwrap().fields.set_flag(
                crate::game::fields::UNIT_DYNAMIC_FLAGS,
                crate::game::map::loot::DYNFLAG_LOOTABLE,
                true,
            );
            m.now += Duration::from_secs(3);
            ai::update_dead(m);
            let waited = matches!(m.creature(kobold).map(|c| c.life), Some(Life::Dead { .. }));
            m.creature_mut(kobold).unwrap().fields.set_flag(
                crate::game::fields::UNIT_DYNAMIC_FLAGS,
                crate::game::map::loot::DYNFLAG_LOOTABLE,
                false,
            );
            ai::update_dead(m);
            let g = of_spawn(m, 1);
            (
                waited,
                matches!(m.creature(g).map(|c| c.life), Some(Life::Alive)),
            )
        })
        .await
        .unwrap();
    assert!(waited, "its loot held it");
    assert!(back, "looted, it came back");
}

/// The chests' respawns as each is looted, `rounds` times over, each back
/// at once.
fn loot_rounds(m: &mut Map, by: Guid, rounds: usize) -> Vec<Duration> {
    let mut out = Vec::new();
    for _ in 0..rounds {
        place_in(m, NORTHSHIRE);
        for s in CHESTS {
            professions::object_looted(m, by, guid::game_object(CHEST, s));
            let at = m
                .loot_state
                .respawns
                .iter()
                .find(|(_, x)| *x == s)
                .map(|(t, _)| *t)
                .expect("waiting to come back");
            out.push(at - m.now);
        }
        let now = m.now;
        for (t, _) in m.loot_state.respawns.iter_mut() {
            *t = now;
        }
        professions::respawn_nodes(m);
    }
    out
}

#[tokio::test]
async fn a_quest_object_looted_fast_comes_back_fast() {
    let (_realm, _a, g, map) = world().await;
    let (times, hot) = map
        .query(move |m| {
            let times = loot_rounds(m, g, 10);
            (times, hyperspawn::hot_pockets(m))
        })
        .await
        .unwrap();
    assert_eq!(times[0], THREE_MINUTES, "the first is quiet");
    assert!(times.windows(2).all(|w| w[1] <= w[0]), "{times:?}");
    assert_eq!(*times.last().unwrap(), Duration::from_secs(2));
    assert_eq!(hot, 1, "the chests' pocket is hot");
}

#[tokio::test]
async fn quest_objects_are_left_alone_with_objects_off() {
    let mut cfg = harness::test_config();
    cfg.hyperspawn.objects = false;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = Realm::new(cfg, store, startup, data(), None);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.sees(guid::game_object(CHEST, 1)).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let times = map.query(move |m| loot_rounds(m, g, 5)).await.unwrap();
    assert!(times.iter().all(|t| *t == THREE_MINUTES), "{times:?}");
}

/// `.hyperspawn` shows the pocket a GM stands in.
#[tokio::test]
async fn hyperspawn_shows_the_pocket_where_the_gm_stands() {
    let (_realm, mut a, g, map) = world().await;
    map.query(move |m| {
        place_in(m, NORTHSHIRE);
        m.players.get_mut(&g).unwrap().area = NORTHSHIRE;
        rounds(m, g, NORTHSHIRE, 2);
    })
    .await
    .unwrap();
    a.say(".hyperspawn").await;
    let text = a.system_message("Pocket").await;
    assert!(
        text.contains("Area 9 (Northshire Valley): a hyperspawn area"),
        "{text}"
    );
    assert!(text.contains("creatures 8 spawns"), "{text}");
    assert!(text.contains("quest objects 4 spawns"), "{text}");
}

/// The default areas, on the real world's navmesh: each is the area
/// `AreaTable` names, in its zone, and the starting creatures and quest
/// objects of each race stand in them.
#[test]
#[ignore = "extended"]
fn the_default_areas_are_the_starting_areas_on_the_navmesh() {
    use crate::game::map::nav::Nav;
    let Some(data) = crate::game::data::load::real_data() else {
        return;
    };
    let names: [(u32, &str, u32); 14] = [
        (9, "Northshire Valley", 12),
        (34, "Echo Ridge Mine", 12),
        (59, "Northshire Vineyards", 12),
        (132, "Coldridge Valley", 1),
        (188, "Shadowglen", 141),
        (256, "Aldrassil", 141),
        (257, "Shadowthread Cave", 141),
        (363, "Valley of Trials", 14),
        (365, "Burning Blade Coven", 14),
        (220, "Red Cloud Mesa", 215),
        (221, "Camp Narache", 215),
        (358, "Brambleblade Ravine", 215),
        (154, "Deathknell", 85),
        (155, "Night Web's Hollow", 85),
    ];
    let defaults = crate::config::HyperspawnConfig::default().areas;
    assert_eq!(
        {
            let mut a = defaults.clone();
            a.sort_unstable();
            a
        },
        {
            let mut a: Vec<u32> = names.iter().map(|n| n.0).collect();
            a.sort_unstable();
            a
        }
    );
    for (id, name, zone) in names {
        let a = data.area(id).unwrap_or_else(|| panic!("area {id}"));
        assert_eq!((a.name.as_str(), a.zone), (name, zone), "area {id}");
    }
    let bake = crate::game::harness::real_data("nav");
    // Each race's first creatures and quest objects, by map: two thirds of
    // each at least in the areas. The navmesh gives the deep ends of Echo
    // Ridge Mine and of the Burning Blade Coven their zones' ids (8 of 25
    // Kobold Laborers in Elwynn Forest's, 5 of 37 Vile Familiars in
    // Durotar's), which hyperspawn can't take without the whole zone.
    let creatures: [(u32, &[&str]); 2] = [
        (
            0,
            &[
                "Kobold Vermin",
                "Kobold Laborer",
                "Rockjaw Trogg",
                "Ragged Young Wolf",
                "Mindless Zombie",
                "Young Night Web Spider",
            ],
        ),
        (
            1,
            &[
                "Young Nightsaber",
                "Young Thistle Boar",
                "Mottled Boar",
                "Vile Familiar",
                "Plainstrider",
                "Bristleback Quilboar",
            ],
        ),
    ];
    let objects: [(u32, &[&str]); 2] = [
        (0, &["Milly's Harvest"]),
        (1, &["Cactus Apple", "Moonpetal Lily", "Hyacinth Mushroom"]),
    ];
    let mut seen: ahash::AHashSet<u32> = ahash::AHashSet::new();
    let mut wrong: Vec<String> = Vec::new();
    for (map, which) in creatures {
        let mut nav =
            Nav::open(&bake, if map == 0 { "Azeroth" } else { "Kalimdor" }).expect("the bake");
        for name in which {
            let entries: Vec<u32> = data
                .creature_templates
                .values()
                .filter(|t| t.name == *name)
                .map(|t| t.entry)
                .collect();
            let (mut inside, mut all, mut outside) = (0, 0, Vec::new());
            for s in data.creature_spawns[&map]
                .iter()
                .filter(|s| s.entries.iter().any(|e| entries.contains(e)))
            {
                let at = Vector3d {
                    x: s.x,
                    y: s.y,
                    z: s.z,
                };
                if let Some((_, a)) = nav.zone_and_area(at) {
                    all += 1;
                    if defaults.contains(&a) {
                        inside += 1;
                        seen.insert(a);
                    } else {
                        outside.push((a, at.x as i32, at.y as i32));
                    }
                }
            }
            if all == 0 || inside * 3 < all * 2 {
                wrong.push(format!(
                    "{name}: {inside} of {all} in the areas; outside: {outside:?}"
                ));
            }
        }
    }
    for (map, which) in objects {
        let mut nav =
            Nav::open(&bake, if map == 0 { "Azeroth" } else { "Kalimdor" }).expect("the bake");
        for name in which {
            let (mut inside, mut all, mut outside) = (0, 0, Vec::new());
            for s in data.gameobject_spawns[&map].iter().filter(|s| {
                data.gameobject_templates
                    .get(&s.entry)
                    .is_some_and(|t| t.name == *name)
            }) {
                let at = Vector3d {
                    x: s.x,
                    y: s.y,
                    z: s.z,
                };
                if let Some((_, a)) = nav.zone_and_area(at) {
                    all += 1;
                    if defaults.contains(&a) {
                        inside += 1;
                        seen.insert(a);
                    } else {
                        outside.push((a, at.x as i32, at.y as i32));
                    }
                }
            }
            if all == 0 || inside * 3 < all * 2 {
                wrong.push(format!(
                    "{name}: {inside} of {all} in the areas; outside: {outside:?}"
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
    // Every race's valley has its creatures in it.
    for a in [9, 34, 59, 132, 188, 363, 220, 358, 154, 155] {
        assert!(seen.contains(&a), "nothing found in area {a}");
    }
}

/// On the real world, with its navmesh: Northshire's Kobold Vermin, killed
/// as fast as they come back, come back at the floor; the area is the
/// navmesh's (nothing set by hand).
#[tokio::test]
#[ignore = "extended"]
async fn northshires_kobolds_killed_fast_come_back_fast() {
    let db = crate::game::harness::real_data("db");
    if crate::game::data::load::real_data().is_none() {
        return;
    }
    let mut cfg = harness::test_config();
    cfg.nav_dir = Some(crate::game::harness::real_data("nav"));
    cfg.terrain_dir = Some(crate::game::harness::real_data("terrain"));
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = Realm::new(cfg, store, startup, harness::load_real(&db), None);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let (first, last, factor) = map
        .query(move |m| {
            let vermin: Vec<u32> = m
                .realm
                .data
                .creature_templates
                .values()
                .filter(|t| t.name == "Kobold Vermin")
                .map(|t| t.entry)
                .collect();
            // The pocket with the most of them.
            let mut pockets: ahash::AHashMap<(i32, i32), Vec<u32>> = ahash::AHashMap::new();
            for (_, c) in m.creatures.iter() {
                if vermin.contains(&c.entry)
                    && let Some(s) = c.spawn_guid
                {
                    let key = (
                        (c.home.x / 50.0).floor() as i32,
                        (c.home.y / 50.0).floor() as i32,
                    );
                    pockets.entry(key).or_default().push(s);
                }
            }
            let spawns = pockets
                .into_values()
                .max_by_key(Vec::len)
                .expect("Kobold Vermin");
            let mut times = Vec::new();
            for _ in 0..40 {
                for s in &spawns {
                    times.push(kill(m, *s, g));
                }
                let now = m.now;
                // Looted clean as they fall.
                for (_, c) in m.creatures.iter_mut() {
                    if let Life::Dead { respawn_at, .. } = &mut c.life {
                        *respawn_at = now;
                        c.fields.set_flag(
                            crate::game::fields::UNIT_DYNAMIC_FLAGS,
                            crate::game::map::loot::DYNFLAG_LOOTABLE,
                            false,
                        );
                    }
                }
                ai::update_dead(m);
                assert!(
                    spawns.iter().all(|s| matches!(
                        m.creature(of_spawn(m, *s)).map(|c| c.life),
                        Some(Life::Alive)
                    )),
                    "all back"
                );
            }
            let at = m.creature(of_spawn(m, spawns[0])).unwrap().home;
            (
                times[0],
                *times.last().unwrap(),
                hyperspawn::reading(m, at).1.factor,
            )
        })
        .await
        .unwrap();
    assert_eq!(first, THREE_MINUTES, "Kobold Vermin's own time, quiet");
    assert_eq!(last, Duration::from_secs(2));
    assert_eq!(factor, 150.0);
}
