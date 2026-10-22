//! The bug hunt's items, quests and loot, through the protocol: no trade
//! with oneself, a disenchant's window answered by the loot it holds, no
//! item opened or wrapped from the buyback or (away from a banker) the
//! bank, no credit for a failed quest, and a creature's pockets let go as
//! it leaves.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::*;
use crate::game::harness::{self, Client};
use crate::game::map::inventory::{BANK_START, BUYBACK_START};
use crate::game::map::loot::{self, Loot, LootKind, LootSlot};
use crate::game::map::{Map, MapKey};
use crate::game::store::BAG_PLAYER;

const CLAM: u32 = 9_871;
const PEARL: u32 = 9_872;
const DUST: u32 = 9_873;
const SHARD: u32 = 9_874;
const PAPER: u32 = 5_042;
const GIFT: u32 = 5_043;
const TRINKET: u32 = 9_875;
const BOAR: u32 = 9_876;
const BOAR_QUEST: u32 = 9_877;

const NONE_WITHIN: Duration = Duration::from_millis(300);

fn item(entry: u32, name: &str, stackable: u32) -> ItemTemplate {
    ItemTemplate {
        entry,
        name: name.into(),
        display_id: 1000,
        allowable_class: -1,
        allowable_race: -1,
        stackable,
        buy_count: 1,
        sell_price: 10,
        ..Default::default()
    }
}

fn data() -> GameData {
    let mut d = GameData::empty();
    let mut clam = item(CLAM, "Clam", 10);
    clam.flags = crate::game::map::items_misc::ITEM_FLAG_LOOTABLE;
    let mut paper = item(PAPER, "Wrapping Paper", 20);
    paper.flags = crate::game::map::items_misc::ITEM_FLAG_WRAPPER;
    for t in [
        clam,
        item(PEARL, "Pearl", 20),
        item(DUST, "Dust", 20),
        item(SHARD, "Shard", 20),
        paper,
        item(GIFT, "Gift", 1),
        item(TRINKET, "Trinket", 1),
    ] {
        d.items.insert(t.entry, t);
    }
    d.loot.item.insert(
        CLAM,
        vec![LootEntry {
            item: PEARL,
            chance: 100.0,
            min: 1,
            max: 1,
            ..Default::default()
        }],
    );
    d.creature_templates.insert(
        BOAR,
        CreatureTemplate {
            entry: BOAR,
            name: "Boar".into(),
            display_ids: [903, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            display_scales: [1.0, 0.0, 0.0, 0.0],
            level_min: 2,
            level_max: 2,
            faction: 7,
            unit_class: 1,
            health_max: 50,
            ..Default::default()
        },
    );
    let mut quest = crate::game::data::npc::Quest {
        id: BOAR_QUEST,
        ..Default::default()
    };
    quest.required_kills[0] = (BOAR as i32, 3);
    d.npc.quests.insert(BOAR_QUEST, quest);
    d
}

/// Players, not GMs.
fn realm() -> Arc<Realm> {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(cfg, store, startup, data(), None)
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

async fn alice(realm: &Arc<Realm>) -> Client {
    let mut a = Client::connect(realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Rogue).await;
    on_map(realm, move |m| {
        let d = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        p.items.clear();
        p.refresh_inventory(&d);
    })
    .await;
    a
}

/// An item of `entry` for `g`, moved to `at` if given: its guid and where
/// it is.
async fn give(
    realm: &Arc<Realm>,
    g: Guid,
    entry: u32,
    count: u32,
    at: Option<(u8, u8)>,
) -> (Guid, (u8, u8)) {
    on_map(realm, move |m| {
        crate::game::map::inventory::add_item(m, g, entry, count, false).expect("room");
        let d = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        let i = p.items.iter().rposition(|i| i.entry == entry).unwrap();
        if let Some(at) = at {
            (p.items[i].bag, p.items[i].slot) = at;
            p.refresh_inventory(&d);
        }
        (p.items[i].guid, (p.items[i].bag, p.items[i].slot))
    })
    .await
}

async fn loot_shown(c: &mut Client) -> bool {
    c.gets(NONE_WITHIN, |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOOT_RESPONSE(_)).then_some(())
    })
    .await
    .is_some()
        || !c
            .take_frames(crate::game::map::loot::SMSG_LOOT_RESPONSE)
            .is_empty()
}

/// mangos's `HandleInitiateTradeOpcode`: a trade with oneself is refused,
/// busy. Both sides would be the same items, taken out twice, the second
/// time whatever had slid into their places.
#[tokio::test]
async fn a_player_cant_trade_with_themselves() {
    let realm = realm();
    let mut a = alice(&realm).await;
    let g = a.guid;
    a.send(CMSG_INITIATE_TRADE { guid: g }).await;
    a.wait_for("busy", |m| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS(s) => {
            matches!(s.status, SMSG_TRADE_STATUS_TradeStatus::Busy).then_some(())
        }
        _ => None,
    })
    .await;
    let trading = on_map(&realm, move |m| m.players[&g].trade.is_some()).await;
    assert!(!trading);
}

