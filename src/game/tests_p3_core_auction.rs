//! Phase 3's auctions, Thistle Tea's `AuctionTest` ported: a sale's terms,
//! bids on one's own (one's account's) auctions, bids too low or too poor,
//! the first bid held and a raise charged its difference, the outbid paid
//! back, a bidder's buyout paying the balance, cancelling with and without
//! a bid, and settling at the deadline.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::inventory::PACK_START;
use crate::game::map::{Map, MapKey};
use crate::game::social::mail::Mail;
use crate::game::store::BAG_PLAYER;

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const AUCTIONEER: u32 = 3170;
/// Sells for 100 each: three for two hours at the goblins' 25% leave 75.
pub(crate) const GOODS: u32 = 9600;

pub(crate) fn data() -> GameData {
    let mut d = GameData::empty();
    d.creature_templates.insert(
        AUCTIONEER,
        CreatureTemplate {
            entry: AUCTIONEER,
            name: "Auctioneer".into(),
            display_ids: [903, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            display_scales: [1.0, 0.0, 0.0, 0.0],
            level_min: 2,
            level_max: 2,
            faction: 35,
            unit_class: 1,
            health_max: 50,
            npc_flags: crate::game::map::npc::NPC_FLAG_AUCTIONEER,
            ..Default::default()
        },
    );
    d.creature_spawns.insert(
        0,
        vec![CreatureSpawn {
            guid: 1,
            entries: vec![AUCTIONEER],
            map: 0,
            x: START.0 + 2.0,
            y: START.1,
            z: START.2,
            ..Default::default()
        }],
    );
    d.items.insert(
        GOODS,
        ItemTemplate {
            entry: GOODS,
            name: "Goods".into(),
            display_id: 1000,
            stackable: 20,
            sell_price: 100,
            allowable_class: -1,
            allowable_race: -1,
            ..Default::default()
        },
    );
    d
}

pub(crate) fn auctioneer() -> Guid {
    guid::unit(AUCTIONEER, 1)
}

async fn on_map<T: Send + 'static>(
    realm: &Arc<Realm>,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    realm
        .maps
        .get(realm, MapKey::world(0))
        .query(f)
        .await
        .expect("the map answers")
}

pub(crate) async fn player(realm: &Arc<Realm>, account: &str, name: &str) -> Client {
    let mut c = Client::connect(realm, account);
    c.enter(name, Race::Human, Class::Warrior).await;
    c.sees(auctioneer()).await;
    set_money(realm, c.guid, 10_000).await;
    c.clear();
    c
}

pub(crate) async fn set_money(realm: &Arc<Realm>, g: Guid, money: u32) {
    on_map(realm, move |m| {
        m.players.get_mut(&g).unwrap().set_money(money)
    })
    .await;
}

async fn money(realm: &Arc<Realm>, g: Guid) -> u32 {
    on_map(realm, move |m| m.players[&g].money()).await
}

/// Three goods in `g`'s first backpack slot.
pub(crate) async fn goods(realm: &Arc<Realm>, g: Guid) -> Guid {
    on_map(realm, move |m| {
        let d = m.realm.data.clone();
        let counter = m
            .realm
            .next_item
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let guid = guid::item(counter);
        let p = m.players.get_mut(&g).unwrap();
        p.items
            .retain(|i| i.slot != PACK_START || i.bag != BAG_PLAYER);
        p.items.push(crate::game::map::player::Item::new(
            &d, guid, g, GOODS, BAG_PLAYER, PACK_START, 3,
        ));
        p.refresh_inventory(&d);
        guid
    })
    .await
}

async fn handled(c: &mut Client) {
    c.send(CMSG_NAME_QUERY { guid: c.guid }).await;
    c.wait_for("SMSG_NAME_QUERY_RESPONSE", |m| match m {
        ServerOpcodeMessage::SMSG_NAME_QUERY_RESPONSE(_) => Some(()),
        _ => None,
    })
    .await;
}

/// The auction's command result: its id and what it says.
async fn result(c: &mut Client) -> (u32, SMSG_AUCTION_COMMAND_RESULT_AuctionCommandAction) {
    c.wait_for("SMSG_AUCTION_COMMAND_RESULT", |m| match m {
        ServerOpcodeMessage::SMSG_AUCTION_COMMAND_RESULT(r) => Some((r.auction_id, r.action)),
        _ => None,
    })
    .await
}

