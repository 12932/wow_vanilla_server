//! Where a leap ends: Blink's way (decision 109).
//!
//! A leap goes as far as its caster could walk straight ahead on the
//! ground, up to its radius, looked at every quarter of a yard, as
//! mangos-classic's `TARGET_LOCATION_CASTER_FRONT_LEAP` walks it every two
//! yards (`Spell::SetTargetMap`). Each look takes the floor there no
//! higher than a step's climb over the last (so never a roof, a ledge or a
//! wall's top above it) and no lower than a step's drop (never off a
//! ledge), with room to stand over it; over each yard the way climbs or
//! drops no more steeply than 50° (mangos's `maxSlope`, the client's own
//! walkable slope); and the way between two looks, at a knee's and a
//! head's height, is clear of the bake's WMOs and doodads (met from either
//! side, as VMaps meet them), of the terrain, and of the game objects in
//! the way, a closed door among them (mangos's dynamic tree). Where a look
//! fails it is narrowed to the farthest that holds, and the leap ends half
//! a yard short of that. Into deep water it swims on at a swimmer's depth,
//! and out of it onto ground a step over where it swims; one swimming
//! keeps its depth. One falling falls on: mangos-classic's falling case,
//! two yards lower ahead, onto ground within ten yards under that, short of
//! what stands in the way, never higher than it was.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::map::Map;
use crate::game::map::obstacles::Obstacles;

/// How far each look along the way is from the last. mangos's `step` is
/// two yards, at which a crate's side, a bench, a door's frame or a gap is
/// stepped across unseen.
pub const LOOK: f32 = 0.25;
/// The most one look climbs or drops from the last: a step's (the bake's
/// walkable climb, namigator's), a stair's riser and more, not a crate's
/// side or a ledge's face.
pub const CLIMB: f32 = 1.0;
/// The most the way climbs or drops over a yard: 50°, tan 50° yards
/// (mangos's `maxSlope`; the 1.12.1 client's walkable faces, its float at
/// `0x80DFFC` being cos 50°).
pub const RISE: f32 = 1.191_753_6;
/// The room a floor needs over it to be stood on: nothing that faces up
/// within this over it (a table's top, a crate's lid, a parapet's); the
/// bake's walkable height.
pub const HEADROOM: f32 = 1.6;
/// The heights over the way's floor at which what stands across it is
/// looked for: a knee's a yard up, a step's climb over the floor; a
/// chest's, for a rail or a beam between; and a head's (the bake's
/// walkable height). mangos-classic looks at 1.5 yards alone, which leaves
/// a rock or a crate lower than that to its slope test, two yards at a
/// time.
pub const BODY: [f32; 3] = [1.0, 1.3, 1.6];
/// How far short of what stops it a leap ends (mangos's hit tests take
/// half a yard off, `GetHitPosition(..., -0.5f)`; the navmesh vmangos
/// walks keeps its agent's half yard off walls and edges).
pub const CLEARANCE: f32 = 0.5;
/// A caster no more than this over its floor is on it; one higher but
/// within [`NEAR_FLOOR`] was jumping close over it, and leaps from it
/// (mangos's "fix origin position if player was jumping and near of the
/// ground": `GetHeightInRange(..., 3.0f)`, 0.5).
const ON_FLOOR: f32 = 0.5;
const NEAR_FLOOR: f32 = 3.0;
/// How far under the surface a leap into water, or on it, puts its caster
/// (mangos's `IN_OR_UNDER_LIQUID_RANGE`).
pub const SWIM_DEPTH: f32 = 0.8;
/// Water this deep is swum in (`creature_motion`'s).
const MIN_DEPTH: f32 = 1.5;
/// The most a walker is over (or under) the water's surface and still goes
/// into it: from a jetty higher, it stops at the edge (mangos's "on edge of
/// water with difference a bit to high to continue", 2 yards).
const WATER_EDGE: f32 = 2.0;
/// Falling: the leap ends this much lower ahead (mangos's "little hack to
/// avoid the impression to go up"), on the ground within [`FALL_GROUND`]
/// under that, short of what stands in the way from half a yard over the
/// caster ([`FALL_EYE`]).
const FALL_DROP: f32 = 2.0;
const FALL_GROUND: f32 = 10.0;
const FALL_EYE: f32 = 0.5;
/// How many halvings a failed look is narrowed by (to a 64th of a yard),
/// and the falling way's first hit.
const REFINE: usize = 4;
const REFINE_FALL: usize = 12;

