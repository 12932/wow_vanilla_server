//! Dungeon and raid copies. Each copy is a map actor of its own, keyed by
//! the map and an instance id. A group shares its copies; a player alone
//! has theirs. Every copy is saved: its dead and when they come back, its
//! instance script's data, and who owns it, so a restart finds it as it
//! was. A dungeon copy goes when it has been empty a while (or its owner
//! resets it); a raid binds those in it for good when a boss that locks it
//! dies, until its reset day, and the binds are kept as records.
//!
//! Entry follows Thistle Tea's admission: a raid needs a raid group, a copy
//! holds its map's limit, and an account may enter at most five copies an
//! hour. A group formed by a copy's owner takes it over; one inside a copy
//! that is no longer theirs (out of the group, out of the raid) has a
//! minute's grace, then goes home ([`crate::game::map::eviction`]).

use std::collections::BTreeMap;
use std::time::Duration;

use ahash::AHashMap;
use serde::{Deserialize, Serialize};
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::guid;
use crate::game::map::{Map, MapKey};

pub const KIND_RAID: &str = "raid";
pub const KIND_INSTANCE: &str = "instance";
/// How long an empty copy lasts.
pub const UNLOAD_EMPTY_AFTER: Duration = Duration::from_secs(30 * 60);
/// Raid resets count from a Tuesday, 04:00 UTC.
const RESET_EPOCH: i64 = 1_101_182_400;
const DAY: i64 = 86_400;
/// An account's copies an hour.
pub const ENTRIES_AN_HOUR: usize = 5;
/// `static_flags2`: killing the creature binds the raid.
pub const STATIC_FLAG2_LOCKS_RAID: u32 = 0x4;
/// A spawn that never comes back on its own.
pub const NEVER: i64 = i64::MAX;

/// Who a copy belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

/// What a copy keeps across a restart.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct InstanceSave {
    pub id: u32,
    pub map: u32,
    pub owner: Option<Owner>,
    /// Unix seconds it was last in use.
    pub touched_at: i64,
    /// Dead spawns, and when (Unix seconds) each comes back.
    pub dead: BTreeMap<u32, i64>,
    /// The instance script's data.
    pub data: BTreeMap<u32, u32>,
}

#[derive(Debug, Default)]
pub struct Instances {
    next_id: u32,
    owned: AHashMap<(Owner, u32), u32>,
    pub raids: AHashMap<u32, RaidSave>,
    pub saves: AHashMap<u32, InstanceSave>,
    /// Each account's copies entered in the last hour: (copy, Unix seconds).
    entries: AHashMap<String, Vec<(MapKey, i64)>>,
}

impl Instances {
    pub fn load(records: &[(String, u64, String)]) -> Self {
        let mut i = Self::default();
        for (kind, _, data) in records {
            match kind.as_str() {
                KIND_RAID => {
                    if let Ok(save) = serde_json::from_str::<RaidSave>(data) {
                        i.next_id = i.next_id.max(save.id);
                        i.raids.insert(save.id, save);
                    }
                }
                KIND_INSTANCE => {
                    if let Ok(save) = serde_json::from_str::<InstanceSave>(data) {
                        i.next_id = i.next_id.max(save.id);
                        if let Some(owner) = save.owner {
                            i.owned.insert((owner, save.map), save.id);
                        }
                        i.saves.insert(save.id, save);
                    }
                }
                _ => {}
            }
        }
        i
    }

    /// A new copy's id.
    pub fn new_id(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id
    }

    /// The raid copy of `map` that `player` is bound to.
    pub fn raid_bind(&self, player: u32, map: u32) -> Option<u32> {
        self.raids
            .values()
            .find(|r| r.map == map && r.players.contains(&player))
            .map(|r| r.id)
    }

