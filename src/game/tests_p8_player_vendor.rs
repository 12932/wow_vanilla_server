//! Round 8's player: selling and buying back.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::MapHandle;
use crate::game::map::inventory::{BUYBACK_START, PACK_START, SLOT_MAIN_HAND};
use crate::game::store::BAG_PLAYER;

const VENDOR: u32 = 20_200;
const SWORD: u32 = 20_201;
const WIDGET: u32 = 20_202;
const SLOW: u32 = 94_200;
const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

fn data() -> GameData {
    let mut d = GameData::empty();
    let item = |entry: u32| ItemTemplate {
        entry,
        name: format!("Item {entry}"),
        display_id: 1000 + entry,
        stackable: 1,
        sell_price: 100,
        allowable_class: -1,
        allowable_race: -1,
        ..Default::default()
    };
    let mut sword = ItemTemplate {
        class: 2,
        subclass: 7,
        inventory_type: 13,
        delay: 2_600,
        ..item(SWORD)
    };
    sword.damages[0] = (30.0, 40.0, 0);
    // Stamina.
    sword.stats[0] = (7, 10);
    d.items.insert(SWORD, sword);
    d.items.insert(WIDGET, item(WIDGET));
    d.creature_templates.insert(
        VENDOR,
        CreatureTemplate {
            entry: VENDOR,
            name: "Merchant".into(),
            display_ids: [903, 0, 0, 0],
            faction: 35,
            npc_flags: crate::game::map::vendor::NPC_FLAG_VENDOR,
            ..Default::default()
        },
    );
    d.creature_spawns.entry(0).or_default().push(CreatureSpawn {
        guid: 1,
        entries: vec![VENDOR],
        map: 0,
        x: START.0 + 2.0,
        y: START.1,
        z: START.2,
        ..Default::default()
    });
    d.spells.cast_times.insert(14, 3000);
    let mut slow = SpellTemplate {
        id: SLOW,
        name: "Slow Cast".into(),
        range_index: 1,
        casting_time_index: 14,
        equipped_item_class: -1,
        ..Default::default()
    };
    slow.effects[0] = SpellEffect {
        effect: 3,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(SLOW, slow);
    d
}

fn vendor() -> Guid {
    guid::unit(VENDOR, 1)
}

async fn setup() -> (std::sync::Arc<crate::game::Realm>, Client, Guid, MapHandle) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(g).unwrap().map();
    map.query(move |m| {
        let d = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        p.items.clear();
        p.refresh_inventory(&d);
    })
    .await
    .unwrap();
    a.sees(vendor()).await;
    a.clear();
    (realm, a, g, map)
}

async fn give(map: &MapHandle, g: Guid, entry: u32, at: (u8, u8)) -> Guid {
    map.query(move |m| {
        let d = m.realm.data.clone();
        let counter = m
            .realm
            .next_item
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let item = guid::item(counter);
        let p = m.players.get_mut(&g).unwrap();
        p.items.push(crate::game::map::player::Item::new(
            &d, item, g, entry, at.0, at.1, 1,
        ));
        p.refresh_inventory(&d);
        p.gear_dirty = true;
        item
    })
    .await
    .unwrap()
}

/// Sell `item`: the refusal, if any.
async fn sell(a: &mut Client, item: Guid) -> Option<SellItemResult> {
    a.clear();
    a.send(CMSG_SELL_ITEM {
        vendor: vendor(),
        item,
        amount: 0,
    })
    .await;
    a.send(CMSG_NAME_QUERY { guid: a.guid }).await;
    a.wait_for("SMSG_NAME_QUERY_RESPONSE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NAME_QUERY_RESPONSE(_)).then_some(())
    })
    .await;
    a.gets(Duration::from_millis(1), |m| match m {
        ServerOpcodeMessage::SMSG_SELL_ITEM(s) if s.item == item => Some(s.result),
        _ => None,
    })
    .await
}

/// Where `item` is.
async fn at(map: &MapHandle, g: Guid, item: Guid) -> Option<(u8, u8)> {
    map.query(move |m| {
        let p = &m.players[&g];
        p.item_by_guid(item)
            .map(|i| (p.items[i].bag, p.items[i].slot))
    })
    .await
    .unwrap()
}

