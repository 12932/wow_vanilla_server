//! Vendors: their lists, buying (from limited stock too), selling, buying
//! back, repairs and wear, as Thistle Tea's `Vendor.*` and
//! `Item.Durability` keep them:
//!
//! - limited stock comes back a buy count at a time, one restock interval
//!   after another from the last purchase;
//! - a damaged item sells for its price less its repair, a charged one for
//!   the share of charges it has left; a part of a stack sold is bought
//!   back on its own; the buyback slots fill in turn, then the first free,
//!   then the oldest, and empty at logout;
//! - repairing everything is all or nothing, at the reputation discount;
//! - dying wears each worn item down a tenth, and a blow now and then a
//!   point off one.

use std::time::{Duration, Instant};

use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::{
    BuyResult, BuybackSlot, ListInventoryItem, SMSG_BUY_FAILED, SMSG_BUY_ITEM, SMSG_LIST_INVENTORY,
    SMSG_SELL_ITEM, SellItemResult,
};

use crate::game::data::{GameData, ItemTemplate, VendorItem};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::inventory::{
    self, BAG_END, BAG_START, BUYBACK_END, BUYBACK_START, EQUIPMENT_END, Fail,
};
use crate::game::map::player::{Item, Player};
use crate::game::store::BAG_PLAYER;

/// `UNIT_NPC_FLAGS`.
pub const NPC_FLAG_VENDOR: u32 = 0x4;
pub const NPC_FLAG_REPAIR: u32 = 0x4000;
/// How far a player may stand from an NPC to use it.
pub const NPC_YARDS: f32 = 8.0;
/// The most money a player holds.
const MAX_MONEY: u32 = 2_147_483_647;
/// A buyback slot's timestamp counts from login, thirty hours on.
const BUYBACK_OFFSET_SECS: u32 = 108_000;
/// One blow in two hundred wears a point off something worn.
const HIT_WEAR_CHANCE: f32 = 0.5;
/// The inventory slots that wear: the equipment.
const WEAR_SLOTS: u8 = EQUIPMENT_END;
/// Past the backpack: the bank, and on.
const BACKPACK_END: u8 = 39;
const KEYRING_START: u8 = 81;

/// A vendor's stock of a limited item (Thistle Tea's `VendorStock`): what's
/// left, since when it has been growing back, and how often a buy count of
/// it comes back.
#[derive(Debug, Clone, Copy)]
pub struct Stock {
    pub count: u32,
    pub last_increment_at: Instant,
    pub restock: Duration,
}

/// The vendor creature `npc`, if `player` may use it now (the services'
/// shared checks).
fn vendor_entry(map: &Map, player: Guid, npc: Guid, flag: u32) -> Option<u32> {
    crate::game::map::npc::npc_entry(map, player, npc, flag)
}

/// The price of `count` of an item, from its template's price for its buy
/// count.
fn price(t: &ItemTemplate, count: u32) -> u32 {
    let per = t.buy_count.max(1);
    t.buy_price.saturating_mul(count) / per
}

/// How many of a limited item a vendor has (`u32::MAX`: unlimited): what
/// was left, and a buy count back for each restock interval since.
pub fn stock_left(map: &mut Map, npc: Guid, v: &VendorItem, buy_count: u32) -> u32 {
    if v.max_count == 0 {
        return u32::MAX;
    }
    let now = map.now;
    let Some(s) = map.vendor_stock.get_mut(&(npc, v.item)) else {
        return v.max_count;
    };
    let intervals = (now
        .saturating_duration_since(s.last_increment_at)
        .as_millis()
        / s.restock.as_millis().max(1)) as u32;
    let count = s
        .count
        .saturating_add(intervals.saturating_mul(buy_count.max(1)))
        .min(v.max_count);
    if count >= v.max_count {
        map.vendor_stock.remove(&(npc, v.item));
        return v.max_count;
    }
    if intervals > 0 {
        s.count = count;
        s.last_increment_at = now;
    }
    count
}

