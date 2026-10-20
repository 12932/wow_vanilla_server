//! Creatures as Thistle Tea has them past the fight: where they may go
//! (water and air), wounded ones slowing, pausing while a player talks to
//! them, critters running; the War Effort's events, LocalDefense's
//! alerts, and a pet's failed casts told to its master.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::MapHandle;

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

fn template(entry: u32, name: &str, faction: u32) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: name.into(),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        display_scales: [1.0, 0.0, 0.0, 0.0],
        level_min: 10,
        level_max: 10,
        faction,
        unit_class: 1,
        health_max: 1000,
        ..Default::default()
    }
}

fn spawn(guid: u32, entry: u32, at: (f32, f32, f32)) -> CreatureSpawn {
    CreatureSpawn {
        guid,
        entries: vec![entry],
        map: 0,
        x: at.0,
        y: at.1,
        z: at.2,
        ..Default::default()
    }
}

async fn enter(
    realm: &std::sync::Arc<crate::game::Realm>,
    account: &str,
    name: &str,
    race: Race,
) -> (Client, Guid, MapHandle) {
    let mut c = Client::connect(realm, account);
    let g = c.enter(name, race, Class::Warrior).await;
    let map = realm.registry.online(g).unwrap().map();
    (c, g, map)
}

/// The War Effort is over in this patch: only its post-war watch runs.
#[test]
fn the_war_effort_is_over() {
    let mut d = GameData::empty();
    for id in [84, 85, 86] {
        d.events.insert(
            id,
            GameEvent {
                id,
                hardcoded: true,
                ..Default::default()
            },
        );
    }
    let now = chrono::Utc::now().timestamp();
    let on = crate::game::events::active(&d, now);
    assert!(on.contains(&86), "the post-war watch");
    assert!(!on.contains(&84) && !on.contains(&85), "no war, no gong");
}

const GUARD: u32 = 68;

/// A town guard felled by an Alliance player warns the Horde on the map
/// that its area is under attack, and not again for ten seconds.
#[tokio::test]
async fn a_fallen_guard_warns_the_other_side() {
    let mut d = GameData::empty();
    let mut guard = template(GUARD, "Stormwind City Guard", 11);
    guard.flags_extra = 0x400;
    d.creature_templates.insert(GUARD, guard);
    d.creature_spawns.insert(
        0,
        vec![
            spawn(1, GUARD, (START.0 + 3.0, START.1, START.2)),
            spawn(2, GUARD, (START.0 - 3.0, START.1, START.2)),
        ],
    );
    let realm = harness::realm_with(d);
    let (_a, me, map) = enter(&realm, "A", "Alliance", Race::Human).await;
    let (mut h, horde, _) = enter(&realm, "H", "Horde", Race::Undead).await;
    let (mut b, _, _) = enter(&realm, "B", "Ally", Race::Human).await;
    assert_eq!(
        realm.registry.online(horde).unwrap().map().key,
        map.key,
        "all on one map"
    );
    map.call(move |m| {
        if let Some(p) = m.players.get_mut(&me) {
            p.area = 12;
        }
    });
    let guards = map
        .query(|m| {
            m.creatures
                .iter()
                .filter(|(_, c)| c.entry == GUARD)
                .map(|(_, c)| c.guid)
                .collect::<Vec<_>>()
        })
        .await
        .unwrap();
    let first = guards[0];
    map.call(move |m| crate::game::map::combat::deal_damage(m, me, first, 1_000_000));
    let area = h
        .wait_frame("SMSG_ZONE_UNDER_ATTACK", 0x254, |_| true)
        .await;
    assert_eq!(area, 12u32.to_le_bytes(), "Elwynn is under attack");
    h.clear();
    let told_own = b
        .gets(Duration::from_millis(200), |m| {
            matches!(m, ServerOpcodeMessage::SMSG_ZONE_UNDER_ATTACK(_)).then_some(())
        })
        .await;
    assert!(told_own.is_none(), "the attackers' side isn't warned");
    // The second guard, at once: quiet.
    let second = guards[1];
    map.call(move |m| crate::game::map::combat::deal_damage(m, me, second, 1_000_000));
    let again = h
        .gets(Duration::from_millis(300), |m| {
            matches!(m, ServerOpcodeMessage::SMSG_ZONE_UNDER_ATTACK(_)).then_some(())
        })
        .await;
    assert!(again.is_none(), "not again within ten seconds");
}

