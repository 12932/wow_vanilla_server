//! Round 9's world tests of Thistle Tea's systems, ported from
//! `test/game/world/system/`: a battleground event's respawns stopped
//! with its living left, the Baron's run's lines, a map event won by a
//! goal creature by its swapped-in source, what reads a map event seeing
//! it change at once, the object-fit condition by an object's spawn
//! alone, a distance unknown without both ends, and spawns back as
//! themselves.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::scripts::{CF_SWAP_TARGETS, Condition, MapTemplate};
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::battleground as bg;
use crate::game::map::conditions::{Truth, check};
use crate::game::map::instance_script::{self, DONE, IN_PROGRESS, get_data, set_data};
use crate::game::map::scripts::{self, creature_of_spawn, execute, object_of_spawn};
use crate::game::map::{Map, MapHandle, MapKey, ai, auras, combat, map_events, objects};
use crate::game::tests_p3_combat::{BEAST, BRUTE, HELPER, START, setup_with, spawn};
use crate::game::tests_p3_entity::{on, step};

/// A realm whose empty copies stay an hour: a test opens one with no one
/// in it.
fn realm_of(d: GameData) -> Arc<Realm> {
    let mut cfg = harness::test_config();
    cfg.instance_unload = Duration::from_secs(3600);
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(cfg, store, startup, d, None)
}

fn object_template(entry: u32) -> GameObjectTemplate {
    GameObjectTemplate {
        entry,
        kind: 0,
        display_id: 1,
        name: format!("Object {entry}"),
        size: 1.0,
        ..Default::default()
    }
}

fn object_spawn(guid: u32, entry: u32, map: u32, at: (f32, f32, f32)) -> GameObjectSpawn {
    GameObjectSpawn {
        guid,
        entry,
        map,
        x: at.0,
        y: at.1,
        z: at.2,
        rotation: [0.0, 0.0, 0.0, 1.0],
        state: 1,
        respawn_min: 60,
        respawn_max: 60,
        ..Default::default()
    }
}

fn creature_spawn(guid: u32, entry: u32, map: u32, at: (f32, f32, f32)) -> CreatureSpawn {
    CreatureSpawn {
        guid,
        entries: vec![entry],
        map,
        x: at.0,
        y: at.1,
        z: at.2,
        respawn_min: 60,
        respawn_max: 60,
        ..Default::default()
    }
}

// A battleground's event.

const WSG: u32 = 489;
/// Warsong Gulch's spawns: a defender and a banner of event 9, and a
/// creature of none.
const DEFENDER: u32 = 98_100;
const BANNER: u32 = 98_101;
const UNBOUND: u32 = 98_102;

