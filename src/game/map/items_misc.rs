//! What players do with items beyond moving and using them: opening
//! (lockboxes, clams, gifts), reading, gift-wrapping, banking at a click,
//! and equipping into a chosen slot.
//!
//! An opened item's loot window is the item's own; its loot lives with the
//! player, so the loot packets for an item are answered here.

use ahash::AHashSet;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::data::items::Body;
use crate::game::map::inventory::{
    self, BANK_BAG_END, BANK_BAG_START, BANK_END, BANK_START, EQUIPMENT_END,
};
use crate::game::map::loot::{self, Loot};
use crate::game::store::BAG_PLAYER;

/// `ITEM_FIELD_FLAGS`: unlocked (a picked lockbox), and wrapped as a gift.
pub const ITEM_DYNFLAG_UNLOCKED: u32 = 0x4;
pub const ITEM_DYNFLAG_WRAPPED: u32 = 0x8;
/// `ItemTemplate.Flags`: it opens into loot; it wraps gifts.
pub const ITEM_FLAG_LOOTABLE: u32 = 0x4;
pub const ITEM_FLAG_WRAPPER: u32 = 0x200;

/// Wrapping paper, and the wrapped gift each makes.
const WRAPPINGS: [(u32, u32); 6] = [
    (5042, 5043),
    (5048, 5044),
    (17303, 17302),
    (17304, 17305),
    (17307, 17308),
    (21830, 21831),
];
/// What an item's stored data says it wraps.
const GIFT_PREFIX: &str = "gift:";

pub fn handle(map: &mut Map, g: Guid, msg: ClientOpcodeMessage) -> Option<ClientOpcodeMessage> {
    use ClientOpcodeMessage as C;
    // The loot packets, while an item's loot window is open.
    let item_window = map
        .players
        .get(&g)
        .and_then(|p| p.looting)
        .is_some_and(|t| guid::high(t) == Some(guid::HighGuid::Item));
    match msg {
        C::CMSG_AUTOSTORE_LOOT_ITEM(m) if item_window => take(map, g, m.item_slot),
        C::CMSG_LOOT_MONEY if item_window => take_money(map, g),
        C::CMSG_LOOT_RELEASE(m) if item_window => release(map, g, m.guid),
        C::CMSG_OPEN_ITEM(m) => open(map, g, m.bag_index, m.slot),
        C::CMSG_READ_ITEM(m) => read(map, g, m.bag_index, m.slot),
        C::CMSG_PAGE_TEXT_QUERY(m) => page_text(map, g, m.page_id),
        C::CMSG_WRAP_ITEM(m) => wrap(
            map,
            g,
            (m.gift_bag_index, m.gift_slot),
            (m.item_bag_index, m.item_slot),
        ),
        C::CMSG_AUTOBANK_ITEM(m) => bank(map, g, (m.bag_index, m.slot_index)),
        C::CMSG_AUTOSTORE_BANK_ITEM(m) => unbank(map, g, (m.bag_index, m.slot_index)),
        C::CMSG_AUTOEQUIP_ITEM_SLOT(m) => equip_to(map, g, m.guid, m.destination_slot),
        other => return Some(other),
    }
    None
}

/// `SMSG_INVENTORY_CHANGE_FAILURE` by hand: the result, the item, no
/// second item, no bag.
pub fn fail(p: &crate::game::map::player::Player, result: InventoryResult, item: Guid) {
    let mut b = Body::default();
    b.u8(result.as_int()).u64(item.guid()).u64(0).u8(0);
    p.outbound.send_frame(b.frame(0x112));
}