#[cfg(test)]
thread_local! {
    /// What a leap's way looked at, when a test asks (`trace`).
    static TRACE: std::cell::RefCell<Option<Vec<String>>> = const { std::cell::RefCell::new(None) };
}

/// A line of the trace, if one is being kept.
#[cfg(test)]
fn note(f: impl FnOnce() -> String) {
    TRACE.with(|t| {
        if let Some(t) = t.borrow_mut().as_mut() {
            t.push(f());
        }
    });
}

#[cfg(not(test))]
fn note(_: impl FnOnce() -> String) {}

/// [`leap_end`], and what its way looked at.
#[cfg(test)]
pub(crate) fn trace(
    map: &mut Map,
    caster: Guid,
    from: Vector3d,
    angle: f32,
    dist: f32,
) -> (Vector3d, Vec<String>) {
    TRACE.with(|t| *t.borrow_mut() = Some(Vec::new()));
    let end = leap_end(map, caster, from, angle, dist);
    (
        end,
        TRACE.with(|t| t.borrow_mut().take().unwrap_or_default()),
    )
}

/// Why a look failed: no floor to go on to (a crack, or a ledge's edge),
/// or something else (what stands in the way, no room to stand).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Miss {
    Floor,
    Other,
}

/// A place on the way: where, how far along it, and whether swimming.
#[derive(Debug, Clone, Copy)]
struct Point {
    at: Vector3d,
    d: f32,
    swim: bool,
}

struct Way {
    from: Vector3d,
    cos: f32,
    sin: f32,
    obstacles: Obstacles,
    floors: Vec<f32>,
}

impl Way {
    fn xy(&self, d: f32) -> (f32, f32) {
        (self.from.x + d * self.cos, self.from.y + d * self.sin)
    }

    /// The floors at `(x, y)` from `high` down to `low`, top first: the
    /// bake's surfaces and the objects'. False where nothing is known yet
    /// (a tile loading, or none). With no navmesh, the caster's own height.
    fn floors_at(&mut self, map: &mut Map, x: f32, y: f32, low: f32, high: f32) -> bool {
        self.floors.clear();
        match map.nav.as_mut() {
            Some(nav) => match nav.surfaces(x, y) {
                Some(s) => self
                    .floors
                    .extend(s.iter().copied().filter(|h| (low..=high).contains(h))),
                None => return false,
            },
            None => self.floors.push(self.from.z),
        }
        self.obstacles.surfaces(x, y, low, high, &mut self.floors);
        self.floors.sort_by(|a, b| b.total_cmp(a));
        true
    }

    /// The surface over `f`, of those [`Way::floors_at`] found, that leaves
    /// too little room to stand on it ([`HEADROOM`]), if one does.
    fn room(&self, f: f32) -> Option<f32> {
        self.floors
            .iter()
            .rev()
            .find(|h| **h > f + 0.05)
            .copied()
            .filter(|over| *over < f + HEADROOM)
    }

    /// The highest floor no higher than `top`, of those [`Way::floors_at`]
    /// found.
    fn floor_under(&self, top: f32) -> Option<f32> {
        self.floors.iter().copied().find(|h| *h <= top)
    }

