//! A player's items: equipment, bags, backpack, bank; moving, stacking,
//! splitting and destroying them; what can be equipped; and the stats gear
//! gives.
//!
//! An item is at `(bag, slot)`: bag [`BAG_PLAYER`] for the player's own
//! slots (equipment 0-18, bags 19-22, backpack 23-38, bank 39-62, bank
//! bags 63-68, buyback 69-80, keyring 81-112), or the player slot of the
//! bag it is in.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::{
    Guid as G, NewItemChatAlert, NewItemCreationType, NewItemSource, SMSG_DESTROY_OBJECT,
    SMSG_INVENTORY_CHANGE_FAILURE, SMSG_INVENTORY_CHANGE_FAILURE_InventoryResult as R,
    SMSG_ITEM_PUSH_RESULT,
};

use crate::game::data::{GameData, ItemTemplate};
use crate::game::fields::Visibility;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::player::{Item, Player};
use crate::game::store::BAG_PLAYER;
use crate::game::update;

pub const EQUIPMENT_END: u8 = 19;
pub const BAG_START: u8 = 19;
pub const BAG_END: u8 = 23;
pub const PACK_START: u8 = 23;
pub const PACK_END: u8 = 39;
pub const BANK_START: u8 = 39;
pub const BANK_END: u8 = 63;
pub const BANK_BAG_START: u8 = 63;
pub const BANK_BAG_END: u8 = 69;
pub const BUYBACK_START: u8 = 69;
pub const BUYBACK_END: u8 = 81;

/// Slots by the equipment each holds.
pub const SLOT_MAIN_HAND: u8 = 15;
pub const SLOT_OFF_HAND: u8 = 16;

/// Why an inventory change failed, as the client is told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fail {
    Level(u8),
    DoesntGoToSlot,
    BagFull,
    NoProficiency,
    NeverUse,
    TwoHanded,
    CantDualWield,
    DoesntGoIntoBag,
    CantCarryMore,
    CantStack,
    SlotEmpty,
    NotFound,
    SplitTooMany,
    NotEnoughMoney,
    OnlyEmptyBags,
    InventoryFull,
    YouAreDead,
}

pub fn send_fail(p: &Player, fail: Fail, item: Guid) {
    let (a, b, s) = (item, G::zero(), 0u8);
    let result = match fail {
        Fail::Level(level) => R::CantEquipLevelI {
            bag_type_subclass: s,
            item1: a,
            item2: b,
            required_level: wow_world_messages::vanilla::Level::new(level),
        },
        Fail::DoesntGoToSlot => R::ItemDoesntGoToSlot {
            bag_type_subclass: s,
            item1: a,
            item2: b,
        },
        Fail::BagFull => R::BagFull {
            bag_type_subclass: s,
            item1: a,
            item2: b,
        },
        Fail::NoProficiency => R::NoRequiredProficiency {
            bag_type_subclass: s,
            item1: a,
            item2: b,
        },
        Fail::NeverUse => R::YouCanNeverUseThatItem {
            bag_type_subclass: s,
            item1: a,
            item2: b,
        },
        Fail::TwoHanded => R::CantEquipWithTwohanded {
            bag_type_subclass: s,
            item1: a,
            item2: b,
        },
        Fail::CantDualWield => R::CantDualWield {
            bag_type_subclass: s,
            item1: a,
            item2: b,
        },
        Fail::DoesntGoIntoBag => R::ItemDoesntGoIntoBag {
            bag_type_subclass: s,
            item1: a,
            item2: b,
        },
        Fail::CantCarryMore => R::CantCarryMoreOfThis {
            bag_type_subclass: s,
            item1: a,
            item2: b,
        },
        Fail::CantStack => R::ItemCantStack {
            bag_type_subclass: s,
            item1: a,
            item2: b,
        },
        Fail::SlotEmpty => R::SlotIsEmpty {
            bag_type_subclass: s,
            item1: a,
            item2: b,
        },
        Fail::NotFound => R::ItemNotFound {
            bag_type_subclass: s,
            item1: a,
            item2: b,
        },
        Fail::SplitTooMany => R::TriedToSplitMoreThanCount {
            bag_type_subclass: s,
            item1: a,
            item2: b,
        },
        Fail::NotEnoughMoney => R::NotEnoughMoney {
            bag_type_subclass: s,
            item1: a,
            item2: b,
        },
        Fail::OnlyEmptyBags => R::CanOnlyDoWithEmptyBags {
            bag_type_subclass: s,
            item1: a,
            item2: b,
        },
        Fail::InventoryFull => R::InventoryFull {
            bag_type_subclass: s,
            item1: a,
            item2: b,
        },
        Fail::YouAreDead => R::YouAreDead {
            bag_type_subclass: s,
            item1: a,
            item2: b,
        },
    };
    p.outbound.send(&SMSG_INVENTORY_CHANGE_FAILURE { result });
}