/// `CMSG_OPEN_ITEM`: a gift unwraps; a lockbox (unlocked) or clam opens
/// into its loot.
pub fn open(map: &mut Map, g: Guid, bag: u8, slot: u8) {
    let data = map.realm.data.clone();
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let Some(i) = p.item_at(bag, slot) else {
        fail(p, InventoryResult::SlotIsEmpty, Guid::zero());
        return;
    };
    let item = &p.items[i];
    let item_guid = item.guid;
    let flags = item.fields.get_u32(ITEM_FIELD_FLAGS);
    // A gift: the wrapped item is back.
    if flags & ITEM_DYNFLAG_WRAPPED != 0 {
        let Some(entry) = item
            .data
            .strip_prefix(GIFT_PREFIX)
            .and_then(|e| e.parse::<u32>().ok())
        else {
            return;
        };
        let item = &mut p.items[i];
        item.entry = entry;
        item.data.clear();
        item.fields.set_u32(OBJECT_FIELD_ENTRY, entry);
        item.fields.set_flag(ITEM_FIELD_FLAGS, ITEM_DYNFLAG_WRAPPED, false);
        item.fields.set_guid(ITEM_FIELD_GIFTCREATOR, Guid::zero());
        p.refresh_inventory(&data);
        return;
    }
    let Some(t) = data.item(item.entry) else {
        return;
    };
    if t.lock_id != 0 && flags & ITEM_DYNFLAG_UNLOCKED == 0 {
        fail(p, InventoryResult::ItemLocked, item_guid);
        return;
    }
    if t.flags & ITEM_FLAG_LOOTABLE == 0 && !data.loot.item.contains_key(&t.entry) {
        fail(p, InventoryResult::CantDoRightNow, item_guid);
        return;
    }
    let mut rng = rand::rng();
    let mut items = Vec::new();
    let wanted = crate::game::map::quests::wanted_items(p, &data);
    if let Some(table) = data.loot.item.get(&t.entry) {
        loot::roll(&data, table, &wanted, &mut rng, &mut items, 0);
    }
    let gold = if t.max_money_loot > t.min_money_loot {
        rand::RngExt::random_range(&mut rng, t.min_money_loot..=t.max_money_loot)
    } else {
        t.min_money_loot
    };
    let l = Loot {
        items,
        gold,
        owner: Some(g),
        looters: AHashSet::from([g]),
        ..Default::default()
    };
    let frame = loot::response(&data, item_guid, &l, g, &wanted);
    p.misc.item_loot = Some((item_guid, l));
    p.looting = Some(item_guid);
    p.outbound.send_frame(frame);
}

fn take(map: &mut Map, g: Guid, slot: u8) {
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let Some((_, l)) = p.misc.item_loot.as_mut() else {
        return;
    };
    let Some(s) = l.items.get(usize::from(slot)).filter(|s| !s.taken).cloned() else {
        return;
    };
    match inventory::add_item(map, g, s.item, s.count, true) {
        Ok(()) => {
            if let Some(p) = map.players.get_mut(&g) {
                if let Some((_, l)) = p.misc.item_loot.as_mut()
                    && let Some(x) = l.items.get_mut(usize::from(slot))
                {
                    x.taken = true;
                }
                p.outbound.send(&SMSG_LOOT_REMOVED { slot });
            }
        }
        Err(f) => {
            if let Some(p) = map.players.get(&g) {
                inventory::send_fail(p, f, Guid::zero());
            }
        }
    }
}

fn take_money(map: &mut Map, g: Guid) {
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let Some((_, l)) = p.misc.item_loot.as_mut() else {
        return;
    };
    let gold = std::mem::take(&mut l.gold);
    if gold == 0 {
        return;
    }
    let money = p.money().saturating_add(gold);
    p.set_money(money);
    p.outbound.send(&SMSG_LOOT_CLEAR_MONEY {});
}

/// The window closes: an emptied item is used up.
fn release(map: &mut Map, g: Guid, target: Guid) {
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    p.looting = None;
    p.outbound.send(&SMSG_LOOT_RELEASE_RESPONSE {
        guid: target,
        unknown1: 1,
    });
    let Some((item, l)) = p.misc.item_loot.take() else {
        return;
    };
    if !l.is_empty() {
        // Not emptied: it may be opened again, with what was left.
        p.misc.item_loot = Some((item, l));
        return;
    }
    p.misc.item_loot = None;
    if let Some(i) = p.item_by_guid(item) {
        inventory::take_item(map, g, i, 1);
    }
}

/// `CMSG_READ_ITEM`: an item with a page may be read.
fn read(map: &mut Map, g: Guid, bag: u8, slot: u8) {
    let data = map.realm.data.clone();
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let Some(i) = p.item_at(bag, slot) else {
        return;
    };
    let item = &p.items[i];
    let readable = data.item(item.entry).is_some_and(|t| t.page_text != 0)
        || item.fields.get_u32(ITEM_FIELD_ITEM_TEXT_ID) != 0;
    if readable {
        p.outbound.send(&SMSG_READ_ITEM_OK { guid: item.guid });
    } else {
        p.outbound.send(&SMSG_READ_ITEM_FAILED { guid: item.guid });
    }
}

/// `CMSG_PAGE_TEXT_QUERY`: a page, and the page after it.
pub fn page_text(map: &Map, g: Guid, page: u32) {
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let (text, next) = map
        .realm
        .data
        .misc
        .page_texts
        .get(&page)
        .cloned()
        .unwrap_or_else(|| ("Item page missing.".to_owned(), 0));
    p.outbound.send(&SMSG_PAGE_TEXT_QUERY_RESPONSE {
        page_id: page,
        text,
        next_page_id: next,
    });
}

