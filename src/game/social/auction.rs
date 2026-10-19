//! The auction houses, as mangos-zero runs them: the Alliance's, the
//! Horde's and the goblins', with `AuctionHouse.dbc`'s deposits and cuts,
//! 2, 8 and 24 hour auctions, bids with their increments, buyouts,
//! cancelling, the searches, and settlement by mail. Each auction is kept
//! as a record.

use ahash::AHashMap;
use serde::{Deserialize, Serialize};
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::GameData;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::player::Item;
use crate::game::social::mail::{self, DAY, Mail};
use crate::game::social::send_to;
use crate::game::store::{ItemRow, Records};

pub const KIND_AUCTION: &str = "auction";
/// The shortest auction, which deposits are counted in.
pub const MIN_AUCTION_SECS: u32 = 2 * 3600;
pub const PAGE: usize = 50;
/// The most an auction may ask: mangos's `MAX_MONEY_AMOUNT`.
const MAX_PRICE: u32 = 0x7FFF_FFFE;

// The mail an auction sends: `item:0:answer`.
const OUTBID: u32 = 0;
const WON: u32 = 1;
const SOLD: u32 = 2;
const EXPIRED: u32 = 3;
const CANCELLED_TO_BIDDER: u32 = 4;
const CANCELLED: u32 = 5;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Auction {
    pub id: u32,
    /// The market: 2 the Alliance's, 6 the Horde's, 7 the goblins'.
    pub market: u32,
    /// The house it was put up at, for its deposit and cut.
    pub house: u32,
    pub item: ItemRow,
    pub owner: u32,
    pub start_bid: u32,
    pub buyout: u32,
    pub bid: u32,
    pub bidder: u32,
    pub deposit: u32,
    /// Unix seconds.
    pub expires_at: i64,
}

/// The random property of an auction's item, which its notices name.
fn random_property(realm: &Realm, item: &ItemRow) -> u32 {
    Item::shown(&realm.data, item).random_property
}

impl Auction {
    /// The least a new bid must add.
    pub fn outbid(&self) -> u32 {
        ((self.bid / 100) * 5).max(1)
    }

    fn list_item(&self, data: &GameData) -> AuctionListItem {
        let now = mail::now();
        let shown = Item::shown(data, &self.item);
        AuctionListItem {
            id: self.id,
            item: self.item.entry,
            item_enchantment: shown.enchantment,
            item_random_property_id: shown.random_property,
            item_suffix_factor: shown.suffix_factor,
            item_count: self.item.count,
            item_charges: shown.charges,
            item_owner: guid::player(self.owner),
            start_bid: self.start_bid,
            minimum_bid: if self.bid > 0 { self.outbid() } else { 0 },
            buyout_amount: self.buyout,
            time_left: std::time::Duration::from_secs((self.expires_at - now).max(0) as u64),
            highest_bidder: if self.bidder > 0 {
                guid::player(self.bidder)
            } else {
                Guid::zero()
            },
            highest_bid: self.bid,
        }
    }
}

#[derive(Debug, Default)]
pub struct Auctions {
    pub auctions: AHashMap<u32, Auction>,
    next_id: u32,
}

impl Auctions {
    pub fn load(records: &[(String, u64, String)]) -> Self {
        let mut a = Self::default();
        for (kind, _, data) in records.iter().filter(|r| r.0 == KIND_AUCTION) {
            let _ = kind;
            match serde_json::from_str::<Auction>(data) {
                Ok(auction) => {
                    a.next_id = a.next_id.max(auction.id);
                    a.auctions.insert(auction.id, auction);
                }
                Err(e) => tracing::warn!("an auction record doesn't read: {e}"),
            }
        }
        a
    }

    pub fn max_item(&self) -> u32 {
        self.auctions
            .values()
            .map(|a| a.item.guid)
            .max()
            .unwrap_or(0)
    }
}

