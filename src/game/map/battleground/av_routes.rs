//! The routes Alterac Valley's launched NPCs walk, as Thistle Tea's
//! `start_waypoints` (source 5, `script_waypoint`) walks them: one point at
//! a time, acting at the points their scripts name ([`super::av_scripts`]),
//! pausing where a script holds them, and taking up the route again where
//! they left it once a fight is over. A leader's formation keeps its place
//! behind it, and a flyer flies.
//!
//! The routes run on the match's clock, seen or not: a creature nobody
//! watches is where its walk would have brought it.

use std::time::{Duration, Instant};

use ahash::AHashMap;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::map::Map;
use crate::game::map::battleground::av_quests::{extras, extras_mut};
use crate::game::map::creature::Motion;

/// What reaching a point does to the route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// On to the next point.
    Next,
    /// Stand a while, then the point's held actions, then on.
    Hold(Duration),
    /// The route ends here.
    Stop,
}

#[derive(Debug, Clone, Copy)]
pub struct Walker {
    /// Whose route it walks.
    pub entry: u32,
    /// The point it walks to.
    pub point: usize,
    /// When it gets there.
    pub ends: Instant,
    /// Held there until then.
    pub held: Option<Instant>,
    /// Off its route for a fight: it goes on once that's over.
    pub interrupted: bool,
    /// Its home stays where a script put it.
    pub fixed_home: bool,
}

/// A member of a formation: its place behind its leader.
#[derive(Debug, Clone, Copy)]
pub struct Place {
    pub leader: Guid,
    pub distance: f32,
    /// From the leader's facing, in radians.
    pub angle: f32,
}

#[derive(Debug, Clone, Default)]
pub struct Routes {
    pub walkers: AHashMap<Guid, Walker>,
    pub formation: AHashMap<Guid, Place>,
    /// The creatures that fly.
    pub flyers: ahash::AHashSet<Guid>,
}

/// Movement flags a flyer shows: it can fly, and is flying.
pub const MOVE_FLYING: u32 = 0x0180_0000;

fn routes_mut(map: &mut Map) -> Option<&mut Routes> {
    extras_mut(map).map(|x| &mut x.routes)
}

/// The route of `entry`, if the valley has one.
pub fn route(map: &Map, entry: u32) -> Option<&[(f32, f32, f32)]> {
    map.realm
        .data
        .bg_routes
        .get(&entry)
        .map(Vec::as_slice)
        .filter(|r| !r.is_empty())
}

/// `g` starts walking `entry`'s route from its first point. False if
/// there is none.
pub fn start(map: &mut Map, g: Guid, entry: u32) -> bool {
    start_at(map, g, entry, 0, false)
}

/// `g` walks `entry`'s route from point `point`, its home kept where it is
/// if `fixed_home`.
pub fn start_at(map: &mut Map, g: Guid, entry: u32, point: usize, fixed_home: bool) -> bool {
    if route(map, entry).is_none_or(|r| point >= r.len()) || map.creature(g).is_none() {
        return false;
    }
    if let Some(c) = map.creature_mut(g) {
        // It walks the route, not where it wanders.
        c.movement_type = 0;
    }
    let now = map.now;
    let Some(r) = routes_mut(map) else {
        return false;
    };
    r.walkers.insert(
        g,
        Walker {
            entry,
            point,
            ends: now,
            held: None,
            interrupted: false,
            fixed_home,
        },
    );
    leg(map, g);
    true
}

/// `g` stops walking.
pub fn stop(map: &mut Map, g: Guid) {
    if let Some(r) = routes_mut(map) {
        r.walkers.remove(&g);
    }
}

pub fn walking(map: &Map, g: Guid) -> bool {
    extras(map).is_some_and(|x| x.routes.walkers.contains_key(&g))
}

/// `g` takes its place in `leader`'s formation, where it stands now.
pub fn join(map: &mut Map, g: Guid, leader: Guid) {
    let (Some(me), Some(them)) = (
        map.creature(g).map(|c| c.position),
        map.creature(leader).map(|c| (c.position, c.orientation)),
    ) else {
        return;
    };
    let distance = ((me.x - them.0.x).powi(2) + (me.y - them.0.y).powi(2)).sqrt();
    let angle = (me.y - them.0.y).atan2(me.x - them.0.x) - them.1;
    if let Some(r) = routes_mut(map) {
        r.formation.insert(
            g,
            Place {
                leader,
                distance,
                angle,
            },
        );
    }
}

pub fn leave(map: &mut Map, g: Guid) {
    if let Some(r) = routes_mut(map) {
        r.formation.remove(&g);
    }
}

