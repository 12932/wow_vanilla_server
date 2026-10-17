//! LocalDefense's "under attack" alert, as Thistle Tea ports vmangos's
//! `Creature::SendZoneUnderAttackMessage`: a player (or what they
//! control) killing a town guard, or a creature that flags its killer for
//! PvP, warns the other side's players on that map that the creature's
//! area is under attack, at most once every ten seconds an area.

use std::sync::LazyLock;
use std::time::{Duration, Instant};

use ahash::AHashMap;
use parking_lot::Mutex;
use wow_world_messages::Guid;

use crate::game::data::items::Body;
use crate::game::guid;
use crate::game::map::Map;

pub const SMSG_ZONE_UNDER_ATTACK: u16 = 0x254;

/// How long an area's alert keeps it quiet.
const COOLDOWN: Duration = Duration::from_secs(10);

/// `flags_extra`: a town guard.
const EXTRA_GUARD: u32 = 0x400;
/// `static_flags1`: it flags its killer for PvP.
const STATIC_PVP_ENABLING: u32 = 0x0040_0000;

/// When each area last raised its alert, across the realm.
static ALERTED: LazyLock<Mutex<AHashMap<u32, Instant>>> = LazyLock::new(Default::default);

/// Whether a creature defends its area: a guard, or one that flags its
/// killer, and nobody's pet.
pub fn defender(map: &Map, g: Guid) -> bool {
    let Some(c) = map.creature(g) else {
        return false;
    };
    if c.owner.is_some() || c.pet.is_some() {
        return false;
    }
    map.realm.data.creature_template(c.entry).is_some_and(|t| {
        t.flags_extra & EXTRA_GUARD != 0 || t.static_flags1 & STATIC_PVP_ENABLING != 0
    })
}

/// Whether the alert for `area` is due at `now`, and if so it is raised.
fn due(area: u32, now: Instant) -> bool {
    let mut alerted = ALERTED.lock();
    match alerted.get(&area) {
        Some(at) if now.duration_since(*at) <= COOLDOWN => false,
        _ => {
            alerted.insert(area, now);
            true
        }
    }
}

/// A creature fell to `killer` (a player, or what one controls): if it
/// defended its area, the area's defenders hear of it.
pub fn fallen(map: &mut Map, victim: Guid, killer: Guid) {
    let player = crate::game::map::pets::master(map, killer);
    if !guid::is_player(player) || !defender(map, victim) {
        return;
    }
    let Some(team) = map.players.get(&player).map(|p| p.team) else {
        return;
    };
    let Some(at) = map.position_of(victim) else {
        return;
    };
    // Its area, or its zone where the area is unknown; with no navmesh or
    // terrain, the killer's.
    let area = crate::game::map::world::zone_and_area(map, at)
        .map(|(zone, area)| if area != 0 { area } else { zone })
        .filter(|a| *a != 0)
        .or_else(|| {
            map.players
                .get(&player)
                .map(|p| if p.area != 0 { p.area } else { p.zone })
                .filter(|a| *a != 0)
        });
    let Some(area) = area else {
        return;
    };
    if !due(area, map.now) {
        return;
    }
    let mut b = Body::default();
    b.u32(area);
    let frame = b.frame(SMSG_ZONE_UNDER_ATTACK);
    for p in map.players.values().filter(|p| p.team != team) {
        p.outbound.send_frame(frame.clone());
    }
}
