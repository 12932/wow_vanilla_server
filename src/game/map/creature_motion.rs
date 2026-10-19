//! What a creature's body allows it, beyond the fight, as Thistle Tea has
//! it:
//!
//! - its habitat, from its template's inhabit type: walkers keep to the
//!   ground, swimmers to the water (at their depth, and in three
//!   dimensions after a target there), flyers to the air (straight where
//!   the way is clear, circling their spawn rather than wandering, a
//!   cyclic path flown without a stop); a flyer that dies falls;
//! - wounds: below 16, 11 and 6 percent of its health it runs at 70, 60
//!   and 50 percent of its speed;
//! - a player greeting it, or picking one of its gossip options, stops its
//!   wandering or patrol for three minutes;
//! - a critter, hurt or cursed, runs for thirty seconds, never fighting
//!   back, and then goes home.

use std::time::Duration;

use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::combat::dist;
use crate::game::map::creature::{MOVEMENT_RANDOM, MOVEMENT_WAYPOINT, Motion};

pub const INHABIT_GROUND: u8 = 1;
pub const INHABIT_WATER: u8 = 2;
pub const INHABIT_AIR: u8 = 4;
/// vmangos's movement type 3: a waypoint path flown round without a stop.
pub const MOVEMENT_CYCLIC: u8 = 3;
/// `static_flags1`: it never moves; it swims (the swimming animation).
const STATIC_SESSILE: u32 = 0x100;
const STATIC_CAN_SWIM: u32 = 0x1000_0000;
/// `static_flags2`: wounds don't slow it.
const STATIC2_NO_WOUNDED_SLOWDOWN: u32 = 0x40;
/// `flags_extra`: talking to it doesn't stop it.
const EXTRA_NO_MOVEMENT_PAUSE: u32 = 0x20;
/// A flyer's movement flags, as Thistle Tea sets them.
pub const FLIGHT_FLAGS: u32 = 0x0180_0000;
/// Water at least this deep is water to swim in.
const MIN_DEPTH: f32 = 1.5;
/// How far over the surface a point still counts as in the water, and
/// how far under its floor.
const SURFACE_REACH: f32 = 2.0;
const FLOOR_TOLERANCE: f32 = 0.5;
/// A path's water is sampled this often.
const SAMPLE: f32 = 3.0;
/// How far above or below a creature that doesn't fly notices a player.
const VERTICAL_RANGE: f32 = 3.0;
const DEFAULT_RADIUS: f32 = 0.389;
/// How long talk keeps a creature still.
pub const TALK_PAUSE: Duration = Duration::from_secs(180);
/// How long a frightened critter keeps away.
pub const ESCAPE: Duration = Duration::from_secs(30);
/// Gravity, and the speed a fall goes no faster than, for a falling
/// corpse.
const GRAVITY: f32 = 19.291_105;
const TERMINAL_VELOCITY: f32 = 60.148_003;

/// Where a creature may go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Habitat {
    pub walk: bool,
    pub swim: bool,
    pub fly: bool,
    /// It swims with the swimming animation: it keeps its depth, and goes
    /// after a target in three dimensions.
    pub swims: bool,
}

impl Default for Habitat {
    fn default() -> Self {
        Habitat {
            walk: true,
            swim: true,
            fly: false,
            swims: false,
        }
    }
}

/// A creature's habitat: its template's, a script's flight over it; a
/// hunter's or summoned pet walks and swims whatever it was.
pub fn habitat(map: &Map, g: Guid) -> Habitat {
    let Some(c) = map.creature(g) else {
        return Habitat::default();
    };
    if c.pet.as_deref().is_some_and(|p| {
        matches!(
            p.kind,
            crate::game::map::pets::Kind::Hunter | crate::game::map::pets::Kind::Demon
        )
    }) {
        return Habitat::default();
    }
    let Some(t) = map.realm.data.creature_template(c.entry) else {
        return Habitat::default();
    };
    let inhabit = if t.inhabit_type == 0 {
        INHABIT_GROUND | INHABIT_WATER
    } else {
        t.inhabit_type
    };
    let swim = inhabit & INHABIT_WATER != 0;
    Habitat {
        walk: inhabit & INHABIT_GROUND != 0,
        swim,
        fly: inhabit & INHABIT_AIR != 0 || c.scripted.as_deref().is_some_and(|s| s.fly),
        swims: swim && t.static_flags1 & STATIC_CAN_SWIM != 0,
    }
}

