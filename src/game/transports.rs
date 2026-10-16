//! Boats and zeppelins: each follows its `TaxiPathNode` path round and
//! round in the time `transports` gives it, stopping where the path says.
//! Every player in its map knows it. When its path crosses to another map,
//! it goes there, and so do those aboard, at the same place on deck.

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

/// One stretch of a route: from one node to the next on one map.
#[derive(Debug, Clone, Copy)]
pub struct Leg {
    pub map: u32,
    pub from: Vector3d,
    pub to: Vector3d,
    /// When on the round it starts and ends, in milliseconds.
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Clone)]
pub struct Route {
    pub entry: u32,
    pub guid: Guid,
    pub period: u32,
    pub legs: Vec<Leg>,
    /// The map it is on now.
    pub map: u32,
}

fn dist(a: Vector3d, b: Vector3d) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2) + (a.z - b.z).powi(2)).sqrt()
}

impl Route {
    /// A route over `nodes` in `period` milliseconds: constant speed, with
    /// each node's stop, and a jump (no time) where the map changes.
    pub fn new(
        entry: u32,
        period: u32,
        nodes: &[crate::game::data::npc::TaxiPathNode],
    ) -> Option<Self> {
        if nodes.len() < 2 || period == 0 {
            return None;
        }
        let at = |n: &crate::game::data::npc::TaxiPathNode| Vector3d {
            x: n.x,
            y: n.y,
            z: n.z,
        };
        let stops: u32 = nodes.iter().map(|n| n.delay * 1000).sum();
        let pairs: Vec<(usize, usize)> = (0..nodes.len())
            .map(|i| (i, (i + 1) % nodes.len()))
            .collect();
        let length: f32 = pairs
            .iter()
            .filter(|(a, b)| nodes[*a].map == nodes[*b].map)
            .map(|(a, b)| dist(at(&nodes[*a]), at(&nodes[*b])))
            .sum();
        let moving = period.saturating_sub(stops).max(1) as f32;
        let mut t = 0u32;
        let mut legs = Vec::new();
        for (a, b) in pairs {
            t += nodes[a].delay * 1000;
            if nodes[a].map != nodes[b].map {
                continue;
            }
            let d = dist(at(&nodes[a]), at(&nodes[b]));
            let span = if length > 0.0 {
                (d / length * moving) as u32
            } else {
                0
            };
            legs.push(Leg {
                map: nodes[a].map,
                from: at(&nodes[a]),
                to: at(&nodes[b]),
                start: t,
                end: t + span,
            });
            t += span;
        }
        let map = legs.first()?.map;
        Some(Self {
            entry,
            guid: guid::make(HighGuid::MoTransport, 0, entry),
            period,
            legs,
            map,
        })
    }

    /// Where it is at `ms` on the round, and on which map.
    pub fn at(&self, ms: u32) -> (u32, Vector3d) {
        let t = ms % self.period;
        let leg = self
            .legs
            .iter()
            .rev()
            .find(|l| l.start <= t)
            .unwrap_or(&self.legs[0]);
        let span = (leg.end - leg.start).max(1) as f32;
        let f = ((t.saturating_sub(leg.start)) as f32 / span).min(1.0);
        (
            leg.map,
            Vector3d {
                x: leg.from.x + (leg.to.x - leg.from.x) * f,
                y: leg.from.y + (leg.to.y - leg.from.y) * f,
                z: leg.from.z + (leg.to.z - leg.from.z) * f,
            },
        )
    }
}

/// Every route, from the game data.
pub fn routes(realm: &Realm) -> Vec<Route> {
    let data = &realm.data;
    data.transports
        .iter()
        .filter_map(|t| {
            let template = data.gameobject_templates.get(&t.entry)?;
            let nodes = data.npc.taxi_path_nodes.get(&template.data[0])?;
            Route::new(t.entry, t.period, nodes)
        })
        .collect()
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
                let (map, pos) = r.at(now);
                if map != r.map {
                    cross(&realm, r, map, pos);
                    r.map = map;
                }
            }
        }
    });
}

/// The transport goes from its map to `to`, with those aboard.
fn cross(realm: &Arc<Realm>, route: &Route, to: u32, pos: Vector3d) {
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
            let at = Vector3d {
                x: pos.x + offset.x,
                y: pos.y + offset.y,
                z: pos.z + offset.z,
            };
            m.teleport_aboard(p, target, at, o, entry, offset);
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
        let transport = p
            .movement
            .flags
            .get_on_transport()
            .map(|t| t.transport);
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

    fn node(map: u32, x: f32, delay: u32) -> TaxiPathNode {
        TaxiPathNode {
            map,
            x,
            y: 0.0,
            z: 0.0,
            flags: 0,
            delay,
        }
    }

    #[test]
    fn a_route_runs_at_a_steady_speed_and_jumps_between_maps() {
        // 100 yards on map 0, a jump, 100 on map 1, a jump back: 2 seconds
        // of moving and a 1-second stop at the start.
        let r = Route::new(
            7,
            3000,
            &[
                node(0, 0.0, 1),
                node(0, 100.0, 0),
                node(1, 500.0, 0),
                node(1, 600.0, 0),
            ],
        )
        .unwrap();
        assert_eq!(r.legs.len(), 2);
        assert_eq!(
            r.at(500),
            (
                0,
                Vector3d {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0
                }
            )
        );
        let (m, p) = r.at(1500);
        assert_eq!(m, 0);
        assert!((p.x - 50.0).abs() < 0.5, "{p:?}");
        let (m, p) = r.at(2500);
        assert_eq!(m, 1);
        assert!((p.x - 550.0).abs() < 0.5, "{p:?}");
        assert_eq!(r.at(3500).0, 0, "round again");
    }
}
