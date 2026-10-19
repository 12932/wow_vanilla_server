//! The run back to a corpse, as Thistle Tea's `Death.CorpseReclaim`,
//! `Death.CorpseTravel` and `Player.Corpses` have it:
//! - deaths close together lengthen the wait to reclaim a corpse, 30, 60
//!   then 120 seconds, the history fading five minutes at a time;
//! - a ghost whose corpse is in a dungeon is shown the dungeon's entrance
//!   outside, enters only the dungeon its corpse is in (or one inside it),
//!   and comes back to life at the portal.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::{SMSG_AREA_TRIGGER_MESSAGE, Vector3d};

use crate::game::data::scripts::MapTemplate;
use crate::game::data::{GameData, Teleport};
use crate::game::map::Map;
use crate::game::map::death::Death;
use crate::game::map::player::Player;

/// How long each death is remembered.
pub const DECAY_MS: i64 = 300_000;
pub const SMSG_CORPSE_RECLAIM_DELAY: u16 = 0x269;
/// The wait after one, two, and three or more deaths close together.
pub const DELAYS_MS: [i64; 3] = [30_000, 60_000, 120_000];

pub fn unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// The history after a death at `now`, from what it was: one period more
/// than is left, by up to three.
pub fn after_death(until: i64, now: i64) -> i64 {
    let periods = if until > now {
        ((until - now) / DECAY_MS + 2).min(3)
    } else {
        1
    };
    now + periods * DECAY_MS
}

/// A player died at `now`.
pub fn died(p: &mut Player, now: i64) {
    p.row.state.reclaim_until = after_death(p.row.state.reclaim_until, now);
}

/// The wait to reclaim at `now`, by the history left.
pub fn delay_ms(until: i64, now: i64) -> i64 {
    let count = ((until - now).max(0) / DECAY_MS).min(2);
    DELAYS_MS[count as usize]
}

/// What is left of the wait since the release, at `now`.
pub fn remaining_ms(p: &Player, now: i64) -> i64 {
    let released = p.row.state.released_at;
    if released <= 0 {
        return 0;
    }
    (released + delay_ms(p.row.state.reclaim_until, released) - now).max(0)
}

/// How long a ghost has been one, from its release.
pub fn since_release(p: &Player, now: i64) -> Duration {
    Duration::from_millis((now - p.row.state.released_at).max(0) as u64)
}

/// Whether a ghost may reclaim their corpse now: the wait, by the history
/// as it stands now, is over.
pub fn ready(p: &Player, map_now: std::time::Instant) -> bool {
    let Death::Ghost { since } = p.death else {
        return false;
    };
    let wait = delay_ms(p.row.state.reclaim_until, unix_ms());
    map_now.saturating_duration_since(since) >= Duration::from_millis(wait as u64)
}

/// `SMSG_CORPSE_RECLAIM_DELAY`, in milliseconds as the client reads it
/// (wow_messages has it in seconds).
pub fn send_delay(p: &Player, ms: u32) {
    let mut b = crate::game::data::items::Body::default();
    b.u32(ms);
    p.outbound.send_frame(b.frame(SMSG_CORPSE_RECLAIM_DELAY));
}

/// A dungeon or raid's template.
pub fn dungeon(data: &GameData, map: u32) -> Option<&MapTemplate> {
    data.scripts
        .map_templates
        .get(&map)
        .filter(|t| matches!(t.map_type, 1 | 2))
}

/// Whether a corpse in `map` is in `target`, or in a dungeon inside it.
fn linked(data: &GameData, map: u32, target: u32) -> bool {
    let mut at = map;
    let mut seen = Vec::new();
    while let Some(t) = dungeon(data, at) {
        if t.map == target {
            return true;
        }
        if t.parent == 0 || seen.contains(&at) {
            return false;
        }
        seen.push(at);
        at = t.parent;
    }
    false
}

/// The first portal into `map`.
fn entrance(data: &GameData, map: u32) -> Option<Teleport> {
    data.area_triggers
        .values()
        .filter_map(|t| t.teleport.as_ref().map(|tp| (t.id, tp)))
        .filter(|(_, tp)| tp.map == map)
        .min_by_key(|(id, _)| *id)
        .map(|(_, tp)| tp.clone())
}