/// Whether a template's creature flies from the start.
pub fn template_flies(t: &crate::game::data::CreatureTemplate) -> bool {
    t.inhabit_type & INHABIT_AIR != 0
}

/// Whether it never moves.
pub fn sessile(map: &Map, g: Guid) -> bool {
    map.creature(g)
        .and_then(|c| map.realm.data.creature_template(c.entry))
        .is_some_and(|t| t.static_flags1 & STATIC_SESSILE != 0)
}

/// The water at a point, deep enough to swim in: its surface and floor.
pub fn water(map: &mut Map, p: Vector3d) -> Option<(f32, f32)> {
    let surface = map
        .nav
        .as_mut()
        .and_then(|n| n.liquid_surface(p))
        .or_else(|| crate::game::map::environment::liquid_at(map, p).map(|l| l.surface))?;
    if p.z > surface + SURFACE_REACH {
        return None;
    }
    let floor = floor_under(map, p)?;
    (surface - floor > MIN_DEPTH && p.z >= floor - FLOOR_TOLERANCE).then_some((surface, floor))
}

/// The highest ground at or under a point.
fn floor_under(map: &mut Map, p: Vector3d) -> Option<f32> {
    let nav = map.nav.as_mut()?;
    let mut best: Option<f32> = None;
    // The navmesh's heights at a column, as the ground nearest each guess.
    for hint in [p.z, p.z - 5.0, p.z - 20.0, p.z - 60.0] {
        if let Some(h) = nav.ground(p.x, p.y, hint)
            && h <= p.z + FLOOR_TOLERANCE
        {
            best = Some(best.map_or(h, |b: f32| b.max(h)));
        }
    }
    best
}

fn samples(a: Vector3d, b: Vector3d) -> Vec<Vector3d> {
    let steps = (crate::game::map::combat::dist(a, b) / SAMPLE)
        .ceil()
        .max(1.0) as usize;
    (0..=steps)
        .map(|i| {
            let f = i as f32 / steps as f32;
            Vector3d {
                x: a.x + (b.x - a.x) * f,
                y: a.y + (b.y - a.y) * f,
                z: a.z + (b.z - a.z) * f,
            }
        })
        .collect()
}

fn clear(map: &mut Map, a: Vector3d, b: Vector3d) -> bool {
    map.nav.as_mut().is_none_or(|n| n.line_of_sight(a, b))
}

/// A path for a creature from `from` to `to` as its habitat allows: its
/// points (the first `from`), whether it gets all the way, and whether
/// it flies it. `None` where it can't go at all.
pub fn route(
    map: &mut Map,
    g: Guid,
    from: Vector3d,
    to: Vector3d,
) -> Option<(Vec<Vector3d>, bool, bool)> {
    let hab = habitat(map, g);
    // With no navmesh, or its tiles here still being read: straight.
    if map.nav_at(&[from, to]).is_none() {
        return Some((vec![from, to], true, hab.fly));
    }
    if hab.fly {
        if clear(map, from, to) {
            return Some((vec![from, to], true, true));
        }
        let mut path = map.nav.as_mut()?.path(from, to)?;
        path.push(to);
        let ok = path.windows(2).all(|w| clear(map, w[0], w[1]));
        return ok.then_some((path, true, true));
    }
    let to_water = water(map, to);
    if !hab.walk && to_water.is_none() && !hab.swim {
        return None;
    }
    // A swimmer after a point in the water goes straight through it.
    if hab.swims
        && let Some((surface, _)) = to_water
    {
        let to = Vector3d {
            z: to.z.min(surface),
            ..to
        };
        let wet = samples(from, to).into_iter().all(|p| water_at(map, p));
        if wet && clear(map, from, to) {
            return Some((vec![from, to], true, false));
        }
    }
    let path = match map.nav.as_mut()?.path(from, to) {
        Some(p) => p,
        // Off the navmesh (in open water): from the bottom under it, or
        // straight where the way is clear.
        None => {
            let bottom = floor_under(map, from).map(|z| Vector3d { z, ..from });
            match bottom.and_then(|b| map.nav.as_mut()?.path(b, to)) {
                Some(mut p) => {
                    p[0] = from;
                    p
                }
                None if clear(map, from, to) => vec![from, to],
                None => return None,
            }
        }
    };
    // What it may not enter, it goes as far as the edge of.
    if hab.walk && hab.swim {
        return Some((path, true, false));
    }
    let mut kept = vec![path[0]];
    for w in path.windows(2) {
        let mut ok = true;
        for p in samples(w[0], w[1]) {
            let wet = water_at(map, p);
            if (wet && !hab.swim) || (!wet && !hab.walk) {
                ok = false;
                break;
            }
            if kept.last().is_none_or(|l| dist(*l, p) > 0.5) {
                kept.push(p);
            }
        }
        if !ok {
            return (kept.len() > 1).then_some((kept, false, false));
        }
    }
    Some((path, true, false))
}