/// The interval chosen at a purchase (Thistle Tea's `restock_delay`): the
/// item's own, varied by 80 to 120% if it says so, shortened past 2,500
/// players online if it says so.
fn restock_delay(map: &Map, v: &VendorItem) -> Duration {
    let mut secs = v.restock_secs.max(1);
    if v.flags & 1 != 0 {
        secs = secs * rand::rng().random_range(80..=120) / 100;
    }
    let population = map.realm.registry.online_count() as u32;
    if v.flags & 2 != 0 && population > 2_500 {
        secs = secs * 2_500 / population;
    }
    Duration::from_secs(u64::from(secs.max(1)))
}

/// `CMSG_LIST_INVENTORY`.
pub fn list(map: &mut Map, player: Guid, npc: Guid) {
    let Some(entry) = vendor_entry(map, player, npc, NPC_FLAG_VENDOR) else {
        return;
    };
    let data = map.realm.data.clone();
    let mut items = Vec::new();
    for (i, v) in data.vendor_items(entry).iter().enumerate() {
        let Some(t) = data.item(v.item) else {
            continue;
        };
        if v.condition != 0
            && !crate::game::map::conditions::met(map, v.condition, Some(npc), Some(player))
        {
            continue;
        }
        // Items for a standing the player hasn't reached aren't shown.
        if t.required_reputation_faction == 0
            && t.required_reputation_rank > 0
            && let Some(f) = crate::game::map::reputation::faction_of(map, npc)
            && map.players.get(&player).is_some_and(|p| {
                !crate::game::map::reputation::meets(p, &data, f, t.required_reputation_rank)
            })
        {
            continue;
        }
        let left = stock_left(map, npc, v, t.buy_count);
        items.push(ListInventoryItem {
            item_stack_count: i as u32 + 1,
            item: v.item,
            item_display_id: t.display_id,
            max_items: left,
            price: crate::game::map::reputation::price(
                map,
                player,
                npc,
                price(t, t.buy_count.max(1)),
            )
            .into(),
            max_durability: t.max_durability,
            durability: t.buy_count.max(1),
        });
    }
    if let Some(p) = map.players.get(&player) {
        p.outbound.send(&SMSG_LIST_INVENTORY { vendor: npc, items });
    }
}

fn buy_failed(map: &Map, player: Guid, npc: Guid, item: u32, result: BuyResult) {
    if let Some(p) = map.players.get(&player) {
        p.outbound.send(&SMSG_BUY_FAILED {
            guid: npc,
            item,
            result,
        });
    }
}

/// `CMSG_BUY_ITEM`: `amount` stacks of an item's buy count.
pub fn buy(map: &mut Map, player: Guid, npc: Guid, item: u32, amount: u8) {
    let Some(entry) = vendor_entry(map, player, npc, NPC_FLAG_VENDOR) else {
        return;
    };
    let data = map.realm.data.clone();
    let list = data.vendor_items(entry);
    let Some((index, v)) = list.iter().enumerate().find(|(_, v)| v.item == item) else {
        buy_failed(map, player, npc, item, BuyResult::CantFindItem);
        return;
    };
    if v.condition != 0
        && !crate::game::map::conditions::met(map, v.condition, Some(npc), Some(player))
    {
        buy_failed(map, player, npc, item, BuyResult::CantFindItem);
        return;
    }
    let Some(t) = data.item(item) else {
        return;
    };
    if map
        .players
        .get(&player)
        .is_some_and(|p| !crate::game::map::pvp::rank_allows_buy(p, t))
    {
        buy_failed(map, player, npc, item, BuyResult::RankRequire);
        return;
    }
    // A standing the item asks for: with its faction, or the vendor's if
    // it names a rank alone, as mangos's BuyItemFromVendor and Thistle
    // Tea's `item_requirement_met?` have it.
    let wanted = match t.required_reputation_faction {
        0 if t.required_reputation_rank > 0 => {
            crate::game::map::reputation::faction_of(map, npc).unwrap_or(0)
        }
        f => f,
    };
    if wanted != 0
        && map.players.get(&player).is_some_and(|p| {
            crate::game::map::reputation::rank(p, &data, wanted) < t.required_reputation_rank
        })
    {
        buy_failed(map, player, npc, item, BuyResult::ReputationRequire);
        return;
    }
    let count = u32::from(amount.max(1)) * t.buy_count.max(1);
    let left = stock_left(map, npc, v, t.buy_count);
    if left != u32::MAX && left < count {
        buy_failed(map, player, npc, item, BuyResult::ItemAlreadySold);
        return;
    }
    let cost = crate::game::map::reputation::price(map, player, npc, price(t, count));
    if map.players.get(&player).is_none_or(|p| p.money() < cost) {
        buy_failed(map, player, npc, item, BuyResult::NotEnoughtMoney);
        return;
    }
    if let Err(f) = inventory::add_item(map, player, item, count, true) {
        if let Some(p) = map.players.get(&player) {
            inventory::send_fail(p, f, Guid::zero());
        }
        return;
    }
    // A purchase starts the restock interval over.
    let new_left = if v.max_count > 0 {
        let stock = Stock {
            count: left - count,
            last_increment_at: map.now,
            restock: restock_delay(map, v),
        };
        map.vendor_stock.insert((npc, item), stock);
        left - count
    } else {
        u32::MAX
    };
    if let Some(p) = map.players.get_mut(&player) {
        let money = p.money() - cost;
        p.set_money(money);
        p.outbound.send(&SMSG_BUY_ITEM {
            guid: npc,
            vendor_slot: index as u32 + 1,
            amount_for_sale: new_left,
            amount_bought: count,
        });
    }
}

