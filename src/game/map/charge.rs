//! Charge (spell effect 96), as Thistle Tea's `Movement.Charge` runs it:
//! the caster runs a path to its target at four times its run speed, at
//! most 24 yards a second, stopping half a yard inside its melee reach,
//! and swings on arriving. A player's client is moved by
//! `SMSG_MONSTER_MOVE` for its own guid, its swings put off by the run;
//! what it sends meanwhile is ignored. A root or stun caught on the way
//! holds once it is there; fear, confusion or another move ends the
//! charge without the swing.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::auras::{AURA_MOD_ROOT, AURA_MOD_STUN};
use crate::game::map::combat::{self, dist};
use crate::game::map::creature::{Motion, Walk};

/// The fastest a charge goes, and how much faster than a run.
const MAX_SPEED: f32 = 24.0;
const RUN_FACTOR: f32 = 4.0;
/// A unit's bounding radius when it has none.
const DEFAULT_RADIUS: f32 = 0.389;

/// A charge under way.
#[derive(Debug, Clone)]
pub struct Charge {
    pub arrives_at: Instant,
    pub spline_id: u32,
    /// Whom it swings at on arriving.
    pub attack: Option<Guid>,
    /// A player's run (a creature's is its motion).
    pub walk: Option<Walk>,
}

/// Thistle Tea's `Charge.approach_path`: `path` (after `origin`) as far as
/// the first point within `reach` of `target`, the last leg cut to end at
/// `reach`.
pub fn approach(
    path: &[Vector3d],
    origin: Vector3d,
    target: Vector3d,
    reach: f32,
) -> Vec<Vector3d> {
    let mut out = Vec::new();
    let mut from = origin;
    for p in path {
        if dist(*p, target) <= reach {
            let d = dist(from, target);
            if d > reach {
                let f = (d - reach) / d;
                out.push(Vector3d {
                    x: from.x + (target.x - from.x) * f,
                    y: from.y + (target.y - from.y) * f,
                    z: from.z + (target.z - from.z) * f,
                });
            }
            return out;
        }
        out.push(*p);
        from = *p;
    }
    out
}

/// Whether a charge holds `g`'s movement: its client's is ignored.
pub fn charging(map: &Map, g: Guid) -> bool {
    match map.players.get(&g) {
        Some(p) => p.charge.is_some(),
        None => map
            .creature(g)
            .is_some_and(|c| c.engagement.charge.is_some()),
    }
}

/// `caster` charges `target`, swinging at it on arrival if `attack`. Not
/// for the dead, the rooted or stunned, one on a flight, nor within reach
/// already, nor with no way there.
pub fn start(map: &mut Map, caster: Guid, target: Guid, attack: bool) {
    let now = map.now;
    let (Some(me), Some(them)) = (map.unit_view(caster), map.unit_view(target)) else {
        return;
    };
    let held = crate::game::map::auras::auras(map, caster)
        .is_some_and(|a| a.has(AURA_MOD_ROOT) || a.has(AURA_MOD_STUN));
    let flying = map.players.get(&caster).is_some_and(|p| p.flight.is_some());
    if caster == target || !me.alive || held || flying {
        return;
    }
    let (from, to) = (me.position, them.position);
    let reach = combat::melee_range(&me, &them);
    if dist(from, to) <= reach {
        return;
    }
    let path = match map.nav.as_mut() {
        Some(nav) => match nav.path(from, to) {
            Some(p) => p,
            None => return,
        },
        None => vec![from, to],
    };
    let mut points = vec![from];
    points.extend(approach(&path[1..], from, to, reach - 0.5));
    if points.len() < 2 {
        return;
    }
    let run = match map.players.get(&caster) {
        Some(p) => p.speeds.run,
        None => map.creature(caster).map_or(7.0, |c| c.speeds.run),
    };
    let speed = (run * RUN_FACTOR).min(MAX_SPEED);
    if speed <= 0.0 {
        return;
    }
    let spline_id = map.next_spline_id;
    map.next_spline_id = map.next_spline_id.wrapping_add(1);
    let walk = Walk::new(points, speed, now, spline_id, true);
    let arrives_at = now + walk.duration.max(Duration::from_millis(1));
    let attack = attack.then_some(target);
    let frame = walk.frame(caster, from);
    let radius = |m: &Map, x: Guid| {
        m.unit_fields(x)
            .map(|f| f.get_f32(crate::game::fields::UNIT_FIELD_BOUNDINGRADIUS))
            .filter(|r| *r > 0.0)
            .unwrap_or(DEFAULT_RADIUS)
    };
    let radii = radius(map, caster) + radius(map, target);
    if let Some(p) = map.players.get_mut(&caster) {
        // Its swings wait out the run (Thistle Tea's `delay_swings`).
        let gap = (dist(from, to) - radii).max(0.0);
        let delay = Duration::from_millis((200.0 + 40.0 * gap) as u64);
        for at in [&mut p.combat.next_swing, &mut p.combat.next_offhand] {
            *at = Some(at.map_or(now, |t| t.max(now)) + delay);
        }
        p.movement.orientation = (to.y - from.y).atan2(to.x - from.x);
        p.charge = Some(Charge {
            arrives_at,
            spline_id,
            attack,
            walk: Some(walk),
        });
        if let Some(frame) = frame {
            map.send_to_observers(caster, &frame, true);
        }
        return;
    }
    let Some(c) = map.creature_mut(caster) else {
        return;
    };
    c.motion = Motion::Moving(walk);
    c.chase_target_at = None;
    c.engagement.charge = Some(Charge {
        arrives_at,
        spline_id,
        attack,
        walk: None,
    });
    if let Some(frame) = frame {
        map.send_to_observers(caster, &frame, false);
    }
}

