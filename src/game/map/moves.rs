//! A burst's movement packets, worked out across threads (`[world]
//! parallel_moves`): the map's run of events keeps the movement it is sent
//! (its players' own, a heartbeat or a step) until something else comes or
//! the burst ends, and then works it out at once.
//!
//! A packet of one mover changes that mover alone, but for what it tells
//! the map: the grid, visibility's marks, the broadcast to those who see
//! it, the safe place remembered. So each mover's packets are checked and
//! taken in their order, across the threads, each mover's player its own
//! task's (the gates, the root and the speed check, the fall, the move
//! itself, the relay encoded: [`step`]); then, on the map's thread, in the
//! order the packets came, what each told the map is done. A packet that
//! would do more than that (put back or pulled back, a cast or a shot it
//! breaks, an aura it ends, a stand, a fall that hurts, a liquid left, the
//! void) isn't taken across the threads: from the first such packet of
//! the burst on, every packet is handled as it would have been alone, and
//! whatever a mover did past it across the threads is undone (each
//! packet's step changes its player's few movement fields, kept before
//! it). Before that packet nothing has been done that a mover's check
//! reads but its own player, so every packet comes out as it would have
//! one by one: the same verdicts, the same state, the same packets in the
//! same order to each client (`tests_moves`).

use std::sync::Arc;
use std::time::Instant;

use rayon::prelude::*;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::config::AnticheatConfig;
use crate::game::map::grid::{self, CellKey};
use crate::game::map::move_guard::{self, MoveGuard};
use crate::game::map::movement::{self, Relay};
use crate::game::map::player::Player;
use crate::game::map::{Map, Room, visibility};

/// Fewer packets than this in a burst are handled one by one.
pub const PARALLEL_FROM: usize = 64;

/// A movement packet kept for the end of the burst.
pub struct Move {
    pub guid: Guid,
    pub info: MovementInfo,
    pub relay: Relay,
    /// The session's share of the mailbox, given back once it is done.
    pub room: Room,
    /// Its opcode's name, for Tracy (while it is on).
    pub name: &'static str,
}

/// What a packet's step left for the map to do.
#[derive(Clone)]
struct Taken {
    old_cell: CellKey,
    new_cell: CellKey,
    anchor: Vector3d,
    info: MovementInfo,
    frame: Option<Arc<[u8]>>,
}

/// What a packet came to across the threads.
enum Step {
    /// Nothing for the map: a gate shut, or held under a root.
    Ignored,
    Taken(Taken),
    /// More than the mover's own: handled as it would be alone.
    Alone,
}

/// The fields a step changes, kept before it, to be put back.
#[derive(Clone, Copy)]
struct Kept {
    guard: MoveGuard,
    move_check: Option<(Vector3d, Instant)>,
    fall_from: Option<f32>,
    fall_far: bool,
    fall_transport: Guid,
    cell: CellKey,
    movement: MovementInfo,
}

impl Kept {
    fn of(p: &Player) -> Self {
        Self {
            guard: p.misc.move_guard,
            move_check: p.move_check,
            fall_from: p.fall_from,
            fall_far: p.misc.fall_far,
            fall_transport: p.misc.fall_transport,
            cell: p.cell,
            movement: p.movement,
        }
    }

    fn restore(self, p: &mut Player) {
        p.misc.move_guard = self.guard;
        p.move_check = self.move_check;
        p.fall_from = self.fall_from;
        p.misc.fall_far = self.fall_far;
        p.misc.fall_transport = self.fall_transport;
        p.cell = self.cell;
        p.movement = self.movement;
    }
}

/// Whether the burst's movement may be worked out across the threads at
/// all: off by its setting, with distant observers sent a share of the
/// heartbeats (which measures from where everyone is at each packet), or
/// with a test's faults about.
pub fn parallel(map: &Map) -> bool {
    #[cfg(test)]
    if map.faults.any() {
        return false;
    }
    map.cfg.parallel_moves && !(map.cfg.distant_move_yards > 0.0 && map.cfg.distant_move_every > 1)
}

/// `event`, kept for the burst's end if it is a player's own movement
/// packet and the burst's movement may be worked out across the threads:
/// `Ok(None)` if there was nothing to keep (a heartbeat's slot already
/// emptied, a packet from a session taken over); the event back otherwise.
#[allow(clippy::result_large_err)] // The event handed back unmoved.
pub fn keep(
    map: &mut Map,
    event: crate::game::map::MapEvent,
) -> Result<Option<Move>, crate::game::map::MapEvent> {
    use crate::game::map::MapEvent;
    if !parallel(map) {
        return Err(event);
    }
    match event {
        MapEvent::Packet(guid, msg, from, room) if movement::is_movement(&msg) => {
            map.stats.events += 1;
            if !map.from_their_session(guid, &from) {
                return Ok(None);
            }
            let name = if crate::tracy::on() {
                crate::game::map::packet_name(&msg)
            } else {
                ""
            };
            let Ok((info, relay)) = movement::movement_of(msg) else {
                return Ok(None);
            };
            Ok(Some(Move {
                guid,
                info,
                relay,
                room,
                name,
            }))
        }
        MapEvent::Heartbeat(guid, beat, from, room) => {
            map.stats.events += 1;
            // Taken whoever it is from: the slot is free for the next.
            let Some(info) = beat.take() else {
                return Ok(None);
            };
            if !map.from_their_session(guid, &from) {
                return Ok(None);
            }
            let relay: Relay =
                |guid, info| crate::game::conn::encode(&MSG_MOVE_HEARTBEAT_Server { guid, info });
            Ok(Some(Move {
                guid,
                info,
                relay,
                room,
                name: "MSG_MOVE_HEARTBEAT",
            }))
        }
        other => Err(other),
    }
}

