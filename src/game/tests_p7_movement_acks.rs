//! Round 7's movement acknowledgements, ported from Thistle Tea's
//! `test/game/inbound/cmsg_*_ack_test.exs` and
//! `test/game/network/message/{knockback,movement_speed}_test.exs`
//! (docs/parity/matrix.csv): every forced movement waits for the client's
//! acknowledgement of its own counter, unit and change; any other is stale
//! and changes nothing; a teleport makes earlier launches stale, a new map
//! every order; a pet left behind by a teleport comes back at its
//! acknowledgement.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::{ClientOpcodeMessage, ServerOpcodeMessage};
use wow_world_messages::vanilla::*;

use crate::game::conn::{Order, SpeedKind};
use crate::game::data::GameData;
use crate::game::data::spells::SpellEffect;
use crate::game::harness::Client;
use crate::game::map::auras::{
    self, AURA_FEATHER_FALL, AURA_MOD_DECREASE_SPEED, AURA_MOD_FEAR, AURA_WATER_WALK,
};
use crate::game::map::{MapHandle, MapKey, death, movement};
use crate::game::tests_p3_combat::{BRUTE, START, data, setup_with, spawn};
use crate::game::tests_p3_entity::{NONE_WITHIN, aura_effect, bob, on, spell};
use crate::game::tests_p3_player2_ammo::handled;

const FEAR: u32 = 97_710;
const WATER_WALK: u32 = 97_711;
const SLOW_FALL: u32 = 97_712;
const SLOW: u32 = 97_713;
const KNOCK: u32 = 97_714;
const WOLF: u32 = 97_715;
const EYES: u32 = 97_716;

