//! The world in a map: its spawns, the creatures that move about in it,
//! the zones and areas players are in, and its area triggers.

use std::time::{Duration, Instant};

use rand::RngExt;
use tracing::info;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::data::{AREA_FLAG_ARENA, AreaTrigger};
use crate::game::fields::PLAYER_FLAGS;
use crate::game::map::creature::{
    MOVEMENT_RANDOM, MOVEMENT_WAYPOINT, Motion, wander_pause,
};
use crate::game::map::gameobject::GameObject;
use crate::game::map::grid::cell_of;
use crate::game::map::nav::Nav;
use crate::game::map::player::PLAYER_FLAGS_FFA_PVP;
use crate::game::map::{Map, MapKey, visibility};

/// Load the map's navmesh and its spawns.
pub fn populate(map: &mut Map) {
    let started = Instant::now();
    let data = map.realm.data.clone();
    if let Some(dir) = map.realm.config.nav_dir.as_ref() {
        let name = data
            .maps
            .get(&map.key.map)
            .map(|m| m.internal_name.clone())
            .filter(|n| !n.is_empty())
            .or_else(|| {
                wow_world_messages::vanilla::Map::try_from(map.key.map)
                    .ok()
                    .map(|m| m.directory_name().to_owned())
            });
        if let Some(name) = name {
            map.nav = Nav::open(dir, &name);
        }
    }
    let on = crate::game::events::current(&map.realm);
    let mut rng = rand::rng();
    if let Some(spawns) = data.creature_spawns.get(&map.key.map) {
        for s in spawns {
            if !crate::game::events::spawned(data.event_creatures.get(&s.guid).copied(), &on) {
                continue;
            }
            if let Some(c) = crate::game::events::spawn_creature(&data, s, &on, &mut rng) {
                map.add_creature(c);
            }
        }
    }
    if let Some(spawns) = data.gameobject_spawns.get(&map.key.map) {
        for s in spawns {
            if !crate::game::events::spawned(data.event_objects.get(&s.guid).copied(), &on) {
                continue;
            }
            if let Some(t) = data.gameobject_templates.get(&s.entry) {
                map.add_gameobject(GameObject::from_spawn(t, s));
            }
        }
    }
    // Nobody is here yet to see any of it.
    map.visibility_dirty.clear();
    if !map.creatures.is_empty() || !map.gameobjects.is_empty() {
        info!(
            "{}: {} creatures, {} game objects, in {} ms",
            map.key,
            map.creatures.len(),
            map.gameobjects.len(),
            started.elapsed().as_millis()
        );
    }
}

/// Move the creatures some player sees.
pub fn update_creatures(map: &mut Map) {
    let now = map.now;
    let keys: Vec<usize> = map
        .creatures
        .iter()
        .filter(|(_, c)| !c.observers.is_empty())
        .map(|(k, _)| k)
        .collect();
    if keys.is_empty() {
        return;
    }
    let mut nav = map.nav.take();
    let mut rng = rand::rng();
    for key in keys {
        step_creature(map, key, now, nav.as_mut(), &mut rng);
    }
    map.nav = nav;
}

