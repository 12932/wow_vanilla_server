//! Phase 3's script routing tests, ported from Thistle Tea's
//! `test/game/world/entity/script_routing_test.exs` and
//! `event_sink/group_scripts_test.exs` (docs/parity/matrix.csv): where a
//! script's steps run, and where its tail stops.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::scripts::*;
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::scripts::{self, server_variable};
use crate::game::map::{Map, MapKey};
use crate::game::tests_p3_combat::{BRUTE, data, setup_with, spawn};
use crate::game::tests_p3_entity::{on, step};

/// A condition always met here (map 0), and one never.
const HERE: u32 = 9_100;
const ELSEWHERE: u32 = 9_101;
/// Server variable 7_777 is six.
const VARIABLE_IS_SIX: u32 = 9_102;
const VARIABLE: u32 = 7_777;

const STAND: u32 = 28;

fn stand(n: u32) -> ScriptStep {
    step(STAND, [n, 0, 0, 0])
}

fn with_flags(mut s: ScriptStep, flags: u32) -> ScriptStep {
    s.data_flags = flags;
    s
}

fn delayed(mut s: ScriptStep, ms: u32) -> ScriptStep {
    s.delay_ms = ms;
    s
}

fn conditioned(mut s: ScriptStep, condition: u32) -> ScriptStep {
    s.condition = condition;
    s
}

fn world() -> GameData {
    let mut d = data();
    for (entry, kind, values) in [
        (HERE, 33, [0, 0, 0, 0]),
        (ELSEWHERE, 33, [1, 0, 0, 0]),
        (VARIABLE_IS_SIX, 11, [VARIABLE as i32, 6, 0, 0]),
    ] {
        d.scripts.conditions.insert(
            entry,
            Condition {
                entry,
                kind,
                values,
                flags: 0,
            },
        );
    }
    let abort = SF_ABORT_ON_FAILURE;
    let g = &mut d.scripts.generic;
    // A terminate on a condition met ends it: neither the next step nor the
    // later one runs.
    g.insert(
        1,
        vec![
            stand(1),
            with_flags(step(32, [HERE, 0, 0, 0]), abort),
            stand(2),
            delayed(stand(3), 50),
        ],
    );
    // A step whose condition fails, with the abort flag.
    g.insert(
        2,
        vec![
            stand(1),
            with_flags(conditioned(stand(5), ELSEWHERE), abort),
            stand(2),
            delayed(stand(3), 50),
        ],
    );
    // A step for a target that isn't here: with the abort flag, and
    // without.
    g.insert(
        3,
        vec![
            stand(1),
            with_flags(stand(5), SF_SWAP_INITIAL_TARGETS | abort),
            stand(2),
        ],
    );
    g.insert(
        4,
        vec![
            stand(1),
            with_flags(stand(5), SF_SWAP_INITIAL_TARGETS),
            stand(2),
        ],
    );
    // Swapped to the target and back: the caller's.
    g.insert(
        5,
        vec![
            with_flags(stand(1), SF_SWAP_INITIAL_TARGETS | SF_SWAP_FINAL_TARGETS),
            stand(2),
        ],
    );
    // A server variable set, seen by the very next step.
    g.insert(
        6,
        vec![
            step(54, [VARIABLE, 6, 0, 0]),
            with_flags(conditioned(stand(1), VARIABLE_IS_SIX), abort),
            stand(2),
        ],
    );
    // A map event started, and the script's tail.
    g.insert(
        7,
        vec![
            stand(0),
            with_flags(step(61, [648, 60, 0, 0]), abort),
            stand(1),
            delayed(stand(2), 50),
        ],
    );
    // A group's script, and a player's: kneel.
    g.insert(8, vec![stand(8)]);
    g.insert(9, vec![delayed(stand(8), 150)]);
    d
}

fn stand_state(m: &Map, g: Guid) -> u8 {
    m.unit_fields(g)
        .map_or(u8::MAX, |f| f.get_byte(UNIT_FIELD_BYTES_1, 0))
}

