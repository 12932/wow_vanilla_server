//! What items do beyond sitting in bags: binding (on pickup, equip or use),
//! on-use charges and cooldowns, conjured items that fade while their owner
//! is away, timed items that run out, and where an item may go: the
//! keyring for keys, a special bag (quiver, ammo pouch, soul, herb or
//! enchanting bag) only for what it is made for. As Thistle Tea's
//! `Core.Item.*` and `Core.Inventory` keep them.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::{SMSG_DESTROY_OBJECT, SMSG_ITEM_COOLDOWN};

use crate::game::data::{GameData, ItemTemplate};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::player::{Item, Player};
use crate::game::store::BAG_PLAYER;

/// `ITEM_FIELD_FLAGS`: bound to its owner.
pub const ITEM_FLAG_SOULBOUND: u32 = 0x1;
/// The template's flags: conjured by a spell.
pub const TEMPLATE_FLAG_CONJURED: u32 = 0x2;

/// `bonding`.
pub const BIND_ON_PICKUP: u32 = 1;
pub const BIND_ON_EQUIP: u32 = 2;
pub const BIND_ON_USE: u32 = 3;
pub const BIND_QUEST: u32 = 4;
pub const BIND_QUEST_2: u32 = 5;

/// Bag families: keys go on the keyring.
pub const BAG_FAMILY_KEYS: u32 = 9;
pub const KEYRING_START: u8 = 81;
pub const KEYRING_MAX: u8 = 32;

/// How long conjured items last while their owner is away.
pub const CONJURED_OFFLINE: i64 = 15 * 60;

/// `SpellCastResult`s.
const FAIL_NO_CHARGES_REMAIN: u8 = 0x44;
const FAIL_NOT_READY: u8 = 0x3c;

/// How often timed items run down.
const TICK: Duration = Duration::from_secs(1);

pub fn bound(item: &Item) -> bool {
    item.fields.get_u32(ITEM_FIELD_FLAGS) & ITEM_FLAG_SOULBOUND != 0
}

fn bind(item: &mut Item) {
    let flags = item.fields.get_u32(ITEM_FIELD_FLAGS);
    item.fields
        .set_u32(ITEM_FIELD_FLAGS, flags | ITEM_FLAG_SOULBOUND);
}

/// A new item in a player's hands: one bound on pickup is theirs.
pub fn on_create(data: &GameData, item: &mut Item) {
    if data
        .item(item.entry)
        .is_some_and(|t| matches!(t.bonding, BIND_ON_PICKUP | BIND_QUEST | BIND_QUEST_2))
    {
        bind(item);
    }
}

/// An item put on: one bound on equip is theirs now.
pub fn on_equip(data: &GameData, item: &mut Item) {
    if data
        .item(item.entry)
        .is_some_and(|t| t.bonding == BIND_ON_EQUIP)
    {
        bind(item);
    }
}

/// A player's honor rank, as `PLAYER_BYTES_3` carries it for the client
/// (the PvP side keeps it there).
pub fn honor_rank(p: &Player) -> u32 {
    u32::from(p.fields.get_byte(PLAYER_BYTES_3, 3))
}

/// The keyring's slots at `level`, as the 1.12 client draws them.
pub fn keyring_size(level: u8) -> u8 {
    match level {
        61.. => 16,
        50.. => 12,
        40.. => 8,
        _ => 4,
    }
}

pub fn is_key(t: &ItemTemplate) -> bool {
    t.bag_family == BAG_FAMILY_KEYS
}

/// A bag's family: what alone it holds (0 for anything).
pub fn bag_family(data: &GameData, p: &Player, bag: u8) -> u32 {
    p.item_at(BAG_PLAYER, bag)
        .and_then(|i| data.item(p.items[i].entry))
        .filter(|t| t.class == 1 || t.class == 11)
        .map_or(0, |t| t.bag_family)
}

/// Whether an item of template `t` (`None`: unknown) may go into the bag in
/// player slot `bag`.
pub fn fits_bag(data: &GameData, p: &Player, bag: u8, t: Option<&ItemTemplate>) -> bool {
    match bag_family(data, p, bag) {
        0 => true,
        family => t.is_some_and(|t| t.bag_family == family && t.class != 1),
    }
}

/// Whether `slot` of the player's own is a keyring slot they have.
pub fn keyring_slot(p: &Player, slot: u8) -> bool {
    slot >= KEYRING_START && slot < KEYRING_START + keyring_size(p.level())
}

