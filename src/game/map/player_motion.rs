//! Players the server moves: feared (Fear, Intimidating Shout, Psychic
//! Scream, Howl of Terror), confused (Scatter Shot's stagger), or charmed
//! by a creature, as Thistle Tea's `ControlMovement` drives them. The
//! client is told it no longer controls its character
//! (`SMSG_CLIENT_CONTROL_UPDATE`), the server walks it with
//! `SMSG_MONSTER_MOVE` for its own guid, and what its client sends is
//! ignored until control comes back.
//!
//! Fear runs at a random angle, further the nearer the one feared is,
//! pausing between runs; confusion staggers about where it began; a charmed
//! player closes on its charmer's foe, or follows its charmer.

use std::time::{Duration, Instant};

use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::map::Map;
use crate::game::map::auras::*;
use crate::game::map::combat::{self, dist};
use crate::game::map::creature::Walk;

/// Confusion staggers within this of where it began.
pub const CONFUSION_RADIUS: f32 = 4.0;
/// A charmed player follows its charmer to this.
const FOLLOW_DISTANCE: f32 = 3.0;
/// How often a charmed player aims again at a foe on the move.
const CHARM_REPLAN: Duration = Duration::from_millis(500);

/// Why the server moves a player.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    Fear,
    Confusion { anchor: Vector3d },
    Charm,
}

/// A player the server moves.
#[derive(Debug, Clone)]
pub struct Driven {
    pub mode: Mode,
    pub walk: Option<Walk>,
    pub next_move_at: Instant,
}

/// Thistle Tea's `Fear.destination_request`: a run at a random angle,
/// longer the nearer the frightener, at most 25 yards.
pub fn fear_destination(
    at: Vector3d,
    frightener_distance: f32,
    rng: &mut impl rand::Rng,
) -> Vector3d {
    let angle = rng.random_range(0.0f32..std::f32::consts::TAU);
    let roll = rng.random_range(0.0f32..1.0);
    let d = frightener_distance;
    let length = if d < 28.0 {
        (0.4 + roll * 0.9) * (28.0 - d)
    } else if d > 38.0 {
        (0.4 + roll * 0.6) * 10.0
    } else {
        (0.6 + roll * 0.6) * 10.0
    };
    let length = length.min(25.0);
    Vector3d {
        x: at.x + angle.cos() * length,
        y: at.y + angle.sin() * length,
        z: at.z,
    }
}

/// How a player's auras and control would have them moved now.
fn wanted(map: &Map, g: Guid) -> Option<Mode> {
    let p = map.players.get(&g)?;
    if p.death != crate::game::map::death::Death::Alive {
        return None;
    }
    let a = &p.auras;
    if a.has(AURA_MOD_CONFUSE) {
        let anchor = match p.driven.as_ref().map(|d| d.mode) {
            Some(Mode::Confusion { anchor }) => anchor,
            _ => p.position(),
        };
        return Some(Mode::Confusion { anchor });
    }
    if a.has(AURA_MOD_FEAR) && !a.has(AURA_PREVENTS_FLEEING) {
        return Some(Mode::Fear);
    }
    if p.owned.possessed_by.is_some_and(|pos| !pos.manual) {
        return Some(Mode::Charm);
    }
    None
}

/// Whether the server moves this player: its client's movement is
/// ignored.
pub fn driven(map: &Map, g: Guid) -> bool {
    map.players.get(&g).is_some_and(|p| p.driven.is_some())
}

/// Bring a player's control in line with its auras and charm: the server
/// takes its movement, or gives it back.
pub fn control(map: &mut Map, g: Guid) {
    let now = map.now;
    let want = wanted(map, g);
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let had = p.driven.as_ref().map(|d| d.mode);
    let same_kind = match (had, want) {
        (Some(Mode::Confusion { .. }), Some(Mode::Confusion { .. })) => true,
        (a, b) => a == b,
    };
    if same_kind {
        return;
    }
    stop(map, g);
    let possessed = map
        .players
        .get(&g)
        .is_some_and(|p| p.owned.possessed_by.is_some());
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    match want {
        Some(mode) => {
            // Taken from its client (again, if a charm just gave it back).
            if (had.is_none() || had == Some(Mode::Charm)) && !possessed {
                p.outbound.send(&SMSG_CLIENT_CONTROL_UPDATE {
                    guid: g,
                    allow_movement: false,
                });
            }
            p.driven = Some(Driven {
                mode,
                walk: None,
                next_move_at: now,
            });
        }
        None => {
            p.driven = None;
            // Where the server left it is where its client starts again.
            let at = p.position();
            p.move_check = Some((at, now));
            if !possessed {
                p.outbound.send(&SMSG_CLIENT_CONTROL_UPDATE {
                    guid: g,
                    allow_movement: true,
                });
            }
        }
    }
}

/// Stop a walk under way, where the player now is.
fn stop(map: &mut Map, g: Guid) {
    advance(map, g);
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let Some(d) = p.driven.as_mut() else {
        return;
    };
    if d.walk.take().is_none() {
        return;
    }
    let at = p.position();
    let spline_id = map.next_spline_id;
    map.next_spline_id = map.next_spline_id.wrapping_add(1);
    let frame = crate::game::map::ai::stop_packet(g, at, spline_id);
    map.send_to_observers(g, &frame, true);
}

