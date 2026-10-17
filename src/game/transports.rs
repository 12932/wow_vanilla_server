//! Boats and zeppelins: each follows its `TaxiPathNode` path round and
//! round in the time `transports` gives it. The path is vmangos's transport
//! keyframes, as Thistle Tea's `Travel.Transport` ports them: Catmull-Rom
//! splines through the nodes, speeding up from each stop and slowing into
//! the next at the template's speed and acceleration, waiting the node's
//! delay at a stop, and jumping to the next map at a transition node. Every
//! player in its map knows it. When its path crosses to another map, it
//! goes there, and so do those aboard, at their place on deck, turned with
//! the ship.
//!
//! Elevators and the like (type 11) move by their `TransportAnimation`
//! frames about where they stand.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::Realm;
use crate::game::data::{GameObjectSpawn, GameObjectTemplate};
use crate::game::fields::*;
use crate::game::guid::{self, HighGuid};
use crate::game::map::gameobject::GameObject;
use crate::game::map::{Map, MapKey};

/// How often transports are checked for a crossing.
pub const TICK: Duration = Duration::from_millis(250);

const MAP_TRANSITION_FLAG: u32 = 0x1;
const STOP_FLAG: u32 = 0x2;
const SPLINE_LENGTH_STEPS: usize = 3;
const TWO_PI: f32 = std::f32::consts::TAU;

type V3 = (f32, f32, f32);

fn lerp(a: V3, b: V3, f: f32) -> V3 {
    (
        a.0 + (b.0 - a.0) * f,
        a.1 + (b.1 - a.1) * f,
        a.2 + (b.2 - a.2) * f,
    )
}

fn distance(a: V3, b: V3) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2) + (a.2 - b.2).powi(2)).sqrt()
}

fn combine(w: [f32; 4], p: &[V3]) -> V3 {
    let mut out = (0.0, 0.0, 0.0);
    for (w, p) in w.iter().zip(p) {
        out.0 += p.0 * w;
        out.1 += p.1 * w;
        out.2 += p.2 * w;
    }
    out
}

/// The Catmull-Rom point on segment `index` of `points` (from
/// `points[index]` to `points[index + 1]`) at `t`.
fn spline_point(points: &[V3], index: usize, t: f32) -> V3 {
    let (t2, t3) = (t * t, t * t * t);
    combine(
        [
            -0.5 * t3 + t2 - 0.5 * t,
            1.5 * t3 - 2.5 * t2 + 1.0,
            -1.5 * t3 + 2.0 * t2 + 0.5 * t,
            0.5 * t3 - 0.5 * t2,
        ],
        &points[index - 1..index + 3],
    )
}

fn spline_derivative(points: &[V3], index: usize, t: f32) -> V3 {
    let t2 = t * t;
    combine(
        [
            -1.5 * t2 + 2.0 * t - 0.5,
            4.5 * t2 - 5.0 * t,
            -4.5 * t2 + 4.0 * t + 0.5,
            1.5 * t2 - t,
        ],
        &points[index - 1..index + 3],
    )
}

fn normalize(o: f32) -> f32 {
    o.rem_euclid(TWO_PI)
}

/// A spline through some keyframes: their points with an extra before and
/// after, and the length from the start to each point.
#[derive(Debug, Clone, Default)]
pub struct Spline {
    points: Vec<V3>,
    lengths: Vec<f32>,
}

impl Spline {
    fn new(points: &[V3]) -> Self {
        let (first, second, last) = (points[0], points[1], points[points.len() - 1]);
        let mut sp = vec![lerp(first, second, -1.0)];
        sp.extend_from_slice(points);
        sp.push(last);
        let n = points.len();
        let mut lengths = vec![0.0; n + 1];
        let mut total = 0.0;
        for index in 1..n {
            let mut previous = sp[index];
            for step in 1..=SPLINE_LENGTH_STEPS {
                let current = spline_point(&sp, index, step as f32 / SPLINE_LENGTH_STEPS as f32);
                total += distance(previous, current);
                previous = current;
            }
            lengths[index + 1] = total;
        }
        Self {
            points: sp,
            lengths,
        }
    }

