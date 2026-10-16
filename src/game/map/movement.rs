//! Movement: a client's movement packets update its player and are relayed
//! at once to every client that knows them; teleports, within a map and
//! between maps.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::conn::encode;
use crate::game::map::player::PendingTeleport;
use crate::game::map::{Map, grid, visibility};

/// The map enum for a map id; maps the protocol doesn't name are taken as
/// Eastern Kingdoms rather than refused.
pub fn map_enum(id: u32) -> wow_world_messages::vanilla::Map {
    wow_world_messages::vanilla::Map::try_from(id).unwrap_or_default()
}

type Relay = fn(Guid, MovementInfo) -> Option<Arc<[u8]>>;

macro_rules! relays {
    ($msg:expr, $($name:ident => $server:ident),* $(,)?) => {
        match $msg {
            $(ClientOpcodeMessage::$name(m) => Some((
                &m.info,
                (|guid, info| encode(&$server { guid, info })) as Relay,
            )),)*
            _ => None,
        }
    };
}

/// The movement a client packet carries, and how to relay it; `None` for
/// any other packet.
#[allow(clippy::result_large_err)] // Handing the packet back unmoved.
pub fn movement_of(msg: ClientOpcodeMessage) -> Result<(MovementInfo, Relay), ClientOpcodeMessage> {
    let moved = relays!(&msg,
        MSG_MOVE_START_FORWARD => MSG_MOVE_START_FORWARD_Server,
        MSG_MOVE_START_BACKWARD => MSG_MOVE_START_BACKWARD_Server,
        MSG_MOVE_STOP => MSG_MOVE_STOP_Server,
        MSG_MOVE_START_STRAFE_LEFT => MSG_MOVE_START_STRAFE_LEFT_Server,
        MSG_MOVE_START_STRAFE_RIGHT => MSG_MOVE_START_STRAFE_RIGHT_Server,
        MSG_MOVE_STOP_STRAFE => MSG_MOVE_STOP_STRAFE_Server,
        MSG_MOVE_JUMP => MSG_MOVE_JUMP_Server,
        MSG_MOVE_START_TURN_LEFT => MSG_MOVE_START_TURN_LEFT_Server,
        MSG_MOVE_START_TURN_RIGHT => MSG_MOVE_START_TURN_RIGHT_Server,
        MSG_MOVE_STOP_TURN => MSG_MOVE_STOP_TURN_Server,
        MSG_MOVE_START_PITCH_UP => MSG_MOVE_START_PITCH_UP_Server,
        MSG_MOVE_START_PITCH_DOWN => MSG_MOVE_START_PITCH_DOWN_Server,
        MSG_MOVE_STOP_PITCH => MSG_MOVE_STOP_PITCH_Server,
        MSG_MOVE_SET_RUN_MODE => MSG_MOVE_SET_RUN_MODE_Server,
        MSG_MOVE_SET_WALK_MODE => MSG_MOVE_SET_WALK_MODE_Server,
        MSG_MOVE_FALL_LAND => MSG_MOVE_FALL_LAND_Server,
        MSG_MOVE_START_SWIM => MSG_MOVE_START_SWIM_Server,
        MSG_MOVE_STOP_SWIM => MSG_MOVE_STOP_SWIM_Server,
        MSG_MOVE_SET_FACING => MSG_MOVE_SET_FACING_Server,
        MSG_MOVE_SET_PITCH => MSG_MOVE_SET_PITCH_Server,
        MSG_MOVE_HEARTBEAT => MSG_MOVE_HEARTBEAT_Server,
    );
    match moved {
        Some((info, relay)) => Ok((info.clone(), relay)),
        None => Err(msg),
    }
}

fn finite(info: &MovementInfo) -> bool {
    let p = info.position;
    p.x.is_finite()
        && p.y.is_finite()
        && p.z.is_finite()
        && info.orientation.is_finite()
        && p.x.abs() < 20_000.0
        && p.y.abs() < 20_000.0
        && p.z.abs() < 20_000.0
}

/// A movement packet from `guid`'s client.
pub fn on_move(map: &mut Map, guid: Guid, info: MovementInfo, relay: Relay) {
    let Some(p) = map.players.get_mut(&guid) else {
        return;
    };
    // Until the client has acknowledged a teleport, it is still sending
    // from where it was.
    if p.teleport.is_some() || p.awaiting_worldport || p.flight.is_some() || !finite(&info) {
        return;
    }
    if p.logout_at.is_some() {
        return;
    }
    let anchor = p.vis_anchor;
    let new_cell = grid::cell_of(info.position.x, info.position.y);
    let old_cell = p.cell;
    let before = p.movement.position;
    p.cell = new_cell;
    p.movement = info.clone();
    let position = info.position;
    if crate::game::map::combat::dist(before, position) > 0.1 {
        crate::game::map::spells::moved(map, guid);
    }
    map.grid.relocate(guid, old_cell, new_cell);
    visibility::moved(map, guid, anchor, position);
    if let Some(frame) = relay(guid, info) {
        map.send_to_observers(guid, &frame, false);
    }
}

