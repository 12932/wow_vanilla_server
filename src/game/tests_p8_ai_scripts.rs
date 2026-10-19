//! Round 8's creature AI tests of scripts, ported from Thistle Tea's
//! `test/game/core/ai/script_test.exs`, `script_step_test.exs` and
//! `script_run_test.exs` (docs/parity/matrix.csv): an emote's
//! alternatives, a teleport's pose and its bad ones, a door's time to go
//! back, an object summoned for players, a map event's target, a scripted
//! flight's time, a terminate's reach and a run whose source left.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::scripts::{InlineText, ScriptStep, ScriptTable, StepExt};
use crate::game::data::{GameData, GameObjectSpawn, GameObjectTemplate};
use crate::game::fields::*;
use crate::game::map::combat::{self, dist};
use crate::game::map::gameobject::GameObject;
use crate::game::map::{Map, ai, scripts};
use crate::game::tests_p3_combat::{BRUTE, START, data, setup_with, spawn};
use crate::game::tests_p3_entity::step;

const DOOR: u32 = 95_820;
const CHEST: u32 = 95_821;

fn world() -> GameData {
    let mut d = data();
    for (entry, kind) in [(DOOR, 0), (CHEST, 3)] {
        d.gameobject_templates.insert(
            entry,
            GameObjectTemplate {
                entry,
                kind,
                name: format!("Thing {entry}"),
                ..Default::default()
            },
        );
    }
    // Emotes a unit stays in, so their field shows the one chosen.
    d.emote_states.extend([1, 2, 11]);
    d
}

fn at(east: f32, north: f32, up: f32) -> Vector3d {
    Vector3d {
        x: START.0 + east,
        y: START.1 + north,
        z: START.2 + up,
    }
}

/// The map's creatures take `n` turns of 50 ms.
fn turns(m: &mut Map, n: u32) {
    for _ in 0..n {
        m.now += Duration::from_millis(50);
        ai::update_creatures(m);
        combat::update_creatures(m);
    }
}

/// Thistle Tea's `emote_ids/1` "collects consecutive alternatives after
/// the first emote": an emote step plays its first emote or one of those
/// after it up to the first zero (11 or 1 of 11, 1, 0, 2; never 2).
#[tokio::test]
async fn a_scripts_emote_is_its_first_or_an_alternative_before_a_zero() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let seen = map
        .query(move |m| {
            let k = spawn(m, ga, BRUTE, 20, 5.0);
            let mut seen = std::collections::BTreeSet::new();
            for _ in 0..100 {
                scripts::execute(m, &step(1, [11, 1, 0, 2]), k, k);
                seen.insert(m.unit_fields(k).unwrap().get_u32(UNIT_NPC_EMOTESTATE));
            }
            seen
        })
        .await
        .unwrap();
    assert_eq!(seen.into_iter().collect::<Vec<_>>(), [1, 11]);
}

/// A command 6 step to `at`, declaring map 0.
fn teleport(x: f32, y: f32, z: f32, o: f32) -> ScriptStep {
    ScriptStep {
        x,
        y,
        z,
        o,
        ..step(6, [0, 9, 0, 0])
    }
}

/// Thistle Tea's `teleport_to/1` "preserves the declared map, options,
/// pose, and routing fields" and "fails closed for malformed payloads": a
/// teleport handed by its final swap to the creature of a spawn puts it at
/// the row's place and facing; one to a place on no map (not a number, or
/// past a map's edge) moves no one, creature or player, and fails.
#[tokio::test]
async fn a_teleport_puts_its_unit_at_the_rows_pose_and_one_to_nowhere_moves_no_one() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let out = map
        .query(move |m| {
            let k = spawn(m, ga, BRUTE, 20, 5.0);
            let other = spawn(m, ga, BRUTE, 20, 6.0);
            m.by_spawn.insert(4_321, other);
            let p = at(10.0, -2.5, 3.75);
            let routed = ScriptStep {
                target_type: 11,
                target_param1: 4_321,
                data_flags: crate::game::data::scripts::SF_SWAP_FINAL_TARGETS,
                ..teleport(p.x, p.y, p.z, 1.25)
            };
            scripts::execute(m, &routed, k, ga);
            let placed = m
                .creature(other)
                .map(|c| (dist(c.position, p) < 0.01, c.orientation));
            let before = (m.position_of(k), m.position_of(ga));
            let bad = [
                teleport(p.x, p.y, f32::NAN, 0.0),
                teleport(f32::INFINITY, p.y, p.z, 0.0),
                teleport(40_000.0, p.y, p.z, 0.0),
                teleport(p.x, p.y, p.z, f32::NAN),
            ];
            let refused: Vec<(bool, bool)> = bad
                .iter()
                .map(|s| {
                    let mut st = s.clone();
                    st.data_flags = crate::game::data::scripts::SF_ABORT_ON_FAILURE;
                    (
                        scripts::execute(m, &st, k, k),
                        scripts::execute(m, &st, ga, ga),
                    )
                })
                .collect();
            let after = (m.position_of(k), m.position_of(ga));
            (placed, refused, before == after)
        })
        .await
        .unwrap();
    let (placed, refused, unmoved) = out;
    assert_eq!(placed, Some((true, 1.25)), "at the row's pose");
    assert_eq!(refused, [(false, false); 4], "nowhere fails");
    assert!(unmoved, "and moves no one");
}

