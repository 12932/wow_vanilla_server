//! Round 5's player tests, ported from Thistle Tea's
//! `test/game/world/entity/player/vendor_stock_test.exs` (`buy_in_slot/6`,
//! docs/parity/matrix.csv): a purchase dropped on a slot of the backpack,
//! of a bag, or of the gear, and what is refused with nothing spent.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::items::ItemSpell;
use crate::game::data::{CreatureTemplate, GameData, ItemTemplate, VendorItem};
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::inventory::{self, PACK_END, PACK_START};
use crate::game::map::{Map, MapHandle, vendor};
use crate::game::store::BAG_PLAYER;
use crate::game::tests_p3_combat::{BRUTE, HUMANS, setup_with, spawn, template};
use crate::game::tests_p3_entity::on;
use crate::game::tests_p3_player2_ammo::handled;

const CLERK: u32 = 3760;
/// Five to a stack, three to a bundle at two copper, six in stock.
const WARES: u32 = 64_400;
/// A trinket of seven stamina, bound when worn, with a use; six in stock.
const TRINKET: u32 = 64_401;
const TRINKET_USE: u32 = 95_400;
const FILLER: u32 = 64_402;
const BAG: u32 = 64_403;
/// The trinket for level 60, and for a skill of 100 in Engineering.
const HIGH_TRINKET: u32 = 64_404;
const SKILLED_TRINKET: u32 = 64_405;

fn world() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    d.creature_templates.insert(
        CLERK,
        CreatureTemplate {
            faction: HUMANS,
            npc_flags: vendor::NPC_FLAG_VENDOR,
            ..template(CLERK, 7)
        },
    );
    let item = |entry: u32, stack: u32| ItemTemplate {
        entry,
        name: format!("Item {entry}"),
        display_id: 1000,
        stackable: stack,
        allowable_class: -1,
        allowable_race: -1,
        buy_count: 1,
        buy_price: 2,
        ..Default::default()
    };
    d.items.insert(
        WARES,
        ItemTemplate {
            buy_count: 3,
            ..item(WARES, 5)
        },
    );
    let mut trinket = ItemTemplate {
        class: 4,
        inventory_type: 12,
        bonding: 2,
        ..item(TRINKET, 1)
    };
    trinket.stats[0] = (7, 7);
    trinket.spells[0] = ItemSpell {
        id: TRINKET_USE,
        trigger: 0,
        cooldown: 120_000,
        ..Default::default()
    };
    d.items.insert(
        HIGH_TRINKET,
        ItemTemplate {
            entry: HIGH_TRINKET,
            required_level: 60,
            ..trinket.clone()
        },
    );
    d.items.insert(
        SKILLED_TRINKET,
        ItemTemplate {
            entry: SKILLED_TRINKET,
            required_skill: 202,
            required_skill_rank: 100,
            ..trinket.clone()
        },
    );
    d.items.insert(TRINKET, trinket);
    d.items.insert(FILLER, item(FILLER, 1));
    // Cloth's proficiency spell, so that proficiencies are judged (a world
    // without it lets anyone use anything).
    d.spells.spells.insert(
        9078,
        crate::game::data::spells::SpellTemplate {
            id: 9078,
            name: "Cloth".into(),
            equipped_item_class: -1,
            ..Default::default()
        },
    );
    d.items.insert(
        BAG,
        ItemTemplate {
            class: 1,
            inventory_type: 18,
            container_slots: 4,
            ..item(BAG, 1)
        },
    );
    d.vendors.insert(
        CLERK,
        [WARES, TRINKET, HIGH_TRINKET, SKILLED_TRINKET]
            .into_iter()
            .enumerate()
            .map(|(slot, item)| VendorItem {
                slot: slot as u32,
                item,
                max_count: 6,
                restock_secs: 600,
                ..Default::default()
            })
            .collect(),
    );
    d
}