fn world() -> GameData {
    let mut d = data();
    let mut aura = |id, kind, amount| {
        d.spells
            .spells
            .insert(id, spell(id, &[aura_effect(kind, amount, 0, 1)]));
    };
    aura(FEAR, AURA_MOD_FEAR, 1);
    aura(WATER_WALK, AURA_WATER_WALK, 1);
    aura(SLOW_FALL, AURA_FEATHER_FALL, 1);
    aura(SLOW, AURA_MOD_DECREASE_SPEED, -50);
    aura(EYES, auras::AURA_MOD_POSSESS_PET, 0);
    d.spells.spells.insert(
        KNOCK,
        spell(
            KNOCK,
            &[SpellEffect {
                effect: crate::game::map::spells::EFFECT_KNOCK_BACK,
                base_points: 69,
                base_dice: 1,
                misc_value: 120,
                target_a: 25,
                ..Default::default()
            }],
        ),
    );
    d.creature_templates.insert(
        WOLF,
        crate::game::data::CreatureTemplate {
            entry: WOLF,
            name: "Wolf".into(),
            display_ids: [903, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            level_min: 20,
            level_max: 20,
            faction: 14,
            unit_class: 1,
            creature_type: 1,
            family: 1,
            health_multiplier: 1.0,
            damage_multiplier: 1.0,
            base_attack_time: 2000,
            health_max: 500,
            ..Default::default()
        },
    );
    d.pets.levels.insert(
        (1, 20),
        crate::game::data::pets::PetLevel {
            health: 600,
            mana: 0,
            armor: 400,
            damage: (0.0, 0.0),
            stats: [40, 30, 40, 20, 20],
        },
    );
    d
}

fn start() -> Vector3d {
    Vector3d {
        x: START.0,
        y: START.1,
        z: START.2,
    }
}

/// A movement packet's info, standing at `position`.
fn at(position: Vector3d) -> MovementInfo {
    MovementInfo {
        flags: MovementInfo_MovementFlags::empty(),
        timestamp: 0,
        position,
        orientation: 0.0,
        fall_time: 0,
    }
}

async fn pending(map: &MapHandle, g: Guid) -> Vec<(u32, Order)> {
    on(map, move |m| m.players[&g].outbound.pending_orders()).await
}

async fn position(map: &MapHandle, g: Guid) -> Vector3d {
    on(map, move |m| m.players[&g].position()).await
}

/// A root or unroot ordered for `g`, and its counter.
async fn order_root(map: &MapHandle, c: &mut Client, g: Guid, root: bool) -> u32 {
    on(map, move |m| {
        movement::root_to(&m.players[&g].outbound, g, root)
    })
    .await;
    c.wait_for("the root order", |m| match m {
        ServerOpcodeMessage::SMSG_FORCE_MOVE_ROOT(r) if root && r.guid == g => Some(r.counter),
        ServerOpcodeMessage::SMSG_FORCE_MOVE_UNROOT(r) if !root && r.guid == g => Some(r.counter),
        _ => None,
    })
    .await
}

fn moved_root(m: &ServerOpcodeMessage) -> Option<Guid> {
    match m {
        ServerOpcodeMessage::MSG_MOVE_ROOT(r) => Some(r.player),
        ServerOpcodeMessage::MSG_MOVE_UNROOT(r) => Some(r.player),
        _ => None,
    }
}

/// A root's acknowledgement settles its order: one with another counter,
/// or for the other change, is stale and leaves it waiting. Settled, it
/// moves the player only where the client moves them: not a corpse, nor a
/// player the server runs (feared), though a released ghost (Thistle
/// Tea's "acknowledges control changes without moving corpses or
/// server-controlled players", "released ghosts still reconcile their
/// unroot acknowledgement", "ignores stale acknowledgements").
#[tokio::test]
async fn a_root_ack_settles_its_order_and_moves_only_a_player_the_client_moves() {
    let (realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let (mut b, _) = bob(&realm, ga).await;
    let start = position(&map, ga).await;
    let there = Vector3d {
        x: start.x + 2.0,
        ..start
    };
    let root = |counter| CMSG_FORCE_MOVE_ROOT_ACK {
        guid: ga,
        movement_counter: counter,
        info: MovementInfo {
            flags: MovementInfo_MovementFlags::new_root(),
            ..at(there)
        },
    };
    // Stale: another counter, or the unroot's acknowledgement.
    let counter = order_root(&map, &mut a, ga, true).await;
    b.clear();
    a.send(root(counter + 1)).await;
    a.send(CMSG_FORCE_MOVE_UNROOT_ACK {
        guid: ga,
        movement_counter: counter,
        info: at(there),
    })
    .await;
    handled(&mut a).await;
    assert_eq!(
        pending(&map, ga).await,
        [(
            counter,
            Order::Root {
                guid: ga,
                root: true
            }
        )]
    );
    assert_eq!(position(&map, ga).await, start, "stale, nothing moved");
    assert!(b.gets(NONE_WITHIN, moved_root).await.is_none());
    // Feared: settled, but the server runs her.
    let fear = on(&map, |m| m.realm.data.spells.get(FEAR).cloned().unwrap()).await;
    let driven = on(&map, move |m| {
        auras::apply(m, ga, ga, &fear, 0, 0);
        crate::game::map::player_motion::driven(m, ga)
    })
    .await;
    assert!(driven, "feared");
    a.send(root(counter)).await;
    handled(&mut a).await;
    assert!(pending(&map, ga).await.is_empty(), "settled");
    assert_ne!(position(&map, ga).await, there, "not where her client says");
    assert!(b.gets(NONE_WITHIN, moved_root).await.is_none());
    on(&map, move |m| auras::remove_spell(m, ga, FEAR, None)).await;
    // Dead: the corpse stays where it fell.
    on(&map, move |m| death::kill(m, ga)).await;
    let fell = position(&map, ga).await;
    let counter = a
        .wait_for("the root of the dead", |m| match m {
            ServerOpcodeMessage::SMSG_FORCE_MOVE_ROOT(r) if r.guid == ga => Some(r.counter),
            _ => None,
        })
        .await;
    b.clear();
    a.send(root(counter)).await;
    handled(&mut a).await;
    assert!(pending(&map, ga).await.iter().all(|(c, _)| *c != counter));
    assert_eq!(position(&map, ga).await, fell, "the corpse stays");
    assert!(b.gets(NONE_WITHIN, moved_root).await.is_none());
    // A released ghost walks where her client says.
    on(&map, move |m| death::release(m, ga)).await;
    a.wait_for("the ghost's teleport", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: ga,
        movement_counter: a.teleport_order(),
        time: Default::default(),
    })
    .await;
    let ghost = position(&map, ga).await;
    let walked = Vector3d {
        x: ghost.x + 2.0,
        ..ghost
    };
    let counter = order_root(&map, &mut a, ga, false).await;
    a.send(CMSG_FORCE_MOVE_UNROOT_ACK {
        guid: ga,
        movement_counter: counter,
        info: at(walked),
    })
    .await;
    handled(&mut a).await;
    let now = position(&map, ga).await;
    assert_eq!((now.x, now.y), (walked.x, walked.y), "the ghost walks");
}

/// What the client acknowledges of an order from before a teleport moves
/// no one: within the map, its place is the old one until the teleport is
/// acknowledged (mangos's `ProcessMovementInfo` while `IsBeingTeleported`),
/// and across maps the order is gone with the new world, while the client
/// loads it and after (Thistle Tea's "acknowledges unroot without
/// replacing the destination while loading a new map", "rejects an
/// old-map acknowledgement even after the new world is ready").
#[tokio::test]
async fn acknowledgements_from_before_a_teleport_move_no_one() {
    let (realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let start = position(&map, ga).await;
    let counter = order_root(&map, &mut a, ga, true).await;
    let away = Vector3d {
        x: start.x + 40.0,
        ..start
    };
    on(&map, move |m| m.teleport(ga, m.key, away, 0.0)).await;
    a.wait_for("the teleport", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    // The root's, sent before the teleport's, from where she was.
    a.send(CMSG_FORCE_MOVE_ROOT_ACK {
        guid: ga,
        movement_counter: counter,
        info: at(start),
    })
    .await;
    handled(&mut a).await;
    assert_eq!(position(&map, ga).await, away, "bound for the teleport");
    a.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: ga,
        movement_counter: a.teleport_order(),
        time: Default::default(),
    })
    .await;
    handled(&mut a).await;
    assert_eq!(position(&map, ga).await, away);
    assert!(pending(&map, ga).await.is_empty());
    // Across maps: an order from the old map, and one while loading.
    let old = order_root(&map, &mut a, ga, false).await;
    let there = Vector3d {
        x: 1629.0,
        y: -4373.0,
        z: 31.0,
    };
    on(&map, move |m| m.teleport(ga, MapKey::world(1), there, 0.0)).await;
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    let kalimdor = realm.maps.get(&realm, MapKey::world(1));
    assert!(pending(&kalimdor, ga).await.is_empty(), "the new world");
    let loading = order_root(&kalimdor, &mut a, ga, true).await;
    a.send(CMSG_FORCE_MOVE_ROOT_ACK {
        guid: ga,
        movement_counter: loading,
        info: at(start),
    })
    .await;
    handled(&mut a).await;
    assert!(pending(&kalimdor, ga).await.is_empty(), "settled");
    assert_eq!(position(&kalimdor, ga).await, there, "while loading");
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    a.sees(ga).await;
    a.send(CMSG_FORCE_MOVE_UNROOT_ACK {
        guid: ga,
        movement_counter: old,
        info: at(start),
    })
    .await;
    handled(&mut a).await;
    assert_eq!(position(&kalimdor, ga).await, there, "the old map's");
}

/// Water walking and Slow Fall, on and off: each acknowledgement settles
/// its own order, by counter, unit, kind and state; the apply word is any
/// nonzero value for on, as build 5875 sends 1.0's bits (Thistle Tea's
/// cmsg_move_water_walk_ack_test and cmsg_move_feather_fall_ack_test).
#[tokio::test]
async fn water_walk_and_slow_fall_acks_settle_their_own_orders() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let other = Guid::new(ga.guid() + 1);
    let (walk, fall) = on(&map, |m| {
        let s = |id| m.realm.data.spells.get(id).cloned().unwrap();
        (s(WATER_WALK), s(SLOW_FALL))
    })
    .await;
    on(&map, move |m| {
        auras::apply(m, ga, ga, &walk, 0, 0);
        auras::remove_spell(m, ga, WATER_WALK, None);
        auras::apply(m, ga, ga, &fall, 0, 0);
        auras::remove_spell(m, ga, SLOW_FALL, None);
    })
    .await;
    let mut counters = Vec::new();
    for _ in 0..4 {
        counters.push(
            a.wait_for("a water walk or Slow Fall order", |m| match m {
                ServerOpcodeMessage::SMSG_MOVE_WATER_WALK(o) => Some(o.counter),
                ServerOpcodeMessage::SMSG_MOVE_LAND_WALK(o) => Some(o.counter),
                ServerOpcodeMessage::SMSG_MOVE_FEATHER_FALL(o) => Some(o.counter),
                ServerOpcodeMessage::SMSG_MOVE_NORMAL_FALL(o) => Some(o.counter),
                _ => None,
            })
            .await,
        );
    }
    let [on_walk, off_walk, on_fall, off_fall] = counters[..] else {
        unreachable!()
    };
    let all = pending(&map, ga).await;
    assert_eq!(
        all,
        [
            (on_walk, Order::WaterWalk { guid: ga, on: true }),
            (
                off_walk,
                Order::WaterWalk {
                    guid: ga,
                    on: false
                }
            ),
            (on_fall, Order::FeatherFall { guid: ga, on: true }),
            (
                off_fall,
                Order::FeatherFall {
                    guid: ga,
                    on: false
                }
            ),
        ]
    );
    let walked = |guid, counter, apply| CMSG_MOVE_WATER_WALK_ACK {
        guid,
        movement_counter: counter,
        info: at(start()),
        apply,
    };
    let fell = |guid, counter, apply| CMSG_MOVE_FEATHER_FALL_ACK {
        guid,
        movement_counter: counter,
        info: at(start()),
        apply,
    };
    // The wrong unit, counter, state or kind: nothing.
    a.send(walked(other, on_walk, 1)).await;
    a.send(walked(ga, on_walk + 100, 1)).await;
    a.send(walked(ga, on_walk, 0)).await;
    a.send(walked(ga, on_fall, 1)).await;
    a.send(fell(ga, on_walk, 1)).await;
    a.send(fell(ga, on_fall, 0)).await;
    handled(&mut a).await;
    assert_eq!(pending(&map, ga).await, all, "none settled");
    // Each its own, the apply word 1.0's bits or 1.
    a.send(walked(ga, on_walk, 0x3F80_0000)).await;
    handled(&mut a).await;
    assert_eq!(pending(&map, ga).await, all[1..]);
    a.send(walked(ga, off_walk, 0)).await;
    a.send(fell(ga, on_fall, 1)).await;
    a.send(fell(ga, off_fall, 0)).await;
    handled(&mut a).await;
    assert!(pending(&map, ga).await.is_empty());
}