/// Thistle Tea's "a remote termination or failed condition cancels the
/// caller's immediate and delayed tail", "missing or wrong-world
/// recipients honor the abort flag" and "a command can swap back to its
/// caller without blocking either owner": a terminating step, or a step
/// whose condition fails or whose target isn't here with the abort flag,
/// ends its script there, its later steps and its delayed ones too;
/// without the flag the script goes on; and a step swapped to its target
/// and back is its caller's.
#[tokio::test]
async fn a_scripts_tail_stops_where_it_aborts() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let out = on(&map, move |m| {
        let mut seen = Vec::new();
        for id in [1, 2] {
            let a = spawn(m, ga, BRUTE, 20, 3.0);
            scripts::start(m, ScriptTable::Generic, id, a, Guid::zero());
            seen.push(a);
        }
        let absent = crate::game::guid::unit(BRUTE, 0x7F_FFFF);
        for id in [3, 4] {
            let a = spawn(m, ga, BRUTE, 20, 3.0);
            scripts::start(m, ScriptTable::Generic, id, a, absent);
            seen.push(a);
        }
        let (a, b) = (spawn(m, ga, BRUTE, 20, 3.0), spawn(m, ga, BRUTE, 20, 4.0));
        scripts::start(m, ScriptTable::Generic, 5, a, b);
        seen.push(a);
        seen.push(b);
        seen
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let states = on(&map, move |m| {
        out.iter().map(|g| stand_state(m, *g)).collect::<Vec<_>>()
    })
    .await;
    assert_eq!(
        states[0], 1,
        "a terminating step ends it, the delayed step too"
    );
    assert_eq!(
        states[1], 1,
        "so does a failed condition with the abort flag"
    );
    assert_eq!(states[2], 1, "and a target not here with it");
    assert_eq!(states[3], 2, "without the flag, it goes on");
    assert_eq!(
        (states[4], states[5]),
        (2, 0),
        "swapped and back: the caller's"
    );
}

/// Thistle Tea's "server variables are visible to the immediate tail and
/// other worlds": a script's server variable is read by its very next
/// step, and on any map.
#[tokio::test]
async fn a_server_variable_is_seen_at_once_and_everywhere() {
    let (realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let a = on(&map, move |m| {
        let a = spawn(m, ga, BRUTE, 20, 3.0);
        scripts::start(m, ScriptTable::Generic, 6, a, Guid::zero());
        a
    })
    .await;
    let state = on(&map, move |m| stand_state(m, a)).await;
    assert_eq!(state, 2, "the next step saw it");
    assert_eq!(server_variable(VARIABLE), 6);
    let kalimdor = realm.maps.get(&realm, MapKey::world(1));
    let seen = on(&kalimdor, move |m| {
        crate::game::map::conditions::met(m, VARIABLE_IS_SIX, None, None)
    })
    .await;
    assert!(seen, "Kalimdor sees it too");
}

/// Thistle Tea's "scripts wake a dormant creature to act on the movement
/// they start" and "game object recipients execute on their own process":
/// a script's wander sets its creature wandering at once; and a step for an
/// object sets the object's state.
#[tokio::test]
async fn a_scripts_wander_and_object_steps_take_hold_at_once() {
    let mut d = world();
    d.gameobject_templates.insert(
        97_100,
        crate::game::tests_p3_entity_objects::object(97_100, 0, "Door", &[]),
    );
    let (_realm, _a, ga, map) = setup_with(d, Class::Warrior).await;
    let out = on(&map, move |m| {
        let a = spawn(m, ga, BRUTE, 20, 3.0);
        let mut wander = step(20, [1, 0, 0, 0]);
        wander.x = 5.0;
        scripts::execute(m, &wander, a, Guid::zero());
        let c = m.creature(a).unwrap();
        let now = m.now;
        let woken = matches!(c.motion, crate::game::map::creature::Motion::Idle { until: Some(t) } if t <= now);
        let wandering = (c.movement_type, c.wander_distance);
        let at = m.position_of(a).unwrap();
        let door = crate::game::map::objects::summon(m, None, 97_100, at, 0.0, 0, 0, 0, None)
            .unwrap();
        let before = m.gameobject(door).unwrap().fields.get_u32(GAMEOBJECT_STATE);
        scripts::execute(m, &step(80, [0, 0, 0, 0]), a, door);
        let after = m.gameobject(door).unwrap().fields.get_u32(GAMEOBJECT_STATE);
        (woken, wandering, before, after)
    })
    .await;
    assert!(out.0, "woken to wander now");
    assert_eq!(
        out.1,
        (crate::game::map::creature::MOVEMENT_RANDOM, 5.0),
        "wandering five yards"
    );
    assert_eq!((out.2, out.3), (1, 0), "the door set open");
}

/// Thistle Tea's "duplicate map-event starts stop the losing script before
/// its tail": a map event under way can't be started again, and a script
/// that tries it with the abort flag stops there.
#[tokio::test]
async fn a_second_start_of_a_map_event_stops_its_script() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let (a, b) = on(&map, move |m| {
        let (a, b) = (spawn(m, ga, BRUTE, 20, 3.0), spawn(m, ga, BRUTE, 20, 4.0));
        scripts::start(m, ScriptTable::Generic, 7, a, Guid::zero());
        scripts::start(m, ScriptTable::Generic, 7, b, Guid::zero());
        (a, b)
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let states = on(&map, move |m| (stand_state(m, a), stand_state(m, b))).await;
    assert_eq!(
        states,
        (2, 0),
        "the first runs on, the second stops at the start"
    );
}

/// Thistle Tea's "starts once on the explicit source owner, leader, and
/// fellow group member" and "includes raid members across subgroups while
/// delivery retains the source world": a group's script (90) starts once on
/// the source and each other member of its creature group; a player's, on
/// each of their raid here, whatever their subgroup, not on one elsewhere.
#[tokio::test]
async fn a_groups_script_starts_once_on_each_member_here() {
    let (realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let out = on(&map, move |m| {
        let (leader, source, member, stranger) = (
            spawn(m, ga, BRUTE, 20, 3.0),
            spawn(m, ga, BRUTE, 20, 4.0),
            spawn(m, ga, BRUTE, 20, 5.0),
            spawn(m, ga, BRUTE, 20, 6.0),
        );
        for g in [source, member] {
            let at = m.position_of(leader).unwrap();
            let _ = at;
            m.script_groups.insert(g, (leader, 0.0, 0.0, 0));
        }
        scripts::execute(m, &step(90, [8, 0, 0, 0]), source, Guid::zero());
        [leader, source, member, stranger].map(|g| stand_state(m, g))
    })
    .await;
    assert_eq!(out, [8, 8, 8, 0], "the group, once each; not a stranger");
    // A raid: Bob here in another subgroup, Cid on Kalimdor.
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let mut c = Client::connect(&realm, "C");
    let gc = c.enter("Cid", Race::Human, Class::Warrior).await;
    for (who, name) in [(&mut b, "Bob"), (&mut c, "Cid")] {
        a.send(CMSG_GROUP_INVITE { name: name.into() }).await;
        who.wait_for("SMSG_GROUP_INVITE", |m| {
            matches!(
                m,
                wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_GROUP_INVITE(_)
            )
            .then_some(())
        })
        .await;
        who.send(CMSG_GROUP_ACCEPT {}).await;
        a.wait_for("SMSG_GROUP_LIST", |m| match m {
            wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_GROUP_LIST(l) => {
                (l.members.len() + 1 >= 2).then_some(())
            }
            _ => None,
        })
        .await;
    }
    a.send(CMSG_GROUP_RAID_CONVERT {}).await;
    a.send(CMSG_GROUP_CHANGE_SUB_GROUP {
        name: "Bob".into(),
        group_number: 7,
    })
    .await;
    c.say(".go xyz 1629 -4373 31 1").await;
    crate::game::tests_p3_entity_objects::until(&map, "Cid gone", move |m| {
        !m.players.contains_key(&gc)
    })
    .await;
    let states = on(&map, move |m| {
        scripts::execute(m, &step(90, [8, 0, 0, 0]), ga, Guid::zero());
        [ga, gb].map(|g| stand_state(m, g))
    })
    .await;
    assert_eq!(states, [8, 8], "Alice and Bob, of another subgroup");
    let kalimdor = realm.registry.online(gc).unwrap().map();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let cid = on(&kalimdor, move |m| stand_state(m, gc)).await;
    assert_eq!(cid, 0, "not Cid, elsewhere");
}

/// Thistle Tea's "a player's pending script stays in its original world":
/// a step that waits, for a player who has left the map by then, isn't run
/// on them where they went.
#[tokio::test]
async fn a_players_waiting_step_stays_behind_when_they_leave() {
    let (realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    on(&map, move |m| {
        scripts::start(m, ScriptTable::Generic, 9, ga, Guid::zero())
    })
    .await;
    a.say(".go xyz 1629 -4373 31 1").await;
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(
            m,
            wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_NEW_WORLD(_)
        )
        .then_some(())
    })
    .await;
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    a.sees(ga).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let kalimdor = realm.registry.online(ga).unwrap().map();
    let state = on(&kalimdor, move |m| stand_state(m, ga)).await;
    assert_eq!(state, 0, "she doesn't kneel in Kalimdor");
}