fn water_at(map: &mut Map, p: Vector3d) -> bool {
    water(map, p).is_some()
}

/// Whether a creature may go after `target` where it is: one that doesn't
/// fly only near its own height, and only where its habitat allows.
pub fn may_reach(map: &Map, g: Guid, target: Guid) -> bool {
    let hab = habitat(map, g);
    let (Some(me), Some(them)) = (map.position_of(g), map.position_of(target)) else {
        return false;
    };
    if !hab.fly {
        let radius = |x: Guid| {
            map.unit_fields(x)
                .map(|f| f.get_f32(UNIT_FIELD_BOUNDINGRADIUS))
                .filter(|r| *r > 0.0)
                .unwrap_or(DEFAULT_RADIUS)
        };
        if (them.z - me.z).abs() > VERTICAL_RANGE + radius(g) + radius(target) {
            return false;
        }
    }
    // A player swimming is in the water; one not, on land.
    let swimming = map
        .players
        .get(&target)
        .map(|p| p.movement.flags.get_swimming().is_some());
    match swimming {
        Some(true) => hab.swim,
        Some(false) => hab.walk || hab.fly,
        None => true,
    }
}

/// The flyers' and swimmers' idle moves: a flyer's circle and cyclic
/// path, and its waypoints through the air; a swimmer's wander at its
/// depth. True if it took the creature's step (the rest walk as before).
pub fn idle_step(map: &mut Map, key: usize) -> bool {
    let now = map.now;
    let Some(c) = map.creatures.get(key) else {
        return false;
    };
    let g = c.guid;
    if c.pet.is_some() || !c.alive() {
        return false;
    }
    if sessile(map, g) {
        return true;
    }
    let hab = habitat(map, g);
    // One keeping its place in a formation goes only where it is led.
    if (hab.fly || hab.swims) && crate::game::map::creature_groups::following(map, g) {
        crate::game::map::ai::advance_motion(map, g);
        return true;
    }
    let Some(c) = map.creatures.get(key) else {
        return false;
    };
    let kind = c.movement_type;
    let held = c
        .scripted
        .as_deref()
        .is_some_and(|s| s.hold.is_some() || s.route.is_some());
    let continuous = hab.fly && (kind == MOVEMENT_RANDOM || kind == MOVEMENT_CYCLIC);
    let swimmer = !hab.fly && hab.swims && kind == MOVEMENT_RANDOM;
    let special = continuous || (hab.fly && kind == MOVEMENT_WAYPOINT) || swimmer;
    if !special || held {
        return false;
    }
    // A continuous one goes on round where a lap ends; the others' moves
    // under way are the usual step's, with their waypoints' stops.
    if continuous {
        crate::game::map::ai::advance_motion(map, g);
    }
    let Some(c) = map.creatures.get(key) else {
        return false;
    };
    let due = match &c.motion {
        Motion::Moving(_) => {
            return continuous;
        }
        Motion::Idle { until: Some(t) } => *t <= now || continuous,
        Motion::Idle { until: None } => continuous,
    };
    if !due {
        return false;
    }
    // A swimmer's wander is a swim about water only (Thistle Tea's
    // `Aquatic.random_point`: its anchor in the water); about dry land it
    // walks as the rest. (Stormwind's guards can swim: on land they stood
    // still, after their fights and before.)
    if swimmer {
        let center = crate::game::map::event_ai::wander_center(c);
        if water(map, center).is_none() {
            return false;
        }
    }
    let path = plan(map, key, g, kind, hab);
    let Some(path) = path.filter(|p| p.len() >= 2) else {
        if let Some(c) = map.creatures.get_mut(key) {
            c.motion = Motion::Idle {
                until: Some(now + Duration::from_secs(5)),
            };
        }
        return true;
    };
    // A circling flyer runs; one on a path flies at a walk unless a script
    // says run (a cyclic one, too, unless its template always runs).
    let scripted_run = map
        .creatures
        .get(key)
        .and_then(|c| c.scripted.as_deref())
        .is_some_and(|s| s.run);
    let run = match kind {
        MOVEMENT_RANDOM => hab.fly || scripted_run,
        MOVEMENT_CYCLIC => scripted_run || always_runs(map, g),
        _ => scripted_run,
    };
    let spline_id = map.next_spline_id;
    map.next_spline_id = map.next_spline_id.wrapping_add(1);
    let Some(c) = map.creatures.get_mut(key) else {
        return true;
    };
    let frame = c.walk_at(path, run, hab.fly, None, now, spline_id);
    if frame.is_none() {
        c.motion = Motion::Idle {
            until: Some(now + Duration::from_secs(5)),
        };
    }
    if let Some(frame) = frame {
        map.send_to_observers(g, &frame, false);
        // It leads its formation, as one on foot does.
        crate::game::map::creature_groups::leader_walked(map, g);
    }
    true
}

