//! Round 7's ground under a player, ported from Thistle Tea's
//! `test/game/core/movement/{falling,safe_position,control_movement}_test.exs`
//! (docs/parity/matrix.csv): a fall measured on a transport's deck, the
//! last safe ground Stuck goes back to, and a corpse that doesn't walk.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::MapHandle;
use crate::game::tests_p3_combat::{START, data, setup_with};
use crate::game::tests_p3_entity::{NONE_WITHIN, on, spell};
use crate::game::tests_p3_player2_ammo::handled;

const STUCK: u32 = 97_740;

fn world() -> GameData {
    let mut d = data();
    d.spells.spells.insert(
        STUCK,
        spell(
            STUCK,
            &[crate::game::data::spells::SpellEffect {
                effect: crate::game::map::spells::EFFECT_STUCK,
                target_a: 1,
                ..Default::default()
            }],
        ),
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

fn east(dx: f32) -> Vector3d {
    Vector3d {
        x: START.0 + dx,
        ..start()
    }
}

fn info(flags: MovementInfo_MovementFlags, position: Vector3d, fall_ms: u32) -> MovementInfo {
    MovementInfo {
        flags,
        timestamp: 0,
        position,
        orientation: 0.0,
        fall_time: fall_ms,
    }
}

/// Aboard `transport` (none: zero), its deck `deck` yards up.
fn aboard(
    flags: MovementInfo_MovementFlags,
    transport: u64,
    deck: f32,
) -> MovementInfo_MovementFlags {
    if transport == 0 {
        return flags;
    }
    flags.set_on_transport(MovementInfo_MovementFlags_OnTransport {
        transport: TransportInfo {
            guid: Guid::new(transport),
            position: Vector3d {
                x: 1.0,
                y: 0.0,
                z: deck,
            },
            orientation: 0.0,
        },
    })
}

fn hurt(m: &ServerOpcodeMessage) -> Option<u32> {
    match m {
        ServerOpcodeMessage::SMSG_ENVIRONMENTAL_DAMAGE_LOG(d) => Some(d.damage),
        _ => None,
    }
}

const BOAT: u64 = 0x1FC0_0000_0000_0007;
const OTHER_BOAT: u64 = 0x1FC0_0000_0000_0008;

/// A fall aboard is measured on the deck: from 30 yards up it to the deck
/// hurts as 30 yards do, wherever the world has the player; one that lands
/// on another transport, or off it, is no fall from that deck (Thistle
/// Tea's "tracks transport-local height and ignores a change of
/// transport").
#[tokio::test]
async fn a_fall_aboard_is_measured_on_its_deck_and_ends_with_it() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let max = on(&map, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        let max = 1000;
        p.fields.set_u32(UNIT_FIELD_MAXHEALTH, max);
        p.fields.set_u32(UNIT_FIELD_HEALTH, max);
        max
    })
    .await;
    for (lands_on, hurts) in [(BOAT, true), (OTHER_BOAT, false), (0, false)] {
        on(&map, move |m| {
            let p = m.players.get_mut(&ga).unwrap();
            p.fields.set_u32(UNIT_FIELD_HEALTH, max);
            p.fall_from = None;
        })
        .await;
        let empty = MovementInfo_MovementFlags::empty;
        a.send(MSG_MOVE_HEARTBEAT_Client {
            info: info(aboard(empty(), BOAT, 30.0), start(), 0),
        })
        .await;
        a.send(MSG_MOVE_JUMP_Client {
            info: info(aboard(empty().set_fallingfar(), BOAT, 30.0), start(), 0),
        })
        .await;
        a.clear();
        a.send(MSG_MOVE_FALL_LAND_Client {
            info: info(aboard(empty(), lands_on, 0.0), start(), 2_000),
        })
        .await;
        handled(&mut a).await;
        let damage = a.gets(NONE_WITHIN, hurt).await;
        if hurts {
            // vmangos's 1.8% a yard, less a quarter: 297 of 1000.
            assert_eq!(damage, Some(297), "30 yards down the deck");
        } else {
            assert_eq!(damage, None, "landed on {lands_on:#x}");
        }
    }
}