/// The slots an item of `t` may be carried in, in the order it is put
/// away: the keyring for a key, a special bag made for it, the backpack,
/// then plain bags.
pub fn carry_slots(data: &GameData, p: &Player, t: Option<&ItemTemplate>) -> Vec<(u8, u8)> {
    use crate::game::map::inventory::{BAG_END, BAG_START, PACK_END, PACK_START};
    let mut out = Vec::new();
    if t.is_some_and(is_key) {
        out.extend(
            (KEYRING_START..KEYRING_START + keyring_size(p.level())).map(|s| (BAG_PLAYER, s)),
        );
    }
    let special: Vec<u8> = (BAG_START..BAG_END)
        .filter(|b| bag_family(data, p, *b) != 0)
        .collect();
    for bag in &special {
        if fits_bag(data, p, *bag, t) {
            out.extend((0..p.bag_size(data, *bag)).map(|s| (*bag, s)));
        }
    }
    out.extend((PACK_START..PACK_END).map(|s| (BAG_PLAYER, s)));
    for bag in (BAG_START..BAG_END).filter(|b| !special.contains(b)) {
        out.extend((0..p.bag_size(data, bag)).map(|s| (bag, s)));
    }
    out
}

/// Why a player can't use their item's on-use spell `index` now: no
/// charges left, or its cooldown.
pub fn check_use(map: &Map, player: Guid, item: Guid, index: usize) -> Option<u8> {
    let data = &map.realm.data;
    let p = map.players.get(&player)?;
    let it = p.items.iter().find(|i| i.guid == item)?;
    let t = data.item(it.entry)?;
    let s = t.spells.get(index)?;
    if s.charges != 0 && it.fields.get_u32(ITEM_FIELD_SPELL_CHARGES + index as u16) as i32 == 0 {
        return Some(FAIL_NO_CHARGES_REMAIN);
    }
    let now = map.now;
    let c = &p.caster;
    if s.category > 0
        && c.category_cooldowns
            .get(&s.category)
            .is_some_and(|t| *t > now)
    {
        return Some(FAIL_NOT_READY);
    }
    if s.cooldown > 0 && c.cooldowns.get(&s.id).is_some_and(|t| *t > now) {
        return Some(FAIL_NOT_READY);
    }
    None
}

/// `check_use` refused: the cast result says why.
pub fn send_failure(map: &Map, player: Guid, spell: u32, code: u8) {
    if let Some(p) = map.players.get(&player) {
        let mut b = crate::game::data::items::Body::default();
        b.u32(spell).u8(2).u8(code);
        p.outbound
            .send_frame(b.frame(crate::game::map::spells::SMSG_CAST_RESULT));
    }
}

/// How long an on-use spell waits after its item is put on.
pub const EQUIP_COOLDOWN: Duration = Duration::from_secs(30);

/// Item `item` was put on (Thistle Tea's `EquipmentTransitions`): each of
/// its on-use spells waits thirty seconds, unless it already waits.
pub fn equipped(map: &mut Map, player: Guid, item: Guid) {
    let data = map.realm.data.clone();
    let now = map.now;
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    let Some(t) = p
        .items
        .iter()
        .find(|i| i.guid == item)
        .and_then(|i| data.item(i.entry))
    else {
        return;
    };
    if t.flags & 0x80 != 0 {
        return;
    }
    for s in t.spells.iter().filter(|s| s.id != 0 && s.trigger == 0) {
        let waiting = p.caster.cooldowns.get(&s.id).is_some_and(|t| *t > now)
            || (s.category > 0
                && p.caster
                    .category_cooldowns
                    .get(&s.category)
                    .is_some_and(|t| *t > now));
        if !waiting {
            p.caster.cooldowns.insert(s.id, now + EQUIP_COOLDOWN);
        }
        p.outbound.send(&SMSG_ITEM_COOLDOWN {
            guid: item,
            id: s.id,
        });
    }
}

/// A spell cast from an item went off: its cooldowns start, a charge goes
/// (an item whose charges are spent and that is used up goes with it), and
/// an item bound on use is bound.
pub fn used(map: &mut Map, player: Guid, item: Guid, spell: u32) {
    let data = map.realm.data.clone();
    let now = map.now;
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    let Some(i) = p.items.iter().position(|x| x.guid == item) else {
        return;
    };
    let Some(t) = data.item(p.items[i].entry).cloned() else {
        return;
    };
    let Some(index) = t.spells.iter().position(|s| s.id == spell) else {
        return;
    };
    let s = t.spells[index];
    if s.cooldown > 0 {
        p.caster
            .cooldowns
            .insert(spell, now + Duration::from_millis(s.cooldown.max(0) as u64));
    }
    if s.category > 0 && s.category_cooldown > 0 {
        p.caster.category_cooldowns.insert(
            s.category,
            now + Duration::from_millis(s.category_cooldown.max(0) as u64),
        );
    }
    if s.cooldown > 0 || s.category_cooldown > 0 {
        p.outbound.send(&SMSG_ITEM_COOLDOWN {
            guid: item,
            id: spell,
        });
    }
    if t.bonding == BIND_ON_USE {
        bind(&mut p.items[i]);
    }
    // Charges: each charged spell spends one; with none left, an item
    // whose charges count down from below zero is used up.
    let mut expendable = false;
    let mut exhausted = false;
    for (k, ks) in t.spells.iter().enumerate() {
        if ks.charges == 0 || ks.id == 0 {
            continue;
        }
        let field = ITEM_FIELD_SPELL_CHARGES + k as u16;
        let have = p.items[i].fields.get_u32(field) as i32;
        let left = have - have.signum();
        if t.stackable < 2 {
            p.items[i].fields.set_i32(field, left);
        }
        expendable |= ks.charges < 0;
        exhausted |= left == 0;
    }
    // An item its spell changes into another is replaced, not used up as
    // well: mangos's EffectSummonChangeItem lets go of the cast item before
    // TakeCastItem, and Thistle Tea keeps it until the change.
    let changes = data.spells.get(spell).is_some_and(|s| {
        s.effects.iter().any(|e| {
            e.effect == crate::game::map::extra_effects::EFFECT_SUMMON_CHANGE_ITEM
                && e.misc_value > 0
        })
    });
    if expendable && exhausted && !changes {
        crate::game::map::inventory::take_item(map, player, i, 1);
    }
}