/// The equipment slots an inventory type may go in.
pub fn slots_for(inventory_type: u32) -> &'static [u8] {
    match inventory_type {
        1 => &[0],
        2 => &[1],
        3 => &[2],
        4 => &[3],
        5 | 20 => &[4],
        6 => &[5],
        7 => &[6],
        8 => &[7],
        9 => &[8],
        10 => &[9],
        11 => &[10, 11],
        12 => &[12, 13],
        13 => &[15, 16],
        14 | 22 | 23 => &[16],
        15 | 25 | 26 | 28 => &[17],
        16 => &[14],
        17 | 21 => &[15],
        18 => &[19, 20, 21, 22],
        19 => &[18],
        _ => &[],
    }
}

impl Player {
    pub fn item_at(&self, bag: u8, slot: u8) -> Option<usize> {
        self.items
            .iter()
            .position(|i| i.bag == bag && i.slot == slot)
    }

    pub fn item_by_guid(&self, g: Guid) -> Option<usize> {
        self.items.iter().position(|i| i.guid == g)
    }

    /// How many slots the bag in player slot `bag` has; 0 for none.
    pub fn bag_size(&self, data: &GameData, bag: u8) -> u8 {
        self.item_at(BAG_PLAYER, bag)
            .and_then(|i| data.item(self.items[i].entry))
            .map(|t| t.container_slots.min(36) as u8)
            .unwrap_or(0)
    }

    /// Every slot an item can be carried in, backpack first: `(bag, slot)`.
    pub fn carry_slots(&self, data: &GameData) -> Vec<(u8, u8)> {
        let mut out: Vec<(u8, u8)> = (PACK_START..PACK_END).map(|s| (BAG_PLAYER, s)).collect();
        for bag in BAG_START..BAG_END {
            for s in 0..self.bag_size(data, bag) {
                out.push((bag, s));
            }
        }
        out
    }

    pub fn free_carry_slot(&self, data: &GameData) -> Option<(u8, u8)> {
        self.carry_slots(data)
            .into_iter()
            .find(|(b, s)| self.item_at(*b, *s).is_none())
    }

    pub fn count_of(&self, entry: u32) -> u32 {
        self.items
            .iter()
            .filter(|i| i.entry == entry)
            .filter(|i| !(i.bag == BAG_PLAYER && (BUYBACK_START..BUYBACK_END).contains(&i.slot)))
            .map(|i| i.count())
            .sum()
    }

    pub fn money(&self) -> u32 {
        self.fields.get_u32(PLAYER_FIELD_COINAGE)
    }

    pub fn set_money(&mut self, money: u32) {
        self.fields.set_u32(PLAYER_FIELD_COINAGE, money);
    }