/// A door of spawn 77 on the map, closed.
fn door(m: &mut Map) -> Guid {
    let t = m.realm.data.gameobject_templates[&DOOR].clone();
    let mut go = GameObject::from_spawn(
        &t,
        &GameObjectSpawn {
            guid: 77,
            entry: DOOR,
            x: START.0,
            y: START.1,
            z: START.2,
            state: 1,
            ..Default::default()
        },
    );
    go.spawn_guid = Some(77);
    m.add_gameobject(go)
}

fn state(m: &Map, o: Guid) -> u32 {
    m.gameobject(o)
        .map_or(u32::MAX, |o| o.fields.get_u32(GAMEOBJECT_STATE))
}

/// Thistle Tea's "door commands enqueue spawn and owner operations": a
/// door opened by a script for datalong2 seconds closes again after them,
/// fifteen at the least (mangos-classic's, where Thistle Tea takes three);
/// one opened with none stays open; opening it open again changes nothing,
/// its time to close kept; a close is undone as an open is.
#[tokio::test]
async fn a_script_door_goes_back_after_its_time_fifteen_seconds_at_least_or_stays_without() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let out = map
        .query(move |m| {
            let k = spawn(m, ga, BRUTE, 20, 5.0);
            let d = door(m);
            let after = |m: &mut Map, secs: u64| {
                m.now += Duration::from_secs(secs);
                scripts::update_objects(m);
                state(m, d)
            };
            // One second asked: fifteen.
            scripts::execute(m, &step(11, [77, 1, 0, 0]), k, k);
            let opened = state(m, d);
            let at_14 = after(m, 14);
            // Opened again while open: nothing new.
            scripts::execute(m, &step(11, [77, 100, 0, 0]), k, k);
            let at_15 = after(m, 1);
            let at_120 = after(m, 105);
            // None asked: it stays.
            scripts::execute(m, &step(11, [77, 0, 0, 0]), k, k);
            let stays = after(m, 600);
            // Closed for twenty: open again after them.
            scripts::execute(m, &step(12, [77, 20, 0, 0]), k, k);
            let closed = (state(m, d), after(m, 19), after(m, 1));
            (opened, at_14, at_15, at_120, stays, closed)
        })
        .await
        .unwrap();
    let (opened, at_14, at_15, at_120, stays, closed) = out;
    assert_eq!((opened, at_14), (0, 0), "open, and still at fourteen");
    assert_eq!((at_15, at_120), (1, 1), "closed at fifteen, for good");
    assert_eq!(stays, 0, "open without a time");
    assert_eq!(closed, (1, 1, 0), "a close undone after its twenty");
}

/// Thistle Tea's "summon_object can leave the object unattached for
/// players to open": an object a script summons with datalong3 1 is no
/// one's (created by none), and stays when its summoner goes, for its
/// time.
#[tokio::test]
async fn a_scripts_unattached_object_is_no_ones_and_outlasts_its_summoner() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let out = map
        .query(move |m| {
            let k = spawn(m, ga, BRUTE, 20, 5.0);
            let s = ScriptStep {
                x: START.0 + 1.0,
                y: START.1 + 2.0,
                z: START.2 + 3.0,
                ..step(76, [CHEST, 40, 1, 0])
            };
            scripts::execute(m, &s, k, k);
            let chest = m
                .gameobjects
                .iter()
                .map(|(_, o)| o)
                .find(|o| o.entry == CHEST)
                .map(|o| o.guid)
                .unwrap();
            let owner = m
                .gameobject(chest)
                .unwrap()
                .fields
                .get_guid(OBJECT_FIELD_CREATED_BY);
            scripts::despawn(m, k);
            m.now += Duration::from_secs(39);
            scripts::update_objects(m);
            let stays = m.gameobject(chest).is_some();
            m.now += Duration::from_secs(1);
            scripts::update_objects(m);
            (owner, stays, m.gameobject(chest).is_some())
        })
        .await
        .unwrap();
    assert_eq!(out, (Guid::zero(), true, false));
}

