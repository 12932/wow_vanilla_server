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
//! minute's grace, then goes home ([`crate::game::map::eviction`]). A group
//! is held by its leader's raid binds, and one who goes into a raid copy
//! their group is bound to is bound to it too. A raid that resets with
//! players inside takes no one more and goes once they have left.

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
/// How long an empty copy lasts, by default (`[instances]
/// unload_delay_secs`); what it remembers is kept at least this long.
/// mangos's: a dungeon copy resets `NORMAL_INSTANCE_RESET_TIME` (30
/// minutes) after its last player leaves, and `Instance.UnloadDelay` is 30
/// minutes too.
pub const UNLOAD_EMPTY_AFTER: Duration = Duration::from_secs(30 * 60);
/// Raid resets count from a Tuesday, 04:00 UTC.
const RESET_EPOCH: i64 = 1_101_182_400;
/// The hour of [`RESET_EPOCH`], the default of `[instances] reset_hour`.
pub const RESET_HOUR: u32 = 4;
const DAY: i64 = 86_400;
/// An account's copies an hour, by default (`[instances]
/// entries_per_hour`).
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
    /// Who has gone in, by GUID counter: each player is on the list of the
    /// copy of the map they last went into.
    pub entered: Vec<u32>,
    /// Its raid reset with players inside: it takes no one more, and goes
    /// once they have left.
    pub expired: bool,
    /// The chests its script raised that have been opened, by spawn: what
    /// is left in them, so one raised again after a reload keeps it.
    pub chests: BTreeMap<u32, SavedChest>,
    /// Its game objects gone (gathered, looted), by spawn, and when (Unix
    /// seconds) each comes back: mangos's `gameobject_respawn`.
    pub objects: BTreeMap<u32, i64>,
    /// A group's copy that a restart left without an owner (the group
    /// gone with it): until a group takes it up, it is still for those
    /// who were in it, those whose last copy of its map it is, as mangos's
    /// group binds outlast a restart.
    pub held: bool,
}

/// What a raised chest holds, as it was last left.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SavedChest {
    pub items: Vec<SavedSlot>,
    pub gold: u32,
    /// Emptied and gone: when (Unix seconds) it comes back, filled anew.
    pub back_at: Option<i64>,
}

/// A slot of a saved chest.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SavedSlot {
    pub item: u32,
    pub count: u32,
    pub taken: bool,
    pub random_property: u32,
    pub quest: bool,
    /// For a quest item, who has taken theirs, by GUID counter.
    pub taken_by: Vec<u32>,
    pub quality: u32,
    pub condition: u32,
}

#[derive(Debug, Default)]
pub struct Instances {
    next_id: u32,
    owned: AHashMap<(Owner, u32), u32>,
    pub raids: AHashMap<u32, RaidSave>,
    pub saves: AHashMap<u32, InstanceSave>,
    /// Each account's copies entered in the last hour: (copy, Unix seconds).
    entries: AHashMap<String, Vec<(MapKey, i64)>>,
    /// Those let into each copy and not in it yet, with when (Unix
    /// seconds): they count against its limit until they arrive, so two
    /// let in at once can't both take its last place.
    on_the_way: AHashMap<MapKey, Vec<(Guid, i64)>>,
    /// The open world's respawn times.
    pub world: crate::game::respawns::World,
}

/// How long one let into a copy counts against its limit before they
/// arrive: a way in takes a moment, so one not there a minute on isn't
/// coming.
const ON_THE_WAY_SECS: i64 = 60;