    /// Bring the slot fields, the bags' slots and the visible gear in line
    /// with the items, and the stats with the gear.
    pub fn refresh_inventory(&mut self, data: &GameData) {
        self.equip_fields();
        // Bags: each slot holds its item's guid.
        let bags: Vec<(u8, Guid)> = self
            .items
            .iter()
            .filter(|i| {
                i.bag == BAG_PLAYER
                    && ((BAG_START..BAG_END).contains(&i.slot)
                        || (BANK_BAG_START..BANK_BAG_END).contains(&i.slot))
            })
            .map(|i| (i.slot, i.guid))
            .collect();
        for (slot, bag_guid) in bags {
            let contents: Vec<(u8, Guid)> = self
                .items
                .iter()
                .filter(|i| i.bag == slot)
                .map(|i| (i.slot, i.guid))
                .collect();
            if let Some(bi) = self.item_by_guid(bag_guid) {
                let f = &mut self.items[bi].fields;
                if f.type_id() == TypeId::Container {
                    for s in 0..36u16 {
                        f.set_guid(CONTAINER_FIELD_SLOT_1 + s * 2, Guid::zero());
                    }
                    for (s, g) in contents {
                        f.set_guid(CONTAINER_FIELD_SLOT_1 + u16::from(s) * 2, g);
                    }
                }
            }
            for item in self.items.iter_mut().filter(|i| i.bag == slot) {
                item.fields.set_guid(ITEM_FIELD_CONTAINED, bag_guid);
            }
        }
        let own = self.guid;
        for item in self.items.iter_mut().filter(|i| i.bag == BAG_PLAYER) {
            item.fields.set_guid(ITEM_FIELD_CONTAINED, own);
        }
        self.apply_stats(data);
        self.gear_dirty = true;
    }
}

impl Item {
    pub fn count(&self) -> u32 {
        self.fields.get_u32(ITEM_FIELD_STACK_COUNT)
    }

    pub fn set_count(&mut self, n: u32) {
        self.fields.set_u32(ITEM_FIELD_STACK_COUNT, n);
    }
}

pub fn proficient(class: u8, level: u8, t: &ItemTemplate) -> bool {
    const WEAPON: u32 = 2;
    const ARMOR: u32 = 4;
    match t.class {
        WEAPON => {
            // Weapon subclasses: axe 0, 2h axe 1, bow 2, gun 3, mace 4,
            // 2h mace 5, polearm 6, sword 7, 2h sword 8, staff 10, fist 13,
            // misc 14, dagger 15, thrown 16, spear 17, crossbow 18,
            // wand 19, fishing pole 20.
            let allowed: &[u32] = match class {
                1 => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 10, 13, 15, 16, 18],
                2 => &[0, 1, 4, 5, 6, 7, 8],
                3 => &[0, 1, 2, 3, 6, 7, 8, 10, 13, 15, 16, 18],
                4 => &[2, 3, 4, 7, 13, 15, 16, 18],
                5 => &[4, 10, 15, 19],
                7 => &[0, 1, 4, 5, 10, 13, 15],
                8 | 9 => &[7, 10, 15, 19],
                11 => &[4, 5, 6, 10, 13, 15],
                _ => &[],
            };
            allowed.contains(&t.subclass) || matches!(t.subclass, 14 | 20)
        }
        ARMOR => match t.subclass {
            0 => true,                                   // misc: rings, trinkets, held
            1 => true,                                   // cloth
            2 => class != 5 && class != 8 && class != 9, // leather
            3 => matches!(class, 1 | 2) || (matches!(class, 3 | 7) && level >= 40), // mail
            4 => matches!(class, 1 | 2) && level >= 40,  // plate
            6 => matches!(class, 1 | 2 | 7),             // shield
            7..=9 => true,                               // librams, idols, totems
            _ => false,
        },
        _ => true,
    }
}

/// Whether `p` may put an item of template `t` in equipment `slot`.
pub fn can_equip(p: &Player, t: &ItemTemplate, slot: u8) -> Result<(), Fail> {
    if !slots_for(t.inventory_type).contains(&slot) {
        return Err(Fail::DoesntGoToSlot);
    }
    if t.allowable_class != -1
        && t.allowable_class != 0
        && t.allowable_class & (1 << (p.row.class - 1)) == 0
    {
        return Err(Fail::NeverUse);
    }
    if t.allowable_race != -1
        && t.allowable_race != 0
        && t.allowable_race & (1 << (p.row.race - 1)) == 0
    {
        return Err(Fail::NeverUse);
    }
    if u32::from(p.level()) < t.required_level {
        return Err(Fail::Level(t.required_level as u8));
    }
    if !proficient(p.row.class, p.level(), t) {
        return Err(Fail::NoProficiency);
    }
    if slot == SLOT_OFF_HAND && t.inventory_type == 13 {
        let dual = match p.row.class {
            4 => true,
            1 | 3 => p.level() >= 20,
            _ => false,
        };
        if !dual {
            return Err(Fail::CantDualWield);
        }
    }
    Ok(())
}

