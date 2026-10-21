//! Round 12, stage 12k: an auction's mails as mangos-classic's
//! `SendAuctionWonMail` and `SendAuctionSuccessfulMail` write them. Their
//! subject names the item's random property (the client's "of the Bear"),
//! their body is the invoice the client reads (`GetInboxInvoiceInfo`: the
//! other party, the bid and buyout, and the seller's deposit and cut), and
//! the seller is paid at once. Bodies were empty, the property 0, and the
//! takings an hour late.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::harness::Client;
use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit};
use crate::game::data::{GameData, ItemTemplate};
use crate::game::guid;
use crate::game::social::auction::Auction;
use crate::game::social::mail;

const AUCTIONEER: u32 = 60_995;
const BLADE: u32 = 60_996;
const BEAR: u32 = 1_017;

fn data(d: &mut GameData) {
    let mut auctioneer = d.creature_templates[&KOBOLD].clone();
    auctioneer.entry = AUCTIONEER;
    auctioneer.npc_flags = 0x1000;
    // Stormwind's.
    auctioneer.faction = 12;
    d.creature_templates.insert(AUCTIONEER, auctioneer);
    d.items.insert(
        BLADE,
        ItemTemplate {
            entry: BLADE,
            class: 2,
            name: "Blade".into(),
            display_id: 1000,
            stackable: 1,
            allowable_class: -1,
            allowable_race: -1,
            ..Default::default()
        },
    );
}

/// The mail list's entry from the auction house: its subject and text id.
async fn mail_of(c: &mut Client) -> (u32, String, u32, u32) {
    c.clear();
    c.send(CMSG_GET_MAIL_LIST {
        mailbox: Guid::zero(),
    })
    .await;
    c.wait_for("SMSG_MAIL_LIST_RESULT", |m| match m {
        ServerOpcodeMessage::SMSG_MAIL_LIST_RESULT(l) => l.mails.first().map(|x| {
            (
                x.message_id,
                x.subject.clone(),
                x.item_text_id,
                x.checked_timestamp,
            )
        }),
        _ => None,
    })
    .await
}

async fn text_of(c: &mut Client, mail_id: u32, text_id: u32) -> String {
    c.clear();
    c.send(CMSG_ITEM_TEXT_QUERY {
        item_text_id: text_id,
        mail_id,
        unknown1: 0,
    })
    .await;
    c.wait_for("SMSG_ITEM_TEXT_QUERY_RESPONSE", |m| match m {
        ServerOpcodeMessage::SMSG_ITEM_TEXT_QUERY_RESPONSE(t) => Some(t.text.clone()),
        _ => None,
    })
    .await
}

