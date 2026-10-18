//! An item's spell charges kept wherever the item is (question 13):
//! carried, in a bag or the bank through a relog, mailed and taken, held by
//! the auction house and back by its mail, traded and bought back; and an
//! item with its charges spent kept or used up as mangos-classic's
//! `Spell::TakeCastItem` has it.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::inventory::{BAG_START, BANK_START, BUYBACK_START, PACK_START};
use crate::game::map::{Map, MapKey, vendor};
use crate::game::social::auction::{Auction, KIND_AUCTION};
use crate::game::social::mail::{KIND_MAIL, Mail};
use crate::game::store::{BAG_PLAYER, CharacterRow};

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const AUCTIONEER: u32 = 3170;
const VENDOR: u32 = 3300;
/// A wand of three charges with a second spell of five that it is used up
/// by; a bomb used up with its one charge; a bag.
const WAND: u32 = 9700;
const BOMB: u32 = 9701;
const SATCHEL: u32 = 9702;
const MONEY: u32 = 100_000;
/// The wand part-used: one of its three left, two of its five.
const SPENT: [i32; 5] = [1, -2, 0, 0, 0];
/// `SMSG_CAST_RESULT`'s "no charges remain".
const NO_CHARGES_REMAIN: u8 = 0x44;

fn creature(entry: u32, name: &str, npc_flags: u32) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: name.into(),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        display_scales: [1.0, 0.0, 0.0, 0.0],
        level_min: 2,
        level_max: 2,
        faction: 35,
        unit_class: 1,
        health_max: 50,
        npc_flags,
        ..Default::default()
    }
}

fn item(entry: u32) -> ItemTemplate {
    ItemTemplate {
        entry,
        name: format!("Item {entry}"),
        display_id: 1000,
        stackable: 1,
        allowable_class: -1,
        allowable_race: -1,
        buy_count: 1,
        buy_price: 100,
        sell_price: 50,
        ..Default::default()
    }
}

fn spell(id: u32, charges: i32) -> crate::game::data::items::ItemSpell {
    crate::game::data::items::ItemSpell {
        id,
        charges,
        ..Default::default()
    }
}

fn data() -> GameData {
    let mut d = GameData::empty();
    d.creature_templates.insert(
        AUCTIONEER,
        creature(
            AUCTIONEER,
            "Auctioneer",
            crate::game::map::npc::NPC_FLAG_AUCTIONEER,
        ),
    );
    d.creature_templates
        .insert(VENDOR, creature(VENDOR, "Vendor", vendor::NPC_FLAG_VENDOR));
    let spawn = |guid, entry, dx| CreatureSpawn {
        guid,
        entries: vec![entry],
        map: 0,
        x: START.0 + dx,
        y: START.1,
        z: START.2,
        ..Default::default()
    };
    d.creature_spawns
        .insert(0, vec![spawn(1, AUCTIONEER, 2.0), spawn(2, VENDOR, -2.0)]);
    let mut wand = item(WAND);
    wand.spells[0] = spell(10, 3);
    wand.spells[1] = spell(11, -5);
    d.items.insert(WAND, wand);
    let mut bomb = item(BOMB);
    bomb.spells[0] = spell(12, -1);
    d.items.insert(BOMB, bomb);
    let mut satchel = item(SATCHEL);
    satchel.class = 1;
    satchel.inventory_type = 18;
    satchel.container_slots = 4;
    d.items.insert(SATCHEL, satchel);
    d
}

fn auctioneer() -> Guid {
    guid::unit(AUCTIONEER, 1)
}