    /// Whether anything stands across the segment from `a` to `b`: a test's
    /// walls, the bake's WMOs and doodads, the terrain, the objects in the
    /// way.
    fn blocked(&self, map: &mut Map, a: Vector3d, b: Vector3d) -> bool {
        // A test's wall is met by a way that ends on it, too: carried a
        // hair past its end, so that a look landing on the wall's line is
        // short of it, not through.
        let len = (b.x - a.x).hypot(b.y - a.y).max(1e-6);
        let past = Vector3d {
            x: b.x + (b.x - a.x) / len * 0.01,
            y: b.y + (b.y - a.y) / len * 0.01,
            ..b
        };
        if map
            .spells
            .walls
            .iter()
            .any(|(p, q)| super::checks::crosses(a, past, *p, *q))
        {
            return true;
        }
        if self.obstacles.crossed(a, b) {
            return true;
        }
        map.nav
            .as_mut()
            .is_some_and(|nav| nav.crossed(a, b) || nav.into_terrain(a, b))
    }

    /// The water's surface at `(x, y)` about `z`, if any.
    fn surface(map: &mut Map, x: f32, y: f32, z: f32) -> Option<f32> {
        let p = Vector3d { x, y, z };
        map.nav
            .as_mut()
            .and_then(|n| n.liquid_surface(p))
            .or_else(|| crate::game::map::environment::liquid_at(map, p).map(|l| l.surface))
    }

    /// The place `d` along the way, on from `prev` (and `back`, the place a
    /// yard or more behind it, or the start): on the floor there a step's
    /// climb or drop from `prev`, and within the slope's of `back`, with
    /// room over it; or swimming, in water deep enough. None if there is no
    /// such floor (a wall, a ledge's face or its edge, a cliff, a jetty over
    /// the water, a crate's side), or anything stands across the way from
    /// `prev` at a knee's or a head's height.
    fn next(&mut self, map: &mut Map, prev: &Point, back: &Point, d: f32) -> Result<Point, Miss> {
        let (x, y) = self.xy(d);
        let top = prev.at.z + CLIMB;
        if !self.floors_at(map, x, y, prev.at.z - 100.0, top + HEADROOM) {
            note(|| format!("{d:.3}: nothing known"));
            return Err(Miss::Other);
        }
        let floor = self.floor_under(top + 0.01);
        let water = Self::surface(map, x, y, prev.at.z).filter(|s| {
            // Deep enough over the floor here, or with none near.
            floor.is_none_or(|f| s - f >= MIN_DEPTH)
                && (prev.swim || (prev.at.z - s).abs() <= WATER_EDGE)
        });
        let (z, swim) = if let Some(s) = water {
            let z = if prev.swim {
                prev.at.z.min(s - SWIM_DEPTH)
            } else {
                s - SWIM_DEPTH
            };
            (floor.map_or(z, |f| z.max(f)), true)
        } else {
            let Some(f) = floor.filter(|f| *f >= prev.at.z - CLIMB) else {
                note(|| {
                    format!(
                        "{d:.3}: no floor from {:.2} (floors {:?})",
                        prev.at.z, self.floors
                    )
                });
                return Err(Miss::Floor);
            };
            let slope = RISE * (d - back.d).max(1.0);
            if !back.swim && (f - back.at.z).abs() > slope + 0.01 {
                note(|| {
                    format!(
                        "{d:.3}: too steep, {f:.2} from {:.2} a yard back",
                        back.at.z
                    )
                });
                return Err(Miss::Other);
            }
            // Room to stand: nothing over it that faces up, within a
            // man's height (a parapet's top, a table's).
            if let Some(over) = self.room(f) {
                note(|| format!("{d:.3}: no room on {f:.2} under {over:.2}"));
                return Err(Miss::Other);
            }
            // And half a look back, for a beam or a shelf's edge between.
            let (mx, my) = self.xy((prev.d + d) / 2.0);
            let (low, high) = (prev.at.z.min(f), prev.at.z.max(f));
            if self.floors_at(map, mx, my, low - CLIMB, high + HEADROOM)
                && let Some(m) = self.floor_under(high + 0.01)
                && let Some(over) = self.room(m)
            {
                note(|| format!("{d:.3}: no room half a look back on {m:.2} under {over:.2}"));
                return Err(Miss::Other);
            }
            (f, false)
        };
        let at = Vector3d { x, y, z };
        for h in BODY {
            let up = |p: Vector3d| Vector3d { z: p.z + h, ..p };
            if self.blocked(map, up(prev.at), up(at)) {
                note(|| format!("{d:.3}: blocked {h} up, {:.2} to {z:.2}", prev.at.z));
                return Err(Miss::Other);
            }
        }
        note(|| {
            format!(
                "{d:.3}: ({x:.2}, {y:.2}, {z:.2}){}",
                if swim { " swim" } else { "" }
            )
        });
        Ok(Point { at, d, swim })
    }
}

