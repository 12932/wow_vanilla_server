//! The creature AI as Thistle Tea runs it: waypoints, calls for help,
//! reaction modes, creatures against creatures, flight, formations.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::scripts::GroupMember;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::combat::dist;
use crate::game::map::creature::Motion;
use crate::game::map::{Map, MapHandle, MapKey};

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

const HUMANS: u32 = 1;
/// Hostile to humans, answering calls for help.
const PACK: u32 = 14;
/// Hostile to humans, deaf to calls.
const LONERS: u32 = 15;
/// Hostile to the pack, not to humans.
const WATCH: u32 = 11;
const NEUTRAL: u32 = 32;

const WOLF: u32 = 4000;
const QUIET: u32 = 4001;
const STILL: u32 = 4002;
const GUARD: u32 = 4003;
const LONER: u32 = 4004;
const WALKER: u32 = 4005;

/// The spawns: (guid, entry, east, north).
const SPAWNS: &[(u32, u32, f32, f32)] = &[
    // A pack far to the east, out of the player's sight range.
    (10, WOLF, 40.0, 0.0),
    (11, WOLF, 40.0, 7.0),
    (12, WOLF, 40.0, 20.0),
    (13, LONER, 44.0, 0.0),
    // Beside the start: a defensive one, a passive one.
    (20, QUIET, 3.0, -3.0),
    (21, STILL, 3.0, 3.0),
    // A guard and a wolf face off to the west.
    (30, GUARD, -40.0, 0.0),
    (31, WOLF, -43.0, 0.0),
    // A walker, and a leader and its follower, to the south.
    (40, WALKER, 0.0, -40.0),
    (50, WALKER, 20.0, -40.0),
    (51, WALKER, 20.0, -43.0),
];

fn faction(id: u32, ours: u32, enemy_mask: u32, flags: u32) -> FactionTemplate {
    FactionTemplate {
        id,
        faction: id,
        ours,
        friend_mask: ours,
        enemy_mask,
        flags,
        ..Default::default()
    }
}

fn creature(entry: u32, name: &str, faction: u32) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: name.into(),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: 2,
        level_max: 2,
        faction,
        unit_class: 1,
        health_multiplier: 100.0,
        damage_multiplier: 0.01,
        base_attack_time: 2000,
        detection_range: 5.0,
        call_for_help_range: 5.0,
        ..Default::default()
    }
}

fn at(dx: f32, dy: f32) -> (f32, f32, f32) {
    (START.0 + dx, START.1 + dy, START.2)
}

fn data() -> GameData {
    let mut d = GameData::empty();
    d.faction_templates.extend([
        (HUMANS, faction(HUMANS, 0x1, 0x2, 0)),
        (PACK, faction(PACK, 0x2, 0x1 | 0x4, 0x01)),
        (LONERS, faction(LONERS, 0x2, 0x1, 0)),
        (WATCH, faction(WATCH, 0x4, 0x2, 0x01)),
        (NEUTRAL, faction(NEUTRAL, 0, 0, 0)),
    ]);
    d.class_level_stats.insert(
        (1, 2),
        ClassLevelStats {
            health: 100,
            melee_damage: 1.0,
            ..Default::default()
        },
    );
    d.creature_templates
        .insert(WOLF, creature(WOLF, "Wolf", PACK));
    let mut quiet = creature(QUIET, "Quiet Wolf", PACK);
    quiet.flags_extra = 0x2;
    d.creature_templates.insert(QUIET, quiet);
    let mut still = creature(STILL, "Still Wolf", PACK);
    still.flags_extra = 0x80;
    d.creature_templates.insert(STILL, still);
    let mut guard = creature(GUARD, "Guard", WATCH);
    guard.detection_range = 10.0;
    d.creature_templates.insert(GUARD, guard);
    d.creature_templates
        .insert(LONER, creature(LONER, "Loner", LONERS));
    d.creature_templates
        .insert(WALKER, creature(WALKER, "Walker", NEUTRAL));
    let spawns = SPAWNS
        .iter()
        .map(|(g, e, dx, dy)| {
            let (x, y, z) = at(*dx, *dy);
            CreatureSpawn {
                guid: *g,
                entries: vec![*e],
                map: 0,
                x,
                y,
                z,
                respawn_min: 60,
                respawn_max: 60,
                ..Default::default()
            }
        })
        .collect();
    d.creature_spawns.insert(0, spawns);
    // The walker's path: its nearest point is the second; 100 is no
    // facing, 0 is one.
    let point = |point, dx: f32, dy: f32, orientation, wait_ms| {
        let (x, y, z) = at(dx, dy);
        Waypoint {
            point,
            x,
            y,
            z,
            orientation,
            wait_ms,
            ..Default::default()
        }
    };
    d.waypoints.insert(
        40,
        vec![
            point(10, 0.0, -60.0, None, 0),
            point(11, 3.0, -40.0, Some(0.0), 30_000),
            point(12, 6.0, -40.0, None, 0),
        ],
    );
    // The leader and its follower, a pace behind it (pi radians).
    d.scripts.groups.insert(
        50,
        vec![
            GroupMember {
                guid: 50,
                flags: crate::game::data::scripts::GROUP_FORMATION,
                ..Default::default()
            },
            GroupMember {
                guid: 51,
                distance: 3.0,
                angle: std::f32::consts::PI,
                flags: crate::game::data::scripts::GROUP_FORMATION,
            },
        ],
    );
    d
}

