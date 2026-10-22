//! The client's acknowledgements of forced movements (roots, speeds, water
//! walking, Slow Fall, hovering, launches, teleports). Each settles the
//! order it names ([`Outbound::settle`](crate::game::conn::Outbound::settle))
//! if one waits under its counter for the same unit and the same change;
//! any other is stale, repeated or forged, and changes nothing (Thistle
//! Tea's `MovementControl`; mangos-classic counts its orders the same way,
//! `GetOrderCounter`). A settled root, unroot or launch carries where the
//! client is, taken as a movement packet is (mangos's
//! `ProcessMovementInfo`: not while a teleport is under way, nor where the
//! server moves the player), and those who see it are told.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::conn::{Order, SpeedKind, encode};
use crate::game::data::items::Body;
use crate::game::map::Map;
use crate::game::map::misc::{
    MSG_MOVE_KNOCK_BACK, MSG_MOVE_SET_RUN_BACK_SPEED, MSG_MOVE_SET_RUN_SPEED,
    MSG_MOVE_SET_SWIM_BACK_SPEED, MSG_MOVE_SET_SWIM_SPEED,
};
use crate::game::map::movement::{self, Relay};

/// How near a speed or a launch's impulse must come back to what was sent
/// (Thistle Tea's 0.01, mangos's speed check): the client's float
/// arithmetic, no more.
const CLOSE: f32 = 0.01;

/// The acknowledgements; anything else is handed back.
pub fn handle(map: &mut Map, g: Guid, msg: ClientOpcodeMessage) -> Option<ClientOpcodeMessage> {
    use ClientOpcodeMessage as C;
    match msg {
        C::CMSG_FORCE_MOVE_ROOT_ACK(m) => root(map, g, m.guid, m.movement_counter, m.info, true),
        C::CMSG_FORCE_MOVE_UNROOT_ACK(m) => root(map, g, m.guid, m.movement_counter, m.info, false),
        C::CMSG_FORCE_RUN_SPEED_CHANGE_ACK(m) => speed(
            map,
            g,
            m.guid,
            m.counter,
            m.info,
            SpeedKind::Run,
            m.new_speed,
        ),
        C::CMSG_FORCE_RUN_BACK_SPEED_CHANGE_ACK(m) => speed(
            map,
            g,
            m.guid,
            m.movement_counter,
            m.info,
            SpeedKind::RunBack,
            m.new_speed,
        ),
        C::CMSG_FORCE_SWIM_SPEED_CHANGE_ACK(m) => speed(
            map,
            g,
            m.guid,
            m.counter,
            m.info,
            SpeedKind::Swim,
            m.new_speed,
        ),
        C::CMSG_FORCE_SWIM_BACK_SPEED_CHANGE_ACK(m) => speed(
            map,
            g,
            m.guid,
            m.counter,
            m.info,
            SpeedKind::SwimBack,
            m.new_speed,
        ),
        C::CMSG_FORCE_WALK_SPEED_CHANGE_ACK(m) => speed(
            map,
            g,
            m.guid,
            m.counter,
            m.info,
            SpeedKind::Walk,
            m.new_speed,
        ),
        C::CMSG_FORCE_TURN_RATE_CHANGE_ACK(m) => speed(
            map,
            g,
            m.guid,
            m.counter,
            m.info,
            SpeedKind::TurnRate,
            m.new_speed,
        ),
        // The apply word is any nonzero value for on: build 5875 sends
        // 1.0's bits (Thistle Tea's "nonzero apply word").
        C::CMSG_MOVE_WATER_WALK_ACK(m) => settle(
            map,
            g,
            m.movement_counter,
            Order::WaterWalk {
                guid: m.guid,
                on: m.apply != 0,
            },
        ),
        C::CMSG_MOVE_FEATHER_FALL_ACK(m) => settle(
            map,
            g,
            m.movement_counter,
            Order::FeatherFall {
                guid: m.guid,
                on: m.apply != 0,
            },
        ),
        C::CMSG_MOVE_HOVER_ACK(m) => settle(
            map,
            g,
            m.counter,
            Order::Hover {
                guid: m.guid,
                on: m.is_applied != 0,
            },
        ),
        C::CMSG_MOVE_KNOCK_BACK_ACK(m) => knock_back(map, g, m.guid, m.counter, m.info),
        C::MSG_MOVE_TELEPORT_ACK(m) => {
            if m.guid == g
                && taken(map, g, m.movement_counter, |o| {
                    *o == Order::Teleport { guid: g }
                })
            {
                movement::teleport_ack(map, g);
            }
        }
        other => return Some(other),
    }
    None
}

/// Whether `g`'s client had the order `counter` waiting, and it `matches`:
/// settled if so.
fn taken(map: &Map, g: Guid, counter: u32, matches: impl FnOnce(&Order) -> bool) -> bool {
    map.players
        .get(&g)
        .is_some_and(|p| p.outbound.settle(counter, matches).is_some())
}