/// Alice with a hundred copper, beside the clerk.
async fn at_the_clerk() -> (
    std::sync::Arc<crate::game::Realm>,
    Client,
    Guid,
    MapHandle,
    Guid,
) {
    let (realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let c = on(&map, move |m| {
        m.players.get_mut(&ga).unwrap().set_money(100);
        spawn(m, ga, CLERK, 20, 2.0)
    })
    .await;
    a.sees(c).await;
    (realm, a, ga, map, c)
}

/// What came of `CMSG_BUY_ITEM_IN_SLOT`: the purchase's count, or the
/// inventory failure; `None` for neither.
async fn buy_in(
    a: &mut Client,
    vendor: Guid,
    item: u32,
    amount: u8,
    bag: Guid,
    bag_slot: u8,
) -> Option<Result<u32, String>> {
    a.clear();
    a.send(CMSG_BUY_ITEM_IN_SLOT {
        vendor,
        item,
        bag,
        bag_slot,
        amount,
    })
    .await;
    handled(a).await;
    a.gets(Duration::from_millis(1), |m| match m {
        ServerOpcodeMessage::SMSG_BUY_ITEM(b) => Some(Ok(b.amount_bought)),
        ServerOpcodeMessage::SMSG_INVENTORY_CHANGE_FAILURE(f) => {
            Some(Err(format!("{:?}", f.result)
                .split([' ', '{'])
                .next()
                .unwrap_or("")
                .to_owned()))
        }
        _ => None,
    })
    .await
}

fn refusal(why: &str) -> Option<Result<u32, String>> {
    Some(Err(why.to_owned()))
}

/// The clerk's stock of `item`, the money and the items Alice has.
fn ledger(m: &mut Map, ga: Guid, c: Guid, item: u32) -> (u32, u32, usize) {
    let v = *m
        .realm
        .data
        .vendor_items(CLERK)
        .iter()
        .find(|v| v.item == item)
        .unwrap();
    let t = m.realm.data.item(item).unwrap().buy_count;
    let left = vendor::stock_left(m, c, &v, t);
    let p = &m.players[&ga];
    (left, p.money(), p.items.len())
}

fn at(m: &Map, ga: Guid, entry: u32) -> Vec<(u8, u8, u32)> {
    let mut v: Vec<_> = m.players[&ga]
        .items
        .iter()
        .filter(|i| i.entry == entry)
        .map(|i| (i.bag, i.slot, i.count()))
        .collect();
    v.sort();
    v
}

/// Thistle Tea's `buy_in_slot` "uses the selected backpack slot",
/// "resolves equipped bag GUIDs and spills bundles into the same bag",
/// "rejects occupied slots without spending stock, money or item rows"
/// and "rejects bank slots and foreign or unequipped bag GUIDs": a
/// purchase dropped on a backpack slot goes there; dropped on a slot of a
/// bag worn, it fills that slot and spills into the same bag; a slot taken
/// by another item, or the bank's, refuses it with nothing spent; and a
/// bag not worn on the bag bar (or no bag) is ignored.
#[tokio::test]
async fn a_purchase_dropped_on_a_slot_goes_there_or_is_refused_with_nothing_spent() {
    let (_realm, mut a, ga, map, c) = at_the_clerk().await;
    assert_eq!(buy_in(&mut a, c, WARES, 1, ga, 30).await, Some(Ok(3)));
    let (left, money, _) = on(&map, move |m| ledger(m, ga, c, WARES)).await;
    assert_eq!((left, money), (3, 98));
    assert_eq!(on(&map, move |m| at(m, ga, WARES)).await, [(255, 30, 3)]);
    // A bag on the bag bar, and one in the backpack.
    let (worn, loose) = on(&map, move |m| {
        m.players
            .get_mut(&ga)
            .unwrap()
            .items
            .retain(|i| i.entry != WARES);
        m.vendor_stock.clear();
        inventory::add_item(m, ga, BAG, 2, false).unwrap();
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&ga).unwrap();
        let i = p.items.iter().position(|i| i.entry == BAG).unwrap();
        p.items[i].slot = inventory::BAG_START;
        let worn = p.items[i].guid;
        let loose = p
            .items
            .iter()
            .find(|i| i.entry == BAG && i.guid != worn)
            .unwrap()
            .guid;
        p.refresh_inventory(&data);
        (worn, loose)
    })
    .await;
    a.clear();
    a.send(CMSG_BUY_ITEM_IN_SLOT {
        vendor: c,
        item: WARES,
        bag: worn,
        bag_slot: 2,
        amount: 2,
    })
    .await;
    let push = a
        .wait_for("SMSG_ITEM_PUSH_RESULT", |m| match m {
            ServerOpcodeMessage::SMSG_ITEM_PUSH_RESULT(r) => Some((r.bag_slot, r.item_count)),
            _ => None,
        })
        .await;
    assert_eq!(push, (inventory::BAG_START, 6), "told into the bag");
    let into = on(&map, move |m| at(m, ga, WARES)).await;
    assert_eq!(
        into,
        [(inventory::BAG_START, 0, 1), (inventory::BAG_START, 2, 5)],
        "the slot, then the same bag"
    );
    // Taken by another item: refused, nothing spent.
    let filler = on(&map, move |m| {
        m.vendor_stock.clear();
        inventory::add_item(m, ga, FILLER, 1, false).unwrap();
        let p = &m.players[&ga];
        let i = p.items.iter().position(|i| i.entry == FILLER).unwrap();
        p.items[i].slot
    })
    .await;
    let before = on(&map, move |m| ledger(m, ga, c, WARES)).await;
    assert_eq!(
        buy_in(&mut a, c, WARES, 1, ga, filler).await,
        refusal("ItemCantStack")
    );
    assert_eq!(on(&map, move |m| ledger(m, ga, c, WARES)).await, before);
    // The bank's slots, and past the last: not where it goes.
    for slot in [39, 63, 100] {
        assert_eq!(
            buy_in(&mut a, c, WARES, 1, ga, slot).await,
            refusal("ItemDoesntGoToSlot"),
            "slot {slot}"
        );
    }
    // No bag of hers on the bag bar: nothing at all.
    for bag in [Guid::zero(), loose, crate::game::guid::player(4242)] {
        assert_eq!(buy_in(&mut a, c, WARES, 1, bag, 0).await, None, "{bag}");
    }
    assert_eq!(on(&map, move |m| ledger(m, ga, c, WARES)).await, before);
}