/// Thistle Tea's "map event targets resolve from immutable context" and
/// "summons attack a map event target resolved from immutable context": a
/// talk step whose target is a map event's target (type 23, the event in
/// target_param1) speaks to her, her name in its line; a summon whose
/// attack target is that type goes for her.
#[tokio::test]
async fn a_map_events_target_is_whom_a_step_talks_to_and_its_summon_attacks() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let k = map
        .query(move |m| {
            let k = spawn(m, ga, BRUTE, 20, 5.0);
            scripts::execute(m, &step(61, [5_944, 600, 0, 0]), k, ga);
            k
        })
        .await
        .unwrap();
    a.sees(k).await;
    a.clear();
    let attacker = map
        .query(move |m| {
            let talk = ScriptStep {
                target_type: 23,
                target_param1: 5_944,
                ext: Some(Box::new(StepExt {
                    texts: vec![InlineText {
                        text: "Found you, $N".into(),
                        chat_type: 0,
                        language: 0,
                        emote: 0,
                    }],
                    ..Default::default()
                })),
                ..step(0, [0; 4])
            };
            scripts::execute(m, &talk, k, Guid::zero());
            let summon = ScriptStep {
                x: START.0 + 2.0,
                y: START.1,
                z: START.2,
                dataint: [0, 0, 23, 0],
                target_param1: 5_944,
                ..step(10, [BRUTE, 60_000, 0, 0])
            };
            scripts::execute(m, &summon, k, Guid::zero());
            m.creatures
                .iter()
                .map(|(_, c)| c)
                .filter(|c| c.guid != k && c.entry == BRUTE)
                .map(|c| c.combat.victim)
                .collect::<Vec<_>>()
        })
        .await
        .unwrap();
    let said = a
        .wait_for("the line", |m| match m {
            ServerOpcodeMessage::SMSG_MESSAGECHAT(c) => Some(c.message.clone()),
            _ => None,
        })
        .await;
    assert_eq!(said, "Found you, Alice");
    assert_eq!(attacker, [ga], "the summon goes for her");
}

/// Thistle Tea's "movement fleeing runs from the target, or from the
/// victim, for its duration": movement 10 runs from the step's target for
/// datalong3 milliseconds, or from its victim (datalong2 1) for seven
/// seconds, as a fear runs, one way then another, whatever its fight; then
/// it stops running.
#[tokio::test]
async fn a_scripted_flight_runs_from_its_target_or_victim_for_its_time() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let k = map
        .query(move |m| spawn(m, ga, BRUTE, 20, 5.0))
        .await
        .unwrap();
    // Seen, so it moves.
    a.sees(k).await;
    let out = map
        .query(move |m| {
            let flee = |m: &Map| {
                m.creature(k)
                    .and_then(|c| c.engagement.flee)
                    .map(|f| (f.from, f.until - m.now))
            };
            // From her, out of any fight, for ten seconds.
            scripts::execute(m, &step(20, [10, 0, 10_000, 0]), k, ga);
            let from_her = flee(m);
            let fleeing = m.unit_fields(k).unwrap().get_u32(UNIT_FIELD_FLAGS)
                & crate::game::map::auras::UNIT_FLAG_FLEEING
                != 0;
            let mut runs = std::collections::BTreeSet::new();
            for _ in 0..190 {
                turns(m, 1);
                if let crate::game::map::creature::Motion::Moving(w) =
                    &m.creature(k).unwrap().motion
                {
                    runs.insert(w.spline_id);
                }
            }
            let before_ten = flee(m).is_some();
            turns(m, 20);
            let after_ten = flee(m);
            // From its victim, for the default seven.
            let other = spawn(m, ga, BRUTE, 20, 6.0);
            m.creature_mut(k).unwrap().combat.victim = other;
            scripts::execute(m, &step(20, [10, 1, 0, 0]), k, ga);
            let from_victim = flee(m);
            (
                from_her,
                fleeing,
                runs.len(),
                before_ten,
                after_ten,
                from_victim,
                other,
            )
        })
        .await
        .unwrap();
    let (from_her, fleeing, runs, before_ten, after_ten, from_victim, other) = out;
    assert_eq!(from_her, Some((ga, Duration::from_secs(10))), "from her");
    assert!(fleeing, "it shows it flees");
    assert!(runs > 1, "one way, then another: {runs}");
    assert!(before_ten && after_ten.is_none(), "for ten seconds");
    assert_eq!(from_victim, Some((other, Duration::from_secs(7))));
}