/// Thistle Tea's `Spawns.stop_respawns/2` "disables future activation
/// without asking the pool to remove living members": an event's respawns
/// stopped (as Alterac Valley's assaulted graveyards and fallen captains
/// stop theirs), its banner no longer belongs while a creature of no event
/// still does; its living defender and its banner stay; the defender
/// killed doesn't come back at its respawn; the event on again brings it
/// back (vmangos's `BattleGround::SpawnEvent` without the despawn).
#[tokio::test]
async fn an_events_respawns_stopped_leave_its_living_and_keep_its_dead_away() {
    let mut d = GameData::empty();
    crate::game::tests_pvp::battleground(&mut d, WSG, 2, "WarsongGulch", [9_801, 9_802]);
    for entry in [DEFENDER, UNBOUND] {
        crate::game::tests_pvp2::npc(&mut d, entry, "Defender", 14);
    }
    d.creature_spawns.insert(
        WSG,
        vec![
            creature_spawn(DEFENDER, DEFENDER, WSG, (10.0, 0.0, 0.0)),
            creature_spawn(UNBOUND, UNBOUND, WSG, (12.0, 0.0, 0.0)),
        ],
    );
    d.bg_creature_events.insert(DEFENDER, vec![(9, 0)]);
    d.gameobject_templates
        .insert(BANNER, object_template(BANNER));
    crate::game::tests_pvp::bound_object(&mut d, WSG, BANNER, BANNER, (14.0, 0.0), (9, 0));
    let realm = realm_of(d);
    let map = realm.maps.get(
        &realm,
        MapKey {
            map: WSG,
            instance: 1,
        },
    );
    let out = on(&map, |m| {
        bg::set_event(m, 9, Some(0));
        let defender = creature_of_spawn(m, DEFENDER).expect("the event's defender");
        let unbound = creature_of_spawn(m, UNBOUND).unwrap();
        let banner_on = bg::spawn_allowed(m, false, BANNER);
        bg::stop_respawns(m, 9);
        let allowed = (
            bg::spawn_allowed(m, false, BANNER),
            bg::spawn_allowed(m, true, UNBOUND),
        );
        let stayed = (
            m.creature(defender).is_some_and(|c| c.alive()),
            object_of_spawn(m, BANNER).is_some(),
        );
        combat::kill(m, unbound, defender);
        m.now += Duration::from_secs(61);
        ai::update_dead(m);
        let respawned = creature_of_spawn(m, DEFENDER).is_some();
        bg::set_event(m, 9, Some(0));
        let back = creature_of_spawn(m, DEFENDER)
            .and_then(|g| m.creature(g))
            .is_some_and(|c| c.alive());
        (banner_on, allowed, stayed, respawned, back)
    })
    .await;
    assert_eq!(
        out,
        (true, (false, true), (true, true), false, true),
        "on; off for its own, not the rest; its living stay; its dead stay away; on, back"
    );
}

// Stratholme.

const STRATHOLME: u32 = 329;
const AT: (f32, f32, f32) = (3_395.0, -3_380.0, 143.0);
const BARON: u32 = 10_440;
const GAUNTLET_GATE: u32 = 175_357;
/// The run's lines and spells: at its start, and with ten minutes left.
const RUN_LINES: [u32; 2] = [11_812, 11_813];
const RUN_SPELLS: [u32; 2] = [27_861, 27_863];

/// Stratholme, its script the template's: the Baron beside where Alice
/// comes in, and his run's lines and spells.
fn stratholme() -> GameData {
    let mut d = GameData::empty();
    d.maps.insert(
        STRATHOLME,
        MapInfo {
            id: STRATHOLME,
            internal_name: "Stratholme".into(),
            name: "Stratholme".into(),
            instance_type: 1,
            player_limit: 0,
        },
    );
    d.scripts.map_templates.insert(
        STRATHOLME,
        MapTemplate {
            map: STRATHOLME,
            map_type: 1,
            player_limit: 5,
            script_name: "instance_stratholme".into(),
            ..Default::default()
        },
    );
    d.creature_templates.insert(
        BARON,
        crate::game::tests_p4_items_quests::template(BARON, "Baron Rivendare"),
    );
    d.creature_spawns.insert(
        STRATHOLME,
        vec![creature_spawn(
            98_112,
            BARON,
            STRATHOLME,
            (AT.0 + 3.0, AT.1, AT.2),
        )],
    );
    for id in RUN_LINES {
        crate::game::tests_p4_items_quests::line(&mut d, id);
    }
    for id in RUN_SPELLS {
        let mut s = SpellTemplate {
            id,
            name: format!("Run {id}"),
            equipped_item_class: -1,
            ..Default::default()
        };
        s.effects[0] = SpellEffect {
            effect: 6,
            aura: 4,
            target_a: 1,
            ..Default::default()
        };
        d.spells.spells.insert(id, s);
    }
    d
}

/// Alice in her copy of Stratholme.
async fn in_stratholme() -> (Arc<Realm>, Client, Guid, MapHandle) {
    let realm = realm_of(stratholme());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let from = realm.registry.online(g).unwrap().map();
    from.call(move |m| {
        m.teleport(
            g,
            MapKey::world(STRATHOLME),
            Vector3d {
                x: AT.0,
                y: AT.1,
                z: AT.2,
            },
            0.0,
        )
    });
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    a.sees(g).await;
    let map = realm.registry.online(g).unwrap().map();
    (realm, a, g, map)
}

