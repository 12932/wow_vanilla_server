//! Trade between two players near each other, as mangos-zero runs it: six
//! slots and money each (a seventh slot shows what isn't traded), both
//! accept, and everything changes hands at once or nothing does.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::combat::dist;
use crate::game::map::death::Death;

pub const SLOTS: usize = 7;
/// The slot whose item stays with its owner.
pub const NOT_TRADED: usize = 6;
pub const TRADE_YARDS: f32 = 11.0;

/// One side of a trade.
#[derive(Debug, Clone, Default)]
pub struct Trade {
    pub with: Guid,
    /// The window is open on both sides.
    pub open: bool,
    pub items: [Option<Guid>; SLOTS],
    pub money: u32,
    pub accepted: bool,
}

fn status(map: &Map, g: Guid, status: SMSG_TRADE_STATUS_TradeStatus) {
    if let Some(p) = map.players.get(&g) {
        p.outbound.send(&SMSG_TRADE_STATUS { status });
    }
}

fn partner(map: &Map, g: Guid) -> Option<Guid> {
    map.players.get(&g)?.trade.as_ref().map(|t| t.with)
}

/// `CMSG_INITIATE_TRADE`.
pub fn initiate(map: &mut Map, g: Guid, with: Guid) {
    type S = SMSG_TRADE_STATUS_TradeStatus;
    let Some(me) = map.players.get(&g) else {
        return;
    };
    if me.trade.is_some() {
        return;
    }
    if me.death != Death::Alive {
        status(map, g, S::YouDead);
        return;
    }
    let Some(them) = map.players.get(&with) else {
        status(map, g, S::NoTarget);
        return;
    };
    if them.trade.is_some() {
        status(map, g, S::Busy);
        return;
    }
    if them.death != Death::Alive {
        status(map, g, S::TargetDead);
        return;
    }
    if them.team != me.team {
        status(map, g, S::WrongFaction);
        return;
    }
    if dist(me.position(), them.position()) > TRADE_YARDS {
        status(map, g, S::TargetToFar);
        return;
    }
    if them.logout_at.is_some() {
        status(map, g, S::TargetLogout);
        return;
    }
    if map.realm.social.lists.lock().ignores(with, g) {
        status(map, g, S::IgnoreYou);
        return;
    }
    for (a, b) in [(g, with), (with, g)] {
        if let Some(p) = map.players.get_mut(&a) {
            p.trade = Some(Trade {
                with: b,
                ..Default::default()
            });
        }
    }
    status(map, with, S::BeginTrade { unknown1: g });
}

/// `CMSG_BEGIN_TRADE`: the window opens on both sides.
pub fn begin(map: &mut Map, g: Guid) {
    let Some(with) = partner(map, g) else {
        return;
    };
    for p in [g, with] {
        if let Some(t) = map.players.get_mut(&p).and_then(|p| p.trade.as_mut()) {
            t.open = true;
        }
        status(map, p, SMSG_TRADE_STATUS_TradeStatus::OpenWindow);
    }
}

/// The trade ends without anything changing hands.
pub fn cancel(map: &mut Map, g: Guid, why: SMSG_TRADE_STATUS_TradeStatus) {
    let Some(with) = partner(map, g) else {
        return;
    };
    for p in [g, with] {
        if let Some(p) = map.players.get_mut(&p) {
            p.trade = None;
        }
    }
    status(map, g, SMSG_TRADE_STATUS_TradeStatus::TradeCanceled);
    status(map, with, why);
}

