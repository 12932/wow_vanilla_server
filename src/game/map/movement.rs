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

/// `SMSG_FORCE_MOVE_ROOT` or `SMSG_FORCE_MOVE_UNROOT`: 1.12's GUID is
/// packed (mangos-zero's SetRoot, mangos-classic's and Thistle Tea's).
pub fn root_frame(guid: Guid, root: bool) -> Arc<[u8]> {
    let frame = if root {
        encode(&SMSG_FORCE_MOVE_ROOT { guid, counter: 0 })
    } else {
        encode(&SMSG_FORCE_MOVE_UNROOT { guid, counter: 0 })
    };
    frame.expect("a root encodes")
}

/// The map enum for a map id; maps the protocol doesn't name are taken as
/// Eastern Kingdoms rather than refused.
pub fn map_enum(id: u32) -> wow_world_messages::vanilla::Map {
    wow_world_messages::vanilla::Map::try_from(id).unwrap_or_default()
}

pub type Relay = fn(Guid, MovementInfo) -> Option<Arc<[u8]>>;

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
    let in_world = p.x.is_finite()
        && p.y.is_finite()
        && p.z.is_finite()
        && info.orientation.is_finite()
        && p.x.abs() < 20_000.0
        && p.y.abs() < 20_000.0
        && p.z.abs() < 20_000.0;
    // A place on deck is on a deck: within 50 yards across and 100 up, as
    // mangos's `VerifyMovementInfo` has it (NaN is on none).
    in_world
        && info.flags.get_on_transport().is_none_or(|t| {
            let d = t.transport.position;
            d.x.abs() <= 50.0
                && d.y.abs() <= 50.0
                && d.z.abs() <= 100.0
                && t.transport.orientation.is_finite()
        })
}

/// A movement packet from `guid`'s client.
pub fn on_move(map: &mut Map, guid: Guid, info: MovementInfo, relay: Relay) {
    if crate::game::map::pets::move_possessed(map, guid, &info, relay) {
        return;
    }
    // Feared, confused, charmed or charging: the server moves it, not its
    // client.
    if crate::game::map::player_motion::driven(map, guid)
        || crate::game::map::charge::charging(map, guid)
    {
        return;
    }
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
    // Faster than the player can go: pulled back to where they were.
    let now = map.now;
    if let Some((last, at)) = p.move_check
        && !p.gm
        && info.flags.get_on_transport().is_none()
    {
        let dt = now.duration_since(at).as_secs_f32();
        let speed = p.speeds.run.max(p.speeds.swim).max(7.0);
        let allowed = speed * SPEED_TOLERANCE * (dt + LATENCY_ALLOWANCE) + 3.0;
        if horizontal(last, info.position) > allowed {
            let o = p.movement.orientation;
            tracing::debug!("{} moved too fast; pulled back", p.name);
            teleport_within(map, guid, last, o);
            return;
        }
    }
    p.move_check = Some((info.position, now));
    let fall = landing(p, &info);
    let anchor = p.vis_anchor;
    let new_cell = grid::cell_of(info.position.x, info.position.y);
    let old_cell = p.cell;
    let before = p.movement.position;
    let f = &info.flags;
    let was_swimming = p.movement.flags.get_swimming().is_some();
    let swimming = f.get_swimming().is_some();
    let translating =
        f.get_forward() || f.get_backward() || f.get_strafe_left() || f.get_strafe_right();
    let moved = translating || crate::game::map::combat::dist(before, info.position) > 0.0;
    let turned = f.get_turn_left()
        || f.get_turn_right()
        || (info.orientation - p.movement.orientation).abs() > f32::EPSILON;
    p.cell = new_cell;
    p.movement = info.clone();
    let position = info.position;
    if crate::game::map::combat::dist(before, position) > 0.1 {
        crate::game::map::spells::moved(map, guid);
        crate::game::map::ranged::moved(map, guid);
    }
    map.grid.relocate(guid, old_cell, new_cell);
    visibility::moved(map, guid, anchor, position);
    if let Some(frame) = relay(guid, info) {
        map.send_to_observers(guid, &frame, false);
    }
    body_moved(map, guid, moved, turned);
    // Into the water or out of it: the auras that ends (Thistle Tea's
    // `interrupt_water_auras`). The terrain's liquid ends them too, for a
    // client that doesn't say (`environment::update`).
    if was_swimming != swimming {
        use crate::game::map::auras::{INTERRUPT_ABOVE_WATER, INTERRUPT_UNDER_WATER};
        let mask = if swimming {
            INTERRUPT_UNDER_WATER
        } else {
            INTERRUPT_ABOVE_WATER
        };
        crate::game::map::auras::interrupt(map, guid, mask);
    }
    if let Some(drop) = fall {
        fall_damage(map, guid, drop);
    }
    remember_safe_position(map, guid);
}