/// Where a leap of `caster` from `from`, `dist` yards toward `angle`, ends
/// (the module's rule).
pub(crate) fn leap_end(
    map: &mut Map,
    caster: Guid,
    from: Vector3d,
    angle: f32,
    dist: f32,
) -> Vector3d {
    if !(dist > 0.0 && dist.is_finite() && angle.is_finite())
        || !(from.x.is_finite() && from.y.is_finite() && from.z.is_finite())
    {
        return from;
    }
    let flags = map.players.get(&caster).map(|p| p.movement.flags);
    let swim_flag = flags.is_some_and(|f| f.get_swimming().is_some());
    let falling = flags.is_some_and(|f| f.get_jumping().is_some() || f.get_fallingfar());
    // Swimming by the client's flag, or under the water's surface (Thistle
    // Tea's `swim_destination`), for a creature's leap too.
    let swimming = swim_flag
        || crate::game::map::creature_motion::water(map, from)
            .is_some_and(|(surface, _)| from.z < surface);
    let mut way = Way {
        from,
        cos: angle.cos(),
        sin: angle.sin(),
        obstacles: Obstacles::along(
            map,
            from,
            Vector3d {
                x: from.x + (dist + CLEARANCE) * angle.cos(),
                y: from.y + (dist + CLEARANCE) * angle.sin(),
                ..from
            },
            1.0,
        ),
        floors: Vec::new(),
    };
    let start = if swimming {
        Point {
            at: from,
            d: 0.0,
            swim: true,
        }
    } else {
        // Its floor: the one nearest it, within half a yard over it or
        // three under (one jumping).
        let ground = way
            .floors_at(map, from.x, from.y, from.z - NEAR_FLOOR, from.z + ON_FLOOR)
            .then(|| {
                way.floors
                    .iter()
                    .copied()
                    .min_by(|a, b| (a - from.z).abs().total_cmp(&(b - from.z).abs()))
            })
            .flatten();
        match ground {
            Some(g) => Point {
                at: Vector3d { z: g, ..from },
                d: 0.0,
                swim: false,
            },
            None if falling => return fall(map, &mut way, from, dist),
            // In the air with no fall under way, or over nothing known (a
            // tile not in yet, a ship's deck): it stays.
            None => return from,
        }
    };
    walk(map, &mut way, start, dist)
}

/// The place a yard or more behind `d` of those walked, or the start.
fn behind(points: &[Point], d: f32) -> Point {
    points
        .iter()
        .rev()
        .find(|p| p.d <= d - 1.0 + 1e-4)
        .copied()
        .unwrap_or(points[0])
}

