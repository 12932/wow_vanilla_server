//! Round 7, against Thistle Tea's `test/game/core/auction`
//! (docs/parity/matrix.csv): the auction house's search filters, and what
//! a sale refused and a bid placed take of a player's money and bags.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::fields::PLAYER_FIELD_COINAGE;
use crate::game::harness::{self, Client};
use crate::game::social::auction::Auction;
use crate::game::tests_p3_entity::on;
use crate::game::tests_p3_items::{AXE, ROBE, auction_data, auctioneer};

/// What a search finds: the auctions' ids.
#[allow(clippy::too_many_arguments)]
async fn search(
    a: &mut Client,
    name: &str,
    class: u32,
    subclass: u32,
    slot: u32,
    quality: ItemQuality,
    levels: (u8, u8),
) -> Vec<u32> {
    a.clear();
    a.send(CMSG_AUCTION_LIST_ITEMS {
        auctioneer: auctioneer(),
        list_start_item: 0,
        searched_name: name.into(),
        minimum_level: levels.0,
        maximum_level: levels.1,
        auction_slot_id: slot,
        auction_main_category: class,
        auction_sub_category: subclass,
        auction_quality: quality,
        usable: 0,
    })
    .await;
    a.wait_for("SMSG_AUCTION_LIST_RESULT", |m| match m {
        ServerOpcodeMessage::SMSG_AUCTION_LIST_RESULT(l) => {
            let mut ids: Vec<u32> = l.auctions.iter().map(|x| x.id).collect();
            ids.sort_unstable();
            Some(ids)
        }
        _ => None,
    })
    .await
}

/// A Silk Robe (cloth, a robe, rare, level 20) and an axe up for auction,
/// at the goblins'.
fn put_up(realm: &crate::game::Realm) {
    let now = crate::game::social::mail::now();
    let mut auctions = realm.social.auctions.lock();
    for (id, entry) in [(1, ROBE), (2, AXE)] {
        auctions.auctions.insert(
            id,
            Auction {
                id,
                market: 7,
                house: 7,
                item: crate::game::store::ItemRow {
                    guid: 800_000 + id,
                    entry,
                    count: 1,
                    ..Default::default()
                },
                owner: 99,
                start_bid: 100,
                buyout: 1000,
                expires_at: now + 7200,
                ..Default::default()
            },
        );
    }
}

/// Thistle Tea's `Query.search/5` "filters localized names, categories,
/// robes, minimum quality, and level bounds": a search for silk, cloth
/// armor, the chest's slot (a robe's too, as mangos's
/// `BuildListAuctionItems` has it), uncommon or better, levels 10 to 30,
/// finds the robe; another name, class, subclass, slot, a higher quality or
/// levels it is outside of find nothing.
#[tokio::test]
async fn the_auction_search_filters_by_name_class_slot_quality_and_level() {
    let realm = harness::realm_with(auction_data());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    a.sees(auctioneer()).await;
    put_up(&realm);
    let q = ItemQuality::Uncommon;
    assert_eq!(
        search(&mut a, "silk", 4, 1, 5, q, (10, 30)).await,
        vec![1],
        "the robe"
    );
    for (name, class, subclass, slot, quality, levels) in [
        ("axe", 4, 1, 5, q, (10, 30)),
        ("silk", 2, 1, 5, q, (10, 30)),
        ("silk", 4, 2, 5, q, (10, 30)),
        ("silk", 4, 1, 1, q, (10, 30)),
        ("silk", 4, 1, 5, ItemQuality::Epic, (10, 30)),
        ("silk", 4, 1, 5, q, (21, 30)),
        ("silk", 4, 1, 5, q, (0, 19)),
    ] {
        assert!(
            search(&mut a, name, class, subclass, slot, quality, levels)
                .await
                .is_empty(),
            "{name} {class} {subclass} {slot} {quality:?} {levels:?}"
        );
    }
    let all = search(
        &mut a,
        "",
        u32::MAX,
        u32::MAX,
        u32::MAX,
        ItemQuality::Poor,
        (0, 0),
    )
    .await;
    assert_eq!(all, vec![1, 2], "no filter, both");
}

/// Thistle Tea's `AuctionInventory.plan/3` "rejects a missing item or fee
/// without producing a partial change" and "reserves bid money without
/// moving inventory": a sale of an item not in the bags, or one whose
/// deposit can't be paid, is refused, the money and the item where they
/// were; a bid takes its money and nothing from the bags.
#[tokio::test]
async fn a_refused_sale_changes_nothing_and_a_bid_takes_only_its_money() {
    let mut d = auction_data();
    d.items.get_mut(&ROBE).unwrap().sell_price = 10_000;
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.sees(auctioneer()).await;
    put_up(&realm);
    let map = realm.registry.online(g).unwrap().map();
    let robe = on(&map, move |m| {
        crate::game::map::inventory::add_item(m, g, ROBE, 1, false).expect("room");
        let p = m.players.get_mut(&g).unwrap();
        p.fields.set_u32(PLAYER_FIELD_COINAGE, 100);
        p.items.iter().find(|i| i.entry == ROBE).unwrap().guid
    })
    .await;
    let state = move |m: &mut crate::game::map::Map| {
        let p = &m.players[&g];
        (
            p.money(),
            p.items.iter().filter(|i| i.entry == ROBE).count(),
        )
    };
    let sell = |item| CMSG_AUCTION_SELL_ITEM {
        auctioneer: auctioneer(),
        item,
        starting_bid: 100,
        buyout: 200,
        auction_duration_in_minutes: 1440,
    };
    for item in [Guid::new(0x4000_0000_0099_9999), robe] {
        a.clear();
        a.send(sell(item)).await;
        a.wait_for("SMSG_AUCTION_COMMAND_RESULT", |m| match m {
            ServerOpcodeMessage::SMSG_AUCTION_COMMAND_RESULT(_) => Some(()),
            _ => None,
        })
        .await;
        assert_eq!(on(&map, state).await, (100, 1), "nothing taken for {item}");
    }
    let up = realm.social.auctions.lock().auctions.len();
    assert_eq!(up, 2, "nothing put up");
    // A bid of a hundred on the axe: all her money, the bags as they were.
    a.send(CMSG_AUCTION_PLACE_BID {
        auctioneer: auctioneer(),
        auction_id: 2,
        price: Gold::new(100),
    })
    .await;
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    while on(&map, state).await.0 == 100 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the bid never taken"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(on(&map, state).await, (0, 1), "its money, and nothing else");
    let bid = realm.social.auctions.lock().auctions[&2].bid;
    assert_eq!(bid, 100);
}