/// The market a house sells in: its faction's, or the goblins'.
pub fn market_of(house: AuctionHouse) -> u32 {
    match house {
        AuctionHouse::Stormwind | AuctionHouse::Alliance | AuctionHouse::Darnassus => 2,
        AuctionHouse::Undercity | AuctionHouse::ThunderBluff | AuctionHouse::Horde => 6,
        AuctionHouse::Goblin => 7,
    }
}

/// `AuctionHouse.dbc`: the deposit and the cut, in percent.
pub fn rates(house: u32) -> (u32, u32) {
    if house == 7 { (25, 15) } else { (5, 5) }
}

/// The house's cut of an auction's bid, worked out wide: a bid of 28,634
/// gold times the goblins' 15 is past a `u32` (mangos's `GetAuctionCut`
/// works it out in floats).
fn cut_of(a: &Auction) -> u32 {
    (u64::from(a.bid) * u64::from(rates(a.house).1) / 100) as u32
}

/// Thistle Tea's deposit (`Auction.deposit`): the item's sell price, by
/// the 2-hour periods, at the house's rate.
pub fn deposit(sell_price: u32, count: u32, secs: u32, house: u32) -> u32 {
    let periods = u64::from(secs / MIN_AUCTION_SECS);
    let base = u64::from(sell_price) * u64::from(count) * periods * u64::from(rates(house).0);
    u32::try_from(base / 100).unwrap_or(u32::MAX)
}

fn save(realm: &Realm, a: &Auction) {
    realm.store.put_record(KIND_AUCTION, u64::from(a.id), a);
}

fn subject(a: &Auction, answer: u32) -> String {
    format!("{}:0:{answer}", a.item.entry)
}

/// A mail from the auction house.
fn mail_from(
    a: &Auction,
    to: u32,
    answer: u32,
    item: Option<ItemRow>,
    money: u32,
    delay: i64,
) -> Mail {
    let now = mail::now();
    Mail {
        kind: mail::AUCTION,
        sender: a.house,
        receiver: to,
        subject: subject(a, answer),
        item,
        money,
        deliver_at: now + delay,
        expires_at: now + delay + 30 * DAY,
        auction: a.id,
        ..Default::default()
    }
}

type R2 = SMSG_AUCTION_COMMAND_RESULT_AuctionCommandResultTwo;
type R = SMSG_AUCTION_COMMAND_RESULT_AuctionCommandResult;

fn command_result(
    map: &Map,
    g: Guid,
    id: u32,
    action: SMSG_AUCTION_COMMAND_RESULT_AuctionCommandAction,
) {
    if let Some(p) = map.players.get(&g) {
        p.outbound.send(&SMSG_AUCTION_COMMAND_RESULT {
            auction_id: id,
            action,
        });
    }
}

/// The house an auctioneer near the player is of, if they may use it now
/// (the services' shared checks, as Thistle Tea checks each auction).
fn house_at(map: &Map, g: Guid, auctioneer: Guid) -> Option<AuctionHouse> {
    crate::game::map::npc::npc_entry(
        map,
        g,
        auctioneer,
        crate::game::map::npc::NPC_FLAG_AUCTIONEER,
    )?;
    Some(crate::game::map::auction::house_of(map, auctioneer))
}