/// [`leap_end`] on the ground, or swimming, from `start`.
fn walk(map: &mut Map, way: &mut Way, start: Point, dist: f32) -> Vector3d {
    // As far as half a yard beyond its end, so that it ends that far short
    // of what stands there.
    let total = dist + CLEARANCE;
    let mut points = vec![start];
    let mut reach = total;
    let n = (total / LOOK).ceil() as usize;
    let mut i = 0;
    while i < n {
        i += 1;
        let d = (i as f32 * LOOK).min(total);
        let last = points[points.len() - 1];
        let back = behind(&points, d);
        let miss = match way.next(map, &last, &back, d) {
            Ok(p) => {
                points.push(p);
                continue;
            }
            Err(miss) => miss,
        };
        // A crack in the floor a look wide (between a floor's planks, a
        // bridge's sections) is stepped over: the look beyond, from the
        // same place, the way to it clear.
        if miss == Miss::Floor && i < n {
            let over = ((i + 1) as f32 * LOOK).min(total);
            if let Ok(p) = way.next(map, &last, &behind(&points, over), over) {
                note(|| format!("{d:.3}: stepped over"));
                points.push(p);
                i += 1;
                continue;
            }
        }
        // The farthest that holds between.
        let (mut lo, mut hi, mut best) = (last.d, d, None);
        for _ in 0..REFINE {
            let mid = (lo + hi) / 2.0;
            match way.next(map, &last, &behind(&points, mid), mid) {
                Ok(p) => {
                    lo = mid;
                    best = Some(p);
                }
                Err(_) => hi = mid,
            }
        }
        points.extend(best);
        reach = lo;
        break;
    }
    let end = if reach >= total - 1e-4 {
        dist
    } else {
        (reach - CLEARANCE).max(0.0)
    };
    let base = points
        .iter()
        .rev()
        .find(|p| p.d <= end + 1e-4)
        .copied()
        .unwrap_or(start);
    if end - base.d < 1e-3 {
        return base.at;
    }
    let back = behind(&points, end);
    way.next(map, &base, &back, end).map_or(base.at, |p| p.at)
}

/// [`leap_end`] by one falling with no ground under it (mangos-classic's
/// falling case): `dist` ahead and [`FALL_DROP`] lower; on the water there
/// (a swimmer's depth under its surface) if that is within
/// [`FALL_GROUND`], else on the ground within it under that (never over the
/// caster); short of the first thing the way there from half a yard over
/// the caster meets, by half a yard; never over the caster's height.
fn fall(map: &mut Map, way: &mut Way, from: Vector3d, dist: f32) -> Vector3d {
    let (x, y) = way.xy(dist);
    let mut next = Vector3d {
        x,
        y,
        z: from.z - FALL_DROP,
    };
    let surface = Way::surface(map, x, y, next.z).filter(|s| (next.z - s).abs() < FALL_GROUND);
    if let Some(s) = surface.filter(|s| next.z < *s) {
        next.z = s - SWIM_DEPTH;
    } else if way.floors_at(map, x, y, next.z - FALL_GROUND, from.z)
        && let Some(g) = way.floor_under(from.z)
    {
        next.z = g;
    }
    let eye = Vector3d {
        z: from.z + FALL_EYE,
        ..from
    };
    let along = |t: f32| Vector3d {
        x: eye.x + (next.x - eye.x) * t,
        y: eye.y + (next.y - eye.y) * t,
        z: eye.z + (next.z - eye.z) * t,
    };
    let t = if way.blocked(map, eye, next) {
        let (mut clear, mut hit) = (0.0f32, 1.0f32);
        for _ in 0..REFINE_FALL {
            let mid = (clear + hit) / 2.0;
            if way.blocked(map, eye, along(mid)) {
                hit = mid;
            } else {
                clear = mid;
            }
        }
        let length =
            ((next.x - eye.x).powi(2) + (next.y - eye.y).powi(2) + (next.z - eye.z).powi(2))
                .sqrt()
                .max(1e-3);
        note(|| format!("falling: hit at {hit:.3} of the way"));
        (clear - CLEARANCE / length).max(0.0)
    } else {
        1.0
    };
    let at = along(t);
    Vector3d {
        z: at.z.min(from.z),
        ..at
    }
}