/// `CMSG_BUY_ITEM_IN_SLOT`: bought as [`buy`] buys, and a new item put
/// where the client dropped it: a slot of the backpack (the bag is the
/// player) or of a bag worn, or (slot 255) anywhere in that bag, as
/// mangos's BuyItemFromVendor stores it at the position asked and Thistle
/// Tea's `buy_in_slot` sends it.
pub fn buy_in_slot(
    map: &mut Map,
    player: Guid,
    npc: Guid,
    item: u32,
    amount: u8,
    bag: Guid,
    slot: u8,
) {
    let data = map.realm.data.clone();
    let Some(p) = map.players.get(&player) else {
        return;
    };
    let bag = if bag == player {
        Some(BAG_PLAYER)
    } else {
        p.items
            .iter()
            .find(|i| {
                i.guid == bag && i.bag == BAG_PLAYER && (BAG_START..BAG_END).contains(&i.slot)
            })
            .map(|i| i.slot)
    };
    let Some(bag) = bag else {
        return;
    };
    let before: Vec<Guid> = p
        .items
        .iter()
        .filter(|i| i.entry == item)
        .map(|i| i.guid)
        .collect();
    buy(map, player, npc, item, amount);
    let Some(p) = map.players.get(&player) else {
        return;
    };
    let Some(new) = p
        .items
        .iter()
        .find(|i| i.entry == item && !before.contains(&i.guid))
        .map(|i| (i.bag, i.slot))
    else {
        return;
    };
    let (first, end) = if bag == BAG_PLAYER {
        (inventory::PACK_START, inventory::PACK_END)
    } else {
        (0, p.bag_size(&data, bag))
    };
    let to = if slot == u8::MAX {
        (first..end).find(|s| p.item_at(bag, *s).is_none())
    } else {
        Some(slot).filter(|s| (first..end).contains(s) && p.item_at(bag, *s).is_none())
    };
    if let Some(to) = to
        && new != (bag, to)
        && !(slot == u8::MAX && new.0 == bag)
    {
        inventory::move_item(map, player, new, (bag, to));
    }
}

/// Whether an item is carried: in the backpack, a bag, or a bag slot.
fn carried(it: &Item) -> bool {
    (it.bag == BAG_PLAYER && (BAG_START..BACKPACK_END).contains(&it.slot))
        || (BAG_START..BAG_END).contains(&it.bag)
}