/// The next idle move of a flyer or swimmer: a lap, a swim, a flight to its
/// next waypoint.
fn plan(map: &mut Map, key: usize, g: Guid, kind: u8, hab: Habitat) -> Option<Vec<Vector3d>> {
    let c = map.creatures.get(key)?;
    let (home, radius, pos) = (
        crate::game::map::event_ai::wander_center(c),
        c.wander_distance,
        c.position,
    );
    match kind {
        MOVEMENT_RANDOM if hab.fly => Some(circle(home, radius.max(5.0), pos)),
        MOVEMENT_RANDOM => {
            swim_point(map, home, radius).and_then(|to| route(map, g, pos, to).map(|(p, _, _)| p))
        }
        MOVEMENT_CYCLIC => {
            let points = waypoints(map, g)?;
            // Far from its lap, it makes its way to the start first.
            let first = points.first().map(|w| Vector3d {
                x: w.x,
                y: w.y,
                z: w.z,
            })?;
            if crate::game::map::combat::dist(pos, first) > 10.0 {
                return route(map, g, pos, first).map(|(p, _, _)| p);
            }
            let mut lap: Vec<Vector3d> = vec![pos];
            lap.extend(points.iter().map(|w| Vector3d {
                x: w.x,
                y: w.y,
                z: w.z,
            }));
            if let Some(first) = points.first() {
                lap.push(Vector3d {
                    x: first.x,
                    y: first.y,
                    z: first.z,
                });
            }
            Some(lap)
        }
        _ => {
            // Waypoints, through the air.
            // To the point it is bound for (the usual step moves it on
            // when it gets there).
            let points = waypoints(map, g)?;
            let c = map.creatures.get(key)?;
            let w = points[c.waypoint % points.len()];
            let to = Vector3d {
                x: w.x,
                y: w.y,
                z: w.z,
            };
            route(map, g, pos, to).map(|(p, _, _)| p)
        }
    }
}

/// Whether its template has it always run (`flags_extra` 0x40).
fn always_runs(map: &Map, g: Guid) -> bool {
    map.creature(g)
        .and_then(|c| map.realm.data.creature_templates.get(&c.entry))
        .is_some_and(|t| t.flags_extra & 0x40 != 0)
}

/// The waypoints a creature walks: a script's route, its spawn's or its
/// entry's.
pub fn waypoints(map: &Map, g: Guid) -> Option<Vec<crate::game::data::Waypoint>> {
    let c = map.creature(g)?;
    let points = match c.scripted.as_deref().and_then(|s| s.route.as_ref()) {
        Some(r) => (*r.points).clone(),
        None => c
            .spawn_guid
            .and_then(|s| map.realm.data.waypoints.get(&s))
            .or_else(|| map.realm.data.waypoints_by_entry.get(&c.entry))
            .cloned()?,
    };
    (!points.is_empty()).then_some(points)
}