/// [`add_item`], with a random property (a suffix) on a new item.
pub fn add_item_with_property(
    map: &mut Map,
    g: Guid,
    entry: u32,
    count: u32,
    alert: bool,
    property: u32,
) -> Result<(), Fail> {
    let before = map.players.get(&g).map_or(0, |p| p.items.len());
    add_item(map, g, entry, count, alert)?;
    if property != 0 {
        let data = map.realm.data.clone();
        if let Some(p) = map.players.get_mut(&g)
            && let Some(item) = p.items.get_mut(before..).and_then(|n| n.iter_mut().find(|i| i.entry == entry))
        {
            crate::game::map::enchant::set_random_property(&data, item, property);
        }
    }
    Ok(())
}

/// Give `p` `count` of item `entry`: onto stacks first, then new items in
/// free slots. Nothing changes unless all of it fits.
pub fn add_item(map: &mut Map, g: Guid, entry: u32, count: u32, alert: bool) -> Result<(), Fail> {
    let data = map.realm.data.clone();
    let Some(t) = data.item(entry).cloned() else {
        return Err(Fail::NotFound);
    };
    let realm = map.realm.clone();
    let Some(p) = map.players.get_mut(&g) else {
        return Err(Fail::NotFound);
    };
    let stack = t.stackable.max(1);
    if t.max_count > 0 && p.count_of(entry) + count > t.max_count {
        return Err(Fail::CantCarryMore);
    }
    // Plan: room on stacks, then free slots.
    let mut left = count;
    let mut onto: Vec<(usize, u32)> = Vec::new();
    for (i, item) in p.items.iter().enumerate() {
        if left == 0 {
            break;
        }
        if item.entry == entry
            && item.slot < BUYBACK_START
            && item.count() < stack
            && !(item.bag == BAG_PLAYER && item.slot < EQUIPMENT_END)
        {
            let add = (stack - item.count()).min(left);
            onto.push((i, add));
            left -= add;
        }
    }
    let mut new_slots = Vec::new();
    if left > 0 {
        for (bag, slot) in p.carry_slots(&data) {
            if left == 0 {
                break;
            }
            if p.item_at(bag, slot).is_some() {
                continue;
            }
            let n = left.min(stack);
            new_slots.push((bag, slot, n));
            left -= n;
        }
    }
    if left > 0 {
        return Err(Fail::InventoryFull);
    }
    let mut last = None;
    for (i, add) in onto {
        let item = &mut p.items[i];
        item.set_count(item.count() + add);
        last = Some((item.bag, item.slot));
    }
    let mut batch = update::Batch::new();
    for (bag, slot, n) in new_slots {
        let counter = realm
            .next_item
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let item = Item::new(&data, guid::item(counter), g, entry, bag, slot, n);
        batch.push(&update::create_block(&item.create(), Visibility::Owner));
        p.items.push(item);
        last = Some((bag, slot));
    }
    for frame in batch.finish() {
        p.outbound.send_frame(frame);
    }
    // The new items are known; their slots go out with the next update.
    for item in p.items.iter_mut() {
        if item.entry == entry {
            item.fields.clear_dirty();
        }
    }
    p.refresh_inventory(&data);
    let notify = alert;
    if notify && let Some((bag, slot)) = last {
        p.outbound.send(&SMSG_ITEM_PUSH_RESULT {
            guid: g,
            source: NewItemSource::Looted,
            creation_type: NewItemCreationType::Received,
            alert_chat: NewItemChatAlert::Show,
            bag_slot: bag,
            item_slot: u32::from(slot),
            item: entry,
            item_suffix_factor: 0,
            item_random_property_id: 0,
            item_count: count,
        });
    }
    crate::game::map::quests::items_changed(map, g, entry);
    Ok(())
}