    fn length(&self, first: usize, last: usize) -> f32 {
        self.lengths[last] - self.lengths[first]
    }
}

/// One point of a route, with when the transport is there.
#[derive(Debug, Clone, Default)]
pub struct Keyframe {
    pub map: u32,
    pub position: V3,
    pub delay: f32,
    pub orientation: f32,
    pub stop: bool,
    pub teleport: bool,
    distance_from_previous: f32,
    distance_since_stop: f32,
    distance_until_stop: f32,
    time_from: f32,
    time_to: f32,
    pub arrive_ms: u32,
    pub depart_ms: u32,
    next_distance_from_previous: f32,
    next_arrive_ms: u32,
    spline: usize,
    spline_index: usize,
}

/// Where a transport is: its map, position and facing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub map: u32,
    pub position: Vector3d,
    pub orientation: f32,
    pub moving: bool,
}

#[derive(Debug, Clone)]
pub struct Route {
    pub entry: u32,
    pub guid: Guid,
    pub period: u32,
    pub frames: Vec<Keyframe>,
    splines: Vec<Spline>,
    speed: f32,
    accel_rate: f32,
    accel_time: f32,
    accel_distance: f32,
    /// The map it is on now.
    pub map: u32,
}

/// The default speed and acceleration of a ship whose template gives none.
pub const DEFAULT_SPEED: f32 = 30.0;
pub const DEFAULT_ACCEL: f32 = 1.0;

fn time_until_stop(f: &Keyframe, speed: f32, accel: f32, accel_distance: f32) -> f32 {
    let total = f.distance_since_stop + f.distance_until_stop;
    if total < 2.0 * accel_distance && f.distance_since_stop < f.distance_until_stop {
        2.0 * (total / accel).sqrt() - (2.0 * f.distance_since_stop / accel).sqrt()
    } else if total < 2.0 * accel_distance {
        (2.0 * f.distance_until_stop / accel).sqrt()
    } else if f.distance_since_stop < accel_distance {
        total / speed + speed / accel - (2.0 * f.distance_since_stop / accel).sqrt()
    } else if f.distance_until_stop < accel_distance {
        (2.0 * f.distance_until_stop / accel).sqrt()
    } else {
        f.distance_until_stop / speed + 0.5 * speed / accel
    }
}

impl Route {
    /// A route over `nodes` at the default speed.
    pub fn new(
        entry: u32,
        period: u32,
        nodes: &[crate::game::data::npc::TaxiPathNode],
    ) -> Option<Self> {
        Self::build(entry, period, nodes, DEFAULT_SPEED, DEFAULT_ACCEL)
    }