type A = SMSG_AUCTION_COMMAND_RESULT_AuctionCommandAction;
type R = SMSG_AUCTION_COMMAND_RESULT_AuctionCommandResult;
type R2 = SMSG_AUCTION_COMMAND_RESULT_AuctionCommandResultTwo;

pub(crate) async fn sell(
    c: &mut Client,
    item: Guid,
    bid: u32,
    buyout: u32,
    minutes: u32,
) -> (u32, A) {
    c.clear();
    c.send(CMSG_AUCTION_SELL_ITEM {
        auctioneer: auctioneer(),
        item,
        starting_bid: bid,
        buyout,
        auction_duration_in_minutes: minutes,
    })
    .await;
    result(c).await
}

pub(crate) async fn bid(c: &mut Client, id: u32, price: u32) -> A {
    c.clear();
    c.send(CMSG_AUCTION_PLACE_BID {
        auctioneer: auctioneer(),
        auction_id: id,
        price: Gold::new(price),
    })
    .await;
    result(c).await.1
}

pub(crate) fn ok(a: &A) -> bool {
    matches!(
        a,
        A::BidPlaced {
            result: R::Ok { .. }
        } | A::Started { result2: R2::Ok }
            | A::Removed { result2: R2::Ok }
    )
}

/// The mail `to` has from the auction house, by its subject's last part.
fn mails(realm: &Arc<Realm>, to: Guid) -> Vec<Mail> {
    let mails = realm.social.mail.lock();
    let mut v: Vec<Mail> = mails
        .mails
        .values()
        .filter(|m| m.receiver == guid::counter(to))
        .cloned()
        .collect();
    v.sort_by_key(|m| m.id);
    v
}

fn answer(m: &Mail) -> &str {
    m.subject.rsplit(':').next().unwrap_or("")
}

/// Thistle Tea's `sell/6` refusals: a duration not 2, 8 or 24 hours, no
/// starting bid, a price past the most, a buyout under the bid, too little
/// for the deposit, another's item; and nothing put up.
#[tokio::test]
async fn a_sale_is_refused_bad_terms_others_items_and_too_little_money() {
    let realm = harness::realm_with(data());
    let mut a = player(&realm, "A", "Alice").await;
    let b = player(&realm, "B", "Bob").await;
    let item = goods(&realm, a.guid).await;
    let theirs = goods(&realm, b.guid).await;
    let refused = |a: &A| matches!(a, A::Started { result2 } if *result2 != R2::Ok);
    for minutes in [0, 1, 119, 121, 360, 2_880] {
        assert!(
            refused(&sell(&mut a, item, 100, 0, minutes).await.1),
            "{minutes}"
        );
    }
    for price in [0, 2_147_483_647] {
        assert!(
            refused(&sell(&mut a, item, price, 0, 120).await.1),
            "{price}"
        );
    }
    assert!(
        refused(&sell(&mut a, item, 100, 99, 120).await.1),
        "under the bid"
    );
    set_money(&realm, a.guid, 74).await;
    assert_eq!(
        sell(&mut a, item, 100, 0, 120).await.1,
        A::Started {
            result2: R2::ErrNotEnoughMoney
        }
    );
    set_money(&realm, a.guid, 10_000).await;
    assert_eq!(
        sell(&mut a, theirs, 100, 0, 120).await.1,
        A::Started {
            result2: R2::ErrItemNotFound
        }
    );
    assert!(realm.social.auctions.lock().auctions.is_empty());
    // Fairly put up: the deposit taken.
    assert!(ok(&sell(&mut a, item, 100, 1_000, 120).await.1));
    assert_eq!(money(&realm, a.guid).await, 10_000 - 75);
}

