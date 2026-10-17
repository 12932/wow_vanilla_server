//! A player inside a dungeon or raid copy that is no longer theirs, as
//! Thistle Tea's `Instance.Eviction` has it: out of the group that owns
//! it, or out of a raid group in a raid, they have a minute's grace, told
//! with `SMSG_RAID_GROUP_ONLY`, then go home. Back in the group in time,
//! they stay. And what Thistle Tea tells of copies on the way in and out:
//! the raids one is saved to ([`crate::game::instances::send_saved`]), and
//! the copy just left (`SMSG_UPDATE_LAST_INSTANCE`).

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::{
    RaidGroupError, SMSG_RAID_GROUP_ONLY, SMSG_UPDATE_LAST_INSTANCE, Vector3d,
};

use crate::game::instances::{belongs, instance_type};
use crate::game::map::player::Player;
use crate::game::map::{Map, MapKey};

/// The grace before one is sent home.
pub const GRACE: Duration = Duration::from_secs(60);
/// The copies are looked at about once a second (updates are 50 ms).
const CHECK_EVERY: u64 = 20;

/// `SMSG_RAID_GROUP_ONLY`: home in `after` (none: refused, or all clear).
pub fn raid_group_only(p: &Player, after: Duration) {
    p.outbound.send(&SMSG_RAID_GROUP_ONLY {
        homebind_timer: after.as_millis() as u32,
        error: RaidGroupError::Required,
    });
}

fn instanced(map: &Map) -> bool {
    map.key.instance != 0 && matches!(instance_type(&map.realm, map.key.map), 1 | 2)
}

/// About once a second in a copy: those who no longer belong start their
/// grace, those who belong again are clear, those whose grace is over go
/// home.
pub fn update(map: &mut Map) {
    if !map.stats.updates.is_multiple_of(CHECK_EVERY) || !instanced(map) || map.players.is_empty() {
        return;
    }
    let now = map.now;
    let everyone: Vec<(Guid, Option<Instant>)> = map
        .players
        .values()
        .filter(|p| !p.gm)
        .map(|p| (p.guid, p.eviction))
        .collect();
    for (g, pending) in everyone {
        let ok = belongs(&map.realm, g, map.key);
        match (ok, pending) {
            (true, Some(_)) => {
                if let Some(p) = map.players.get_mut(&g) {
                    p.eviction = None;
                    raid_group_only(p, Duration::ZERO);
                }
            }
            (false, None) => {
                if let Some(p) = map.players.get_mut(&g) {
                    p.eviction = Some(now + GRACE);
                    raid_group_only(p, GRACE);
                }
            }
            (false, Some(at)) if at <= now => evict(map, g),
            _ => {}
        }
    }
}

/// Home: the player's bind, or, with none, outside the copy's entrance.
fn evict(map: &mut Map, g: Guid) {
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    p.eviction = None;
    raid_group_only(p, Duration::ZERO);
    let home = p.row.state.bind.map(|b| {
        (
            MapKey::world(b.map),
            Vector3d {
                x: b.x,
                y: b.y,
                z: b.z,
            },
        )
    });
    let entrance = || {
        map.realm
            .data
            .area_triggers
            .values()
            .find(|t| t.teleport.as_ref().is_some_and(|tp| tp.map == map.key.map))
            .map(|t| {
                (
                    MapKey::world(t.map),
                    Vector3d {
                        x: t.x,
                        y: t.y,
                        z: t.z,
                    },
                )
            })
    };
    if let Some((key, at)) = home.or_else(entrance) {
        map.teleport(g, key, at, 0.0);
    }
}

/// A player arriving from `p.row.map` onto `to`: leaving a copy (a
/// dungeon's, a raid's, a battleground's) for the open world, the client
/// hears which once it's there; a grace under way is over.
pub fn transferred(realm: &crate::game::Realm, p: &mut Player, to: MapKey) {
    let from = p.row.map;
    p.eviction = None;
    let copy = matches!(instance_type(realm, from), 1..=4)
        || crate::game::battlegrounds::bg_of_map(from).is_some();
    if copy && to.instance == 0 && from != to.map {
        p.last_instance = Some(from);
    }
}

/// `MSG_MOVE_WORLDPORT_ACK`: the copy just left, if one was; one still in
/// a match elsewhere has left it.
pub fn arrived(map: &mut Map, g: Guid) {
    crate::game::battlegrounds::strayed(map, g);
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    if let Some(from) = p.last_instance.take() {
        p.outbound.send(&SMSG_UPDATE_LAST_INSTANCE {
            map: crate::game::map::movement::map_enum(from),
        });
    }
}

/// A player logged in on this map: one taken back into their match joins
/// it.
pub fn logged_in(map: &mut Map, g: Guid) {
    if map.battleground.is_some() {
        crate::game::map::battleground::arrived(map, g);
    }
}