/// A player coming into the world: conjured items left alone too long are
/// gone.
pub fn on_login(p: &mut Player, data: &GameData) {
    let now = crate::game::social::mail::now();
    let away = now - p.row.logout_time;
    if p.row.logout_time <= 0 || away < CONJURED_OFFLINE {
        return;
    }
    let before = p.items.len();
    p.items.retain(|i| {
        data.item(i.entry)
            .is_none_or(|t| t.flags & TEMPLATE_FLAG_CONJURED == 0)
    });
    if p.items.len() != before {
        p.refresh_inventory(data);
    }
}

/// Timed items run down while their owner is in the world, and go when
/// their time is up.
pub fn update(map: &mut Map) {
    let now = map.now;
    let last = *map.loot_state.items_tick_at.get_or_insert(now);
    let elapsed = now.duration_since(last);
    if elapsed < TICK {
        return;
    }
    map.loot_state.items_tick_at = Some(now);
    let secs = elapsed.as_secs() as u32;
    let mut gone: Vec<(Guid, Guid)> = Vec::new();
    for p in map.players.values_mut() {
        for item in p.items.iter_mut() {
            let left = item.fields.get_u32(ITEM_FIELD_DURATION);
            if left == 0 {
                continue;
            }
            let left = left.saturating_sub(secs);
            item.fields.set_u32(ITEM_FIELD_DURATION, left);
            if left == 0 {
                gone.push((p.guid, item.guid));
            }
        }
    }
    for (player, item) in gone {
        destroy(map, player, item);
    }
}

/// An item ceases to be (its time up): a bag's contents first (Thistle
/// Tea's `ItemLifetime.plan`, mangos's bag destruction); a cast from any of
/// them stops, a loot window on one closes, and worn, the stats are
/// worked out again.
pub fn destroy(map: &mut Map, player: Guid, item: Guid) {
    let data = map.realm.data.clone();
    let Some(p) = map.players.get(&player) else {
        return;
    };
    let Some(i) = p.items.iter().position(|x| x.guid == item) else {
        return;
    };
    let it = &p.items[i];
    let bag_slot = it.bag == BAG_PLAYER
        && ((crate::game::map::inventory::BAG_START..crate::game::map::inventory::BAG_END)
            .contains(&it.slot)
            || (crate::game::map::inventory::BANK_BAG_START
                ..crate::game::map::inventory::BANK_BAG_END)
                .contains(&it.slot));
    let mut gone: Vec<Guid> = if bag_slot {
        p.items
            .iter()
            .filter(|x| x.bag == it.slot)
            .map(|x| x.guid)
            .collect()
    } else {
        Vec::new()
    };
    gone.push(item);
    let casting = p
        .caster
        .casting
        .as_ref()
        .and_then(|c| c.item)
        .is_some_and(|i| gone.contains(&i));
    if casting {
        crate::game::map::spells::cancel(map, player);
    }
    for g in gone {
        let Some(p) = map.players.get_mut(&player) else {
            return;
        };
        let Some(i) = p.items.iter().position(|x| x.guid == g) else {
            continue;
        };
        let it = p.items.remove(i);
        p.outbound.send(&SMSG_DESTROY_OBJECT { guid: it.guid });
        if p.misc.item_loot.as_ref().is_some_and(|(l, _)| *l == g) {
            p.misc.item_loot = None;
            if p.looting == Some(g) {
                p.looting = None;
                p.outbound
                    .send(&wow_world_messages::vanilla::SMSG_LOOT_RELEASE_RESPONSE {
                        guid: g,
                        unknown1: 1,
                    });
            }
        }
        p.refresh_inventory(&data);
        crate::game::map::quests::items_changed(map, player, it.entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keyring_grows_with_level() {
        assert_eq!(keyring_size(1), 4);
        assert_eq!(keyring_size(40), 8);
        assert_eq!(keyring_size(55), 12);
        assert_eq!(keyring_size(60), 12);
    }
}