/// Thistle Tea's `bid/6`: none on one's own, nor one's account's other
/// character's; none under the start or past one's money; the first bid
/// held whole, another no more than it refused, one short of the increment
/// refused, the bidder's raise charged its difference.
#[tokio::test]
async fn bids_are_checked_held_and_raised() {
    let realm = harness::realm_with(data());
    let mut a = player(&realm, "A", "Alice").await;
    let mut b = player(&realm, "B", "Bob").await;
    let item = goods(&realm, a.guid).await;
    let (id, _) = sell(&mut a, item, 100, 1_000, 120).await;
    assert_eq!(
        bid(&mut a, id, 100).await,
        A::BidPlaced {
            result: R::ErrBidOwn
        }
    );
    // Alice away, her account's other character.
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    let alt = a.enter("Alia", Race::Human, Class::Warrior).await;
    a.sees(auctioneer()).await;
    set_money(&realm, alt, 10_000).await;
    assert_eq!(
        bid(&mut a, id, 100).await,
        A::BidPlaced {
            result: R::ErrBidOwn
        },
        "the account's"
    );

    assert_eq!(
        bid(&mut b, id, 99).await,
        A::BidPlaced {
            result: R::ErrBidIncrement
        }
    );
    set_money(&realm, b.guid, 99).await;
    assert_eq!(
        bid(&mut b, id, 100).await,
        A::BidPlaced {
            result: R::ErrNotEnoughMoney
        }
    );
    set_money(&realm, b.guid, 10_000).await;
    assert!(ok(&bid(&mut b, id, 199).await));
    assert_eq!(money(&realm, b.guid).await, 10_000 - 199, "held");
    assert!(matches!(
        bid(&mut b, id, 199).await,
        A::BidPlaced {
            result: R::ErrHigherBid { new_bid: 199, .. }
        }
    ));
    assert_eq!(
        bid(&mut b, id, 203).await,
        A::BidPlaced {
            result: R::ErrBidIncrement
        }
    );
    set_money(&realm, b.guid, 5).await;
    assert!(ok(&bid(&mut b, id, 204).await), "five more");
    assert_eq!(money(&realm, b.guid).await, 0);
    assert!(mails(&realm, b.guid).is_empty(), "no refund to oneself");
}

/// Thistle Tea's refunds and buyouts: each outbid bidder mailed their bid
/// back; the bidder buying out pays the balance, and it is gone.
#[tokio::test]
async fn the_outbid_are_paid_back_and_a_bidders_buyout_pays_the_balance() {
    let realm = harness::realm_with(data());
    let mut a = player(&realm, "A", "Alice").await;
    let mut b = player(&realm, "B", "Bob").await;
    let mut c = player(&realm, "C", "Cid").await;
    let item = goods(&realm, a.guid).await;
    let (id, _) = sell(&mut a, item, 100, 1_000, 120).await;
    assert!(ok(&bid(&mut b, id, 100).await));
    assert!(ok(&bid(&mut c, id, 105).await));
    assert!(ok(&bid(&mut b, id, 110).await));
    assert!(ok(&bid(&mut c, id, 115).await));
    let refunds: Vec<(String, u32)> = mails(&realm, b.guid)
        .iter()
        .map(|m| (answer(m).to_owned(), m.money))
        .collect();
    assert_eq!(refunds, vec![("0".into(), 100), ("0".into(), 110)]);
    assert_eq!(
        mails(&realm, b.guid)[0].subject,
        format!("{GOODS}:0:0"),
        "the outbid subject"
    );

    // Bob bids 900, then buys it out with 100 more.
    assert!(ok(&bid(&mut b, id, 900).await));
    set_money(&realm, b.guid, 100).await;
    assert!(ok(&bid(&mut b, id, 1_000).await));
    assert_eq!(money(&realm, b.guid).await, 0);
    assert!(realm.social.auctions.lock().auctions.is_empty());
    let won = mails(&realm, b.guid)
        .into_iter()
        .find(|m| answer(m) == "1")
        .expect("the won mail");
    assert_eq!(won.item.map(|i| i.guid), Some(guid::counter(item)));
    assert_eq!(
        bid(&mut c, id, 1_000).await,
        A::BidPlaced {
            result: R::ErrItemNotFound
        },
        "gone"
    );
}

