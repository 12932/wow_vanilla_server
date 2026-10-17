//! Vendors: their lists, buying (from limited stock too), selling, buying
//! back, and repairs.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::{
    BuyResult, BuybackSlot, ListInventoryItem, SMSG_BUY_FAILED, SMSG_BUY_ITEM, SMSG_LIST_INVENTORY,
    SMSG_SELL_ITEM, SellItemResult,
};

use crate::game::data::{GameData, ItemTemplate};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::combat::dist;
use crate::game::map::inventory::{self, BUYBACK_END, BUYBACK_START, EQUIPMENT_END, Fail};
use crate::game::store::BAG_PLAYER;

/// `UNIT_NPC_FLAGS`.
pub const NPC_FLAG_VENDOR: u32 = 0x4;
pub const NPC_FLAG_REPAIR: u32 = 0x4000;
/// How far a player may stand from an NPC to use it.
pub const NPC_YARDS: f32 = 8.0;

/// A vendor's stock of a limited item.
#[derive(Debug, Clone, Copy)]
pub struct Stock {
    pub count: u32,
    pub restock_at: Option<Instant>,
}

/// The vendor creature `npc`, if `player` may use it now.
fn vendor_entry(map: &Map, player: Guid, npc: Guid, flag: u32) -> Option<u32> {
    let p = map.players.get(&player)?;
    let c = map.creature(npc)?;
    if !c.alive() || c.fields.get_u32(UNIT_NPC_FLAGS) & flag == 0 {
        return None;
    }
    if dist(p.position(), c.position) > NPC_YARDS {
        return None;
    }
    Some(c.entry)
}

/// The price of `count` of an item, from its template's price for its buy
/// count.
fn price(t: &ItemTemplate, count: u32) -> u32 {
    let per = t.buy_count.max(1);
    t.buy_price.saturating_mul(count) / per
}

fn stock_left(map: &mut Map, npc: Guid, item: u32, max: u32, restock: u32) -> u32 {
    if max == 0 {
        return u32::MAX;
    }
    let now = map.now;
    let s = map.vendor_stock.entry((npc, item)).or_insert(Stock {
        count: max,
        restock_at: None,
    });
    if s.restock_at.is_some_and(|t| t <= now) {
        s.count = max;
        s.restock_at = None;
    }
    let _ = restock;
    s.count
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
        let left = stock_left(map, npc, v.item, v.max_count, v.restock_secs);
        items.push(ListInventoryItem {
            item_stack_count: i as u32 + 1,
            item: v.item,
            item_display_id: t.display_id,
            max_items: if left == u32::MAX { u32::MAX } else { left },
            price: crate::game::map::reputation::price(map, player, npc, price(t, t.buy_count.max(1)))
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
    if v.condition != 0 && !crate::game::map::conditions::met(map, v.condition, Some(npc), Some(player)) {
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
    let count = u32::from(amount.max(1)) * t.buy_count.max(1);
    let left = stock_left(map, npc, item, v.max_count, v.restock_secs);
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
    let mut new_left = u32::MAX;
    if v.max_count > 0
        && let Some(s) = map.vendor_stock.get_mut(&(npc, item))
    {
        s.count -= count;
        new_left = s.count;
        if s.restock_at.is_none() {
            s.restock_at = Some(map.now + Duration::from_secs(u64::from(v.restock_secs.max(60))));
        }
    }
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
    if it.bag == BAG_PLAYER
        && (it.slot < EQUIPMENT_END || (BUYBACK_START..BUYBACK_END).contains(&it.slot))
    {
        reply(map, SellItemResult::CantSellItem);
        return;
    }
    let Some(t) = data.item(it.entry) else {
        return;
    };
    if t.sell_price == 0 {
        reply(map, SellItemResult::CantSellItem);
        return;
    }
    let count = if amount == 0 {
        it.count()
    } else {
        u32::from(amount).min(it.count())
    };
    let gain = t.sell_price.saturating_mul(count);
    let whole = count == it.count();
    let money = p.money().saturating_add(gain);
    p.set_money(money);
    if whole {
        // Into the next buyback slot, round and round.
        let slot = BUYBACK_START + p.buyback_next % (BUYBACK_END - BUYBACK_START);
        p.buyback_next = p.buyback_next.wrapping_add(1);
        if let Some(old) = p.item_at(BAG_PLAYER, slot) {
            let gone = p.items.remove(old);
            p.outbound
                .send(&wow_world_messages::vanilla::SMSG_DESTROY_OBJECT { guid: gone.guid });
        }
        let Some(i) = p.item_by_guid(item) else {
            return;
        };
        p.items[i].bag = BAG_PLAYER;
        p.items[i].slot = slot;
        let n = u16::from(slot - BUYBACK_START);
        p.fields.set_u32(PLAYER_FIELD_BUYBACK_PRICE_1 + n, gain);
        p.fields.set_u32(
            PLAYER_FIELD_BUYBACK_TIMESTAMP_1 + n,
            u32::try_from(chrono::Utc::now().timestamp()).unwrap_or(0),
        );
        p.refresh_inventory(&data);
    } else {
        inventory::take_item(map, player, i, count);
    }
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
    p.refresh_inventory(&data);
}

/// What repairing `lost` durability of an item costs (vmangos's
/// `DurabilityRepair`).
pub fn repair_cost(data: &GameData, t: &ItemTemplate, lost: u32) -> u32 {
    let Some(costs) = data.durability_costs.get(&t.item_level) else {
        return 0;
    };
    let index = match t.class {
        2 => t.subclass as usize,
        4 => 21 + t.subclass as usize,
        _ => return 0,
    };
    let base = costs.get(index).copied().unwrap_or(0);
    let quality = data
        .durability_quality
        .get(&((t.quality + 1) * 2))
        .copied()
        .unwrap_or(1.0);
    (lost as f32 * base as f32 * quality) as u32
}

/// `CMSG_REPAIR_ITEM`: one item, or (with a zero guid) everything worn and
/// carried.
pub fn repair(map: &mut Map, player: Guid, npc: Guid, item: Guid) {
    if vendor_entry(map, player, npc, NPC_FLAG_REPAIR).is_none() {
        return;
    }
    let data = map.realm.data.clone();
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    let indexes: Vec<usize> = if item.is_zero() {
        (0..p.items.len()).collect()
    } else {
        p.item_by_guid(item).into_iter().collect()
    };
    for i in indexes {
        let it = &p.items[i];
        let (max, now) = (
            it.fields.get_u32(ITEM_FIELD_MAXDURABILITY),
            it.fields.get_u32(ITEM_FIELD_DURABILITY),
        );
        if max == 0 || now >= max {
            continue;
        }
        let Some(t) = data.item(it.entry) else {
            continue;
        };
        let cost = repair_cost(&data, t, max - now);
        if p.money() < cost {
            break;
        }
        let money = p.money() - cost;
        p.set_money(money);
        p.items[i].fields.set_u32(ITEM_FIELD_DURABILITY, max);
    }
}

/// Wear on everything worn: `fraction` of each item's full durability.
pub fn wear(map: &mut Map, player: Guid, fraction: f32) {
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    for it in p
        .items
        .iter_mut()
        .filter(|i| i.bag == BAG_PLAYER && i.slot < EQUIPMENT_END)
    {
        let max = it.fields.get_u32(ITEM_FIELD_MAXDURABILITY);
        if max == 0 {
            continue;
        }
        let now = it.fields.get_u32(ITEM_FIELD_DURABILITY);
        let loss = (max as f32 * fraction) as u32;
        it.fields
            .set_u32(ITEM_FIELD_DURABILITY, now.saturating_sub(loss));
    }
}