fn forced_speed(m: &ServerOpcodeMessage) -> Option<(SpeedKind, Guid, u32, f32)> {
    match m {
        ServerOpcodeMessage::SMSG_FORCE_RUN_SPEED_CHANGE(s) => {
            Some((SpeedKind::Run, s.guid, s.move_event, s.speed))
        }
        ServerOpcodeMessage::SMSG_FORCE_RUN_BACK_SPEED_CHANGE(s) => {
            Some((SpeedKind::RunBack, s.guid, s.move_event, s.speed))
        }
        ServerOpcodeMessage::SMSG_FORCE_SWIM_SPEED_CHANGE(s) => {
            Some((SpeedKind::Swim, s.guid, s.move_event, s.speed))
        }
        ServerOpcodeMessage::SMSG_FORCE_SWIM_BACK_SPEED_CHANGE(s) => {
            Some((SpeedKind::SwimBack, s.guid, s.move_event, s.speed))
        }
        _ => None,
    }
}

/// A speed's acknowledgement, of its kind.
fn speed_ack(kind: SpeedKind, guid: Guid, counter: u32, new_speed: f32) -> ClientOpcodeMessage {
    let info = at(start());
    match kind {
        SpeedKind::Run => CMSG_FORCE_RUN_SPEED_CHANGE_ACK {
            guid,
            counter,
            info,
            new_speed,
        }
        .into(),
        SpeedKind::RunBack => CMSG_FORCE_RUN_BACK_SPEED_CHANGE_ACK {
            guid,
            movement_counter: counter,
            info,
            new_speed,
        }
        .into(),
        SpeedKind::Swim => CMSG_FORCE_SWIM_SPEED_CHANGE_ACK {
            guid,
            counter,
            info,
            new_speed,
        }
        .into(),
        _ => CMSG_FORCE_SWIM_BACK_SPEED_CHANGE_ACK {
            guid,
            counter,
            info,
            new_speed,
        }
        .into(),
    }
}