/// What `g` offers, as their partner's window shows it.
fn offer(map: &Map, g: Guid) -> Option<SMSG_TRADE_STATUS_EXTENDED> {
    let p = map.players.get(&g)?;
    let t = p.trade.as_ref()?;
    let data = &map.realm.data;
    let trade_slots = std::array::from_fn(|i| {
        let item = t.items[i]
            .and_then(|it| p.item_by_guid(it))
            .map(|i| &p.items[i]);
        match item {
            Some(item) => TradeSlot {
                trade_slot_number: i as u8,
                item: item.entry,
                display_id: data.item(item.entry).map_or(0, |t| t.display_id),
                stack_count: item.count(),
                wrapped: item.fields.get_u32(ITEM_FIELD_FLAGS) & 0x8 != 0,
                gift_wrapper: item.fields.get_guid(ITEM_FIELD_GIFTCREATOR),
                enchantment: 0,
                item_creator: item.fields.get_guid(ITEM_FIELD_CREATOR),
                spell_charges: 0,
                item_suffix_factor: 0,
                item_random_properties_id: 0,
                lock_id: data.item(item.entry).map_or(0, |t| t.lock_id),
                max_durability: item.fields.get_u32(ITEM_FIELD_MAXDURABILITY),
                durability: item.fields.get_u32(ITEM_FIELD_DURABILITY),
            },
            None => TradeSlot {
                trade_slot_number: i as u8,
                ..Default::default()
            },
        }
    });
    Some(SMSG_TRADE_STATUS_EXTENDED {
        self_player: true,
        trade_slot_count1: SLOTS as u32,
        trade_slot_count2: SLOTS as u32,
        money_in_trade: Gold::new(t.money),
        spell_on_lowest_slot: 0,
        trade_slots,
    })
}

/// A change to `g`'s offer: neither side has accepted it, and the partner
/// sees it.
fn changed(map: &mut Map, g: Guid) {
    let Some(with) = partner(map, g) else {
        return;
    };
    let was_accepted = [g, with].iter().any(|p| {
        map.players
            .get(p)
            .and_then(|p| p.trade.as_ref())
            .is_some_and(|t| t.accepted)
    });
    for p in [g, with] {
        if let Some(t) = map.players.get_mut(&p).and_then(|p| p.trade.as_mut()) {
            t.accepted = false;
        }
    }
    if was_accepted {
        status(map, g, SMSG_TRADE_STATUS_TradeStatus::BackToTrade);
        status(map, with, SMSG_TRADE_STATUS_TradeStatus::BackToTrade);
    }
    if let (Some(o), Some(p)) = (offer(map, g), map.players.get(&with)) {
        p.outbound.send(&o);
    }
}

/// `CMSG_SET_TRADE_ITEM`.
pub fn set_item(map: &mut Map, g: Guid, slot: u8, bag: u8, bag_slot: u8) {
    let slot = usize::from(slot);
    let Some(p) = map.players.get(&g) else {
        return;
    };
    if slot >= SLOTS || p.trade.is_none() {
        return;
    }
    let Some(i) = p.item_at(bag, bag_slot) else {
        return;
    };
    let item = &p.items[i];
    let bound = item.fields.get_u32(ITEM_FIELD_FLAGS) & 0x1 != 0;
    let worn = item.bag == crate::game::store::BAG_PLAYER && item.slot < 19;
    let guid = item.guid;
    if (bound && slot != NOT_TRADED) || worn {
        return;
    }
    let p = map.players.get_mut(&g).expect("checked above");
    let t = p.trade.as_mut().expect("checked above");
    if t.items.contains(&Some(guid)) {
        return;
    }
    t.items[slot] = Some(guid);
    changed(map, g);
}

/// `CMSG_CLEAR_TRADE_ITEM`.
pub fn clear_item(map: &mut Map, g: Guid, slot: u8) {
    if let Some(t) = map.players.get_mut(&g).and_then(|p| p.trade.as_mut())
        && let Some(s) = t.items.get_mut(usize::from(slot))
    {
        *s = None;
        changed(map, g);
    }
}

/// `CMSG_SET_TRADE_GOLD`.
pub fn set_money(map: &mut Map, g: Guid, money: u32) {
    if let Some(t) = map.players.get_mut(&g).and_then(|p| p.trade.as_mut()) {
        t.money = money;
        changed(map, g);
    }
}