/// What `count` of `it` sells for (Thistle Tea's `Buyback.sale_price`): a
/// charged item's share of its charges left, less what mending it would
/// cost, but at least a copper; `None` when it can't be priced.
fn sale_price(data: &GameData, it: &Item, t: &ItemTemplate, count: u32) -> Option<u32> {
    let base = t.sell_price.saturating_mul(count);
    let price = match t
        .spells
        .iter()
        .enumerate()
        .find(|(_, s)| s.id != 0 && s.charges < 0)
    {
        Some((k, s)) => {
            let left = it.fields.get_u32(ITEM_FIELD_SPELL_CHARGES + k as u16) as i32;
            (base as f32 * (left as f32 / s.charges as f32)) as u32
        }
        None => base,
    };
    let (max, now) = (
        it.fields.get_u32(ITEM_FIELD_MAXDURABILITY),
        it.fields.get_u32(ITEM_FIELD_DURABILITY),
    );
    let penalty = if max > now {
        repair_cost(data, t, max - now, 1.0)?
    } else {
        0
    };
    Some(if penalty > price { 1 } else { price - penalty })
}

/// `CMSG_SELL_ITEM`: the item goes to the buyback slots, and its price to
/// the player.
pub fn sell(map: &mut Map, player: Guid, npc: Guid, item: Guid, amount: u8) {
    let data = map.realm.data.clone();
    let reply = |map: &Map, result| {
        if let Some(p) = map.players.get(&player) {
            p.outbound.send(&SMSG_SELL_ITEM {
                guid: npc,
                item,
                result,
            });
        }
    };
    if vendor_entry(map, player, npc, NPC_FLAG_VENDOR).is_none() {
        reply(map, SellItemResult::CantFindVendor);
        return;
    }
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    let Some(i) = p.item_by_guid(item) else {
        reply(map, SellItemResult::CantFindItem);
        return;
    };
    let it = &p.items[i];
    // Carried, and a bag only empty.
    let full_bag = it.bag == BAG_PLAYER
        && (BAG_START..BAG_END).contains(&it.slot)
        && p.items.iter().any(|x| x.bag == it.slot);
    if !carried(it) || full_bag {
        reply(map, SellItemResult::CantSellItem);
        return;
    }
    let Some(t) = data.item(it.entry) else {
        return;
    };
    if t.sell_price == 0 || u32::from(amount) > it.count() {
        reply(map, SellItemResult::CantSellItem);
        return;
    }
    let count = if amount == 0 {
        it.count()
    } else {
        u32::from(amount)
    };
    let Some(gain) = sale_price(&data, it, t, count) else {
        reply(map, SellItemResult::CantSellItem);
        return;
    };
    if p.money().saturating_add(gain) > MAX_MONEY {
        reply(map, SellItemResult::CantSellItem);
        return;
    }
    let whole = count == it.count();
    let (entry, left) = (it.entry, it.count() - count);
    let money = p.money() + gain;
    p.set_money(money);
    // The slot it goes to, and what was there is gone for good.
    let slot = buyback_slot(p);
    if let Some(old) = p.item_at(BAG_PLAYER, slot) {
        let gone = p.items.remove(old);
        p.outbound
            .send(&wow_world_messages::vanilla::SMSG_DESTROY_OBJECT { guid: gone.guid });
    }
    if whole {
        let Some(i) = p.item_by_guid(item) else {
            return;
        };
        p.items[i].bag = BAG_PLAYER;
        p.items[i].slot = slot;
    } else {
        // Part of a stack: what's sold is its own item now.
        let Some(i) = p.item_by_guid(item) else {
            return;
        };
        p.items[i].set_count(left);
        let counter = map
            .realm
            .next_item
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let new = Item::new(
            &data,
            guid::item(counter),
            player,
            entry,
            BAG_PLAYER,
            slot,
            count,
        );
        let mut batch = crate::game::update::Batch::new();
        batch.push(&crate::game::update::create_block(
            &new.create(),
            Visibility::Owner,
        ));
        for frame in batch.finish() {
            p.outbound.send_frame(frame);
        }
        p.items.push(new);
        if let Some(last) = p.items.last_mut() {
            last.fields.clear_dirty();
        }
    }
    let now = map.now;
    let n = u16::from(slot - BUYBACK_START);
    let since = now.saturating_duration_since(p.buyback_started).as_secs() as u32;
    p.fields.set_u32(PLAYER_FIELD_BUYBACK_PRICE_1 + n, gain);
    p.fields.set_u32(
        PLAYER_FIELD_BUYBACK_TIMESTAMP_1 + n,
        since + BUYBACK_OFFSET_SECS,
    );
    p.buyback_next = (slot - BUYBACK_START + 1).min(BUYBACK_END - BUYBACK_START - 1);
    p.refresh_inventory(&data);
}