    /// vmangos's keyframes over `nodes`, at `speed` with `accel`; the
    /// round lasts `period` milliseconds if given, else as long as the path
    /// takes.
    pub fn build(
        entry: u32,
        period: u32,
        nodes: &[crate::game::data::npc::TaxiPathNode],
        speed: f32,
        accel: f32,
    ) -> Option<Self> {
        if nodes.len() < 3 || speed <= 0.0 || accel <= 0.0 {
            return None;
        }
        let pos = |n: &crate::game::data::npc::TaxiPathNode| (n.x, n.y, n.z);
        // The orientation points: the nodes, with one before and two after.
        let points: Vec<V3> = nodes.iter().map(pos).collect();
        let (first, second) = (points[0], points[1]);
        let (last, previous) = (points[points.len() - 1], points[points.len() - 2]);
        let before = lerp(first, second, -0.2);
        let after = lerp(last, previous, -0.2);
        let fin = lerp(after, last, -1.0);
        let mut ext = vec![before];
        ext.extend_from_slice(&points);
        ext.push(after);
        ext.push(fin);
        let mut frames: Vec<Keyframe> = Vec::new();
        let mut map_change = false;
        for index in 1..=nodes.len() - 2 {
            let (node, next) = (&nodes[index], &nodes[index + 1]);
            if map_change {
                map_change = false;
                continue;
            }
            if node.flags & MAP_TRANSITION_FLAG != 0 || node.map != next.map {
                if let Some(f) = frames.last_mut() {
                    f.teleport = true;
                }
                map_change = true;
                continue;
            }
            let d = spline_derivative(&ext, index + 1, 0.0);
            frames.push(Keyframe {
                map: node.map,
                position: pos(node),
                delay: node.delay as f32,
                orientation: normalize(d.1.atan2(d.0) + std::f32::consts::PI),
                stop: node.flags == STOP_FLAG,
                distance_from_previous: -1.0,
                distance_since_stop: -1.0,
                distance_until_stop: -1.0,
                ..Default::default()
            });
        }
        if let Some(f) = frames.last_mut() {
            f.teleport = true;
        }
        if frames.len() < 2 {
            return None;
        }
        // Splines, one for each stretch between map changes.
        let positions: Vec<V3> = frames.iter().map(|f| f.position).collect();
        let mut splines = Vec::new();
        let mut start = 0;
        let count = frames.len();
        for index in 1..count {
            let teleport = frames[index - 1].teleport;
            if !(teleport || index + 1 == count) {
                continue;
            }
            let extra = usize::from(!teleport);
            let n = index - start + extra;
            if n < 2 {
                start = index;
                continue;
            }
            let spline = Spline::new(&positions[start..start + n]);
            let id = splines.len();
            for fi in start..index + extra {
                let d = spline.length(fi - start, fi + 1 - start);
                frames[fi].spline = id;
                frames[fi].spline_index = fi - start + 1;
                frames[fi].distance_from_previous = d;
                if fi > 0 {
                    frames[fi - 1].next_distance_from_previous = d;
                }
            }
            if teleport {
                frames[index].spline = id;
                frames[index].spline_index = (index - start + 1).min(n);
                frames[index].distance_from_previous = 0.0;
                frames[index - 1].next_distance_from_previous = 0.0;
            }
            splines.push(spline);
            start = index;
        }
        let first_distance = frames[0].distance_from_previous;
        frames[count - 1].next_distance_from_previous = first_distance;
        // Distances from the last stop and to the next.
        let stops: Vec<usize> = (0..count).filter(|i| frames[*i].stop).collect();
        let first_stop = stops.first().copied().unwrap_or(0);
        let last_stop = stops.last().copied().unwrap_or(0);
        let mut d = 0.0;
        for offset in 0..count {
            let i = (offset + last_stop) % count;
            d = if frames[i].stop || i == last_stop {
                0.0
            } else {
                d + frames[i].distance_from_previous
            };
            frames[i].distance_since_stop = d;
        }
        let mut d = 0.0;
        for offset in (0..count).rev() {
            let i = (offset + first_stop) % count;
            let next = (i + 1) % count;
            d += frames[next].distance_from_previous;
            frames[i].distance_until_stop = d;
            if frames[i].stop || i == first_stop {
                d = 0.0;
            }
        }
        // Travel times.
        let accel_time = speed / accel;
        let accel_distance = 0.5 * speed * speed / accel;
        for f in frames.iter_mut() {
            f.time_to = time_until_stop(f, speed, accel, accel_distance);
        }
        let mut segment_time = 0.0;
        for offset in 0..count {
            let i = (offset + last_stop) % count;
            if frames[i].stop || i == last_stop {
                segment_time = frames[i].time_to;
            }
            frames[i].time_from = segment_time - frames[i].time_to;
        }
        // The schedule.
        let mut path_time = if frames[0].stop { frames[0].delay } else { 0.0 };
        frames[0].arrive_ms = 0;
        frames[0].depart_ms = if frames[0].stop {
            (path_time * 1000.0) as u32
        } else {
            0
        };
        for index in 1..count {
            path_time += frames[index - 1].time_to;
            if frames[index].stop {
                let arrive = (path_time * 1000.0) as u32;
                frames[index - 1].next_arrive_ms = arrive;
                frames[index].arrive_ms = arrive;
                frames[index].depart_ms = ((path_time + frames[index].delay) * 1000.0) as u32;
                path_time += frames[index].delay;
            } else {
                path_time -= frames[index].time_to;
                let arrive = (path_time * 1000.0) as u32;
                frames[index - 1].next_arrive_ms = arrive;
                frames[index].arrive_ms = arrive;
                frames[index].depart_ms = arrive;
            }
        }
        let last_departure = frames[count - 1].depart_ms;
        frames[count - 1].next_arrive_ms = last_departure;
        if period > 0 {
            frames[count - 1].depart_ms = period;
        }
        let period = if period > 0 {
            period
        } else {
            frames[count - 1].depart_ms.max(1)
        };
        let map = frames[0].map;
        Some(Self {
            entry,
            guid: guid::make(HighGuid::MoTransport, 0, entry),
            period,
            frames,
            splines,
            speed,
            accel_rate: accel,
            accel_time,
            accel_distance,
            map,
        })
    }

