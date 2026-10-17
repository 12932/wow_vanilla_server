//! Thistle Tea's inbound and message tests (Phase 3, `game/inbound`,
//! `game/network`) for what NPCs do, through the protocol on a world made
//! for them: a vendor's standing and stock.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::misc::FactionDef;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapKey};

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const STORMWIND: u32 = 72;
const VENDOR: u32 = 2001;
const JERKY: u32 = 117;
const TABARD: u32 = 50_100;
const LIMITED: u32 = 50_101;

fn spawn(guid: u32, entry: u32, dx: f32) -> CreatureSpawn {
    CreatureSpawn {
        guid,
        entries: vec![entry],
        map: 0,
        x: START.0 + dx,
        y: START.1,
        z: START.2,
        respawn_min: 300,
        respawn_max: 300,
        ..Default::default()
    }
}

fn creature(entry: u32, name: &str, faction: u32, npc_flags: u32) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: name.into(),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: 2,
        level_max: 2,
        faction,
        npc_flags,
        unit_class: 1,
        health_multiplier: 1.0,
        health_max: 40,
        ..Default::default()
    }
}

fn item(entry: u32, name: &str) -> ItemTemplate {
    ItemTemplate {
        entry,
        name: name.into(),
        allowable_class: -1,
        allowable_race: -1,
        stackable: 1,
        buy_count: 1,
        ..Default::default()
    }
}

fn data() -> GameData {
    let mut d = GameData::empty();
    d.misc.factions.insert(
        STORMWIND,
        FactionDef {
            id: STORMWIND,
            index: 0,
            name: "Stormwind".into(),
            variants: [(1, 0, 0, 0x11), (0, 0, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0)],
            ..Default::default()
        },
    );
    d.misc.faction_by_index.insert(0, STORMWIND);
    d.faction_templates.insert(
        12,
        FactionTemplate {
            id: 12,
            faction: STORMWIND,
            ours: 2,
            friend_mask: 2,
            ..Default::default()
        },
    );
    d.creature_templates
        .insert(VENDOR, creature(VENDOR, "Vendor", 12, 0x4));
    d.creature_spawns.insert(0, vec![spawn(11, VENDOR, 2.0)]);
    let sold = |slot, item, max_count| VendorItem {
        slot,
        item,
        max_count,
        restock_secs: 3600,
        ..Default::default()
    };
    d.vendors.insert(
        VENDOR,
        vec![sold(1, JERKY, 0), sold(2, TABARD, 0), sold(3, LIMITED, 2)],
    );
    let mut jerky = item(JERKY, "Tough Jerky");
    jerky.buy_price = 25;
    jerky.stackable = 20;
    d.items.insert(JERKY, jerky);
    // Friendly with Stormwind (rank 4) to buy.
    let mut tabard = item(TABARD, "Stormwind Tabard");
    tabard.buy_price = 1;
    tabard.required_reputation_faction = STORMWIND;
    tabard.required_reputation_rank = 4;
    d.items.insert(TABARD, tabard);
    let mut limited = item(LIMITED, "Rare Recipe");
    limited.buy_price = 1;
    d.items.insert(LIMITED, limited);
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

fn money(m: &Map, g: Guid) -> u32 {
    m.players[&g].fields.get_u32(PLAYER_FIELD_COINAGE)
}

async fn bought(c: &mut Client) -> Result<u32, BuyResult> {
    c.wait_for("SMSG_BUY_ITEM or SMSG_BUY_FAILED", |m| match m {
        ServerOpcodeMessage::SMSG_BUY_ITEM(b) => Some(Ok(b.amount_bought)),
        ServerOpcodeMessage::SMSG_BUY_FAILED(f) => Some(Err(f.result)),
        _ => None,
    })
    .await
}

async fn listed(c: &mut Client, vendor: Guid) -> Vec<(u32, u32)> {
    c.send(CMSG_LIST_INVENTORY { guid: vendor }).await;
    c.wait_for("SMSG_LIST_INVENTORY", |m| match m {
        ServerOpcodeMessage::SMSG_LIST_INVENTORY(l) => {
            Some(l.items.iter().map(|i| (i.item, i.max_items)).collect())
        }
        _ => None,
    })
    .await
}

/// Thistle Tea's cmsg_buy_item_reputation_test and smsg_list_inventory_test:
/// an item asking for a standing the buyer hasn't is refused with
/// `REPUTATION_REQUIRE`; an Honored buyer's discount is on the whole
/// purchase, rounded once; and limited stock sold out lists as none left,
/// not as unlimited.
#[tokio::test]
async fn a_vendor_asks_for_standing_discounts_the_whole_and_runs_out() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let vendor = guid::unit(VENDOR, 11);
    a.sees(vendor).await;
    on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().set_money(1_000)
    })
    .await;
    // Neutral with Stormwind: the tabard is refused, and nothing paid.
    a.send(CMSG_BUY_ITEM {
        vendor,
        item: TABARD,
        amount: 1,
        unknown1: 0,
    })
    .await;
    assert_eq!(bought(&mut a).await, Err(BuyResult::ReputationRequire));
    assert_eq!(on_map(&realm, move |m| money(m, g)).await, 1_000);
    // Honored: two jerkies at 25 each are 45, the tenth off 50 rounded
    // once.
    on_map(&realm, move |m| {
        crate::game::map::reputation::modify(m, g, STORMWIND, 9_000, false)
    })
    .await;
    a.send(CMSG_BUY_ITEM {
        vendor,
        item: JERKY,
        amount: 2,
        unknown1: 0,
    })
    .await;
    assert_eq!(bought(&mut a).await, Ok(2));
    assert_eq!(on_map(&realm, move |m| money(m, g)).await, 955);
    a.send(CMSG_BUY_ITEM {
        vendor,
        item: TABARD,
        amount: 1,
        unknown1: 0,
    })
    .await;
    assert_eq!(bought(&mut a).await, Ok(1), "friendly enough now");
    // The rare recipe's two bought: none left, not unlimited.
    assert!(listed(&mut a, vendor).await.contains(&(LIMITED, 2)));
    a.send(CMSG_BUY_ITEM {
        vendor,
        item: LIMITED,
        amount: 2,
        unknown1: 0,
    })
    .await;
    assert_eq!(bought(&mut a).await, Ok(2));
    let list = listed(&mut a, vendor).await;
    assert!(list.contains(&(LIMITED, 0)), "{list:?}");
    assert!(list.contains(&(JERKY, u32::MAX)), "unlimited: {list:?}");
}
