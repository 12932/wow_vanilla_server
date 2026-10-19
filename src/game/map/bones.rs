//! Bones: what a player's corpse turns to when they rise, as
//! mangos-classic's `ObjectAccessor::ConvertCorpseForPlayer` makes them
//! (called by `Player::SpawnCorpseBones` at a reclaim, a spirit healer's or
//! a spell's resurrection). The corpse goes and bones lie where it was,
//! seen by everyone: the corpse's fields with its flags `CORPSE_FLAG_UNK2 |
//! CORPSE_FLAG_BONES`, its owner kept and the gear it wore gone. They lie
//! `[players] bones_hours`, and aren't kept across a restart (mangos never
//! saves bones). Insignia taken in a battleground leave bones too
//! (`battleground::insignia`).

use std::time::Duration;

use ahash::AHashSet;
use wow_world_messages::Guid;

use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::death::Corpse;

/// `CORPSE_FIELD_FLAGS`: bones (mangos's `CORPSE_FLAG_BONES`), and the
/// flag every player's corpse has (`CORPSE_FLAG_UNK2`).
pub const CORPSE_FLAG_BONES: u32 = 0x01;
pub const CORPSE_FLAG_UNK2: u32 = 0x04;
/// The equipment slots a corpse shows (`CORPSE_FIELD_ITEM`).
const EQUIPMENT_SLOTS: u16 = 19;

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
/// bones a new object, of a GUID no player's corpse has. The bones' GUID,
/// or `None` with no corpse here.
pub fn convert(map: &mut Map, owner: Guid, lies: Duration, dynamic_flags: u32) -> Option<Guid> {
    let corpse = guid::corpse(guid::counter(owner));
    let c = map.corpses.get(&corpse)?;
    let (position, orientation, cell) = (c.position, c.orientation, c.cell);
    let g = next_guid(map);
    // Everything but the GUID and the type, as mangos copies it.
    let mut fields = Fields::new(TypeId::Corpse, g, 0);
    let c = &map.corpses[&corpse];
    for i in OBJECT_FIELD_ENTRY..c.fields.len() {
        fields.set_u32(i, c.fields.get_u32(i));
    }
    fields.set_u32(CORPSE_FIELD_FLAGS, CORPSE_FLAG_UNK2 | CORPSE_FLAG_BONES);
    fields.set_u32(CORPSE_FIELD_DYNAMIC_FLAGS, dynamic_flags);
    for slot in 0..EQUIPMENT_SLOTS {
        fields.set_u32(CORPSE_FIELD_ITEM + slot, 0);
    }
    map.remove_corpse(corpse);
    map.add_corpse(Corpse {
        guid: g,
        owner,
        fields,
        position,
        orientation,
        observers: AHashSet::new(),
        cell,
    });
    let until = map.now + lies;
    map.bones.push((until, g));
    Some(g)
}

/// A GUID for new bones: a counter of the map's with the top bit set,
/// which no player's corpse (the player's own counter) has.
fn next_guid(map: &mut Map) -> Guid {
    let counter = map.next_creature_counter;
    map.next_creature_counter += 1;
    guid::corpse(0x8000_0000 | counter)
}

/// Bones that have lain their time go.
pub fn update(map: &mut Map) {
    let now = map.now;
    if map.bones.iter().all(|(t, _)| *t > now) {
        return;
    }
    let (gone, rest): (Vec<_>, Vec<_>) = map.bones.drain(..).partition(|(t, _)| *t <= now);
    map.bones = rest;
    for (_, g) in gone {
        map.remove_corpse(g);
    }
}