async fn setup() -> (std::sync::Arc<crate::game::Realm>, Client, Guid, MapHandle) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    a.sees(unit(WOLF, 10)).await;
    (realm, a, ga, map)
}

fn unit(entry: u32, spawn: u32) -> Guid {
    guid::unit(entry, spawn)
}

fn attacking(m: &Map, g: Guid) -> bool {
    m.creature(g).is_some_and(|c| c.combat.attacking)
}

/// Called into a fight, a wolf brings those of its pack within 10 yards
/// that answer, a moment later; they call no one on; a loner, deaf to
/// calls, stays out.
#[tokio::test]
async fn a_call_for_help_brings_the_near_pack_a_moment_later() {
    let (_realm, _a, ga, map) = setup().await;
    let (caller, near, far, loner) = (
        unit(WOLF, 10),
        unit(WOLF, 11),
        unit(WOLF, 12),
        unit(LONER, 13),
    );
    let at_once = map
        .query(move |m| {
            crate::game::map::ai::attacked(m, caller, ga);
            attacking(m, near)
        })
        .await
        .unwrap();
    assert!(!at_once, "not at once");
    tokio::time::sleep(Duration::from_millis(1800)).await;
    let (near, far, loner) = map
        .query(move |m| (attacking(m, near), attacking(m, far), attacking(m, loner)))
        .await
        .unwrap();
    assert!(near, "the one within ten yards");
    assert!(!far, "not the one twenty off, nor by a chain");
    assert!(!loner, "not one deaf to calls");
}

/// A defensive creature starts no fight; a passive one, struck, takes no
/// victim, and its fight ends when no one is left in it.
#[tokio::test]
async fn reaction_modes() {
    let (_realm, mut a, ga, map) = setup().await;
    let (quiet, still) = (unit(QUIET, 20), unit(STILL, 21));
    a.sees(quiet).await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let calm = map.query(move |m| !attacking(m, quiet)).await.unwrap();
    assert!(calm, "the defensive one doesn't attack on sight");
    let (in_fight, victim) = map
        .query(move |m| {
            crate::game::map::ai::attacked(m, still, ga);
            let c = m.creature(still).unwrap();
            (
                c.fields.has_flag(
                    UNIT_FIELD_FLAGS,
                    crate::game::map::player::UNIT_FLAG_IN_COMBAT,
                ),
                c.combat.attacking,
            )
        })
        .await
        .unwrap();
    assert!(in_fight, "struck, it is in the fight");
    assert!(!victim, "but takes no victim");
    // Struck back, the defensive one fights.
    let fights = map
        .query(move |m| {
            crate::game::map::ai::attacked(m, quiet, ga);
            attacking(m, quiet)
        })
        .await
        .unwrap();
    assert!(fights);
    // No one left in its fight, the passive one's ends.
    let in_combat = |m: &Map, g: Guid| {
        m.creature(g).is_some_and(|c| {
            c.fields.has_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            )
        })
    };
    let still_in = map
        .query(move |m| {
            let still_in = in_combat(m, still);
            crate::game::map::death::kill(m, ga);
            still_in
        })
        .await
        .unwrap();
    assert!(still_in, "in the fight while its foe stands");
    let mut over = false;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        over = map
            .query(move |m| m.creature(still).is_some() && !in_combat(m, still))
            .await
            .unwrap();
        if over {
            break;
        }
    }
    assert!(over, "the passive one's fight is over");
}

