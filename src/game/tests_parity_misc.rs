//! Stage 2g's world against Thistle Tea, through the protocol: a speed
//! change told to those watching once, at the change (a ghost's release, a
//! revive and `.speed` among them); water's interrupts by the terrain's
//! liquid; and Bind's Hearthstone given by the spell.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::GameData;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::harness::{self, Client};
use crate::game::map::misc::{MSG_MOVE_SET_RUN_SPEED, MSG_MOVE_SET_SWIM_SPEED};
use crate::game::map::{Map, MapKey};

/// Northshire, where humans start.
const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

const SPRINT: u32 = 900_100;

fn aura(id: u32, aura: u32, points: i32) -> SpellTemplate {
    let mut effects = [SpellEffect::default(); 3];
    effects[0] = SpellEffect {
        effect: 6,
        aura,
        base_points: points - 1,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    SpellTemplate {
        id,
        name: format!("Spell {id}"),
        effects,
        equipped_item_class: -1,
        ..Default::default()
    }
}

fn data() -> GameData {
    let mut d = GameData::empty();
    d.spells.spells.insert(SPRINT, aura(SPRINT, 31, 50));
    d
}

async fn on_map<T: Send + 'static>(
    realm: &Arc<Realm>,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    realm
        .maps
        .get(realm, MapKey::world(0))
        .query(f)
        .await
        .expect("the map answers")
}

/// The speed an `MSG_MOVE_SET_*_SPEED` carries, last.
fn speed_of(frame: &[u8]) -> f32 {
    f32::from_le_bytes(frame[frame.len() - 4..].try_into().unwrap())
}

/// The guid an observer's `MSG_MOVE_SET_*_SPEED` is for, packed first.
fn speed_for(frame: &[u8]) -> Guid {
    let mask = frame[0];
    let mut raw = [0u8; 8];
    let mut at = 1;
    for (i, byte) in raw.iter_mut().enumerate() {
        if mask & (1 << i) != 0 {
            *byte = frame[at];
            at += 1;
        }
    }
    Guid::new(u64::from_le_bytes(raw))
}

/// The speeds `watcher` is told of for `of` by `opcode`, waiting for one.
async fn told_speed(watcher: &mut Client, of: Guid, opcode: u16) -> f32 {
    let frame = watcher
        .wait_frame("MSG_MOVE_SET_*_SPEED", opcode, |f| speed_for(f) == of)
        .await;
    speed_of(&frame)
}

/// Whether any `MSG_MOVE_SET_*_SPEED` of `opcodes` reached `watcher`
/// before `of`'s next heartbeat, which `mover` sends now: the map handles
/// a client's packets in order, so what an earlier one sent is in first.
async fn told_before_heartbeat(
    mover: &Client,
    watcher: &mut Client,
    of: Guid,
    opcodes: &[u16],
) -> bool {
    mover.heartbeat(START.0, START.1, START.2).await;
    watcher
        .wait_for("the heartbeat", |m| match m {
            ServerOpcodeMessage::MSG_MOVE_HEARTBEAT(h) if h.guid == of => Some(()),
            _ => None,
        })
        .await;
    // The observer speed packets aren't wow_messages', so come raw.
    watcher.raw.iter().any(|f| {
        let opcode = u16::from_le_bytes([f[2], f[3]]);
        opcodes.contains(&opcode) && speed_for(&f[4..]) == of
    })
}

fn here() -> MovementInfo {
    MovementInfo {
        flags: MovementInfo_MovementFlags::empty(),
        timestamp: 10,
        position: Vector3d {
            x: START.0,
            y: START.1,
            z: START.2,
        },
        orientation: 0.0,
        fall_time: 0.0,
    }
}

#[tokio::test]
async fn a_speed_change_is_told_once_at_the_change_not_at_the_acknowledgement() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    b.sees(ga).await;
    b.clear();
    on_map(&realm, move |m| {
        crate::game::map::spells::cast_triggered(m, ga, SPRINT, Some(ga));
        crate::game::map::auras::recalc(m, ga);
    })
    .await;
    let speed = told_speed(&mut b, ga, MSG_MOVE_SET_RUN_SPEED).await;
    assert!((speed - 10.5).abs() < 0.01, "{speed}");
    b.raw.clear();
    // Alice's client takes it up; Bob hears nothing more of it (nor of a
    // speed her client makes up).
    a.send(CMSG_FORCE_RUN_SPEED_CHANGE_ACK {
        guid: ga,
        counter: 0,
        info: here(),
        new_speed: 10.5,
    })
    .await;
    a.send(CMSG_FORCE_SWIM_SPEED_CHANGE_ACK {
        guid: ga,
        counter: 0,
        info: here(),
        new_speed: 50.0,
    })
    .await;
    let again = told_before_heartbeat(
        &a,
        &mut b,
        ga,
        &[MSG_MOVE_SET_RUN_SPEED, MSG_MOVE_SET_SWIM_SPEED],
    )
    .await;
    assert!(!again, "the acknowledgement isn't passed on");
}

/// Release the dead `who`, and take up the teleport to where the ghost
/// goes (where they are, without graveyards).
async fn released(who: &mut Client, g: Guid) {
    who.send(CMSG_REPOP_REQUEST {}).await;
    who.wait_for("the teleport to the graveyard", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    who.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: g,
        movement_counter: 0,
        time: Default::default(),
    })
    .await;
}

#[tokio::test]
async fn a_ghosts_release_and_revive_tell_their_speeds_at_the_change() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    b.sees(ga).await;
    // Bob is a ghost already, so he sees hers.
    on_map(&realm, move |m| crate::game::map::death::kill(m, gb)).await;
    released(&mut b, gb).await;
    on_map(&realm, move |m| crate::game::map::death::kill(m, ga)).await;
    b.clear();
    // Her release: the ghost's quarter faster, told without her client
    // acknowledging anything.
    released(&mut a, ga).await;
    let run = told_speed(&mut b, ga, MSG_MOVE_SET_RUN_SPEED).await;
    assert!((run - 8.75).abs() < 0.01, "{run}");
    let swim = told_speed(&mut b, ga, MSG_MOVE_SET_SWIM_SPEED).await;
    assert!((swim - 4.722_222 * 1.25).abs() < 0.01, "{swim}");
    // Her revive: back to a run.
    on_map(&realm, move |m| {
        crate::game::map::death::resurrect(m, ga, 1.0);
    })
    .await;
    let run = told_speed(&mut b, ga, MSG_MOVE_SET_RUN_SPEED).await;
    assert!((run - 7.0).abs() < 0.01, "{run}");
    let swim = told_speed(&mut b, ga, MSG_MOVE_SET_SWIM_SPEED).await;
    assert!((swim - 4.722_222).abs() < 0.01, "{swim}");
}

#[tokio::test]
async fn the_speed_commands_tell_those_watching_at_the_change() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    b.sees(ga).await;
    b.clear();
    a.say(".speed 2").await;
    let run = told_speed(&mut b, ga, MSG_MOVE_SET_RUN_SPEED).await;
    assert!((run - 14.0).abs() < 0.01, "{run}");
    a.say(".modify speed 3").await;
    let run = told_speed(&mut b, ga, MSG_MOVE_SET_RUN_SPEED).await;
    assert!((run - 21.0).abs() < 0.01, "{run}");
}