/// An acknowledgement that only settles its order.
fn settle(map: &Map, g: Guid, counter: u32, expected: Order) {
    taken(map, g, counter, |o| *o == expected);
}

/// A root or unroot taken: the client is where it says, and those who see
/// it are told (`MSG_MOVE_ROOT`, `MSG_MOVE_UNROOT`), as mangos's
/// `HandleMoveRootAck` tells them. A possessed unit's settles its order and
/// no more (Thistle Tea's controlled change); the unit moves by its own
/// packets.
fn root(map: &mut Map, g: Guid, unit: Guid, counter: u32, info: MovementInfo, root: bool) {
    if !taken(map, g, counter, |o| *o == Order::Root { guid: unit, root }) || unit != g {
        return;
    }
    let relay: Relay = if root {
        |player, info| encode(&MSG_MOVE_ROOT_Server { player, info })
    } else {
        |player, info| encode(&MSG_MOVE_UNROOT_Server { player, info })
    };
    let came = if root {
        movement::Came::RootAck
    } else {
        movement::Came::Move
    };
    movement::moved_self_as(map, g, info, relay, came);
}

/// A forced speed taken, at the speed it was sent at. Those who see a unit
/// the player's client moves (Mind Control, Eyes of the Beast) hear of its
/// new speed now, as mangos-classic tells them; a player's own they heard
/// at the change (`misc::speed_to_observers`).
fn speed(
    map: &mut Map,
    g: Guid,
    unit: Guid,
    counter: u32,
    info: MovementInfo,
    kind: SpeedKind,
    speed: f32,
) {
    let settled = taken(map, g, counter, |o| {
        matches!(*o, Order::Speed { guid, kind: k, speed: sent }
            if guid == unit && k == kind && (sent - speed).abs() < CLOSE)
    });
    let moved = map
        .players
        .get(&g)
        .is_some_and(|p| unit != g && p.owned.mover == Some(unit));
    if !settled || !moved {
        return;
    }
    let opcode = match kind {
        SpeedKind::Run => MSG_MOVE_SET_RUN_SPEED,
        SpeedKind::RunBack => MSG_MOVE_SET_RUN_BACK_SPEED,
        SpeedKind::Swim => MSG_MOVE_SET_SWIM_SPEED,
        SpeedKind::SwimBack => MSG_MOVE_SET_SWIM_BACK_SPEED,
        SpeedKind::Walk | SpeedKind::TurnRate => return,
    };
    crate::game::map::misc::relay(map, unit, info, opcode, &speed.to_le_bytes());
}

/// A launch taken: the client's jump is the impulse it was sent (its
/// direction and speeds, the jumping flag on), once. The client is where
/// it says, and those who see it watch it fly (`MSG_MOVE_KNOCK_BACK`):
/// the player, or the unit the player's client moves.
fn knock_back(map: &mut Map, g: Guid, unit: Guid, counter: u32, info: MovementInfo) {
    let Some(j) = info.flags.get_jumping() else {
        return;
    };
    let got = [j.cos_angle, j.sin_angle, j.xy_speed, j.z_speed];
    let settled = taken(map, g, counter, |o| {
        matches!(o, Order::KnockBack { guid, impulse }
            if *guid == unit && impulse.iter().zip(got).all(|(a, b)| (a - b).abs() < CLOSE))
    });
    if !settled {
        return;
    }
    if unit == g {
        // In its flight, it goes at the launch's speed (mangos's
        // `IsLaunched`, until it lands; wvs's, no longer than its arc
        // and a fall as it would).
        let now = map.now;
        if let Some(p) = map.players.get_mut(&g) {
            crate::game::map::move_guard::launched(p, &j, info.position, now);
        }
        movement::moved_self(map, g, info, launched);
    } else if map
        .players
        .get(&g)
        .is_some_and(|p| p.owned.mover == Some(unit))
    {
        // The possessed unit's flight, as a player's own.
        let now = map.now;
        if let Some(v) = map.players.get_mut(&unit) {
            crate::game::map::move_guard::launched(v, &j, info.position, now);
        } else if let Some(s) = crate::game::map::pets::state_mut(map, unit) {
            crate::game::map::move_guard::launched_with(&mut s.move_guard, &j, info.position, now);
        }
        crate::game::map::pets::move_possessed(map, g, &info, launched);
    }
}

/// `MSG_MOVE_KNOCK_BACK`: the movement, then the jump's direction and
/// speeds.
fn launched(guid: Guid, info: MovementInfo) -> Option<Arc<[u8]>> {
    let j = info.flags.get_jumping()?;
    let mut b = Body(crate::game::map::misc::movement_bytes(guid, &info));
    b.f32(j.cos_angle)
        .f32(j.sin_angle)
        .f32(j.xy_speed)
        .f32(j.z_speed);
    Some(b.frame(MSG_MOVE_KNOCK_BACK))
}
