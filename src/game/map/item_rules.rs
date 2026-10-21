//! Item rules Thistle Tea keeps beyond the bags:
//! - an item bound to a map or zone (`item_template.map_bound` and
//!   `area_bound`, vmangos's `IsLimitedToAnotherMapOrZone`) goes when its
//!   owner is alive outside it; the bank only counts at login, and the
//!   dead keep what they carry until they are alive again;
//! - an item used on a creature (`item_required_target`) is used only on
//!   one of those, alive or dead as it asks;
//! - a timed item's time left is told the client
//!   (`SMSG_ITEM_TIME_UPDATE`) when it comes.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::items::Body;
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::death::Death;
use crate::game::map::inventory::{
    BAG_END, BAG_START, BANK_BAG_END, BANK_BAG_START, BANK_END, BANK_START,
};
use crate::game::map::spells::checks::fail;
use crate::game::store::BAG_PLAYER;

/// How often a player's items are looked over.
const CHECK_EVERY: Duration = Duration::from_secs(1);

/// `CMSG_USE_ITEM`: an item for some creatures alone, at another target.
pub fn handle(map: &mut Map, g: Guid, msg: ClientOpcodeMessage) -> Option<ClientOpcodeMessage> {
    if let ClientOpcodeMessage::CMSG_USE_ITEM(m) = &msg
        && let Some(spell) = wrong_target(map, g, m.bag_index, m.bag_slot, &m.targets)
    {
        if let Some(p) = map.players.get(&g) {
            let mut b = Body::default();
            b.u32(spell).u8(2).u8(fail::BAD_TARGETS);
            p.outbound
                .send_frame(b.frame(crate::game::map::spells::SMSG_CAST_RESULT));
        }
        return None;
    }
    Some(msg)
}

/// The item at `(bag, slot)` asks for creatures its target isn't one of:
/// its on-use spell, to refuse.
fn wrong_target(map: &Map, g: Guid, bag: u8, slot: u8, t: &SpellCastTargets) -> Option<u32> {
    let data = &map.realm.data;
    let p = map.players.get(&g)?;
    let entry = p.items[p.item_at(bag, slot)?].entry;
    let wanted = data
        .crafts
        .item_targets
        .get(&entry)
        .filter(|w| !w.is_empty())?;
    let spell = data
        .item(entry)?
        .spells
        .iter()
        .find(|s| s.id != 0 && s.trigger == 0)?
        .id;
    let target = crate::game::map::spells::unit_of(t).and_then(|u| map.creature(u));
    let fits = target.is_some_and(|c| wanted.contains(&(c.entry, c.alive())));
    (!fits).then_some(spell)
}

/// Whether an item bound to `map_bound` and `area_bound` may be carried in
/// `map` and `zone` (`None`: not known, so not held against it).
pub fn allowed(map_bound: u32, area_bound: u32, map: u32, zone: Option<u32>) -> bool {
    (map_bound == 0 || map_bound == map)
        && (area_bound == 0 || zone.is_none_or(|z| z == area_bound))
}

fn in_bank(bag: u8, slot: u8) -> bool {
    (BANK_BAG_START..BANK_BAG_END).contains(&bag)
        || (bag == BAG_PLAYER
            && ((BANK_START..BANK_END).contains(&slot)
                || (BANK_BAG_START..BANK_BAG_END).contains(&slot)))
}

/// The zone a player is in, as the server knows it (the navmesh or
/// terrain, else the area last worked out from them).
fn zone_of(map: &mut Map, g: Guid) -> Option<u32> {
    let p = map.players.get(&g)?;
    let (pos, area) = (p.position(), p.area);
    if let Some((zone, _)) =
        crate::game::map::world::zone_and_area(map, pos).filter(|(z, _)| *z != 0)
    {
        return Some(zone);
    }
    let data = &map.realm.data;
    data.area(area)
        .filter(|a| a.map == map.key.map)
        .map(|_| data.zone_of(area))
}

/// At login, before the client hears of a thing: an item bound to another
/// map or zone, the bank's too, is gone from a living character, with
/// what is in it, never sent to be destroyed (mangos's `_LoadInventory`,
/// `IsAlive() && IsLimitedToAnotherMapOrZone`: deleted as it loads;
/// Thistle Tea's "login commits removal of a banked item before inventory
/// is sent"). The zone is the navmesh's or terrain's where they stand,
/// else the one they were saved in, if it is of this map.
pub fn at_login(map: &mut Map, p: &mut crate::game::map::player::Player) {
    let data = map.realm.data.clone();
    if p.row.state.dead {
        return;
    }
    let bound = |entry: u32| {
        data.item(entry)
            .map_or((0, 0), |t| (t.map_bound, t.area_bound))
    };
    if !p.items.iter().any(|i| bound(i.entry) != (0, 0)) {
        return;
    }
    let here = map.key.map;
    let zone = crate::game::map::world::zone_and_area(map, p.position())
        .map(|(z, _)| z)
        .filter(|z| *z != 0)
        .or_else(|| {
            data.area(p.zone)
                .filter(|a| a.map == here)
                .map(|_| data.zone_of(p.zone))
        });
    let gone: Vec<Guid> = p
        .items
        .iter()
        .filter(|i| {
            let (m, a) = bound(i.entry);
            !allowed(m, a, here, zone)
        })
        .map(|i| i.guid)
        .collect();
    if gone.is_empty() {
        return;
    }
    let bags: Vec<u8> = p
        .items
        .iter()
        .filter(|i| gone.contains(&i.guid) && i.bag == BAG_PLAYER)
        .filter(|i| {
            (BAG_START..BAG_END).contains(&i.slot)
                || (BANK_BAG_START..BANK_BAG_END).contains(&i.slot)
        })
        .map(|i| i.slot)
        .collect();
    let worn = p.items.iter().any(|i| {
        gone.contains(&i.guid)
            && i.bag == BAG_PLAYER
            && i.slot < crate::game::map::inventory::EQUIPMENT_END
    });
    p.items
        .retain(|i| !gone.contains(&i.guid) && !bags.contains(&i.bag));
    p.refresh_inventory(&data);
    if worn {
        p.apply_stats(&data);
    }
    // Saved soon, as mangos deletes them at once.
    p.save_at = map.now;
}

