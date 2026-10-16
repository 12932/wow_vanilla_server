//! Dungeon and raid copies. Each copy is a map actor of its own, keyed by
//! the map and an instance id. A group shares its copies; a player alone
//! has theirs. A dungeon copy goes when it has been empty half an hour (or
//! its owner resets it); a raid binds those who enter until its reset day,
//! and the binds are kept as records.

use std::time::Duration;

use ahash::AHashMap;
use serde::{Deserialize, Serialize};
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::guid;
use crate::game::map::{Map, MapKey};

pub const KIND_RAID: &str = "raid";
/// How long an empty copy lasts.
pub const UNLOAD_EMPTY_AFTER: Duration = Duration::from_secs(30 * 60);
/// Raid resets count from a Tuesday, 04:00 UTC.
const RESET_EPOCH: i64 = 1_101_182_400;
const DAY: i64 = 86_400;

/// Who a copy belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Owner {
    Group(u32),
    Player(u32),
}

/// A raid copy and those bound to it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RaidSave {
    pub id: u32,
    pub map: u32,
    /// Unix seconds.
    pub reset_at: i64,
    pub players: Vec<u32>,
}

#[derive(Debug, Default)]
pub struct Instances {
    next_id: u32,
    owned: AHashMap<(Owner, u32), u32>,
    pub raids: AHashMap<u32, RaidSave>,
}

impl Instances {
    pub fn load(records: &[(String, u64, String)]) -> Self {
        let mut i = Self::default();
        for (_, _, data) in records.iter().filter(|r| r.0 == KIND_RAID) {
            if let Ok(save) = serde_json::from_str::<RaidSave>(data) {
                i.next_id = i.next_id.max(save.id);
                i.raids.insert(save.id, save);
            }
        }
        i
    }

    /// The raid copy of `map` that `player` is bound to.
    /// A new copy's id.
    pub fn new_id(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id
    }

    pub fn raid_bind(&self, player: u32, map: u32) -> Option<u32> {
        self.raids
            .values()
            .find(|r| r.map == map && r.players.contains(&player))
            .map(|r| r.id)
    }
}

pub fn instance_type(realm: &Realm, map: u32) -> u32 {
    realm.data.maps.get(&map).map_or(0, |m| m.instance_type)
}

/// How many may be in a copy of `map`.
pub fn limit(map: u32, instance_type: u32) -> usize {
    match (map, instance_type) {
        (309 | 509, _) => 20,
        (_, 2) => 40,
        (229, _) => 10,
        _ => 5,
    }
}

/// Days between a raid's resets: Onyxia five, Zul'Gurub and the Ruins
/// three, the rest a week.
pub fn reset_days(map: u32) -> i64 {
    match map {
        249 => 5,
        309 | 509 => 3,
        _ => 7,
    }
}

/// The next reset of `map` after `now`.
pub fn next_reset(map: u32, now: i64) -> i64 {
    let period = reset_days(map) * DAY;
    let since = (now - RESET_EPOCH).max(0);
    RESET_EPOCH + (since / period + 1) * period
}

/// Why a player can't go into a copy.
#[derive(Debug, Clone, PartialEq)]
pub enum Refused {
    Abort(TransferAbortReason),
    Message(String),
}

/// The copy of `map` `g` goes into: the raid they are bound to, their
/// group's, or their own, made if there is none.
pub fn key_for(realm: &Realm, g: Guid, gm: bool, map: u32) -> Result<MapKey, Refused> {
    let kind = instance_type(realm, map);
    if !matches!(kind, 1 | 2) {
        return Ok(MapKey::world(map));
    }
    let me = guid::counter(g);
    let group = {
        let groups = realm.social.groups.lock();
        groups.group_of(g).map(|gr| (gr.id, gr.raid))
    };
    if kind == 2 && !gm && !group.is_some_and(|(_, raid)| raid) {
        return Err(Refused::Message(
            "You must be in a raid group to enter this instance.".into(),
        ));
    }
    let mut inst = realm.social.instances.lock();
    let owner = match group {
        Some((id, _)) => Owner::Group(id),
        None => Owner::Player(me),
    };
    let id = match inst.raid_bind(me, map) {
        Some(id) => id,
        None => match inst.owned.get(&(owner, map)) {
            Some(id) => *id,
            None => {
                inst.next_id += 1;
                let id = inst.next_id;
                inst.owned.insert((owner, map), id);
                id
            }
        },
    };
    let key = MapKey { map, instance: id };
    let inside = realm
        .registry
        .all_online()
        .iter()
        .filter(|o| o.guid != g && o.map().key == key)
        .count();
    if inside >= limit(map, kind) && !gm {
        return Err(Refused::Abort(TransferAbortReason::IsFull));
    }
    if kind == 2 {
        let now = crate::game::social::mail::now();
        let save = inst.raids.entry(id).or_insert_with(|| RaidSave {
            id,
            map,
            reset_at: next_reset(map, now),
            players: Vec::new(),
        });
        if !save.players.contains(&me) {
            save.players.push(me);
            realm.store.put_record(KIND_RAID, u64::from(id), save);
        }
    }
    Ok(key)
}