fn step_creature(
    map: &mut Map,
    key: usize,
    now: Instant,
    mut nav: Option<&mut Nav>,
    rng: &mut impl rand::Rng,
) {
    let waypoints = {
        let Some(c) = map.creatures.get(key) else {
            return;
        };
        if c.movement_type == MOVEMENT_WAYPOINT {
            c.spawn_guid
                .and_then(|g| map.realm.data.waypoints.get(&g))
                .cloned()
        } else {
            None
        }
    };
    let spline_id = map.next_spline_id;
    let Some(c) = map.creatures.get_mut(key) else {
        return;
    };
    let guid = c.guid;
    let mut frame = None;
    match &c.motion {
        Motion::Moving(w) => {
            let done = w.done(now);
            let (pos, o) = if done {
                (w.destination(), c.orientation)
            } else {
                w.position_at(now)
            };
            c.position = pos;
            c.orientation = o;
            if done {
                let pause = match &waypoints {
                    Some(wps) if !wps.is_empty() => {
                        let wp = wps[c.waypoint % wps.len()];
                        if wp.orientation != 0.0 {
                            c.orientation = wp.orientation;
                        }
                        Duration::from_millis(u64::from(wp.wait_ms))
                    }
                    _ => wander_pause(rng),
                };
                c.motion = Motion::Idle {
                    until: Some(now + pause),
                };
            }
        }
        Motion::Idle { until: Some(t) } if *t <= now => {
            let target = match (&waypoints, c.movement_type) {
                (Some(wps), _) if !wps.is_empty() => {
                    c.waypoint = (c.waypoint + 1) % wps.len();
                    let wp = wps[c.waypoint];
                    Some(Vector3d {
                        x: wp.x,
                        y: wp.y,
                        z: wp.z,
                    })
                }
                (_, MOVEMENT_RANDOM) => {
                    let (home, radius) = (c.home, c.wander_distance);
                    nav.as_mut()
                        .and_then(|n| n.random_point(home, radius))
                        .or_else(|| Some(random_point(home, radius, rng)))
                }
                _ => None,
            };
            match target {
                Some(target) => {
                    let from = c.position;
                    let path = match nav {
                        Some(n) => {
                            let target = n
                                .ground(target.x, target.y, target.z)
                                .map(|z| Vector3d { z, ..target })
                                .unwrap_or(target);
                            n.path(from, target).unwrap_or_else(|| vec![from, target])
                        }
                        None => vec![from, target],
                    };
                    frame = c.walk(path, false, now, spline_id);
                    if frame.is_none() {
                        c.motion = Motion::Idle {
                            until: Some(now + wander_pause(rng)),
                        };
                    }
                }
                None => c.motion = Motion::Idle { until: None },
            }
        }
        Motion::Idle { .. } => {}
    }
    let (pos, anchor, old_cell) = (c.position, c.vis_anchor, c.cell);
    let new_cell = cell_of(pos.x, pos.y);
    c.cell = new_cell;
    if frame.is_some() {
        map.next_spline_id = map.next_spline_id.wrapping_add(1);
    }
    map.grid.relocate(guid, old_cell, new_cell);
    visibility::moved(map, guid, anchor, pos);
    if let Some(frame) = frame {
        map.send_to_observers(guid, &frame, false);
    }
}

/// A point within `radius` of `center`, on the same height.
fn random_point(center: Vector3d, radius: f32, rng: &mut impl rand::Rng) -> Vector3d {
    let angle = rng.random_range(0.0..std::f32::consts::TAU);
    let dist = radius * rng.random_range(0.2f32..1.0).sqrt();
    Vector3d {
        x: center.x + dist * angle.cos(),
        y: center.y + dist * angle.sin(),
        z: center.z,
    }
}

/// Work out the zone and area a player is in, from the navmesh, and the
/// free-for-all flag that goes with an arena.
pub fn update_zone(map: &mut Map, guid: Guid) {
    let Some(pos) = map.players.get(&guid).map(|p| p.position()) else {
        return;
    };
    let Some(nav) = map.nav.as_mut() else {
        return;
    };
    let Some((zone, area)) = nav.zone_and_area(pos) else {
        return;
    };
    let data = map.realm.data.clone();
    let Some(p) = map.players.get_mut(&guid) else {
        return;
    };
    let zone = if zone != 0 { zone } else { data.zone_of(area) };
    let new_zone = zone != 0 && zone != p.zone;
    if new_zone {
        p.zone = zone;
        p.online
            .zone
            .store(zone, std::sync::atomic::Ordering::Relaxed);
    }
    p.area = area;
    let ffa = data
        .area(area)
        .is_some_and(|a| a.flags & AREA_FLAG_ARENA != 0);
    p.fields.set_flag(PLAYER_FLAGS, PLAYER_FLAGS_FFA_PVP, ffa);
    if new_zone {
        crate::game::map::world_states::send_init(map, guid);
    }
}

/// Whether a player at `p` is inside a trigger, with some slack for the
/// client having sent it from its edge.
fn inside(t: &AreaTrigger, p: Vector3d) -> bool {
    const SLACK: f32 = 5.0;
    let (dx, dy, dz) = (p.x - t.x, p.y - t.y, p.z - t.z);
    if t.radius > 0.0 {
        return dx * dx + dy * dy + dz * dz <= (t.radius + SLACK).powi(2);
    }
    // A box, turned by its yaw.
    let (s, c) = (-t.box_yaw).sin_cos();
    let (lx, ly) = (dx * c - dy * s, dx * s + dy * c);
    lx.abs() <= t.box_length / 2.0 + SLACK
        && ly.abs() <= t.box_width / 2.0 + SLACK
        && dz.abs() <= t.box_height / 2.0 + SLACK
}