/// Thistle Tea's `cancel/5`: an auction with no bid comes back by mail,
/// the deposit kept; only its seller cancels it; with a bid, the seller
/// pays the cut or can't cancel, and the bidder is paid back.
#[tokio::test]
async fn cancelling_returns_the_item_keeps_the_deposit_and_pays_the_bidder() {
    let realm = harness::realm_with(data());
    let mut a = player(&realm, "A", "Alice").await;
    let mut b = player(&realm, "B", "Bob").await;
    let item = goods(&realm, a.guid).await;
    let (id, _) = sell(&mut a, item, 100, 0, 120).await;
    b.send(CMSG_AUCTION_REMOVE_ITEM {
        auctioneer: auctioneer(),
        auction_id: id,
    })
    .await;
    handled(&mut b).await;
    assert!(
        realm.social.auctions.lock().auctions.contains_key(&id),
        "not Bob's"
    );
    a.clear();
    a.send(CMSG_AUCTION_REMOVE_ITEM {
        auctioneer: auctioneer(),
        auction_id: id,
    })
    .await;
    assert!(ok(&result(&mut a).await.1));
    assert_eq!(money(&realm, a.guid).await, 10_000 - 75, "the deposit kept");
    let back = mails(&realm, a.guid);
    assert_eq!(back.len(), 1);
    assert_eq!((answer(&back[0]), back[0].money), ("5", 0));
    assert_eq!(
        back[0].item.as_ref().map(|i| i.guid),
        Some(guid::counter(item))
    );

    // With Bob's 500 on it: the goblins' 15% cut, 75, to cancel.
    let item = goods(&realm, a.guid).await;
    let (id, _) = sell(&mut a, item, 100, 0, 120).await;
    assert!(ok(&bid(&mut b, id, 500).await));
    set_money(&realm, a.guid, 74).await;
    a.clear();
    a.send(CMSG_AUCTION_REMOVE_ITEM {
        auctioneer: auctioneer(),
        auction_id: id,
    })
    .await;
    assert_eq!(
        result(&mut a).await.1,
        A::Removed {
            result2: R2::ErrNotEnoughMoney
        }
    );
    set_money(&realm, a.guid, 1_000).await;
    a.send(CMSG_AUCTION_REMOVE_ITEM {
        auctioneer: auctioneer(),
        auction_id: id,
    })
    .await;
    assert!(ok(&result(&mut a).await.1));
    assert_eq!(money(&realm, a.guid).await, 1_000 - 75);
    let refund = mails(&realm, b.guid);
    assert_eq!(
        refund
            .iter()
            .map(|m| (answer(m), m.money))
            .collect::<Vec<_>>(),
        vec![("4", 500)]
    );
}

/// Thistle Tea's `expire/2`: at the deadline an unsold auction goes home
/// and a sold one to its bidder, its money to the seller less the cut with
/// the deposit back; each once.
#[tokio::test]
async fn auctions_settle_once_at_their_deadline() {
    let realm = harness::realm_with(data());
    let mut a = player(&realm, "A", "Alice").await;
    let mut b = player(&realm, "B", "Bob").await;
    let unsold = goods(&realm, a.guid).await;
    let (first, _) = sell(&mut a, unsold, 100, 0, 120).await;
    let sold = goods(&realm, a.guid).await;
    let (second, _) = sell(&mut a, sold, 100, 0, 120).await;
    assert!(ok(&bid(&mut b, second, 100).await));
    crate::game::social::auction::sweep(&realm);
    assert_eq!(realm.social.auctions.lock().auctions.len(), 2, "not yet");
    for id in [first, second] {
        realm
            .social
            .auctions
            .lock()
            .auctions
            .get_mut(&id)
            .unwrap()
            .expires_at = crate::game::social::mail::now() - 1;
    }
    crate::game::social::auction::sweep(&realm);
    crate::game::social::auction::sweep(&realm);
    assert!(realm.social.auctions.lock().auctions.is_empty());
    let mut home: Vec<(String, u32, Option<u32>)> = mails(&realm, a.guid)
        .iter()
        .map(|m| {
            (
                answer(m).to_owned(),
                m.money,
                m.item.as_ref().map(|i| i.guid),
            )
        })
        .collect();
    home.sort();
    // 100 less the 15 cut, the 75 deposit back.
    assert_eq!(
        home,
        vec![
            ("2".into(), 160, None),
            ("3".into(), 0, Some(guid::counter(unsold)))
        ]
    );
    let won: Vec<Option<u32>> = mails(&realm, b.guid)
        .iter()
        .map(|m| m.item.as_ref().map(|i| i.guid))
        .collect();
    assert_eq!(won, vec![Some(guid::counter(sold))]);
}