/// How far a player moves before where it stands is remembered again, and
/// how near the ground it must be (Thistle Tea's `SafePosition`).
const SAFE_SAMPLE: f32 = 3.0;
const SAFE_SURFACE: f32 = 3.0;

/// A player on the ground (not in the air, the water or on a transport)
/// three yards from where it last stood: the place is remembered for
/// Stuck, if the navmesh has ground there.
fn remember_safe_position(map: &mut Map, g: Guid) {
    let map_id = map.key.map;
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let f = &p.movement.flags;
    let grounded = f.get_jumping().is_none()
        && !f.get_fallingfar()
        && f.get_swimming().is_none()
        && f.get_on_transport().is_none();
    let at = p.movement.position;
    let moved = p.safe_position.is_none_or(|(m, last, _)| {
        m != map_id || crate::game::map::combat::dist(last, at) >= SAFE_SAMPLE
    });
    if !grounded || !moved {
        return;
    }
    let o = p.movement.orientation;
    let on_ground = map.nav.as_mut().is_none_or(|n| {
        n.ground(at.x, at.y, at.z)
            .is_some_and(|z| (z - at.z).abs() <= SAFE_SURFACE)
    });
    if on_ground && let Some(p) = map.players.get_mut(&g) {
        p.safe_position = Some((map_id, at, o));
    }
}

/// A player moved or turned (Thistle Tea's `Emote.move` and movement
/// interrupts): moving ends an emote state, moving or turning stands them
/// up, and the auras moving or turning breaks go.
fn body_moved(map: &mut Map, g: Guid, moved: bool, turned: bool) {
    use crate::game::map::auras::{INTERRUPT_MOVE, INTERRUPT_NOT_SEATED, INTERRUPT_TURNING};
    if !moved && !turned {
        return;
    }
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    if moved && p.fields.get_u32(crate::game::fields::UNIT_NPC_EMOTESTATE) != 0 {
        p.fields
            .set_u32(crate::game::fields::UNIT_NPC_EMOTESTATE, 0);
    }
    // Sitting, sleeping or kneeling; not dead: up, and told so, as mangos's
    // SetStandState and Thistle Tea's `Emote.move` tell it.
    if !matches!(p.stand_state(), 0 | 7) && p.death == crate::game::map::death::Death::Alive {
        p.set_stand_state(0);
        p.outbound.send(&SMSG_STANDSTATE_UPDATE {
            state: UnitStandState::Stand,
        });
    }
    let mask = if moved {
        INTERRUPT_MOVE | INTERRUPT_TURNING | INTERRUPT_NOT_SEATED
    } else {
        INTERRUPT_TURNING
    };
    crate::game::map::auras::interrupt(map, g, mask);
}

/// How much faster than their speed a player may seem to go, and the
/// lag a packet may carry.
const SPEED_TOLERANCE: f32 = 1.25;
const LATENCY_ALLOWANCE: f32 = 0.5;
/// vmangos's safe fall: below this many yards a fall doesn't hurt.
const SAFE_FALL: f32 = 14.57;

fn horizontal(a: Vector3d, b: Vector3d) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

/// The least fall time (ms) a landing hurts after (vmangos's
/// `HandleFall`).
const HURTING_FALL_MS: u32 = 1229;

/// Where a fall is measured: on a transport, its deck's height and the
/// transport.
fn height(info: &MovementInfo) -> (f32, Guid) {
    match info.flags.get_on_transport() {
        Some(t) => (t.transport.position.z, t.transport.guid),
        None => (info.position.z, Guid::zero()),
    }
}