/// The kept packets, done: across the threads if there are enough of
/// them, else one by one.
pub fn handle_all(map: &mut Map, moves: &mut Vec<Move>) {
    if moves.is_empty() {
        return;
    }
    let moves = std::mem::take(moves);
    if moves.len() < PARALLEL_FROM || !parallel(map) {
        for m in moves {
            alone(map, m);
        }
        return;
    }
    let first = across(map, &moves);
    let upto = first.unwrap_or(moves.len());
    map.stats.moves_across += upto as u64;
    map.stats.moves_alone += (moves.len() - upto) as u64;
    for (i, m) in moves.into_iter().enumerate() {
        if first.is_some_and(|f| i >= f) {
            alone(map, m);
        }
    }
}

/// One packet, as the map handles it alone.
fn alone(map: &mut Map, m: Move) {
    let _z = crate::tracy::on().then(|| crate::tracy::Zone::named("packet", m.name));
    movement::on_move(map, m.guid, m.info, m.relay);
    drop(m.room);
    crate::game::map::after_event(map);
}

/// The packets before the first that must be handled alone, stepped across
/// the threads and their tellings done; the index of that first, if any.
fn across(map: &mut Map, moves: &[Move]) -> Option<usize> {
    // Each mover's packets, in order.
    let mut by_mover: ahash::AHashMap<Guid, Vec<usize>> = ahash::AHashMap::new();
    for (i, m) in moves.iter().enumerate() {
        by_mover.entry(m.guid).or_default().push(i);
    }
    let now = map.now;
    let cfg = map.cfg.clone();
    let check = &cfg.anticheat;
    // Each mover's player, its own task's.
    let mut work: Vec<(&mut Player, Vec<usize>)> = Vec::with_capacity(by_mover.len());
    for p in map.players.values_mut() {
        if let Some(idx) = by_mover.remove(&p.guid) {
            work.push((p, idx));
        }
    }
    // Movers no longer here: handled alone (and found gone) in their turn.
    let mut first = by_mover.values().filter_map(|v| v.first().copied()).min();
    let stepped: Vec<Vec<(usize, Kept, Step)>> = {
        let _z = crate::zone!("moves: stepped");
        let go = |(p, idx): &mut (&mut Player, Vec<usize>)| {
            let mut out = Vec::with_capacity(idx.len());
            for &i in idx.iter() {
                let m = &moves[i];
                let kept = Kept::of(p);
                let s = step(p, m.guid, m.info, m.relay, now, check);
                let alone = matches!(s, Step::Alone);
                out.push((i, kept, s));
                if alone {
                    break;
                }
            }
            out
        };
        match visibility::pool(cfg.visibility_threads).filter(|_| cfg.visibility_threads != 1) {
            Some(pool) => pool.install(|| work.par_iter_mut().map(go).collect()),
            None => work.iter_mut().map(go).collect(),
        }
    };
    for steps in &stepped {
        if let Some((i, _, Step::Alone)) = steps.last() {
            first = Some(first.map_or(*i, |f| f.min(*i)));
        }
    }
    // Past the first to be handled alone, each mover as it was before it.
    let mut taken: Vec<Option<(Guid, Taken)>> = vec![None; moves.len()];
    for ((p, _), steps) in work.iter_mut().zip(stepped) {
        for (i, kept, s) in steps {
            if first.is_some_and(|f| i >= f) {
                kept.restore(p);
                break;
            }
            if let Step::Taken(t) = s {
                taken[i] = Some((p.guid, t));
            }
        }
    }
    drop(work);
    // What each told the map, in the order the packets came.
    let _z = crate::zone!("moves: told");
    let upto = first.unwrap_or(moves.len());
    for t in taken.into_iter().take(upto) {
        let Some((g, t)) = t else {
            continue;
        };
        tell(map, g, t);
    }
    first
}

