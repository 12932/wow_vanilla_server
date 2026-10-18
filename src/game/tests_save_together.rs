//! What moves between characters, or between a character and a mail or an
//! auction, is saved on both sides at once (`Store::save_together`), as
//! mangos-classic saves them in one transaction: after a trade, a mail
//! sent or taken, and an auction put up, bought out or cancelled, the
//! store already has both sides, with no other save. (A crash between two
//! saves apart copied or lost what moved.)

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::inventory::PACK_START;
use crate::game::map::{Map, MapKey};
use crate::game::social::auction::{Auction, KIND_AUCTION};
use crate::game::social::mail::{KIND_MAIL, Mail};
use crate::game::store::{BAG_PLAYER, CharacterRow};

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const AUCTIONEER: u32 = 3170;
/// Sells for 100 each: three for two hours at the goblins' 25% leave 75.
const GOODS: u32 = 9600;
const MONEY: u32 = 10_000;

fn data() -> GameData {
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

fn auctioneer() -> Guid {
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

/// A player in the world near the auctioneer, with money that only the
/// map knows of (no save has it).
async fn player(realm: &Arc<Realm>, name: &str) -> Client {
    let mut c = Client::connect(realm, name);
    let g = c.enter(name, Race::Human, Class::Warrior).await;
    c.sees(auctioneer()).await;
    on_map(realm, move |m| {
        let d = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        p.items
            .retain(|i| i.slot != PACK_START || i.bag != BAG_PLAYER);
        p.set_money(MONEY);
        p.refresh_inventory(&d);
    })
    .await;
    c.clear();
    c
}

/// Three goods in `g`'s first backpack slot: its item GUID counter.
async fn goods(realm: &Arc<Realm>, g: Guid) -> Guid {
    on_map(realm, move |m| {
        let d = m.realm.data.clone();
        let counter = m
            .realm
            .next_item
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let item = guid::item(counter);
        let p = m.players.get_mut(&g).unwrap();
        p.items.push(crate::game::map::player::Item::new(
            &d, item, g, GOODS, BAG_PLAYER, PACK_START, 3,
        ));
        p.refresh_inventory(&d);
        item
    })
    .await
}

/// `g`'s character as the store has it, once the map has done what it was
/// sent (its handlers send their saves before they return).
async fn stored(realm: &Arc<Realm>, g: Guid) -> CharacterRow {
    on_map(realm, |_| ()).await;
    realm
        .store
        .load(guid::counter(g))
        .await
        .expect("the character is stored")
}

fn holds(row: &CharacterRow, item: Guid) -> bool {
    row.items.iter().any(|i| i.guid == guid::counter(item))
}

async fn stored_mail(realm: &Arc<Realm>, id: u32) -> Option<Mail> {
    on_map(realm, |_| ()).await;
    let data = realm.store.record(KIND_MAIL, u64::from(id)).await?;
    Some(serde_json::from_str(&data).expect("a mail"))
}

async fn stored_auction(realm: &Arc<Realm>, id: u32) -> Option<Auction> {
    on_map(realm, |_| ()).await;
    let data = realm.store.record(KIND_AUCTION, u64::from(id)).await?;
    Some(serde_json::from_str(&data).expect("an auction"))
}

/// The mails `to` has, by id.
fn mails_of(realm: &Arc<Realm>, to: Guid) -> Vec<Mail> {
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

async fn handled(c: &mut Client) {
    c.send(CMSG_NAME_QUERY { guid: c.guid }).await;
    c.wait_for("SMSG_NAME_QUERY_RESPONSE", |m| match m {
        ServerOpcodeMessage::SMSG_NAME_QUERY_RESPONSE(_) => Some(()),
        _ => None,
    })
    .await;
}

/// A trade: Alice's goods and 100 copper for Bob's 50.
#[tokio::test]
async fn a_trade_saves_both_sides_together() {
    type S = SMSG_TRADE_STATUS_TradeStatus;
    let realm = harness::realm_with(data());
    let mut a = player(&realm, "Alice").await;
    let mut b = player(&realm, "Bob").await;
    let item = goods(&realm, a.guid).await;
    a.send(CMSG_INITIATE_TRADE { guid: b.guid }).await;
    b.wait_for("the trade asked", |m| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS(s) => {
            matches!(s.status, S::BeginTrade { .. }).then_some(())
        }
        _ => None,
    })
    .await;
    b.send(CMSG_BEGIN_TRADE {}).await;
    a.wait_for("the window open", |m| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS(s) => {
            matches!(s.status, S::OpenWindow).then_some(())
        }
        _ => None,
    })
    .await;
    a.send(CMSG_SET_TRADE_ITEM {
        trade_slot: 0,
        bag: BAG_PLAYER,
        slot: PACK_START,
    })
    .await;
    a.send(CMSG_SET_TRADE_GOLD {
        gold: Gold::new(100),
    })
    .await;
    b.send(CMSG_SET_TRADE_GOLD {
        gold: Gold::new(50),
    })
    .await;
    handled(&mut a).await;
    handled(&mut b).await;
    b.clear();
    a.send(CMSG_ACCEPT_TRADE { unknown1: 0 }).await;
    handled(&mut a).await;
    b.send(CMSG_ACCEPT_TRADE { unknown1: 0 }).await;
    b.wait_for("the trade done", |m| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS(s) => {
            matches!(s.status, S::TradeComplete).then_some(())
        }
        _ => None,
    })
    .await;

    let (alice, bob) = (stored(&realm, a.guid).await, stored(&realm, b.guid).await);
    assert!(!holds(&alice, item), "the goods out of Alice's stored bags");
    assert!(holds(&bob, item), "and in Bob's");
    assert_eq!(alice.money, MONEY - 100 + 50);
    assert_eq!(bob.money, MONEY + 100 - 50);
    // Their periodic saves put off: they were just saved.
    let interval = realm.config.save_interval;
    let (ga, gb) = (a.guid, b.guid);
    let later = on_map(&realm, move |m| {
        [ga, gb]
            .iter()
            .all(|g| m.players[g].save_at > m.now + interval - Duration::from_secs(60))
    })
    .await;
    assert!(later, "the periodic saves put off");
}