/// `CMSG_WRAP_ITEM`: wrapping paper makes an item a gift, which keeps what
/// it was.
fn wrap(map: &mut Map, g: Guid, paper: (u8, u8), what: (u8, u8)) {
    let data = map.realm.data.clone();
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let (Some(pi), Some(wi)) = (p.item_at(paper.0, paper.1), p.item_at(what.0, what.1)) else {
        fail(p, InventoryResult::ItemNotFound, Guid::zero());
        return;
    };
    let paper_entry = p.items[pi].entry;
    let Some(gift) = WRAPPINGS
        .iter()
        .find(|(w, _)| *w == paper_entry)
        .map(|(_, gift)| *gift)
        .filter(|_| data.item(paper_entry).is_none_or(|t| t.flags & ITEM_FLAG_WRAPPER != 0))
    else {
        fail(p, InventoryResult::CantDoRightNow, p.items[pi].guid);
        return;
    };
    let item = &p.items[wi];
    let template = data.item(item.entry);
    let why = if pi == wi {
        Some(InventoryResult::CantDoRightNow)
    } else if item.fields.get_u32(ITEM_FIELD_FLAGS) & ITEM_DYNFLAG_WRAPPED != 0 {
        Some(InventoryResult::WrappedCantBeWrapped)
    } else if item.bag == BAG_PLAYER && item.slot < EQUIPMENT_END {
        Some(InventoryResult::EquippedCantBeWrapped)
    } else if item.count() > 1 {
        Some(InventoryResult::StackableCantBeWrapped)
    } else if template.is_some_and(|t| t.bonding == 1 || t.max_count > 0) {
        Some(InventoryResult::BoundCantBeWrapped)
    } else {
        None
    };
    if let Some(why) = why {
        fail(p, why, item.guid);
        return;
    }
    let item = &mut p.items[wi];
    item.data = format!("{GIFT_PREFIX}{}", item.entry);
    item.entry = gift;
    item.fields.set_u32(OBJECT_FIELD_ENTRY, gift);
    item.fields.set_flag(ITEM_FIELD_FLAGS, ITEM_DYNFLAG_WRAPPED, true);
    item.fields.set_guid(ITEM_FIELD_GIFTCREATOR, g);
    p.refresh_inventory(&data);
    inventory::take_item(map, g, pi, 1);
}

/// The bank's slots: its own, then its bags'.
fn bank_slots(p: &crate::game::map::player::Player, data: &crate::game::data::GameData) -> Vec<(u8, u8)> {
    let mut slots: Vec<(u8, u8)> = (BANK_START..BANK_END).map(|s| (BAG_PLAYER, s)).collect();
    for bag in BANK_BAG_START..BANK_BAG_END {
        if p.item_at(BAG_PLAYER, bag).is_some() {
            slots.extend((0..p.bag_size(data, bag)).map(|s| (bag, s)));
        }
    }
    slots
}

fn in_bank(at: (u8, u8)) -> bool {
    (at.0 == BAG_PLAYER && (BANK_START..BANK_END).contains(&at.1))
        || (BANK_BAG_START..BANK_BAG_END).contains(&at.0)
}

/// `CMSG_AUTOBANK_ITEM`: into the first free bank slot.
fn bank(map: &mut Map, g: Guid, from: (u8, u8)) {
    let data = map.realm.data.clone();
    let Some(p) = map.players.get(&g) else {
        return;
    };
    if p.item_at(from.0, from.1).is_none() || in_bank(from) {
        return;
    }
    match bank_slots(p, &data)
        .into_iter()
        .find(|(b, s)| p.item_at(*b, *s).is_none())
    {
        Some(to) => inventory::move_item(map, g, from, to),
        None => fail(p, InventoryResult::BankFull, Guid::zero()),
    }
}

/// `CMSG_AUTOSTORE_BANK_ITEM`: from the bank into the first free carried
/// slot.
fn unbank(map: &mut Map, g: Guid, from: (u8, u8)) {
    let data = map.realm.data.clone();
    let Some(p) = map.players.get(&g) else {
        return;
    };
    if p.item_at(from.0, from.1).is_none() || !in_bank(from) {
        return;
    }
    match p.free_carry_slot(&data) {
        Some(to) => inventory::move_item(map, g, from, to),
        None => fail(p, InventoryResult::InventoryFull, Guid::zero()),
    }
}

/// `CMSG_AUTOEQUIP_ITEM_SLOT`: an item, wherever it is, into an
/// equipment slot.
fn equip_to(map: &mut Map, g: Guid, item: Guid, slot: u8) {
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let Some(i) = p.item_by_guid(item) else {
        return;
    };
    if slot >= EQUIPMENT_END {
        return;
    }
    let from = (p.items[i].bag, p.items[i].slot);
    inventory::move_item(map, g, from, (BAG_PLAYER, slot));
}