async fn heard(a: &mut Client, what: &str) -> String {
    a.wait_for(what, |m| match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(c) => Some(c.message.clone()),
        _ => None,
    })
    .await
}

/// Thistle Tea's `InstanceTest` "dispatches exact-copy effects and owns
/// script timers", what `the_barons_run_starts_once_and_ends_won_or_lost`
/// (tests_p3_core_strat.rs) leaves out: the gauntlet's gate opened, the
/// Baron says his first line (11812) as the run's spell goes on Alice; the
/// first of its timers, thirty-five minutes on, has him say the next
/// (11813) as its spell goes on her; his field (5) done ends the run, only
/// Ysida's reward to come (vmangos's `instance_stratholme`).
#[tokio::test]
async fn the_barons_run_has_him_speak_at_its_start_and_ten_minutes_before_its_end() {
    let (_realm, mut a, g, map) = in_stratholme().await;
    a.clear();
    let started = on(&map, move |m| {
        instance_script::object_used(m, GAUNTLET_GATE);
        let first = m.instance.timers.iter().map(|(t, _)| *t).min().unwrap();
        (
            get_data(m, 0),
            auras::auras(m, g).is_some_and(|x| x.has_spell(RUN_SPELLS[0])),
            first.duration_since(m.now).as_secs() / 60,
        )
    })
    .await;
    assert_eq!(
        started,
        (IN_PROGRESS, true, 35),
        "its first timer at 35 minutes"
    );
    assert_eq!(heard(&mut a, "the run's first line").await, "Line 11812.");
    let cast = on(&map, move |m| {
        let first = m.instance.timers.iter().map(|(t, _)| *t).min().unwrap();
        for (t, _) in m.instance.timers.iter_mut() {
            if *t == first {
                *t = m.now;
            }
        }
        instance_script::update(m);
        auras::auras(m, g).is_some_and(|x| x.has_spell(RUN_SPELLS[1]))
    })
    .await;
    assert!(cast, "ten minutes left");
    assert_eq!(heard(&mut a, "ten minutes left").await, "Line 11813.");
    let won = on(&map, |m| {
        set_data(m, 5, DONE, 0);
        (get_data(m, 0), m.instance.timers.len())
    })
    .await;
    assert_eq!(won, (DONE, 1), "won; only the reward to come");
}

// Map events and conditions.

/// The goal creature, near the event's source; the script a won event
/// runs, marking its source (emote state 10).
const GOAL: u32 = BEAST;
const WON: u32 = 98_120;
/// Conditions: the goal within six yards, its targets swapped and not;
/// map event 2's data 0 at two; map event 2 under way; within eight
/// yards of the target; the object of spawn `SHARED` spawned (and its
/// child).
const GOAL_NEAR: u32 = 98_121;
const GOAL_NEAR_UNSWAPPED: u32 = 98_122;
const DATA_AT_TWO: u32 = 98_123;
const EVENT_ON: u32 = 98_124;
const WITHIN_EIGHT: u32 = 98_125;
const OBJECT_FIT: u32 = 98_126;
const SPAWNED: u32 = 98_127;
/// An object's spawn and a creature's, of one number.
const SHARED: u32 = 98_128;
const SHARED_OBJECT: u32 = 98_129;

