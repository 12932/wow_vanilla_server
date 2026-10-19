//! Phase 3's world facts, ported from Thistle Tea's
//! `test/game/world/entity/ai_environment_test.exs` (docs/parity/matrix.csv):
//! what a condition reads of the world, a game event, a map event, the local
//! clock and a copy's instance data, each copy its own.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::scripts::*;
use crate::game::harness;
use crate::game::map::conditions::met;
use crate::game::map::instance_script::{self, Script};
use crate::game::map::{Map, MapKey};
use crate::game::tests_p3_combat::{BRUTE, data, setup_with, spawn};
use crate::game::tests_p3_entity::{on, step};

const EVENT_ON: u32 = 9_200;
const MAP_EVENT_ON: u32 = 9_201;
const NOW_IS_IN: u32 = 9_202;
const NOW_IS_OUT: u32 = 9_203;
const DATA_IS_THREE: u32 = 9_204;
const FAIRE: i32 = 4;

fn condition(d: &mut GameData, entry: u32, kind: i32, values: [i32; 4]) {
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

fn world() -> GameData {
    use chrono::Timelike;
    let mut d = data();
    condition(&mut d, EVENT_ON, 12, [FAIRE, 0, 0, 0]);
    condition(&mut d, MAP_EVENT_ON, 36, [648, 0, 0, 0]);
    // The local clock now, within a window about it, and outside one an
    // hour off (wrapping round midnight a window that can't hold it).
    let now = chrono::Local::now();
    let (h, mi) = (now.hour() as i32, now.minute() as i32);
    condition(&mut d, NOW_IS_IN, 53, [0, 0, 23, 59]);
    let later = (h + 1) % 24;
    condition(&mut d, NOW_IS_OUT, 53, [later, 0, later, 0]);
    let _ = mi;
    condition(&mut d, DATA_IS_THREE, 34, [99, 3, 0, 0]);
    d
}

/// Thistle Tea's "lends active game events to the conditions that name
/// one", "requests scripted map-event facts from their owner" and "reads
/// the local clock only for local-time conditions": a condition reads
/// whether a game event is on, whether a map event is under way, and the
/// hour and minute of the server's local clock.
#[tokio::test]
async fn a_condition_reads_the_events_and_the_local_clock() {
    let (realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let realm2 = realm.clone();
    let out = on(&map, move |m| {
        let off = met(m, EVENT_ON, None, None);
        realm2.social.events.lock().insert(FAIRE as u32);
        let on_ = met(m, EVENT_ON, None, None);
        let brute = spawn(m, ga, BRUTE, 20, 3.0);
        let before = met(m, MAP_EVENT_ON, Some(brute), None);
        crate::game::map::scripts::execute(m, &step(61, [648, 60, 0, 0]), brute, Guid::zero());
        let during = met(m, MAP_EVENT_ON, Some(brute), None);
        crate::game::map::scripts::execute(m, &step(62, [648, 0, 0, 0]), brute, Guid::zero());
        let after = met(m, MAP_EVENT_ON, Some(brute), None);
        (
            (off, on_),
            (before, during, after),
            (
                met(m, NOW_IS_IN, None, None),
                met(m, NOW_IS_OUT, None, None),
            ),
        )
    })
    .await;
    assert_eq!(out.0, (false, true), "the Faire, off then on");
    assert_eq!(
        out.1,
        (false, true, false),
        "the map event, before, under way, done"
    );
    assert_eq!(out.2, (true, false), "the local clock");
}

/// Thistle Tea's "batches EventAI and loaded-script instance fields once
/// for the exact copy", "uses a game object's full instance identity" and
/// "keeps identical map IDs isolated by instance ID": a condition on
/// instance data reads its own copy's, a creature's or an object's alike,
/// and two copies of one dungeon keep theirs apart.
#[tokio::test]
async fn each_copy_keeps_its_own_instance_data() {
    let mut d = world();
    d.gameobject_templates.insert(
        97_200,
        crate::game::tests_p3_entity_objects::object(97_200, 5, "Post", &[]),
    );
    let realm = harness::realm_with(d);
    let at = Vector3d {
        x: -16.0,
        y: -383.0,
        z: 61.0,
    };
    let copies = [
        realm.maps.get(
            &realm,
            MapKey {
                map: 36,
                instance: 1,
            },
        ),
        realm.maps.get(
            &realm,
            MapKey {
                map: 36,
                instance: 2,
            },
        ),
    ];
    let seen = move |m: &mut Map, data: u32| {
        m.instance.script = Some(Script::Deadmines);
        if data != 0 {
            instance_script::set_data(m, 99, data, 0);
        }
        let creature = {
            let t = m.realm.data.creature_template(BRUTE).cloned().unwrap();
            let counter = m.next_creature_counter;
            m.next_creature_counter += 1;
            let c = crate::game::map::creature::Creature::new(
                crate::game::guid::unit(BRUTE, counter | 0x80_0000),
                &t,
                20,
                at,
                0.0,
            );
            m.add_creature(c.with_base())
        };
        let object =
            crate::game::map::objects::summon(m, None, 97_200, at, 0.0, 0, 0, 0, None).unwrap();
        (
            met(m, DATA_IS_THREE, Some(creature), None),
            met(m, DATA_IS_THREE, Some(object), None),
        )
    };
    let first = on(&copies[0], move |m| seen(m, 3)).await;
    let second = on(&copies[1], move |m| seen(m, 0)).await;
    assert_eq!(
        first,
        (true, true),
        "the first copy's three, for a creature and an object"
    );
    assert_eq!(second, (false, false), "the second's own, untouched");
}

const LEVEL_ONE: u32 = 9_205;
const LEVEL_SIXTY: u32 = 9_206;
const WATCHER: u32 = 3_730;

fn watcher_world() -> GameData {
    let mut d = world();
    condition(&mut d, LEVEL_ONE, 15, [1, 0, 0, 0]);
    condition(&mut d, LEVEL_SIXTY, 15, [60, 0, 0, 0]);
    let mut t = crate::game::tests_p3_combat::template(WATCHER, 7);
    t.name = "Watcher".into();
    d.creature_templates.insert(WATCHER, t);
    // In a fight, it yells "one" if its victim is level 1, "sixty" if 60.
    for (script, text, line) in [(11, 310, "one"), (12, 311, "sixty")] {
        d.scripts.ai_scripts.insert(
            script,
            vec![ScriptStep {
                id: script,
                command: 0,
                dataint: [text, 0, 0, 0],
                ..Default::default()
            }],
        );
        d.npc.broadcast_texts.insert(
            text as u32,
            crate::game::data::npc::BroadcastText {
                male: line.into(),
                chat_type: 1,
                ..Default::default()
            },
        );
    }
    let event = |id, condition, action| AiEvent {
        id,
        creature: WATCHER,
        condition,
        kind: EVENT_TIMER_IN_COMBAT,
        chance: 100,
        params: [0, 0, 60_000, 60_000],
        actions: [action, 0, 0],
        ..Default::default()
    };
    d.scripts.ai_events.insert(
        WATCHER,
        vec![event(1, LEVEL_ONE, 11), event(2, LEVEL_SIXTY, 12)],
    );
    d
}

/// Thistle Tea's "captures EventAI world facts against the current victim"
/// and "selected targets receive their own immutable condition facts": an
/// EventAI event's condition is judged on the creature's victim, and a
/// script step's on the target it selected.
#[tokio::test]
async fn an_event_is_judged_on_the_victim_and_a_step_on_its_target() {
    use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
    let (_realm, mut a, ga, map) = setup_with(watcher_world(), Class::Warrior).await;
    let watcher = on(&map, move |m| spawn(m, ga, WATCHER, 20, 3.0)).await;
    a.sees(watcher).await;
    a.clear();
    on(&map, move |m| {
        crate::game::map::ai::attacked(m, watcher, ga)
    })
    .await;
    let line = a
        .wait_for("the watcher's yell", |m| match m {
            ServerOpcodeMessage::SMSG_MESSAGECHAT(c) => Some(c.message.clone()),
            _ => None,
        })
        .await;
    assert_eq!(line, "one", "judged on Alice, its victim, level 1");
    assert!(
        a.gets(crate::game::tests_p3_entity::NONE_WITHIN, |m| match m {
            ServerOpcodeMessage::SMSG_MESSAGECHAT(c) if c.message == "sixty" => Some(()),
            _ => None,
        })
        .await
        .is_none(),
        "not on itself, level 20"
    );
    // A step aimed at the nearest brute, its condition on that brute's
    // level: the level-1 one is knelt, the level-60 one isn't.
    let out = on(&map, move |m| {
        let low = spawn(m, ga, BRUTE, 1, 5.0);
        let high = spawn(m, ga, BRUTE, 60, 9.0);
        for (target, cond) in [(low, LEVEL_ONE), (high, LEVEL_ONE)] {
            let mut kneel = step(28, [8, 1, 0, 0]);
            kneel.condition = cond;
            crate::game::map::scripts::execute(m, &kneel, watcher, target);
        }
        [low, high].map(|g| {
            m.unit_fields(g).map_or(u8::MAX, |f| {
                f.get_byte(crate::game::fields::UNIT_FIELD_BYTES_1, 0)
            })
        })
    })
    .await;
    assert_eq!(out, [8, 0], "each target judged on its own");
}

/// Thistle Tea's "observes the active melee victim after selection is
/// cleared" and "observes an auto shot target after melee attack stop
/// clears the selected target": a melee attack goes on when the player's
/// selection is cleared, and Auto Shot keeps its target when the melee
/// attack stops.
#[tokio::test]
async fn an_attack_keeps_its_victim_when_the_selection_goes() {
    let (_realm, a, ga, map) = setup_with(world(), Class::Warrior).await;
    // Alice outlasts the brute's blows while the test looks (its spawn
    // holds her replies for an update).
    let brute = on(&map, move |m| {
        let f = &mut m.players.get_mut(&ga).unwrap().fields;
        f.set_u32(crate::game::fields::UNIT_FIELD_MAXHEALTH, 1_000_000);
        f.set_u32(crate::game::fields::UNIT_FIELD_HEALTH, 1_000_000);
        spawn(m, ga, BRUTE, 20, 2.0)
    })
    .await;
    a.send(CMSG_SET_SELECTION { target: brute }).await;
    a.send(CMSG_ATTACKSWING { guid: brute }).await;
    crate::game::tests_p3_entity_objects::until(&map, "Alice attacks", move |m| {
        m.players[&ga].combat.attacking
    })
    .await;
    a.send(CMSG_SET_SELECTION {
        target: Guid::zero(),
    })
    .await;
    let out = {
        let mut a = a;
        a.send(CMSG_QUERY_TIME {}).await;
        a.wait_for("SMSG_QUERY_TIME_RESPONSE", |m| {
            matches!(
                m,
                wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_QUERY_TIME_RESPONSE(
                    _
                )
            )
            .then_some(())
        })
        .await;
        let melee = on(&map, move |m| {
            let c = &m.players[&ga].combat;
            (c.attacking, c.victim)
        })
        .await;
        // Auto Shot at the brute; the melee attack stops.
        on(&map, move |m| {
            let now = m.now;
            m.players.get_mut(&ga).unwrap().combat.auto_repeat =
                Some(crate::game::map::combat::AutoRepeat {
                    spell: 75,
                    target: brute,
                    next_at: now + std::time::Duration::from_secs(60),
                });
        })
        .await;
        a.send(CMSG_ATTACKSTOP {}).await;
        a.send(CMSG_QUERY_TIME {}).await;
        a.wait_for("SMSG_QUERY_TIME_RESPONSE", |m| {
            matches!(
                m,
                wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_QUERY_TIME_RESPONSE(
                    _
                )
            )
            .then_some(())
        })
        .await;
        let shot = on(&map, move |m| {
            let c = &m.players[&ga].combat;
            (c.attacking, c.auto_repeat.map(|r| r.target))
        })
        .await;
        (melee, shot)
    };
    assert_eq!(out.0, (true, brute), "still swinging at the brute");
    assert_eq!(
        out.1,
        (false, Some(brute)),
        "the melee stopped, the shot still at it"
    );
}