/// Alice mails Bob her goods and 200 copper: `id`.
async fn mail_goods(a: &mut Client, item: Guid) -> u32 {
    a.clear();
    a.send(CMSG_SEND_MAIL {
        mailbox: Guid::zero(),
        receiver: "Bob".into(),
        subject: "goods".into(),
        body: String::new(),
        unknown1: 0,
        unknown2: 0,
        item,
        money: Gold::new(200),
        cash_on_delivery_amount: 0,
        unknown3: 0,
        unknown4: 0,
    })
    .await;
    a.wait_for("SMSG_SEND_MAIL_RESULT", |m| match m {
        ServerOpcodeMessage::SMSG_SEND_MAIL_RESULT(r) => Some(r.mail_id),
        _ => None,
    })
    .await
}

/// A mail sent with an item and money: the mail and the sender's bags and
/// money stored together.
#[tokio::test]
async fn a_mail_sent_saves_the_sender_with_it() {
    let realm = harness::realm_with(data());
    let mut a = player(&realm, "Alice").await;
    let _b = player(&realm, "Bob").await;
    let item = goods(&realm, a.guid).await;
    let id = mail_goods(&mut a, item).await;

    let mail = stored_mail(&realm, id).await.expect("the mail stored");
    assert_eq!(mail.item.map(|i| i.guid), Some(guid::counter(item)));
    assert_eq!(mail.money, 200);
    let alice = stored(&realm, a.guid).await;
    assert!(!holds(&alice, item), "the goods out of Alice's stored bags");
    assert_eq!(alice.money, MONEY - 200 - 2 * 30, "the money and postage");
}

/// A mail's item and money taken: the mail without them and the taker
/// with them stored together.
#[tokio::test]
async fn a_mail_taken_saves_the_taker_with_it() {
    type R = SMSG_SEND_MAIL_RESULT_MailResult;
    let realm = harness::realm_with(data());
    let mut a = player(&realm, "Alice").await;
    let mut b = player(&realm, "Bob").await;
    let item = goods(&realm, a.guid).await;
    let id = mail_goods(&mut a, item).await;
    b.clear();
    b.send(CMSG_MAIL_TAKE_ITEM {
        mailbox: Guid::zero(),
        mail_id: id,
    })
    .await;
    b.wait_for("the item taken", |m| match m {
        ServerOpcodeMessage::SMSG_SEND_MAIL_RESULT(r) => match r.action {
            SMSG_SEND_MAIL_RESULT_MailAction::ItemTaken {
                result: R::Ok { .. },
            } => Some(()),
            _ => None,
        },
        _ => None,
    })
    .await;
    let mail = stored_mail(&realm, id).await.expect("the mail stored");
    assert!(mail.item.is_none(), "the stored mail without its item");
    let bob = stored(&realm, b.guid).await;
    assert!(holds(&bob, item), "and Bob's stored bags with it");

    b.send(CMSG_MAIL_TAKE_MONEY {
        mailbox: Guid::zero(),
        mail_id: id,
    })
    .await;
    b.wait_for("the money taken", |m| match m {
        ServerOpcodeMessage::SMSG_SEND_MAIL_RESULT(r) => matches!(
            r.action,
            SMSG_SEND_MAIL_RESULT_MailAction::MoneyTaken { .. }
        )
        .then_some(()),
        _ => None,
    })
    .await;
    let mail = stored_mail(&realm, id).await.expect("the mail stored");
    assert_eq!(mail.money, 0, "the stored mail without its money");
    assert_eq!(stored(&realm, b.guid).await.money, MONEY + 200);
}

type A = SMSG_AUCTION_COMMAND_RESULT_AuctionCommandAction;
type R2 = SMSG_AUCTION_COMMAND_RESULT_AuctionCommandResultTwo;