/// The player along its walk at `now`: position, cell and what it sees.
fn advance(map: &mut Map, g: Guid) {
    let now = map.now;
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let Some(w) = p.driven.as_mut().and_then(|d| d.walk.as_ref()) else {
        return;
    };
    let done = w.done(now);
    let (pos, o) = if done {
        (w.destination(), p.movement.orientation)
    } else {
        w.position_at(now)
    };
    if done && let Some(d) = p.driven.as_mut() {
        d.walk = None;
    }
    p.movement.position = pos;
    p.movement.orientation = o;
    let (anchor, old_cell) = (p.vis_anchor, p.cell);
    let new_cell = crate::game::map::grid::cell_of(pos.x, pos.y);
    p.cell = new_cell;
    map.grid.relocate(g, old_cell, new_cell);
    crate::game::map::visibility::moved(map, g, anchor, pos);
}

/// Walk a player along `points`, its own client and its watchers told.
fn walk(map: &mut Map, g: Guid, points: Vec<Vector3d>, run: bool, pause: Duration) {
    let now = map.now;
    if points.len() < 2 {
        return;
    }
    let spline_id = map.next_spline_id;
    map.next_spline_id = map.next_spline_id.wrapping_add(1);
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let speed = if run { p.speeds.run } else { p.speeds.walk };
    let w = Walk::new(points, speed, now, spline_id, run);
    let frame = crate::game::conn::encode(&w.packet(g, p.position()));
    if let Some(d) = p.driven.as_mut() {
        // The pause comes after the run.
        d.next_move_at = now + w.duration + pause;
        d.walk = Some(w);
    }
    if let Some(frame) = frame {
        map.send_to_observers(g, &frame, true);
    }
}

fn path(map: &mut Map, from: Vector3d, to: Vector3d) -> Vec<Vector3d> {
    match map.nav.as_mut() {
        Some(nav) => nav.path(from, to).unwrap_or_else(|| vec![from, to]),
        None => vec![from, to],
    }
}

/// The players the server moves take their next steps.
pub fn update(map: &mut Map) {
    let now = map.now;
    let driven: Vec<Guid> = map
        .players
        .values()
        .filter(|p| p.driven.is_some())
        .map(|p| p.guid)
        .collect();
    for g in driven {
        // Control comes back when what took it has gone.
        control(map, g);
        advance(map, g);
        let Some(p) = map.players.get(&g) else {
            continue;
        };
        let Some(d) = p.driven.as_ref() else {
            continue;
        };
        // Stunned or rooted: it stands where it is.
        if p.auras.has(AURA_MOD_STUN) || p.auras.has(AURA_MOD_ROOT) {
            if d.walk.is_some() {
                stop(map, g);
            }
            continue;
        }
        // A charmed player chasing a foe aims again as it goes.
        let replan = d.mode == Mode::Charm
            && d.walk
                .as_ref()
                .is_some_and(|w| now.duration_since(w.started) >= CHARM_REPLAN);
        if (d.walk.is_some() && !replan) || d.next_move_at > now {
            continue;
        }
        let (mode, at) = (d.mode, p.position());
        let mut rng = rand::rng();
        match mode {
            Mode::Fear => {
                let source = p
                    .auras
                    .holders
                    .iter()
                    .find(|h| h.has(AURA_MOD_FEAR))
                    .map(|h| h.caster);
                let distance = source
                    .and_then(|s| map.position_of(s))
                    .map_or(0.0, |s| dist(s, at));
                let to = fear_destination(at, distance, &mut rng);
                let route = path(map, at, to);
                let pause = Duration::from_millis(rng.random_range(800..1500));
                walk(map, g, route, true, pause);
            }
            Mode::Confusion { anchor } => {
                let a = rng.random_range(0.0f32..std::f32::consts::TAU);
                let r = rng.random_range(0.5f32..CONFUSION_RADIUS);
                let to = Vector3d {
                    x: anchor.x + r * a.cos(),
                    y: anchor.y + r * a.sin(),
                    z: anchor.z,
                };
                let route = path(map, at, to);
                let pause = Duration::from_millis(rng.random_range(500..1500));
                walk(map, g, route, false, pause);
            }
            Mode::Charm => charm_step(map, g),
        }
    }
}

/// A creature-charmed player closes on the foe it fights, or follows its
/// charmer (Thistle Tea's `BT.Charm`).
fn charm_step(map: &mut Map, g: Guid) {
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let Some(master) = p.owned.possessed_by.map(|pos| pos.master) else {
        return;
    };
    let melee = matches!(p.row.class, 1 | 2 | 4 | 11);
    let fighting = p.combat.attacking.then_some(p.combat.victim).or_else(|| {
        p.caster
            .casting
            .as_ref()
            .and_then(|c| c.targets.unit)
            .filter(|t| *t != g)
    });
    let (Some(me), at) = (map.unit_view(g), p.position()) else {
        return;
    };
    let goal = match fighting.and_then(|t| map.unit_view(t)) {
        Some(them) => {
            let reach = if melee || p.combat.attacking {
                combat::melee_range(&me, &them) * 0.8
            } else {
                25.0
            };
            if dist(at, them.position) <= reach {
                None
            } else {
                Some(towards(them.position, at, reach))
            }
        }
        None => map
            .position_of(master)
            .filter(|m| dist(*m, at) > FOLLOW_DISTANCE + 1.0)
            .map(|m| towards(m, at, FOLLOW_DISTANCE)),
    };
    let Some(goal) = goal else {
        // There: it stands to fight.
        stop(map, g);
        return;
    };
    let route = path(map, at, goal);
    walk(map, g, route, true, Duration::ZERO);
}

/// A point `reach` short of `target`, on the side `from` comes from.
fn towards(target: Vector3d, from: Vector3d, reach: f32) -> Vector3d {
    let (dx, dy) = (from.x - target.x, from.y - target.y);
    let len = (dx * dx + dy * dy).sqrt().max(0.01);
    let r = reach.min(len);
    Vector3d {
        x: target.x + dx / len * r,
        y: target.y + dy / len * r,
        z: target.z,
    }
}
