//! The defaults taken from mangos-classic (`mangosd.conf.dist.in`, read in
//! `World.cpp`), each through the harness's own config, which keeps them:
//! a dungeon's view of 170 yards, a leash of 30, a call for help of 5, and
//! the corpses of rares, rare elites and bosses lying 15, 20 and 60
//! minutes. (The call for help is in `tests_ai5`.)

use std::sync::Arc;
use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::scripts::MapTemplate;
use crate::game::data::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::MapKey;
use crate::game::map::creature::Life;
use crate::game::{Realm, RealmConfig};

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const DUNGEON: u32 = 36;
const WOLF: u32 = 2100;
const SHEEP: u32 = 2101;
const RARE: u32 = 2102;
const RARE_ELITE: u32 = 2103;
const BOSS: u32 = 2104;

fn faction(id: u32, ours: u32, friend_mask: u32, enemy_mask: u32) -> (u32, FactionTemplate) {
    (
        id,
        FactionTemplate {
            id,
            faction: id,
            ours,
            friend_mask,
            enemy_mask,
            ..Default::default()
        },
    )
}

fn template(entry: u32, faction: u32, rank: u32) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: format!("Creature {entry}"),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: 2,
        level_max: 2,
        faction,
        rank,
        unit_class: 1,
        health_multiplier: 1.0,
        damage_multiplier: 1.0,
        base_attack_time: 2000,
        // A short reach, so the leash is the config's, not half again it.
        detection_range: 5.0,
        ..Default::default()
    }
}

fn spawn(guid: u32, entry: u32, map: u32, (x, y, z): (f32, f32, f32)) -> CreatureSpawn {
    CreatureSpawn {
        guid,
        entries: vec![entry],
        map,
        x,
        y,
        z,
        // Two hours: no corpse is cut short by its respawn.
        respawn_min: 7200,
        respawn_max: 7200,
        ..Default::default()
    }
}

/// A hostile wolf, and a rare, a rare elite and a boss, neutral, near the
/// start; in a dungeon, sheep 150 and 190 yards from where a player lands.
fn data() -> GameData {
    let mut d = GameData::empty();
    d.faction_templates.extend([
        faction(1, 3, 2, 12),
        faction(14, 8, 0, 1),
        faction(32, 0, 0, 0),
    ]);
    for (entry, faction, rank) in [
        (WOLF, 14, 0),
        (SHEEP, 32, 0),
        (RARE, 32, 4),
        (RARE_ELITE, 32, 2),
        (BOSS, 32, 3),
    ] {
        d.creature_templates
            .insert(entry, template(entry, faction, rank));
    }
    let near = |dx: f32| (START.0 + dx, START.1, START.2);
    d.creature_spawns.insert(
        0,
        vec![
            spawn(10, WOLF, 0, near(15.0)),
            spawn(11, RARE, 0, near(-3.0)),
            spawn(12, RARE_ELITE, 0, near(-4.0)),
            spawn(13, BOSS, 0, near(-5.0)),
        ],
    );
    d.maps.insert(
        DUNGEON,
        MapInfo {
            id: DUNGEON,
            internal_name: "Dungeon".into(),
            name: "Dungeon".into(),
            instance_type: 1,
            player_limit: 0,
        },
    );
    d.scripts.map_templates.insert(
        DUNGEON,
        MapTemplate {
            map: DUNGEON,
            map_type: 1,
            player_limit: 5,
            ..Default::default()
        },
    );
    d.creature_spawns.insert(
        DUNGEON,
        vec![
            spawn(20, SHEEP, DUNGEON, (150.0, 0.0, 0.0)),
            spawn(21, SHEEP, DUNGEON, (190.0, 0.0, 0.0)),
        ],
    );
    d
}

fn realm() -> Arc<Realm> {
    realm_with(harness::test_config())
}

fn realm_with(cfg: RealmConfig) -> Arc<Realm> {
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(cfg, store, startup, data(), None)
}

