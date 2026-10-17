//! Phase 3's trades, Thistle Tea's `TradeTest` ported: an edit unaccepts
//! both sides, an item offered once, traded stacks onto the receiver's,
//! the room what goes leaves, a full receiver, money short or past the
//! most, an offer changed since, and a timed item's time kept.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::inventory::{BANK_START, PACK_END, PACK_START};
use crate::game::map::{Map, MapKey};
use crate::game::store::BAG_PLAYER;

const STACK: u32 = 9500;
const PLAIN: u32 = 9501;
const FILLER: u32 = 9502;
const TIMED: u32 = 9503;

fn data() -> GameData {
    let mut d = GameData::empty();
    for (entry, stack, duration) in [
        (STACK, 20, 0),
        (PLAIN, 1, 0),
        (FILLER, 1, 0),
        (TIMED, 1, 3600),
    ] {
        d.items.insert(
            entry,
            ItemTemplate {
                entry,
                name: format!("Item {entry}"),
                display_id: 1000,
                stackable: stack,
                duration,
                allowable_class: -1,
                allowable_race: -1,
                ..Default::default()
            },
        );
    }
    d
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

async fn player(realm: &Arc<Realm>, name: &str) -> Client {
    let mut c = Client::connect(realm, name);
    let g = c.enter(name, Race::Human, Class::Warrior).await;
    on_map(realm, move |m| {
        let d = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        p.items.clear();
        p.set_money(1000);
        p.refresh_inventory(&d);
    })
    .await;
    c.clear();
    c
}

async fn give_at(realm: &Arc<Realm>, g: Guid, entry: u32, count: u32, at: (u8, u8)) -> Guid {
    on_map(realm, move |m| {
        let d = m.realm.data.clone();
        let counter = m
            .realm
            .next_item
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let guid = crate::game::guid::item(counter);
        let p = m.players.get_mut(&g).unwrap();
        p.items.push(crate::game::map::player::Item::new(
            &d, guid, g, entry, at.0, at.1, count,
        ));
        p.refresh_inventory(&d);
        guid
    })
    .await
}

async fn fill_pack(realm: &Arc<Realm>, g: Guid, from: u8) {
    for slot in from..PACK_END {
        give_at(realm, g, FILLER, 1, (BAG_PLAYER, slot)).await;
    }
}

async fn handled(c: &mut Client) {
    c.send(CMSG_NAME_QUERY { guid: c.guid }).await;
    c.wait_for("SMSG_NAME_QUERY_RESPONSE", |m| match m {
        ServerOpcodeMessage::SMSG_NAME_QUERY_RESPONSE(_) => Some(()),
        _ => None,
    })
    .await;
}

/// A trade open between `a` and `b`.
async fn open(a: &mut Client, b: &mut Client) {
    a.send(CMSG_INITIATE_TRADE { guid: b.guid }).await;
    b.wait_for("the trade asked", |m| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS(s) => {
            matches!(s.status, SMSG_TRADE_STATUS_TradeStatus::BeginTrade { .. }).then_some(())
        }
        _ => None,
    })
    .await;
    b.send(CMSG_BEGIN_TRADE {}).await;
    a.wait_for("the window open", |m| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS(s) => {
            matches!(s.status, SMSG_TRADE_STATUS_TradeStatus::OpenWindow).then_some(())
        }
        _ => None,
    })
    .await;
}

async fn offer(c: &Client, trade_slot: u8, at: (u8, u8)) {
    c.send(CMSG_SET_TRADE_ITEM {
        trade_slot,
        bag: at.0,
        slot: at.1,
    })
    .await;
}

/// Both accept: whether it went through, else what each was told.
async fn both_accept(a: &mut Client, b: &mut Client) -> Result<(), String> {
    // Each side's edits in before either accepts.
    handled(a).await;
    handled(b).await;
    a.clear();
    b.clear();
    a.send(CMSG_ACCEPT_TRADE { unknown1: 0 }).await;
    handled(a).await;
    b.send(CMSG_ACCEPT_TRADE { unknown1: 0 }).await;
    handled(b).await;
    let ended = |m: &ServerOpcodeMessage| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS(s) => match s.status {
            SMSG_TRADE_STATUS_TradeStatus::TradeComplete => Some(Ok(())),
            SMSG_TRADE_STATUS_TradeStatus::BackToTrade => Some(Err("back to trade".to_owned())),
            SMSG_TRADE_STATUS_TradeStatus::TradeCanceled => Some(Err("canceled".to_owned())),
            _ => None,
        },
        _ => None,
    };
    b.wait_for("the trade's end", ended).await
}

/// `g`'s trade: whether accepted, and its items' slots taken.
async fn trade_of(realm: &Arc<Realm>, g: Guid) -> Option<(bool, Vec<bool>)> {
    on_map(realm, move |m| {
        m.players[&g]
            .trade
            .as_ref()
            .map(|t| (t.accepted, t.items.iter().map(Option::is_some).collect()))
    })
    .await
}