/// Where a portal into a dungeon takes `g`: for a ghost, only into the
/// dungeon their corpse is in (through it to an inner one); `Err` with
/// what they are told otherwise.
pub fn portal(map: &Map, g: Guid, tp: &Teleport) -> Result<Teleport, String> {
    let data = &map.realm.data;
    let Some(p) = map.players.get(&g) else {
        return Ok(tp.clone());
    };
    let Some(d) = dungeon(data, tp.map).filter(|_| p.death != Death::Alive) else {
        return Ok(tp.clone());
    };
    match p.row.state.corpse {
        Some(c) if linked(data, c.map, tp.map) => Ok(if c.map == tp.map {
            tp.clone()
        } else {
            entrance(data, c.map).unwrap_or_else(|| tp.clone())
        }),
        _ => {
            let name = if d.name.is_empty() {
                "this dungeon"
            } else {
                &d.name
            };
            Err(format!("You cannot enter {name} while in ghost form."))
        }
    }
}

/// A portal turned a ghost away.
pub fn refuse(map: &Map, g: Guid, text: String) {
    if let Some(p) = map.players.get(&g) {
        p.outbound
            .send(&SMSG_AREA_TRIGGER_MESSAGE { message: text });
    }
}

/// A ghost going through a portal into the dungeon their corpse is in
/// comes back to life first, with half their health and mana.
pub fn revive_for_map(map: &mut Map, g: Guid, target: u32) {
    let data = map.realm.data.clone();
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let corpse_there = p.row.state.corpse.is_some_and(|c| c.map == target);
    if matches!(p.death, Death::Ghost { .. })
        && dungeon(&data, target).is_some()
        && map.key.map != target
        && corpse_there
    {
        crate::game::map::death::resurrect(map, g, 0.5);
    }
}

/// A player arriving in a map: a corpse of theirs left there from before
/// they came back to life is bones.
pub fn arrived(map: &mut Map, g: Guid) {
    let corpse = crate::game::guid::corpse(crate::game::guid::counter(g));
    let alive = map
        .players
        .get(&g)
        .is_some_and(|p| p.row.state.corpse.is_none());
    if alive && map.corpses.contains_key(&corpse) {
        crate::game::map::bones::at_rising(map, g);
    }
}

/// Where a ghost's corpse is shown: in a dungeon they are outside, its
/// entrance (on the ground there); else the corpse itself.
pub fn marker(map: &mut Map, corpse_map: u32, at: Vector3d) -> (u32, Vector3d) {
    let data = map.realm.data.clone();
    let entrance = dungeon(&data, corpse_map)
        .filter(|_| corpse_map != map.key.map)
        .and_then(|d| d.ghost_entrance);
    let Some((m, x, y)) = entrance else {
        return (corpse_map, at);
    };
    let z = if m == map.key.map {
        map.nav
            .as_mut()
            .and_then(|n| n.ground(x, y, 100_000.0))
            .unwrap_or(0.0)
    } else {
        0.0
    };
    (m, Vector3d { x, y, z })
}

/// The zone a dungeon's graveyards are found by: the player's, unless no
/// graveyard is linked to it and the dungeon's own zone has one.
pub fn graveyard_zone(map: &Map, zone: u32) -> u32 {
    let data = &map.realm.data;
    let linked = |z: u32| data.graveyards.iter().any(|gy| gy.zone == z);
    match dungeon(data, map.key.map) {
        Some(d) if d.linked_zone != 0 && !linked(zone) && linked(d.linked_zone) => d.linked_zone,
        _ => zone,
    }
}

/// Where a graveyard's distance is reckoned from for a ghost in `map` at
/// `pos`: on another map, the dungeon's entrance there.
pub fn reckoned_from(data: &GameData, map: u32, graveyard_map: u32, pos: Vector3d) -> Vector3d {
    match dungeon(data, map).and_then(|d| d.ghost_entrance) {
        Some((m, x, y)) if m == graveyard_map && m != map => Vector3d { x, y, z: pos.z },
        _ => pos,
    }
}