/// A copy unloaded: a dungeon's owners forget it, so the next visit gets a
/// fresh one.
pub fn unloaded(realm: &Realm, key: MapKey) {
    let mut inst = realm.social.instances.lock();
    if !inst.raids.contains_key(&key.instance) {
        inst.owned
            .retain(|(_, map), id| !(*map == key.map && *id == key.instance));
    }
}

/// `CMSG_RESET_INSTANCES`: the player's (or, for a group's leader, the
/// group's) empty dungeon copies are reset.
pub fn reset(map: &Map, g: Guid) {
    let realm = &map.realm;
    let owner = {
        let groups = realm.social.groups.lock();
        match groups.group_of(g) {
            Some(gr) if gr.leader != g => return,
            Some(gr) => Owner::Group(gr.id),
            None => Owner::Player(guid::counter(g)),
        }
    };
    let mine: Vec<(u32, u32)> = realm
        .social
        .instances
        .lock()
        .owned
        .iter()
        .filter(|((o, _), _)| *o == owner)
        .map(|((_, m), id)| (*m, *id))
        .collect();
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let online = realm.registry.all_online();
    for (m, id) in mine {
        let key = MapKey {
            map: m,
            instance: id,
        };
        let occupied = online.iter().any(|o| o.map().key == key);
        let map_enum = crate::game::map::movement::map_enum(m);
        if occupied {
            p.outbound.send(&SMSG_INSTANCE_RESET_FAILED {
                reason: InstanceResetFailedReason::General,
                map: map_enum,
            });
            continue;
        }
        realm.social.instances.lock().owned.remove(&(owner, m));
        realm.maps.stop(key);
        p.outbound.send(&SMSG_INSTANCE_RESET { map: map_enum });
    }
}

/// `CMSG_REQUEST_RAID_INFO`: the raids the player is bound to.
pub fn raid_info(map: &Map, g: Guid) {
    let me = guid::counter(g);
    let now = crate::game::social::mail::now();
    let raid_infos = map
        .realm
        .social
        .instances
        .lock()
        .raids
        .values()
        .filter(|r| r.players.contains(&me))
        .map(|r| RaidInfo {
            map: crate::game::map::movement::map_enum(r.map),
            reset_time: (r.reset_at - now).max(0) as u32,
            instance_id: r.id,
        })
        .collect();
    if let Some(p) = map.players.get(&g) {
        p.outbound.send(&SMSG_RAID_INSTANCE_INFO { raid_infos });
    }
}

/// Raid copies past their reset day go, binds and all.
pub fn sweep(realm: &Realm) {
    let now = crate::game::social::mail::now();
    let expired: Vec<RaidSave> = {
        let mut inst = realm.social.instances.lock();
        let ids: Vec<u32> = inst
            .raids
            .values()
            .filter(|r| r.reset_at <= now)
            .map(|r| r.id)
            .collect();
        ids.iter().filter_map(|id| inst.raids.remove(id)).collect()
    };
    for r in expired {
        realm.store.delete_record(KIND_RAID, u64::from(r.id));
        realm.maps.stop(MapKey {
            map: r.map,
            instance: r.id,
        });
        realm
            .social
            .instances
            .lock()
            .owned
            .retain(|_, id| *id != r.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raids_reset_on_their_days() {
        // A Wednesday: the next weekly reset is the Tuesday after.
        let wednesday = RESET_EPOCH + DAY;
        assert_eq!(next_reset(409, wednesday), RESET_EPOCH + 7 * DAY);
        assert_eq!(next_reset(309, wednesday), RESET_EPOCH + 3 * DAY);
        assert_eq!(limit(36, 1), 5);
        assert_eq!(limit(409, 2), 40);
        assert_eq!(limit(309, 2), 20);
    }
}