/// A creature's fight waits on its charge: whether one is under way.
pub fn step(map: &mut Map, g: Guid) -> bool {
    if map
        .creature(g)
        .is_none_or(|c| c.engagement.charge.is_none())
    {
        return false;
    }
    crate::game::map::ai::advance_motion(map, g);
    let now = map.now;
    let Some(c) = map.creature(g) else {
        return false;
    };
    let Some(charge) = c.engagement.charge.as_ref() else {
        return false;
    };
    let on_it = matches!(&c.motion, Motion::Moving(w) if w.spline_id == charge.spline_id);
    if on_it && now < charge.arrives_at && c.alive() {
        return true;
    }
    let arrived = now >= charge.arrives_at && (on_it || matches!(c.motion, Motion::Idle { .. }));
    finish(map, g, arrived);
    false
}

/// The charges under way go on, or end.
pub fn update(map: &mut Map) {
    let now = map.now;
    let players: Vec<Guid> = map
        .players
        .values()
        .filter(|p| p.charge.is_some())
        .map(|p| p.guid)
        .collect();
    for g in players {
        advance(map, g);
        let Some(p) = map.players.get(&g) else {
            continue;
        };
        let Some(charge) = p.charge.as_ref() else {
            continue;
        };
        // Fear, confusion, a charm or a teleport take it over.
        let taken = p.driven.is_some() || p.teleport.is_some() || p.awaiting_worldport;
        let alive = p.death == crate::game::map::death::Death::Alive;
        if now >= charge.arrives_at || taken || !alive {
            finish(map, g, !taken && alive);
        }
    }
    let creatures: Vec<Guid> = map
        .creatures
        .iter()
        .filter(|(_, c)| c.engagement.charge.is_some())
        .map(|(_, c)| c.guid)
        .collect();
    for g in creatures {
        step(map, g);
    }
}

/// A player along its charge at `now`: position, cell and what it sees.
fn advance(map: &mut Map, g: Guid) {
    let now = map.now;
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let Some(w) = p.charge.as_ref().and_then(|c| c.walk.as_ref()) else {
        return;
    };
    let (pos, o) = if w.done(now) {
        (w.destination(), p.movement.orientation)
    } else {
        w.position_at(now)
    };
    p.movement.position = pos;
    p.movement.orientation = o;
    let (anchor, old_cell) = (p.vis_anchor, p.cell);
    let new_cell = crate::game::map::grid::cell_of(pos.x, pos.y);
    p.cell = new_cell;
    map.grid.relocate(g, old_cell, new_cell);
    crate::game::map::visibility::moved(map, g, anchor, pos);
}

/// A charge ends: there (`arrived`), it swings at its target; a player's
/// client goes on from where the run left it.
fn finish(map: &mut Map, g: Guid, arrived: bool) {
    let now = map.now;
    let charge = match map.players.get_mut(&g) {
        Some(p) => {
            let charge = p.charge.take();
            let at = p.position();
            p.move_check = Some((at, now));
            charge
        }
        None => map.creature_mut(g).and_then(|c| c.engagement.charge.take()),
    };
    let Some(target) = charge.and_then(|c| c.attack) else {
        return;
    };
    let alive = map.unit_view(g).is_some_and(|u| u.alive);
    if arrived && alive && map.unit_view(target).is_some_and(|u| u.alive) {
        if guid::is_unit(g) {
            crate::game::map::ai::attacked(map, g, target);
        }
        combat::start_attack(map, g, target);
    }
}