/// Take `count` of the item at index `i` away; the whole item if it is all
/// of it.
pub fn take_item(map: &mut Map, g: Guid, i: usize, count: u32) {
    let data = map.realm.data.clone();
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let Some(item) = p.items.get_mut(i) else {
        return;
    };
    if count < item.count() {
        item.set_count(item.count() - count);
        return;
    }
    let item = p.items.remove(i);
    p.outbound.send(&SMSG_DESTROY_OBJECT { guid: item.guid });
    p.refresh_inventory(&data);
    crate::game::map::quests::items_changed(map, g, item.entry);
}

/// Whether `(bag, slot)` is a slot of `p`'s, and an item may be put there.
fn slot_exists(p: &Player, data: &GameData, bag: u8, slot: u8) -> bool {
    if bag == BAG_PLAYER {
        return slot < BUYBACK_START;
    }
    ((BAG_START..BAG_END).contains(&bag) || (BANK_BAG_START..BANK_BAG_END).contains(&bag))
        && slot < p.bag_size(data, bag)
}

/// Whether an item may go to `(bag, slot)`: equipment rules for equipment
/// slots, empty bags only into bag slots, no bag into itself.
fn may_place(p: &Player, data: &GameData, item: &Item, bag: u8, slot: u8) -> Result<(), Fail> {
    if !slot_exists(p, data, bag, slot) {
        return Err(Fail::DoesntGoToSlot);
    }
    let t = data.item(item.entry);
    if bag == BAG_PLAYER && slot < EQUIPMENT_END {
        let Some(t) = t else {
            return Err(Fail::DoesntGoToSlot);
        };
        can_equip(p, t, slot)?;
        // A two-hander needs the off hand free, and the reverse.
        if slot == SLOT_OFF_HAND
            && p.item_at(BAG_PLAYER, SLOT_MAIN_HAND)
                .and_then(|i| data.item(p.items[i].entry))
                .is_some_and(|m| m.inventory_type == 17)
        {
            return Err(Fail::TwoHanded);
        }
    }
    if bag == BAG_PLAYER
        && ((BAG_START..BAG_END).contains(&slot) || (BANK_BAG_START..BANK_BAG_END).contains(&slot))
        && t.is_none_or(|t| t.class != 1)
    {
        return Err(Fail::DoesntGoToSlot);
    }
    if bag != BAG_PLAYER
        && t.is_some_and(|t| t.class == 1)
        && p.items
            .iter()
            .any(|i| i.bag == item.slot && item.bag == BAG_PLAYER)
    {
        return Err(Fail::OnlyEmptyBags);
    }
    if item.bag == BAG_PLAYER && item.slot == bag {
        return Err(Fail::DoesntGoIntoBag);
    }
    Ok(())
}

/// Move the item at `from` to `to`: into an empty slot, onto a stack of the
/// same item, or swapping with what is there.
pub fn move_item(map: &mut Map, g: Guid, from: (u8, u8), to: (u8, u8)) {
    let data = map.realm.data.clone();
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    if from == to {
        return;
    }
    let Some(a) = p.item_at(from.0, from.1) else {
        send_fail(p, Fail::SlotEmpty, Guid::zero());
        return;
    };
    let b = p.item_at(to.0, to.1);
    // A bag with things in it doesn't move.
    let is_bag = |p: &Player, i: usize| {
        p.items[i].bag == BAG_PLAYER
            && ((BAG_START..BAG_END).contains(&p.items[i].slot)
                || (BANK_BAG_START..BANK_BAG_END).contains(&p.items[i].slot))
    };
    for i in [Some(a), b].into_iter().flatten() {
        if is_bag(p, i) && p.items.iter().any(|x| x.bag == p.items[i].slot) {
            let g = p.items[i].guid;
            send_fail(p, Fail::OnlyEmptyBags, g);
            return;
        }
    }
    if let Err(f) = may_place(p, &data, &p.items[a].clone(), to.0, to.1) {
        let ag = p.items[a].guid;
        send_fail(p, f, ag);
        return;
    }
    if let Some(b) = b {
        // Onto a stack.
        if p.items[b].entry == p.items[a].entry {
            let stack = data
                .item(p.items[a].entry)
                .map(|t| t.stackable.max(1))
                .unwrap_or(1);
            let room = stack.saturating_sub(p.items[b].count());
            if room > 0 {
                let n = room.min(p.items[a].count());
                let total_b = p.items[b].count() + n;
                p.items[b].set_count(total_b);
                if n == p.items[a].count() {
                    let gone = p.items.remove(a);
                    p.outbound.send(&SMSG_DESTROY_OBJECT { guid: gone.guid });
                } else {
                    let left = p.items[a].count() - n;
                    p.items[a].set_count(left);
                }
                p.refresh_inventory(&data);
                return;
            }
        }
        if let Err(f) = may_place(p, &data, &p.items[b].clone(), from.0, from.1) {
            let bg = p.items[b].guid;
            send_fail(p, f, bg);
            return;
        }
        p.items[b].bag = from.0;
        p.items[b].slot = from.1;
    }
    p.items[a].bag = to.0;
    p.items[a].slot = to.1;
    p.refresh_inventory(&data);
}