async fn at(realm: &Arc<Realm>, g: Guid, item: Guid) -> Option<(u8, u8, u32)> {
    on_map(realm, move |m| {
        let p = &m.players[&g];
        p.item_by_guid(item)
            .map(|i| (p.items[i].bag, p.items[i].slot, p.items[i].count()))
    })
    .await
}

async fn stacks(realm: &Arc<Realm>, g: Guid, entry: u32) -> Vec<u32> {
    on_map(realm, move |m| {
        let mut v: Vec<u32> = m.players[&g]
            .items
            .iter()
            .filter(|i| i.entry == entry)
            .map(|i| i.count())
            .collect();
        v.sort();
        v
    })
    .await
}

const PACK: (u8, u8) = (BAG_PLAYER, PACK_START);

/// Thistle Tea's negotiation: an accepted offer unaccepted on both sides
/// by either's edit, the accepting side told; an item in one trade slot
/// only; no slot past the seventh.
#[tokio::test]
async fn an_edit_unaccepts_both_and_an_item_is_offered_once() {
    let realm = harness::realm_with(data());
    let mut a = player(&realm, "Alice").await;
    let mut b = player(&realm, "Bob").await;
    let (ga, gb) = (a.guid, b.guid);
    give_at(&realm, ga, PLAIN, 1, PACK).await;
    give_at(&realm, gb, PLAIN, 1, PACK).await;
    open(&mut a, &mut b).await;
    a.send(CMSG_ACCEPT_TRADE { unknown1: 0 }).await;
    handled(&mut a).await;
    assert_eq!(trade_of(&realm, ga).await.unwrap().0, true);
    a.clear();
    b.send(CMSG_SET_TRADE_GOLD {
        gold: Gold::new(100),
    })
    .await;
    a.wait_for("back to trade", |m| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS(s) => {
            matches!(s.status, SMSG_TRADE_STATUS_TradeStatus::BackToTrade).then_some(())
        }
        _ => None,
    })
    .await;
    assert_eq!(trade_of(&realm, ga).await.unwrap().0, false);
    assert_eq!(trade_of(&realm, gb).await.unwrap().0, false);
    a.send(CMSG_ACCEPT_TRADE { unknown1: 0 }).await;
    handled(&mut a).await;
    offer(&b, 0, PACK).await;
    handled(&mut b).await;
    assert_eq!(
        trade_of(&realm, ga).await.unwrap().0,
        false,
        "an item edit too"
    );

    offer(&a, 0, PACK).await;
    offer(&a, 1, PACK).await;
    offer(&a, 7, PACK).await;
    handled(&mut a).await;
    let slots = trade_of(&realm, ga).await.unwrap().1;
    assert_eq!(slots, vec![true, false, false, false, false, false, false]);
}

/// Thistle Tea's merged incoming stacks: five onto ten makes fifteen, the
/// five's item gone; fifteen onto fifteen fills one to twenty and the
/// item, still itself, keeps the ten left.
#[tokio::test]
async fn traded_stacks_go_onto_the_receivers() {
    let realm = harness::realm_with(data());
    let mut a = player(&realm, "Alice").await;
    let mut b = player(&realm, "Bob").await;
    let (ga, gb) = (a.guid, b.guid);
    let five = give_at(&realm, ga, STACK, 5, PACK).await;
    give_at(&realm, gb, STACK, 10, PACK).await;
    open(&mut a, &mut b).await;
    offer(&a, 0, PACK).await;
    assert_eq!(both_accept(&mut a, &mut b).await, Ok(()));
    assert_eq!(stacks(&realm, gb, STACK).await, vec![15]);
    assert_eq!(at(&realm, ga, five).await, None);
    assert_eq!(at(&realm, gb, five).await, None, "merged away");

    let fifteen = give_at(&realm, ga, STACK, 15, PACK).await;
    open(&mut a, &mut b).await;
    offer(&a, 0, PACK).await;
    assert_eq!(both_accept(&mut a, &mut b).await, Ok(()));
    assert_eq!(stacks(&realm, gb, STACK).await, vec![10, 20]);
    assert_eq!(at(&realm, gb, fifteen).await.map(|(_, _, n)| n), Some(10));
}

