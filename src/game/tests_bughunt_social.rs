//! The bug hunt's findings on mail, auctions and the social services
//! (`social`), each shown through the protocol.

use std::sync::Arc;
use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapKey};
use crate::game::social::auction::Auction;
use crate::game::social::mail;
use crate::game::store::ItemRow;
use crate::game::tests_p3_items::{AXE, auction_data, auctioneer};

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

/// An auction of an axe in the goblins' market, by character 99.
fn axe(id: u32, expires_at: i64) -> Auction {
    Auction {
        id,
        market: 7,
        house: 7,
        item: ItemRow {
            guid: 800_000 + id,
            entry: AXE,
            count: 1,
            ..Default::default()
        },
        owner: 99,
        start_bid: 100,
        buyout: 1000,
        expires_at,
        ..Default::default()
    }
}

/// The bidder list looked each auction up in the client's list of ids with
/// a linear scan, under the realm's auctions lock: 16,000 ids against
/// 20,000 auctions held every auction house in the realm for seconds. The
/// ids are sorted, and searched so.
#[tokio::test]
async fn a_long_outbid_list_is_answered_at_once() {
    let realm = harness::realm_with(auction_data());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    a.sees(auctioneer()).await;
    let later = mail::now() + 7200;
    {
        let mut auctions = realm.social.auctions.lock();
        for id in 1..=20_000 {
            auctions.auctions.insert(id, axe(id, later));
        }
    }
    let asked: Vec<u32> = (100_000..116_000).chain([7]).collect();
    a.clear();
    let started = Instant::now();
    a.send(CMSG_AUCTION_LIST_BIDDER_ITEMS {
        auctioneer: auctioneer(),
        start_from_page: 0,
        outbid_item_ids: asked,
    })
    .await;
    let bids = a
        .wait_for("SMSG_AUCTION_BIDDER_LIST_RESULT", |m| match m {
            ServerOpcodeMessage::SMSG_AUCTION_BIDDER_LIST_RESULT(l) => {
                Some(l.auctions.iter().map(|x| x.id).collect::<Vec<_>>())
            }
            _ => None,
        })
        .await;
    let took = started.elapsed();
    assert_eq!(bids, vec![7]);
    assert!(took < Duration::from_millis(500), "{took:?}");
}