/// A lap of a circle about `anchor`, from the bearing `from` is at, as
/// Thistle Tea's `CreatureMovement.circle`.
pub fn circle(anchor: Vector3d, radius: f32, from: Vector3d) -> Vec<Vector3d> {
    let count = ((radius * 2.0).ceil() as usize).clamp(8, 256);
    let start = (from.y - anchor.y).atan2(from.x - anchor.x);
    let mut out = vec![from];
    for i in 1..=count {
        let bearing = start + i as f32 * std::f32::consts::TAU / count as f32;
        out.push(Vector3d {
            x: anchor.x + radius * bearing.cos(),
            y: anchor.y + radius * bearing.sin(),
            z: anchor.z,
        });
    }
    out
}

/// A point to swim to within `radius` of `anchor`, at its depth, with
/// water over it.
fn swim_point(map: &mut Map, anchor: Vector3d, radius: f32) -> Option<Vector3d> {
    let mut rng = rand::rng();
    for _ in 0..4 {
        let a = rng.random_range(0.0..std::f32::consts::TAU);
        let d = rng.random_range(0.0..radius.max(1.0));
        let p = Vector3d {
            x: anchor.x + d * a.cos(),
            y: anchor.y + d * a.sin(),
            z: anchor.z,
        };
        if let Some((surface, floor)) = water(map, p) {
            let z = anchor.z.max(floor + FLOOR_TOLERANCE);
            if z <= surface - 2.0 {
                return Some(Vector3d { z, ..p });
            }
        }
    }
    None
}

/// A flyer that died falls to the ground under it.
pub fn fall_dead(map: &mut Map, g: Guid) {
    let Some(c) = map.creature(g) else {
        return;
    };
    let template = map.realm.data.creature_template(c.entry);
    if c.pet.is_some() || !template.is_some_and(template_flies) && c.move_flags & FLIGHT_FLAGS == 0
    {
        return;
    }
    let at = c.position;
    // To the highest ground under it.
    let Some(ground) = floor_under(map, at).filter(|z| *z < at.z - 0.1) else {
        return;
    };
    let height = at.z - ground;
    let time = fall_time(height);
    let to = Vector3d { z: ground, ..at };
    let now = map.now;
    let spline_id = map.next_spline_id;
    map.next_spline_id = map.next_spline_id.wrapping_add(1);
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    c.move_flags &= !FLIGHT_FLAGS;
    let mut walk = crate::game::map::creature::Walk::new(
        vec![at, to],
        height / time.max(0.05),
        now,
        spline_id,
        false,
    );
    walk.falling = true;
    let frame = walk.frame(g, at);
    c.position = to;
    c.motion = Motion::Idle { until: None };
    let (anchor, old) = (c.vis_anchor, c.cell);
    let cell = crate::game::map::grid::cell_of(to.x, to.y);
    c.cell = cell;
    map.grid.relocate(g, old, cell);
    crate::game::map::visibility::moved(map, g, anchor, to);
    if let Some(frame) = frame {
        map.send_to_observers(g, &frame, false);
    }
}

/// How long a fall of `height` yards takes (Thistle Tea's
/// `Falling.duration`): under gravity, no faster than its terminal
/// velocity, to the millisecond.
pub fn fall_time(height: f32) -> f32 {
    let terminal_time = TERMINAL_VELOCITY / GRAVITY;
    let terminal_distance = TERMINAL_VELOCITY * TERMINAL_VELOCITY / (2.0 * GRAVITY);
    let seconds = if height >= terminal_distance {
        (height - terminal_distance) / TERMINAL_VELOCITY + terminal_time
    } else {
        (2.0 * height / GRAVITY).sqrt()
    };
    (seconds * 1000.0).ceil() / 1000.0
}

/// How fast a creature's wounds let it run: 70, 60 and 50 percent below
/// 16, 11 and 6 percent of its health. Pets, world bosses and those
/// whose template says so run whole.
pub fn wounded_multiplier(map: &Map, g: Guid) -> f32 {
    let Some(c) = map.creature(g) else {
        return 1.0;
    };
    if c.pet.is_some() || c.owner.is_some() {
        return 1.0;
    }
    if map
        .realm
        .data
        .creature_template(c.entry)
        .is_some_and(|t| t.rank == 3 || t.static_flags2 & STATIC2_NO_WOUNDED_SLOWDOWN != 0)
    {
        return 1.0;
    }
    band(
        c.fields.get_u32(UNIT_FIELD_HEALTH),
        c.fields.get_u32(UNIT_FIELD_MAXHEALTH),
    )
    .0
}