/// A slow's forced speeds (run, backwards, swimming): each acknowledgement
/// settles its own, at the speed sent within the client's float error, in
/// any order, and changes no speed; the wrong unit, counter, kind or speed
/// is stale (Thistle Tea's "settles a shared sequence without changing
/// authoritative speeds", "rejects wrong owners, counters, modes and
/// speeds", "accepts float precision differences").
#[tokio::test]
async fn speed_acks_settle_at_the_speed_sent_and_change_no_speed() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let slow = on(&map, |m| m.realm.data.spells.get(SLOW).cloned().unwrap()).await;
    a.clear();
    on(&map, move |m| {
        auras::apply(m, ga, ga, &slow, 0, -50);
        auras::recalc(m, ga);
    })
    .await;
    let mut sent = Vec::new();
    while let Some(s) = a.gets(NONE_WITHIN, forced_speed).await {
        sent.push(s);
    }
    let kinds: Vec<SpeedKind> = sent.iter().map(|s| s.0).collect();
    assert!(
        [SpeedKind::Run, SpeedKind::RunBack, SpeedKind::Swim]
            .iter()
            .all(|k| kinds.contains(k)),
        "{kinds:?}"
    );
    let all = pending(&map, ga).await;
    assert_eq!(all.len(), sent.len());
    let speeds = on(&map, move |m| m.players[&ga].speeds).await;
    let (kind, _, counter, speed) = sent[0];
    let wrong_kind = if kind == SpeedKind::Run {
        SpeedKind::RunBack
    } else {
        SpeedKind::Run
    };
    for stale in [
        speed_ack(kind, Guid::new(ga.guid() + 1), counter, speed),
        speed_ack(kind, ga, counter + 100, speed),
        speed_ack(kind, ga, counter, 7.0),
        speed_ack(wrong_kind, ga, counter, speed),
    ] {
        a.send(stale).await;
    }
    handled(&mut a).await;
    assert_eq!(pending(&map, ga).await, all, "none settled");
    for &(kind, _, counter, speed) in sent.iter().rev() {
        a.send(speed_ack(kind, ga, counter, speed + 0.000_01)).await;
    }
    handled(&mut a).await;
    assert!(pending(&map, ga).await.is_empty());
    let after = on(&map, move |m| m.players[&ga].speeds).await;
    assert_eq!(
        (after.run, after.run_back, after.swim, after.swim_back),
        (speeds.run, speeds.run_back, speeds.swim, speeds.swim_back),
        "the server's speeds stay"
    );
}