fn vendor_guid() -> Guid {
    guid::unit(VENDOR, 2)
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

/// A player in the world by the auctioneer and the vendor, carrying
/// nothing, with money.
async fn player(realm: &Arc<Realm>, name: &str) -> Client {
    let mut c = Client::connect(realm, name);
    let g = c.enter(name, Race::Human, Class::Warrior).await;
    c.sees(auctioneer()).await;
    c.sees(vendor_guid()).await;
    on_map(realm, move |m| {
        let d = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        p.items.clear();
        p.set_money(MONEY);
        p.refresh_inventory(&d);
    })
    .await;
    c.clear();
    c
}

/// `entry` at `at`, with `charges` left if given: its GUID.
async fn give_at(
    realm: &Arc<Realm>,
    g: Guid,
    entry: u32,
    at: (u8, u8),
    charges: Option<[i32; 5]>,
) -> Guid {
    on_map(realm, move |m| {
        let d = m.realm.data.clone();
        let counter = m
            .realm
            .next_item
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let item = guid::item(counter);
        let p = m.players.get_mut(&g).unwrap();
        let mut it = crate::game::map::player::Item::new(&d, item, g, entry, at.0, at.1, 1);
        for (k, c) in charges.iter().flatten().enumerate() {
            it.fields.set_i32(ITEM_FIELD_SPELL_CHARGES + k as u16, *c);
        }
        p.items.push(it);
        p.refresh_inventory(&d);
        item
    })
    .await
}

/// A part-used wand in `g`'s first backpack slot.
async fn spent_wand(realm: &Arc<Realm>, g: Guid) -> Guid {
    give_at(realm, g, WAND, (BAG_PLAYER, PACK_START), Some(SPENT)).await
}

/// `item`'s charges and place, if `g` has it.
async fn charges(realm: &Arc<Realm>, g: Guid, item: Guid) -> Option<([i32; 5], (u8, u8))> {
    on_map(realm, move |m| {
        let p = &m.players[&g];
        p.item_by_guid(item).map(|i| {
            let it = &p.items[i];
            (
                std::array::from_fn(|k| {
                    it.fields.get_u32(ITEM_FIELD_SPELL_CHARGES + k as u16) as i32
                }),
                (it.bag, it.slot),
            )
        })
    })
    .await
}

async fn stored(realm: &Arc<Realm>, g: Guid) -> CharacterRow {
    on_map(realm, |_| ()).await;
    realm
        .store
        .load(guid::counter(g))
        .await
        .expect("the character is stored")
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

/// Log out and back in on the same connection.
async fn relog(c: &mut Client) {
    c.send(CMSG_LOGOUT_REQUEST {}).await;
    c.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    c.clear();
    let g = c.guid;
    c.login(g).await;
}

/// Take mail `id`'s item.
async fn take(c: &mut Client, id: u32) {
    type R = SMSG_SEND_MAIL_RESULT_MailResult;
    c.clear();
    c.send(CMSG_MAIL_TAKE_ITEM {
        mailbox: Guid::zero(),
        mail_id: id,
    })
    .await;
    c.wait_for("the item taken", |m| match m {
        ServerOpcodeMessage::SMSG_SEND_MAIL_RESULT(r) => match r.action {
            SMSG_SEND_MAIL_RESULT_MailAction::ItemTaken {
                result: R::Ok { .. },
            } => Some(()),
            SMSG_SEND_MAIL_RESULT_MailAction::ItemTaken { result } => {
                panic!("taking mail {id}: {result:?}")
            }
            _ => None,
        },
        _ => None,
    })
    .await;
}

/// The part-used wand, carried, in a bag and in the bank, keeps its charges
/// through a relog, saved as five numbers; one the store saved before
/// charges were (none stored) has its template's.
#[tokio::test]
async fn charges_are_kept_through_a_relog_carried_in_a_bag_and_banked() {
    let realm = harness::realm_with(data());
    let mut a = player(&realm, "Alice").await;
    let g = a.guid;
    let carried = spent_wand(&realm, g).await;
    give_at(&realm, g, SATCHEL, (BAG_PLAYER, BAG_START), None).await;
    let bagged = give_at(&realm, g, WAND, (BAG_START, 0), Some([3, -1, 0, 0, 0])).await;
    let banked = give_at(
        &realm,
        g,
        WAND,
        (BAG_PLAYER, PACK_START + 1),
        Some([0, -4, 0, 0, 0]),
    )
    .await;
    a.send(CMSG_AUTOBANK_ITEM {
        bag_index: BAG_PLAYER,
        slot_index: PACK_START + 1,
    })
    .await;
    handled(&mut a).await;
    assert_eq!(
        charges(&realm, g, banked).await.map(|c| c.1),
        Some((BAG_PLAYER, BANK_START)),
        "banked"
    );
    relog(&mut a).await;
    assert_eq!(
        charges(&realm, g, carried).await,
        Some((SPENT, (BAG_PLAYER, PACK_START)))
    );
    assert_eq!(
        charges(&realm, g, bagged).await,
        Some(([3, -1, 0, 0, 0], (BAG_START, 0)))
    );
    assert_eq!(
        charges(&realm, g, banked).await,
        Some(([0, -4, 0, 0, 0], (BAG_PLAYER, BANK_START)))
    );
    let row = stored(&realm, g).await;
    let saved = row
        .items
        .iter()
        .find(|i| i.guid == guid::counter(carried))
        .expect("saved");
    assert_eq!(saved.charges, Some(SPENT));

    // Saved without charges: the template's.
    let mut old = saved.clone();
    old.charges = None;
    let loaded = crate::game::map::player::Item::from_row(&realm.data, &old, g);
    assert_eq!(
        loaded.fields.get_u32(ITEM_FIELD_SPELL_CHARGES) as i32,
        3,
        "the template's first"
    );
    assert_eq!(
        loaded.fields.get_u32(ITEM_FIELD_SPELL_CHARGES + 1) as i32,
        -5,
        "and second"
    );
}

/// Mailed: the mail keeps the charges, the mailbox shows them, and the
/// item taken has them.
#[tokio::test]
async fn charges_are_kept_through_a_mail_sent_and_taken() {
    let realm = harness::realm_with(data());
    let mut a = player(&realm, "Alice").await;
    let mut b = player(&realm, "Bob").await;
    let wand = spent_wand(&realm, a.guid).await;
    a.send(CMSG_SEND_MAIL {
        mailbox: Guid::zero(),
        receiver: "Bob".into(),
        subject: "a wand".into(),
        body: String::new(),
        unknown1: 0,
        unknown2: 0,
        item: wand,
        money: Gold::new(0),
        cash_on_delivery_amount: 0,
        unknown3: 0,
        unknown4: 0,
    })
    .await;
    let id = a
        .wait_for("SMSG_SEND_MAIL_RESULT", |m| match m {
            ServerOpcodeMessage::SMSG_SEND_MAIL_RESULT(r) => Some(r.mail_id),
            _ => None,
        })
        .await;
    let mail = stored_mail(&realm, id).await.expect("the mail stored");
    assert_eq!(mail.item.and_then(|i| i.charges), Some(SPENT));
    b.send(CMSG_GET_MAIL_LIST {
        mailbox: Guid::zero(),
    })
    .await;
    let shown = b
        .wait_for("SMSG_MAIL_LIST_RESULT", |m| match m {
            ServerOpcodeMessage::SMSG_MAIL_LIST_RESULT(l) => {
                l.mails.first().map(|m| m.item_spell_charges)
            }
            _ => None,
        })
        .await;
    assert_eq!(shown, 1, "the mailbox shows what is left");
    take(&mut b, id).await;
    assert_eq!(
        charges(&realm, b.guid, wand).await.map(|c| c.0),
        Some(SPENT)
    );
    relog(&mut b).await;
    assert_eq!(
        charges(&realm, b.guid, wand).await.map(|c| c.0),
        Some(SPENT),
        "and after a relog"
    );
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

/// The wand put up for 100, bought out at 1000: the auction, which keeps
/// the charges.
async fn put_up(realm: &Arc<Realm>, a: &mut Client, item: Guid) -> u32 {
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
    let held = stored_auction(realm, id).await.expect("the auction stored");
    assert_eq!(held.item.charges, Some(SPENT), "held by the auction house");
    id
}

/// The one mail `to` has: its id.
fn only_mail(realm: &Arc<Realm>, to: Guid) -> u32 {
    let mails = mails_of(realm, to);
    assert_eq!(mails.len(), 1, "one mail");
    assert_eq!(mails[0].item.as_ref().and_then(|i| i.charges), Some(SPENT));
    mails[0].id
}

/// Sold: the auction house lists what is left, and the buyer takes the
/// wand with it.
#[tokio::test]
async fn charges_are_kept_through_an_auction_sold() {
    let realm = harness::realm_with(data());
    let mut a = player(&realm, "Alice").await;
    let mut b = player(&realm, "Bob").await;
    let wand = spent_wand(&realm, a.guid).await;
    let id = put_up(&realm, &mut a, wand).await;
    b.send(CMSG_AUCTION_LIST_ITEMS {
        auctioneer: auctioneer(),
        list_start_item: 0,
        searched_name: String::new(),
        minimum_level: 0,
        maximum_level: 0,
        auction_slot_id: u32::MAX,
        auction_main_category: u32::MAX,
        auction_sub_category: u32::MAX,
        auction_quality: ItemQuality::Poor,
        usable: 0,
    })
    .await;
    let listed = b
        .wait_for("SMSG_AUCTION_LIST_RESULT", |m| match m {
            ServerOpcodeMessage::SMSG_AUCTION_LIST_RESULT(l) => {
                l.auctions.first().map(|i| i.item_charges)
            }
            _ => None,
        })
        .await;
    assert_eq!(listed, 1, "listed with what is left");
    b.clear();
    b.send(CMSG_AUCTION_PLACE_BID {
        auctioneer: auctioneer(),
        auction_id: id,
        price: Gold::new(1_000),
    })
    .await;
    auction_result(&mut b).await;
    let mail = only_mail(&realm, b.guid);
    take(&mut b, mail).await;
    assert_eq!(
        charges(&realm, b.guid, wand).await.map(|c| c.0),
        Some(SPENT)
    );
}

/// Cancelled: the wand comes back by mail with its charges.
#[tokio::test]
async fn charges_are_kept_through_an_auction_cancelled() {
    let realm = harness::realm_with(data());
    let mut a = player(&realm, "Alice").await;
    let wand = spent_wand(&realm, a.guid).await;
    let id = put_up(&realm, &mut a, wand).await;
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
    let mail = only_mail(&realm, a.guid);
    take(&mut a, mail).await;
    assert_eq!(
        charges(&realm, a.guid, wand).await.map(|c| c.0),
        Some(SPENT)
    );
}

/// Expired: the wand comes back by mail with its charges.
#[tokio::test]
async fn charges_are_kept_through_an_auction_expired() {
    let realm = harness::realm_with(data());
    let mut a = player(&realm, "Alice").await;
    let wand = spent_wand(&realm, a.guid).await;
    let id = put_up(&realm, &mut a, wand).await;
    realm
        .social
        .auctions
        .lock()
        .auctions
        .get_mut(&id)
        .unwrap()
        .expires_at = crate::game::social::mail::now() - 1;
    crate::game::social::auction::sweep(&realm);
    let mail = only_mail(&realm, a.guid);
    take(&mut a, mail).await;
    assert_eq!(
        charges(&realm, a.guid, wand).await.map(|c| c.0),
        Some(SPENT)
    );
}

/// Traded: the window shows what is left, and Bob has the wand with it.
#[tokio::test]
async fn charges_are_kept_through_a_trade() {
    type S = SMSG_TRADE_STATUS_TradeStatus;
    let realm = harness::realm_with(data());
    let mut a = player(&realm, "Alice").await;
    let mut b = player(&realm, "Bob").await;
    let wand = spent_wand(&realm, a.guid).await;
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
    assert_eq!(
        charges(&realm, b.guid, wand).await.map(|c| c.0),
        Some(SPENT)
    );
}

/// Sold to a vendor and bought back: the charges with it.
#[tokio::test]
async fn charges_are_kept_through_the_buyback() {
    let realm = harness::realm_with(data());
    let mut a = player(&realm, "Alice").await;
    let g = a.guid;
    let wand = spent_wand(&realm, g).await;
    a.send(CMSG_SELL_ITEM {
        vendor: vendor_guid(),
        item: wand,
        amount: 0,
    })
    .await;
    handled(&mut a).await;
    assert_eq!(
        charges(&realm, g, wand).await,
        Some((SPENT, (BAG_PLAYER, BUYBACK_START))),
        "in the buyback"
    );
    a.send(CMSG_BUYBACK_ITEM {
        guid: vendor_guid(),
        slot: BuybackSlot::Slot1,
    })
    .await;
    handled(&mut a).await;
    assert_eq!(
        charges(&realm, g, wand).await,
        Some((SPENT, (BAG_PLAYER, PACK_START)))
    );
}

/// mangos's `TakeCastItem`: a wand whose positive charges are spent is
/// kept, saved with none, and refused after a relog; a bomb whose negative
/// charge is spent is used up.
#[tokio::test]
async fn an_item_with_its_charges_spent_is_kept_or_used_up_as_mangos_has_it() {
    let realm = harness::realm_with(data());
    let mut a = player(&realm, "Alice").await;
    let g = a.guid;
    let wand = give_at(&realm, g, WAND, (BAG_PLAYER, PACK_START), None).await;
    let bomb = give_at(&realm, g, BOMB, (BAG_PLAYER, PACK_START + 1), None).await;
    on_map(&realm, move |m| {
        for _ in 0..3 {
            crate::game::map::items::used(m, g, wand, 10);
        }
        crate::game::map::items::used(m, g, bomb, 12);
    })
    .await;
    assert_eq!(charges(&realm, g, bomb).await, None, "the bomb used up");
    // Each use spends one of each spell's; the wand is kept while its last
    // charged spell has charges, though its first has none.
    assert_eq!(
        charges(&realm, g, wand).await.map(|c| c.0),
        Some([0, -2, 0, 0, 0]),
        "the wand kept with none of its first spell's"
    );
    relog(&mut a).await;
    assert_eq!(
        charges(&realm, g, wand).await.map(|c| c.0),
        Some([0, -2, 0, 0, 0]),
        "and so after a relog"
    );
    let refused = on_map(&realm, move |m| {
        (
            crate::game::map::items::check_use(m, g, wand, 0),
            crate::game::map::items::check_use(m, g, wand, 1),
        )
    })
    .await;
    assert_eq!(
        refused,
        (Some(NO_CHARGES_REMAIN), None),
        "its spent spell refused, the other not"
    );
    // Its last charged spell's two spent: used up.
    let left = on_map(&realm, move |m| {
        crate::game::map::items::used(m, g, wand, 11);
        let one = m.players[&g].item_by_guid(wand).is_some();
        crate::game::map::items::used(m, g, wand, 11);
        (one, m.players[&g].item_by_guid(wand).is_some())
    })
    .await;
    assert_eq!(left, (true, false), "kept with one, gone with none");
}
