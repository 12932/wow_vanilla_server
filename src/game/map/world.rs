//! The world in a map: its spawns, the creatures that move about in it,
//! the zones and areas players are in, and its area triggers.

use std::time::{Duration, Instant};

use rand::RngExt;
use tracing::info;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::data::{AREA_FLAG_ARENA, AreaTrigger};
use crate::game::fields::PLAYER_FLAGS;
use crate::game::map::creature::{MOVEMENT_RANDOM, MOVEMENT_WAYPOINT, Motion, wander_pause};
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
    if let Some(dir) = map.realm.config.terrain_dir.as_ref() {
        map.terrain = crate::game::map::terrain::Terrain::open(dir, map.key.map);
    }
    let on = crate::game::events::current(&map.realm);
    let mut rng = rand::rng();
    let (skip_creatures, skip_objects) = crate::game::map::pools::populate(map, &on, &mut rng);
    // A copy's save: its dead stay dead until they are due back.
    let save = (map.key.instance != 0)
        .then(|| crate::game::instances::save_of(&map.realm, map.key.instance))
        .flatten();
    if let Some(s) = &save {
        crate::game::map::instance_script::load(map, &s.data);
    }
    let unix = crate::game::social::mail::now();
    let mut lying = Vec::new();
    if let Some(spawns) = data.creature_spawns.get(&map.key.map) {
        for s in spawns {
            if s.spawn_flags & crate::game::data::SPAWN_FLAG_DISABLED != 0
                || !crate::game::events::spawned(data.event_creatures.get(&s.guid).copied(), &on)
                || !crate::game::map::battleground::spawn_allowed(map, true, s.guid)
                || skip_creatures.contains(&s.guid)
            {
                continue;
            }
            if let Some(c) = crate::game::events::spawn_creature(&data, s, &on, &mut rng) {
                let g = map.add_creature(c);
                crate::game::map::ai::lie_dead_if_spawned_so(map, g, s);
                if let Some(back) = save.as_ref().and_then(|v| v.dead.get(&s.guid)).copied()
                    && back > unix
                {
                    let wait = (back != crate::game::instances::NEVER)
                        .then(|| Duration::from_secs((back - unix) as u64));
                    lying.push((g, wait));
                }
            }
        }
    }
    for (g, wait) in lying {
        crate::game::map::ai::lie_dead(map, g, wait);
    }
    // Linked slaves that can't spawn with their master dead.
    let barred: Vec<Guid> = map
        .creatures
        .iter()
        .filter(|(_, c)| c.alive())
        .filter_map(|(_, c)| c.spawn_guid.map(|s| (c.guid, s)))
        .filter(|(_, s)| data.scripts.links.contains_key(s))
        .map(|(g, _)| g)
        .collect();
    for g in barred {
        let spawn = map.creature(g).and_then(|c| c.spawn_guid).unwrap_or(0);
        if !crate::game::map::creature_groups::may_spawn(map, spawn) {
            crate::game::map::ai::lie_dead(map, g, Some(Duration::from_secs(30)));
        }
    }
    if let Some(spawns) = data.gameobject_spawns.get(&map.key.map) {
        for s in spawns {
            if s.despawned
                || s.spawn_flags & crate::game::data::SPAWN_FLAG_DISABLED != 0
                || !crate::game::events::spawned(data.event_objects.get(&s.guid).copied(), &on)
                || !crate::game::map::battleground::spawn_allowed(map, false, s.guid)
                || skip_objects.contains(&s.guid)
            {
                continue;
            }
            if let Some(t) = data.gameobject_templates.get(&s.entry) {
                map.add_gameobject(GameObject::from_spawn(t, s));
            }
        }
    }
    crate::game::map::instance_script::settle_spawns(map);
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
        if crate::game::map::creature_groups::following(map, c.guid)
            && !matches!(c.motion, Motion::Moving(_))
        {
            return;
        }
        // A path held by a script waits.
        if c.scripted.as_deref().is_some_and(|s| s.hold.is_some())
            && !matches!(c.motion, Motion::Moving(_))
        {
            return;
        }
        if c.movement_type == MOVEMENT_WAYPOINT {
            match c.scripted.as_deref().and_then(|s| s.route.as_ref()) {
                Some(r) => Some((*r.points).clone()),
                None => c
                    .spawn_guid
                    .and_then(|g| map.realm.data.waypoints.get(&g))
                    .or_else(|| map.realm.data.waypoints_by_entry.get(&c.entry))
                    .cloned(),
            }
        } else {
            None
        }
    };
    // A script's path that doesn't go round ends at its last point.
    let route_ends = map.creatures.get(key).is_some_and(|c| {
        matches!(c.motion, Motion::Idle { until: Some(t) } if t <= now)
            && c.scripted
                .as_deref()
                .and_then(|s| s.route.as_ref())
                .is_some_and(|r| !r.repeat && c.waypoint >= r.points.len())
    });
    if route_ends {
        if let Some(c) = map.creatures.get_mut(key) {
            c.movement_type = crate::game::map::creature::MOVEMENT_IDLE;
            c.motion = Motion::Idle { until: None };
            if let Some(s) = c.scripted.as_deref_mut() {
                s.route = None;
            }
        }
        return;
    }
    let mut arrived = None;
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
            // A script's move isn't a waypoint reached.
            let scripted = c
                .scripted
                .as_deref()
                .and_then(|s| s.scripted_move)
                .is_some_and(|m| m.spline_id == w.spline_id);
            if done {
                let pause = match &waypoints {
                    Some(wps) if !wps.is_empty() && !scripted => {
                        // Reached: it is the point to come back to, and the
                        // next is the one it is bound for.
                        let i = c.waypoint % wps.len();
                        let wp = wps[i];
                        if let Some(o) = wp.orientation {
                            c.orientation = o;
                        }
                        arrived = Some((wp.point, wp.script_id));
                        let repeat = c
                            .scripted
                            .as_deref()
                            .and_then(|s| s.route.as_ref())
                            .is_none_or(|r| r.repeat);
                        c.last_waypoint = Some(i);
                        c.waypoint = if i + 1 < wps.len() {
                            i + 1
                        } else if repeat {
                            0
                        } else {
                            wps.len()
                        };
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
                    let wp = wps[c.waypoint % wps.len()];
                    Some(Vector3d {
                        x: wp.x,
                        y: wp.y,
                        z: wp.z,
                    })
                }
                (_, MOVEMENT_RANDOM) => {
                    let (home, radius) = (
                        crate::game::map::event_ai::wander_center(c),
                        c.wander_distance,
                    );
                    match nav.as_mut() {
                        Some(n) => wander_point(n, home, radius, rng),
                        None => Some(random_point(home, radius, rng)),
                    }
                }
                _ => None,
            };
            let wanders = waypoints.as_ref().is_none_or(|w| w.is_empty())
                && c.movement_type == MOVEMENT_RANDOM;
            match target {
                Some(target) => {
                    let from = c.position;
                    // A waypoint is reached even where the navmesh stops
                    // short; a wanderer goes nowhere it has no path to.
                    let path = match nav {
                        Some(n) => {
                            let target = n
                                .ground(target.x, target.y, target.z)
                                .map(|z| Vector3d { z, ..target })
                                .unwrap_or(target);
                            match n.path(from, target) {
                                Some(p) => Some(p),
                                None if wanders => None,
                                None => Some(vec![from, target]),
                            }
                        }
                        None => Some(vec![from, target]),
                    };
                    let run = c.scripted.as_deref().is_some_and(|s| s.run);
                    frame = path.and_then(|p| c.walk(p, run, now, spline_id));
                    if frame.is_none() {
                        c.motion = Motion::Idle {
                            until: Some(now + wander_pause(rng)),
                        };
                    }
                }
                // No point for a wanderer: it idles two to six seconds.
                None if wanders => {
                    c.motion = Motion::Idle {
                        until: Some(now + idle_pause(rng)),
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
    let walked = frame.is_some() && matches!(c.motion, Motion::Moving(_));
    if frame.is_some() {
        map.next_spline_id = map.next_spline_id.wrapping_add(1);
    }
    map.grid.relocate(guid, old_cell, new_cell);
    visibility::moved(map, guid, anchor, pos);
    if let Some(frame) = frame {
        map.send_to_observers(guid, &frame, false);
    }
    if walked {
        crate::game::map::creature_groups::leader_walked(map, guid);
    }
    if let Some((point, script)) = arrived {
        crate::game::map::event_ai::movement_inform(map, guid, 2, point);
        if script != 0 {
            crate::game::map::scripts::start(
                map,
                crate::game::data::scripts::ScriptTable::Movement,
                script,
                guid,
                Guid::zero(),
            );
        }
    }
}

/// Thistle Tea's wander point (`Pathfinding.find_random_point_around_circle`):
/// a random point of the navmesh around `anchor`; one the navmesh gives
/// beyond `radius` is pulled back toward the anchor, to a random distance
/// within it, as far as a walk over the navmesh gets.
pub fn wander_point(
    nav: &mut Nav,
    anchor: Vector3d,
    radius: f32,
    rng: &mut impl rand::Rng,
) -> Option<Vector3d> {
    let p = nav.random_point(anchor, radius)?;
    let d = ((p.x - anchor.x).powi(2) + (p.y - anchor.y).powi(2)).sqrt();
    if d <= radius {
        return Some(p);
    }
    let f = rng.random_range(0.0f32..1.0).sqrt() * radius / d;
    let to = Vector3d {
        x: anchor.x + (p.x - anchor.x) * f,
        y: anchor.y + (p.y - anchor.y) * f,
        z: anchor.z,
    };
    nav.walk_hit(anchor, to)
}

/// How long a wanderer with nowhere to go waits: two to six seconds
/// (Thistle Tea's `idle_delay`).
fn idle_pause(rng: &mut impl rand::Rng) -> Duration {
    Duration::from_millis(rng.random_range(2_000..6_000))
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

/// The zone and area at a point, as Thistle Tea works them out: the
/// navmesh's; else those of the terrain tile's area bit; else the map's
/// only zone. `None` with neither a navmesh nor terrain tiles.
pub fn zone_and_area(map: &mut Map, pos: Vector3d) -> Option<(u32, u32)> {
    if map.nav.is_none() && map.terrain.is_none() {
        return None;
    }
    let data = map.realm.data.clone();
    let id = map.key.map;
    map.nav
        .as_mut()
        .and_then(|n| n.zone_and_area(pos))
        .map(|(zone, area)| (if zone != 0 { zone } else { data.zone_of(area) }, area))
        .or_else(|| {
            let bit = map.terrain.as_mut()?.area_bit(pos.x, pos.y)? as u32;
            let a = data
                .areas
                .values()
                .find(|a| a.map == id && a.explore_bit == bit)?;
            Some((data.zone_of(a.id), a.id))
        })
        .or_else(|| {
            let mut zones = data.areas.values().filter(|a| a.map == id && a.zone == 0);
            match (zones.next(), zones.next()) {
                (Some(z), None) => Some((z.id, z.id)),
                _ => None,
            }
        })
}

/// Whether `area` is free-for-all PvP (an arena's, as Gurubashi's).
pub fn area_is_ffa(data: &crate::game::data::GameData, area: u32) -> bool {
    data.area(area)
        .is_some_and(|a| a.flags & AREA_FLAG_ARENA != 0)
}

/// Work out the zone and area a player is in, from the navmesh or the
/// terrain, and the free-for-all flag that goes with an arena.
pub fn update_zone(map: &mut Map, guid: Guid) {
    let Some(pos) = map.players.get(&guid).map(|p| p.position()) else {
        return;
    };
    let Some((zone, area)) = zone_and_area(map, pos) else {
        return;
    };
    let data = map.realm.data.clone();
    let Some(p) = map.players.get_mut(&guid) else {
        return;
    };
    let new_zone = zone != 0 && zone != p.zone;
    if new_zone {
        p.zone = zone;
        p.online
            .zone
            .store(zone, std::sync::atomic::Ordering::Relaxed);
    }
    let new_area = p.area != area;
    p.area = area;
    // Explored as it is walked in, not only as it is entered: a ghost's
    // area once they live again.
    crate::game::map::exploration::entered(map, guid, area);
    if zone != 0 && zone != area {
        crate::game::map::exploration::entered(map, guid, zone);
    }
    if new_area {
        crate::game::map::spell_areas::reconcile(map, guid);
    }
    let ffa = area_is_ffa(&data, area) || crate::game::map::arena::fighting(map, guid);
    let Some(p) = map.players.get_mut(&guid) else {
        return;
    };
    p.fields.set_flag(PLAYER_FLAGS, PLAYER_FLAGS_FFA_PVP, ffa);
    if new_zone {
        crate::game::map::world_states::send_init(map, guid);
    }
}

/// Whether a player at `p` is inside a trigger, with some slack for the
/// client having sent it from its edge.
pub fn inside(t: &AreaTrigger, p: Vector3d) -> bool {
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
    // Quests and scripts are the living's (Thistle Tea's `AreaTriggers`).
    let alive = p.death == crate::game::map::death::Death::Alive;
    if alive {
        crate::game::map::quests::explored(map, guid, id);
    }
    crate::game::map::rest::area_trigger(map, guid, id);
    crate::game::map::battleground::area_trigger(map, guid, id);
    crate::game::map::outdoor_pvp::area_trigger(map, guid, id);
    crate::game::map::pvp::entrance(map, guid, id);
    // A trigger with a cooldown runs for the first through, then rests in
    // this copy (vmangos's `m_areaTriggerCooldowns`).
    let now = map.now;
    let resting = map.trigger_cooldowns.get(&id).is_some_and(|t| *t > now);
    let scripted = alive && !resting;
    if scripted && let Some(secs) = data.scripts.areatrigger_cooldown.get(&id) {
        map.trigger_cooldowns
            .insert(id, now + std::time::Duration::from_secs(u64::from(*secs)));
    }
    if let Some(script) = data
        .scripts
        .areatrigger_script
        .get(&id)
        .copied()
        .filter(|_| scripted)
    {
        crate::game::map::scripts::start(
            map,
            crate::game::data::scripts::ScriptTable::Generic,
            script,
            guid,
            guid,
        );
    }
    if scripted && data.scripts.areatrigger.contains_key(&id) {
        crate::game::map::scripts::start(
            map,
            crate::game::data::scripts::ScriptTable::AreaTrigger,
            id,
            guid,
            guid,
        );
    }
    let Some(tp) = &t.teleport else {
        return;
    };
    // A ghost enters only the dungeon their corpse is in.
    let tp = match crate::game::map::corpses::portal(map, guid, tp) {
        Ok(tp) => tp,
        Err(text) => {
            crate::game::map::corpses::refuse(map, guid, text);
            return;
        }
    };
    let barred = data
        .scripts
        .teleport_condition
        .get(&id)
        .is_some_and(|c| !crate::game::map::conditions::met(map, *c, None, Some(guid)));
    let Some(p) = map.players.get(&guid) else {
        return;
    };
    // Refused: the trigger's own message, if it has one.
    if (p.level() < tp.required_level || barred) && !p.gm {
        if !tp.message.is_empty() {
            crate::game::map::corpses::refuse(map, guid, tp.message.clone());
        }
        return;
    }
    crate::game::map::corpses::revive_for_map(map, guid, tp.map);
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
    // Flyers and swimmers go their own ways.
    if crate::game::map::creature_motion::idle_step(map, key) {
        return;
    }
    let now = map.now;
    let mut nav = map.nav.take();
    let mut rng = rand::rng();
    step_creature(map, key, now, nav.as_mut(), &mut rng);
    map.nav = nav;
}

/// Game object types.
pub const GO_DOOR: u8 = 0;
pub const GO_BUTTON: u8 = 1;
pub const GO_QUESTGIVER: u8 = 2;
pub const GO_CHEST: u8 = 3;
pub const GO_CHAIR: u8 = 7;
pub const GO_SPELLCASTER: u8 = 22;

/// `CMSG_GAMEOBJ_USE`: a door or button flips, a chest opens, a chair is
/// sat on.
pub fn use_object(map: &mut Map, player: Guid, object: Guid) {
    // Not one flagged against use, nor a door a copy's script works.
    let inert = map.gameobject(object).is_some_and(|o| {
        o.fields.get_u32(crate::game::fields::GAMEOBJECT_FLAGS)
            & crate::game::map::objects::GO_FLAG_NO_INTERACT
            != 0
    });
    if inert || crate::game::map::instance_script::scripted_door(map, object) {
        return;
    }
    if crate::game::map::battleground::use_object(map, player, object)
        || crate::game::map::outdoor_pvp::use_object(map, player, object)
    {
        return;
    }
    if crate::game::map::professions::use_object(map, player, object) {
        return;
    }
    // A quest object runs its scripts only when it goes off.
    if crate::game::map::objects::use_goober(map, player, object) {
        return;
    }
    // Near enough: by its model, for the objects measured so.
    if !crate::game::map::objects::in_use_range(map, player, object) {
        return;
    }
    // A quest giver or a page: not under an immunity it bars; the auras a
    // use breaks go.
    if !crate::game::map::objects::prepare_use(map, player, object) {
        return;
    }
    // A locked chest opens by its spell, not by a hand.
    let gm = map.players.get(&player).is_some_and(|p| p.gm);
    let locked_chest = map.gameobject(object).is_some_and(|o| {
        o.kind == GO_CHEST
            && map
                .realm
                .data
                .gameobject_templates
                .get(&o.entry)
                .is_some_and(|t| t.data[0] != 0)
    });
    if locked_chest && !gm {
        return;
    }
    crate::game::map::scripts::object_used(map, player, object);
    // A launcher by Elune's lake counts its fireworks; it launches them too.
    crate::game::omen::use_object(map, object);
    // Doors, traps, pages, rituals, spell casters; quest credit for a use.
    if crate::game::map::objects::use_object(map, player, object) {
        return;
    }
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
        GO_QUESTGIVER => crate::game::map::quests::object_hello(map, player, object),
        GO_CHEST => {
            // A locked chest needs its key or skill (Phase 7); an open one
            // just opens.
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