/// `CMSG_AUCTION_SELL_ITEM`.
pub fn sell(map: &mut Map, g: Guid, m: &CMSG_AUCTION_SELL_ITEM) {
    let realm = map.realm.clone();
    let Some(house) = house_at(map, g, m.auctioneer) else {
        return;
    };
    let fail = |map: &Map, r: R2| {
        command_result(
            map,
            g,
            0,
            SMSG_AUCTION_COMMAND_RESULT_AuctionCommandAction::Started { result2: r },
        );
    };
    let secs = m.auction_duration_in_minutes * 60;
    if !matches!(m.auction_duration_in_minutes, 120 | 480 | 1440) || m.starting_bid == 0 {
        fail(map, R2::ErrDatabase);
        return;
    }
    // No price past the most a player may hold (mangos's
    // `MAX_MONEY_AMOUNT`), nor a buyout under the bid.
    if m.starting_bid > MAX_PRICE
        || m.buyout > MAX_PRICE
        || (m.buyout > 0 && m.buyout < m.starting_bid)
    {
        fail(map, R2::ErrDatabase);
        return;
    }
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let Some(i) = p.item_by_guid(m.item) else {
        fail(map, R2::ErrItemNotFound);
        return;
    };
    let item = &p.items[i];
    // Carried, not worn nor banked; not bound, conjured, timed, nor a bag
    // with something in it (mangos's `HandleAuctionSellItem`, Thistle Tea's
    // `validate_sale`), as mangos tells it.
    let bound = crate::game::map::items::bound_now(&realm.data, item);
    let worn = item.bag == crate::game::store::BAG_PLAYER && item.slot < 19;
    let conjured = realm
        .data
        .item(item.entry)
        .is_some_and(|t| t.flags & crate::game::map::items::TEMPLATE_FLAG_CONJURED != 0);
    let timed = item
        .fields
        .get_u32(crate::game::fields::ITEM_FIELD_DURATION)
        != 0;
    let full_bag =
        item.bag == crate::game::store::BAG_PLAYER && p.items.iter().any(|x| x.bag == item.slot);
    if bound || worn || !item.carried() || conjured || timed || full_bag {
        fail(
            map,
            R2::ErrInventory {
                inventory_result2: InventoryResult::ItemNotFound,
            },
        );
        return;
    }
    let count = item.count();
    let sell_price = realm.data.item(item.entry).map_or(0, |t| t.sell_price);
    let house_id = house.as_int();
    let deposit = deposit(sell_price, count, secs, house_id);
    if p.money() < deposit {
        fail(map, R2::ErrNotEnoughMoney);
        return;
    }
    let p = map.players.get_mut(&g).expect("checked above");
    let money = p.money();
    p.set_money(money - deposit);
    let Some(row) = crate::game::map::inventory::detach_item(map, g, i) else {
        return;
    };
    let mut auctions = realm.social.auctions.lock();
    auctions.next_id += 1;
    let a = Auction {
        id: auctions.next_id,
        market: market_of(house),
        house: house_id,
        item: row,
        owner: guid::counter(g),
        start_bid: m.starting_bid,
        buyout: m.buyout,
        bid: 0,
        bidder: 0,
        deposit,
        expires_at: mail::now() + i64::from(secs),
    };
    let mut records = Records::new();
    records.put(KIND_AUCTION, u64::from(a.id), &a);
    let id = a.id;
    auctions.auctions.insert(id, a);
    // The auction with its item, and the seller without it or the
    // deposit, in one transaction (mangos's `HandleAuctionSellItem`), sent
    // under the auctions' lock so that no later change to it is written
    // first.
    map.save_together(&[g], records);
    drop(auctions);
    command_result(
        map,
        g,
        id,
        SMSG_AUCTION_COMMAND_RESULT_AuctionCommandAction::Started { result2: R2::Ok },
    );
}