/// The members of `leader`'s formation.
pub fn members(map: &Map, leader: Guid) -> Vec<Guid> {
    extras(map).map_or_else(Vec::new, |x| {
        let mut out: Vec<Guid> = x
            .routes
            .formation
            .iter()
            .filter(|(_, p)| p.leader == leader)
            .map(|(g, _)| *g)
            .collect();
        out.sort_by_key(|g| g.guid());
        out
    })
}

/// `g` flies (or lands): its walks go through the air.
pub fn set_fly(map: &mut Map, g: Guid, on: bool) {
    if let Some(c) = map.creature_mut(g) {
        if on {
            c.move_flags |= MOVE_FLYING;
        } else {
            c.move_flags &= !MOVE_FLYING;
        }
    }
    map.visibility_dirty.insert(g);
    if let Some(r) = routes_mut(map) {
        if on {
            r.flyers.insert(g);
        } else {
            r.flyers.remove(&g);
        }
    }
}

pub fn flies(map: &Map, g: Guid) -> bool {
    extras(map).is_some_and(|x| x.routes.flyers.contains(&g))
}

/// `g` goes straight to `to`, walking, running or flying. When it gets
/// there.
pub fn go(map: &mut Map, g: Guid, to: Vector3d) -> Option<Instant> {
    crate::game::map::ai::advance_motion(map, g);
    let now = map.now;
    let fly = flies(map, g);
    let spline_id = map.next_spline_id;
    let c = map.creature_mut(g)?;
    let from = c.position;
    let run = c.scripted.as_deref().is_some_and(|s| s.run);
    let frame = c.walk_at(vec![from, to], run, fly, None, now, spline_id)?;
    let Motion::Moving(w) = &c.motion else {
        return None;
    };
    let ends = w.started + w.duration;
    c.orientation = (to.y - from.y).atan2(to.x - from.x);
    map.next_spline_id = map.next_spline_id.wrapping_add(1);
    map.send_to_observers(g, &frame, false);
    Some(ends)
}

/// Walk the leg to the walker's point; its formation follows.
fn leg(map: &mut Map, g: Guid) {
    let Some(w) = extras(map).and_then(|x| x.routes.walkers.get(&g)).copied() else {
        return;
    };
    let Some(&(x, y, z)) = route(map, w.entry).and_then(|r| r.get(w.point)) else {
        stop(map, g);
        return;
    };
    let to = Vector3d { x, y, z };
    crate::game::map::ai::advance_motion(map, g);
    if !w.fixed_home
        && let Some(c) = map.creature_mut(g)
    {
        c.home = c.position;
        c.home_orientation = c.orientation;
    }
    let ends = go(map, g, to).unwrap_or(map.now);
    if let Some(w) = routes_mut(map).and_then(|r| r.walkers.get_mut(&g)) {
        w.ends = ends;
    }
    follow(map, g, to);
}

/// A leader walks to `to`: its formation, and the creature group it leads,
/// keep their places behind it.
fn follow(map: &mut Map, leader: Guid, to: Vector3d) {
    let Some((facing, run)) = map
        .creature(leader)
        .map(|c| (c.orientation, c.scripted.as_deref().is_some_and(|s| s.run)))
    else {
        return;
    };
    for m in crate::game::map::creature_groups::members_of(map, leader) {
        if m != leader {
            crate::game::map::ai::advance_motion(map, m);
        }
    }
    crate::game::map::creature_groups::leader_walked(map, leader);
    let places: Vec<(Guid, Place)> = extras(map)
        .map(|x| {
            x.routes
                .formation
                .iter()
                .filter(|(_, p)| p.leader == leader)
                .map(|(g, p)| (*g, *p))
                .collect()
        })
        .unwrap_or_default();
    for (m, p) in places {
        let free = map
            .creature(m)
            .is_some_and(|c| c.alive() && !c.combat.attacking && !c.evading);
        if !free || walking(map, m) {
            continue;
        }
        let angle = facing + p.angle;
        let place = Vector3d {
            x: to.x + p.distance * angle.cos(),
            y: to.y + p.distance * angle.sin(),
            z: to.z,
        };
        if let Some(s) = crate::game::map::event_ai::scripted_mut(map, m) {
            s.run = run;
        }
        go(map, m, place);
    }
}