/// A disenchant's window is an item's, but its loot is the map's: the
/// client's take and one release reach it, the shard taken and the dust
/// left stored, the loot let go.
#[tokio::test]
async fn a_disenchants_window_takes_and_releases_through_the_protocol() {
    let realm = realm();
    let mut a = alice(&realm).await;
    let g = a.guid;
    let item = crate::game::guid::item(990_101);
    on_map(&realm, move |m| {
        let slot = |entry, count| LootSlot {
            item: entry,
            count,
            ..Default::default()
        };
        loot::show_item_loot(
            m,
            g,
            item,
            Loot {
                items: vec![slot(SHARD, 1), slot(DUST, 3)],
                kind: LootKind::Disenchant,
                owner: Some(g),
                ..Default::default()
            },
        );
    })
    .await;
    a.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
    a.wait_for("the shard taken", |m| match m {
        ServerOpcodeMessage::SMSG_LOOT_REMOVED(r) => (r.slot == 0).then_some(()),
        _ => None,
    })
    .await;
    a.send(CMSG_LOOT_RELEASE { guid: item }).await;
    a.wait_for("released", |m| match m {
        ServerOpcodeMessage::SMSG_LOOT_RELEASE_RESPONSE(r) => (r.guid == item).then_some(()),
        _ => None,
    })
    .await;
    let had = on_map(&realm, move |m| {
        let p = &m.players[&g];
        (
            p.count_of(SHARD),
            p.count_of(DUST),
            m.loot_state.item_loot.contains_key(&item),
        )
    })
    .await;
    assert_eq!(had, (1, 3, false));
}

/// A clam sold, waiting in the buyback, can't be opened: the sale's money
/// is had already. One in the bags opens.
#[tokio::test]
async fn a_clam_in_the_buyback_doesnt_open() {
    let realm = realm();
    let mut a = alice(&realm).await;
    let g = a.guid;
    give(&realm, g, CLAM, 1, Some((BAG_PLAYER, BUYBACK_START))).await;
    a.clear();
    a.send(CMSG_OPEN_ITEM {
        bag_index: BAG_PLAYER,
        slot: BUYBACK_START,
    })
    .await;
    assert!(!loot_shown(&mut a).await, "no loot from the buyback");
    let open = on_map(&realm, move |m| m.players[&g].misc.item_loot.is_some()).await;
    assert!(!open);
    let (_, at) = give(&realm, g, CLAM, 1, None).await;
    a.send(CMSG_OPEN_ITEM {
        bag_index: at.0,
        slot: at.1,
    })
    .await;
    let open = {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            if on_map(&realm, move |m| m.players[&g].misc.item_loot.is_some()).await {
                break true;
            }
            if tokio::time::Instant::now() > deadline {
                break false;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    assert!(open, "one carried opens");
}

/// Nor is an item in the buyback wrapped, nor wrapped with paper there.
#[tokio::test]
async fn nothing_in_the_buyback_is_wrapped() {
    let realm = realm();
    let a = alice(&realm).await;
    let g = a.guid;
    let (_, paper) = give(&realm, g, PAPER, 5, None).await;
    let (trinket, sold) = give(&realm, g, TRINKET, 1, Some((BAG_PLAYER, BUYBACK_START))).await;
    a.send(CMSG_WRAP_ITEM {
        gift_bag_index: paper.0,
        gift_slot: paper.1,
        item_bag_index: sold.0,
        item_slot: sold.1,
    })
    .await;
    let (_, carried) = give(&realm, g, TRINKET, 1, None).await;
    on_map(&realm, move |m| {
        let d = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        let i = p.items.iter().position(|i| i.entry == PAPER).unwrap();
        (p.items[i].bag, p.items[i].slot) = (BAG_PLAYER, BUYBACK_START + 1);
        p.refresh_inventory(&d);
    })
    .await;
    a.send(CMSG_WRAP_ITEM {
        gift_bag_index: BAG_PLAYER,
        gift_slot: BUYBACK_START + 1,
        item_bag_index: carried.0,
        item_slot: carried.1,
    })
    .await;
    harness::an_update(&realm.maps.get(&realm, MapKey::world(0))).await;
    let (gifts, paper_left, sold_entry) = on_map(&realm, move |m| {
        let p = &m.players[&g];
        let sold = p.item_by_guid(trinket).map(|i| p.items[i].entry);
        let paper: u32 = p
            .items
            .iter()
            .filter(|i| i.entry == PAPER)
            .map(|i| i.count())
            .sum();
        (p.count_of(GIFT), paper, sold)
    })
    .await;
    assert_eq!((gifts, paper_left, sold_entry), (0, 5, Some(TRINKET)));
}