/// Bob buys Alice's Blade of the Bear out: his mail names the blade of the
/// Bear and carries the invoice of Alice, the bid and the buyout; hers,
/// there at once, the money and the invoice of Bob, the bid, the buyout,
/// her deposit and the house's cut. Neither is copied onto a letter.
#[tokio::test]
async fn an_auctions_mails_carry_their_invoice_and_name_the_items_suffix() {
    // Mail between accounts an hour on its way, as the shipped config.
    let mut config = crate::game::harness::test_config();
    config.mail_delay = std::time::Duration::from_secs(3_600);
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = crate::game::Realm::new(config, store, startup, data_with(data), None);
    let mut a = Client::connect(&realm, "P12KAUCA");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "P12KAUCB");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let map = world(&realm);
    let auctioneer = map
        .query(move |m| {
            kit::ready(m, gb, 10, &[]);
            m.players.get_mut(&gb).unwrap().set_money(10_000);
            kit::spawn(m, gb, AUCTIONEER, 10, 3.0)
        })
        .await
        .unwrap();
    b.sees(auctioneer).await;
    let house = map
        .query(move |m| crate::game::map::auction::house_of(m, auctioneer) as u32)
        .await
        .unwrap();
    realm.social.auctions.lock().auctions.insert(
        41,
        Auction {
            id: 41,
            market: 2,
            house,
            item: crate::game::store::ItemRow {
                guid: 9_041,
                entry: BLADE,
                count: 1,
                data: format!("rp:{BEAR}"),
                ..Default::default()
            },
            owner: guid::counter(ga),
            start_bid: 500,
            buyout: 2_000,
            deposit: 60,
            expires_at: mail::now() + 3_600,
            ..Default::default()
        },
    );
    b.send(CMSG_AUCTION_PLACE_BID {
        auctioneer,
        auction_id: 41,
        price: Gold::new(2_000),
    })
    .await;
    b.wait_for("SMSG_AUCTION_COMMAND_RESULT", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_AUCTION_COMMAND_RESULT(_)).then_some(())
    })
    .await;
    // Bob's: won (1), from Alice.
    let (id, subject, text, flags) = mail_of(&mut b).await;
    assert_eq!(subject, format!("{BLADE}:{BEAR}:1"));
    assert_eq!(
        flags & mail::COPIED,
        mail::COPIED,
        "not copied onto a letter"
    );
    assert_eq!(
        text_of(&mut b, id, text).await,
        format!("{:>16x}:2000:2000", guid::counter(ga))
    );
    // Alice's: sold (2), at once, 2000 less the cut of 5% and her deposit
    // back.
    let (id, subject, text, _) = mail_of(&mut a).await;
    assert_eq!(subject, format!("{BLADE}:{BEAR}:2"));
    assert_eq!(
        text_of(&mut a, id, text).await,
        format!("{:>16x}:2000:2000:60:100", guid::counter(gb))
    );
    let money = realm
        .social
        .mail
        .lock()
        .mails
        .get(&id)
        .map(|m| (m.money, m.deliver_at <= mail::now()));
    assert_eq!(money, Some((2_000 - 100 + 60, true)));
}

/// A search for a suffix finds the items of it alone, and the matches
/// page in the order of their auctions, as before its names were kept.
#[tokio::test]
async fn a_search_finds_a_suffix_and_pages_in_order() {
    let realm = fast_realm(|d| {
        data(d);
        d.crafts
            .random_property_names
            .insert(BEAR, "of the Bear".into());
    });
    let mut b = Client::connect(&realm, "P12KSEARCH");
    let gb = b.enter("Seeker", Race::Human, Class::Warrior).await;
    let map = world(&realm);
    let auctioneer = map
        .query(move |m| kit::spawn(m, gb, AUCTIONEER, 10, 3.0))
        .await
        .unwrap();
    b.sees(auctioneer).await;
    let house = map
        .query(move |m| crate::game::map::auction::house_of(m, auctioneer) as u32)
        .await
        .unwrap();
    {
        let mut auctions = realm.social.auctions.lock();
        for id in [70u32, 3, 55, 12, 90] {
            let bear = id % 2 == 1;
            auctions.auctions.insert(
                id,
                Auction {
                    id,
                    market: 2,
                    house,
                    item: crate::game::store::ItemRow {
                        guid: 9_100 + id,
                        entry: BLADE,
                        count: 1,
                        data: if bear {
                            format!("rp:{BEAR}")
                        } else {
                            String::new()
                        },
                        ..Default::default()
                    },
                    owner: 999,
                    start_bid: 10,
                    expires_at: mail::now() + 3_600,
                    ..Default::default()
                },
            );
        }
    }
    let search = async |b: &mut Client, name: &str| {
        b.clear();
        b.send(CMSG_AUCTION_LIST_ITEMS {
            auctioneer,
            list_start_item: 0,
            searched_name: name.into(),
            minimum_level: 0,
            maximum_level: 0,
            auction_slot_id: u32::MAX,
            auction_main_category: u32::MAX,
            auction_sub_category: u32::MAX,
            auction_quality: ItemQuality::Poor,
            usable: 0,
        })
        .await;
        b.wait_for("SMSG_AUCTION_LIST_RESULT", |m| match m {
            ServerOpcodeMessage::SMSG_AUCTION_LIST_RESULT(r) => {
                Some(r.auctions.iter().map(|a| a.id).collect::<Vec<_>>())
            }
            _ => None,
        })
        .await
    };
    assert_eq!(search(&mut b, "").await, vec![3, 12, 55, 70, 90]);
    assert_eq!(search(&mut b, "of the BEAR").await, vec![3, 55]);
    assert_eq!(search(&mut b, "blade").await, vec![3, 12, 55, 70, 90]);
}