/// Thistle Tea's `buy_in_slot` "equips into an empty slot despite a full
/// backpack and starts stats, binding and cooldowns" and "equipment count,
/// level, proficiency and combat failures preserve the purchase": a
/// trinket dropped on the trinket slot is worn, though the backpack is
/// full: bound, its stamina on, its use cooling down; a second, the slot
/// taken, is refused. Two at once, a level short, a skill not had, or in a
/// fight: refused, with nothing spent.
#[tokio::test]
async fn a_purchase_dropped_on_the_gear_is_worn_or_refused_with_nothing_spent() {
    let (_realm, mut a, ga, map, c) = at_the_clerk().await;
    on(&map, move |m| {
        let free = (PACK_START..PACK_END)
            .filter(|s| m.players[&ga].item_at(BAG_PLAYER, *s).is_none())
            .count() as u32;
        inventory::add_item(m, ga, FILLER, free, false).unwrap();
    })
    .await;
    let refused = async |a: &mut Client, item: u32, amount: u8, why: &str| {
        let before = on(&map, move |m| ledger(m, ga, c, item)).await;
        assert_eq!(
            buy_in(a, c, item, amount, ga, 12).await,
            refusal(why),
            "{why}"
        );
        assert_eq!(on(&map, move |m| ledger(m, ga, c, item)).await, before);
    };
    refused(&mut a, TRINKET, 2, "ItemCantBeEquipped").await;
    refused(&mut a, HIGH_TRINKET, 1, "CantEquipLevelI").await;
    refused(&mut a, SKILLED_TRINKET, 1, "NoRequiredProficiency").await;
    let brute = on(&map, move |m| {
        let b = spawn(m, ga, BRUTE, 20, 30.0);
        crate::game::map::combat::engage(m, ga, b);
        b
    })
    .await;
    refused(&mut a, TRINKET, 1, "NotInCombat").await;
    on(&map, move |m| {
        crate::game::map::combat::leave_combat(m, ga);
        m.remove_creature(brute);
    })
    .await;
    a.clear();
    a.send(CMSG_BUY_ITEM_IN_SLOT {
        vendor: c,
        item: TRINKET,
        bag: ga,
        bag_slot: 12,
        amount: 1,
    })
    .await;
    let cooling = a
        .wait_for("SMSG_ITEM_COOLDOWN", |m| match m {
            ServerOpcodeMessage::SMSG_ITEM_COOLDOWN(c) => Some((c.guid, c.id)),
            _ => None,
        })
        .await;
    let worn = on(&map, move |m| {
        let data = m.realm.data.clone();
        let p = &m.players[&ga];
        let i = p.item_at(BAG_PLAYER, 12).expect("worn");
        (
            p.items[i].guid,
            p.items[i].fields.get_u32(ITEM_FIELD_FLAGS) & 1,
            inventory::gear_bonuses(p, &data).0[2],
            p.money(),
            p.caster.cooldowns.contains_key(&TRINKET_USE),
        )
    })
    .await;
    assert_eq!(cooling, (worn.0, TRINKET_USE));
    assert_eq!(&(worn.1, worn.2, worn.3, worn.4), &(1, 7, 98, true));
    assert_eq!(
        buy_in(&mut a, c, TRINKET, 1, ga, 12).await,
        refusal("NoEquipmentSlotAvailable"),
        "the slot taken"
    );
    let (left, money, _) = on(&map, move |m| ledger(m, ga, c, TRINKET)).await;
    assert_eq!((left, money), (5, 98));
}