/// Every player's items, looked over each second.
pub fn update(map: &mut Map) {
    let now = map.now;
    let due: Vec<Guid> = map
        .players
        .values()
        .filter(|p| {
            p.misc
                .items_checked_at
                .is_none_or(|t| now.saturating_duration_since(t) >= CHECK_EVERY)
        })
        .map(|p| p.guid)
        .collect();
    for g in due {
        if let Some(p) = map.players.get_mut(&g) {
            p.misc.items_checked_at = Some(now);
        }
        tell_times(map, g);
        limit_location(map, g);
    }
}

/// Timed items not yet told of: their time left.
fn tell_times(map: &mut Map, g: Guid) {
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let timed: Vec<(Guid, u32)> = p
        .items
        .iter()
        .map(|i| (i.guid, i.fields.get_u32(ITEM_FIELD_DURATION)))
        .filter(|(_, left)| *left > 0)
        .collect();
    p.misc
        .timed_told
        .retain(|g| timed.iter().any(|(t, _)| t == g));
    for (item, duration) in timed {
        if !p.misc.timed_told.contains(&item) {
            p.misc.timed_told.push(item);
            p.outbound.send(&SMSG_ITEM_TIME_UPDATE {
                guid: item,
                duration,
            });
        }
    }
}

/// Items bound elsewhere go: with what is in them, a restricted bag's
/// contents first.
pub fn limit_location(map: &mut Map, g: Guid) {
    let data = map.realm.data.clone();
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    if p.death != Death::Alive {
        return;
    }
    let login = !std::mem::replace(&mut p.misc.items_login_checked, true);
    let bound = |entry: u32| {
        data.item(entry)
            .map_or((0, 0), |t| (t.map_bound, t.area_bound))
    };
    let restricted = p.items.iter().any(|i| bound(i.entry) != (0, 0));
    if !restricted {
        return;
    }
    let zone = zone_of(map, g);
    let here = map.key.map;
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let mut gone: Vec<Guid> = p
        .items
        .iter()
        .filter(|i| login || !in_bank(i.bag, i.slot))
        .filter(|i| {
            let (m, a) = bound(i.entry);
            !allowed(m, a, here, zone)
        })
        .map(|i| i.guid)
        .collect();
    if gone.is_empty() {
        return;
    }
    // A bag's contents go with it, before it.
    let bags: Vec<u8> = p
        .items
        .iter()
        .filter(|i| gone.contains(&i.guid) && i.bag == BAG_PLAYER)
        .filter(|i| {
            (BAG_START..BAG_END).contains(&i.slot)
                || (BANK_BAG_START..BANK_BAG_END).contains(&i.slot)
        })
        .map(|i| i.slot)
        .collect();
    let contents: Vec<Guid> = p
        .items
        .iter()
        .filter(|i| bags.contains(&i.bag) && !gone.contains(&i.guid))
        .map(|i| i.guid)
        .collect();
    gone.splice(0..0, contents);
    let worn = p.items.iter().any(|i| {
        gone.contains(&i.guid)
            && i.bag == BAG_PLAYER
            && i.slot < crate::game::map::inventory::EQUIPMENT_END
    });
    let casting = p
        .caster
        .casting
        .as_ref()
        .and_then(|c| c.item)
        .is_some_and(|i| gone.contains(&i));
    if casting {
        crate::game::map::spells::cancel(map, g);
    }
    for item in gone {
        let Some((i, count)) = map
            .players
            .get(&g)
            .and_then(|p| p.item_by_guid(item).map(|i| (i, p.items[i].count())))
        else {
            continue;
        };
        crate::game::map::inventory::take_item(map, g, i, count);
        if let Some(p) = map.players.get_mut(&g)
            && p.misc.item_loot.as_ref().is_some_and(|(l, _)| *l == item)
        {
            p.misc.item_loot = None;
            // Its window closes with it (mangos's `Loot::~Loot` releases
            // every looter).
            if p.looting == Some(item) {
                p.looting = None;
                p.outbound
                    .send(&wow_world_messages::vanilla::SMSG_LOOT_RELEASE_RESPONSE {
                        guid: item,
                        unknown1: 1,
                    });
            }
        }
    }
    if worn && let Some(p) = map.players.get_mut(&g) {
        p.apply_stats(&data);
        p.gear_dirty = true;
    }
}