/// A pet told to cast at a target out of its reach tells its master why.
#[tokio::test]
async fn a_pets_failed_cast_is_told_to_its_master() {
    use crate::game::tests_pets as tp;
    let realm = harness::realm_with(tp::data());
    let (mut w, gw, map) = tp::enter(&realm, "W", "Warlock", Race::Human, Class::Warlock).await;
    tp::learn(&map, gw, &[tp::SUMMON_IMP]);
    let wolf = tp::place(&map, gw, tp::WOLF, 0x70_0009, 80.0).await;
    w.sees(wolf).await;
    tp::cast(&w, tp::SUMMON_IMP, None).await;
    let imp = tp::pet_bar(&mut w).await;
    assert!(!imp.is_zero());
    w.sees(imp).await;
    w.send(CMSG_PET_CAST_SPELL {
        guid: imp,
        id: tp::FIREBOLT,
        targets: tp::at(wolf),
    })
    .await;
    let failed = w.wait_frame("SMSG_PET_CAST_FAILED", 0x138, |_| true).await;
    let mut want = tp::FIREBOLT.to_le_bytes().to_vec();
    want.extend([2, crate::game::map::spells::fail::OUT_OF_RANGE]);
    assert_eq!(failed, want, "out of range");
    w.wait_for("the cooldown cleared on the bar", |m| match m {
        ServerOpcodeMessage::SMSG_CLEAR_COOLDOWN(c) if c.id == tp::FIREBOLT && c.target == imp => {
            Some(())
        }
        _ => None,
    })
    .await;
}

async fn creature_of(map: &MapHandle, entry: u32) -> Guid {
    map.query(move |m| {
        m.creatures
            .iter()
            .find(|(_, c)| c.entry == entry)
            .map(|(_, c)| c.guid)
    })
    .await
    .flatten()
    .expect("the creature")
}

const WOLF: u32 = 299;
const BOSS: u32 = 300;