    /// Whether `account` may enter `key` now: a copy it entered in the
    /// hour, or a sixth.
    fn admit(&mut self, account: &str, key: MapKey, now: i64) -> bool {
        let entries = self.entries.entry(account.to_owned()).or_default();
        entries.retain(|(_, at)| *at + 3600 > now);
        if entries.iter().any(|(k, _)| *k == key) {
            return true;
        }
        if entries.len() >= ENTRIES_AN_HOUR {
            return false;
        }
        entries.push((key, now));
        true
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
    /// A raid, for a raid group only.
    RaidGroupOnly,
}

fn now() -> i64 {
    crate::game::social::mail::now()
}

/// The copy of `map` `g` goes into: the raid they are bound to, their
/// group's, or their own, made if there is none.
pub fn key_for(realm: &Realm, g: Guid, gm: bool, map: u32) -> Result<MapKey, Refused> {
    let kind = instance_type(realm, map);
    if !matches!(kind, 1 | 2) {
        return Ok(MapKey::world(map));
    }
    let me = guid::counter(g);
    let (group, leader) = {
        let groups = realm.social.groups.lock();
        (
            groups.group_of(g).map(|gr| (gr.id, gr.raid)),
            groups.group_of(g).map(|gr| gr.leader),
        )
    };
    if kind == 2 && !gm && !group.is_some_and(|(_, raid)| raid) {
        return Err(Refused::RaidGroupOnly);
    }
    let account = realm
        .registry
        .online(g)
        .map(|o| o.account.to_string())
        .unwrap_or_default();
    let now = now();
    let mut inst = realm.social.instances.lock();
    let owner = match group {
        Some((id, _)) => Owner::Group(id),
        None => Owner::Player(me),
    };
    if let (Some((id, _)), Some(leader)) = (group, leader) {
        adopt(realm, &mut inst, id, leader, map);
    }
    let bound = inst.raid_bind(me, map);
    let owned = inst.owned.get(&(owner, map)).copied();
    // A raid bind holds: into another copy of the raid, no.
    if let (Some(b), Some(o)) = (bound, owned)
        && b != o
        && !gm
    {
        return Err(Refused::Message(
            "You are already saved to another copy of this instance.".into(),
        ));
    }
    let id = match bound.or(owned) {
        Some(id) => id,
        None => {
            inst.next_id += 1;
            inst.next_id
        }
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
    if !gm && !inst.admit(&account, key, now) {
        return Err(Refused::Abort(TransferAbortReason::TooManyInstances));
    }
    inst.owned.entry((owner, map)).or_insert(id);
    let save = inst.saves.entry(id).or_insert_with(|| InstanceSave {
        id,
        map,
        owner: Some(owner),
        ..Default::default()
    });
    save.touched_at = now;
    let record = save.clone();
    if kind == 2 {
        inst.raids.entry(id).or_insert_with(|| RaidSave {
            id,
            map,
            reset_at: next_reset(map, now),
            players: Vec::new(),
        });
    }
    drop(inst);
    realm
        .store
        .put_record(KIND_INSTANCE, u64::from(id), &record);
    Ok(key)
}

/// A group takes over its leader's copy of `map` (one not yet locked to a
/// raid), or one left without an owner by a group gone that its leader is
/// in, as Thistle Tea's `Instance.group_changed` hands them over.
fn adopt(realm: &Realm, inst: &mut Instances, group: u32, leader: Guid, map: u32) {
    if inst.owned.contains_key(&(Owner::Group(group), map)) {
        return;
    }
    let solo = inst
        .owned
        .get(&(Owner::Player(guid::counter(leader)), map))
        .copied()
        .filter(|id| inst.raids.get(id).is_none_or(|r| r.players.is_empty()));
    let orphaned = || {
        let key = realm.registry.online(leader)?.map().key;
        (key.map == map && key.instance != 0)
            .then_some(key.instance)
            .filter(|id| {
                inst.saves.get(id).is_some_and(|s| match s.owner {
                    Some(Owner::Group(old)) => !realm.social.groups.lock().formed(old),
                    _ => false,
                })
            })
    };
    let Some(id) = solo.or_else(orphaned) else {
        return;
    };
    if let Some(old) = inst.saves.get(&id).and_then(|s| s.owner) {
        inst.owned.remove(&(old, map));
    }
    inst.owned.insert((Owner::Group(group), map), id);
    if let Some(s) = inst.saves.get_mut(&id) {
        s.owner = Some(Owner::Group(group));
        let record = s.clone();
        realm
            .store
            .put_record(KIND_INSTANCE, u64::from(id), &record);
    }
}

/// Whether `g` belongs in the copy `key` still: it is their group's (or,
/// alone, their own, or the raid they're bound to and own), and a raid's
/// is for a raid group.
pub fn belongs(realm: &Realm, g: Guid, key: MapKey) -> bool {
    let kind = instance_type(realm, key.map);
    if key.instance == 0 || !matches!(kind, 1 | 2) {
        return true;
    }
    let (group, raid, leader) = {
        let groups = realm.social.groups.lock();
        match groups.group_of(g) {
            Some(gr) => (Some(gr.id), gr.raid, Some(gr.leader)),
            None => (None, false, None),
        }
    };
    if kind == 2 && !raid {
        return false;
    }
    let me = guid::counter(g);
    let owner = group.map_or(Owner::Player(me), Owner::Group);
    let mut inst = realm.social.instances.lock();
    if let (Some(id), Some(leader)) = (group, leader) {
        adopt(realm, &mut inst, id, leader, key.map);
    }
    let bound = inst.raid_bind(me, key.map);
    if bound.is_some_and(|b| b != key.instance) {
        return false;
    }
    let held_by = inst.saves.get(&key.instance).and_then(|s| s.owner);
    held_by == Some(owner)
        || (bound == Some(key.instance) && held_by == Some(Owner::Player(me)))
}

/// What a player is saved to, as Thistle Tea tells it at login and each
/// worldport: whether any raid, and each raid's map.
pub fn send_saved(realm: &Realm, g: Guid, out: &crate::game::conn::Outbound) {
    let me = guid::counter(g);
    let maps: Vec<u32> = {
        let inst = realm.social.instances.lock();
        let mut m: Vec<u32> = inst
            .raids
            .values()
            .filter(|r| r.players.contains(&me))
            .map(|r| r.map)
            .collect();
        m.sort_unstable();
        m
    };
    out.send(&SMSG_UPDATE_INSTANCE_OWNERSHIP {
        player_is_saved_to_a_raid: !maps.is_empty(),
    });
    for map in maps {
        out.send(&SMSG_UPDATE_LAST_INSTANCE {
            map: crate::game::map::movement::map_enum(map),
        });
    }
}

/// A copy's save, as it stands.
pub fn save_of(realm: &Realm, id: u32) -> Option<InstanceSave> {
    realm.social.instances.lock().saves.get(&id).cloned()
}

fn change_save(realm: &Realm, key: MapKey, f: impl FnOnce(&mut InstanceSave)) {
    if key.instance == 0 {
        return;
    }
    let record = {
        let mut inst = realm.social.instances.lock();
        let save = inst
            .saves
            .entry(key.instance)
            .or_insert_with(|| InstanceSave {
                id: key.instance,
                map: key.map,
                ..Default::default()
            });
        f(save);
        save.touched_at = now();
        save.clone()
    };
    realm
        .store
        .put_record(KIND_INSTANCE, u64::from(key.instance), &record);
}

/// A spawn in a copy died; it comes back after `respawn` (`None`: never,
/// a boss).
pub fn spawn_died(realm: &Realm, key: MapKey, spawn: u32, respawn: Option<Duration>) {
    let back = respawn.map_or(NEVER, |d| now() + d.as_secs() as i64);
    change_save(realm, key, |s| {
        s.dead.insert(spawn, back);
    });
}

/// A spawn in a copy is back.
pub fn spawn_back(realm: &Realm, key: MapKey, spawn: u32) {
    if key.instance == 0 {
        return;
    }
    let had = realm
        .social
        .instances
        .lock()
        .saves
        .get(&key.instance)
        .is_some_and(|s| s.dead.contains_key(&spawn));
    if had {
        change_save(realm, key, |s| {
            s.dead.remove(&spawn);
        });
    }
}

/// The instance script's data, saved with the copy.
pub fn data_changed(realm: &Realm, key: MapKey, data: &BTreeMap<u32, u32>) {
    let data = data.clone();
    change_save(realm, key, |s| s.data = data);
}

/// A boss that locks its raid died: everyone in the copy is bound to it
/// for good.
pub fn bind_raid(map: &Map) {
    let key = map.key;
    if key.instance == 0 || instance_type(&map.realm, key.map) != 2 {
        return;
    }
    let realm = &map.realm;
    let players: Vec<(Guid, u32)> = map
        .players
        .keys()
        .map(|g| (*g, guid::counter(*g)))
        .collect();
    let (record, newly) = {
        let mut inst = realm.social.instances.lock();
        let save = inst.raids.entry(key.instance).or_insert_with(|| RaidSave {
            id: key.instance,
            map: key.map,
            reset_at: next_reset(key.map, now()),
            players: Vec::new(),
        });
        let mut newly = Vec::new();
        for (g, p) in &players {
            if !save.players.contains(p) {
                save.players.push(*p);
                newly.push(*g);
            }
        }
        (save.clone(), newly)
    };
    if newly.is_empty() {
        return;
    }
    realm
        .store
        .put_record(KIND_RAID, u64::from(key.instance), &record);
    for g in newly {
        if let Some(p) = map.players.get(&g) {
            p.outbound.send(&SMSG_INSTANCE_SAVE_CREATED { unknown: 0 });
            send_saved(realm, g, &p.outbound);
        }
    }
}

/// A copy unloaded: a dungeon's owners forget it, so the next visit gets a
/// fresh one; its save goes with it.
pub fn unloaded(realm: &Realm, key: MapKey) {
    let mut inst = realm.social.instances.lock();
    let bound = inst
        .raids
        .get(&key.instance)
        .is_some_and(|r| !r.players.is_empty());
    if !bound {
        inst.raids.remove(&key.instance);
        inst.owned
            .retain(|(_, map), id| !(*map == key.map && *id == key.instance));
        if inst.saves.remove(&key.instance).is_some() {
            realm
                .store
                .delete_record(KIND_INSTANCE, u64::from(key.instance));
        }
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
    let mine: Vec<(u32, u32)> = {
        let inst = realm.social.instances.lock();
        inst.owned
            .iter()
            .filter(|((o, _), id)| *o == owner && !inst.raids.contains_key(id))
            .map(|((_, m), id)| (*m, *id))
            .collect()
    };
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
        {
            let mut inst = realm.social.instances.lock();
            inst.owned.remove(&(owner, m));
            inst.saves.remove(&id);
        }
        realm.store.delete_record(KIND_INSTANCE, u64::from(id));
        realm.maps.stop(key);
        p.outbound.send(&SMSG_INSTANCE_RESET { map: map_enum });
    }
}

/// `CMSG_REQUEST_RAID_INFO`: the raids the player is bound to.
pub fn raid_info(map: &Map, g: Guid) {
    let me = guid::counter(g);
    let now = now();
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

/// Raid copies past their reset day go, binds and all; dungeon saves of
/// copies empty long enough (as after a restart) go too.
pub fn sweep(realm: &Realm) {
    let now = now();
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
        realm.store.delete_record(KIND_INSTANCE, u64::from(r.id));
        realm.maps.stop(MapKey {
            map: r.map,
            instance: r.id,
        });
        let mut inst = realm.social.instances.lock();
        inst.owned.retain(|_, id| *id != r.id);
        inst.saves.remove(&r.id);
    }
    let unload =
        (realm.config.instance_unload.as_secs() as i64).max(UNLOAD_EMPTY_AFTER.as_secs() as i64);
    let online = realm.registry.all_online();
    let stale: Vec<u32> = {
        let inst = realm.social.instances.lock();
        inst.saves
            .values()
            .filter(|s| !inst.raids.contains_key(&s.id))
            .filter(|s| s.touched_at + unload < now)
            .filter(|s| !online.iter().any(|o| o.map().key.instance == s.id))
            .map(|s| s.id)
            .collect()
    };
    for id in stale {
        {
            let mut inst = realm.social.instances.lock();
            inst.saves.remove(&id);
            inst.owned.retain(|_, i| *i != id);
        }
        realm.store.delete_record(KIND_INSTANCE, u64::from(id));
    }
}

/// A copy in use: its save is touched, so the sweep leaves it.
pub fn touch(realm: &Realm, key: MapKey) {
    if key.instance == 0 {
        return;
    }
    let mut inst = realm.social.instances.lock();
    if let Some(s) = inst.saves.get_mut(&key.instance) {
        s.touched_at = now();
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

    #[test]
    fn an_account_enters_five_copies_an_hour() {
        let mut i = Instances::default();
        let key = |n| MapKey {
            map: 36,
            instance: n,
        };
        for n in 1..=5 {
            assert!(i.admit("A", key(n), 1000));
        }
        assert!(!i.admit("A", key(6), 1000), "a sixth is refused");
        assert!(i.admit("A", key(3), 1500), "one already entered isn't");
        assert!(i.admit("B", key(6), 1000), "another account has its own");
        assert!(i.admit("A", key(6), 1000 + 3601), "an hour on, it may");
    }
}