/// `CMSG_AUTOEQUIP_ITEM`: equip an item into the slot it goes in, the
/// first free one, else the first.
pub fn auto_equip(map: &mut Map, g: Guid, from: (u8, u8)) {
    let data = map.realm.data.clone();
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let Some(i) = p.item_at(from.0, from.1) else {
        send_fail(p, Fail::SlotEmpty, Guid::zero());
        return;
    };
    let Some(t) = data.item(p.items[i].entry) else {
        send_fail(p, Fail::DoesntGoToSlot, p.items[i].guid);
        return;
    };
    let slots = slots_for(t.inventory_type);
    let Some(&slot) = slots
        .iter()
        .find(|s| p.item_at(BAG_PLAYER, **s).is_none())
        .or(slots.first())
    else {
        send_fail(p, Fail::DoesntGoToSlot, p.items[i].guid);
        return;
    };
    // A two-hander takes the off hand's item off first.
    if t.inventory_type == 17
        && let Some(off) = p.item_at(BAG_PLAYER, SLOT_OFF_HAND)
    {
        let Some(free) = p.free_carry_slot(&data) else {
            send_fail(p, Fail::InventoryFull, p.items[off].guid);
            return;
        };
        move_item(map, g, (BAG_PLAYER, SLOT_OFF_HAND), free);
    }
    move_item(map, g, from, (BAG_PLAYER, slot));
}

/// `CMSG_AUTOSTORE_BAG_ITEM`: put an item in the first free slot of a bag.
pub fn auto_store(map: &mut Map, g: Guid, from: (u8, u8), bag: u8) {
    let data = map.realm.data.clone();
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let slots: Vec<(u8, u8)> = if bag == BAG_PLAYER {
        (PACK_START..PACK_END).map(|s| (BAG_PLAYER, s)).collect()
    } else {
        (0..p.bag_size(&data, bag)).map(|s| (bag, s)).collect()
    };
    match slots.into_iter().find(|(b, s)| p.item_at(*b, *s).is_none()) {
        Some(to) => move_item(map, g, from, to),
        None => send_fail(p, Fail::BagFull, Guid::zero()),
    }
}

/// `CMSG_SPLIT_ITEM`: part of a stack into an empty slot.
pub fn split(map: &mut Map, g: Guid, from: (u8, u8), to: (u8, u8), count: u32) {
    let data = map.realm.data.clone();
    let realm = map.realm.clone();
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let Some(a) = p.item_at(from.0, from.1) else {
        return;
    };
    if count == 0 || count >= p.items[a].count() {
        let ag = p.items[a].guid;
        send_fail(p, Fail::SplitTooMany, ag);
        return;
    }
    if p.item_at(to.0, to.1).is_some()
        || !slot_exists(p, &data, to.0, to.1)
        || (to.0 == BAG_PLAYER && to.1 < EQUIPMENT_END)
    {
        let ag = p.items[a].guid;
        send_fail(p, Fail::CantStack, ag);
        return;
    }
    let entry = p.items[a].entry;
    let left = p.items[a].count() - count;
    p.items[a].set_count(left);
    let counter = realm
        .next_item
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let item = Item::new(&data, guid::item(counter), g, entry, to.0, to.1, count);
    let mut batch = update::Batch::new();
    batch.push(&update::create_block(&item.create(), Visibility::Owner));
    for frame in batch.finish() {
        p.outbound.send_frame(frame);
    }
    p.items.push(item);
    if let Some(last) = p.items.last_mut() {
        last.fields.clear_dirty();
    }
    p.refresh_inventory(&data);
}