/// The buyback slot a sale goes to (Thistle Tea's `available_slot`): the
/// next one, else the first free past the first, else the oldest.
fn buyback_slot(p: &Player) -> u8 {
    let next = BUYBACK_START + p.buyback_next.min(BUYBACK_END - BUYBACK_START - 1);
    if p.item_at(BAG_PLAYER, next).is_none() {
        return next;
    }
    if let Some(free) =
        (BUYBACK_START + 1..BUYBACK_END).find(|s| p.item_at(BAG_PLAYER, *s).is_none())
    {
        return free;
    }
    (BUYBACK_START..BUYBACK_END)
        .min_by_key(|s| {
            (
                p.fields
                    .get_u32(PLAYER_FIELD_BUYBACK_TIMESTAMP_1 + u16::from(*s - BUYBACK_START)),
                *s,
            )
        })
        .unwrap_or(BUYBACK_START)
}

/// `CMSG_BUYBACK_ITEM`.
pub fn buyback(map: &mut Map, player: Guid, npc: Guid, slot: BuybackSlot) {
    if vendor_entry(map, player, npc, NPC_FLAG_VENDOR).is_none() {
        return;
    }
    let data = map.realm.data.clone();
    let slot = slot.as_int() as u8;
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    let Some(i) = p.item_at(BAG_PLAYER, slot) else {
        return;
    };
    let n = u16::from(slot - BUYBACK_START);
    let cost = p.fields.get_u32(PLAYER_FIELD_BUYBACK_PRICE_1 + n);
    if p.money() < cost {
        let entry = p.items[i].entry;
        buy_failed(map, player, npc, entry, BuyResult::NotEnoughtMoney);
        return;
    }
    let Some(to) = p.free_carry_slot(&data) else {
        inventory::send_fail(p, Fail::InventoryFull, Guid::zero());
        return;
    };
    let money = p.money() - cost;
    p.set_money(money);
    p.items[i].bag = to.0;
    p.items[i].slot = to.1;
    p.fields.set_u32(PLAYER_FIELD_BUYBACK_PRICE_1 + n, 0);
    p.fields.set_u32(PLAYER_FIELD_BUYBACK_TIMESTAMP_1 + n, 0);
    // The next sale fills the slot freed, if the next one is taken.
    let next = BUYBACK_START + p.buyback_next;
    if p.item_at(BAG_PLAYER, next).is_some() {
        p.buyback_next = slot - BUYBACK_START;
    }
    p.refresh_inventory(&data);
}

/// Whether an item is in a buyback slot (gone at logout).
pub fn in_buyback(bag: u8, slot: u8) -> bool {
    bag == BAG_PLAYER && (BUYBACK_START..BUYBACK_END).contains(&slot)
}

/// What repairing `lost` durability of an item costs (Thistle Tea's
/// `Durability.repair_cost`, vmangos's `DurabilityRepair`): the item
/// level's cost for its kind times its quality's, at `discount`, rounded
/// half to even, at least a copper; `None` for a kind with no cost.
pub fn repair_cost(data: &GameData, t: &ItemTemplate, lost: u32, discount: f32) -> Option<u32> {
    if lost == 0 {
        return Some(0);
    }
    let costs = data.durability_costs.get(&t.item_level)?;
    let index = match t.class {
        2 => t.subclass as usize,
        4 => 21 + t.subclass as usize,
        _ => return None,
    };
    let base = costs.get(index).copied().unwrap_or(0);
    let quality = data
        .durability_quality
        .get(&((t.quality + 1) * 2))
        .copied()
        .unwrap_or(1.0);
    let cost = (lost as f32 * base as f32 * quality).round() * discount;
    let whole = cost.trunc();
    let rounded = if cost - whole == 0.5 {
        whole as u32 + whole as u32 % 2
    } else {
        cost.round() as u32
    };
    Some(rounded.max(1))
}