/// The player's health ceiling, main-hand damage and visible weapon.
async fn worn_stats(map: &MapHandle, g: Guid) -> (u32, f32, u32) {
    harness::an_update(map).await;
    map.query(move |m| {
        let f = &m.players[&g].fields;
        (
            f.get_u32(UNIT_FIELD_MAXHEALTH),
            f.get_f32(UNIT_FIELD_MINDAMAGE),
            f.get_u32(PLAYER_VISIBLE_ITEM_1_0 + 12 * u16::from(SLOT_MAIN_HAND)),
        )
    })
    .await
    .unwrap()
}

/// Thistle Tea's buyback_test "quest inventory and equipped bonuses leave
/// with the sale and return only when appropriate": gear worn is sold off
/// the paper doll (mangos's `HandleSellItemOpcode` takes any item of the
/// player's), its stamina and damage and its look going with it; bought
/// back, it comes to the bags, not the hand.
#[tokio::test]
async fn a_worn_sword_sells_off_the_hand_and_comes_back_to_the_bags() {
    let (_realm, mut a, g, map) = setup().await;
    let bare = worn_stats(&map, g).await;
    let sword = give(&map, g, SWORD, (BAG_PLAYER, SLOT_MAIN_HAND)).await;
    let armed = worn_stats(&map, g).await;
    assert!(
        armed.0 > bare.0 && armed.1 > bare.1 && armed.2 == SWORD,
        "{armed:?}"
    );
    assert_eq!(sell(&mut a, sword).await, None, "sold");
    assert_eq!(at(&map, g, sword).await, Some((BAG_PLAYER, BUYBACK_START)));
    assert_eq!(worn_stats(&map, g).await, bare, "its bonuses gone with it");
    a.send(CMSG_BUYBACK_ITEM {
        guid: vendor(),
        slot: BuybackSlot::try_from(u32::from(BUYBACK_START)).unwrap(),
    })
    .await;
    for _ in 0..200 {
        if at(&map, g, sword).await == Some((BAG_PLAYER, PACK_START)) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(
        at(&map, g, sword).await,
        Some((BAG_PLAYER, PACK_START)),
        "to the pack"
    );
    assert_eq!(worn_stats(&map, g).await, bare, "not worn");
}

/// Thistle Tea's buyback_test "selling a casting item interrupts it and
/// logout discards buyback items" and "invalid counts, bank access, active
/// loot, and remote control cannot sell": a sale ends a cast under way
/// (mangos's `InterruptNonMeleeSpells`); nothing sells while the client
/// moves another unit; what was sold is gone after a logout.
#[tokio::test]
async fn a_sale_ends_a_cast_and_the_buyback_ends_at_logout() {
    let (_realm, mut a, g, map) = setup().await;
    let widget = give(&map, g, WIDGET, (BAG_PLAYER, PACK_START)).await;
    let other = give(&map, g, WIDGET, (BAG_PLAYER, PACK_START + 1)).await;
    map.query(move |m| m.players.get_mut(&g).unwrap().spells.push(SLOW))
        .await
        .unwrap();
    a.send(CMSG_CAST_SPELL {
        spell: SLOW,
        targets: crate::game::map::spells::wire_targets(&Default::default()),
    })
    .await;
    let casting = || async {
        map.query(move |m| m.players[&g].caster.casting.is_some())
            .await
            .unwrap()
    };
    for _ in 0..200 {
        if casting().await {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(casting().await, "casting");
    assert_eq!(sell(&mut a, widget).await, None, "sold");
    assert!(!casting().await, "the cast ended by the sale");
    // Moving another unit, nothing sells.
    map.query(move |m| m.players.get_mut(&g).unwrap().owned.mover = Some(guid::unit(VENDOR, 9)))
        .await
        .unwrap();
    assert_eq!(
        sell(&mut a, other).await,
        Some(SellItemResult::CantSellItem)
    );
    map.query(move |m| m.players.get_mut(&g).unwrap().owned.mover = None)
        .await
        .unwrap();
    // Logged out and in, the sold widget is gone for good.
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    a.login(g).await;
    let map = _realm.registry.online(g).unwrap().map();
    assert_eq!(at(&map, g, widget).await, None, "sold, gone");
    assert_eq!(at(&map, g, other).await, Some((BAG_PLAYER, PACK_START + 1)));
}