/// Move a player within the map: the client is told, and its movement is
/// ignored until it acknowledges.
pub fn teleport_within(map: &mut Map, guid: Guid, position: Vector3d, orientation: f32) {
    let now = map.now;
    let Some(p) = map.players.get_mut(&guid) else {
        return;
    };
    p.movement.position = position;
    p.movement.orientation = orientation;
    p.movement.flags = MovementInfo_MovementFlags::empty();
    p.movement.fall_time = 0.0;
    p.teleport = Some(PendingTeleport { since: now });
    p.outbound.send(&MSG_MOVE_TELEPORT_ACK_Server {
        guid,
        movement_counter: 0,
        info: p.movement.clone(),
    });
    let old_cell = p.cell;
    let new_cell = grid::cell_of(position.x, position.y);
    p.cell = new_cell;
    map.grid.relocate(guid, old_cell, new_cell);
    map.visibility_dirty.insert(guid);
    // Observers still in range see the player jump there.
    if let Some(p) = map.players.get(&guid)
        && let Some(frame) = encode(&MSG_MOVE_HEARTBEAT_Server {
            guid,
            info: p.movement.clone(),
        })
    {
        map.send_to_observers(guid, &frame, false);
    }
}

/// `MSG_MOVE_TELEPORT_ACK`: the client is where it was sent.
pub fn teleport_ack(map: &mut Map, guid: Guid) {
    if let Some(p) = map.players.get_mut(&guid) {
        p.teleport = None;
    }
}

/// `MSG_MOVE_WORLDPORT_ACK`: the client has loaded the map it was sent to;
/// it gets itself, then the world around it.
pub fn worldport_ack(map: &mut Map, guid: Guid) {
    let Some(p) = map.players.get_mut(&guid) else {
        return;
    };
    if !p.awaiting_worldport {
        return;
    }
    p.awaiting_worldport = false;
    visibility::send_self(map, guid);
    map.visibility_dirty.insert(guid);
}

/// Handle the packets of the movement family other than movement itself.
pub fn handle(map: &mut Map, guid: Guid, msg: ClientOpcodeMessage) -> Option<ClientOpcodeMessage> {
    match msg {
        ClientOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_) => teleport_ack(map, guid),
        ClientOpcodeMessage::MSG_MOVE_WORLDPORT_ACK => worldport_ack(map, guid),
        ClientOpcodeMessage::CMSG_MOVE_FALL_RESET(m) => {
            let relay: Relay = |_, _| None;
            on_move(map, guid, m.info, relay);
        }
        ClientOpcodeMessage::CMSG_MOVE_NOT_ACTIVE_MOVER(_)
        | ClientOpcodeMessage::CMSG_SET_ACTIVE_MOVER(_)
        | ClientOpcodeMessage::CMSG_MOVE_TIME_SKIPPED(_)
        | ClientOpcodeMessage::CMSG_MOVE_SPLINE_DONE(_)
        | ClientOpcodeMessage::CMSG_FORCE_RUN_SPEED_CHANGE_ACK(_)
        | ClientOpcodeMessage::CMSG_FORCE_RUN_BACK_SPEED_CHANGE_ACK(_)
        | ClientOpcodeMessage::CMSG_FORCE_SWIM_SPEED_CHANGE_ACK(_)
        | ClientOpcodeMessage::CMSG_FORCE_SWIM_BACK_SPEED_CHANGE_ACK(_)
        | ClientOpcodeMessage::CMSG_FORCE_WALK_SPEED_CHANGE_ACK(_)
        | ClientOpcodeMessage::CMSG_FORCE_TURN_RATE_CHANGE_ACK(_)
        | ClientOpcodeMessage::CMSG_FORCE_MOVE_ROOT_ACK(_)
        | ClientOpcodeMessage::CMSG_FORCE_MOVE_UNROOT_ACK(_)
        | ClientOpcodeMessage::CMSG_MOVE_KNOCK_BACK_ACK(_)
        | ClientOpcodeMessage::CMSG_MOVE_HOVER_ACK(_)
        | ClientOpcodeMessage::CMSG_MOVE_FEATHER_FALL_ACK(_)
        | ClientOpcodeMessage::CMSG_MOVE_WATER_WALK_ACK(_) => {}
        other => return Some(other),
    }
    None
}