/// The auction won: the item to the winner, the money to the seller.
/// The mails and the auction's end go in `records`, to be saved together.
fn settle(realm: &Realm, a: &Auction, records: &mut Records) {
    let cut = cut_of(a);
    mail::post_in(
        realm,
        mail_from(a, a.bidder, WON, Some(a.item.clone()), 0, 0),
        records,
    );
    let delay = realm.config().mail_delay.as_secs() as i64;
    mail::post_in(
        realm,
        mail_from(
            a,
            a.owner,
            SOLD,
            None,
            (a.bid - cut).saturating_add(a.deposit),
            delay,
        ),
        records,
    );
    // The winner is told they won (a bid of 0), the seller that it sold
    // (no bidder named), as mangos-classic's `SendAuctionBidderNotification`
    // and `SendAuctionOwnerNotification` tell them: the client shows a
    // named bidder's notice as a bid, and a bid's as an outbid.
    send_to(
        realm,
        guid::player(a.bidder),
        &SMSG_AUCTION_BIDDER_NOTIFICATION {
            auction_house: AuctionHouse::try_from(a.house).unwrap_or(AuctionHouse::Goblin),
            auction_id: a.id,
            bidder: guid::player(a.bidder),
            won: 0,
            out_bid: a.outbid(),
            item_template: a.item.entry,
            item_random_property_id: random_property(realm, &a.item),
        },
    );
    send_to(
        realm,
        guid::player(a.owner),
        &SMSG_AUCTION_OWNER_NOTIFICATION {
            auction_id: a.id,
            bid: a.bid,
            auction_out_bid: 0,
            bidder: Guid::zero(),
            item: a.item.entry,
            item_random_property_id: random_property(realm, &a.item),
        },
    );
    records.delete(KIND_AUCTION, u64::from(a.id));
}

/// `CMSG_AUCTION_PLACE_BID`: a bid, or the buyout.
pub fn bid(map: &mut Map, g: Guid, m: &CMSG_AUCTION_PLACE_BID) {
    let realm = map.realm.clone();
    let Some(house) = house_at(map, g, m.auctioneer) else {
        return;
    };
    let price = m.price.as_int();
    let me = guid::counter(g);
    let fail = |map: &Map, r: R| {
        command_result(
            map,
            g,
            m.auction_id,
            SMSG_AUCTION_COMMAND_RESULT_AuctionCommandAction::BidPlaced { result: r },
        );
    };
    let mut auctions = realm.social.auctions.lock();
    let Some(a) = auctions
        .auctions
        .get_mut(&m.auction_id)
        .filter(|a| a.market == market_of(house))
    else {
        drop(auctions);
        fail(map, R::ErrItemNotFound);
        return;
    };
    // Not one's own, nor another of one's account's characters'
    // (mangos's `HandleAuctionPlaceBid`).
    let account = |c: u32| {
        realm
            .registry
            .name(guid::player(c))
            .map(|n| n.account.clone())
    };
    if a.owner == me || account(a.owner).is_some_and(|o| Some(o) == account(me)) {
        drop(auctions);
        fail(map, R::ErrBidOwn);
        return;
    }
    // No more than the bid there is, as mangos tells it.
    if a.bid > 0 && price <= a.bid {
        let higher = R::ErrHigherBid {
            auction_outbid2: a.outbid(),
            higher_bidder: guid::player(a.bidder),
            new_bid: a.bid,
        };
        drop(auctions);
        fail(map, higher);
        return;
    }
    let least = if a.bid > 0 {
        a.bid.saturating_add(a.outbid())
    } else {
        a.start_bid
    };
    let buyout = a.buyout > 0 && price >= a.buyout;
    if price < least && !buyout {
        drop(auctions);
        fail(map, R::ErrBidIncrement);
        return;
    }
    let price = if buyout { a.buyout } else { price };
    // A raise on one's own bid costs the difference.
    let cost = if a.bidder == me { price - a.bid } else { price };
    if map.players.get(&g).is_none_or(|p| p.money() < cost) {
        drop(auctions);
        fail(map, R::ErrNotEnoughMoney);
        return;
    }
    let mut records = Records::new();
    if a.bidder != 0 && a.bidder != me {
        // The one outbid gets their money back.
        mail::post_in(
            &realm,
            mail_from(a, a.bidder, OUTBID, None, a.bid, 0),
            &mut records,
        );
        send_to(
            &realm,
            guid::player(a.bidder),
            &SMSG_AUCTION_BIDDER_NOTIFICATION {
                auction_house: house,
                auction_id: a.id,
                bidder: g,
                // The bid lost: not 0, which the client shows as won.
                won: a.bid,
                out_bid: a.outbid(),
                item_template: a.item.entry,
                item_random_property_id: random_property(&realm, &a.item),
            },
        );
    }
    a.bid = price;
    a.bidder = me;
    let id = a.id;
    let outbid = a.outbid();
    let done = buyout.then(|| a.clone());
    if let Some(a) = &done {
        auctions.auctions.remove(&a.id);
        settle(&realm, a, &mut records);
    } else {
        records.put(KIND_AUCTION, u64::from(a.id), a);
        send_to(
            &realm,
            guid::player(a.owner),
            &SMSG_AUCTION_OWNER_NOTIFICATION {
                auction_id: a.id,
                bid: a.bid,
                auction_out_bid: outbid,
                bidder: g,
                item: a.item.entry,
                item_random_property_id: random_property(&realm, &a.item),
            },
        );
    }
    if let Some(p) = map.players.get_mut(&g) {
        let money = p.money();
        p.set_money(money - cost);
    }
    // The bidder's money, the auction (or its settling mails) and the
    // outbid's refund in one transaction (mangos's
    // `HandleAuctionPlaceBid`), under the auctions' lock.
    map.save_together(&[g], records);
    drop(auctions);
    fail(
        map,
        R::Ok {
            auction_outbid1: outbid,
        },
    );
    let _ = id;
}