    /// The maps the route passes through.
    pub fn maps(&self) -> std::collections::BTreeSet<u32> {
        self.frames.iter().map(|f| f.map).collect()
    }

    fn accelerated(&self, t: f32) -> f32 {
        if t < self.accel_time {
            0.5 * self.accel_rate * t * t
        } else {
            self.accel_distance + (t - self.accel_time) * self.speed
        }
    }

    /// Where the transport is `ms` into its life.
    pub fn pose_at(&self, ms: u32) -> Pose {
        let p = ms % self.period;
        let stopped = |f: &Keyframe| p >= f.arrive_ms && p < f.depart_ms;
        let index = self
            .frames
            .iter()
            .position(|f| stopped(f) || (p >= f.depart_ms && p < f.next_arrive_ms))
            .unwrap_or(self.frames.len() - 1);
        let f = &self.frames[index];
        let at = |v: V3| Vector3d {
            x: v.0,
            y: v.1,
            z: v.2,
        };
        if stopped(f) {
            return Pose {
                map: f.map,
                position: at(f.position),
                orientation: f.orientation,
                moving: false,
            };
        }
        let elapsed = p as f32 / 1000.0 - f.depart_ms as f32 / 1000.0;
        let since = f.time_from + elapsed;
        let until = f.time_to - elapsed;
        let d = if since < until {
            self.accelerated(since) - f.distance_since_stop
        } else {
            f.distance_until_stop - self.accelerated(until)
        };
        let fraction = if f.next_distance_from_previous > 0.0 {
            (d / f.next_distance_from_previous).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let spline = &self.splines[f.spline.min(self.splines.len().saturating_sub(1))];
        let si = f.spline_index.clamp(1, spline.points.len() - 3);
        let position = spline_point(&spline.points, si, fraction);
        let dv = spline_derivative(&spline.points, si, fraction);
        Pose {
            map: f.map,
            position: at(position),
            orientation: normalize(dv.1.atan2(dv.0) + std::f32::consts::PI),
            moving: true,
        }
    }

    /// Where it is at `ms` on the round, and on which map.
    pub fn at(&self, ms: u32) -> (u32, Vector3d) {
        let pose = self.pose_at(ms);
        (pose.map, pose.position)
    }
}

/// A passenger's place in the world from their place on deck (`local`,
/// with its facing) and the transport's pose.
pub fn passenger_world(local: Vector3d, local_o: f32, pose: &Pose) -> (Vector3d, f32) {
    let (c, s) = (pose.orientation.cos(), pose.orientation.sin());
    (
        Vector3d {
            x: pose.position.x + local.x * c - local.y * s,
            y: pose.position.y + local.y * c + local.x * s,
            z: pose.position.z + local.z,
        },
        normalize(pose.orientation + local_o),
    )
}

/// A place on deck from a place in the world.
pub fn passenger_local(world: Vector3d, world_o: f32, pose: &Pose) -> (Vector3d, f32) {
    let (dx, dy) = (world.x - pose.position.x, world.y - pose.position.y);
    let (c, s) = (pose.orientation.cos(), pose.orientation.sin());
    (
        Vector3d {
            x: dx * c + dy * s,
            y: dy * c - dx * s,
            z: world.z - pose.position.z,
        },
        normalize(world_o - pose.orientation),
    )
}

/// An animated object's position `ms` into its animation: its frames' local
/// offset at that time, turned by the object's rotation, about where it
/// stands (Thistle Tea's animation route).
pub fn animation_position(
    frames: &[crate::game::data::world::AnimationFrame],
    ms: u32,
    stands: Vector3d,
    rotation: [f32; 4],
) -> Option<Vector3d> {
    let period = frames.last()?.time_ms.max(1);
    let p = ms % period;
    let next = frames
        .iter()
        .position(|f| f.time_ms >= p)
        .unwrap_or(frames.len() - 1);
    let (a, b) = (&frames[next.saturating_sub(1)], &frames[next]);
    let local = if a.time_ms == b.time_ms {
        (a.x, a.y, a.z)
    } else {
        let t = (p - a.time_ms) as f32 / (b.time_ms - a.time_ms) as f32;
        lerp((a.x, a.y, a.z), (b.x, b.y, b.z), t)
    };
    let [qx, qy, qz, qw] = rotation;
    let cross = |a: V3, b: V3| {
        (
            a.1 * b.2 - a.2 * b.1,
            a.2 * b.0 - a.0 * b.2,
            a.0 * b.1 - a.1 * b.0,
        )
    };
    let uv = cross((qx, qy, qz), local);
    let uuv = cross((qx, qy, qz), uv);
    let r = (
        local.0 + 2.0 * (uv.0 * qw + uuv.0),
        local.1 + 2.0 * (uv.1 * qw + uuv.1),
        local.2 + 2.0 * (uv.2 * qw + uuv.2),
    );
    Some(Vector3d {
        x: stands.x + r.0,
        y: stands.y - r.1,
        z: stands.z + r.2,
    })
}

/// Every route, from the game data.
pub fn routes(realm: &Realm) -> Vec<Route> {
    let data = &realm.data;
    data.transports
        .iter()
        .filter_map(|t| {
            let template = data.gameobject_templates.get(&t.entry)?;
            let nodes = data.npc.taxi_path_nodes.get(&template.data[0])?;
            let speed = if template.data[1] > 0 {
                template.data[1] as f32
            } else {
                DEFAULT_SPEED
            };
            let accel = if template.data[2] > 0 {
                template.data[2] as f32
            } else {
                DEFAULT_ACCEL
            };
            Route::build(t.entry, t.period, nodes, speed, accel)
        })
        .collect()
}

/// The route of transport `entry`.
pub fn route_of(realm: &Realm, entry: u32) -> Option<Route> {
    routes(realm).into_iter().find(|r| r.entry == entry)
}

/// A player who logged out aboard a transport comes back on its deck,
/// wherever it has gone since, if it is on their map.
pub fn place_aboard(realm: &Realm, p: &mut crate::game::map::player::Player) {
    let Some((entry, x, y, z, o)) = p.row.state.transport.take() else {
        return;
    };
    let Some(route) = route_of(realm, entry) else {
        return;
    };
    let pose = route.pose_at(crate::game::uptime_ms());
    if pose.map != p.row.map {
        return;
    }
    let (at, facing) = passenger_world(Vector3d { x, y, z }, o, &pose);
    p.movement.position = at;
    p.movement.orientation = facing;
}

/// The transport's object, at `pos` on its map.
fn object(template: &GameObjectTemplate, route: &Route, map: u32, pos: Vector3d) -> GameObject {
    let mut go = GameObject::from_spawn(
        template,
        &GameObjectSpawn {
            guid: route.entry,
            entry: route.entry,
            map,
            x: pos.x,
            y: pos.y,
            z: pos.z,
            state: 1,
            anim_progress: 255,
            ..Default::default()
        },
    );
    go.guid = route.guid;
    go.spawn_guid = None;
    go.fields.set_guid(OBJECT_FIELD_GUID, route.guid);
    go.fields.set_u32(GAMEOBJECT_LEVEL, route.period);
    go
}

fn place(realm: &Arc<Realm>, route: &Route, map: u32, pos: Vector3d) {
    let Some(template) = realm.data.gameobject_templates.get(&route.entry).cloned() else {
        return;
    };
    let go = object(&template, route, map, pos);
    let g = route.guid;
    realm.maps.get(realm, MapKey::world(map)).call(move |m| {
        if m.gameobject(g).is_none() {
            m.add_gameobject(go);
            m.globals.push(g);
        }
    });
}

/// Start the boats and zeppelins: each on its map, and a clock that moves
/// them from map to map with those aboard.
pub fn start(realm: &Arc<Realm>) {
    let mut routes = routes(realm);
    if routes.is_empty() || tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    let now = crate::game::uptime_ms();
    for r in &mut routes {
        let (map, pos) = r.at(now);
        r.map = map;
        place(realm, r, map, pos);
    }
    tracing::info!("{} transports running", routes.len());
    let weak = Arc::downgrade(realm);
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(TICK);
        loop {
            tick.tick().await;
            let Some(realm) = weak.upgrade() else {
                return;
            };
            let now = crate::game::uptime_ms();
            for r in &mut routes {
                let pose = r.pose_at(now);
                if pose.map != r.map {
                    cross(&realm, r, pose);
                    r.map = pose.map;
                }
            }
        }
    });
}