fn events_world() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    d.scripts.generic.insert(
        WON,
        vec![step(2, [u32::from(UNIT_NPC_EMOTESTATE), 10, 0, 0])],
    );
    for (entry, kind, values, flags) in [
        (GOAL_NEAR, 20, [GOAL as i32, 6, 0, 0], CF_SWAP_TARGETS),
        (GOAL_NEAR_UNSWAPPED, 20, [GOAL as i32, 6, 0, 0], 0),
        (DATA_AT_TWO, 35, [2, 0, 2, 0], 0),
        (EVENT_ON, 36, [2, 0, 0, 0], 0),
        (WITHIN_EIGHT, 38, [8, 2, 0, 0], 0),
        (OBJECT_FIT, 50, [SHARED as i32, SPAWNED as i32, 0, 0], 0),
        (SPAWNED, 48, [0; 4], 0),
    ] {
        d.scripts.conditions.insert(
            entry,
            Condition {
                entry,
                kind,
                values,
                flags,
            },
        );
    }
    d.gameobject_templates
        .insert(SHARED_OBJECT, object_template(SHARED_OBJECT));
    d.gameobject_spawns.insert(
        0,
        vec![object_spawn(
            SHARED,
            SHARED_OBJECT,
            0,
            (START.0, START.1 + 12.0, START.2),
        )],
    );
    d.creature_spawns.insert(
        0,
        vec![creature_spawn(
            SHARED,
            HELPER,
            0,
            (START.0, START.1 - 12.0, START.2),
        )],
    );
    d
}

fn marked(m: &Map, g: Guid) -> bool {
    m.unit_fields(g)
        .is_some_and(|f| f.get_u32(UNIT_NPC_EMOTESTATE) == 10)
}

/// Thistle Tea's `ScriptedEvent` "a goal creature beside the swapped-in
/// source satisfies the success condition": a map event started by a
/// creature fifteen yards from Alice, its success "the goal creature
/// within six yards" with its targets swapped, is won at its next check by
/// the goal four yards past the creature, nineteen from her, and runs its
/// success script; the same condition unswapped looks about her, and its
/// event goes on (vmangos's `CONDITION_FLAG_SWAP_TARGETS` on
/// `ScriptedEvent::UpdateEvent`'s target and source).
#[tokio::test]
async fn a_map_event_is_won_by_its_goal_beside_the_swapped_in_source() {
    let (_realm, _a, ga, map) = setup_with(events_world(), Class::Warrior).await;
    let out = on(&map, move |m| {
        let host = spawn(m, ga, BRUTE, 20, 15.0);
        spawn(m, ga, GOAL, 20, 19.0);
        for (event, condition) in [(1, GOAL_NEAR), (3, GOAL_NEAR_UNSWAPPED)] {
            let start = crate::game::data::scripts::ScriptStep {
                dataint: [condition as i32, WON as i32, 0, 0],
                ..step(61, [event, 600, 0, 0])
            };
            assert!(execute(m, &start, host, ga));
        }
        m.now += Duration::from_millis(600);
        map_events::update(m);
        (
            map_events::active(m, 1),
            map_events::active(m, 3),
            marked(m, host),
        )
    })
    .await;
    assert_eq!(
        out,
        (false, true, true),
        "won, its script run; the other on"
    );
}

/// Thistle Tea's `ScriptedEvent` "map event lifecycle, targets and changed
/// data announce world facts", what its announcing is for: a condition on
/// a map event sees it start, its data set and its end at once, with no
/// update between (wvs reads the event where Thistle Tea announces it).
#[tokio::test]
async fn what_reads_a_map_event_sees_it_change_at_once() {
    let (_realm, _a, ga, map) = setup_with(events_world(), Class::Warrior).await;
    let seen = on(&map, move |m| {
        let host = spawn(m, ga, BRUTE, 20, 5.0);
        let look = |m: &Map| {
            [EVENT_ON, DATA_AT_TWO].map(|c| check(m, c, Some(host), Some(ga)) == Truth::Met)
        };
        let before = look(m);
        execute(m, &step(61, [2, 600, 0, 0]), host, ga);
        let started = look(m);
        execute(m, &step(65, [2, 0, 2, 0]), host, ga);
        let set = look(m);
        execute(m, &step(62, [2, 1, 0, 0]), host, ga);
        [before, started, set, look(m)]
    })
    .await;
    assert_eq!(
        seen,
        [[false, false], [true, false], [true, true], [false, false]]
    );
}

