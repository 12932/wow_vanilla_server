//! Round 12, stage 12k: vendors and repairs as mangos-classic has them. A
//! vendor lists no item of another race, nor one bound when picked up of
//! another class (`SendListInventory`); a PvP rank of Sergeant takes a
//! tenth off at a friendly vendor, with Honored's (`GetReputationPriceDiscount`);
//! "Repair All" mends item by item, worn first, while the money lasts
//! (`DurabilityRepairAll`), not all or nothing.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::harness::Client;
use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit};
use crate::game::data::{FactionTemplate, GameData, ItemTemplate, VendorItem};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::player::Item;
use crate::game::store::BAG_PLAYER;

const VENDOR: u32 = 60_980;
/// Any race's; the Night Elves' alone; a mage's bound when picked up; a
/// mage's not bound.
const ANYONES: u32 = 60_981;
const ELVES: u32 = 60_982;
const MAGES_BOUND: u32 = 60_983;
const MAGES_FREE: u32 = 60_984;
const BOOTS: u32 = 60_985;
const GLOVES: u32 = 60_986;
/// The faction template humans and the vendor share.
const HUMANS: u32 = 1;

fn data(d: &mut GameData) {
    let mut vendor = d.creature_templates[&KOBOLD].clone();
    vendor.entry = VENDOR;
    vendor.npc_flags = 0x4 | crate::game::map::vendor::NPC_FLAG_REPAIR;
    vendor.faction = HUMANS;
    d.creature_templates.insert(VENDOR, vendor);
    d.faction_templates.insert(
        HUMANS,
        FactionTemplate {
            id: HUMANS,
            faction: 1,
            ours: 1,
            friend_mask: 1,
            ..Default::default()
        },
    );
    let good = |entry: u32, race: i32, class: i32, bonding: u32| ItemTemplate {
        entry,
        class: 15,
        name: format!("Good {entry}"),
        display_id: 1000,
        stackable: 1,
        buy_price: 1_000,
        allowable_race: race,
        allowable_class: class,
        bonding,
        ..Default::default()
    };
    // Night Elves are race 4, mages class 8.
    for t in [
        good(ANYONES, -1, -1, 0),
        good(ELVES, 1 << 3, -1, 0),
        good(MAGES_BOUND, -1, 1 << 7, 1),
        good(MAGES_FREE, -1, 1 << 7, 2),
    ] {
        d.items.insert(t.entry, t);
    }
    d.vendors.insert(
        VENDOR,
        [ANYONES, ELVES, MAGES_BOUND, MAGES_FREE]
            .into_iter()
            .enumerate()
            .map(|(i, item)| VendorItem {
                slot: i as u32 + 1,
                item,
                ..Default::default()
            })
            .collect(),
    );
    let armor = |entry: u32, inventory_type: u32| ItemTemplate {
        entry,
        class: 4,
        subclass: 1,
        name: format!("Armor {entry}"),
        display_id: 1000,
        inventory_type,
        item_level: 10,
        max_durability: 50,
        stackable: 1,
        ..Default::default()
    };
    d.items.insert(BOOTS, armor(BOOTS, 8));
    d.items.insert(GLOVES, armor(GLOVES, 10));
    // A point of cloth at level 10 costs ten copper.
    let mut costs = vec![0u32; 29];
    costs[22] = 10;
    d.durability_costs.insert(10, costs);
    d.durability_quality.insert(2, 1.0);
}

async fn listed(c: &mut Client, vendor: Guid) -> Vec<(u32, u32)> {
    c.clear();
    c.send(CMSG_LIST_INVENTORY { guid: vendor }).await;
    c.wait_for("SMSG_LIST_INVENTORY", |m| match m {
        ServerOpcodeMessage::SMSG_LIST_INVENTORY(l) if l.vendor == vendor => Some(
            l.items
                .iter()
                .map(|i| (i.item, i.price.as_int()))
                .collect(),
        ),
        _ => None,
    })
    .await
}

/// A human warrior, no GM, and the vendor before them.
async fn at_vendor(realm: &std::sync::Arc<crate::game::Realm>, account: &str) -> (Client, Guid, Guid) {
    let (mut c, g, map) = human(realm, account, "Shopper", Class::Warrior).await;
    let vendor = map
        .query(move |m| {
            kit::ready(m, g, 10, &[]);
            let p = m.players.get_mut(&g).unwrap();
            p.gm_level = 0;
            p.fields.set_u32(UNIT_FIELD_FACTIONTEMPLATE, HUMANS);
            p.set_money(100_000);
            kit::spawn(m, g, VENDOR, 10, 3.0)
        })
        .await
        .unwrap();
    c.sees(vendor).await;
    (c, g, vendor)
}