/// The transport goes from its map to the pose's, with those aboard.
fn cross(realm: &Arc<Realm>, route: &Route, pose: Pose) {
    let (to, pos) = (pose.map, pose.position);
    let g = route.guid;
    let entry = route.entry;
    let from = realm.maps.get(realm, MapKey::world(route.map));
    let target = MapKey::world(to);
    from.call(move |m| {
        m.globals.retain(|x| *x != g);
        m.remove_gameobject(g);
        let aboard: Vec<(Guid, Vector3d, f32)> = m
            .players
            .values()
            .filter_map(|p| {
                let t = p.movement.flags.get_on_transport()?;
                (t.transport.guid == g).then_some((
                    p.guid,
                    t.transport.position,
                    t.transport.orientation,
                ))
            })
            .collect();
        for (p, offset, o) in aboard {
            let (at, facing) = passenger_world(offset, o, &pose);
            m.teleport_aboard(p, target, at, facing, entry, offset);
        }
    });
    place(realm, route, to, pos);
}

impl Map {
    /// Send a player aboard a transport to another map: the client is
    /// told it travels with the transport, and arrives on deck.
    pub fn teleport_aboard(
        &mut self,
        g: Guid,
        to: MapKey,
        at: Vector3d,
        orientation: f32,
        transport_entry: u32,
        offset: Vector3d,
    ) {
        use wow_world_messages::vanilla::*;
        let Some(p) = self.players.get(&g) else {
            return;
        };
        let transport = p.movement.flags.get_on_transport().map(|t| t.transport);
        p.outbound.send(&SMSG_TRANSFER_PENDING {
            map: crate::game::map::movement::map_enum(to.map),
            has_transport: Some(SMSG_TRANSFER_PENDING_has_transport {
                transport: transport_entry,
                transport_map: crate::game::map::movement::map_enum(self.key.map),
            }),
        });
        let Some(mut p) = self.take_player(g) else {
            return;
        };
        p.movement.position = at;
        p.movement.orientation = orientation;
        p.movement.flags = match transport {
            Some(mut t) => {
                t.position = offset;
                MovementInfo_MovementFlags::empty()
                    .set_on_transport(MovementInfo_MovementFlags_OnTransport { transport: t })
            }
            None => MovementInfo_MovementFlags::empty(),
        };
        p.logout_at = None;
        let session = p.online.session.clone();
        let _ = session.send(crate::game::session::Control::Transfer {
            player: Box::new(p),
            to,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::data::npc::TaxiPathNode;

    fn path(nodes: &[(u32, f32, f32, u32, u32)]) -> Vec<TaxiPathNode> {
        nodes
            .iter()
            .map(|(map, x, y, flags, delay)| TaxiPathNode {
                map: *map,
                x: *x,
                y: *y,
                z: 0.0,
                flags: *flags,
                delay: *delay,
            })
            .collect()
    }

    #[test]
    fn a_route_stops_speeds_up_and_jumps_between_maps() {
        // Along x on map 0, stopping 10 s at 100; the jump; along x on map 1.
        let nodes = path(&[
            (0, 0.0, 0.0, 0, 0),
            (0, 50.0, 0.0, 0, 0),
            (0, 100.0, 0.0, 2, 10),
            (0, 150.0, 0.0, 0, 0),
            (0, 200.0, 0.0, 1, 0),
            (1, 1000.0, 0.0, 0, 0),
            (1, 1050.0, 0.0, 0, 0),
            (1, 1100.0, 0.0, 2, 10),
            (1, 1150.0, 0.0, 0, 0),
            (1, 1200.0, 0.0, 0, 0),
        ]);
        let r = Route::build(7, 0, &nodes, 10.0, 2.0).expect("a route");
        assert_eq!(r.maps().into_iter().collect::<Vec<_>>(), vec![0, 1]);
        // At the stop, still and facing along the path.
        let stop = r
            .frames
            .iter()
            .find(|f| f.stop && f.map == 0)
            .unwrap()
            .clone();
        let pose = r.pose_at(stop.arrive_ms + 1000);
        assert!(!pose.moving);
        assert!((pose.position.x - 100.0).abs() < 0.01, "{pose:?}");
        // Leaving it, slowly at first.
        let a = r.pose_at(stop.depart_ms + 500).position.x;
        let b = r.pose_at(stop.depart_ms + 1000).position.x;
        let c = r.pose_at(stop.depart_ms + 1500).position.x;
        assert!(a > 100.0 && b > a && c - b > b - a, "{a} {b} {c}");
        // The other map comes round, and the round comes round.
        assert!((0..r.period).step_by(500).any(|t| r.pose_at(t).map == 1));
        assert_eq!(r.pose_at(r.period + 10).map, r.pose_at(10).map);
    }

    #[test]
    fn a_passenger_turns_with_the_ship() {
        let pose = Pose {
            map: 0,
            position: Vector3d {
                x: 10.0,
                y: 20.0,
                z: 5.0,
            },
            orientation: std::f32::consts::FRAC_PI_2,
            moving: true,
        };
        let deck = Vector3d {
            x: 3.0,
            y: 0.0,
            z: 2.0,
        };
        let (world, o) = passenger_world(deck, 0.5, &pose);
        assert!(
            (world.x - 10.0).abs() < 0.001 && (world.y - 23.0).abs() < 0.001,
            "{world:?}"
        );
        assert!((world.z - 7.0).abs() < 0.001);
        let (back, bo) = passenger_local(world, o, &pose);
        assert!((back.x - 3.0).abs() < 0.001 && back.y.abs() < 0.001);
        assert!((bo - 0.5).abs() < 0.001);
    }

    #[test]
    fn an_elevator_rises_and_falls_by_its_frames() {
        use crate::game::data::world::AnimationFrame;
        let frames = [
            AnimationFrame {
                time_ms: 0,
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            AnimationFrame {
                time_ms: 1000,
                x: 0.0,
                y: 0.0,
                z: 10.0,
            },
            AnimationFrame {
                time_ms: 2000,
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
        ];
        let at = Vector3d {
            x: 1.0,
            y: 2.0,
            z: 3.0,
        };
        let up = animation_position(&frames, 500, at, [0.0, 0.0, 0.0, 1.0]).unwrap();
        assert!((up.z - 8.0).abs() < 0.001, "{up:?}");
        let top = animation_position(&frames, 1000, at, [0.0, 0.0, 0.0, 1.0]).unwrap();
        assert!((top.z - 13.0).abs() < 0.001);
        let round = animation_position(&frames, 2500, at, [0.0, 0.0, 0.0, 1.0]).unwrap();
        assert!((round.z - 8.0).abs() < 0.001);
    }
}