impl Instances {
    pub fn load(records: &[(String, u64, String)]) -> Self {
        let mut i = Self::default();
        let now = now();
        i.world = crate::game::respawns::World::load(records, now);
        use crate::game::store::parse_record;
        for (kind, id, data) in records {
            let id = *id as u32;
            match kind.as_str() {
                KIND_RAID => {
                    // Every id counts, read or not (`Mails::load`).
                    i.next_id = i.next_id.max(id);
                    if let Some(mut save) = parse_record::<RaidSave>(KIND_RAID, id.into(), data) {
                        save.id = id;
                        i.raids.insert(save.id, save);
                    }
                }
                KIND_INSTANCE => {
                    i.next_id = i.next_id.max(id);
                    if let Some(mut save) =
                        parse_record::<InstanceSave>(KIND_INSTANCE, id.into(), data)
                    {
                        save.id = id;
                        // Empty since the restart, as far as the sweep can
                        // tell: it has as long from now as a copy left
                        // empty has. Its stored time is its last kill or
                        // entry (a copy in use isn't written as it is
                        // touched), and one that was occupied at the
                        // shutdown never emptied: before, the first sweep,
                        // at start-up, took every copy whose last kill was
                        // five minutes before, the downtime counted too,
                        // and those saved inside went home at their login.
                        save.touched_at = now;
                        if let Some(owner) = save.owner
                            && !save.expired
                        {
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

    /// The copies of groups that aren't, `kept` telling which are, left
    /// without an owner, for their old leader's next group to take up:
    /// the groups kept in the store keep theirs across a restart, as
    /// mangos's `group_instance` binds.
    pub fn orphan_groups(&mut self, kept: impl Fn(u32) -> bool) {
        for save in self.saves.values_mut() {
            if let Some(Owner::Group(g)) = save.owner
                && !kept(g)
            {
                self.owned.remove(&(Owner::Group(g), save.map));
                save.owner = None;
            }
        }
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
    /// hour (its hour starts again), or one more than the `most` it has.
    fn admit(&mut self, account: &str, key: MapKey, now: i64, most: usize) -> bool {
        let entries = self.entries.entry(account.to_owned()).or_default();
        entries.retain(|(_, at)| *at + 3600 > now);
        if let Some(e) = entries.iter_mut().find(|(k, _)| *k == key) {
            e.1 = now;
            return true;
        }
        if entries.len() >= most {
            return false;
        }
        entries.push((key, now));
        true
    }

    /// How many copies `account` has entered in the hour to `now`, for the
    /// tests.
    #[cfg(test)]
    pub(crate) fn entries_within_hour(&self, account: &str, now: i64) -> usize {
        self.entries
            .get(account)
            .map_or(0, |e| e.iter().filter(|(_, at)| *at + 3600 > now).count())
    }

    /// The last copy id handed out, for the tests.
    #[cfg(test)]
    pub(crate) fn last_id(&self) -> u32 {
        self.next_id
    }

    /// Whether players are bound to copy `id` of a raid.
    fn saved(&self, id: u32) -> bool {
        self.raids.get(&id).is_some_and(|r| !r.players.is_empty())
    }

    /// The copy of `map` a restart left held that `player` was in.
    fn held_for(&self, player: u32, map: u32) -> Option<u32> {
        self.last_entered(player, map).filter(|id| {
            self.saves
                .get(id)
                .is_some_and(|s| s.held && s.owner.is_none() && !s.expired)
        })
    }

    /// The copy of `map` that `player` last went into.
    fn last_entered(&self, player: u32, map: u32) -> Option<u32> {
        self.saves
            .values()
            .find(|s| s.map == map && s.entered.contains(&player))
            .map(|s| s.id)
    }

    /// `player` went into copy `id` of `map`: it is the one of the map they
    /// last went into. The saves changed, to be stored.
    fn enter(&mut self, player: u32, map: u32, id: u32) -> Vec<InstanceSave> {
        let mut changed = Vec::new();
        for s in self.saves.values_mut().filter(|s| s.map == map) {
            let was = s.entered.contains(&player);
            if s.id == id && !was {
                s.entered.push(player);
            } else if s.id != id && was {
                s.entered.retain(|p| *p != player);
            } else {
                continue;
            }
            changed.push(s.clone());
        }
        changed
    }

    /// `g` is let into `key`: on their way there, and nowhere else. A way
    /// in that never ended is forgotten too.
    fn heading(&mut self, g: Guid, key: MapKey, now: i64) {
        self.on_the_way.retain(|_, way| {
            way.retain(|(p, at)| *p != g && *at + ON_THE_WAY_SECS > now);
            !way.is_empty()
        });
        self.on_the_way.entry(key).or_default().push((g, now));
    }

    /// Whether anyone let into `key` is on their way there now.
    fn anyone_coming(&self, key: MapKey, now: i64) -> bool {
        self.on_the_way
            .get(&key)
            .is_some_and(|way| way.iter().any(|(_, at)| *at + ON_THE_WAY_SECS > now))
    }

    /// Those on their way into `key` now, but `g`.
    fn coming(&mut self, key: MapKey, g: Guid, now: i64) -> Vec<Guid> {
        let Some(way) = self.on_the_way.get_mut(&key) else {
            return Vec::new();
        };
        way.retain(|(_, at)| *at + ON_THE_WAY_SECS > now);
        let coming = way.iter().map(|(p, _)| *p).filter(|p| *p != g).collect();
        if way.is_empty() {
            self.on_the_way.remove(&key);
        }
        coming
    }

    /// Every copy's owner, account's hour and way in forgotten, for a test
    /// that makes many copies.
    #[cfg(test)]
    pub(crate) fn forget_admissions(&mut self) {
        self.owned.clear();
        self.entries.clear();
        self.on_the_way.clear();
    }
}

pub fn instance_type(realm: &Realm, map: u32) -> u32 {
    realm.data.maps.get(&map).map_or(0, |m| m.instance_type)
}

/// How many may be in a copy of `map`: its template's player limit, none
/// for 0 or no template.
pub fn limit(data: &crate::game::data::GameData, map: u32) -> usize {
    match data.scripts.map_templates.get(&map).map(|t| t.player_limit) {
        Some(n) if n > 0 => n as usize,
        _ => usize::MAX,
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

/// The next reset of `map` after `now`, at 04:00 UTC.
pub fn next_reset(map: u32, now: i64) -> i64 {
    next_reset_at(map, now, RESET_HOUR)
}

/// The next reset of `map` after `now`, at `hour` UTC (`[instances]
/// reset_hour`).
pub fn next_reset_at(map: u32, now: i64, hour: u32) -> i64 {
    let epoch = RESET_EPOCH + (i64::from(hour.min(23)) - i64::from(RESET_HOUR)) * 3600;
    let period = reset_days(map) * DAY;
    let since = (now - epoch).max(0);
    epoch + (since / period + 1) * period
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

/// Who `g` goes in with: their group's id, whether it is a raid, and its
/// leader.
fn group_of(realm: &Realm, g: Guid) -> Option<(u32, bool, Guid)> {
    let groups = realm.social.groups.lock();
    groups.group_of(g).map(|gr| (gr.id, gr.raid, gr.leader))
}

/// The raid copy of `map` a group is bound to: its leader's, as Thistle
/// Tea binds a group to its leader's saves when it forms and when its
/// leader changes.
fn group_bind(inst: &Instances, group: Option<(u32, bool, Guid)>, map: u32) -> Option<u32> {
    group.and_then(|(_, _, leader)| inst.raid_bind(guid::counter(leader), map))
}

/// The copy of `map` `g` goes into: the raid their group or they are bound
/// to, their group's, or their own, made if there is none.
pub fn key_for(realm: &Realm, g: Guid, gm: bool, map: u32) -> Result<MapKey, Refused> {
    let kind = instance_type(realm, map);
    if !matches!(kind, 1 | 2) {
        return Ok(MapKey::world(map));
    }
    let me = guid::counter(g);
    let group = group_of(realm, g);
    if kind == 2 && !gm && !group.is_some_and(|(_, raid, _)| raid) {
        return Err(Refused::RaidGroupOnly);
    }
    // (A raid group still, at the way in: mangos's `GetAreaTriggerLockStatus`.)
    let owner = group.map_or(Owner::Player(me), |(id, _, _)| Owner::Group(id));
    let account = account_of(realm, g);
    let now = now();
    let reset_hour = realm.config().instances.reset_hour;
    // One lock from choosing the copy to storing it, admission between:
    // two of a group going in at once both find the copy the first made,
    // and two let into a copy's last place can't both take it.
    let mut inst = realm.social.instances.lock();
    let bound = inst.raid_bind(me, map);
    let group_bound = group_bind(&inst, group, map);
    // A group that follows its leader's save leaves the copy it had.
    if let Some(b) = group_bound
        && inst.owned.get(&(owner, map)).is_some_and(|o| *o != b)
    {
        inst.owned.remove(&(owner, map));
    }
    let owned = inst.owned.get(&(owner, map)).copied();
    // A copy a restart left held, theirs who were in it: back into it,
    // as mangos's group bind outlasts the restart. It stays held, for the
    // rest of them.
    let held = inst.held_for(me, map);
    let selected = group_bound.or(bound).or(owned).or(held);
    // A raid bind holds: into another copy of the raid, no.
    if let Some(id) = selected
        && !gm
        && ([group_bound, bound]
            .iter()
            .any(|b| b.is_some_and(|b| b != id))
            || (kind == 2 && owned.is_some_and(|o| o != id)))
    {
        return Err(Refused::Abort(TransferAbortReason::NotFound));
    }
    // A new copy's id is spent only once it is let in.
    let id = selected.unwrap_or(inst.next_id + 1);
    let key = MapKey { map, instance: id };
    if gm {
        // Let in past every limit, but on their way all the same, so the
        // copy isn't reset under them.
        inst.heading(g, key, now);
    } else {
        admit_locked(&mut inst, realm, g, &account, key, now)?;
    }
    if selected.is_none() {
        inst.new_id();
    }
    if held != Some(id) {
        inst.owned.entry((owner, map)).or_insert(id);
    }
    let save = inst.saves.entry(id).or_insert_with(|| InstanceSave {
        id,
        map,
        owner: Some(owner),
        ..Default::default()
    });
    save.touched_at = now;
    if kind == 2 {
        inst.raids.entry(id).or_insert_with(|| RaidSave {
            id,
            map,
            reset_at: next_reset_at(map, now, reset_hour),
            players: Vec::new(),
        });
    }
    drop(inst);
    entered(realm, g, key, kind, group_bound == Some(id));
    Ok(key)
}

/// Whether `g` may go into the copy `key` now: it is not gone, it has
/// room, and their account has not been into five others this hour.
fn admit_copy(realm: &Realm, g: Guid, gm: bool, key: MapKey) -> Result<(), Refused> {
    if gm {
        realm.social.instances.lock().heading(g, key, now());
        return Ok(());
    }
    let account = account_of(realm, g);
    let mut inst = realm.social.instances.lock();
    admit_locked(&mut inst, realm, g, &account, key, now())
}

/// `g`'s account, for its hourly count.
fn account_of(realm: &Realm, g: Guid) -> String {
    realm
        .registry
        .online(g)
        .map(|o| o.account.to_string())
        .unwrap_or_default()
}

/// [`admit_copy`] with the copies locked, from the count to the way in:
/// those inside and those let in on their way count against its limit,
/// and one let in is on their way until they arrive ([`arrived`]).
fn admit_locked(
    inst: &mut Instances,
    realm: &Realm,
    g: Guid,
    account: &str,
    key: MapKey,
    now: i64,
) -> Result<(), Refused> {
    if inst.saves.get(&key.instance).is_some_and(|s| s.expired) {
        return Err(Refused::Abort(TransferAbortReason::NotFound));
    }
    let mut inside: ahash::AHashSet<Guid> = realm
        .registry
        .all_online()
        .iter()
        .filter(|o| o.guid != g && o.map().key == key)
        .map(|o| o.guid)
        .collect();
    inside.extend(inst.coming(key, g, now));
    if inside.len() >= limit(&realm.data, key.map) {
        return Err(Refused::Abort(TransferAbortReason::IsFull));
    }
    let most = realm.config().instances.entries_per_hour;
    if !inst.admit(account, key, now, most) {
        return Err(Refused::Abort(TransferAbortReason::TooManyInstances));
    }
    inst.heading(g, key, now);
    Ok(())
}

/// `g` has arrived in the copy `key`: counted inside it now, no more on
/// their way.
pub fn arrived(realm: &Realm, key: MapKey, g: Guid) {
    let mut inst = realm.social.instances.lock();
    if let Some(way) = inst.on_the_way.get_mut(&key) {
        way.retain(|(p, _)| *p != g);
        if way.is_empty() {
            inst.on_the_way.remove(&key);
        }
    }
}

/// Why the copy `key` can't go now: someone in it, or someone let in and
/// on their way (mangos's "zoning"), whose online entry names the map they
/// left until the copy has them ([`arrived`]). Asked under the copies'
/// lock, under which every way in is let in, so none is let in before the
/// caller acts on the answer. Before, a reset and a raid's reset looked
/// at the online list alone, and stopped a copy with a player's arrival in
/// its queue.
fn in_use(
    inst: &Instances,
    realm: &Realm,
    key: MapKey,
    now: i64,
) -> Option<InstanceResetFailedReason> {
    if inst.anyone_coming(key, now) {
        return Some(InstanceResetFailedReason::Zoning);
    }
    realm
        .registry
        .all_online()
        .iter()
        .any(|o| o.map().key == key)
        .then_some(InstanceResetFailedReason::General)
}

/// `g` goes into copy `key`: it is the one of its map they last went into,
/// and one who goes into a raid copy their group is bound to is bound to it
/// too, as Thistle Tea's `Lockouts.inherit` has it.
fn entered(realm: &Realm, g: Guid, key: MapKey, kind: u32, group_bound: bool) {
    let me = guid::counter(g);
    let raid = {
        let mut inst = realm.social.instances.lock();
        let changed = inst.enter(me, key.map, key.instance);
        let raid = (kind == 2 && group_bound && inst.raid_bind(me, key.map).is_none())
            .then(|| {
                inst.raids.get_mut(&key.instance).map(|r| {
                    r.players.push(me);
                    r.clone()
                })
            })
            .flatten();
        let mut changed = changed;
        if let Some(s) = inst.saves.get(&key.instance)
            && !changed.iter().any(|c| c.id == s.id)
        {
            changed.push(s.clone());
        }
        // Sent under the lock, so the store has the copy's records in the
        // order they changed (another map's thread changes them too).
        for s in changed {
            realm.store.put_record(KIND_INSTANCE, u64::from(s.id), &s);
        }
        if let Some(r) = &raid {
            realm.store.put_record(KIND_RAID, u64::from(r.id), r);
        }
        raid
    };
    if raid.is_some() {
        lockout_changed(realm, g, true);
    }
}

/// Back into the copy of `map` a player logged out in, as Thistle Tea's
/// `resume` has it: the copy they last went into, if it is still theirs
/// and will have them; else none, and they go home.
pub fn resume(realm: &Realm, g: Guid, gm: bool, map: u32) -> Option<MapKey> {
    let kind = instance_type(realm, map);
    if !matches!(kind, 1 | 2) {
        return Some(MapKey::world(map));
    }
    let me = guid::counter(g);
    let id = {
        let inst = realm.social.instances.lock();
        inst.last_entered(me, map)
            .filter(|id| inst.saves.get(id).is_some_and(|s| !s.expired))?
    };
    let key = MapKey { map, instance: id };
    if !gm && !belongs(realm, g, key) {
        return None;
    }
    admit_copy(realm, g, gm, key).ok()?;
    // The group first: the groups' lock is never taken under this one's,
    // as a group's change takes this one under theirs.
    let group = group_of(realm, g);
    let group_bound = {
        let inst = realm.social.instances.lock();
        group_bind(&inst, group, map) == Some(id)
    };
    entered(realm, g, key, kind, group_bound);
    Some(key)
}

/// A group formed, went, or changed its leader: its copies follow, as
/// Thistle Tea's `Instance.group_changed` hands them over. `before` and
/// `after` are the group as it was and is (its id and leader), `None` for
/// none. Called with the groups locked, so it takes their lock no more.
pub fn group_changed(realm: &Realm, before: Option<(u32, Guid)>, after: Option<(u32, Guid)>) {
    match (before, after) {
        (None, Some((group, leader))) => adopt(realm, group, leader),
        (Some((group, _)), None) => orphan(realm, group),
        (Some((group, old)), Some((same, new))) if group == same && old != new => {
            releader(realm, group, old, new)
        }
        _ => {}
    }
}

/// A group formed takes over its leader's copies, the one of each map
/// they last went into, that were theirs alone or were left without an
/// owner, and are neither gone nor bound to a raid. The leader alone has
/// them no more.
fn adopt(realm: &Realm, group: u32, leader: Guid) {
    let me = guid::counter(leader);
    {
        let mut inst = realm.social.instances.lock();
        let ids: Vec<u32> = inst
            .saves
            .values()
            .filter(|s| s.entered.contains(&me))
            .filter(|s| s.owner.is_none_or(|o| o == Owner::Player(me)))
            .filter(|s| !s.expired)
            .map(|s| s.id)
            .collect();
        let mut changed = Vec::new();
        for id in ids {
            if inst.saved(id) {
                continue;
            }
            let Some(s) = inst.saves.get_mut(&id) else {
                continue;
            };
            let (map, old) = (s.map, s.owner);
            s.owner = Some(Owner::Group(group));
            s.held = false;
            changed.push(s.clone());
            if let Some(old) = old
                && inst.owned.get(&(old, map)) == Some(&id)
            {
                inst.owned.remove(&(old, map));
            }
            inst.owned.insert((Owner::Group(group), map), id);
        }
        for s in changed {
            realm.store.put_record(KIND_INSTANCE, u64::from(s.id), &s);
        }
    }
}

/// A group gone leaves its copies without an owner, for its old leader's
/// next group to take up.
fn orphan(realm: &Realm, group: u32) {
    let mut inst = realm.social.instances.lock();
    inst.owned.retain(|(o, _), _| *o != Owner::Group(group));
    for s in inst
        .saves
        .values_mut()
        .filter(|s| s.owner == Some(Owner::Group(group)))
    {
        s.owner = None;
        realm.store.put_record(KIND_INSTANCE, u64::from(s.id), &*s);
    }
}

/// A group under a new leader lets go of its copies of the raids either
/// leader is bound to: the new leader's binds hold it now.
fn releader(realm: &Realm, group: u32, old: Guid, new: Guid) {
    let leaders = [guid::counter(old), guid::counter(new)];
    let mut inst = realm.social.instances.lock();
    let maps: Vec<u32> = inst
        .raids
        .values()
        .filter(|r| r.players.iter().any(|p| leaders.contains(p)))
        .map(|r| r.map)
        .collect();
    for map in maps {
        inst.owned.remove(&(Owner::Group(group), map));
    }
}

/// Whether `g` belongs in the copy `key` still: it is not gone, no bind
/// of theirs or their group's holds them to another, and it is their
/// group's (or, alone, their own), their group's raid, or the raid they
/// are bound to; a raid's is for a raid group.
///
/// A group's copy a restart left [`InstanceSave::held`] is still theirs
/// who were in it, grouped or not, raid group or not: mangos keeps a
/// player in the copy their group (saved with it) is bound to, and asks
/// for a raid group only at the way in, not at a login.
pub fn belongs(realm: &Realm, g: Guid, key: MapKey) -> bool {
    let kind = instance_type(realm, key.map);
    if key.instance == 0 || !matches!(kind, 1 | 2) {
        return true;
    }
    let group = group_of(realm, g);
    let me = guid::counter(g);
    let owner = group.map_or(Owner::Player(me), |(id, _, _)| Owner::Group(id));
    let inst = realm.social.instances.lock();
    if inst.saves.get(&key.instance).is_none_or(|s| s.expired) {
        return false;
    }
    let bound = inst.raid_bind(me, key.map);
    let group_bound = group_bind(&inst, group, key.map);
    if [bound, group_bound]
        .iter()
        .any(|b| b.is_some_and(|b| b != key.instance))
    {
        return false;
    }
    if inst.held_for(me, key.map) == Some(key.instance) {
        return true;
    }
    if kind == 2 && !group.is_some_and(|(_, raid, _)| raid) {
        return false;
    }
    inst.owned.get(&(owner, key.map)) == Some(&key.instance)
        || group_bound == Some(key.instance)
        || bound == Some(key.instance)
}

/// Whether the copy `key` has gone with its raid's reset, and goes once it
/// is empty.
pub fn expired(realm: &Realm, key: MapKey) -> bool {
    key.instance != 0
        && realm
            .social
            .instances
            .lock()
            .saves
            .get(&key.instance)
            .is_some_and(|s| s.expired)
}

/// A player's raid binds changed: they hear of the new save, if one was
/// made, then what they are saved to now, as Thistle Tea's
/// `lockout_changed` tells them.
fn lockout_changed(realm: &Realm, g: Guid, created: bool) {
    let Some(o) = realm.registry.online(g) else {
        return;
    };
    if created {
        o.outbound.send(&SMSG_INSTANCE_SAVE_CREATED { unknown: 0 });
    }
    send_saved(realm, g, &o.outbound);
    o.outbound.send(&SMSG_RAID_INSTANCE_INFO {
        raid_infos: raid_infos(realm, guid::counter(g)),
    });
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
    // A battleground's match is never resumed: what dies in it isn't saved
    // (in Alterac Valley, hundreds of writes a match, kept past its end).
    if key.instance == 0 || crate::game::battlegrounds::bg_of_map(key.map).is_some() {
        return;
    }
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
    // Sent under the lock: a change on another thread (a player entering,
    // on their map's) can't reach the store ahead of this one with what
    // it had before, and be written over it.
    realm
        .store
        .put_record(KIND_INSTANCE, u64::from(key.instance), &*save);
}

/// A spawn died; it comes back after `respawn` (`None`: never, a boss in
/// a copy). Saved in its copy, or in the open world's respawns.
pub fn spawn_died(realm: &Realm, key: MapKey, spawn: u32, respawn: Option<Duration>) {
    let back = respawn.map_or(NEVER, |d| now() + d.as_secs() as i64);
    if key.instance == 0 {
        crate::game::respawns::gone(realm, false, key.map, spawn, back);
        return;
    }
    change_save(realm, key, |s| {
        s.dead.insert(spawn, back);
    });
}

/// A spawn is back.
pub fn spawn_back(realm: &Realm, key: MapKey, spawn: u32) {
    if key.instance == 0 {
        crate::game::respawns::back(realm, false, key.map, spawn);
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

/// A spawned game object went (gathered, looted, used up); it comes back
/// after `after`. Saved in its copy, or in the open world's respawns.
pub fn object_gone(realm: &Realm, key: MapKey, spawn: u32, after: Duration) {
    let back = now() + after.as_secs() as i64;
    if key.instance == 0 {
        crate::game::respawns::gone(realm, true, key.map, spawn, back);
        return;
    }
    change_save(realm, key, |s| {
        s.objects.insert(spawn, back);
    });
}

/// A spawned game object is back.
pub fn object_back(realm: &Realm, key: MapKey, spawn: u32) {
    if key.instance == 0 {
        crate::game::respawns::back(realm, true, key.map, spawn);
        return;
    }
    let had = realm
        .social
        .instances
        .lock()
        .saves
        .get(&key.instance)
        .is_some_and(|s| s.objects.contains_key(&spawn));
    if had {
        change_save(realm, key, |s| {
            s.objects.remove(&spawn);
        });
    }
}

/// What of map `key` is down as it starts, by spawn, and when (Unix
/// seconds) each comes back: its creatures, then its game objects.
pub fn down_at_start(realm: &Realm, key: MapKey) -> (BTreeMap<u32, i64>, BTreeMap<u32, i64>) {
    let inst = realm.social.instances.lock();
    if key.instance == 0 {
        return inst.world.of(key.map);
    }
    inst.saves
        .get(&key.instance)
        .map(|s| (s.dead.clone(), s.objects.clone()))
        .unwrap_or_default()
}

/// The instance script's data, saved with the copy.
pub fn data_changed(realm: &Realm, key: MapKey, data: &BTreeMap<u32, u32>) {
    let data = data.clone();
    change_save(realm, key, |s| s.data = data);
}

/// A raised chest's loot as it is now, saved with the copy; `None`
/// forgets it.
pub fn chest_changed(realm: &Realm, key: MapKey, spawn: u32, chest: Option<SavedChest>) {
    let unchanged = realm
        .social
        .instances
        .lock()
        .saves
        .get(&key.instance)
        .is_some_and(|s| s.chests.get(&spawn) == chest.as_ref());
    if !unchanged {
        change_save(realm, key, |s| match chest {
            Some(c) => {
                s.chests.insert(spawn, c);
            }
            None => {
                s.chests.remove(&spawn);
            }
        });
    }
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
    let reset_hour = map.cfg.instances.reset_hour;
    let newly = {
        let mut inst = realm.social.instances.lock();
        let save = inst.raids.entry(key.instance).or_insert_with(|| RaidSave {
            id: key.instance,
            map: key.map,
            reset_at: next_reset_at(key.map, now(), reset_hour),
            players: Vec::new(),
        });
        let mut newly = Vec::new();
        for (g, p) in &players {
            if !save.players.contains(p) {
                save.players.push(*p);
                newly.push(*g);
            }
        }
        if !newly.is_empty() {
            realm
                .store
                .put_record(KIND_RAID, u64::from(key.instance), &*save);
        }
        newly
    };
    for g in newly {
        lockout_changed(realm, g, true);
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
    let now = now();
    for (m, id) in mine {
        let key = MapKey {
            map: m,
            instance: id,
        };
        let map_enum = crate::game::map::movement::map_enum(m);
        // Asked and reset under one lock: none is let in between.
        let refused = {
            let mut inst = realm.social.instances.lock();
            let refused = in_use(&inst, realm, key, now);
            if refused.is_none() {
                inst.owned.remove(&(owner, m));
                inst.saves.remove(&id);
                realm.store.delete_record(KIND_INSTANCE, u64::from(id));
            }
            refused
        };
        if let Some(reason) = refused {
            p.outbound.send(&SMSG_INSTANCE_RESET_FAILED {
                reason,
                map: map_enum,
            });
            continue;
        }
        realm.maps.stop(key);
        p.outbound.send(&SMSG_INSTANCE_RESET { map: map_enum });
    }
}

/// A character deleted: their raid binds go, as mangos's `DeleteFromDB`
/// deletes their `character_instance` rows.
pub fn character_deleted(realm: &Realm, me: u32) {
    let mut inst = realm.social.instances.lock();
    for r in inst.raids.values_mut().filter(|r| r.players.contains(&me)) {
        r.players.retain(|p| *p != me);
        realm.store.put_record(KIND_RAID, u64::from(r.id), &*r);
    }
}

/// The raids player `me` is bound to, and how long each has left.
fn raid_infos(realm: &Realm, me: u32) -> Vec<RaidInfo> {
    let now = now();
    let mut infos: Vec<RaidInfo> = realm
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
    infos.sort_by_key(|i| i.map.as_int());
    infos
}

/// `CMSG_REQUEST_RAID_INFO`: the raids the player is bound to.
pub fn raid_info(map: &Map, g: Guid) {
    let raid_infos = raid_infos(&map.realm, guid::counter(g));
    if let Some(p) = map.players.get(&g) {
        p.outbound.send(&SMSG_RAID_INSTANCE_INFO { raid_infos });
    }
}

/// Raid copies past their reset day go, binds and all (an occupied one
/// once it empties); dungeon saves of copies empty long enough (as after a
/// restart) go too.
pub fn sweep(realm: &Realm) {
    crate::game::respawns::sweep(realm);
    let now = now();
    let due: Vec<u32> = {
        let inst = realm.social.instances.lock();
        inst.raids
            .values()
            .filter(|r| r.reset_at <= now)
            .map(|r| r.id)
            .collect()
    };
    expire(realm, &due);
    let unload =
        (realm.config().instance_unload.as_secs() as i64).max(UNLOAD_EMPTY_AFTER.as_secs() as i64);
    // Asked and removed under one lock, those on their way in counting as
    // inside: none is let in between.
    {
        let mut inst = realm.social.instances.lock();
        let online = realm.registry.all_online();
        let stale: Vec<u32> = inst
            .saves
            .values()
            .filter(|s| !inst.raids.contains_key(&s.id))
            .filter(|s| s.touched_at + unload < now)
            .filter(|s| !online.iter().any(|o| o.map().key.instance == s.id))
            .filter(|s| {
                !inst.anyone_coming(
                    MapKey {
                        map: s.map,
                        instance: s.id,
                    },
                    now,
                )
            })
            .map(|s| s.id)
            .collect();
        for id in &stale {
            inst.saves.remove(id);
            inst.owned.retain(|_, i| i != id);
            realm.store.delete_record(KIND_INSTANCE, u64::from(*id));
        }
    }
}

/// Every copy of raid `map` resets now, as a GM may have it: what
/// [`sweep`] does on the reset day. How many copies there were.
pub fn reset_raid(realm: &Realm, map: u32) -> usize {
    let ids: Vec<u32> = {
        let inst = realm.social.instances.lock();
        inst.raids
            .values()
            .filter(|r| r.map == map)
            .map(|r| r.id)
            .collect()
    };
    expire(realm, &ids);
    ids.len()
}

/// Raid copies `ids` reset, as Thistle Tea's `expire_raid` has it: their
/// binds go, and those who were bound hear of it; an empty copy goes at
/// once, and one with players inside takes no one more, its players have
/// the minute's grace, and it goes once they have left.
fn expire(realm: &Realm, ids: &[u32]) {
    if ids.is_empty() {
        return;
    }
    let now = now();
    let mut told = Vec::new();
    for id in ids {
        let r = {
            let mut inst = realm.social.instances.lock();
            let Some(r) = inst.raids.remove(id) else {
                continue;
            };
            realm.store.delete_record(KIND_RAID, u64::from(r.id));
            r
        };
        told.extend(r.players.iter().copied());
        let key = MapKey {
            map: r.map,
            instance: r.id,
        };
        // Asked and decided under one lock: one on their way in counts as
        // inside, so the copy waits for them to leave.
        let (occupied, kept) = {
            let mut inst = realm.social.instances.lock();
            let occupied = in_use(&inst, realm, key, now).is_some();
            inst.owned.retain(|_, i| *i != r.id);
            let kept = if occupied {
                inst.saves.get_mut(&r.id).map(|s| {
                    s.expired = true;
                    realm.store.put_record(KIND_INSTANCE, u64::from(s.id), &*s);
                })
            } else {
                inst.saves.remove(&r.id);
                None
            };
            if kept.is_none() {
                realm.store.delete_record(KIND_INSTANCE, u64::from(r.id));
            }
            (occupied, kept)
        };
        if kept.is_none() && !occupied {
            realm.maps.stop(key);
        }
    }
    told.sort_unstable();
    told.dedup();
    for o in realm
        .registry
        .all_online()
        .iter()
        .filter(|o| told.contains(&guid::counter(o.guid)))
    {
        lockout_changed(realm, o.guid, false);
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
        // At another hour (`[instances] reset_hour`): the same days, the
        // hour moved; an hour before 04:00 on the Tuesday, 03:00 is next.
        assert_eq!(
            next_reset_at(409, wednesday, 6),
            RESET_EPOCH + 7 * DAY + 2 * 3600
        );
        assert_eq!(
            next_reset_at(409, RESET_EPOCH + 7 * DAY - 2 * 3600, 3),
            RESET_EPOCH + 7 * DAY - 3600
        );
    }

    #[test]
    fn an_account_enters_five_copies_an_hour() {
        let mut i = Instances::default();
        let key = |n| MapKey {
            map: 36,
            instance: n,
        };
        let most = ENTRIES_AN_HOUR;
        for n in 1..=5 {
            assert!(i.admit("A", key(n), 1000, most));
        }
        assert!(!i.admit("A", key(6), 1000, most), "a sixth is refused");
        assert!(
            i.admit("A", key(3), 1500, most),
            "one already entered isn't"
        );
        assert!(
            i.admit("B", key(6), 1000, most),
            "another account has its own"
        );
        assert!(
            i.admit("A", key(6), 1000 + 3601, most),
            "an hour on, it may"
        );
        // At one an hour (`[instances] entries_per_hour`).
        assert!(i.admit("C", key(1), 1000, 1));
        assert!(!i.admit("C", key(2), 1000, 1), "one an hour");
    }
}