/// Below 16 percent a creature runs at 70 percent of its speed, below 11
/// at 60, and whole again when healed; a world boss never slows.
#[tokio::test]
async fn wounds_slow_a_creature() {
    let mut d = GameData::empty();
    d.creature_templates.insert(WOLF, template(WOLF, "Wolf", 7));
    let mut boss = template(BOSS, "Boss", 7);
    boss.rank = 3;
    d.creature_templates.insert(BOSS, boss);
    d.creature_spawns.insert(
        0,
        vec![
            spawn(1, WOLF, (START.0 + 5.0, START.1, START.2)),
            spawn(2, BOSS, (START.0 - 5.0, START.1, START.2)),
        ],
    );
    let realm = harness::realm_with(d);
    let (mut a, me, map) = enter(&realm, "A", "Alice", Race::Human).await;
    let (wolf, boss) = (creature_of(&map, WOLF).await, creature_of(&map, BOSS).await);
    a.sees(wolf).await;
    let speed = |map: MapHandle, g: Guid| async move {
        map.query(move |m| m.creature(g).unwrap().speeds.run)
            .await
            .unwrap()
    };
    let whole = speed(map.clone(), wolf).await;
    map.call(move |m| {
        crate::game::map::combat::deal_damage(m, me, wolf, 860);
        crate::game::map::combat::deal_damage(m, me, boss, 950);
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    let hurt = speed(map.clone(), wolf).await;
    assert!((hurt - whole * 0.7).abs() < 0.01, "{hurt} of {whole}");
    a.wait_for("the slower run speed", |m| match m {
        ServerOpcodeMessage::SMSG_SPLINE_SET_RUN_SPEED(s) if s.guid == wolf => Some(()),
        _ => None,
    })
    .await;
    map.call(move |m| crate::game::map::combat::deal_damage(m, me, wolf, 50));
    tokio::time::sleep(Duration::from_millis(50)).await;
    let worse = speed(map.clone(), wolf).await;
    assert!((worse - whole * 0.6).abs() < 0.01, "{worse} of {whole}");
    let boss_speed = speed(map.clone(), boss).await;
    assert!((boss_speed - whole).abs() < 0.01, "a world boss runs whole");
    // Healed, it runs whole again.
    map.call(move |m| {
        if let Some(c) = m.creature_mut(wolf) {
            c.fields
                .set_u32(crate::game::fields::UNIT_FIELD_HEALTH, 1000);
        }
        crate::game::map::combat::sync_aura_state(m, wolf);
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!((speed(map.clone(), wolf).await - whole).abs() < 0.01);
}

const CITIZEN: u32 = 301;

/// A wandering creature a player greets stands for three minutes.
#[tokio::test]
async fn a_greeted_creature_stops_wandering() {
    let mut d = GameData::empty();
    let mut citizen = template(CITIZEN, "Citizen", 12);
    citizen.npc_flags = 1;
    d.creature_templates.insert(CITIZEN, citizen);
    let mut s = spawn(1, CITIZEN, (START.0 + 3.0, START.1, START.2));
    s.movement_type = 1;
    s.wander_distance = 5.0;
    d.creature_spawns.insert(0, vec![s]);
    let realm = harness::realm_with(d);
    let (mut a, _me, map) = enter(&realm, "A", "Alice", Race::Human).await;
    let citizen = creature_of(&map, CITIZEN).await;
    a.sees(citizen).await;
    a.send(CMSG_GOSSIP_HELLO { guid: citizen }).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let waits = map
        .query(move |m| {
            let c = m.creature(citizen).unwrap();
            match c.motion {
                crate::game::map::creature::Motion::Idle { until: Some(t) } => {
                    t.duration_since(m.now) > Duration::from_secs(170)
                }
                _ => false,
            }
        })
        .await
        .unwrap();
    assert!(waits, "it waits out the talk");
}

const SHEEP: u32 = 302;
const SHEPHERD: u32 = 303;

/// A critter hurt runs, calls no one and fights no one.
#[tokio::test]
async fn a_hurt_critter_runs() {
    let mut d = GameData::empty();
    let mut sheep = template(SHEEP, "Sheep", 31);
    sheep.creature_type = 8;
    d.creature_templates.insert(SHEEP, sheep);
    let mut shepherd = template(SHEPHERD, "Shepherd", 31);
    shepherd.call_for_help_range = 30.0;
    d.creature_templates.insert(SHEPHERD, shepherd);
    d.creature_spawns.insert(
        0,
        vec![
            spawn(1, SHEEP, (START.0 + 4.0, START.1, START.2)),
            spawn(2, SHEPHERD, (START.0 + 6.0, START.1, START.2)),
        ],
    );
    let realm = harness::realm_with(d);
    let (mut a, me, map) = enter(&realm, "A", "Alice", Race::Human).await;
    let sheep = creature_of(&map, SHEEP).await;
    let shepherd = creature_of(&map, SHEPHERD).await;
    a.sees(sheep).await;
    map.call(move |m| {
        crate::game::map::combat::deal_damage(m, me, sheep, 10);
        crate::game::map::ai::attacked(m, sheep, me);
    });
    tokio::time::sleep(Duration::from_millis(600)).await;
    let (fleeing, escape, helped, health) = map
        .query(move |m| {
            let c = m.creature(sheep).unwrap();
            (
                c.fields
                    .has_flag(UNIT_FIELD_FLAGS, crate::game::map::auras::UNIT_FLAG_FLEEING),
                c.engagement.escape_at.is_some(),
                m.creature(shepherd).unwrap().combat.attacking,
                m.players[&me].fields.get_u32(UNIT_FIELD_HEALTH),
            )
        })
        .await
        .unwrap();
    assert!(fleeing && escape, "it runs");
    assert!(!helped, "and calls no one");
    let full = map
        .query(move |m| m.players[&me].fields.get_u32(UNIT_FIELD_MAXHEALTH))
        .await
        .unwrap();
    assert_eq!(health, full, "nor strikes back");
}

const BAT: u32 = 304;

/// A flyer is in the air from its spawn, circles its spawn at its height
/// rather than wandering, and flies a cyclic path round without a stop.
#[tokio::test]
async fn flyers_circle_and_fly_their_paths() {
    let mut d = GameData::empty();
    let mut bat = template(BAT, "Bat", 7);
    bat.inhabit_type = 4;
    d.creature_templates.insert(BAT, bat);
    let high = (START.0 + 10.0, START.1, START.2 + 20.0);
    let mut circler = spawn(1, BAT, high);
    circler.movement_type = 1;
    circler.wander_distance = 10.0;
    let mut cyclic = spawn(2, BAT, (START.0 - 10.0, START.1, START.2 + 20.0));
    cyclic.movement_type = 3;
    d.creature_spawns.insert(0, vec![circler, cyclic]);
    d.waypoints.insert(
        2,
        (0..4)
            .map(|i| Waypoint {
                x: START.0 - 10.0 + 8.0 * (i as f32).cos(),
                y: START.1 + 8.0 * (i as f32).sin(),
                z: START.2 + 20.0,
                ..Default::default()
            })
            .collect(),
    );
    let realm = harness::realm_with(d);
    let (mut a, _me, map) = enter(&realm, "A", "Alice", Race::Human).await;
    let bats = map
        .query(|m| {
            let mut v: Vec<(u32, Guid)> = m
                .creatures
                .iter()
                .filter(|(_, c)| c.entry == BAT)
                .map(|(_, c)| (c.spawn_guid.unwrap(), c.guid))
                .collect();
            v.sort();
            v.into_iter().map(|(_, g)| g).collect::<Vec<_>>()
        })
        .await
        .unwrap();
    for b in &bats {
        a.sees(*b).await;
    }
    let walk = |map: MapHandle, g: Guid| async move {
        map.query(move |m| {
            let c = m.creature(g).unwrap();
            let flags = c.move_flags;
            match &c.motion {
                crate::game::map::creature::Motion::Moving(w) => {
                    Some((w.fly, w.points.clone(), flags))
                }
                _ => None,
            }
        })
        .await
        .unwrap()
    };
    let mut seen = [None, None];
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        for (i, b) in bats.iter().enumerate() {
            if seen[i].is_none() {
                seen[i] = walk(map.clone(), *b).await;
            }
        }
        if seen.iter().all(Option::is_some) {
            break;
        }
    }
    let (fly, points, flags) = seen[0].clone().expect("the circler flies");
    assert!(fly, "through the air");
    assert_ne!(flags & crate::game::map::creature_motion::FLIGHT_FLAGS, 0);
    assert!(points.len() >= 9, "a lap of the circle: {}", points.len());
    assert!(
        points[1..].iter().all(|p| (p.z - high.2).abs() < 0.01),
        "at its height"
    );
    let (fly, points, _) = seen[1].clone().expect("the cyclic flyer flies");
    assert!(fly);
    assert_eq!(
        points.len(),
        1 + 4 + 1,
        "the whole lap, back to its first point"
    );
}

const LOCH_FRENZY: u32 = 1193;
const LOCH_HOME: (f32, f32, f32) = (-4798.74, -3383.09, 290.05);
/// The loch's bank, where Thistle Tea's mage climbed out.
const LOCH_BANK: (f32, f32, f32) = (-4850.95, -3374.76, 302.37);
static TERRAIN: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| crate::game::harness::real_data("terrain"));
static BAKE: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| crate::game::harness::real_data("nav"));

/// The real navmesh and terrain: a Loch Frenzy keeps to its loch, at its
/// depth, and after a target on the bank swims only to the water's edge.
#[tokio::test]
#[ignore = "extended"]
async fn a_fish_keeps_to_its_water() {
    let bake = BAKE.as_path();
    if !bake.join("Azeroth.map").exists() || !TERRAIN.as_path().exists() {
        return;
    }
    let mut d = GameData::empty();
    let mut fish = template(LOCH_FRENZY, "Loch Frenzy", 7);
    fish.inhabit_type = 2;
    fish.static_flags1 = 0x1000_0000;
    d.creature_templates.insert(LOCH_FRENZY, fish);
    let mut s = spawn(8283, LOCH_FRENZY, LOCH_HOME);
    s.movement_type = 1;
    s.wander_distance = 5.0;
    d.creature_spawns.insert(0, vec![s]);
    let mut cfg = harness::test_config();
    cfg.terrain_dir = Some(TERRAIN.clone());
    cfg.nav_dir = Some(BAKE.clone());
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = crate::game::Realm::new(cfg, store, startup, d, None);
    let map = realm.maps.get(&realm, crate::game::map::MapKey::world(0));
    let v = |(x, y, z): (f32, f32, f32)| Vector3d { x, y, z };
    let (wet, bank_wet, route) = map
        .query(move |m| {
            let fish = m
                .creatures
                .iter()
                .find(|(_, c)| c.entry == LOCH_FRENZY)
                .map(|(_, c)| c.guid)
                .unwrap();
            let wet = crate::game::map::creature_motion::water(m, v(LOCH_HOME)).is_some();
            let bank_wet = crate::game::map::creature_motion::water(m, v(LOCH_BANK)).is_some();
            let route =
                crate::game::map::creature_motion::route(m, fish, v(LOCH_HOME), v(LOCH_BANK));
            let end_wet = route
                .as_ref()
                .and_then(|(p, _, _)| p.last().copied())
                .map(|p| crate::game::map::creature_motion::water(m, p).is_some());
            (
                wet,
                bank_wet,
                route.map(|(p, reached, _)| (p.len(), reached, end_wet)),
            )
        })
        .await
        .unwrap();
    assert!(wet, "its home is in the loch");
    assert!(!bank_wet, "the bank is dry");
    let (len, reached, end_wet) = route.expect("as far as the water goes");
    assert!(len >= 2 && !reached, "it doesn't reach the bank");
    assert_eq!(end_wet, Some(true), "it stops in the water");
    // Wandering, it keeps its depth.
    let swim = map
        .query(move |m| {
            let (key, g) = m
                .creatures
                .iter()
                .find(|(_, c)| c.entry == LOCH_FRENZY)
                .map(|(k, c)| (k, c.guid))
                .unwrap();
            let now = m.now;
            for _ in 0..10 {
                if let Some(c) = m.creature_mut(g) {
                    c.motion = crate::game::map::creature::Motion::Idle { until: Some(now) };
                }
                crate::game::map::creature_motion::idle_step(m, key);
                if let crate::game::map::creature::Motion::Moving(w) =
                    &m.creature(g).unwrap().motion
                {
                    return Some(w.points.clone());
                }
            }
            None
        })
        .await
        .unwrap()
        .expect("it swims about");
    assert!(
        swim.iter().all(|p| (p.z - LOCH_HOME.2).abs() < 0.5),
        "at its depth: {swim:?}"
    );
}

/// The real navmesh: a flyer killed in the air falls to the ground under
/// it.
#[tokio::test]
#[ignore = "extended"]
async fn a_flyer_killed_in_the_air_falls() {
    let bake = BAKE.as_path();
    if !bake.join("Azeroth.map").exists() {
        return;
    }
    let mut d = GameData::empty();
    let mut bat = template(BAT, "Bat", 7);
    bat.inhabit_type = 4;
    d.creature_templates.insert(BAT, bat);
    d.creature_spawns
        .insert(0, vec![spawn(9, BAT, (-9460.0, 62.0, 80.0))]);
    let mut cfg = harness::test_config();
    cfg.nav_dir = Some(BAKE.clone());
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = crate::game::Realm::new(cfg, store, startup, d, None);
    let map = realm.maps.get(&realm, crate::game::map::MapKey::world(0));
    let (bat, ground, at, falling, flags) = map
        .query(|m| {
            let bat = m
                .creatures
                .iter()
                .find(|(_, c)| c.entry == BAT)
                .map(|(_, c)| c.guid)
                .unwrap();
            let ground = m.nav.as_mut().unwrap().ground(-9460.0, 62.0, 80.0);
            crate::game::map::combat::deal_damage(m, bat, bat, 1_000_000);
            let c = m.creature(bat).unwrap();
            let falling = match &c.motion {
                crate::game::map::creature::Motion::Moving(w) if w.falling => Some(w.duration),
                _ => None,
            };
            (bat, ground, c.position.z, falling, c.move_flags)
        })
        .await
        .unwrap();
    let ground = ground.expect("Goldshire's ground");
    // Falling from where it died, as long as a fall from there takes.
    assert!((at - 80.0).abs() < 0.1, "in the air as it dies: {at}");
    let falls = falling.expect("falling");
    let expected = crate::game::map::creature_motion::fall_time(80.0 - ground);
    assert!(
        (falls.as_secs_f32() - expected).abs() < 0.01,
        "{falls:?} for {expected}"
    );
    assert_eq!(flags & crate::game::map::creature_motion::FLIGHT_FLAGS, 0);
    tokio::time::sleep(falls + Duration::from_millis(200)).await;
    let landed = map
        .query(move |m| m.creature(bat).unwrap().position.z)
        .await
        .unwrap();
    assert!(
        (landed - ground).abs() < 0.1,
        "on the ground: {landed} over {ground}"
    );
}