/// `CMSG_DESTROYITEM`.
pub fn destroy(map: &mut Map, g: Guid, at: (u8, u8), count: u32) {
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let Some(i) = p.item_at(at.0, at.1) else {
        return;
    };
    let bag_with_items = p.items[i].bag == BAG_PLAYER
        && p.items
            .iter()
            .any(|x| x.bag == p.items[i].slot && (BAG_START..BAG_END).contains(&p.items[i].slot));
    if bag_with_items {
        send_fail(p, Fail::OnlyEmptyBags, p.items[i].guid);
        return;
    }
    let n = if count == 0 {
        p.items[i].count()
    } else {
        count
    };
    take_item(map, g, i, n);
}

/// The stat bonuses `p`'s equipped gear gives: strength, agility, stamina,
/// intellect, spirit; health; mana; armor.
pub fn gear_bonuses(p: &Player, data: &GameData) -> ([i32; 5], i32, i32, i32) {
    let mut stats = [0i32; 5];
    let (mut health, mut mana, mut armor) = (0, 0, 0);
    for item in p
        .items
        .iter()
        .filter(|i| i.bag == BAG_PLAYER && i.slot < EQUIPMENT_END)
    {
        let Some(t) = data.item(item.entry) else {
            continue;
        };
        armor += t.armor;
        for (kind, value) in t.stats {
            match kind {
                0 => mana += value,
                1 => health += value,
                3 => stats[1] += value,
                4 => stats[0] += value,
                5 => stats[3] += value,
                6 => stats[4] += value,
                7 => stats[2] += value,
                _ => {}
            }
        }
    }
    (stats, health, mana, armor)
}

/// Item spells' triggers.
pub const ITEM_SPELL_ON_USE: u32 = 0;
pub const ITEM_SPELL_ON_EQUIP: u32 = 1;
pub const ITEM_SPELL_CHANCE_ON_HIT: u32 = 2;

/// The auras a player's gear gives, for those whose gear changed.
pub fn sync_gear_all(map: &mut Map) {
    let dirty: Vec<Guid> = map
        .players
        .values()
        .filter(|p| p.gear_dirty)
        .map(|p| p.guid)
        .collect();
    for g in dirty {
        sync_gear(map, g);
    }
}