/// `CMSG_AREATRIGGER`: a client stepped into a trigger.
pub fn area_trigger(map: &mut Map, guid: Guid, id: u32) {
    let data = map.realm.data.clone();
    let Some(t) = data.area_triggers.get(&id) else {
        return;
    };
    let Some(p) = map.players.get(&guid) else {
        return;
    };
    if t.map != map.key.map || !inside(t, p.position()) {
        return;
    }
    crate::game::map::quests::explored(map, guid, id);
    crate::game::map::battleground::area_trigger(map, guid, id);
    let Some(p) = map.players.get(&guid) else {
        return;
    };
    let Some(tp) = &t.teleport else {
        return;
    };
    if p.level() < tp.required_level && !p.gm {
        let text = if tp.message.is_empty() {
            format!("You must be level {} to enter.", tp.required_level)
        } else {
            tp.message.clone()
        };
        map.system_message(guid, &text);
        return;
    }
    map.teleport(
        guid,
        MapKey::world(tp.map),
        Vector3d {
            x: tp.x,
            y: tp.y,
            z: tp.z,
        },
        tp.orientation,
    );
}

/// Move one creature along its spawn's movement, by slab key.
pub fn step_creature_key(map: &mut Map, key: usize) {
    let now = map.now;
    let mut nav = map.nav.take();
    let mut rng = rand::rng();
    step_creature(map, key, now, nav.as_mut(), &mut rng);
    map.nav = nav;
}

/// Game object types.
pub const GO_DOOR: u8 = 0;
pub const GO_BUTTON: u8 = 1;
pub const GO_CHEST: u8 = 3;
pub const GO_CHAIR: u8 = 7;
pub const GO_SPELLCASTER: u8 = 22;

/// `CMSG_GAMEOBJ_USE`: a door or button flips, a chest opens, a chair is
/// sat on.
pub fn use_object(map: &mut Map, player: Guid, object: Guid) {
    if crate::game::map::battleground::use_object(map, player, object) {
        return;
    }
    if crate::game::map::professions::use_object(map, player, object) {
        return;
    }
    let Some(ppos) = map.players.get(&player).map(|p| p.position()) else {
        return;
    };
    let Some(o) = map.gameobject(object) else {
        return;
    };
    if crate::game::map::combat::dist(ppos, o.position) > 10.0 {
        return;
    }
    // A launcher by Elune's lake counts its fireworks; it launches them too.
    crate::game::omen::use_object(map, object);
    let Some(o) = map.gameobject(object) else {
        return;
    };
    let (kind, position, orientation) = (o.kind, o.position, o.orientation);
    let lock = map
        .realm
        .data
        .gameobject_templates
        .get(&o.entry)
        .map(|t| t.data[0])
        .unwrap_or(0);
    match kind {
        GO_DOOR | GO_BUTTON => {
            if let Some(o) = map.gameobject_mut(object) {
                let state = o.fields.get_u32(crate::game::fields::GAMEOBJECT_STATE);
                o.fields.set_u32(
                    crate::game::fields::GAMEOBJECT_STATE,
                    if state == 0 { 1 } else { 0 },
                );
            }
        }
        GO_CHEST => {
            // A locked chest needs its key or skill (Phase 7); an open one
            // just opens.
            let gm = map.players.get(&player).is_some_and(|p| p.gm);
            if lock == 0 || gm {
                crate::game::map::loot::open_object(map, player, object);
            }
        }
        // A spellcaster: its spell, on the one who uses it.
        GO_SPELLCASTER => {
            let spell = map
                .realm
                .data
                .gameobject_templates
                .get(&map.gameobject(object).map_or(0, |o| o.entry))
                .map_or(0, |t| t.data[0]);
            if spell != 0 {
                crate::game::map::spells::cast_triggered(map, player, spell, Some(player));
            }
        }
        GO_CHAIR => {
            map.teleport(player, map.key, position, orientation);
            if let Some(p) = map.players.get_mut(&player) {
                p.set_stand_state(crate::game::map::player::STAND_STATE_SIT);
            }
        }
        _ => {}
    }
}