/// A packet's step, on its mover alone: [`movement::moved_self_as`] for a
/// packet that changes nothing but its mover, or [`Step::Alone`] (with
/// nothing changed) for one that would.
fn step(
    p: &mut Player,
    guid: Guid,
    info: MovementInfo,
    relay: Relay,
    now: Instant,
    check: &AnticheatConfig,
) -> Step {
    use crate::game::map::auras::{
        AURA_WATER_WALK, INTERRUPT_ABOVE_WATER, INTERRUPT_MOVE, INTERRUPT_NOT_SEATED,
        INTERRUPT_TURNING, INTERRUPT_UNDER_WATER,
    };
    use crate::game::map::death::Death;
    // Another's to move, or moving another (`pets::move_possessed`).
    if p.owned.mover.is_some() {
        return Step::Alone;
    }
    if p.owned.possessed_by.is_some() || p.driven.is_some() || p.charge.is_some() {
        return Step::Ignored;
    }
    let walks_on_water = matches!(p.death, Death::Ghost { .. }) || p.auras.has(AURA_WATER_WALK);
    if p.teleport.is_some()
        || p.awaiting_worldport
        || p.flight.is_some()
        || !movement::finite(&info)
        || p.logout_at.is_some()
    {
        return Step::Ignored;
    }
    let dead = matches!(p.death, Death::Dead { .. });
    if dead && movement::horizontal(p.position(), info.position) > 0.5 {
        return Step::Ignored;
    }
    let info = if dead {
        MovementInfo {
            flags: movement::unsteered(info.flags),
            ..info
        }
    } else {
        info
    };
    let mut guard = p.misc.move_guard;
    if move_guard::held_back_with(p, &mut guard, &info, now) {
        if movement::horizontal(p.position(), info.position) > check.distance_allowance_yards {
            // Put back: a teleport.
            return Step::Alone;
        }
        p.misc.move_guard = guard;
        return Step::Ignored;
    }
    let info = move_guard::stripped(p, info, walks_on_water);
    if let Some((last, at)) = p.move_check
        && move_guard::speed_checked(p, &guard, check)
        && info.flags.get_on_transport().is_none()
        && move_guard::too_fast_with(p, &mut guard, last, at, &info, now, check)
    {
        // Pulled back: a teleport.
        return Step::Alone;
    }
    move_guard::taken_with(&mut guard, &info);
    // What else it does, looked at before anything changes.
    let before = p.movement.position;
    let f = &info.flags;
    let was_swimming = p.movement.flags.get_swimming().is_some();
    let swimming = f.get_swimming().is_some();
    let translating =
        f.get_forward() || f.get_backward() || f.get_strafe_left() || f.get_strafe_right();
    let d = crate::game::map::combat::dist(before, info.position);
    let moved = translating || d > 0.0;
    let turned = f.get_turn_left()
        || f.get_turn_right()
        || (info.orientation - p.movement.orientation).abs() > f32::EPSILON;
    let jumped = f.get_jumping().is_some();
    let interrupts = |mask: u32| {
        p.auras
            .holders
            .iter()
            .any(|h| h.interrupt_flags & mask != 0)
    };
    let mask = if moved {
        INTERRUPT_MOVE | INTERRUPT_TURNING | INTERRUPT_NOT_SEATED
    } else if turned {
        INTERRUPT_TURNING
    } else {
        0
    };
    let water = match (was_swimming, swimming) {
        (false, true) => INTERRUPT_UNDER_WATER,
        (true, false) => INTERRUPT_ABOVE_WATER,
        _ => 0,
    };
    let stands =
        (moved || turned || jumped) && !matches!(p.stand_state(), 0 | 7) && p.death == Death::Alive;
    let emotes = moved && p.fields.get_u32(crate::game::fields::UNIT_NPC_EMOTESTATE) != 0;
    // A cast it may break, a shot it delays, a liquid's spell, the void.
    let casting = p.caster.casting.is_some() && (d > 0.0 || turned || jumped);
    let shooting = d > 0.1 && p.combat.auto_repeat.is_some();
    let liquid = (d > 0.0 || moved) && p.misc.liquid_spell.is_some();
    let void = info.position.z < crate::game::map::death::VOID_Z
        && info.flags.get_on_transport().is_none();
    if (mask != 0 && interrupts(mask))
        || (water != 0 && interrupts(water))
        || stands
        || emotes
        || casting
        || shooting
        || liquid
        || void
    {
        return Step::Alone;
    }
    // The fall: one that ends hurting goes alone.
    let kept = Kept::of(p);
    if movement::landing(p, &info).is_some() {
        kept.restore(p);
        return Step::Alone;
    }
    p.misc.move_guard = guard;
    p.move_check = Some((info.position, now));
    let old_cell = p.cell;
    let new_cell = grid::cell_of(info.position.x, info.position.y);
    p.cell = new_cell;
    p.movement = info;
    Step::Taken(Taken {
        old_cell,
        new_cell,
        anchor: p.vis_anchor,
        info,
        frame: relay(guid, info),
    })
}

/// What a taken packet tells the map, as [`movement::moved_self_as`] does
/// after its mover's own changes.
fn tell(map: &mut Map, g: Guid, t: Taken) {
    crate::game::map::record::client_move(map, g, &t.info, t.frame.as_ref());
    map.grid.relocate(g, t.old_cell, t.new_cell);
    visibility::moved(map, g, t.anchor, t.info.position);
    if let Some(frame) = t.frame {
        map.send_move(g, &frame);
    }
    movement::remember_safe_position_as(map, g, &t.info);
}