/// `CMSG_AUCTION_REMOVE_ITEM`: the seller takes it back. With a bid on it,
/// the seller pays the cut and the bidder is paid back.
pub fn cancel(map: &mut Map, g: Guid, m: &CMSG_AUCTION_REMOVE_ITEM) {
    let realm = map.realm.clone();
    // At an auctioneer, as any other auction command (mangos's
    // `HandleAuctionRemoveItem`).
    if house_at(map, g, m.auctioneer).is_none() {
        return;
    }
    let mut auctions = realm.social.auctions.lock();
    let Some(a) = auctions
        .auctions
        .get(&m.auction_id)
        .filter(|a| a.owner == guid::counter(g))
        .cloned()
    else {
        return;
    };
    let fee = if a.bidder > 0 { cut_of(&a) } else { 0 };
    if map.players.get(&g).is_none_or(|p| p.money() < fee) {
        drop(auctions);
        command_result(
            map,
            g,
            a.id,
            SMSG_AUCTION_COMMAND_RESULT_AuctionCommandAction::Removed {
                result2: R2::ErrNotEnoughMoney,
            },
        );
        return;
    }
    auctions.auctions.remove(&a.id);
    let mut records = Records::new();
    records.delete(KIND_AUCTION, u64::from(a.id));
    if let Some(p) = map.players.get_mut(&g) {
        let money = p.money();
        p.set_money(money - fee);
    }
    if a.bidder > 0 {
        mail::post_in(
            &realm,
            mail_from(&a, a.bidder, CANCELLED_TO_BIDDER, None, a.bid, 0),
            &mut records,
        );
    }
    mail::post_in(
        &realm,
        mail_from(&a, a.owner, CANCELLED, Some(a.item.clone()), 0, 0),
        &mut records,
    );
    // The auction gone, its item mailed back, the bid refunded and the
    // seller's fee in one transaction (mangos's `HandleAuctionRemoveItem`).
    map.save_together(&[g], records);
    drop(auctions);
    command_result(
        map,
        g,
        a.id,
        SMSG_AUCTION_COMMAND_RESULT_AuctionCommandAction::Removed { result2: R2::Ok },
    );
}

/// Inventory types: a chest, and a robe (a chest's).
const INVTYPE_CHEST: u32 = 5;
const INVTYPE_ROBE: u32 = 20;