/// A guard and a wolf of factions at odds fight on sight, with no player
/// in it.
#[tokio::test]
async fn creatures_fight_creatures_on_sight() {
    let (_realm, mut a, _ga, map) = setup().await;
    let (guard, wolf) = (unit(GUARD, 30), unit(WOLF, 31));
    a.sees(guard).await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let (g, w) = map
        .query(move |m| {
            let victim = |x: Guid| m.creature(x).map(|c| c.combat.victim);
            (victim(guard), victim(wolf))
        })
        .await
        .unwrap();
    assert!(
        g == Some(wolf) || w == Some(guard),
        "one of them attacks the other: {g:?} {w:?}"
    );
}

/// Script command 47: a plain flee seeks no help; with datalong, it runs
/// to the nearest of its pack that answers.
#[tokio::test]
async fn a_flee_seeks_help_only_when_told() {
    let (_realm, _a, ga, map) = setup().await;
    let (caller, near) = (unit(WOLF, 10), unit(WOLF, 11));
    let (plain, seeking) = map
        .query(move |m| {
            crate::game::map::ai::attacked(m, caller, ga);
            crate::game::map::creature_combat::flee_for_assistance(m, caller, false);
            let plain = m.creature(caller).and_then(|c| c.engagement.flee);
            if let Some(c) = m.creature_mut(caller) {
                c.engagement.flee = None;
            }
            crate::game::map::creature_combat::flee_for_assistance(m, caller, true);
            let seeking = m.creature(caller).and_then(|c| c.engagement.flee);
            (plain, seeking)
        })
        .await
        .unwrap();
    let plain = plain.expect("it flees");
    assert_eq!(plain.helper, None, "in fear, for no one");
    assert_eq!(seeking.and_then(|f| f.helper), Some(near), "to its friend");
}

/// A walker sets off for the point of its path nearest its spawn, moves
/// on to the next only on getting there, takes a point's facing (none for
/// 100), and goes back to the last point it reached after a fight.
#[tokio::test]
async fn waypoints_as_thistle_tea_walks_them() {
    let (_realm, mut a, ga, map) = setup().await;
    let walker = unit(WALKER, 40);
    a.sees(walker).await;
    let bound = map
        .query(move |m| m.creature(walker).map(|c| c.waypoint))
        .await
        .unwrap();
    assert_eq!(bound, Some(1), "bound for the point nearest its spawn");
    // Three yards at a walk.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    let (last, next, facing) = map
        .query(move |m| {
            let c = m.creature(walker).unwrap();
            (c.last_waypoint, c.waypoint, c.orientation)
        })
        .await
        .unwrap();
    assert_eq!(last, Some(1), "reached it");
    assert_eq!(next, 2, "bound for the next");
    assert_eq!(facing, 0.0, "a point's facing of 0 is taken");
    // A fight, then home: to the point it reached, not its spawn.
    let home = map
        .query(move |m| {
            crate::game::map::ai::attacked(m, walker, ga);
            // Drawn off a way.
            let (x, y, z) = at(0.0, -50.0);
            if let Some(c) = m.creature_mut(walker) {
                c.motion = Motion::Idle { until: None };
                c.position = Vector3d { x, y, z };
            }
            crate::game::map::ai::enter_evade(m, walker);
            match &m.creature(walker).unwrap().motion {
                Motion::Moving(w) => Some(w.destination()),
                _ => None,
            }
        })
        .await
        .unwrap();
    let (x, y, _) = at(3.0, -40.0);
    let home = home.expect("on its way back");
    assert!(
        dist(home, Vector3d { x, y, z: START.2 }) < 0.5,
        "back to its last point: {home:?}"
    );
}

/// A formation's angles are radians: pi puts the follower behind.
#[tokio::test]
async fn a_follower_keeps_its_place_in_radians() {
    let (_realm, mut a, _ga, map) = setup().await;
    let (leader, follower) = (unit(WALKER, 50), unit(WALKER, 51));
    a.sees(follower).await;
    let (x, y, z) = at(30.0, -40.0);
    let to = Vector3d { x, y, z };
    let place = map
        .query(move |m| {
            let now = m.now;
            let c = m.creature_mut(leader).unwrap();
            let from = c.position;
            c.walk(vec![from, to], false, now, 1);
            crate::game::map::creature_groups::leader_walked(m, leader);
            match &m.creature(follower).unwrap().motion {
                Motion::Moving(w) => Some(w.destination()),
                _ => None,
            }
        })
        .await
        .unwrap()
        .expect("it follows");
    assert!((place.x - (to.x - 3.0)).abs() < 0.5, "behind: {place:?}");
    assert!((place.y - to.y).abs() < 0.5, "behind: {place:?}");
}