/// Each update: walkers at their point act and go on, those held go on
/// when their wait is over, those fighting take up their route again
/// when the fight is done.
pub fn update(map: &mut Map) {
    let now = map.now;
    let due: Vec<(Guid, Walker)> = match extras(map) {
        Some(x) => x
            .routes
            .walkers
            .iter()
            .filter(|(_, w)| w.ends <= now || w.held.is_some() || w.interrupted)
            .map(|(g, w)| (*g, *w))
            .collect(),
        None => return,
    };
    // Those who fight leave their route for it.
    let fighting: Vec<Guid> = extras(map)
        .map(|x| {
            x.routes
                .walkers
                .keys()
                .copied()
                .filter(|g| {
                    map.creature(*g)
                        .is_some_and(|c| c.combat.attacking || c.evading)
                })
                .collect()
        })
        .unwrap_or_default();
    for g in fighting {
        if let Some(w) = routes_mut(map).and_then(|r| r.walkers.get_mut(&g)) {
            w.interrupted = true;
        }
    }
    for (g, w) in due {
        step(map, g, w, now);
    }
    // Gone, or dead: their routes and places go with them.
    let gone: Vec<Guid> = extras(map)
        .map(|x| {
            x.routes
                .walkers
                .keys()
                .chain(x.routes.formation.keys())
                .copied()
                .filter(|g| map.creature(*g).is_none_or(|c| !c.alive()))
                .collect()
        })
        .unwrap_or_default();
    if let Some(r) = routes_mut(map) {
        for g in gone {
            r.walkers.remove(&g);
            r.formation.remove(&g);
        }
    }
}

fn step(map: &mut Map, g: Guid, w: Walker, now: Instant) {
    let Some(c) = map.creature(g).filter(|c| c.alive()) else {
        return;
    };
    let (fighting, moving) = (
        c.combat.attacking || c.evading,
        matches!(c.motion, Motion::Moving(_)),
    );
    if let Some(until) = w.held {
        if until > now || fighting {
            return;
        }
        if let Some(w) = routes_mut(map).and_then(|r| r.walkers.get_mut(&g)) {
            w.held = None;
        }
        let flow = super::av_scripts::released(map, g, w.entry, w.point);
        next(map, g, flow);
        return;
    }
    if w.interrupted {
        if fighting || moving {
            return;
        }
        if let Some(w) = routes_mut(map).and_then(|r| r.walkers.get_mut(&g)) {
            w.interrupted = false;
        }
        leg(map, g);
        return;
    }
    if fighting {
        return;
    }
    // There: where the walk ends.
    crate::game::map::ai::advance_motion(map, g);
    if let Some(&(x, y, z)) = route(map, w.entry).and_then(|r| r.get(w.point)) {
        let at = Vector3d { x, y, z };
        let off = map.creature(g).is_some_and(|c| {
            !matches!(c.motion, Motion::Moving(_))
                && crate::game::map::combat::dist(c.position, at) > 0.5
        });
        if off {
            // Pushed or held up on the way: the leg again.
            leg(map, g);
            return;
        }
    }
    crate::game::map::event_ai::movement_inform(map, g, 2, w.point as u32);
    let flow = super::av_scripts::arrived(map, g, w.entry, w.point);
    next(map, g, flow);
}

fn next(map: &mut Map, g: Guid, flow: Flow) {
    let now = map.now;
    let len = extras(map)
        .and_then(|x| x.routes.walkers.get(&g))
        .and_then(|w| route(map, w.entry))
        .map_or(0, <[_]>::len);
    match flow {
        Flow::Hold(d) => {
            if let Some(w) = routes_mut(map).and_then(|r| r.walkers.get_mut(&g)) {
                w.held = Some(now + d);
            }
        }
        Flow::Next => {
            let more = routes_mut(map)
                .and_then(|r| r.walkers.get_mut(&g))
                .is_some_and(|w| {
                    w.point += 1;
                    w.point < len
                });
            if more {
                leg(map, g);
            } else {
                finish(map, g);
            }
        }
        Flow::Stop => finish(map, g),
    }
}

/// The route is over: it stands where it is.
fn finish(map: &mut Map, g: Guid) {
    stop(map, g);
    if let Some(c) = map.creature_mut(g) {
        c.movement_type = 0;
        if !matches!(c.motion, Motion::Moving(_)) {
            c.motion = Motion::Idle { until: None };
        }
    }
}

/// Every route walked through now, as far as it goes: points reached,
/// holds over.
#[cfg(test)]
pub fn hurry(map: &mut Map) {
    hurry_until(map, |_| false);
}

/// The routes walked a point at a time until `done` (or as far as they
/// go).
#[cfg(test)]
pub fn hurry_until(map: &mut Map, done: impl Fn(&Map) -> bool) {
    for _ in 0..1000 {
        if done(map) {
            return;
        }
        let now = map.now;
        // Every walk is done: the walkers', those following them, and
        // any other.
        for (_, c) in map.creatures.iter_mut() {
            if let Motion::Moving(w) = &mut c.motion
                && let Some(started) = now.checked_sub(w.duration)
            {
                w.started = started;
            }
        }
        let Some(r) = routes_mut(map) else {
            return;
        };
        if r.walkers.is_empty() {
            return;
        }
        for w in r.walkers.values_mut() {
            w.ends = now;
            if w.held.is_some() {
                w.held = Some(now);
            }
        }
        update(map);
    }
}
