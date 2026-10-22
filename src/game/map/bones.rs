//! Bones: what a player's corpse turns to when they rise, as
//! mangos-classic's `ObjectAccessor::ConvertCorpseForPlayer` makes them
//! (called by `Player::SpawnCorpseBones` at a reclaim, a spirit healer's or
//! a spell's resurrection). The corpse goes and bones lie where it was,
//! seen by everyone: the corpse's fields with its flags `CORPSE_FLAG_UNK2 |
//! CORPSE_FLAG_BONES`, its owner kept and the gear it wore gone. They lie
//! `[players] bones_hours`. Insignia taken in a battleground leave bones
//! too (`battleground::insignia`).
//!
//! Bones in the open world are kept across a restart, in the store's
//! `bones` table (mangos never saves bones): a row when they are made,
//! deleted when they go. As a map starts it takes its bones still to go
//! (`restore`), where they lay, as they looked, with the time they had
//! left; the store sweeps those gone while it was closed. A copy's bones
//! (a dungeon's, a battleground's) aren't kept: they go with the copy, and
//! no copy comes back from a restart as it was.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use ahash::AHashMap;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::death::Corpse;
use crate::game::map::grid::cell_of;
use crate::game::store::BonesRow;

/// `CORPSE_FIELD_FLAGS`: bones (mangos's `CORPSE_FLAG_BONES`), and the
/// flag every player's corpse has (`CORPSE_FLAG_UNK2`).
pub const CORPSE_FLAG_BONES: u32 = 0x01;
pub const CORPSE_FLAG_UNK2: u32 = 0x04;
/// The equipment slots a corpse shows (`CORPSE_FIELD_ITEM`).
const EQUIPMENT_SLOTS: u16 = 19;
/// The bit of a bones GUID's counter over its id, which no player's
/// corpse (the player's own counter) has.
const BONES_BIT: u32 = 0x8000_0000;

/// The realm's bones ids, and the bones kept in the store waiting for
/// their map to start.
#[derive(Debug, Default)]
pub struct Kept {
    next_id: AtomicU32,
    waiting: parking_lot::Mutex<AHashMap<u32, Vec<BonesRow>>>,
}

impl Kept {
    /// The bones the store had, by map; ids go on from `max_id`.
    pub fn load(rows: Vec<BonesRow>, max_id: u32) -> Self {
        let mut waiting: AHashMap<u32, Vec<BonesRow>> = AHashMap::new();
        for r in rows {
            waiting.entry(r.map).or_default().push(r);
        }
        Self {
            next_id: AtomicU32::new(max_id),
            waiting: parking_lot::Mutex::new(waiting),
        }
    }

    /// A new bones id: within the 31 bits under [`BONES_BIT`].
    fn next_id(&self) -> u32 {
        self.next_id.fetch_add(1, Ordering::Relaxed).wrapping_add(1) & !BONES_BIT
    }

    /// The kept bones of `map` yet to be restored, now taken.
    fn take(&self, map: u32) -> Vec<BonesRow> {
        self.waiting.lock().remove(&map).unwrap_or_default()
    }

    /// Bones to be restored again, as a map starts afresh.
    fn give_back(&self, map: u32, rows: Vec<BonesRow>) {
        if !rows.is_empty() {
            self.waiting.lock().entry(map).or_default().extend(rows);
        }
    }
}

/// A player rose: their corpse in this map, if it lies here, turns to
/// bones lying `bones_hours` (none at 0: the corpse just goes). The bones'
/// GUID, if any.
pub fn at_rising(map: &mut Map, owner: Guid) -> Option<Guid> {
    let hours = map.cfg.players.bones_hours;
    if hours == 0 {
        map.remove_corpse(guid::corpse(guid::counter(owner)));
        return None;
    }
    convert(map, owner, Duration::from_secs(hours * 3600), 0)
}

/// `owner`'s corpse in this map turned to bones where it lies, for `lies`,
/// with `dynamic_flags` (an insignia's loot): the corpse is forgotten, the
/// bones a new object, of a GUID no player's corpse has, kept in the store
/// in the open world. The bones' GUID, or `None` with no corpse here.
pub fn convert(map: &mut Map, owner: Guid, lies: Duration, dynamic_flags: u32) -> Option<Guid> {
    let corpse = guid::corpse(guid::counter(owner));
    let c = map.corpses.get(&corpse)?;
    let (position, orientation, cell) = (c.position, c.orientation, c.cell);
    let id = map.realm.bones.next_id();
    let g = guid_of(id);
    // Everything but the GUID and the type, as mangos copies it.
    let mut fields = Fields::new(TypeId::Corpse, g, 0);
    for i in OBJECT_FIELD_ENTRY..c.fields.len() {
        fields.set_u32(i, c.fields.get_u32(i));
    }
    fields.set_u32(CORPSE_FIELD_FLAGS, CORPSE_FLAG_UNK2 | CORPSE_FLAG_BONES);
    fields.set_u32(CORPSE_FIELD_DYNAMIC_FLAGS, dynamic_flags);
    for slot in 0..EQUIPMENT_SLOTS {
        fields.set_u32(CORPSE_FIELD_ITEM + slot, 0);
    }
    map.remove_corpse(corpse);
    let bones = Corpse {
        guid: g,
        owner,
        fields,
        position,
        orientation,
        observers: Default::default(),
        cell,
    };
    if map.key.instance == 0 {
        let expires_at = crate::game::social::mail::now() + lies.as_secs() as i64;
        map.realm
            .store
            .put_bones(row_of(map.key.map, id, &bones, expires_at));
    }
    map.add_corpse(bones);
    let until = map.now + lies;
    map.bones.push((until, g));
    Some(g)
}