async fn safe(map: &MapHandle, g: Guid) -> Option<(u32, Vector3d)> {
    on(map, move |m| {
        m.players[&g].safe_position.map(|(m, at, _)| (m, at))
    })
    .await
}

async fn step(a: &mut Client, flags: MovementInfo_MovementFlags, at: Vector3d) {
    a.send(MSG_MOVE_HEARTBEAT_Client {
        info: info(flags, at, 0),
    })
    .await;
    handled(a).await;
    // Not faster than a run (the speed check).
    tokio::time::sleep(Duration::from_millis(100)).await;
}

/// The last safe place is ground the player stood on, three yards or more
/// from the last: not in the air, swimming or aboard, nor a step from it
/// (Thistle Tea's "retains the last grounded position through falling,
/// swimming, and transport", the sampling of "samples moved ground
/// positions only when they match terrain height"). Stuck goes back to it,
/// not to one on another map ("does not recall an anchor from another
/// world").
#[tokio::test]
async fn the_last_safe_place_is_ground_stood_on_and_stuck_goes_back_there() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let empty = MovementInfo_MovementFlags::empty;
    step(&mut a, empty(), east(4.0)).await;
    assert_eq!(safe(&map, ga).await, Some((0, east(4.0))));
    for (what, flags) in [
        ("falling", empty().set_fallingfar()),
        ("jumping", empty().set_jumping(Default::default())),
        ("swimming", empty().set_swimming(Default::default())),
        ("aboard", aboard(empty(), BOAT, 1.0)),
    ] {
        step(&mut a, flags, east(8.0)).await;
        assert_eq!(safe(&map, ga).await, Some((0, east(4.0))), "{what}");
    }
    step(&mut a, empty(), east(9.0)).await;
    assert_eq!(safe(&map, ga).await, Some((0, east(9.0))), "ground again");
    step(&mut a, empty(), east(10.0)).await;
    assert_eq!(safe(&map, ga).await, Some((0, east(9.0))), "a step from it");
    // Stuck: back there; one remembered on another map, nowhere.
    on(&map, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        p.safe_position = p.safe_position.map(|(_, at, o)| (1, at, o));
        crate::game::map::spells::cast_triggered(m, ga, STUCK, Some(ga));
    })
    .await;
    let teleported = |m: &ServerOpcodeMessage| match m {
        ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(t) => Some(t.info.position),
        _ => None,
    };
    assert!(
        a.gets(NONE_WITHIN, teleported).await.is_none(),
        "another map's"
    );
    on(&map, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        p.safe_position = p.safe_position.map(|(_, at, o)| (0, at, o));
        crate::game::map::spells::cast_triggered(m, ga, STUCK, Some(ga));
    })
    .await;
    let back = a.wait_for("the teleport back", teleported).await;
    assert_eq!(back, east(9.0));
}

/// A corpse's client moving it moves nothing; a released ghost walks
/// (Thistle Tea's "ordinary movement cannot move a corpse but permits a
/// released ghost").
#[tokio::test]
async fn a_corpse_doesnt_walk_but_its_ghost_does() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    on(&map, move |m| crate::game::map::death::kill(m, ga)).await;
    let fell = on(&map, move |m| m.players[&ga].position()).await;
    let empty = MovementInfo_MovementFlags::empty;
    step(&mut a, empty().set_forward(), east(4.0)).await;
    assert_eq!(
        on(&map, move |m| m.players[&ga].position()).await,
        fell,
        "the corpse stays"
    );
    on(&map, move |m| crate::game::map::death::release(m, ga)).await;
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
    let ghost = on(&map, move |m| m.players[&ga].position()).await;
    let walked = Vector3d {
        x: ghost.x + 4.0,
        ..ghost
    };
    step(&mut a, empty().set_forward(), walked).await;
    let now = on(&map, move |m| m.players[&ga].position()).await;
    assert_eq!(now, walked, "the ghost walks");
}