/// A unit a player's client moves (Mind Control): its forced speed is
/// settled by that client's acknowledgement for the unit, at the speed
/// sent, and only then do those who see it hear of it; the player's own
/// GUID, another unit, another speed or counter are stale (Thistle Tea's
/// "settles controlled creature speeds only for the recorded mover and
/// value"; mangos-classic's `HandleForceSpeedChangeAckOpcodes`).
#[tokio::test]
async fn a_possessed_units_speed_ack_settles_only_for_its_mover_and_value() {
    let (realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let (mut b, gb) = bob(&realm, ga).await;
    let held = on(&map, move |m| spawn(m, ga, BRUTE, 10, 5.0)).await;
    a.sees(held).await;
    let slow = on(&map, |m| m.realm.data.spells.get(SLOW).cloned().unwrap()).await;
    b.clear();
    on(&map, move |m| {
        // Bob's client moves it.
        m.players.get_mut(&gb).unwrap().owned.mover = Some(held);
        auras::apply(m, ga, held, &slow, 0, -50);
        auras::recalc(m, held);
    })
    .await;
    let (_, _, counter, speed) = b
        .wait_for("the held one's speed", |m| {
            forced_speed(m).filter(|s| s.1 == held)
        })
        .await;
    let creature_speed = on(&map, move |m| m.creature(held).unwrap().speeds.run).await;
    a.clear();
    let heard = |a: &mut Client| {
        let opcode = crate::game::map::misc::MSG_MOVE_SET_RUN_SPEED;
        let frames = a.captured();
        frames
            .iter()
            .filter(|f| u16::from_le_bytes([f[2], f[3]]) == opcode)
            .count()
    };
    let before = heard(&mut a);
    for stale in [
        speed_ack(SpeedKind::Run, gb, counter, speed),
        speed_ack(SpeedKind::Run, Guid::new(held.guid() + 1), counter, speed),
        speed_ack(SpeedKind::Run, held, counter, 7.0),
        speed_ack(SpeedKind::Run, held, counter + 1, speed),
    ] {
        b.send(stale).await;
    }
    handled(&mut b).await;
    tokio::time::sleep(NONE_WITHIN).await;
    assert_eq!(heard(&mut a), before, "nothing heard of a stale one");
    assert_eq!(
        pending(&map, gb).await,
        [(
            counter,
            Order::Speed {
                guid: held,
                kind: SpeedKind::Run,
                speed
            }
        )]
    );
    b.send(speed_ack(SpeedKind::Run, held, counter, speed))
        .await;
    a.wait_frame(
        "those who see it hear of it",
        crate::game::map::misc::MSG_MOVE_SET_RUN_SPEED,
        |_| true,
    )
    .await;
    assert!(pending(&map, gb).await.is_empty());
    let now = on(&map, move |m| m.creature(held).unwrap().speeds.run).await;
    assert_eq!(now, creature_speed, "its speed stays the server's");
}

/// Knock Bob back from Alice: his launch's counter and impulse.
async fn launch(map: &MapHandle, b: &mut Client, ga: Guid, gb: Guid) -> (u32, [f32; 4]) {
    b.clear();
    on(map, move |m| {
        crate::game::map::spells::cast_triggered(m, ga, KNOCK, Some(gb));
    })
    .await;
    let body = b.wait_frame("SMSG_MOVE_KNOCK_BACK", 0x0EF, |_| true).await;
    let mut i = 0;
    let guid = packed_guid(&body, &mut i);
    assert_eq!(guid, Some(gb));
    let f = |n: usize| f32::from_le_bytes(body[i + 4 + 4 * n..i + 8 + 4 * n].try_into().unwrap());
    let counter = u32::from_le_bytes(body[i..i + 4].try_into().unwrap());
    (counter, [f(0), f(1), f(2), f(3)])
}

fn packed_guid(b: &[u8], i: &mut usize) -> Option<Guid> {
    let mask = *b.get(*i)?;
    *i += 1;
    let mut v = 0u64;
    for bit in 0..8 {
        if mask & (1 << bit) != 0 {
            v |= u64::from(*b.get(*i)?) << (bit * 8);
            *i += 1;
        }
    }
    Some(Guid::new(v))
}

fn launched(guid: Guid, counter: u32, impulse: [f32; 4], at: Vector3d) -> CMSG_MOVE_KNOCK_BACK_ACK {
    CMSG_MOVE_KNOCK_BACK_ACK {
        guid,
        counter,
        info: MovementInfo {
            flags: MovementInfo_MovementFlags::new_jumping(MovementInfo_MovementFlags_Jumping {
                cos_angle: impulse[0],
                sin_angle: impulse[1],
                xy_speed: impulse[2],
                z_speed: impulse[3],
            }),
            timestamp: 0,
            position: at,
            orientation: 0.0,
            fall_time: 0,
        },
    }
}

/// A launch's acknowledgement settles it once, for the unit, counter and
/// impulse sent, the jumping flag on: then those who see him watch him fly
/// (`MSG_MOVE_KNOCK_BACK`), and not again. A teleport makes a launch
/// before it stale, and a new map every one (Thistle Tea's knockback_test:
/// "consumes it once", "rejects wrong mover, counter, impulse, or movement
/// flags", "teleport and worldport invalidate earlier launches").
#[tokio::test]
async fn a_launch_ack_settles_once_for_the_launch_sent() {
    let (realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let (mut b, gb) = bob(&realm, ga).await;
    let (counter, impulse) = launch(&map, &mut b, ga, gb).await;
    let here = position(&map, gb).await;
    let flown = |a: &mut Client| {
        let frames = a.captured();
        frames
            .iter()
            .filter(|f| {
                u16::from_le_bytes([f[2], f[3]]) == crate::game::map::misc::MSG_MOVE_KNOCK_BACK
            })
            .count()
    };
    let before = flown(&mut a);
    let [cos, sin, xy, z] = impulse;
    let mut flat = launched(gb, counter, impulse, here);
    flat.info.flags = MovementInfo_MovementFlags::empty();
    for stale in [
        launched(ga, counter, impulse, here),
        launched(gb, counter + 1, impulse, here),
        flat,
        launched(gb, counter, [sin, cos, xy, z], here),
        launched(gb, counter, [cos, sin, xy * 2.0, z], here),
        launched(gb, counter, [cos, sin, xy, -z], here),
    ] {
        b.send(stale).await;
    }
    handled(&mut b).await;
    tokio::time::sleep(NONE_WITHIN).await;
    assert_eq!(flown(&mut a), before, "none taken");
    assert!(
        pending(&map, gb).await.iter().any(
            |(c, o)| *c == counter && matches!(o, Order::KnockBack { guid, .. } if *guid == gb)
        )
    );
    b.send(launched(gb, counter, impulse, here)).await;
    handled(&mut b).await;
    a.wait_frame(
        "MSG_MOVE_KNOCK_BACK",
        crate::game::map::misc::MSG_MOVE_KNOCK_BACK,
        |_| true,
    )
    .await;
    b.send(launched(gb, counter, impulse, here)).await;
    handled(&mut b).await;
    tokio::time::sleep(NONE_WITHIN).await;
    assert_eq!(flown(&mut a), before + 1, "once");
    // A teleport after a launch: the launch is stale.
    let (counter, impulse) = launch(&map, &mut b, ga, gb).await;
    on(&map, move |m| m.teleport(gb, m.key, here, 0.0)).await;
    b.wait_for("the teleport", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    let orders = pending(&map, gb).await;
    assert_eq!(
        orders,
        [(b.teleport_order(), Order::Teleport { guid: gb })],
        "the launch is gone"
    );
    b.send(launched(gb, counter, impulse, here)).await;
    handled(&mut b).await;
    assert_eq!(pending(&map, gb).await, orders);
    // And a new map, every order.
    launch(&map, &mut b, ga, gb).await;
    on(&map, move |m| {
        let to = Vector3d {
            x: 1629.0,
            y: -4373.0,
            z: 31.0,
        };
        m.teleport(gb, MapKey::world(1), to, 0.0)
    })
    .await;
    b.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    let kalimdor = realm.maps.get(&realm, MapKey::world(1));
    assert!(pending(&kalimdor, gb).await.is_empty());
}

/// A hunter, her wolf, and her client moving it (Eyes of the Beast).
async fn hunter_moving_her_pet() -> (
    std::sync::Arc<crate::game::Realm>,
    Client,
    Guid,
    Guid,
    MapHandle,
) {
    let realm = crate::game::harness::realm_with(world());
    let mut a = Client::connect(&realm, "H");
    let h = a.enter("Hunter", Race::Dwarf, Class::Hunter).await;
    // Its login's own active mover handled first.
    handled(&mut a).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let pet = on(&map, move |m| {
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&h).unwrap();
        crate::game::map::stats::set_level(p, &data, 20);
        let at = p.position();
        let pet = crate::game::map::pets::testing::spawn(
            m,
            h,
            WOLF,
            crate::game::map::pets::Kind::Hunter,
            20,
            at,
            0.0,
        );
        m.players.get_mut(&h).unwrap().owned.pet = Some(pet);
        crate::game::map::pets::sync_record(m, h);
        // Not unhappy enough to run away meanwhile.
        let later = m.now + Duration::from_secs(3600);
        if let Some(s) = crate::game::map::pets::state_mut(m, pet) {
            s.loyalty_at = later;
            s.happiness_at = later;
        }
        let eyes = m.realm.data.spells.get(EYES).cloned().unwrap();
        auras::apply(m, h, pet, &eyes, 0, 0);
        crate::game::map::pets::set_mover(m, h, pet);
        assert_eq!(m.players[&h].owned.mover, Some(pet), "she moves it");
        pet
    })
    .await;
    (realm, a, h, pet, map)
}

/// A launch goes to the client that moves the unit: a pet its master's
/// client moves is launched by that client, its acknowledgement for the
/// pet settles it and those who see it watch it fly; a creature no client
/// moves is sent no launch at all (Thistle Tea's "supports possessed units
/// and leaves ordinary creature movement to the server"; mangos's
/// `KnockBackFrom` to `GetClientControlling`).
#[tokio::test]
async fn a_launch_goes_to_the_client_that_moves_the_unit() {
    let (realm, mut a, h, pet, map) = hunter_moving_her_pet().await;
    // Bob beside her, where dwarves start.
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Dwarf, Class::Warrior).await;
    b.sees(pet).await;
    let free = on(&map, move |m| {
        let free = spawn(m, h, BRUTE, 10, 8.0);
        crate::game::map::spells::cast_triggered(m, h, KNOCK, Some(pet));
        crate::game::map::spells::cast_triggered(m, h, KNOCK, Some(free));
        free
    })
    .await;
    let body = a
        .wait_frame("SMSG_MOVE_KNOCK_BACK", 0x0EF, |_| true)
        .await;
    let mut i = 0;
    assert_eq!(packed_guid(&body, &mut i), Some(pet), "the pet's launch");
    let counter = u32::from_le_bytes(body[i..i + 4].try_into().unwrap());
    let f = |n: usize| f32::from_le_bytes(body[i + 4 + 4 * n..i + 8 + 4 * n].try_into().unwrap());
    let impulse = [f(0), f(1), f(2), f(3)];
    let launches = |c: &mut Client| {
        c.captured()
            .iter()
            .filter(|f| u16::from_le_bytes([f[2], f[3]]) == 0x0EF)
            .count()
    };
    tokio::time::sleep(NONE_WITHIN).await;
    assert_eq!(launches(&mut a), 1, "none for the creature no client moves");
    assert_eq!(launches(&mut b), 0);
    let _ = free;
    let at = on(&map, move |m| m.creature(pet).unwrap().position).await;
    a.send(launched(pet, counter, impulse, at)).await;
    b.wait_frame("the pet flies", crate::game::map::misc::MSG_MOVE_KNOCK_BACK, |b| {
        let mut i = 0;
        packed_guid(b, &mut i) == Some(pet)
    })
    .await;
    assert!(pending(&map, h).await.is_empty());
}

/// A hunter's pet she moves (Eyes of the Beast), feared: her client is
/// told it no longer moves it, what it sends for it moves nothing, and
/// she has it back when the fear goes (Thistle Tea's "possessed creatures
/// reject input and notify the controller"; mangos's
/// `AbstractRandomMovementGenerator`: `UpdateClientControl`).
#[tokio::test]
async fn a_feared_unit_is_taken_from_the_client_that_moves_it() {
    let (_realm, mut a, h, pet, map) = hunter_moving_her_pet().await;
    let fear = on(&map, |m| m.realm.data.spells.get(FEAR).cloned().unwrap()).await;
    on(&map, move |m| auras::apply(m, h, pet, &fear, 0, 0)).await;
    let control = move |m: &ServerOpcodeMessage| match m {
        ServerOpcodeMessage::SMSG_CLIENT_CONTROL_UPDATE(c) if c.guid == pet => {
            Some(c.allow_movement)
        }
        _ => None,
    };
    // (Its possession granted her client the pet first.)
    a.wait_for("the pet taken from her", |m| {
        control(m).filter(|allow| !allow)
    })
    .await;
    a.clear();
    let before = on(&map, move |m| m.creature(pet).unwrap().position).await;
    let away = Vector3d {
        x: before.x + 3.0,
        ..before
    };
    a.send(MSG_MOVE_HEARTBEAT_Client { info: at(away) }).await;
    handled(&mut a).await;
    let after = on(&map, move |m| m.creature(pet).unwrap().position).await;
    assert_ne!(after, away, "her client moves it no more");
    on(&map, move |m| auras::remove_spell(m, pet, FEAR, None)).await;
    assert!(a.wait_for("and back", control).await);
}

/// A hunter teleported far from her pet within the map: it goes, and comes
/// back at her side at the acknowledgement of that teleport, not at a
/// stale one (mangos's `TeleportTo` leaving a pet more than 50 yards away,
/// `ResummonPetTemporaryUnSummonedIfAny` at `MSG_MOVE_TELEPORT_ACK`;
/// Thistle Tea's "restores a suspended pet after the matching teleport
/// acknowledgement", "does not restore a pet for a stale teleport
/// acknowledgement"). A short teleport keeps it out.
#[tokio::test]
async fn a_pet_left_by_a_teleport_comes_back_at_its_acknowledgement() {
    let realm = crate::game::harness::realm_with(world());
    let mut a = Client::connect(&realm, "H");
    let h = a.enter("Hunter", Race::Dwarf, Class::Hunter).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let pet = on(&map, move |m| {
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&h).unwrap();
        crate::game::map::stats::set_level(p, &data, 20);
        let at = p.position();
        let pet = crate::game::map::pets::testing::spawn(
            m,
            h,
            WOLF,
            crate::game::map::pets::Kind::Hunter,
            20,
            at,
            0.0,
        );
        m.players.get_mut(&h).unwrap().owned.pet = Some(pet);
        crate::game::map::pets::sync_record(m, h);
        pet
    })
    .await;
    let start = position(&map, h).await;
    let out = move |m: &crate::game::map::Map| {
        m.players[&h].owned.pet.filter(|p| m.creature(*p).is_some())
    };
    // Near: it stays.
    let near = Vector3d {
        x: start.x + 20.0,
        ..start
    };
    on(&map, move |m| m.teleport(h, m.key, near, 0.0)).await;
    assert_eq!(on(&map, move |m| out(m)).await, Some(pet), "a short one");
    a.wait_for("the teleport", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: h,
        movement_counter: a.teleport_order(),
        time: Default::default(),
    })
    .await;
    handled(&mut a).await;
    // Far: it goes.
    let far = Vector3d {
        x: start.x + 100.0,
        ..start
    };
    on(&map, move |m| m.teleport(h, m.key, far, 0.0)).await;
    a.wait_for("the teleport", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    assert_eq!(on(&map, move |m| out(m)).await, None, "left behind");
    // A stale acknowledgement: it stays away.
    a.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: h,
        movement_counter: a.teleport_order().wrapping_sub(1),
        time: Default::default(),
    })
    .await;
    handled(&mut a).await;
    let updates = on(&map, |m| m.stats.updates).await;
    while on(&map, |m| m.stats.updates).await < updates + 3 {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(on(&map, move |m| out(m)).await, None, "not for a stale one");
    // Its own: back at her side.
    a.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: h,
        movement_counter: a.teleport_order(),
        time: Default::default(),
    })
    .await;
    let mut back = None;
    for _ in 0..100 {
        back = on(&map, move |m| out(m).and_then(|p| m.position_of(p))).await;
        if back.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let back = back.expect("the pet is back");
    assert!(
        crate::game::map::combat::dist(back, far) < 10.0,
        "at her side: {back:?}"
    );
}