/// A health's band: the speed it allows, and the aura state bits it
/// shows (health under 15, 10 and 5 percent).
pub fn band(health: u32, max: u32) -> (f32, u32) {
    if health == 0 || max == 0 {
        return (1.0, 0);
    }
    let (h, m) = (u64::from(health) * 100, u64::from(max));
    if h < m * 6 {
        (0.5, 0x700)
    } else if h < m * 11 {
        (0.6, 0x300)
    } else if h < m * 16 {
        (0.7, 0x100)
    } else {
        (1.0, 0)
    }
}

/// A creature's health changed: its run speed follows its wounds.
pub fn health_changed(map: &mut Map, g: Guid) {
    if !guid::is_unit(g) {
        return;
    }
    let factor = wounded_multiplier(map, g);
    let Some(c) = map.creature(g) else {
        return;
    };
    if (c.wounded - factor).abs() < 0.001 {
        return;
    }
    if let Some(c) = map.creature_mut(g) {
        c.wounded = factor;
    }
    // Its run goes on at its new speed (`derive::unit`'s retime).
    crate::game::map::derive::unit(map, g);
}

/// Whether a creature heeds a distraction (Thistle Tea's
/// `Distraction.responsive?`): alive, out of a fight and going nowhere of
/// its own, not feared, stunned, confused or feigning death.
fn heeds_distraction(map: &Map, g: Guid) -> bool {
    use crate::game::map::auras::{
        AURA_FEIGN_DEATH, AURA_MOD_CONFUSE, AURA_MOD_FEAR, AURA_MOD_STUN, AURA_PREVENTS_FLEEING,
    };
    map.creature(g).is_some_and(|c| {
        let a = &c.auras;
        c.alive()
            && c.pet.is_none()
            && !c.evading
            && !c.combat.attacking
            && !c.fields.has_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            )
            && !(a.has(AURA_MOD_FEAR) && !a.has(AURA_PREVENTS_FLEEING))
            && !a.has(AURA_MOD_STUN)
            && !a.has(AURA_MOD_CONFUSE)
            && !a.has(AURA_FEIGN_DEATH)
    })
}

/// Distract (spell effect 69), as Thistle Tea's `Distraction.apply`: a
/// creature that heeds it stops where it is, turns to `at`, its watchers
/// told, and stands for the effect's `seconds`, its wandering or patrol
/// waiting (the point it was bound for kept). One that never moves pays it
/// no heed.
pub fn distract(map: &mut Map, g: Guid, at: Vector3d, seconds: i32) {
    if seconds <= 0 || !heeds_distraction(map, g) || sessile(map, g) {
        return;
    }
    let now = map.now;
    let until = now + Duration::from_secs(seconds as u64);
    crate::game::map::ai::halt(map, g);
    let spline_id = map.next_spline_id;
    map.next_spline_id = map.next_spline_id.wrapping_add(1);
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    let pos = c.position;
    if at.x != pos.x || at.y != pos.y {
        c.orientation = (at.y - pos.y).atan2(at.x - pos.x);
    }
    let o = c.orientation;
    c.engagement.distracted_until = Some(until);
    c.motion = Motion::Idle { until: Some(until) };
    let frame = crate::game::map::ai::face_packet(g, pos, o, spline_id);
    map.send_to_observers(g, &frame, false);
}

/// Whether a distraction holds a creature still now. It ends with its
/// time, or as soon as the creature no longer heeds it (a fight, fear).
pub fn distracted(map: &mut Map, g: Guid) -> bool {
    let now = map.now;
    let until = map.creature(g).and_then(|c| c.engagement.distracted_until);
    let Some(until) = until else {
        return false;
    };
    if until > now && heeds_distraction(map, g) {
        return true;
    }
    if let Some(c) = map.creature_mut(g) {
        c.engagement.distracted_until = None;
    }
    false
}