/// Thistle Tea's `ScriptedEvent` "object-fit conditions isolate copies and
/// entity types sharing a database id": the object-fit condition (50) on
/// spawn 98128 is met by that object's spawn, and unmet once it is gone,
/// though a creature of the same spawn number stands in the map (a
/// creature's and an object's spawns are numbered apart, as vmangos's
/// tables are). Copies keep apart as each is its own map.
#[tokio::test]
async fn the_object_fit_condition_is_an_objects_spawn_not_a_creatures_of_its_number() {
    let (_realm, _a, ga, map) = setup_with(events_world(), Class::Warrior).await;
    let out = on(&map, move |m| {
        let object = object_of_spawn(m, SHARED).expect("the object");
        let creature = creature_of_spawn(m, SHARED).expect("the creature");
        let there = check(m, OBJECT_FIT, None, Some(ga));
        objects::despawn(m, object);
        let gone = check(m, OBJECT_FIT, None, Some(ga));
        (there, gone, m.creature(creature).is_some_and(|c| c.alive()))
    })
    .await;
    assert_eq!(out, (Truth::Met, Truth::Unmet, true));
}

/// Thistle Tea's `ScriptedEvent.condition_results/4` "treats spawn-time
/// distance without an invoker as unknown": a distance to the target
/// (38, at most eight yards) with its source, its target, or both missing
/// isn't judged, nor with one that isn't in the map, and isn't met; with
/// both, fifteen yards apart, it is unmet.
#[tokio::test]
async fn a_distance_without_both_ends_is_unknown() {
    let (_realm, _a, ga, map) = setup_with(events_world(), Class::Warrior).await;
    let out = on(&map, move |m| {
        let host = spawn(m, ga, BRUTE, 20, 15.0);
        let away = crate::game::guid::unit(BRUTE, 0x7F_FFFF);
        let unknown = [
            (Some(host), None),
            (None, Some(ga)),
            (None, None),
            (Some(away), Some(ga)),
        ]
        .map(|(s, t)| {
            (
                check(m, WITHIN_EIGHT, s, t),
                crate::game::map::conditions::met(m, WITHIN_EIGHT, s, t),
            )
        });
        (unknown, check(m, WITHIN_EIGHT, Some(host), Some(ga)))
    })
    .await;
    assert_eq!(out, ([(Truth::Unknown, false); 4], Truth::Unmet));
}

// Spawns back.

/// Thistle Tea's `SpawnPool` "retains database identity across instance
/// refreshes and repeated suspension", "reactivates a scripted object
/// after its respawn delay" and "loads and despawns a game object spawn
/// with delayed reactivation": an object a script takes away for a
/// hundred seconds (81) isn't back a second before, and is at them, the
/// same spawn and guid, which a script finds by its spawn again, three
/// times over; a creature of a spawn killed three times comes back each
/// time as its spawn's, found by it (vmangos's
/// `SCRIPT_COMMAND_DESPAWN_GAMEOBJECT`, and a respawn's same guid).
#[tokio::test]
async fn an_object_and_a_creature_come_back_as_their_spawns_each_time() {
    let (_realm, _a, ga, map) = setup_with(events_world(), Class::Warrior).await;
    let out = on(&map, move |m| {
        let (object, creature) = (
            object_of_spawn(m, SHARED).unwrap(),
            creature_of_spawn(m, SHARED).unwrap(),
        );
        let mut seen = Vec::new();
        for _ in 0..3 {
            assert!(execute(m, &step(81, [SHARED, 100, 0, 0]), ga, ga));
            m.now += Duration::from_secs(99);
            objects::update(m);
            let early = object_of_spawn(m, SHARED);
            m.now += Duration::from_secs(1);
            objects::update(m);
            let o = object_of_spawn(m, SHARED);
            let c = creature_of_spawn(m, SHARED).unwrap();
            combat::kill(m, ga, c);
            m.now += Duration::from_secs(61);
            ai::update_dead(m);
            scripts::update(m);
            let c =
                creature_of_spawn(m, SHARED).filter(|g| m.creature(*g).is_some_and(|c| c.alive()));
            seen.push((early, o, c));
        }
        (object, creature, seen)
    })
    .await;
    let (object, creature, seen) = out;
    assert_eq!(seen, vec![(None, Some(object), Some(creature)); 3]);
}