/// `CMSG_UNACCEPT_TRADE`.
pub fn unaccept(map: &mut Map, g: Guid) {
    let Some(with) = partner(map, g) else {
        return;
    };
    if let Some(t) = map.players.get_mut(&g).and_then(|p| p.trade.as_mut()) {
        t.accepted = false;
    }
    status(map, with, SMSG_TRADE_STATUS_TradeStatus::BackToTrade);
}

/// The items `g` trades away, by inventory index.
fn traded(map: &Map, g: Guid) -> Vec<usize> {
    let Some(p) = map.players.get(&g) else {
        return Vec::new();
    };
    let Some(t) = p.trade.as_ref() else {
        return Vec::new();
    };
    t.items[..NOT_TRADED]
        .iter()
        .flatten()
        .filter_map(|it| p.item_by_guid(*it))
        .collect()
}

/// `CMSG_ACCEPT_TRADE`: when both have, it goes through.
pub fn accept(map: &mut Map, g: Guid) {
    type S = SMSG_TRADE_STATUS_TradeStatus;
    let Some(with) = partner(map, g) else {
        return;
    };
    if let Some(t) = map.players.get_mut(&g).and_then(|p| p.trade.as_mut()) {
        t.accepted = true;
    }
    let both = map
        .players
        .get(&with)
        .and_then(|p| p.trade.as_ref())
        .is_some_and(|t| t.accepted);
    if !both {
        status(map, with, S::TradeAccept);
        return;
    }
    let data = map.realm.data.clone();
    // Each side can pay, is near, and has room for what comes.
    let near = match (map.players.get(&g), map.players.get(&with)) {
        (Some(a), Some(b)) => dist(a.position(), b.position()) <= TRADE_YARDS,
        _ => false,
    };
    if !near {
        cancel(map, g, S::TargetToFar);
        return;
    }
    let (mine, theirs) = (traded(map, g), traded(map, with));
    for (side, give, get) in [(g, &mine, &theirs), (with, &theirs, &mine)] {
        let Some(p) = map.players.get(&side) else {
            return;
        };
        let money = p.trade.as_ref().map_or(0, |t| t.money);
        let free = p
            .carry_slots(&data)
            .into_iter()
            .filter(|(b, s)| p.item_at(*b, *s).is_none())
            .count();
        if p.money() < money || free + give.len() < get.len() {
            for s in [g, with] {
                if let Some(t) = map.players.get_mut(&s).and_then(|p| p.trade.as_mut()) {
                    t.accepted = false;
                }
                status(map, s, S::BackToTrade);
            }
            return;
        }
    }
    // Take both sides' items out (highest index first), then hand them over.
    let take = |map: &mut Map, side: Guid, mut idx: Vec<usize>| {
        idx.sort_unstable_by(|a, b| b.cmp(a));
        idx.into_iter()
            .filter_map(|i| crate::game::map::inventory::detach_item(map, side, i))
            .collect::<Vec<_>>()
    };
    let to_them = take(map, g, mine);
    let to_me = take(map, with, theirs);
    for row in &to_them {
        let _ = crate::game::map::inventory::attach_item(map, with, row);
    }
    for row in &to_me {
        let _ = crate::game::map::inventory::attach_item(map, g, row);
    }
    let money = |map: &Map, s: Guid| {
        map.players
            .get(&s)
            .and_then(|p| p.trade.as_ref())
            .map_or(0, |t| t.money)
    };
    let (m1, m2) = (money(map, g), money(map, with));
    for (s, out, inn) in [(g, m1, m2), (with, m2, m1)] {
        if let Some(p) = map.players.get_mut(&s) {
            let have = p.money();
            p.set_money(have - out + inn);
            p.trade = None;
        }
        status(map, s, S::TradeComplete);
    }
}