/// The defaults are mangos-classic's, and the harness keeps them.
#[test]
fn the_defaults_are_mangos_classics() {
    let c = crate::config::ServerConfig::default();
    assert_eq!(
        c.visibility.instances_yards, 170.0,
        "Visibility.Distance.Instances"
    );
    assert_eq!(c.creatures.leash_radius, 30.0, "LeashRadius");
    assert_eq!(
        c.creatures.assistance_radius, 5.0,
        "CreatureFamilyAssistanceRadius"
    );
    assert_eq!(
        (
            c.creatures.corpse_decay_rare,
            c.creatures.corpse_decay_rare_elite,
            c.creatures.corpse_decay_world_boss
        ),
        (900, 1200, 3600),
        "Corpse.Decay.RARE, RAREELITE and WORLDBOSS"
    );
    let ours = harness::test_config();
    let theirs = RealmConfig::from_config(&c);
    assert_eq!(
        ours.instance_visibility_yards,
        theirs.instance_visibility_yards
    );
    assert_eq!(ours.creatures.leash_radius, theirs.creatures.leash_radius);
    assert_eq!(
        ours.creatures.assistance_radius,
        theirs.creatures.assistance_radius
    );
}

/// In a dungeon, a player sees 170 yards: a sheep 150 yards off (beyond
/// wvs's old 100), not one 190 off.
#[tokio::test]
async fn a_dungeon_is_seen_to_a_hundred_and_seventy_yards() {
    let realm = realm();
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let (near, far) = (guid::unit(SHEEP, 20), guid::unit(SHEEP, 21));
    realm.registry.online(ga).unwrap().map().call(move |m| {
        m.teleport(
            ga,
            MapKey::world(DUNGEON),
            Vector3d {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            0.0,
        )
    });
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    a.sees(near).await;
    let map = realm.registry.online(ga).unwrap().map();
    assert_ne!(map.key.instance, 0, "in a copy");
    let (radius, knows_far) = map
        .query(move |m| {
            (
                crate::game::map::visibility::radius(m),
                m.players.get(&ga).is_some_and(|p| p.known.contains(&far)),
            )
        })
        .await
        .unwrap();
    assert_eq!(radius, 170.0);
    assert!(!knows_far, "not 190 yards off");
}

/// A creature 40 yards from where its fight began (within wvs's old 50),
/// its victim further, nothing done to it for 13 seconds, goes home; 25
/// yards off, it holds.
#[tokio::test]
async fn the_leash_is_thirty_yards() {
    let realm = realm();
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let wolf = guid::unit(WOLF, 10);
    a.sees(wolf).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let (reach, near, far) = map
        .query(move |m| {
            crate::game::map::ai::attacked(m, wolf, ga);
            let reach = 1.5 * crate::game::map::ai::attack_distance(m, wolf, ga);
            let origin = m.creature(wolf).unwrap().engagement.origin.unwrap();
            let off = |m: &mut crate::game::map::Map, yards: f32| {
                let c = m.creature_mut(wolf).unwrap();
                c.position = Vector3d {
                    x: origin.x + yards,
                    ..origin
                };
                c.engagement.last_hostile = Some(Instant::now() - Duration::from_secs(13));
                m.players.get_mut(&ga).unwrap().movement.position = Vector3d {
                    x: origin.x + yards + 5.0,
                    ..origin
                };
                crate::game::map::creature_combat::should_evade(m, wolf, ga)
            };
            let near = off(m, 25.0);
            let far = off(m, 40.0);
            (reach, near, far)
        })
        .await
        .unwrap();
    assert!(reach < 25.0, "its own reach is shorter: {reach}");
    assert!(!near, "25 yards off, it holds");
    assert!(far, "40 yards off, it goes home");
}

/// A rare's corpse lies 15 minutes, a rare elite's 20 and a boss's an
/// hour, as mangos's `Creature::InitEntry` sets them by rank.
#[tokio::test]
async fn rare_and_boss_corpses_lie_as_mangos_has_them() {
    let realm = realm();
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let all: [(Guid, u64); 3] = [
        (guid::unit(RARE, 11), 900),
        (guid::unit(RARE_ELITE, 12), 1200),
        (guid::unit(BOSS, 13), 3600),
    ];
    for (g, _) in all {
        a.sees(g).await;
    }
    let map = realm.maps.get(&realm, MapKey::world(0));
    let lie = map
        .query(move |m| {
            all.map(|(g, _)| {
                crate::game::map::ai::die(m, g, ga);
                let now = m.now;
                m.creature(g).and_then(|c| match c.life {
                    Life::Dead { corpse_until, .. } => Some((corpse_until - now).as_secs()),
                    _ => None,
                })
            })
        })
        .await
        .unwrap();
    assert_eq!(lie, all.map(|(_, secs)| Some(secs)));
}