/// A player's gear auras in line with what they wear: each equipped,
/// unbroken item's on-equip spells, and nothing from gear they took off.
pub fn sync_gear(map: &mut Map, g: Guid) {
    let data = map.realm.data.clone();
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    p.gear_dirty = false;
    let mut wanted: Vec<(Guid, u32)> = Vec::new();
    for item in p
        .items
        .iter()
        .filter(|i| i.bag == BAG_PLAYER && i.slot < EQUIPMENT_END)
    {
        if item.fields.get_u32(ITEM_FIELD_MAXDURABILITY) > 0
            && item.fields.get_u32(ITEM_FIELD_DURABILITY) == 0
        {
            continue;
        }
        let Some(t) = data.item(item.entry) else {
            continue;
        };
        for s in t.spells.iter().filter(|s| s.id != 0 && s.trigger == ITEM_SPELL_ON_EQUIP) {
            wanted.push((item.guid, s.id));
        }
        for s in crate::game::map::enchant::equip_spells(&data, item) {
            wanted.push((item.guid, s));
        }
    }
    wanted.extend(crate::game::map::enchant::set_spells(&data, p));
    wanted.sort_unstable_by_key(|(g, s)| (g.guid(), *s));
    wanted.dedup();
    let have: Vec<(Guid, u32)> = p
        .auras
        .holders
        .iter()
        .filter(|h| h.equipped)
        .filter_map(|h| h.item.map(|i| (i, h.spell)))
        .collect();
    let stale: Vec<(Guid, u32)> = have.iter().filter(|x| !wanted.contains(x)).copied().collect();
    for (item, spell) in stale {
        crate::game::map::auras::remove_where(map, g, |h| {
            h.equipped && h.item == Some(item) && h.spell == spell
        });
    }
    for (item, spell) in wanted.into_iter().filter(|x| !have.contains(x)) {
        let Some(s) = data.spells.get(spell) else {
            continue;
        };
        let effects: Vec<(usize, i32)> = (0..3)
            .filter(|i| matches!(s.effects[*i].effect, 6 | 35) && s.effects[*i].aura != 0)
            .map(|i| (i, crate::game::map::spells::points(s, i, 1)))
            .collect();
        if effects.is_empty() {
            continue;
        }
        let _ = crate::game::map::auras::apply_spell(
            map,
            g,
            g,
            s,
            &effects,
            crate::game::map::auras::Context {
                item: Some(item),
                equipped: true,
                duration: Some(None),
                ..Default::default()
            },
        );
    }
}

/// Take an item out of a player's hands whole, for the mail, the auction
/// house or a trade: it keeps its GUID, wear and flags.
pub fn detach_item(map: &mut Map, g: Guid, i: usize) -> Option<crate::game::store::ItemRow> {
    let data = map.realm.data.clone();
    let p = map.players.get_mut(&g)?;
    if i >= p.items.len() {
        return None;
    }
    let item = p.items.remove(i);
    p.outbound.send(&SMSG_DESTROY_OBJECT { guid: item.guid });
    p.refresh_inventory(&data);
    let entry = item.entry;
    let row = item.to_row();
    crate::game::map::quests::items_changed(map, g, entry);
    Some(row)
}

/// Put an item that was detached into a free slot of a player's bags.
pub fn attach_item(map: &mut Map, g: Guid, row: &crate::game::store::ItemRow) -> Result<(), Fail> {
    let data = map.realm.data.clone();
    let p = map.players.get_mut(&g).ok_or(Fail::NotFound)?;
    let (bag, slot) = p.free_carry_slot(&data).ok_or(Fail::InventoryFull)?;
    let mut row = row.clone();
    row.bag = bag;
    row.slot = slot;
    let item = Item::from_row(&data, &row, g);
    let mut batch = update::Batch::new();
    batch.push(&update::create_block(&item.create(), Visibility::Owner));
    for frame in batch.finish() {
        p.outbound.send_frame(frame);
    }
    let entry = item.entry;
    p.items.push(item);
    if let Some(it) = p.items.last_mut() {
        it.fields.clear_dirty();
    }
    p.refresh_inventory(&data);
    p.outbound.send(&SMSG_ITEM_PUSH_RESULT {
        guid: g,
        source: NewItemSource::Looted,
        creation_type: NewItemCreationType::Received,
        alert_chat: NewItemChatAlert::Show,
        bag_slot: bag,
        item_slot: u32::from(slot),
        item: entry,
        item_suffix_factor: 0,
        item_random_property_id: 0,
        item_count: row.count,
    });
    crate::game::map::quests::items_changed(map, g, entry);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_two_hander_goes_only_in_the_main_hand_and_rings_in_either_finger() {
        assert_eq!(slots_for(17), &[15]);
        assert_eq!(slots_for(11), &[10, 11]);
        assert_eq!(slots_for(18), &[19, 20, 21, 22]);
    }

    #[test]
    fn plate_waits_for_level_forty() {
        let plate = ItemTemplate {
            class: 4,
            subclass: 4,
            ..Default::default()
        };
        assert!(!proficient(1, 39, &plate));
        assert!(proficient(1, 40, &plate));
        assert!(!proficient(8, 60, &plate));
    }
}