/// `CMSG_AUCTION_LIST_ITEMS`: a page of the market's auctions by name,
/// level, slot, class, subclass and quality.
pub fn list(map: &Map, g: Guid, m: &CMSG_AUCTION_LIST_ITEMS) {
    let realm = &map.realm;
    let Some(house) = house_at(map, g, m.auctioneer) else {
        return;
    };
    let Some(me) = map.players.get(&g) else {
        return;
    };
    let market = market_of(house);
    let name = m.searched_name.to_lowercase();
    let now = mail::now();
    let auctions = realm.social.auctions.lock();
    let mut found: Vec<&Auction> = auctions
        .auctions
        .values()
        .filter(|a| a.market == market && a.expires_at > now)
        .filter(|a| {
            let Some(t) = realm.data.item(a.item.entry) else {
                return false;
            };
            // Its name with its suffix, "Silk Robe of the Eagle" (Thistle
            // Tea's `Query`).
            let full = match realm
                .data
                .crafts
                .random_property_names
                .get(&Item::shown(&realm.data, &a.item).random_property)
            {
                Some(suffix) => format!("{} {}", t.name, suffix),
                None => t.name.clone(),
            };
            (name.is_empty() || full.to_lowercase().contains(&name))
                && (m.usable == 0 || usable_by(me, &realm.data, t))
                && (m.minimum_level == 0 || t.required_level >= u32::from(m.minimum_level))
                && (m.maximum_level == 0 || t.required_level <= u32::from(m.maximum_level))
                && (m.auction_slot_id == u32::MAX
                    || t.inventory_type == m.auction_slot_id
                    // The chest's slot finds the robes too (mangos's
                    // `BuildListAuctionItems`, Thistle Tea's `Query`).
                    || (m.auction_slot_id == INVTYPE_CHEST && t.inventory_type == INVTYPE_ROBE))
                && (m.auction_main_category == u32::MAX || t.class == m.auction_main_category)
                && (m.auction_sub_category == u32::MAX || t.subclass == m.auction_sub_category)
                && t.quality >= m.auction_quality.as_int() as u32
        })
        .collect();
    found.sort_by_key(|a| a.id);
    let total = found.len() as u32;
    let page = found
        .into_iter()
        .skip(m.list_start_item as usize)
        .take(PAGE)
        .map(|a| a.list_item(&realm.data))
        .collect();
    if let Some(p) = map.players.get(&g) {
        p.outbound.send(&SMSG_AUCTION_LIST_RESULT {
            auctions: page,
            total_amount_of_auctions: total,
        });
    }
}

/// Whether `p` could use `t`, for the usable filter (Thistle Tea's
/// `Auction.Eligibility`): the level, class, race and proficiency, its
/// spell known, and a recipe not learned already.
fn usable_by(
    p: &crate::game::map::player::Player,
    data: &GameData,
    t: &crate::game::data::ItemTemplate,
) -> bool {
    // A recipe's second spell is what it teaches (its first, Learning, 483,
    // teaches it), or a spell of it teaches one.
    const RECIPE: u32 = 9;
    const LEARNING: u32 = 483;
    let teaches = |id: u32| {
        std::iter::once(id).chain(
            data.spells
                .get(id)
                .into_iter()
                .flat_map(|s| s.effects.iter().map(|e| e.trigger_spell)),
        )
    };
    let learned = t.class == RECIPE
        && t.spells
            .iter()
            .filter(|s| s.id != 0 && s.id != LEARNING)
            .flat_map(|s| teaches(s.id))
            .any(|taught| taught != 0 && p.spells.contains(&taught));
    u32::from(p.level()) >= t.required_level
        && crate::game::map::loot::usable(p, data, t)
        && (t.required_spell == 0 || p.spells.contains(&t.required_spell))
        && !learned
}

