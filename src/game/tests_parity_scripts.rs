//! Script commands and encounters as Thistle Tea runs them: the open
//! commands of `docs/AI5_GAPS.md`, summons' despawn types, the unkillable
//! flag, creature spell lists' last items, and an object opened by the item
//! that is its key (Molten Core's runes).

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::crafts::{LOCK_KEY_ITEM, LockKey};
use crate::game::data::items::ItemSpell;
use crate::game::data::scripts::*;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::MapHandle;

/// Northshire, where humans start.
const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

fn at(dx: f32, dy: f32) -> Vector3d {
    Vector3d {
        x: START.0 + dx,
        y: START.1 + dy,
        z: START.2,
    }
}

async fn player(realm: &Arc<Realm>, name: &str) -> (Client, MapHandle) {
    let mut c = Client::connect(realm, name);
    let me = c.enter(name, Race::Human, Class::Warrior).await;
    c.sees(me).await;
    let map = realm.registry.online(me).unwrap().map();
    (c, map)
}

const RUNE: u32 = 600;
const RUNE_LOCK: u32 = 900;
const QUINTESSENCE: u32 = 700;
const DOUSE: u32 = 6100;

/// A rune (a button) whose lock takes the Quintessence, and the
/// Quintessence, spent by its spell, that opens it.
fn rune_data() -> GameData {
    let mut d = GameData::empty();
    let mut rune = GameObjectTemplate {
        entry: RUNE,
        kind: 1,
        display_id: 2670,
        name: "Rune".into(),
        size: 1.0,
        ..Default::default()
    };
    rune.data[1] = RUNE_LOCK;
    d.gameobject_templates.insert(RUNE, rune);
    d.gameobject_spawns.insert(
        0,
        vec![GameObjectSpawn {
            guid: 1,
            entry: RUNE,
            map: 0,
            x: START.0 + 2.0,
            y: START.1,
            z: START.2,
            rotation: [0.0, 0.0, 0.0, 1.0],
            state: 1,
            ..Default::default()
        }],
    );
    d.crafts.locks.insert(
        RUNE_LOCK,
        vec![LockKey {
            kind: LOCK_KEY_ITEM,
            index: QUINTESSENCE,
            skill: 0,
        }],
    );
    let mut quintessence = ItemTemplate {
        entry: QUINTESSENCE,
        name: "Quintessence".into(),
        stackable: 1,
        ..Default::default()
    };
    quintessence.spells[0] = ItemSpell {
        id: DOUSE,
        charges: -1,
        cooldown: -1,
        ..Default::default()
    };
    d.items.insert(QUINTESSENCE, quintessence);
    let mut douse = SpellTemplate {
        id: DOUSE,
        name: "Douse".into(),
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    douse.effects[0] = SpellEffect {
        effect: 59,
        target_a: 23,
        ..Default::default()
    };
    d.spells.spells.insert(DOUSE, douse);
    d
}

/// The item that is an object's key opens it, though its use spends it
/// before its spell lands (Aqual Quintessence on a rune of Molten Core).
#[tokio::test]
async fn an_object_opens_to_the_item_that_is_its_key() {
    let realm = harness::realm_with(rune_data());
    let (mut a, map) = player(&realm, "Alice").await;
    let me = a.guid;
    let rune = guid::game_object(RUNE, 1);
    a.sees(rune).await;
    let state = map
        .query(move |m| {
            let _ = crate::game::map::inventory::add_item(m, me, QUINTESSENCE, 1, false);
            let item = m.players[&me]
                .items
                .iter()
                .find(|i| i.entry == QUINTESSENCE)
                .map(|i| i.guid);
            crate::game::map::spells::cast(
                m,
                me,
                DOUSE,
                crate::game::map::spells::Targets {
                    object: Some(rune),
                    ..Default::default()
                },
                item,
            );
            let spent = !m.players[&me].items.iter().any(|i| i.entry == QUINTESSENCE);
            (
                spent,
                m.gameobject(rune)
                    .map(|o| o.fields.get_u32(GAMEOBJECT_STATE)),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        state,
        (true, Some(0)),
        "the Quintessence spent, the rune open"
    );
}

const HOST: u32 = 3100;
const BUDDY: u32 = 3101;
const THIRD: u32 = 3102;
const DOOR: u32 = 601;
const HIDDEN: u32 = 602;
const LEVER: u32 = 603;
/// Spawns: the host and its buddies; the door, a lever and an object held
/// back.
const HOST_SPAWN: u32 = 10;
const BUDDY_SPAWN: u32 = 11;
const THIRD_SPAWN: u32 = 12;
const DOOR_SPAWN: u32 = 20;
const HIDDEN_SPAWN: u32 = 21;
const LEVER_SPAWN: u32 = 22;

fn creature(entry: u32, name: &str, faction: u32) -> CreatureTemplate {
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
        health_max: 500,
        speed_walk: 1.0,
        speed_run: 1.0,
        ..Default::default()
    }
}

fn creature_spawn(guid: u32, entry: u32, dx: f32) -> CreatureSpawn {
    CreatureSpawn {
        guid,
        entries: vec![entry],
        map: 0,
        x: START.0 + dx,
        y: START.1,
        z: START.2,
        respawn_min: 60,
        respawn_max: 60,
        ..Default::default()
    }
}

fn object(entry: u32, kind: u8, name: &str) -> GameObjectTemplate {
    GameObjectTemplate {
        entry,
        kind,
        display_id: 411,
        name: name.into(),
        size: 1.0,
        ..Default::default()
    }
}

fn object_spawn(guid: u32, entry: u32, dy: f32) -> GameObjectSpawn {
    GameObjectSpawn {
        guid,
        entry,
        map: 0,
        x: START.0,
        y: START.1 + dy,
        z: START.2,
        rotation: [0.0, 0.0, 0.0, 1.0],
        state: 1,
        respawn_min: 30,
        respawn_max: 30,
        ..Default::default()
    }
}

/// Three friendly creatures in a row beside the start, a door and a lever
/// on one side, and an object that only scripts bring out.
fn world() -> GameData {
    let mut d = GameData::empty();
    for (entry, name) in [(HOST, "Host"), (BUDDY, "Buddy"), (THIRD, "Third")] {
        d.creature_templates
            .insert(entry, creature(entry, name, 35));
    }
    d.creature_spawns.insert(
        0,
        vec![
            creature_spawn(HOST_SPAWN, HOST, 3.0),
            creature_spawn(BUDDY_SPAWN, BUDDY, 6.0),
            creature_spawn(THIRD_SPAWN, THIRD, 9.0),
        ],
    );
    d.gameobject_templates.insert(DOOR, object(DOOR, 0, "Door"));
    d.gameobject_templates
        .insert(HIDDEN, object(HIDDEN, 3, "Hidden Chest"));
    d.gameobject_templates
        .insert(LEVER, object(LEVER, 1, "Lever"));
    let mut hidden = object_spawn(HIDDEN_SPAWN, HIDDEN, 4.0);
    hidden.despawned = true;
    d.gameobject_spawns.insert(
        0,
        vec![
            object_spawn(DOOR_SPAWN, DOOR, 2.0),
            hidden,
            object_spawn(LEVER_SPAWN, LEVER, -2.0),
        ],
    );
    d
}

fn host() -> Guid {
    guid::unit(HOST, HOST_SPAWN)
}

fn buddy() -> Guid {
    guid::unit(BUDDY, BUDDY_SPAWN)
}

fn third() -> Guid {
    guid::unit(THIRD, THIRD_SPAWN)
}

fn step(command: u32, datalong: [u32; 4]) -> ScriptStep {
    ScriptStep {
        command,
        datalong,
        ..Default::default()
    }
}

fn stand_state(m: &crate::game::map::Map, g: Guid) -> u8 {
    m.unit_fields(g)
        .map_or(u8::MAX, |f| f.get_byte(UNIT_FIELD_BYTES_1, 0))
}

/// The self flag makes the target the one a final swap hands the step to;
/// and an initial swap with no one to hand it to still finds the unit its
/// target type names, which runs it on no one (S3, S7).
#[tokio::test]
async fn a_step_runs_from_the_unit_its_swaps_and_self_flag_name() {
    let realm = harness::realm_with(world());
    let (mut a, map) = player(&realm, "Alice").await;
    a.sees(buddy()).await;
    let (host, buddy) = (host(), buddy());
    let states = map
        .query(move |m| {
            // Sit (28), at the nearest Buddy: swapped, the self flag
            // makes both Buddy.
            let sit = ScriptStep {
                target_type: 10,
                target_param1: BUDDY,
                data_flags: SF_SWAP_FINAL_TARGETS | SF_TARGET_SELF,
                ..step(28, [1, 0, 0, 0])
            };
            crate::game::map::scripts::execute(m, &sit, host, Guid::zero());
            let first = (stand_state(m, host), stand_state(m, buddy));
            // Kneel, the step handed on from no one: Buddy kneels.
            let kneel = ScriptStep {
                target_type: 10,
                target_param1: BUDDY,
                data_flags: SF_SWAP_INITIAL_TARGETS | SF_SWAP_FINAL_TARGETS,
                ..step(28, [8, 0, 0, 0])
            };
            let ran = crate::game::map::scripts::execute(
                m,
                &ScriptStep {
                    data_flags: kneel.data_flags | SF_ABORT_ON_FAILURE,
                    ..kneel
                },
                host,
                Guid::zero(),
            );
            (first, ran, stand_state(m, buddy))
        })
        .await
        .unwrap();
    assert_eq!(states.0, (0, 1), "Buddy sits, not the host");
    assert!(states.1, "the step ran");
    assert_eq!(states.2, 8, "Buddy kneels");
}

/// An object found by its spawn runs `activate_object` on the step's
/// source: a door opens, and its scripts run on that source, at the door
/// (S4).
#[tokio::test]
async fn activate_object_works_the_object_it_finds_for_the_source() {
    let mut d = world();
    // The door's own script: whoever used it sits.
    d.scripts
        .gameobject
        .insert(DOOR_SPAWN, vec![step(28, [1, 0, 0, 0])]);
    let realm = harness::realm_with(d);
    let (mut a, map) = player(&realm, "Alice").await;
    a.sees(host()).await;
    let host = host();
    let (state, sat) = map
        .query(move |m| {
            let door = crate::game::map::scripts::object_of_spawn(m, DOOR_SPAWN).unwrap();
            let activate = ScriptStep {
                target_type: 14,
                target_param1: DOOR_SPAWN,
                ..step(13, [0; 4])
            };
            crate::game::map::scripts::execute(m, &activate, host, Guid::zero());
            (
                m.gameobject(door)
                    .map(|o| o.fields.get_u32(GAMEOBJECT_STATE)),
                stand_state(m, host),
            )
        })
        .await
        .unwrap();
    assert_eq!(state, Some(0), "the door opens");
    assert_eq!(sat, 1, "its script ran on the host");
}

/// `turn_to` shows the turn: a facing move to the target, or to the angle
/// (S6).
#[tokio::test]
async fn turn_to_sends_a_facing_move() {
    let realm = harness::realm_with(world());
    let (mut a, map) = player(&realm, "Alice").await;
    let me = a.guid;
    let host = host();
    a.sees(host).await;
    a.clear();
    map.call(move |m| {
        crate::game::map::scripts::execute(m, &step(35, [0; 4]), host, me);
    });
    let toward = a
        .wait_for("a facing move", |m| match m {
            ServerOpcodeMessage::SMSG_MONSTER_MOVE(mv) if mv.guid == host => Some(mv.clone()),
            _ => None,
        })
        .await;
    assert!(
        matches!(toward.move_type, SMSG_MONSTER_MOVE_MonsterMoveType::FacingTarget { target } if target == me),
        "{:?}",
        toward.move_type
    );
    map.call(move |m| {
        let turn = ScriptStep {
            o: 1.5,
            ..step(35, [1, 0, 0, 0])
        };
        crate::game::map::scripts::execute(m, &turn, host, me);
    });
    let angle = a
        .wait_for("a facing move", |m| match m {
            ServerOpcodeMessage::SMSG_MONSTER_MOVE(mv) if mv.guid == host => Some(mv.clone()),
            _ => None,
        })
        .await;
    assert!(
        matches!(angle.move_type, SMSG_MONSTER_MOVE_MonsterMoveType::FacingAngle { angle } if (angle - 1.5).abs() < 0.01),
        "{:?}",
        angle.move_type
    );
}

/// An object plays its own animation, not an emote (S11).
#[tokio::test]
async fn an_object_plays_its_custom_animation() {
    let realm = harness::realm_with(world());
    let (mut a, map) = player(&realm, "Alice").await;
    let lever = guid::game_object(LEVER, LEVER_SPAWN);
    a.sees(lever).await;
    let host = host();
    map.call(move |m| {
        let anim = ScriptStep {
            target_type: 14,
            target_param1: LEVER_SPAWN,
            ..step(89, [1, 0, 0, 0])
        };
        crate::game::map::scripts::execute(m, &anim, host, Guid::zero());
    });
    let id = a
        .wait_for("SMSG_GAMEOBJECT_CUSTOM_ANIM", |m| match m {
            ServerOpcodeMessage::SMSG_GAMEOBJECT_CUSTOM_ANIM(c) if c.guid == lever => {
                Some(c.animation_id)
            }
            _ => None,
        })
        .await;
    assert_eq!(id, 1);
}

/// `respawn_game_object` brings out an object held back, for its time (at
/// least five seconds); `despawn_game_object` takes one in and brings it
/// back after datalong2 seconds (S2, S8).
#[tokio::test]
async fn scripts_bring_objects_out_and_take_them_in() {
    let realm = harness::realm_with(world());
    let (mut a, map) = player(&realm, "Alice").await;
    let hidden = guid::game_object(HIDDEN, HIDDEN_SPAWN);
    let door = guid::game_object(DOOR, DOOR_SPAWN);
    a.sees(door).await;
    let host = host();
    let out = map
        .query(move |m| {
            let before = m.gameobject(hidden).is_some();
            crate::game::map::scripts::execute(m, &step(9, [HIDDEN_SPAWN, 0, 0, 0]), host, host);
            let goes_in = m
                .object_resets
                .iter()
                .find(|(_, g, s)| *g == hidden && *s == u32::MAX)
                .map(|(t, _, _)| t.duration_since(m.now));
            // One already out stays as it is.
            crate::game::map::scripts::execute(m, &step(9, [DOOR_SPAWN, 10, 0, 0]), host, host);
            let door_kept = !m.object_resets.iter().any(|(_, g, _)| *g == door);
            // The door taken in, back in 20 seconds.
            crate::game::map::scripts::execute(m, &step(81, [DOOR_SPAWN, 20, 0, 0]), host, host);
            let back = m
                .objects
                .respawns
                .iter()
                .find(|(_, s)| *s == DOOR_SPAWN)
                .map(|(t, _)| t.duration_since(m.now));
            (
                before,
                m.gameobject(hidden).is_some(),
                goes_in,
                door_kept,
                m.gameobject(door).is_some(),
                back,
            )
        })
        .await
        .unwrap();
    assert!(!out.0 && out.1, "brought out");
    assert_eq!(out.2, Some(Duration::from_secs(5)), "for five seconds");
    assert!(out.3, "the door, out already, is left");
    assert!(!out.4, "the door taken in");
    assert_eq!(out.5, Some(Duration::from_secs(20)), "back in twenty");
    a.sees(hidden).await;
    a.forgets(door).await;
}

/// `respawn_creature` respawns its source: dead, or with datalong alive
/// too (S10).
#[tokio::test]
async fn respawn_creature_respawns_its_source() {
    let realm = harness::realm_with(world());
    let (mut a, map) = player(&realm, "Alice").await;
    a.sees(buddy()).await;
    let (host, buddy) = (host(), buddy());
    let health = |m: &crate::game::map::Map, g| {
        m.unit_fields(g)
            .map(|f| f.get_u32(UNIT_FIELD_HEALTH))
            .unwrap_or(0)
    };
    let out = map
        .query(move |m| {
            for g in [host, buddy] {
                if let Some(f) = m.unit_fields_mut(g) {
                    f.set_u32(UNIT_FIELD_HEALTH, 100);
                }
            }
            // At Buddy, but the host respawns: alive, only with datalong.
            crate::game::map::scripts::execute(m, &step(71, [0; 4]), host, buddy);
            let kept = health(m, host);
            crate::game::map::scripts::execute(m, &step(71, [1, 0, 0, 0]), host, buddy);
            (kept, health(m, host), health(m, buddy))
        })
        .await
        .unwrap();
    assert_eq!(out, (100, 500, 100));
}

/// An object's gossip option runs its script on the player, at the object
/// (S12).
#[tokio::test]
async fn an_objects_gossip_option_runs_on_the_player() {
    let mut d = world();
    d.npc.gossip_menus.insert(
        500,
        crate::game::data::npc::GossipMenu {
            texts: vec![(1, 0)],
            options: vec![crate::game::data::npc::GossipOption {
                id: 0,
                text: "Sit down.".into(),
                kind: 1,
                script: 900,
                ..Default::default()
            }],
        },
    );
    // The script: its source sits.
    d.scripts.gossip.insert(900, vec![step(28, [1, 0, 0, 0])]);
    let realm = harness::realm_with(d);
    let (mut a, map) = player(&realm, "Alice").await;
    let me = a.guid;
    let lever = guid::game_object(LEVER, LEVER_SPAWN);
    a.sees(lever).await;
    map.query(move |m| {
        if let Some(p) = m.players.get_mut(&me) {
            p.gossip_menu = Some((lever, 500));
        }
    })
    .await;
    a.send(CMSG_GOSSIP_SELECT_OPTION {
        guid: lever,
        gossip_list_id: 0,
        unknown: None,
    })
    .await;
    let mut sat = false;
    for _ in 0..50 {
        sat = map.query(move |m| stand_state(m, me) == 1).await.unwrap();
        if sat {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(sat, "Alice sits");
}

/// Each of the three creatures sets its phase to 1 on map event 7's script
/// event with data 5.
fn hear_event_seven(d: &mut GameData) {
    for entry in [HOST, BUDDY, THIRD] {
        d.scripts.ai_events.insert(
            entry,
            vec![AiEvent {
                id: entry * 100,
                creature: entry,
                kind: EVENT_SCRIPT_EVENT,
                chance: 100,
                flags: EF_REPEATABLE,
                params: [7, 5, 0, 0],
                actions: [entry * 100, 0, 0],
                ..Default::default()
            }],
        );
        d.scripts
            .ai_scripts
            .insert(entry * 100, vec![step(44, [1, 0, 0, 0])]);
    }
}

/// `send_map_event` sends the event's own id with datalong2's data, to
/// its source and target (datalong3 0), its extra targets (1), or all
/// (S9); `edit_map_event` clears what it is given a zero for and keeps
/// what it is given less (S22).
#[tokio::test]
async fn map_events_send_their_id_and_edit_by_sign() {
    let mut d = world();
    hear_event_seven(&mut d);
    let realm = harness::realm_with(d);
    let (mut a, map) = player(&realm, "Alice").await;
    a.sees(third()).await;
    let (host, buddy, third) = (host(), buddy(), third());
    let phases = map
        .query(move |m| {
            let phase = |m: &crate::game::map::Map| {
                [host, buddy, third].map(|g| crate::game::map::event_ai::phase(m, g))
            };
            let start = ScriptStep {
                dataint: [11, 12, 13, 14],
                ..step(61, [7, 600, 0, 0])
            };
            crate::game::map::scripts::execute(m, &start, host, buddy);
            crate::game::map::scripts::execute(m, &step(63, [7, 0, 0, 0]), third, host);
            crate::game::map::scripts::execute(m, &step(66, [7, 5, 1, 0]), host, host);
            let extra = phase(m);
            crate::game::map::scripts::execute(m, &step(66, [7, 5, 0, 0]), host, host);
            let main = phase(m);
            let edit = ScriptStep {
                dataint: [0, -1, 0, -1],
                ..step(69, [7, 0, 0, 0])
            };
            crate::game::map::scripts::execute(m, &edit, host, host);
            let e = &m.map_events.active[&7];
            let edited = (
                e.success_condition,
                e.success_script,
                e.failure_condition,
                e.failure_script,
            );
            (extra, main, edited)
        })
        .await
        .unwrap();
    assert_eq!(phases.0, [0, 0, 1], "the extra target alone");
    assert_eq!(phases.1, [1, 1, 1], "then the source and target");
    assert_eq!(phases.2, (0, 12, 0, 14), "zeros clear, less keeps");
}

/// A summon's attack target is chosen with the step's target parameters:
/// players only finds none among creatures (D18).
#[tokio::test]
async fn a_summons_attack_target_keeps_the_steps_parameters() {
    let mut d = world();
    d.creature_templates
        .insert(4000, creature(4000, "Druid of the Nightmare", 14));
    let realm = harness::realm_with(d);
    let (mut a, map) = player(&realm, "Alice").await;
    a.sees(buddy()).await;
    let (host, buddy) = (host(), buddy());
    let victims = map
        .query(move |m| {
            // The host fights Buddy, a creature, and no player.
            if let Some(c) = m.creature_mut(host) {
                c.threat.push((buddy, 10.0));
            }
            let summon = |flags| ScriptStep {
                dataint: [0, 0, 4, 0],
                target_param1: flags,
                x: START.0,
                y: START.1 + 5.0,
                z: START.2,
                ..step(10, [4000, 0, 0, 0])
            };
            let mut out = Vec::new();
            for flags in [0x2, 0] {
                crate::game::map::scripts::execute(m, &summon(flags), host, Guid::zero());
                let g = m
                    .creatures
                    .iter()
                    .filter(|(_, c)| c.entry == 4000)
                    .map(|(_, c)| c.guid)
                    .max_by_key(|g| g.guid())
                    .unwrap();
                out.push(m.creature(g).map(|c| c.combat.victim).unwrap_or_default());
            }
            out
        })
        .await
        .unwrap();
    assert_eq!(victims, vec![Guid::zero(), buddy], "players only: none");
}

/// "Nearest player" with no radius looks 30 yards, as Thistle Tea's
/// buddies do (D20).
#[tokio::test]
async fn the_nearest_player_with_no_radius_is_within_thirty_yards() {
    let realm = harness::realm_with(world());
    let (mut a, map) = player(&realm, "Alice").await;
    let me = a.guid;
    let host = host();
    a.sees(host).await;
    let out = map
        .query(move |m| {
            let kneel = ScriptStep {
                target_type: 25,
                ..step(28, [8, 1, 0, 0])
            };
            let at = m.position_of(host).unwrap();
            let mut seen = Vec::new();
            for off in [50.0, 20.0] {
                if let Some(p) = m.players.get_mut(&me) {
                    p.movement.position = Vector3d {
                        x: at.x + off,
                        ..at
                    };
                    p.set_stand_state(0);
                }
                crate::game::map::scripts::execute(m, &kneel, host, Guid::zero());
                seen.push(stand_state(m, me));
            }
            seen
        })
        .await
        .unwrap();
    assert_eq!(out, vec![0, 8], "not at 50 yards; at 20");
}

/// Broadcast texts 100 to 106, said as each chat type 0 to 6, and 107 as a
/// zone emote.
fn with_texts(d: &mut GameData) {
    for (id, chat_type) in (100..108).zip(0..8) {
        d.npc.broadcast_texts.insert(
            id,
            crate::game::data::npc::BroadcastText {
                male: format!("Line {id}."),
                chat_type,
                ..Default::default()
            },
        );
    }
}

fn heard(m: &ServerOpcodeMessage, line: &str) -> Option<SMSG_MESSAGECHAT_ChatType> {
    match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(c) if c.message == line => Some(c.chat_type.clone()),
        _ => None,
    }
}

/// Talk's datalong names the chat type over the text's own; a boss emote
/// carries as far as a yell, not over the map; a zone yell is the zone's;
/// a zone emote is an emote, and a boss whisper the raid boss whisper of
/// 1.12's layout (S16).
#[tokio::test]
async fn talk_has_thistle_teas_chat_types_and_reach() {
    let mut d = world();
    with_texts(&mut d);
    let realm = harness::realm_with(d);
    let (mut a, map) = player(&realm, "Alice").await;
    let (mut b, _) = player(&realm, "Bob").await;
    let (me, bob, host) = (a.guid, b.guid, host());
    a.sees(host).await;
    // Alice 50 yards off in the host's zone; Bob 400 yards off, in another.
    map.query(move |m| {
        let at = m.position_of(host).unwrap();
        if let Some(c) = m.creature_mut(host) {
            c.zone_area = Some((12, 12));
        }
        for (g, off, zone) in [(me, 50.0, 12), (bob, 400.0, 40)] {
            if let Some(p) = m.players.get_mut(&g) {
                p.movement.position = Vector3d {
                    x: at.x + off,
                    ..at
                };
                p.zone = zone;
            }
        }
    })
    .await;
    a.clear();
    b.clear();
    let talk = move |text: i32, chat: u32| {
        move |m: &mut crate::game::map::Map| {
            let say = ScriptStep {
                dataint: [text, 0, 0, 0],
                ..step(0, [chat, 0, 0, 0])
            };
            crate::game::map::scripts::execute(m, &say, host, me);
        }
    };
    // A say yelled carries to Alice.
    map.call(talk(100, 1));
    let kind = a.wait_for("the yell", |m| heard(m, "Line 100.")).await;
    assert!(matches!(
        kind,
        SMSG_MESSAGECHAT_ChatType::MonsterYell { .. }
    ));
    // A boss emote: Alice, not Bob.
    map.call(talk(103, 0));
    let kind = a
        .wait_for("the boss emote", |m| heard(m, "Line 103."))
        .await;
    assert!(
        matches!(kind, SMSG_MESSAGECHAT_ChatType::RaidBossEmote { monster, .. } if monster == me)
    );
    // A zone yell: Alice in the zone hears it; Bob, out of it, doesn't.
    map.call(talk(106, 0));
    a.wait_for("the zone yell", |m| heard(m, "Line 106.")).await;
    // A zone emote is an emote, as near as a say.
    map.query(move |m| {
        let at = m.position_of(host).unwrap();
        if let Some(p) = m.players.get_mut(&me) {
            p.movement.position = Vector3d {
                x: at.x + 20.0,
                ..at
            };
        }
    })
    .await;
    map.call(talk(107, 0));
    let kind = a
        .wait_for("the zone emote", |m| heard(m, "Line 107."))
        .await;
    assert!(matches!(
        kind,
        SMSG_MESSAGECHAT_ChatType::MonsterEmote { .. }
    ));
    // A boss whisper: chat type 0x59, the speaker's name, Alice, the line.
    map.call(talk(105, 0));
    let body = a
        .wait_frame("the boss whisper", 0x096, |b| b.first() == Some(&0x59))
        .await;
    let name_len = u32::from_le_bytes(body[5..9].try_into().unwrap()) as usize;
    assert_eq!(&body[9..9 + name_len - 1], b"Host");
    let at = 9 + name_len;
    assert_eq!(
        u64::from_le_bytes(body[at..at + 8].try_into().unwrap()),
        me.guid()
    );
    assert!(body.windows(9).any(|w| w == b"Line 105."));
    // Bob, 400 yards off in another zone, heard none of them.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let bob_heard = b
        .gets(Duration::from_millis(50), |m| match m {
            ServerOpcodeMessage::SMSG_MESSAGECHAT(c) if c.message.starts_with("Line") => Some(()),
            _ => None,
        })
        .await;
    assert!(bob_heard.is_none(), "Bob is out of reach");
}

/// A script's emote that the unit stays in becomes its emote state (and
/// none, 0, clears it); another plays once (D-R8).
#[tokio::test]
async fn a_script_emote_that_lasts_is_an_emote_state() {
    let mut d = world();
    // Emote 10 (dance) is one a unit stays in.
    d.emote_states.insert(10);
    let realm = harness::realm_with(d);
    let (mut a, map) = player(&realm, "Alice").await;
    let host = host();
    a.sees(host).await;
    a.clear();
    let state = move |m: &crate::game::map::Map| {
        m.unit_fields(host)
            .map_or(0, |f| f.get_u32(UNIT_NPC_EMOTESTATE))
    };
    let dancing = map
        .query(move |m| {
            crate::game::map::scripts::execute(m, &step(1, [10, 0, 0, 0]), host, host);
            state(m)
        })
        .await
        .unwrap();
    assert_eq!(dancing, 10, "it dances on");
    let played = a
        .gets(Duration::from_millis(200), |m| match m {
            ServerOpcodeMessage::SMSG_EMOTE(e) if e.guid == host => Some(()),
            _ => None,
        })
        .await;
    assert!(played.is_none(), "no emote played for a state");
    let (talking, cleared) = map
        .query(move |m| {
            crate::game::map::scripts::execute(m, &step(1, [1, 0, 0, 0]), host, host);
            let talking = state(m);
            crate::game::map::scripts::execute(m, &step(1, [0, 0, 0, 0]), host, host);
            (talking, state(m))
        })
        .await
        .unwrap();
    assert_eq!(
        (talking, cleared),
        (10, 0),
        "a one-off leaves it; 0 clears it"
    );
    a.wait_for("the talk emote", |m| match m {
        ServerOpcodeMessage::SMSG_EMOTE(e) if e.guid == host => Some(()),
        _ => None,
    })
    .await;
}

fn faction_of(m: &crate::game::map::Map, g: Guid) -> u32 {
    m.unit_fields(g)
        .map_or(0, |f| f.get_u32(UNIT_FIELD_FACTIONTEMPLATE))
}

/// `set_faction`'s datalong2 says when the old faction comes back: when a
/// fight stops (0x2), home again (0x4), at a respawn (0x1, and without it
/// the faction stays over one); datalong 0 brings it back (S14).
#[tokio::test]
async fn a_scripts_faction_comes_back_as_its_flags_say() {
    let realm = harness::realm_with(world());
    let (mut a, map) = player(&realm, "Alice").await;
    let me = a.guid;
    let (host, buddy) = (host(), buddy());
    a.sees(buddy).await;
    let out = map
        .query(move |m| {
            let set = |flags| step(22, [14, flags, 0, 0]);
            // Until the fight stops.
            crate::game::map::scripts::execute(m, &set(0x2), host, host);
            let set_it = faction_of(m, host);
            crate::game::map::scripts::execute(m, &step(73, [0; 4]), host, host);
            let stopped = faction_of(m, host);
            // Not over a respawn, then over one.
            crate::game::map::scripts::execute(m, &set(0), host, host);
            crate::game::map::scripts::execute(m, &set(0), buddy, buddy);
            crate::game::map::scripts::execute(m, &set(0x1), buddy, buddy);
            for g in [host, buddy] {
                crate::game::map::combat::deal_damage(m, me, g, 10_000);
                crate::game::map::ai::respawn_now(m, g);
            }
            let respawned = (faction_of(m, host), faction_of(m, buddy));
            // Datalong 0: back to the one before.
            crate::game::map::scripts::execute(m, &step(22, [0; 4]), host, host);
            (set_it, stopped, respawned, faction_of(m, host))
        })
        .await
        .unwrap();
    assert_eq!(out, (14, 35, (14, 35), 35));
}

/// Home again from an evade, a faction set until then comes back (S14).
#[tokio::test]
async fn a_scripts_faction_until_home_comes_back_there() {
    let realm = harness::realm_with(world());
    let (mut a, map) = player(&realm, "Alice").await;
    let host = host();
    a.sees(host).await;
    map.query(move |m| {
        crate::game::map::scripts::execute(m, &step(22, [14, 0x4, 0, 0]), host, host);
        if let Some(c) = m.creature_mut(host) {
            c.position.x += 6.0;
        }
        crate::game::map::ai::enter_evade(m, host);
    })
    .await;
    let mut back = 0;
    for _ in 0..100 {
        back = map.query(move |m| faction_of(m, host)).await.unwrap();
        if back == 35 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(back, 35, "home, it is friendly again");
}

/// `despawn`'s datalong2 is the respawn's delay; with none, the spawn's
/// own time; datalong puts it off (S18).
#[tokio::test]
async fn despawn_respawns_after_its_delay_or_the_spawns_time() {
    let realm = harness::realm_with(world());
    let (mut a, map) = player(&realm, "Alice").await;
    let (host, buddy, third) = (host(), buddy(), third());
    a.sees(third).await;
    let respawn_in = |m: &crate::game::map::Map, g| match m.creature(g).map(|c| c.life) {
        Some(crate::game::map::creature::Life::Dead { respawn_at, .. }) => {
            Some(respawn_at.duration_since(m.now).as_secs())
        }
        _ => None,
    };
    let out = map
        .query(move |m| {
            crate::game::map::scripts::execute(m, &step(18, [0, 40, 0, 0]), host, host);
            crate::game::map::scripts::execute(m, &step(18, [0, 0, 0, 0]), buddy, buddy);
            crate::game::map::scripts::execute(m, &step(18, [300, 0, 0, 0]), third, third);
            (
                respawn_in(m, host),
                respawn_in(m, buddy),
                respawn_in(m, third),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        (Some(40), Some(60), None),
        "its own delay; its spawn's; not yet"
    );
    a.forgets(third).await;
}

/// `C_SET_REGENERATION` says what it regenerates: nothing, or health
/// alone (S20).
#[tokio::test]
async fn set_regeneration_says_what_comes_back() {
    let mut d = world();
    d.creature_templates.get_mut(&HOST).unwrap().unit_class = 8;
    let realm = harness::realm_with(d);
    let (mut a, map) = player(&realm, "Alice").await;
    let host = host();
    a.sees(host).await;
    let out = map
        .query(move |m| {
            let tick = |m: &mut crate::game::map::Map| {
                let past = m.now - Duration::from_secs(1);
                if let Some(c) = m.creature_mut(host) {
                    c.fields.set_u32(UNIT_FIELD_HEALTH, 100);
                    c.fields.set_u32(UNIT_FIELD_MAXPOWER1, 300);
                    c.fields.set_u32(UNIT_FIELD_POWER1, 0);
                    c.engagement.regen_at = Some(past);
                }
                crate::game::map::creature_combat::regenerate(m);
                m.unit_fields(host).map(|f| {
                    (
                        f.get_u32(UNIT_FIELD_HEALTH) > 100,
                        f.get_u32(UNIT_FIELD_POWER1) > 0,
                    )
                })
            };
            let regen = |stats| ScriptStep {
                command: crate::game::map::scripts::C_SET_REGENERATION,
                datalong: [stats, 0, 0, 0],
                ..Default::default()
            };
            crate::game::map::scripts::execute(m, &regen(0), host, host);
            let none = tick(m);
            crate::game::map::scripts::execute(m, &regen(1), host, host);
            (none, tick(m))
        })
        .await
        .unwrap();
    assert_eq!(out, (Some((false, false)), Some((true, false))));
}

/// `set_equipment`: a slot below zero is left, at zero emptied, else the
/// item; with datalong, what it spawned with (S21).
#[tokio::test]
async fn set_equipment_leaves_empties_and_resets_slots() {
    let mut d = world();
    for entry in [5000, 5001, 5002, 5003] {
        d.items.insert(
            entry,
            ItemTemplate {
                entry,
                name: format!("Weapon {entry}"),
                display_id: entry + 10,
                ..Default::default()
            },
        );
    }
    d.creature_equipment.insert(77, [5000, 5001, 5002]);
    d.creature_templates.get_mut(&HOST).unwrap().equipment_id = 77;
    let realm = harness::realm_with(d);
    let (mut a, map) = player(&realm, "Alice").await;
    let host = host();
    a.sees(host).await;
    let out = map
        .query(move |m| {
            let displays = |m: &crate::game::map::Map| {
                [0u16, 1, 2].map(|s| {
                    m.unit_fields(host)
                        .map_or(0, |f| f.get_u32(UNIT_VIRTUAL_ITEM_SLOT_DISPLAY + s))
                })
            };
            let spawned = displays(m);
            let change = ScriptStep {
                dataint: [-1, 0, 5003, 0],
                ..step(19, [0; 4])
            };
            crate::game::map::scripts::execute(m, &change, host, host);
            let changed = displays(m);
            crate::game::map::scripts::execute(m, &step(19, [1, 0, 0, 0]), host, host);
            (spawned, changed, displays(m))
        })
        .await
        .unwrap();
    assert_eq!(out.0, [5010, 5011, 5012]);
    assert_eq!(out.1, [5010, 0, 5013], "left, emptied, the new one");
    assert_eq!(out.2, [5010, 5011, 5012], "back as it spawned");
}

const DUNGEON: u32 = 33;

/// `zone_combat_pulse` draws a dungeon into the fight, every three seconds
/// while it lasts; outside a dungeon it does nothing (S15).
#[tokio::test]
async fn the_zone_pulse_is_a_dungeons_and_goes_on() {
    let mut d = world();
    d.maps.insert(
        DUNGEON,
        MapInfo {
            id: DUNGEON,
            internal_name: "Dungeon".into(),
            name: "Dungeon".into(),
            instance_type: 1,
            player_limit: 5,
        },
    );
    d.creature_templates
        .insert(4100, creature(4100, "Warden", 14));
    d.creature_spawns.insert(
        DUNGEON,
        vec![CreatureSpawn {
            guid: 70,
            entries: vec![4100],
            map: DUNGEON,
            x: 40.0,
            ..Default::default()
        }],
    );
    d.creature_templates
        .insert(4101, creature(4101, "Brute", 14));
    d.creature_spawns.get_mut(&0).unwrap().push(CreatureSpawn {
        guid: 71,
        entries: vec![4101],
        map: 0,
        x: START.0 + 40.0,
        y: START.1,
        z: START.2,
        ..Default::default()
    });
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = Realm::new(cfg, store, startup, d, None);
    let (mut a, map) = player(&realm, "Alice").await;
    let me = a.guid;
    // In the open world, nothing.
    let brute = guid::unit(4101, 71);
    let open = map
        .query(move |m| {
            crate::game::map::scripts::execute(m, &step(49, [1, 0, 0, 0]), brute, brute);
            m.creature(brute).map(|c| c.combat.victim)
        })
        .await
        .flatten();
    assert_eq!(open, Some(Guid::zero()), "the open world isn't drawn in");
    map.call(move |m| {
        m.teleport(
            me,
            crate::game::map::MapKey::world(DUNGEON),
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
    a.sees(me).await;
    let inside = realm.registry.online(me).unwrap().map();
    let warden = guid::unit(4100, 70);
    let pulsed = inside
        .query(move |m| {
            crate::game::map::scripts::execute(m, &step(49, [1, 0, 0, 0]), warden, warden);
            m.creature(warden).map(|c| {
                (
                    c.combat.victim,
                    c.engagement.zone_pulse_at.map(|t| t.duration_since(m.now)),
                )
            })
        })
        .await
        .flatten();
    assert_eq!(
        pulsed,
        Some((me, Some(Duration::from_secs(3)))),
        "the nearest, 40 yards off, first; again in three seconds"
    );
}

const SON: u32 = 4300;

fn with_son(d: &mut GameData) {
    d.creature_templates
        .insert(SON, creature(SON, "Son of Flame", 14));
}

/// The host summons a Son for `ms` (0 for no time) of despawn type
/// `kind`, beside it.
fn summon_son(m: &mut crate::game::map::Map, ms: u32, kind: i32) -> Guid {
    let summon = ScriptStep {
        dataint: [0, 0, -1, kind],
        x: START.0 + 3.0,
        y: START.1 + 4.0,
        z: START.2,
        ..step(10, [SON, ms, 0, 0])
    };
    crate::game::map::scripts::execute(m, &summon, host(), Guid::zero());
    m.creatures
        .iter()
        .filter(|(_, c)| c.entry == SON)
        .map(|(_, c)| c.guid)
        .max_by_key(|g| g.guid())
        .unwrap()
}

/// Summons go as their despawn types have them (D-R1): 2 and 5 vanish at
/// death; 1 waits out a fight, its time starting again when it ends; 3
/// goes in one; 4 times again from its death; 7 has no time.
#[tokio::test]
async fn summons_go_as_their_despawn_types_say() {
    let mut d = world();
    with_son(&mut d);
    let realm = harness::realm_with(d);
    let (mut a, map) = player(&realm, "Alice").await;
    let me = a.guid;
    a.sees(host()).await;
    let out = map
        .query(move |m| {
            let due = |m: &mut crate::game::map::Map, g| {
                let past = m.now - Duration::from_millis(1);
                if let Some(s) = crate::game::map::event_ai::scripted_mut(m, g) {
                    s.despawn_at = Some(past);
                }
                crate::game::map::event_ai::update_despawns(m);
            };
            let time_left = |m: &crate::game::map::Map, g| {
                crate::game::map::event_ai::scripted(m, g)
                    .and_then(|s| s.despawn_at)
                    .map(|t| t.saturating_duration_since(m.now).as_secs())
            };
            let kill = |m: &mut crate::game::map::Map, g| {
                crate::game::map::combat::deal_damage(m, me, g, 100_000);
            };
            // 2 and 5: gone at death.
            let two = summon_son(m, 60_000, 2);
            let five = summon_son(m, 0, 5);
            kill(m, two);
            kill(m, five);
            let vanished = (m.creature(two).is_none(), m.creature(five).is_none());
            // 1: in a fight, it waits; the fight over, its minute again.
            let one = summon_son(m, 60_000, 1);
            crate::game::map::combat::engage(m, me, one);
            due(m, one);
            let waited = m.creature(one).is_some();
            crate::game::map::ai::enter_evade(m, one);
            let again = time_left(m, one);
            // 3: gone, fight or not.
            let three = summon_son(m, 60_000, 3);
            crate::game::map::combat::engage(m, me, three);
            due(m, three);
            let gone = m.creature(three).is_none();
            // 4: dead, its time starts from its death.
            let four = summon_son(m, 30_000, 4);
            kill(m, four);
            let from_death = (m.creature(four).is_some(), time_left(m, four));
            // 7: no time at all.
            let seven = summon_son(m, 60_000, 7);
            (
                vanished,
                waited,
                again,
                gone,
                from_death,
                time_left(m, seven),
            )
        })
        .await
        .unwrap();
    assert_eq!(out.0, (true, true), "2 and 5 vanish at death");
    assert!(out.1, "1 waits out its fight");
    assert_eq!(out.2, Some(60), "and starts its minute again after it");
    assert!(out.3, "3 goes in a fight");
    assert_eq!(out.4, (true, Some(30)), "4's corpse waits its time");
    assert_eq!(out.5, None, "7 has none");
}

/// A summoner told of a summon's death no longer counts it among the
/// living (D-R2): the second of two Sons dying is the last.
#[tokio::test]
async fn a_dying_summon_is_not_counted_alive() {
    let mut d = world();
    with_son(&mut d);
    // No Son alive within 100 yards (reversed).
    d.scripts.conditions.insert(
        900,
        Condition {
            entry: 900,
            kind: 20,
            values: [SON as i32, 100, 0, 0],
            flags: CF_REVERSE_RESULT,
        },
    );
    d.scripts.ai_events.insert(
        HOST,
        vec![AiEvent {
            id: 1,
            creature: HOST,
            kind: EVENT_SUMMONED_JUST_DIED,
            chance: 100,
            flags: EF_REPEATABLE,
            params: [SON as i32, 0, 0, 0],
            actions: [901, 0, 0],
            ..Default::default()
        }],
    );
    d.scripts.ai_scripts.insert(
        901,
        vec![ScriptStep {
            condition: 900,
            ..step(44, [1, 0, 0, 0])
        }],
    );
    let realm = harness::realm_with(d);
    let (mut a, map) = player(&realm, "Alice").await;
    let me = a.guid;
    let host = host();
    a.sees(host).await;
    let phases = map
        .query(move |m| {
            let sons = [summon_son(m, 0, 0), summon_son(m, 0, 0)];
            let mut seen = Vec::new();
            for s in sons {
                crate::game::map::combat::deal_damage(m, me, s, 100_000);
                seen.push(crate::game::map::event_ai::phase(m, host));
            }
            seen
        })
        .await
        .unwrap();
    assert_eq!(phases, vec![0, 1], "the last Son's death is the last");
}

/// An unkillable template keeps a creature at one health, until a script
/// says otherwise (D-R4).
#[tokio::test]
async fn an_unkillable_creature_stops_at_one_health() {
    let mut d = world();
    d.creature_templates.get_mut(&HOST).unwrap().static_flags1 = 0x8;
    let realm = harness::realm_with(d);
    let (mut a, map) = player(&realm, "Alice").await;
    let me = a.guid;
    let host = host();
    a.sees(host).await;
    let out = map
        .query(move |m| {
            crate::game::map::combat::deal_damage(m, me, host, 100_000);
            let held = m
                .creature(host)
                .map(|c| (c.alive(), c.fields.get_u32(UNIT_FIELD_HEALTH)));
            // Invincibility 0 lets it die.
            crate::game::map::scripts::execute(m, &step(52, [0; 4]), host, host);
            crate::game::map::combat::deal_damage(m, me, host, 100_000);
            (held, m.creature(host).map(|c| c.alive()))
        })
        .await
        .unwrap();
    assert_eq!(out, (Some((true, 1)), Some(false)));
}

const LATHORIC: u32 = 4400;

/// `update_entry` makes it the other entry whole: look, size, faction,
/// flags, level, stats in proportion and weapons; it keeps its old
/// entry's EventAI; a respawn brings the old entry back (S17, D-R6).
#[tokio::test]
async fn update_entry_becomes_the_entry_and_keeps_its_events() {
    let mut d = world();
    hear_event_seven(&mut d);
    let mut lathoric = creature(LATHORIC, "Lathoric the Black", 14);
    lathoric.display_ids = [7777, 0, 0, 0];
    lathoric.display_scales = [2.0, 0.0, 0.0, 0.0];
    lathoric.level_min = 20;
    lathoric.level_max = 20;
    lathoric.npc_flags = 0x1;
    lathoric.unit_flags = 0x100;
    lathoric.speed_run = 2.0;
    lathoric.equipment_id = 78;
    d.creature_templates.insert(LATHORIC, lathoric);
    d.items.insert(
        5100,
        ItemTemplate {
            entry: 5100,
            name: "Black Blade".into(),
            display_id: 5110,
            ..Default::default()
        },
    );
    d.creature_equipment.insert(78, [5100, 0, 0]);
    let realm = harness::realm_with(d);
    let (mut a, map) = player(&realm, "Alice").await;
    let me = a.guid;
    let host = host();
    a.sees(host).await;
    let out = map
        .query(move |m| {
            if let Some(f) = m.unit_fields_mut(host) {
                let max = f.get_u32(UNIT_FIELD_MAXHEALTH);
                f.set_u32(UNIT_FIELD_HEALTH, max / 2);
            }
            crate::game::map::scripts::execute(m, &step(27, [LATHORIC, 0, 0, 0]), host, host);
            let c = m.creature(host).unwrap();
            let f = &c.fields;
            let became = (
                c.entry,
                f.get_u32(UNIT_FIELD_DISPLAYID),
                f.get_f32(OBJECT_FIELD_SCALE_X),
                f.get_u32(UNIT_FIELD_FACTIONTEMPLATE),
                f.get_u32(UNIT_NPC_FLAGS),
                f.get_u32(UNIT_FIELD_FLAGS) & 0x100,
                f.get_u32(UNIT_FIELD_LEVEL),
                f.get_u32(UNIT_FIELD_HEALTH) * 2 / f.get_u32(UNIT_FIELD_MAXHEALTH).max(1),
                f.get_u32(UNIT_VIRTUAL_ITEM_SLOT_DISPLAY),
                c.speeds.run,
            );
            // Its old entry's events still run: map event 7's script event.
            crate::game::map::event_ai::script_event(m, host, 7, 5, None);
            let phase = crate::game::map::event_ai::phase(m, host);
            crate::game::map::combat::deal_damage(m, me, host, 100_000);
            crate::game::map::ai::respawn_now(m, host);
            (became, phase, m.creature(host).map(|c| c.entry))
        })
        .await
        .unwrap();
    assert_eq!(
        out.0,
        (LATHORIC, 7777, 2.0, 14, 0x1, 0x100, 20, 1, 5110, 14.0)
    );
    assert_eq!(out.1, 1, "the host's own events");
    assert_eq!(out.2, Some(HOST), "back as it was at its respawn");
}

/// A script's walk home is an arrival home: its reached-home events fire
/// (D25, Mizzle's walk back for the Gordok tribute).
#[tokio::test]
async fn a_scripts_walk_home_fires_reached_home() {
    let mut d = world();
    d.scripts.ai_events.insert(
        HOST,
        vec![AiEvent {
            id: 1,
            creature: HOST,
            kind: EVENT_REACHED_HOME,
            chance: 100,
            flags: EF_REPEATABLE,
            actions: [902, 0, 0],
            ..Default::default()
        }],
    );
    d.scripts
        .ai_scripts
        .insert(902, vec![step(44, [1, 0, 0, 0])]);
    let realm = harness::realm_with(d);
    let (mut a, map) = player(&realm, "Alice").await;
    let host = host();
    a.sees(host).await;
    map.query(move |m| {
        if let Some(c) = m.creature_mut(host) {
            c.position.y += 6.0;
        }
        crate::game::map::scripts::execute(m, &step(20, [7, 0, 0, 0]), host, host);
    })
    .await;
    let mut phase = 0;
    for _ in 0..100 {
        phase = map
            .query(move |m| crate::game::map::event_ai::phase(m, host))
            .await
            .unwrap();
        if phase == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(phase, 1, "home, its event fired");
}

/// Generic script 950: its source sits.
fn with_sit(d: &mut GameData) {
    d.scripts.generic.insert(950, vec![step(28, [1, 0, 0, 0])]);
}

/// The smaller commands of S23: zone scripts by zone only (92); an entry
/// leaves players out (68); the wander radius (67); a permanent aura (74);
/// the source teleports (6); a group script runs on an ungrouped source
/// (90); the newest extra target is the one found (63).
#[tokio::test]
async fn the_smaller_commands_are_thistle_teas() {
    let mut d = world();
    with_sit(&mut d);
    let mut aura = SpellTemplate {
        id: 6200,
        name: "Warded".into(),
        equipped_item_class: -1,
        duration_index: 77,
        ..Default::default()
    };
    aura.effects[0] = SpellEffect {
        effect: 6,
        aura: 4,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(6200, aura);
    d.spells.durations.insert(
        77,
        crate::game::data::spells::SpellDuration {
            base: 10_000,
            per_level: 0,
            max: 10_000,
        },
    );
    let realm = harness::realm_with(d);
    let (mut a, map) = player(&realm, "Alice").await;
    let me = a.guid;
    let (host, buddy, third) = (host(), buddy(), third());
    a.sees(third).await;
    let out = map
        .query(move |m| {
            let reset = |m: &mut crate::game::map::Map| {
                for g in [me, host, buddy, third] {
                    if let Some(f) = m.unit_fields_mut(g) {
                        f.set_byte(UNIT_FIELD_BYTES_1, 0, 0);
                    }
                }
            };
            let sitting = |m: &crate::game::map::Map| {
                [me, host, buddy, third].map(|g| stand_state(m, g) == 1)
            };
            if let Some(p) = m.players.get_mut(&me) {
                p.zone = 12;
            }
            // 92: zone 0 is no one's zone; zone 12 is Alice's.
            crate::game::map::scripts::execute(m, &step(92, [950, 0, 0, 0]), host, host);
            let zone_none = sitting(m);
            crate::game::map::scripts::execute(m, &step(92, [950, 12, 0, 0]), host, host);
            let zone_twelve = sitting(m);
            reset(m);
            // 68: units of Buddy's entry within 30 yards: not Alice.
            crate::game::map::scripts::execute(m, &step(68, [950, 1, BUDDY, 30]), host, host);
            let for_all = sitting(m);
            reset(m);
            // 67: wandering 7 yards about.
            crate::game::map::scripts::execute(m, &step(67, [1, 0, 7, 0]), host, host);
            let wander = m.creature(host).map(|c| c.wander_distance);
            // 74 with 0x8: for good.
            crate::game::map::scripts::execute(m, &step(74, [6200, 0x8, 0, 0]), host, host);
            let lasting = crate::game::map::auras::auras(m, host).and_then(|a| {
                a.holders
                    .iter()
                    .find(|h| h.spell == 6200)
                    .map(|h| h.expires)
            });
            // 6: the host, the source, teleports; Alice, its target, stays.
            let alice_at = m.position_of(me).unwrap();
            let teleport = ScriptStep {
                x: START.0 + 20.0,
                y: START.1,
                z: START.2,
                ..step(6, [0; 4])
            };
            crate::game::map::scripts::execute(m, &teleport, host, me);
            let moved = (
                m.position_of(host).map(|p| p.x - START.0),
                m.position_of(me) == Some(alice_at),
            );
            // 90: ungrouped, the host runs it itself.
            let group = ScriptStep {
                dataint: [100, 0, 0, 0],
                ..step(90, [950, 0, 0, 0])
            };
            crate::game::map::scripts::execute(m, &group, host, Guid::zero());
            let grouped = sitting(m);
            reset(m);
            // 63: Buddy, then Third, extra targets: the newest is found.
            crate::game::map::scripts::execute(m, &step(61, [8, 600, 0, 0]), host, host);
            crate::game::map::scripts::execute(m, &step(63, [8, 0, 0, 0]), buddy, host);
            crate::game::map::scripts::execute(m, &step(63, [8, 0, 0, 0]), third, host);
            let newest = crate::game::map::map_events::target_of(m, 24, 8, 0);
            (
                zone_none,
                zone_twelve,
                for_all,
                wander,
                lasting,
                moved,
                grouped,
                newest,
            )
        })
        .await
        .unwrap();
    assert_eq!(out.0, [false; 4], "92: zone 0 is no one's");
    assert_eq!(out.1, [true, false, false, false], "92: Alice's zone");
    assert_eq!(out.2, [false, false, true, false], "68: Buddy, not Alice");
    assert_eq!(out.3, Some(7.0), "67's wander radius");
    assert_eq!(out.4, Some(None), "74: for good");
    assert_eq!(out.5, (Some(20.0), true), "6: the host goes, Alice stays");
    assert_eq!(out.6, [false, true, false, false], "90: the host itself");
    assert_eq!(out.7, Some(third), "63: the newest");
}

/// A unique summon counts its kind within 50 yards of its place (not 100),
/// the dead too for a type whose corpse keeps its time (S23).
#[tokio::test]
async fn a_unique_summon_looks_fifty_yards_and_counts_lasting_corpses() {
    let mut d = world();
    with_son(&mut d);
    let realm = harness::realm_with(d);
    let (mut a, map) = player(&realm, "Alice").await;
    let me = a.guid;
    a.sees(host()).await;
    let out = map
        .query(move |m| {
            let host = host();
            let unique = |kind: i32, dx: f32| ScriptStep {
                dataint: [0x4, 0, -1, kind],
                x: START.0 + dx,
                y: START.1 + 4.0,
                z: START.2,
                ..step(10, [SON, 60_000, 1, 0])
            };
            let sons = |m: &crate::game::map::Map| {
                m.creatures.iter().filter(|(_, c)| c.entry == SON).count()
            };
            // One 70 yards from the next one's place: beyond 50.
            crate::game::map::scripts::execute(m, &unique(1, 73.0), host, Guid::zero());
            crate::game::map::scripts::execute(m, &unique(1, 3.0), host, Guid::zero());
            let far = sons(m);
            // The near one dead: type 1 doesn't count it, type 3 does.
            let near = m
                .creatures
                .iter()
                .filter(|(_, c)| c.entry == SON)
                .min_by(|x, y| x.1.position.x.total_cmp(&y.1.position.x))
                .map(|(_, c)| c.guid)
                .unwrap();
            crate::game::map::combat::deal_damage(m, me, near, 100_000);
            crate::game::map::scripts::execute(m, &unique(3, 3.0), host, Guid::zero());
            let counted = sons(m);
            crate::game::map::scripts::execute(m, &unique(1, 3.0), host, Guid::zero());
            (far, counted, sons(m))
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        (2, 2, 3),
        "beyond 50; a lasting corpse counts; a plain one doesn't"
    );
}

/// A step's script start runs at once, and the steps of the scripts under
/// way that are due with it still run that tick (S23, nested starts).
#[tokio::test]
async fn a_nested_start_slips_no_one_a_tick() {
    let mut d = world();
    with_sit(&mut d);
    // 960: the host kneels. 961: start 950 (Buddy sits), then phase 2.
    d.scripts.generic.insert(
        960,
        vec![ScriptStep {
            delay_ms: 100,
            ..step(28, [8, 0, 0, 0])
        }],
    );
    d.scripts.generic.insert(
        961,
        vec![
            ScriptStep {
                delay_ms: 100,
                ..step(39, [950, 0, 0, 0])
            },
            ScriptStep {
                delay_ms: 100,
                ..step(44, [2, 0, 0, 0])
            },
        ],
    );
    let realm = harness::realm_with(d);
    let (mut a, map) = player(&realm, "Alice").await;
    let (host, buddy) = (host(), buddy());
    a.sees(buddy).await;
    let out = map
        .query(move |m| {
            crate::game::map::scripts::start(m, ScriptTable::Generic, 960, host, host);
            crate::game::map::scripts::start(m, ScriptTable::Generic, 961, buddy, buddy);
            let past = m.now - Duration::from_millis(200);
            for r in m.scripts.runs.iter_mut() {
                r.started = past;
            }
            crate::game::map::scripts::update(m);
            (
                stand_state(m, host),
                stand_state(m, buddy),
                crate::game::map::event_ai::phase(m, buddy),
            )
        })
        .await
        .unwrap();
    assert_eq!(out, (8, 1, 2), "all three due steps ran in the one update");
}