async fn auction_result(c: &mut Client) -> (u32, A) {
    c.wait_for("SMSG_AUCTION_COMMAND_RESULT", |m| match m {
        ServerOpcodeMessage::SMSG_AUCTION_COMMAND_RESULT(r) => Some((r.auction_id, r.action)),
        _ => None,
    })
    .await
}

/// Alice's goods put up for 100, bought out at 1000: the auction's id.
async fn put_up(a: &mut Client, item: Guid) -> u32 {
    a.clear();
    a.send(CMSG_AUCTION_SELL_ITEM {
        auctioneer: auctioneer(),
        item,
        starting_bid: 100,
        buyout: 1_000,
        auction_duration_in_minutes: 120,
    })
    .await;
    let (id, r) = auction_result(a).await;
    assert_eq!(r, A::Started { result2: R2::Ok });
    id
}

async fn bid(c: &mut Client, id: u32, price: u32) {
    c.clear();
    c.send(CMSG_AUCTION_PLACE_BID {
        auctioneer: auctioneer(),
        auction_id: id,
        price: Gold::new(price),
    })
    .await;
    let (_, r) = auction_result(c).await;
    assert!(
        matches!(
            r,
            A::BidPlaced {
                result: SMSG_AUCTION_COMMAND_RESULT_AuctionCommandResult::Ok { .. }
            }
        ),
        "{r:?}"
    );
}

/// An auction put up: the auction with its item and the seller without it
/// or the deposit stored together.
#[tokio::test]
async fn an_auction_put_up_saves_the_seller_with_it() {
    let realm = harness::realm_with(data());
    let mut a = player(&realm, "Alice").await;
    let item = goods(&realm, a.guid).await;
    let id = put_up(&mut a, item).await;
    let auction = stored_auction(&realm, id)
        .await
        .expect("the auction stored");
    assert_eq!(auction.item.guid, guid::counter(item));
    let alice = stored(&realm, a.guid).await;
    assert!(!holds(&alice, item), "the goods out of Alice's stored bags");
    assert_eq!(alice.money, MONEY - 75, "the deposit");
}

/// An auction bought out: the buyer's money, the auction's end and the
/// mails that settle it stored together.
#[tokio::test]
async fn a_buyout_saves_the_buyer_with_the_auctions_end() {
    let realm = harness::realm_with(data());
    let mut a = player(&realm, "Alice").await;
    let mut b = player(&realm, "Bob").await;
    let item = goods(&realm, a.guid).await;
    let id = put_up(&mut a, item).await;
    bid(&mut b, id, 1_000).await;
    assert!(
        stored_auction(&realm, id).await.is_none(),
        "the auction gone"
    );
    assert_eq!(stored(&realm, b.guid).await.money, MONEY - 1_000);
    let won = mails_of(&realm, b.guid);
    assert_eq!(won.len(), 1, "the item mailed to the buyer");
    let won = stored_mail(&realm, won[0].id).await.expect("stored");
    assert_eq!(won.item.map(|i| i.guid), Some(guid::counter(item)));
    let sold = mails_of(&realm, a.guid);
    assert_eq!(sold.len(), 1, "the money mailed to the seller");
    assert!(
        stored_mail(&realm, sold[0].id)
            .await
            .is_some_and(|m| m.money > 0)
    );
}

/// An auction cancelled with a bid on it: the auction's end, its item
/// mailed back, the bid refunded and the seller's fee stored together.
#[tokio::test]
async fn a_cancel_saves_the_seller_with_the_auctions_end() {
    let realm = harness::realm_with(data());
    let mut a = player(&realm, "Alice").await;
    let mut b = player(&realm, "Bob").await;
    let item = goods(&realm, a.guid).await;
    let id = put_up(&mut a, item).await;
    bid(&mut b, id, 100).await;
    assert_eq!(
        stored(&realm, b.guid).await.money,
        MONEY - 100,
        "the bid held"
    );
    a.clear();
    a.send(CMSG_AUCTION_REMOVE_ITEM {
        auctioneer: auctioneer(),
        auction_id: id,
    })
    .await;
    assert_eq!(
        auction_result(&mut a).await,
        (id, A::Removed { result2: R2::Ok })
    );
    assert!(
        stored_auction(&realm, id).await.is_none(),
        "the auction gone"
    );
    // The goblins' cut of the bid: 15%.
    assert_eq!(stored(&realm, a.guid).await.money, MONEY - 75 - 15);
    let back = mails_of(&realm, a.guid);
    assert_eq!(back.len(), 1, "the item mailed back");
    let back = stored_mail(&realm, back[0].id).await.expect("stored");
    assert_eq!(back.item.map(|i| i.guid), Some(guid::counter(item)));
    let refund = mails_of(&realm, b.guid);
    assert_eq!(refund.len(), 1, "the bid refunded");
    assert_eq!(
        stored_mail(&realm, refund[0].id).await.map(|m| m.money),
        Some(100)
    );
}