/// Thistle Tea's room: both packs full, each item traded takes the slot
/// the other leaves; a full receiver given one item takes nothing, and
/// both are sent back to the trade.
#[tokio::test]
async fn what_goes_leaves_room_and_a_full_receiver_refuses() {
    let realm = harness::realm_with(data());
    let mut a = player(&realm, "Alice").await;
    let mut b = player(&realm, "Bob").await;
    let (ga, gb) = (a.guid, b.guid);
    let first = give_at(&realm, ga, PLAIN, 1, PACK).await;
    let second = give_at(&realm, gb, TIMED, 1, PACK).await;
    fill_pack(&realm, ga, PACK_START + 1).await;
    fill_pack(&realm, gb, PACK_START + 1).await;
    open(&mut a, &mut b).await;
    offer(&a, 0, PACK).await;
    offer(&b, 0, PACK).await;
    assert_eq!(both_accept(&mut a, &mut b).await, Ok(()));
    assert_eq!(
        at(&realm, gb, first).await,
        Some((BAG_PLAYER, PACK_START, 1))
    );
    assert_eq!(
        at(&realm, ga, second).await,
        Some((BAG_PLAYER, PACK_START, 1))
    );

    open(&mut a, &mut b).await;
    offer(&a, 0, PACK).await;
    assert_eq!(
        both_accept(&mut a, &mut b).await,
        Err("back to trade".to_owned())
    );
    assert_eq!(
        at(&realm, ga, second).await,
        Some((BAG_PLAYER, PACK_START, 1))
    );
    assert_eq!(trade_of(&realm, gb).await.unwrap().0, false);
}

/// Thistle Tea's money: more offered than is there, or more than the
/// receiver may hold, and neither side pays.
#[tokio::test]
async fn money_short_or_past_the_most_is_refused() {
    let realm = harness::realm_with(data());
    let mut a = player(&realm, "Alice").await;
    let mut b = player(&realm, "Bob").await;
    let (ga, gb) = (a.guid, b.guid);
    open(&mut a, &mut b).await;
    a.send(CMSG_SET_TRADE_GOLD {
        gold: Gold::new(1_001),
    })
    .await;
    assert!(both_accept(&mut a, &mut b).await.is_err(), "short");
    on_map(&realm, move |m| {
        m.players.get_mut(&gb).unwrap().set_money(2_147_483_647)
    })
    .await;
    a.send(CMSG_SET_TRADE_GOLD { gold: Gold::new(1) }).await;
    assert!(both_accept(&mut a, &mut b).await.is_err(), "past the most");
    let money = on_map(&realm, move |m| {
        (m.players[&ga].money(), m.players[&gb].money())
    })
    .await;
    assert_eq!(money, (1000, 2_147_483_647));
}

/// Thistle Tea's offer checked again: a stack split since, an item banked
/// since or bound since calls the trade off, nothing moved; a timed
/// item's time goes with it.
#[tokio::test]
async fn an_offer_changed_since_is_called_off_and_time_goes_with_an_item() {
    let realm = harness::realm_with(data());
    let mut a = player(&realm, "Alice").await;
    let mut b = player(&realm, "Bob").await;
    let (ga, gb) = (a.guid, b.guid);
    let stack = give_at(&realm, ga, STACK, 10, PACK).await;
    open(&mut a, &mut b).await;
    offer(&a, 0, PACK).await;
    handled(&mut a).await;
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        let i = p.item_by_guid(stack).unwrap();
        p.items[i].set_count(9);
    })
    .await;
    assert_eq!(
        both_accept(&mut a, &mut b).await,
        Err("canceled".to_owned())
    );
    assert_eq!(
        at(&realm, ga, stack).await,
        Some((BAG_PLAYER, PACK_START, 9))
    );

    for change in ["banked", "bound"] {
        open(&mut a, &mut b).await;
        offer(&a, 0, PACK).await;
        handled(&mut a).await;
        on_map(&realm, move |m| {
            let p = m.players.get_mut(&ga).unwrap();
            let i = p.item_by_guid(stack).unwrap();
            if change == "banked" {
                p.items[i].slot = BANK_START;
            } else {
                p.items[i].fields.set_u32(ITEM_FIELD_FLAGS, 1);
            }
        })
        .await;
        assert_eq!(
            both_accept(&mut a, &mut b).await,
            Err("canceled".to_owned()),
            "{change}"
        );
        on_map(&realm, move |m| {
            let p = m.players.get_mut(&ga).unwrap();
            let i = p.item_by_guid(stack).unwrap();
            p.items[i].slot = PACK_START;
            p.items[i].fields.set_u32(ITEM_FIELD_FLAGS, 0);
        })
        .await;
    }
    assert_eq!(stacks(&realm, gb, STACK).await, Vec::<u32>::new());

    // A timed item's time left goes with it.
    let timed = give_at(&realm, ga, TIMED, 1, (BAG_PLAYER, PACK_START + 1)).await;
    let left = on_map(&realm, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        let i = p.item_by_guid(timed).unwrap();
        p.items[i].fields.set_u32(ITEM_FIELD_DURATION, 1234);
        1234
    })
    .await;
    open(&mut a, &mut b).await;
    offer(&a, 0, (BAG_PLAYER, PACK_START + 1)).await;
    assert_eq!(both_accept(&mut a, &mut b).await, Ok(()));
    let now = on_map(&realm, move |m| {
        let p = &m.players[&gb];
        p.items[p.item_by_guid(timed).unwrap()]
            .fields
            .get_u32(ITEM_FIELD_DURATION)
    })
    .await;
    assert!(now <= left && now + 2 >= left, "{now} of {left}");
}