/// A fall's start is the highest point since leaving the ground (on a
/// transport, of its deck), as Thistle Tea's `Falling` tracks it; a landing
/// after a long fall (the falling-far flag seen, 1,229 ms in the air, on the
/// same transport) returns how far it was. Swimming ends a fall.
fn landing(p: &mut crate::game::map::player::Player, info: &MovementInfo) -> Option<f32> {
    let falling = info.flags.get_jumping().is_some() || info.flags.get_fallingfar();
    let (z, transport) = height(info);
    // Swimming ends a fall, and so does slowing it: what was fallen before
    // Slow Fall (or a hover) took hold doesn't count once it is gone
    // (Thistle Tea's `Falling` discards it).
    use crate::game::map::auras::{AURA_FEATHER_FALL, AURA_HOVER};
    if info.flags.get_swimming().is_some()
        || p.auras.has(AURA_FEATHER_FALL)
        || p.auras.has(AURA_HOVER)
    {
        p.fall_from = None;
        p.misc.fall_far = false;
        return None;
    }
    match (falling, p.fall_from) {
        (true, Some(top)) if p.misc.fall_transport == transport => {
            p.fall_from = Some(top.max(z));
            p.misc.fall_far |= info.flags.get_fallingfar();
            None
        }
        (true, _) => {
            let (was, _) = height(&p.movement);
            let start = if p.misc.fall_transport == transport || p.fall_from.is_none() {
                was.max(z)
            } else {
                z
            };
            p.fall_from = Some(start);
            p.misc.fall_transport = transport;
            p.misc.fall_far = info.flags.get_fallingfar();
            None
        }
        (false, Some(top)) => {
            p.fall_from = None;
            let far = std::mem::take(&mut p.misc.fall_far);
            let long = info.fall_time.to_bits() >= HURTING_FALL_MS;
            (far && long && p.misc.fall_transport == transport).then_some(top - z)
        }
        (false, None) => None,
    }
}

/// vmangos's fall damage, as Thistle Tea's `Falling`: 1.8% of maximum
/// health a yard past the safe fall, less a quarter, by physical damage
/// taken modifiers and at most all of it. Not for the dead, a god, a
/// flight, Slow Fall, hover or physical immunity.
fn fall_damage(map: &mut Map, g: Guid, drop: f32) {
    let Some(p) = map.players.get(&g) else {
        return;
    };
    use crate::game::map::auras::{
        AURA_FEATHER_FALL, AURA_HOVER, AURA_MOD_DAMAGE_PERCENT_TAKEN, AURA_SAFE_FALL,
    };
    if p.auras.has(AURA_FEATHER_FALL)
        || p.auras.has(AURA_HOVER)
        || p.auras.school_immune(1)
        || p.misc.god
        || p.flight.is_some()
        || p.death != crate::game::map::death::Death::Alive
    {
        return;
    }
    if drop < SAFE_FALL {
        return;
    }
    // Safe Fall takes yards off the drop.
    let drop = drop - p.auras.sum(AURA_SAFE_FALL).max(0) as f32;
    let max = p.fields.get_u32(crate::game::fields::UNIT_FIELD_MAXHEALTH) as f32;
    let taken = p.auras.product_mask(AURA_MOD_DAMAGE_PERCENT_TAKEN, 1);
    let damage = ((0.018 * drop - 0.2426) * max * taken).clamp(0.0, max) as u32;
    if damage == 0 {
        return;
    }
    if let Some(frame) = encode(&SMSG_ENVIRONMENTAL_DAMAGE_LOG {
        guid: g,
        damage_type: EnvironmentalDamageType::Fall,
        damage,
        absorb: 0,
        resist: 0,
    }) {
        map.send_to_observers(g, &frame, true);
    }
    crate::game::map::spells::pushback_by(map, g, g);
    crate::game::map::combat::deal_damage(map, g, g, damage);
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
    p.move_check = Some((position, now));
    p.fall_from = None;
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
    // Observers still in range see the player there at once:
    // `MSG_MOVE_TELEPORT`, the packed GUID and the movement, as mangos's
    // SendTeleportPacket and Thistle Tea send it (a heartbeat would have
    // them run there).
    if let Some(p) = map.players.get(&guid)
        && let Some(frame) = encode(&MSG_MOVE_TELEPORT_Server {
            player: guid,
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
    crate::game::map::pvp::arrived(map, guid);
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
    crate::game::map::death::resend_water_walk(map, guid);
    map.visibility_dirty.insert(guid);
    crate::game::map::battleground::arrived(map, guid);
    crate::game::map::eviction::arrived(map, guid);
    crate::game::map::pvp::arrived(map, guid);
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
        // Whose movement the client sends: its own, or a possessed unit's.
        ClientOpcodeMessage::CMSG_SET_ACTIVE_MOVER(m) => {
            crate::game::map::pets::set_mover(map, guid, m.guid)
        }
        ClientOpcodeMessage::CMSG_MOVE_NOT_ACTIVE_MOVER(m) => {
            crate::game::map::pets::not_mover(map, guid, m.old_mover)
        }
        ClientOpcodeMessage::CMSG_MOVE_TIME_SKIPPED(_)
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