/// `CMSG_AUCTION_LIST_OWNER_ITEMS` and `CMSG_AUCTION_LIST_BIDDER_ITEMS`:
/// the player's auctions, or those they hold the bid on and those they ask
/// after, outbid (mangos's `HandleAuctionListBidderItems`).
pub fn list_mine(map: &Map, g: Guid, auctioneer: Guid, bids: bool, outbid: &[u32]) {
    let realm = &map.realm;
    let Some(house) = house_at(map, g, auctioneer) else {
        return;
    };
    let me = guid::counter(g);
    let market = market_of(house);
    let auctions = realm.social.auctions.lock();
    let mut asked: Vec<u32> = outbid.to_vec();
    asked.sort_unstable();
    asked.dedup();
    let mine: Vec<AuctionListItem> = auctions
        .auctions
        .values()
        .filter(|a| {
            a.market == market
                && if bids {
                    a.bidder == me || asked.contains(&a.id)
                } else {
                    a.owner == me
                }
        })
        .map(|a| a.list_item(&realm.data))
        .collect();
    let total = mine.len() as u32;
    let Some(p) = map.players.get(&g) else {
        return;
    };
    if bids {
        p.outbound.send(&SMSG_AUCTION_BIDDER_LIST_RESULT {
            auctions: mine,
            total_amount_of_auctions: total,
        });
    } else {
        p.outbound.send(&SMSG_AUCTION_OWNER_LIST_RESULT {
            auctions: mine,
            total_amount_of_auctions: total,
        });
    }
}

/// Auctions that have run out: won ones are settled, the rest go back.
pub fn sweep(realm: &Realm) {
    let now = mail::now();
    let done: Vec<Auction> = {
        let mut auctions = realm.social.auctions.lock();
        let ids: Vec<u32> = auctions
            .auctions
            .values()
            .filter(|a| a.expires_at <= now)
            .map(|a| a.id)
            .collect();
        ids.iter()
            .filter_map(|id| auctions.auctions.remove(id))
            .collect()
    };
    // Each auction's end and the mails it sends in one transaction: apart,
    // a crash between them would keep the item in both, or in neither.
    for a in done {
        let mut records = Records::new();
        if a.bidder > 0 {
            settle(realm, &a, &mut records);
        } else {
            mail::post_in(
                realm,
                mail_from(&a, a.owner, EXPIRED, Some(a.item.clone()), 0, 0),
                &mut records,
            );
            send_to(
                realm,
                guid::player(a.owner),
                &SMSG_AUCTION_REMOVED_NOTIFICATION {
                    item: a.id,
                    item_template: a.item.entry,
                    random_property_id: random_property(realm, &a.item),
                },
            );
            records.delete(KIND_AUCTION, u64::from(a.id));
        }
        realm.store.save_together(Vec::new(), records);
    }
}

/// A character deleted: their auctions go, and their bids with them.
pub fn character_deleted(realm: &Realm, counter: u32) {
    let mut auctions = realm.social.auctions.lock();
    let owned: Vec<u32> = auctions
        .auctions
        .values()
        .filter(|a| a.owner == counter)
        .map(|a| a.id)
        .collect();
    for id in owned {
        let Some(a) = auctions.auctions.remove(&id) else {
            continue;
        };
        // A bid on it is paid back, as a cancel pays it, in one
        // transaction with the auction's end.
        let mut records = Records::new();
        records.delete(KIND_AUCTION, u64::from(id));
        if a.bidder > 0 {
            mail::post_in(
                realm,
                mail_from(&a, a.bidder, CANCELLED_TO_BIDDER, None, a.bid, 0),
                &mut records,
            );
        }
        realm.store.save_together(Vec::new(), records);
    }
    for a in auctions
        .auctions
        .values_mut()
        .filter(|a| a.bidder == counter)
    {
        a.bidder = 0;
        a.bid = 0;
        save(realm, a);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deposits_and_increments_follow_thistle_tea() {
        // A 1s item, two hours at a faction house: 5% of its price.
        assert_eq!(deposit(100, 1, 7200, 2), 5);
        // Twenty-four hours: twelve periods.
        assert_eq!(deposit(100, 1, 86_400, 2), 60);
        // The goblins take five times as much.
        assert_eq!(deposit(100, 1, 7200, 7), 25);
        let a = Auction {
            bid: 1000,
            ..Default::default()
        };
        assert_eq!(a.outbid(), 50);
        assert_eq!(
            Auction {
                bid: 10,
                ..Default::default()
            }
            .outbid(),
            1
        );
    }
}