/// [`distracted`], without ending one.
pub fn held_by_distraction(map: &Map, g: Guid) -> bool {
    let now = map.now;
    map.creature(g)
        .and_then(|c| c.engagement.distracted_until)
        .is_some_and(|t| t > now)
        && heeds_distraction(map, g)
}

/// A player greets a creature or picks its gossip: its wandering or patrol
/// waits three minutes, unless it is fighting, dead, flying, on a
/// script's move, or its template keeps it going.
pub fn talk_pause(map: &mut Map, g: Guid) {
    let now = map.now;
    let Some(c) = map.creature(g) else {
        return;
    };
    let keeps_going = map
        .realm
        .data
        .creature_template(c.entry)
        .is_some_and(|t| t.flags_extra & EXTRA_NO_MOVEMENT_PAUSE != 0);
    let scripted = c
        .scripted
        .as_deref()
        .is_some_and(|s| s.scripted_move.is_some() || s.route.is_some() || s.follow.is_some());
    if !c.alive()
        || c.combat.attacking
        || c.evading
        || keeps_going
        || scripted
        || habitat(map, g).fly
        || !matches!(c.movement_type, MOVEMENT_RANDOM | MOVEMENT_WAYPOINT)
    {
        return;
    }
    let moving = matches!(c.motion, Motion::Moving(_));
    if moving {
        crate::game::map::ai::advance_motion(map, g);
    }
    let spline_id = map.next_spline_id;
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    // It only ever waits longer (vmangos's `AddPauseTime`): a longer wait
    // already set stands.
    let until = match c.motion {
        Motion::Idle { until: Some(t) } if t > now + TALK_PAUSE => t,
        _ => now + TALK_PAUSE,
    };
    c.motion = Motion::Idle { until: Some(until) };
    let at = c.position;
    if moving {
        map.next_spline_id = map.next_spline_id.wrapping_add(1);
        let frame = crate::game::map::ai::stop_packet(g, at, spline_id);
        map.send_to_observers(g, &frame, false);
    }
}

/// Whether a creature is an ordinary critter: a critter by its type, with
/// no AI of its own.
pub fn critter(map: &Map, g: Guid) -> bool {
    let Some(c) = map.creature(g) else {
        return false;
    };
    if c.pet.is_some() || c.owner.is_some() {
        return false;
    }
    let data = &map.realm.data;
    data.creature_template(c.entry).is_some_and(|t| {
        t.creature_type == 8
            && matches!(t.ai_name.as_str(), "" | "CritterAI")
            && data
                .scripts
                .ai_events
                .get(&c.entry)
                .is_none_or(|e| e.is_empty())
    })
}

/// A critter hurt or cursed by `source`: it runs from it, for thirty
/// seconds from the last of it; an escape under way isn't begun again.
pub fn critter_hit(map: &mut Map, g: Guid, source: Guid) {
    let now = map.now;
    if source == g || !critter(map, g) || !map.creature(g).is_some_and(|c| c.alive()) {
        return;
    }
    map.rare.flee = true;
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    c.engagement.escape_at = Some(now + ESCAPE);
    if c.engagement.flee.is_some_and(|f| f.until > now) {
        return;
    }
    c.engagement.flee = Some(crate::game::map::creature_combat::Flee {
        from: source,
        until: now + ESCAPE,
        helper: None,
        called_at: None,
        scripted: false,
    });
    c.engagement.next_move_at = Some(now);
    // Held from fleeing, its escape is kept but it doesn't run, nor show
    // it, until it may (Thistle Tea's critter under prevent fleeing).
    let held = c.auras.has(crate::game::map::auras::AURA_PREVENTS_FLEEING);
    c.fields.set_flag(
        UNIT_FIELD_FLAGS,
        crate::game::map::auras::UNIT_FLAG_FLEEING,
        !held,
    );
}

/// A critter's fight: it runs, then waits out its escape, then goes home.
/// True if it was one.
pub fn critter_step(map: &mut Map, g: Guid) -> bool {
    if !critter(map, g) {
        return false;
    }
    let now = map.now;
    let escape = map.creature(g).and_then(|c| c.engagement.escape_at);
    match escape {
        Some(t) if t > now => {}
        _ => {
            if let Some(c) = map.creature_mut(g) {
                c.engagement.escape_at = None;
            }
            crate::game::map::ai::enter_evade(map, g);
        }
    }
    true
}