/// A human warrior is offered neither the Night Elves' item nor a mage's
/// bound when picked up, but a mage's that isn't bound; and can't buy the
/// Night Elves' by asking for it.
#[tokio::test]
async fn a_vendor_lists_no_other_races_item_nor_another_classs_bound_one() {
    let realm = fast_realm(data);
    let (mut c, g, vendor) = at_vendor(&realm, "P12KRACE").await;
    let items: Vec<u32> = listed(&mut c, vendor).await.iter().map(|i| i.0).collect();
    assert_eq!(items, vec![ANYONES, MAGES_FREE]);
    c.clear();
    c.send(CMSG_BUY_ITEM {
        vendor,
        item: ELVES,
        amount: 1,
        unknown1: 0,
    })
    .await;
    let failed = c
        .wait_for("SMSG_BUY_FAILED", |m| match m {
            ServerOpcodeMessage::SMSG_BUY_FAILED(f) => Some(f.result),
            _ => None,
        })
        .await;
    assert_eq!(failed, BuyResult::CantFindItem);
    let held = world(&realm)
        .query(move |m| m.players[&g].count_of(ELVES))
        .await
        .unwrap();
    assert_eq!(held, 0);
}

/// Sergeant and up (the client's rank 3, the field's 7) pays nine tenths
/// at a vendor friendly to them; a Corporal the full price.
#[tokio::test]
async fn a_sergeant_pays_a_tenth_less_at_a_friendly_vendor() {
    let realm = fast_realm(data);
    let (mut c, g, vendor) = at_vendor(&realm, "P12KRANK").await;
    let price = async |c: &mut Client| {
        listed(c, vendor)
            .await
            .iter()
            .find(|i| i.0 == ANYONES)
            .map(|i| i.1)
    };
    let map = world(&realm);
    let rank = async |r: u8| {
        map.query(move |m| {
            m.players
                .get_mut(&g)
                .unwrap()
                .fields
                .set_byte(PLAYER_BYTES_3, 3, r)
        })
        .await
        .unwrap()
    };
    rank(6).await;
    assert_eq!(price(&mut c).await, Some(1_000));
    rank(7).await;
    assert_eq!(price(&mut c).await, Some(900));
}

fn equip(m: &mut Map, g: Guid, entry: u32, slot: u8) {
    let data = m.realm.data.clone();
    let counter = m
        .realm
        .next_item
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let p = m.players.get_mut(&g).unwrap();
    p.items.retain(|i| !(i.bag == BAG_PLAYER && i.slot == slot));
    p.items.push(Item::new(
        &data,
        guid::item(counter),
        g,
        entry,
        BAG_PLAYER,
        slot,
        1,
    ));
    p.refresh_inventory(&data);
    let i = p.item_at(BAG_PLAYER, slot).unwrap();
    p.items[i].fields.set_u32(ITEM_FIELD_DURABILITY, 40);
}

/// "Repair All" with money for one of two: the boots (worn first, slot 7)
/// are mended, for their hundred copper, and the gloves wait. It mended
/// nothing.
#[tokio::test]
async fn repair_all_mends_what_the_money_pays_for_worn_first() {
    let realm = fast_realm(data);
    let (mut c, g, vendor) = at_vendor(&realm, "P12KREPAIR").await;
    let map = world(&realm);
    map.query(move |m| {
        equip(m, g, GLOVES, 9);
        equip(m, g, BOOTS, 7);
        m.players.get_mut(&g).unwrap().set_money(150);
    })
    .await
    .unwrap();
    c.send(CMSG_REPAIR_ITEM {
        npc: vendor,
        item: Guid::zero(),
    })
    .await;
    let state = || {
        map.query(move |m| {
            let p = &m.players[&g];
            let d = |slot| {
                p.items[p.item_at(BAG_PLAYER, slot).unwrap()]
                    .fields
                    .get_u32(ITEM_FIELD_DURABILITY)
            };
            (d(7), d(9), p.money())
        })
    };
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let s = state().await.unwrap();
        if s != (40, 40, 150) || tokio::time::Instant::now() > deadline {
            assert_eq!(s, (50, 40, 50));
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}