/// Bones of id `id`.
fn guid_of(id: u32) -> Guid {
    guid::corpse(BONES_BIT | id)
}

/// The id of bones `g`.
fn id_of(g: Guid) -> u32 {
    guid::counter(g) & !BONES_BIT
}

/// Bones as the store keeps them.
fn row_of(map: u32, id: u32, c: &Corpse, expires_at: i64) -> BonesRow {
    let f = &c.fields;
    BonesRow {
        id,
        map,
        x: c.position.x,
        y: c.position.y,
        z: c.position.z,
        orientation: c.orientation,
        owner: guid::counter(c.owner),
        display: f.get_u32(CORPSE_FIELD_DISPLAY_ID),
        bytes_1: f.get_u32(CORPSE_FIELD_BYTES_1),
        bytes_2: f.get_u32(CORPSE_FIELD_BYTES_2),
        guild: f.get_u32(CORPSE_FIELD_GUILD),
        flags: f.get_u32(CORPSE_FIELD_FLAGS),
        dynamic_flags: f.get_u32(CORPSE_FIELD_DYNAMIC_FLAGS),
        expires_at,
    }
}

/// Kept bones as they lie again: the fields [`convert`] leaves them.
fn bones_of(r: &BonesRow) -> Corpse {
    let g = guid_of(r.id);
    let owner = guid::player(r.owner);
    let position = Vector3d {
        x: r.x,
        y: r.y,
        z: r.z,
    };
    let mut f = Fields::new(TypeId::Corpse, g, 0);
    f.set_guid(CORPSE_FIELD_OWNER, owner);
    f.set_f32(CORPSE_FIELD_FACING, r.orientation);
    f.set_f32(CORPSE_FIELD_POS_X, r.x);
    f.set_f32(CORPSE_FIELD_POS_X + 1, r.y);
    f.set_f32(CORPSE_FIELD_POS_X + 2, r.z);
    f.set_u32(CORPSE_FIELD_DISPLAY_ID, r.display);
    f.set_u32(CORPSE_FIELD_BYTES_1, r.bytes_1);
    f.set_u32(CORPSE_FIELD_BYTES_2, r.bytes_2);
    f.set_u32(CORPSE_FIELD_GUILD, r.guild);
    f.set_u32(CORPSE_FIELD_FLAGS, r.flags);
    f.set_u32(CORPSE_FIELD_DYNAMIC_FLAGS, r.dynamic_flags);
    Corpse {
        guid: g,
        owner,
        fields: f,
        position,
        orientation: r.orientation,
        observers: Default::default(),
        cell: cell_of(r.x, r.y),
    }
}

/// A map starting: the bones kept of it lie again, each for the time it
/// had left; those whose time is up go from the store.
pub fn restore(map: &mut Map) {
    if map.key.instance != 0 {
        return;
    }
    let rows = map.realm.bones.take(map.key.map);
    if rows.is_empty() {
        return;
    }
    let unix = crate::game::social::mail::now();
    let mut restored = 0;
    for r in rows {
        if r.expires_at <= unix {
            map.realm.store.delete_bones(r.id);
            continue;
        }
        let c = bones_of(&r);
        // Ids are the realm's; one the map already has is a bug's.
        if map.corpses.contains_key(&c.guid) {
            tracing::debug!("{}: bones {} twice; the second dropped", map.key, r.id);
            continue;
        }
        let g = c.guid;
        map.add_corpse(c);
        let left = Duration::from_secs((r.expires_at - unix) as u64);
        map.bones.push((map.now + left, g));
        restored += 1;
    }
    if restored > 0 {
        tracing::info!("{}: {restored} bones lie again", map.key);
    }
}

/// A map about to be built again (`supervise::restart`): its bones are
/// given back to be restored in the new one, with the time they have left.
pub fn hand_back(map: &Map) {
    if map.key.instance != 0 {
        return;
    }
    let unix = crate::game::social::mail::now();
    let rows = map
        .bones
        .iter()
        .filter_map(|(until, g)| {
            let c = map.corpses.get(g)?;
            let left = until.saturating_duration_since(map.now).as_secs() as i64;
            Some(row_of(map.key.map, id_of(*g), c, unix + left))
        })
        .collect();
    map.realm.bones.give_back(map.key.map, rows);
}

/// Bones that have lain their time go, from the store too.
pub fn update(map: &mut Map) {
    let now = map.now;
    if map.bones.iter().all(|(t, _)| *t > now) {
        return;
    }
    let (gone, rest): (Vec<_>, Vec<_>) = map.bones.drain(..).partition(|(t, _)| *t <= now);
    map.bones = rest;
    for (_, g) in gone {
        map.remove_corpse(g);
        if map.key.instance == 0 {
            map.realm.store.delete_bones(id_of(g));
        }
    }
}