/// Whether an item wears and is repaired: worn, carried, or on the
/// keyring (not banked, not sold).
fn mendable(it: &Item) -> bool {
    (it.bag == BAG_PLAYER && (it.slot < BACKPACK_END || it.slot >= KEYRING_START)) || carried(it)
}

/// `CMSG_REPAIR_ITEM`: one item, or (with a zero guid) everything worn and
/// carried, paid for at once or not at all.
pub fn repair(map: &mut Map, player: Guid, npc: Guid, item: Guid) {
    if vendor_entry(map, player, npc, NPC_FLAG_REPAIR).is_none() {
        return;
    }
    let data = map.realm.data.clone();
    let discount = crate::game::map::reputation::discount(map, player, npc);
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    let mut costs = Vec::new();
    for (i, it) in p.items.iter().enumerate() {
        if !(item.is_zero() && mendable(it) || it.guid == item) {
            continue;
        }
        let (max, now) = (
            it.fields.get_u32(ITEM_FIELD_MAXDURABILITY),
            it.fields.get_u32(ITEM_FIELD_DURABILITY),
        );
        if max == 0 || now >= max {
            continue;
        }
        let Some(cost) = data
            .item(it.entry)
            .and_then(|t| repair_cost(&data, t, max - now, discount))
        else {
            return;
        };
        costs.push((i, cost, max));
    }
    let total: u32 = costs.iter().map(|(_, c, _)| c).sum();
    if costs.is_empty() || p.money() < total {
        return;
    }
    let money = p.money() - total;
    p.set_money(money);
    for (i, _, max) in costs {
        p.items[i].fields.set_u32(ITEM_FIELD_DURABILITY, max);
    }
    p.gear_dirty = true;
}

/// Wear on everything worn: `fraction` of each item's full durability, at
/// least a point.
pub fn wear(map: &mut Map, player: Guid, fraction: f32) {
    wear_where(map, player, fraction, |i| {
        i.bag == BAG_PLAYER && i.slot < WEAR_SLOTS
    });
}

/// Wear on everything worn and carried (a spirit healer's price).
pub fn wear_carried(map: &mut Map, player: Guid, fraction: f32) {
    wear_where(map, player, fraction, mendable);
}

fn wear_where(map: &mut Map, player: Guid, fraction: f32, pick: impl Fn(&Item) -> bool) {
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    for it in p.items.iter_mut().filter(|i| pick(i)) {
        let max = it.fields.get_u32(ITEM_FIELD_MAXDURABILITY);
        if max == 0 {
            continue;
        }
        let now = it.fields.get_u32(ITEM_FIELD_DURABILITY);
        let loss = ((max as f32 * fraction) as u32).max(1);
        it.fields
            .set_u32(ITEM_FIELD_DURABILITY, now.saturating_sub(loss));
    }
    p.gear_dirty = true;
}

/// A blow between `attacker` and `victim` (Thistle Tea's `hit_loss`): each
/// player of the two, one time in two hundred, loses a point off a random
/// equipment slot's item.
pub fn wear_from_blow(map: &mut Map, attacker: Guid, victim: Guid) {
    if attacker == victim {
        return;
    }
    let mut rng = rand::rng();
    for g in [victim, attacker] {
        if !guid::is_player(g) || rng.random_range(0.0f32..100.0) >= HIT_WEAR_CHANCE {
            continue;
        }
        let slot = rng.random_range(0..WEAR_SLOTS);
        let Some(p) = map.players.get_mut(&g) else {
            continue;
        };
        let Some(i) = p.item_at(BAG_PLAYER, slot) else {
            continue;
        };
        let it = &mut p.items[i];
        let (max, now) = (
            it.fields.get_u32(ITEM_FIELD_MAXDURABILITY),
            it.fields.get_u32(ITEM_FIELD_DURABILITY),
        );
        if max == 0 || now == 0 {
            continue;
        }
        it.fields.set_u32(ITEM_FIELD_DURABILITY, now - 1);
        if now == 1 {
            p.gear_dirty = true;
        }
    }
}