/// Thistle Tea's "termination cancels matching invocations but preserves
/// other scripts and targets" and "world changes discard the original
/// tail": a run a terminate ends ends every run of its script from its
/// source on its target, not another script's or one on someone else; a
/// run whose source has left the map runs no more of its steps here.
#[tokio::test]
async fn a_terminate_ends_its_scripts_runs_on_its_target_and_a_departed_sources_run_stops() {
    let mut d = world();
    // 9_581: after a second, stop unless a creature of entry 1 is near
    // (none is), then sit. 9_582: sit after a second.
    let terminate = ScriptStep {
        command: 31,
        delay_ms: 1_000,
        datalong: [1, 0, 0, 0],
        ..Default::default()
    };
    let sit = |delay_ms| ScriptStep {
        delay_ms,
        ..step(28, [1, 0, 0, 0])
    };
    d.scripts.generic.insert(9_581, vec![terminate, sit(2_000)]);
    d.scripts.generic.insert(9_582, vec![sit(1_000)]);
    let (_realm, _a, ga, map) = setup_with(d, Class::Warrior).await;
    let out = map
        .query(move |m| {
            let k = spawn(m, ga, BRUTE, 20, 5.0);
            let other = spawn(m, ga, BRUTE, 20, 6.0);
            let runs = |m: &Map| {
                m.scripts
                    .runs
                    .iter()
                    .filter(|r| r.next != usize::MAX)
                    .map(|r| (r.id, r.target))
                    .collect::<Vec<_>>()
            };
            // Two of 9_581 on her, the second half a second later, with
            // one on the other and one of 9_582: when the first's
            // terminate comes, the others' haven't.
            scripts::start(m, ScriptTable::Generic, 9_581, k, ga);
            m.now += Duration::from_millis(500);
            for (id, target) in [(9_581, ga), (9_581, other), (9_582, ga)] {
                scripts::start(m, ScriptTable::Generic, id, k, target);
            }
            let started = runs(m).len();
            m.now += Duration::from_millis(500);
            scripts::update(m);
            let left = runs(m);
            (started, left, k, other)
        })
        .await
        .unwrap();
    let (started, left, _k, other) = out;
    assert_eq!(started, 4);
    // The terminate (whose entry isn't the source's) ends its run, and the
    // other one of 9_581 on her; that on the other and 9_582 go on.
    assert_eq!(left, [(9_581, other), (9_582, ga)]);
    let gone = map
        .query(move |m| {
            let k = spawn(m, ga, BRUTE, 20, 7.0);
            scripts::start(m, ScriptTable::Generic, 9_582, k, ga);
            // It leaves the map (as a creature goes to another).
            let stand = m
                .creature(k)
                .unwrap()
                .fields
                .get_byte(UNIT_FIELD_BYTES_1, 0);
            m.remove_creature(k);
            m.now += Duration::from_secs(2);
            scripts::update(m);
            let running = m
                .scripts
                .runs
                .iter()
                .any(|r| r.source == k && r.next != usize::MAX);
            (stand, running)
        })
        .await
        .unwrap();
    assert_eq!(gone, (0, false), "its run is dropped");
}

/// Thistle Tea's `ScriptStep.build/1` "decodes commands, delays, and
/// target flags": a script row as read is its command and data, its delay
/// in milliseconds from its seconds, its target type, its flags (0x0C:
/// self-targeted and aborting, neither swap) and its place and facing;
/// a script's rows run in order of delay, then priority.
#[test]
fn a_script_row_reads_as_its_command_delay_flags_and_place() {
    use crate::game::data::scripts::{
        SF_ABORT_ON_FAILURE, SF_SWAP_FINAL_TARGETS, SF_SWAP_INITIAL_TARGETS, SF_TARGET_SELF,
    };
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE generic_scripts (id, delay, priority, command, datalong, datalong2, \
         datalong3, datalong4, dataint, dataint2, dataint3, dataint4, target_type, \
         target_param1, target_param2, data_flags, x, y, z, o, condition_id);
         INSERT INTO generic_scripts VALUES \
           (100, 3, 1, 15, 12544, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 12, 1.0, 2.0, 3.0, 4.0, 0);
         INSERT INTO generic_scripts VALUES \
           (100, 3, 0, 1, 11, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0);
         INSERT INTO generic_scripts VALUES \
           (100, 0, 5, 0, 0, 0, 0, 0, 7, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0);",
    )
    .unwrap();
    let mut d = GameData::empty();
    crate::game::data::scripts_load::load(&db, &mut d);
    let steps = &d.scripts.generic[&100];
    assert_eq!(
        steps.iter().map(|s| s.command).collect::<Vec<_>>(),
        [0, 1, 15],
        "by delay, then priority"
    );
    let s = &steps[2];
    assert_eq!(
        (s.delay_ms, s.datalong, s.target_type),
        (3_000, [12_544, 2, 0, 0], 0)
    );
    let f = s.data_flags;
    assert_eq!(
        (
            f & SF_TARGET_SELF != 0,
            f & SF_ABORT_ON_FAILURE != 0,
            f & SF_SWAP_INITIAL_TARGETS != 0,
            f & SF_SWAP_FINAL_TARGETS != 0,
        ),
        (true, true, false, false)
    );
    assert_eq!((s.x, s.y, s.z, s.o), (1.0, 2.0, 3.0, 4.0));
}
