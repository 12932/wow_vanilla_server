//! Phase 3's bags, Thistle Tea's `InventoryTest` ported: moving and
//! swapping, bags in bags, what may be worn where (the off hand, a
//! two-hander's off hand put away, one quiver), stacks merged and split,
//! destroying, storing new items, the bank and its bags, and the keyring.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::inventory::{
    self, BAG_START, BANK_BAG_START, BANK_START, PACK_END, PACK_START, SLOT_MAIN_HAND,
    SLOT_OFF_HAND,
};
use crate::game::map::items::KEYRING_START;
use crate::game::map::{Map, MapKey};
use crate::game::store::BAG_PLAYER;

const CHEST: u32 = 100;
const SWORD: u32 = 200;
const GREATSWORD: u32 = 300;
const SHIELD: u32 = 400;
const RING: u32 = 500;
const HIGH_LEVEL: u32 = 600;
const PRIEST_ONLY: u32 = 700;
const PELT: u32 = 750;
const BAG: u32 = 800;
const HERB_BAG: u32 = 801;
const SOUL_BAG: u32 = 802;
const SMALL_BAG: u32 = 803;
const QUIVER: u32 = 990;
const POUCH: u32 = 991;
const BOE_BAG: u32 = 992;
const BOE_HIGH: u32 = 993;
const BOE_CHEST: u32 = 994;
const FILLER: u32 = 1000;
const SECOND_SWORD: u32 = 1400;
const HERB: u32 = 2000;
const TIMED: u32 = 2001;
const SHARD: u32 = 6265;
const UNIQUE: u32 = 6948;
const KEY: u32 = 4000;
const OTHER_KEY: u32 = 4001;
/// Cloth: in the world, so a player's proficiency comes from their spells.
const CLOTH: u32 = 9078;
/// Every weapon and armor, and Dual Wield.
const WEAPONS: u32 = 9_400_001;
const ARMOR: u32 = 9_400_002;
const DUAL_WIELD: u32 = 674;
const REAGENT_SPELL: u32 = 9_400_003;
const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const GIVER: u32 = 3150;
const PELT_QUEST: u32 = 9401;
const TWO_QUEST: u32 = 9402;
const RING_QUEST: u32 = 9403;

fn item(entry: u32) -> ItemTemplate {
    ItemTemplate {
        entry,
        name: format!("Item {entry}"),
        display_id: 1000 + entry,
        stackable: 1,
        allowable_class: -1,
        allowable_race: -1,
        ..Default::default()
    }
}

fn worn(entry: u32, class: u32, subclass: u32, inventory_type: u32) -> ItemTemplate {
    ItemTemplate {
        class,
        subclass,
        inventory_type,
        ..item(entry)
    }
}

fn bag(entry: u32, class: u32, subclass: u32, family: u32, slots: u32) -> ItemTemplate {
    ItemTemplate {
        class,
        subclass,
        inventory_type: 18,
        container_slots: slots,
        bag_family: family,
        ..item(entry)
    }
}

fn proficiency(id: u32, class: i32) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Proficiency {id}"),
        equipped_item_class: class,
        equipped_item_subclass_mask: -1,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: crate::game::map::spells::EFFECT_PROFICIENCY,
        target_a: 1,
        ..Default::default()
    };
    s
}

fn data() -> GameData {
    let mut d = GameData::empty();
    for t in [
        worn(CHEST, 4, 1, 5),
        worn(SWORD, 2, 7, 13),
        worn(GREATSWORD, 2, 8, 17),
        worn(SHIELD, 4, 6, 14),
        worn(RING, 4, 0, 11),
        worn(SECOND_SWORD, 2, 7, 13),
        bag(BAG, 1, 0, 0, 6),
        bag(HERB_BAG, 1, 6, 0x20, 6),
        bag(SOUL_BAG, 1, 1, 4, 6),
        bag(SMALL_BAG, 1, 0, 0, 1),
        bag(QUIVER, 11, 2, 1, 6),
        bag(POUCH, 11, 3, 2, 6),
        item(FILLER),
    ] {
        d.items.insert(t.entry, t);
    }
    let mut high = worn(HIGH_LEVEL, 4, 1, 5);
    high.required_level = 60;
    d.items.insert(HIGH_LEVEL, high);
    let mut priest = worn(PRIEST_ONLY, 4, 1, 5);
    priest.allowable_class = 16;
    d.items.insert(PRIEST_ONLY, priest);
    let mut boe_bag = bag(BOE_BAG, 1, 0, 0, 6);
    boe_bag.bonding = crate::game::map::items::BIND_ON_EQUIP;
    d.items.insert(BOE_BAG, boe_bag);
    let mut boe_high = worn(BOE_HIGH, 4, 1, 5);
    boe_high.bonding = crate::game::map::items::BIND_ON_EQUIP;
    boe_high.required_level = 60;
    d.items.insert(BOE_HIGH, boe_high);
    let mut boe_chest = worn(BOE_CHEST, 4, 1, 5);
    boe_chest.bonding = crate::game::map::items::BIND_ON_EQUIP;
    d.items.insert(BOE_CHEST, boe_chest);
    for (entry, stack, family) in [(PELT, 10, 0), (HERB, 20, 0x20), (SHARD, 1, 4)] {
        let mut t = item(entry);
        t.stackable = stack;
        t.bag_family = family;
        d.items.insert(entry, t);
    }
    let mut timed = item(TIMED);
    timed.stackable = 20;
    timed.duration = 300;
    d.items.insert(TIMED, timed);
    let mut unique = item(UNIQUE);
    unique.max_count = 1;
    d.items.insert(UNIQUE, unique);
    for entry in [KEY, OTHER_KEY] {
        let mut key = item(entry);
        key.class = 13;
        key.bag_family = crate::game::map::items::BAG_FAMILY_KEYS;
        d.items.insert(entry, key);
    }
    d.spells.spells.insert(CLOTH, proficiency(CLOTH, 4));
    d.spells.spells.insert(WEAPONS, proficiency(WEAPONS, 2));
    d.spells.spells.insert(ARMOR, proficiency(ARMOR, 4));
    let mut dual = SpellTemplate {
        id: DUAL_WIELD,
        name: "Dual Wield".into(),
        equipped_item_class: -1,
        ..Default::default()
    };
    dual.effects[0] = SpellEffect {
        effect: 40,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(DUAL_WIELD, dual);
    // A spell of six pelts.
    let mut reagents = SpellTemplate {
        id: REAGENT_SPELL,
        name: "Pelt Craft".into(),
        equipped_item_class: -1,
        ..Default::default()
    };
    reagents.reagents[0] = (PELT, 6);
    reagents.effects[0] = SpellEffect {
        effect: 3,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(REAGENT_SPELL, reagents);
    // A quest giver: a pelt for a chest, two rewards for nothing, a ring
    // for a chest.
    d.creature_templates.insert(
        GIVER,
        CreatureTemplate {
            entry: GIVER,
            name: "Giver".into(),
            display_ids: [903, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            display_scales: [1.0, 0.0, 0.0, 0.0],
            level_min: 2,
            level_max: 2,
            faction: 35,
            unit_class: 1,
            health_max: 50,
            npc_flags: 0x2,
            ..Default::default()
        },
    );
    d.creature_spawns.insert(
        0,
        vec![CreatureSpawn {
            guid: 1,
            entries: vec![GIVER],
            map: 0,
            x: START.0 + 2.0,
            y: START.1,
            z: START.2,
            ..Default::default()
        }],
    );
    let quest = |id: u32| crate::game::data::npc::Quest {
        id,
        title: format!("Quest {id}"),
        min_level: 1,
        level: 1,
        ..Default::default()
    };
    let mut pelt_for_chest = quest(PELT_QUEST);
    pelt_for_chest.required_items[0] = (PELT, 1);
    pelt_for_chest.rewards[0] = (CHEST, 1);
    let mut two = quest(TWO_QUEST);
    two.rewards[0] = (CHEST, 1);
    two.rewards[1] = (RING, 1);
    let mut ring_for_chest = quest(RING_QUEST);
    ring_for_chest.required_items[0] = (RING, 1);
    ring_for_chest.rewards[0] = (CHEST, 1);
    let ids = vec![PELT_QUEST, TWO_QUEST, RING_QUEST];
    for q in [pelt_for_chest, two, ring_for_chest] {
        d.npc.quests.insert(q.id, q);
    }
    d.npc.creature_starts.insert(GIVER, ids.clone());
    d.npc.creature_ends.insert(GIVER, ids);
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

/// A warrior proficient in every weapon and armor, without Dual Wield.
async fn setup() -> (Arc<Realm>, Client, Guid) {
    setup_with(data()).await
}

async fn setup_with(d: GameData) -> (Arc<Realm>, Client, Guid) {
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        p.spells.extend([CLOTH, WEAPONS, ARMOR]);
        // Nothing carried but what a test gives.
        p.items.clear();
        let d = m.realm.data.clone();
        m.players.get_mut(&g).unwrap().refresh_inventory(&d);
    })
    .await;
    a.clear();
    (realm, a, g)
}

async fn learn(realm: &Arc<Realm>, g: Guid, spell: u32) {
    on_map(realm, move |m| {
        m.players.get_mut(&g).unwrap().spells.push(spell)
    })
    .await;
}

/// Give `count` of `entry` at `(bag, slot)`; its guid.
async fn give_at(realm: &Arc<Realm>, g: Guid, entry: u32, count: u32, at: (u8, u8)) -> Guid {
    on_map(realm, move |m| {
        let d = m.realm.data.clone();
        let counter = m
            .realm
            .next_item
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let guid = crate::game::guid::item(counter);
        let p = m.players.get_mut(&g).unwrap();
        assert!(p.item_at(at.0, at.1).is_none(), "{at:?} is free");
        p.items.push(crate::game::map::player::Item::new(
            &d, guid, g, entry, at.0, at.1, count,
        ));
        p.refresh_inventory(&d);
        guid
    })
    .await
}

/// Fill every free backpack slot.
async fn fill_pack(realm: &Arc<Realm>, g: Guid) {
    for slot in PACK_START..PACK_END {
        let free = on_map(realm, move |m| {
            m.players[&g].item_at(BAG_PLAYER, slot).is_none()
        })
        .await;
        if free {
            give_at(realm, g, FILLER, 1, (BAG_PLAYER, slot)).await;
        }
    }
}

/// Where `item` is, and how many.
async fn at(realm: &Arc<Realm>, g: Guid, item: Guid) -> Option<(u8, u8, u32)> {
    on_map(realm, move |m| {
        let p = &m.players[&g];
        p.item_by_guid(item)
            .map(|i| (p.items[i].bag, p.items[i].slot, p.items[i].count()))
    })
    .await
}

async fn field(realm: &Arc<Realm>, g: Guid, item: Guid, f: u16) -> u32 {
    on_map(realm, move |m| {
        let p = &m.players[&g];
        p.items[p.item_by_guid(item).unwrap()].fields.get_u32(f)
    })
    .await
}

async fn guid_field(realm: &Arc<Realm>, g: Guid, item: Guid, f: u16) -> Guid {
    on_map(realm, move |m| {
        let p = &m.players[&g];
        p.items[p.item_by_guid(item).unwrap()].fields.get_guid(f)
    })
    .await
}

/// Wait until `a`'s earlier packets have been handled.
async fn handled(a: &mut Client) {
    a.send(CMSG_NAME_QUERY { guid: a.guid }).await;
    a.wait_for("SMSG_NAME_QUERY_RESPONSE", |m| match m {
        ServerOpcodeMessage::SMSG_NAME_QUERY_RESPONSE(_) => Some(()),
        _ => None,
    })
    .await;
}

/// The refusal `a` was sent since it last looked, by its name.
async fn refusal(a: &mut Client) -> Option<String> {
    handled(a).await;
    let r = a
        .gets(Duration::from_millis(1), |m| match m {
            ServerOpcodeMessage::SMSG_INVENTORY_CHANGE_FAILURE(f) => {
                Some(format!("{:?}", f.result))
            }
            _ => None,
        })
        .await;
    a.clear();
    r.map(|r| r.split([' ', '{']).next().unwrap().to_owned())
}

async fn swap(a: &mut Client, from: (u8, u8), to: (u8, u8)) -> Option<String> {
    a.clear();
    a.send(CMSG_SWAP_ITEM {
        destination_bag: to.0,
        destionation_slot: to.1,
        source_bag: from.0,
        source_slot: from.1,
    })
    .await;
    refusal(a).await
}

async fn auto_equip(a: &mut Client, from: (u8, u8)) -> Option<String> {
    a.clear();
    a.send(CMSG_AUTOEQUIP_ITEM {
        source_bag: from.0,
        source_slot: from.1,
    })
    .await;
    refusal(a).await
}

async fn auto_store(a: &mut Client, from: (u8, u8), bag: u8) -> Option<String> {
    a.clear();
    a.send(CMSG_AUTOSTORE_BAG_ITEM {
        source_bag: from.0,
        source_slot: from.1,
        destination_bag: bag,
    })
    .await;
    refusal(a).await
}

async fn cast(a: &Client, spell: u32) {
    a.send(CMSG_CAST_SPELL {
        spell,
        targets: crate::game::map::spells::wire_targets(&crate::game::map::spells::Targets {
            unit: Some(a.guid),
            ..Default::default()
        }),
    })
    .await;
}

async fn split(a: &mut Client, from: (u8, u8), to: (u8, u8), amount: u8) -> Option<String> {
    a.clear();
    a.send(CMSG_SPLIT_ITEM {
        source_bag: from.0,
        source_slot: from.1,
        destination_bag: to.0,
        destination_slot: to.1,
        amount,
    })
    .await;
    refusal(a).await
}

async fn destroy(a: &mut Client, at: (u8, u8), amount: u8) -> Option<String> {
    a.clear();
    a.send(CMSG_DESTROYITEM {
        bag: at.0,
        slot: at.1,
        amount,
        unknown1: 0,
        unknown2: 0,
        unknown3: 0,
    })
    .await;
    refusal(a).await
}

const PACK: (u8, u8) = (BAG_PLAYER, PACK_START);
const PACK2: (u8, u8) = (BAG_PLAYER, PACK_START + 1);
const PACK3: (u8, u8) = (BAG_PLAYER, PACK_START + 2);
const BAG1: (u8, u8) = (BAG_PLAYER, BAG_START);
const BAG2: (u8, u8) = (BAG_PLAYER, BAG_START + 1);
const MAIN: (u8, u8) = (BAG_PLAYER, SLOT_MAIN_HAND);
const OFF: (u8, u8) = (BAG_PLAYER, SLOT_OFF_HAND);
const CHEST_SLOT: (u8, u8) = (BAG_PLAYER, 4);
const BANK1: (u8, u8) = (BAG_PLAYER, BANK_START);
const BANK2: (u8, u8) = (BAG_PLAYER, BANK_START + 1);
const BANK_BAG1: (u8, u8) = (BAG_PLAYER, BANK_BAG_START);

/// Thistle Tea's `swap/7`: two backpack slots swapped, an item into a bag
/// and out of it again (the bag's slot and the item's container kept), a
/// full bag kept on the bar, no bag into itself, no item but a bag on the
/// bag bar.
#[tokio::test]
async fn items_swap_and_go_in_and_out_of_bags() {
    let (realm, mut a, g) = setup().await;
    let chest = give_at(&realm, g, CHEST, 1, PACK).await;
    let sword = give_at(&realm, g, SWORD, 1, PACK2).await;
    a.send(CMSG_SWAP_INV_ITEM {
        source_slot: ItemSlot::try_from(PACK.1).unwrap(),
        destination_slot: ItemSlot::try_from(PACK2.1).unwrap(),
    })
    .await;
    assert_eq!(refusal(&mut a).await, None);
    assert_eq!(at(&realm, g, chest).await, Some((BAG_PLAYER, PACK2.1, 1)));
    assert_eq!(at(&realm, g, sword).await, Some((BAG_PLAYER, PACK.1, 1)));

    let bag = give_at(&realm, g, BAG, 1, BAG1).await;
    assert_eq!(swap(&mut a, PACK2, (BAG_START, 0)).await, None);
    assert_eq!(at(&realm, g, chest).await, Some((BAG_START, 0, 1)));
    assert_eq!(
        guid_field(&realm, g, bag, CONTAINER_FIELD_SLOT_1).await,
        chest
    );
    assert_eq!(
        guid_field(&realm, g, chest, ITEM_FIELD_CONTAINED).await,
        bag
    );

    // A bag with something in it stays on the bar, and no bag goes into
    // itself.
    assert_eq!(
        swap(&mut a, BAG1, PACK3).await.as_deref(),
        Some("CanOnlyDoWithEmptyBags")
    );
    assert_eq!(at(&realm, g, bag).await, Some((BAG_PLAYER, BAG_START, 1)));

    assert_eq!(swap(&mut a, (BAG_START, 0), PACK2).await, None);
    assert_eq!(at(&realm, g, chest).await, Some((BAG_PLAYER, PACK2.1, 1)));
    assert_eq!(
        guid_field(&realm, g, bag, CONTAINER_FIELD_SLOT_1).await,
        Guid::zero()
    );
    assert_eq!(guid_field(&realm, g, chest, ITEM_FIELD_CONTAINED).await, g);

    assert!(
        swap(&mut a, BAG1, (BAG_START, 0)).await.is_some(),
        "into itself"
    );
    assert_eq!(at(&realm, g, bag).await, Some((BAG_PLAYER, BAG_START, 1)));
    assert_eq!(
        swap(&mut a, PACK2, BAG2).await.as_deref(),
        Some("ItemDoesntGoToSlot"),
        "not a bag"
    );
    assert_eq!(at(&realm, g, chest).await, Some((BAG_PLAYER, PACK2.1, 1)));
}

/// Thistle Tea's off hand: a one-hander there only with Dual Wield, nothing
/// there beside a two-hander, and a two-hander worn puts the off hand's
/// item away: beside the main hand's swapped out, into the slot it leaves,
/// and not at all with nowhere carried for it (from the bank, or its bags).
#[tokio::test]
async fn the_off_hand_takes_dual_wield_and_a_two_hander_puts_it_away() {
    let (realm, mut a, g) = setup().await;
    let sword = give_at(&realm, g, SWORD, 1, PACK).await;
    assert_eq!(
        swap(&mut a, PACK, OFF).await.as_deref(),
        Some("CantDualWield")
    );
    learn(&realm, g, DUAL_WIELD).await;
    assert_eq!(swap(&mut a, PACK, OFF).await, None);
    assert_eq!(swap(&mut a, OFF, PACK).await, None);

    // Beside a two-hander, no off hand.
    let great = give_at(&realm, g, GREATSWORD, 1, MAIN).await;
    let shield = give_at(&realm, g, SHIELD, 1, PACK2).await;
    assert_eq!(
        swap(&mut a, PACK2, OFF).await.as_deref(),
        Some("CantEquipWithTwohanded")
    );
    assert_eq!(at(&realm, g, shield).await, Some((BAG_PLAYER, PACK2.1, 1)));

    // A sword and shield worn; the two-hander dragged onto the sword: the
    // sword where it was, the shield in the next free slot.
    assert_eq!(swap(&mut a, MAIN, PACK3).await, None);
    assert_eq!(swap(&mut a, PACK, MAIN).await, None);
    assert_eq!(swap(&mut a, PACK2, OFF).await, None);
    assert_eq!(
        at(&realm, g, sword).await,
        Some((BAG_PLAYER, SLOT_MAIN_HAND, 1))
    );
    assert_eq!(
        at(&realm, g, shield).await,
        Some((BAG_PLAYER, SLOT_OFF_HAND, 1))
    );
    assert_eq!(swap(&mut a, PACK3, MAIN).await, None);
    assert_eq!(
        at(&realm, g, great).await,
        Some((BAG_PLAYER, SLOT_MAIN_HAND, 1))
    );
    assert_eq!(at(&realm, g, sword).await, Some((BAG_PLAYER, PACK3.1, 1)));
    assert_eq!(at(&realm, g, shield).await, Some((BAG_PLAYER, PACK.1, 1)));
    let visible = on_map(&realm, move |m| {
        m.players[&g]
            .fields
            .get_u32(PLAYER_VISIBLE_ITEM_1_0 + u16::from(SLOT_OFF_HAND) * VISIBLE_ITEM_SIZE)
    })
    .await;
    assert_eq!(visible, 0, "the off hand shows nothing");
}

/// Thistle Tea's displaced off hand with the bags full: a two-hander from
/// the bank or a bank bag isn't worn, one from the backpack or a bag
/// leaves its slot for the off hand's item.
#[tokio::test]
async fn a_two_hander_from_full_bags_leaves_its_slot_for_the_off_hand() {
    let (realm, mut a, g) = setup().await;
    on_map(&realm, move |m| {
        m.players
            .get_mut(&g)
            .unwrap()
            .fields
            .set_byte(PLAYER_BYTES_2, 2, 1)
    })
    .await;
    let shield = give_at(&realm, g, SHIELD, 1, OFF).await;
    let great = give_at(&realm, g, GREATSWORD, 1, BANK1).await;
    fill_pack(&realm, g).await;
    assert_eq!(
        swap(&mut a, BANK1, MAIN).await.as_deref(),
        Some("InventoryFull"),
        "from the bank"
    );
    assert_eq!(
        at(&realm, g, shield).await,
        Some((BAG_PLAYER, SLOT_OFF_HAND, 1))
    );
    let bank_bag = give_at(&realm, g, SMALL_BAG, 1, BANK_BAG1).await;
    assert_eq!(swap(&mut a, BANK1, (BANK_BAG_START, 0)).await, None);
    assert_eq!(
        swap(&mut a, (BANK_BAG_START, 0), MAIN).await.as_deref(),
        Some("InventoryFull"),
        "from a bank bag"
    );
    assert_eq!(at(&realm, g, great).await, Some((BANK_BAG_START, 0, 1)));
    let _ = bank_bag;

    // From the backpack: the shield where the two-hander was.
    let last = (BAG_PLAYER, PACK_END - 1);
    let filler = on_map(&realm, move |m| {
        let p = &m.players[&g];
        p.items[p.item_at(last.0, last.1).unwrap()].guid
    })
    .await;
    on_map(&realm, move |m| {
        let d = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        let i = p.item_by_guid(filler).unwrap();
        p.items.remove(i);
        p.refresh_inventory(&d);
    })
    .await;
    assert_eq!(swap(&mut a, (BANK_BAG_START, 0), last).await, None);
    assert_eq!(swap(&mut a, last, MAIN).await, None);
    assert_eq!(
        at(&realm, g, great).await,
        Some((BAG_PLAYER, SLOT_MAIN_HAND, 1))
    );
    assert_eq!(at(&realm, g, shield).await, Some((last.0, last.1, 1)));

    // From a one-slot bag: the same.
    assert_eq!(swap(&mut a, MAIN, (BANK_BAG_START, 0)).await, None);
    assert_eq!(swap(&mut a, last, OFF).await, None);
    give_at(&realm, g, FILLER, 1, last).await;
    give_at(&realm, g, SMALL_BAG, 1, BAG1).await;
    assert_eq!(
        swap(&mut a, (BANK_BAG_START, 0), (BAG_START, 0)).await,
        None
    );
    assert_eq!(swap(&mut a, (BAG_START, 0), MAIN).await, None);
    assert_eq!(
        at(&realm, g, great).await,
        Some((BAG_PLAYER, SLOT_MAIN_HAND, 1))
    );
    assert_eq!(at(&realm, g, shield).await, Some((BAG_START, 0, 1)));
}

/// Thistle Tea's `auto_equip/6`: over what is worn, a bag into a free bag
/// slot, refused above the level and for another class (unbound), a bag
/// bound as it goes on, and a second one-hander into the off hand with
/// Dual Wield, else over the first.
#[tokio::test]
async fn auto_equip_swaps_binds_and_refuses() {
    let (realm, mut a, g) = setup().await;
    let old = give_at(&realm, g, CHEST, 1, CHEST_SLOT).await;
    let new = give_at(&realm, g, BOE_CHEST, 1, PACK).await;
    assert_eq!(auto_equip(&mut a, PACK).await, None);
    assert_eq!(at(&realm, g, new).await, Some((BAG_PLAYER, 4, 1)));
    assert_eq!(at(&realm, g, old).await, Some((BAG_PLAYER, PACK.1, 1)));
    let flags = field(&realm, g, new, ITEM_FIELD_FLAGS).await;
    assert_eq!(flags & crate::game::map::items::ITEM_FLAG_SOULBOUND, 1);
    // Bound it stays, off and on again.
    assert_eq!(swap(&mut a, CHEST_SLOT, PACK2).await, None);
    assert_eq!(field(&realm, g, new, ITEM_FIELD_FLAGS).await & 1, 1);
    assert_eq!(auto_equip(&mut a, PACK2).await, None);
    assert_eq!(at(&realm, g, new).await, Some((BAG_PLAYER, 4, 1)));

    let bag = give_at(&realm, g, BOE_BAG, 1, PACK2).await;
    assert_eq!(auto_equip(&mut a, PACK2).await, None);
    assert_eq!(at(&realm, g, bag).await, Some((BAG_PLAYER, BAG_START, 1)));
    assert_eq!(
        field(&realm, g, bag, ITEM_FIELD_FLAGS).await & 1,
        1,
        "bound"
    );

    let high = give_at(&realm, g, BOE_HIGH, 1, PACK2).await;
    assert_eq!(
        auto_equip(&mut a, PACK2).await.as_deref(),
        Some("CantEquipLevelI")
    );
    assert_eq!(
        field(&realm, g, high, ITEM_FIELD_FLAGS).await & 1,
        0,
        "unbound"
    );
    let priest = give_at(&realm, g, PRIEST_ONLY, 1, PACK3).await;
    assert_eq!(
        auto_equip(&mut a, PACK3).await.as_deref(),
        Some("YouCanNeverUseThatItem")
    );
    assert_eq!(at(&realm, g, priest).await, Some((BAG_PLAYER, PACK3.1, 1)));

    // A second sword: over the first without Dual Wield, beside it with.
    let first = give_at(&realm, g, SWORD, 1, MAIN).await;
    let second = give_at(&realm, g, SECOND_SWORD, 1, (BAG_START, 0)).await;
    assert_eq!(auto_equip(&mut a, (BAG_START, 0)).await, None);
    assert_eq!(
        at(&realm, g, second).await,
        Some((BAG_PLAYER, SLOT_MAIN_HAND, 1))
    );
    assert_eq!(at(&realm, g, first).await, Some((BAG_START, 0, 1)));
    learn(&realm, g, DUAL_WIELD).await;
    assert_eq!(auto_equip(&mut a, (BAG_START, 0)).await, None);
    assert_eq!(
        at(&realm, g, first).await,
        Some((BAG_PLAYER, SLOT_OFF_HAND, 1))
    );
    assert_eq!(
        at(&realm, g, second).await,
        Some((BAG_PLAYER, SLOT_MAIN_HAND, 1))
    );
}

/// Thistle Tea's ammunition bags: one quiver or ammo pouch on the bag bar
/// (by the worn one's kind), though one may take another's place, move
/// along the bar, or go in the bank; and the bank's bag slots only once
/// bought.
#[tokio::test]
async fn one_quiver_on_the_bar_and_bank_bag_slots_bought() {
    for (worn, other, error) in [
        (QUIVER, POUCH, "CanEquipOnly1Quiver"),
        (POUCH, QUIVER, "CanEquipOnly1Ammopouch"),
    ] {
        let (realm, mut a, g) = setup().await;
        let existing = give_at(&realm, g, worn, 1, BAG1).await;
        let incoming = give_at(&realm, g, other, 1, PACK).await;
        assert_eq!(auto_equip(&mut a, PACK).await.as_deref(), Some(error));
        assert_eq!(swap(&mut a, PACK, BAG2).await.as_deref(), Some(error));
        assert_eq!(
            swap(&mut a, PACK, BANK_BAG1).await.as_deref(),
            Some("MustPurchaseThatBagSlot")
        );
        on_map(&realm, move |m| {
            m.players
                .get_mut(&g)
                .unwrap()
                .fields
                .set_byte(PLAYER_BYTES_2, 2, 1)
        })
        .await;
        assert_eq!(swap(&mut a, PACK, BAG1).await, None, "in its place");
        assert_eq!(
            at(&realm, g, incoming).await,
            Some((BAG_PLAYER, BAG_START, 1))
        );
        assert_eq!(at(&realm, g, existing).await, Some((BAG_PLAYER, PACK.1, 1)));
        assert_eq!(swap(&mut a, BAG1, BAG2).await, None, "along the bar");
        assert_eq!(swap(&mut a, PACK, BANK_BAG1).await, None, "in the bank");
        assert_eq!(
            at(&realm, g, existing).await,
            Some((BAG_PLAYER, BANK_BAG_START, 1))
        );
    }
}

/// Thistle Tea's `auto_store_in_bag/5`: off the main hand into the
/// backpack; into the bag asked for alone, onto its stacks first and the
/// rest beside them; refused for a full bag, no bag, a bag not made for
/// it, a full bag into the backpack, a bag into itself; and a key into the
/// backpack goes on the keyring.
#[tokio::test]
async fn auto_store_goes_to_the_bag_asked_for() {
    let (realm, mut a, g) = setup().await;
    let sword = give_at(&realm, g, SWORD, 1, MAIN).await;
    assert_eq!(auto_store(&mut a, MAIN, BAG_PLAYER).await, None);
    assert_eq!(at(&realm, g, sword).await, Some((BAG_PLAYER, PACK.1, 1)));
    give_at(&realm, g, BAG, 1, BAG1).await;
    assert_eq!(auto_store(&mut a, PACK, BAG_START).await, None);
    assert_eq!(at(&realm, g, sword).await, Some((BAG_START, 0, 1)));

    let source = give_at(&realm, g, PELT, 7, PACK).await;
    let destination = give_at(&realm, g, PELT, 8, (BAG_START, 1)).await;
    assert_eq!(auto_store(&mut a, PACK, BAG_START).await, None);
    assert_eq!(at(&realm, g, destination).await, Some((BAG_START, 1, 10)));
    assert_eq!(at(&realm, g, source).await, Some((BAG_START, 2, 5)));

    for s in 3..6 {
        give_at(&realm, g, FILLER, 1, (BAG_START, s)).await;
    }
    let ring = give_at(&realm, g, RING, 1, PACK).await;
    assert_eq!(
        auto_store(&mut a, PACK, BAG_START).await.as_deref(),
        Some("BagFull")
    );
    assert_eq!(
        auto_store(&mut a, PACK, BAG_START + 1).await.as_deref(),
        Some("ItemDoesntGoToSlot"),
        "no bag there"
    );
    assert_eq!(
        auto_store(&mut a, BAG1, BAG_PLAYER).await.as_deref(),
        Some("CanOnlyDoWithEmptyBags")
    );
    let herbs = give_at(&realm, g, HERB_BAG, 1, BAG2).await;
    assert!(
        auto_store(&mut a, PACK, BAG_START + 1).await.is_some(),
        "herbs only"
    );
    assert!(
        auto_store(&mut a, BAG2, BAG_START + 1).await.is_some(),
        "into itself"
    );
    assert_eq!(at(&realm, g, ring).await, Some((BAG_PLAYER, PACK.1, 1)));
    assert_eq!(
        at(&realm, g, herbs).await,
        Some((BAG_PLAYER, BAG_START + 1, 1))
    );

    let key = give_at(&realm, g, KEY, 1, PACK2).await;
    assert_eq!(auto_store(&mut a, PACK2, BAG_PLAYER).await, None);
    assert_eq!(
        at(&realm, g, key).await,
        Some((BAG_PLAYER, KEYRING_START, 1))
    );
}

/// Thistle Tea's `swap/7 stacking`: a stack dropped on its kind merges into
/// it, fills a nearly full one and keeps the rest, and swaps with a full
/// one.
#[tokio::test]
async fn stacks_dropped_on_their_kind_merge() {
    let (realm, mut a, g) = setup().await;
    let src = give_at(&realm, g, PELT, 3, PACK).await;
    let dst = give_at(&realm, g, PELT, 4, PACK2).await;
    assert_eq!(swap(&mut a, PACK, PACK2).await, None);
    assert_eq!(at(&realm, g, src).await, None, "merged away");
    assert_eq!(at(&realm, g, dst).await, Some((BAG_PLAYER, PACK2.1, 7)));

    let src = give_at(&realm, g, PELT, 5, PACK).await;
    assert_eq!(swap(&mut a, PACK, PACK2).await, None);
    assert_eq!(at(&realm, g, src).await, Some((BAG_PLAYER, PACK.1, 2)));
    assert_eq!(at(&realm, g, dst).await, Some((BAG_PLAYER, PACK2.1, 10)));

    assert_eq!(swap(&mut a, PACK, PACK2).await, None);
    assert_eq!(
        at(&realm, g, src).await,
        Some((BAG_PLAYER, PACK2.1, 2)),
        "swapped"
    );
    assert_eq!(at(&realm, g, dst).await, Some((BAG_PLAYER, PACK.1, 10)));
}

/// Thistle Tea's `split/6`: into an empty slot, the new stack the old
/// one's (its binding, maker and time left); not all of a stack, not onto
/// another item, not into a bag not made for it; and in the bank, split
/// and destroyed there.
#[tokio::test]
async fn a_stack_splits_into_the_old_ones_kind() {
    let (realm, mut a, g) = setup().await;
    let src = give_at(&realm, g, TIMED, 10, PACK).await;
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        let i = p.item_by_guid(src).unwrap();
        let f = &mut p.items[i].fields;
        f.set_u32(ITEM_FIELD_FLAGS, 1);
        f.set_u32(ITEM_FIELD_DURATION, 45);
        f.set_guid(ITEM_FIELD_CREATOR, Guid::new(99));
    })
    .await;
    assert_eq!(split(&mut a, PACK, PACK2, 3).await, None);
    let new = on_map(&realm, move |m| {
        let p = &m.players[&g];
        p.items[p.item_at(PACK2.0, PACK2.1).unwrap()].guid
    })
    .await;
    assert_ne!(new, src);
    assert_eq!(at(&realm, g, new).await, Some((BAG_PLAYER, PACK2.1, 3)));
    assert_eq!(at(&realm, g, src).await, Some((BAG_PLAYER, PACK.1, 7)));
    assert_eq!(field(&realm, g, new, ITEM_FIELD_FLAGS).await, 1);
    assert_eq!(field(&realm, g, new, ITEM_FIELD_DURATION).await, 45);
    assert_eq!(
        guid_field(&realm, g, new, ITEM_FIELD_CREATOR).await,
        Guid::new(99)
    );

    assert_eq!(
        split(&mut a, PACK, PACK3, 7).await.as_deref(),
        Some("TriedToSplitMoreThanCount")
    );
    give_at(&realm, g, CHEST, 1, PACK3).await;
    assert_eq!(
        split(&mut a, PACK, PACK3, 2).await.as_deref(),
        Some("ItemCantStack")
    );
    give_at(&realm, g, HERB_BAG, 1, BAG1).await;
    assert_eq!(
        split(&mut a, PACK, (BAG_START, 0), 2).await.as_deref(),
        Some("ItemDoesntGoIntoBag")
    );
    assert_eq!(at(&realm, g, src).await, Some((BAG_PLAYER, PACK.1, 7)));

    // In the bank, and into a bank bag of the wrong kind.
    let banked = give_at(&realm, g, PELT, 8, BANK1).await;
    assert_eq!(split(&mut a, BANK1, BANK2, 3).await, None);
    assert_eq!(
        at(&realm, g, banked).await,
        Some((BAG_PLAYER, BANK_START, 5))
    );
    assert_eq!(destroy(&mut a, BANK2, 0).await, None);
    let left = on_map(&realm, move |m| {
        m.players[&g].item_at(BANK2.0, BANK2.1).is_none()
    })
    .await;
    assert!(left, "destroyed in the bank");
    on_map(&realm, move |m| {
        m.players
            .get_mut(&g)
            .unwrap()
            .fields
            .set_byte(PLAYER_BYTES_2, 2, 1)
    })
    .await;
    give_at(&realm, g, HERB_BAG, 1, BANK_BAG1).await;
    assert_eq!(
        split(&mut a, BANK1, (BANK_BAG_START, 0), 2)
            .await
            .as_deref(),
        Some("ItemDoesntGoIntoBag")
    );
    assert_eq!(
        at(&realm, g, banked).await,
        Some((BAG_PLAYER, BANK_START, 5))
    );
}

/// Thistle Tea's `destroy/3`: a worn item gone and no longer shown, one in
/// a bag gone from it; a bag with something in it kept; an empty slot
/// "not found"; part of a stack taken from it; the stack chosen, not
/// another of its kind.
#[tokio::test]
async fn destroying_takes_what_is_there() {
    let (realm, mut a, g) = setup().await;
    let chest = give_at(&realm, g, CHEST, 1, CHEST_SLOT).await;
    assert_eq!(destroy(&mut a, CHEST_SLOT, 0).await, None);
    assert_eq!(at(&realm, g, chest).await, None);
    let visible = on_map(&realm, move |m| {
        m.players[&g]
            .fields
            .get_u32(PLAYER_VISIBLE_ITEM_1_0 + 4 * VISIBLE_ITEM_SIZE)
    })
    .await;
    assert_eq!(visible, 0);

    let bag = give_at(&realm, g, BAG, 1, BAG1).await;
    let inside = give_at(&realm, g, CHEST, 1, (BAG_START, 0)).await;
    assert_eq!(
        destroy(&mut a, BAG1, 0).await.as_deref(),
        Some("CanOnlyDoWithEmptyBags")
    );
    assert_eq!(at(&realm, g, bag).await, Some((BAG_PLAYER, BAG_START, 1)));
    assert_eq!(destroy(&mut a, (BAG_START, 0), 0).await, None);
    assert_eq!(at(&realm, g, inside).await, None);
    assert_eq!(
        guid_field(&realm, g, bag, CONTAINER_FIELD_SLOT_1).await,
        Guid::zero()
    );
    assert_eq!(
        destroy(&mut a, CHEST_SLOT, 0).await.as_deref(),
        Some("ItemNotFound")
    );
    // Part of a stack: the rest kept where it is (Thistle Tea's
    // `reduce_stack/4`).
    let pelts = give_at(&realm, g, PELT, 8, PACK).await;
    assert_eq!(destroy(&mut a, PACK, 3).await, None);
    assert_eq!(at(&realm, g, pelts).await, Some((BAG_PLAYER, PACK.1, 5)));
    // The one chosen, not another of its kind.
    let other = give_at(&realm, g, PELT, 5, PACK2).await;
    assert_eq!(destroy(&mut a, PACK2, 0).await, None);
    assert_eq!(at(&realm, g, other).await, None);
    assert_eq!(at(&realm, g, pelts).await, Some((BAG_PLAYER, PACK.1, 5)));
}

/// Thistle Tea's bank transitions: swaps between the bags and the bank
/// every way; banking onto a stack there first and the rest beside it;
/// into a bank bag made for it before the bank's own slots; a full bank,
/// or full bags, leaving the item where it was; an item a bank bag isn't
/// made for kept out of it; bags only empty into and out of the bank's bag
/// slots; and an item found anywhere by its GUID (Thistle Tea's
/// `find_position/3`).
#[tokio::test]
async fn the_bank_takes_items_onto_its_stacks_and_into_its_bags() {
    let (realm, mut a, g) = setup().await;
    on_map(&realm, move |m| {
        m.players
            .get_mut(&g)
            .unwrap()
            .fields
            .set_byte(PLAYER_BYTES_2, 2, 2)
    })
    .await;
    let carried1 = give_at(&realm, g, CHEST, 1, PACK).await;
    let carried2 = give_at(&realm, g, RING, 1, PACK2).await;
    let banked1 = give_at(&realm, g, SWORD, 1, BANK1).await;
    let banked2 = give_at(&realm, g, SHIELD, 1, BANK2).await;
    assert_eq!(swap(&mut a, PACK, BANK1).await, None);
    assert_eq!(at(&realm, g, banked1).await, Some((BAG_PLAYER, PACK.1, 1)));
    assert_eq!(
        at(&realm, g, carried1).await,
        Some((BAG_PLAYER, BANK_START, 1))
    );
    assert_eq!(swap(&mut a, BANK1, BANK2).await, None);
    assert_eq!(
        at(&realm, g, banked2).await,
        Some((BAG_PLAYER, BANK_START, 1))
    );
    assert_eq!(
        at(&realm, g, carried1).await,
        Some((BAG_PLAYER, BANK_START + 1, 1))
    );
    assert_eq!(swap(&mut a, PACK2, PACK).await, None);
    assert_eq!(at(&realm, g, carried2).await, Some((BAG_PLAYER, PACK.1, 1)));
    assert_eq!(at(&realm, g, banked1).await, Some((BAG_PLAYER, PACK2.1, 1)));

    // Onto the banked stack first.
    let pelts = give_at(&realm, g, PELT, 5, PACK3).await;
    let stack = give_at(&realm, g, PELT, 8, (BAG_PLAYER, BANK_START + 2)).await;
    a.send(CMSG_AUTOBANK_ITEM {
        bag_index: PACK3.0,
        slot_index: PACK3.1,
    })
    .await;
    assert_eq!(refusal(&mut a).await, None);
    assert_eq!(
        at(&realm, g, stack).await,
        Some((BAG_PLAYER, BANK_START + 2, 10))
    );
    assert_eq!(
        at(&realm, g, pelts).await,
        Some((BAG_PLAYER, BANK_START + 3, 3))
    );
    // And back, onto a carried stack first.
    let carried = give_at(&realm, g, PELT, 8, PACK3).await;
    a.send(CMSG_AUTOSTORE_BANK_ITEM {
        bag_index: BAG_PLAYER,
        slot_index: BANK_START + 3,
    })
    .await;
    assert_eq!(refusal(&mut a).await, None);
    assert_eq!(
        at(&realm, g, carried).await,
        Some((BAG_PLAYER, PACK3.1, 10))
    );
    assert_eq!(
        at(&realm, g, pelts).await,
        Some((BAG_PLAYER, PACK3.1 + 1, 1))
    );

    // Herbs go into the herb bag in the bank before the bank's slots; an
    // ordinary item doesn't go in it.
    let herb_bag = give_at(&realm, g, HERB_BAG, 1, BANK_BAG1).await;
    let herb = give_at(&realm, g, HERB, 1, (BAG_PLAYER, PACK_START + 5)).await;
    a.send(CMSG_AUTOBANK_ITEM {
        bag_index: BAG_PLAYER,
        slot_index: PACK_START + 5,
    })
    .await;
    assert_eq!(refusal(&mut a).await, None);
    assert_eq!(at(&realm, g, herb).await, Some((BANK_BAG_START, 0, 1)));
    assert_eq!(
        swap(&mut a, PACK, (BANK_BAG_START, 1)).await.as_deref(),
        Some("ItemDoesntGoIntoBag")
    );
    let _ = herb_bag;

    // A bag with something in it doesn't go into the bank's bag slots; an
    // empty one goes in and comes out.
    let full = give_at(&realm, g, BAG, 1, BAG1).await;
    give_at(&realm, g, FILLER, 1, (BAG_START, 0)).await;
    let bank_bag2 = (BAG_PLAYER, BANK_BAG_START + 1);
    assert_eq!(
        swap(&mut a, BAG1, bank_bag2).await.as_deref(),
        Some("CanOnlyDoWithEmptyBags")
    );
    assert_eq!(destroy(&mut a, (BAG_START, 0), 0).await, None);
    assert_eq!(swap(&mut a, BAG1, bank_bag2).await, None);
    assert_eq!(
        at(&realm, g, full).await,
        Some((BAG_PLAYER, BANK_BAG_START + 1, 1))
    );
    assert_eq!(swap(&mut a, bank_bag2, BAG1).await, None);
    assert_eq!(at(&realm, g, full).await, Some((BAG_PLAYER, BAG_START, 1)));

    // An item in a bag found by its GUID and worn.
    let sword = give_at(&realm, g, SECOND_SWORD, 1, (BAG_START, 3)).await;
    a.send(CMSG_AUTOEQUIP_ITEM_SLOT {
        guid: sword,
        destination_slot: SLOT_MAIN_HAND,
    })
    .await;
    assert_eq!(refusal(&mut a).await, None);
    assert_eq!(
        at(&realm, g, sword).await,
        Some((BAG_PLAYER, SLOT_MAIN_HAND, 1))
    );
}

/// Thistle Tea's full bank and full bags: nothing moves either way.
#[tokio::test]
async fn a_full_bank_or_full_bags_leave_an_item_where_it_is() {
    let (realm, mut a, g) = setup().await;
    let source = give_at(&realm, g, CHEST, 1, PACK).await;
    for slot in BANK_START..crate::game::map::inventory::BANK_END {
        give_at(&realm, g, FILLER, 1, (BAG_PLAYER, slot)).await;
    }
    a.send(CMSG_AUTOBANK_ITEM {
        bag_index: PACK.0,
        slot_index: PACK.1,
    })
    .await;
    assert_eq!(refusal(&mut a).await.as_deref(), Some("BankFull"));
    assert_eq!(at(&realm, g, source).await, Some((BAG_PLAYER, PACK.1, 1)));

    let (realm, mut a, g) = setup().await;
    let source = give_at(&realm, g, CHEST, 1, BANK1).await;
    fill_pack(&realm, g).await;
    a.send(CMSG_AUTOSTORE_BANK_ITEM {
        bag_index: BANK1.0,
        slot_index: BANK1.1,
    })
    .await;
    assert_eq!(refusal(&mut a).await.as_deref(), Some("InventoryFull"));
    assert_eq!(
        at(&realm, g, source).await,
        Some((BAG_PLAYER, BANK_START, 1))
    );
}

/// Thistle Tea's `store/4`, `storable_count/4` and keyring: a new item in
/// the first free backpack slot, a bag's once the backpack is full, nowhere
/// once everything is; onto a stack and the rest beside it; the room
/// counted from stacks and free slots, a unique limit's (the bank's
/// counted toward it), and a soul bag's only for shards, the bank's bags
/// not; keys on the keyring until it is full, then in the backpack, and
/// nothing else on it; and not into the bank.
#[tokio::test]
async fn new_items_go_where_they_fit() {
    let (realm, _a, g) = setup().await;
    let add = |entry: u32, count: u32| {
        let realm = realm.clone();
        async move {
            on_map(&realm, move |m| {
                inventory::add_item(m, g, entry, count, false)
            })
            .await
        }
    };
    let newest = |entry: u32| {
        let realm = realm.clone();
        async move {
            on_map(&realm, move |m| {
                let p = &m.players[&g];
                let i = p.items.iter().rposition(|i| i.entry == entry).unwrap();
                (p.items[i].bag, p.items[i].slot, p.items[i].count())
            })
            .await
        }
    };
    give_at(&realm, g, FILLER, 1, BANK1).await;
    assert_eq!(add(CHEST, 1).await, Ok(()));
    assert_eq!(
        newest(CHEST).await,
        (BAG_PLAYER, PACK_START, 1),
        "not the bank"
    );
    assert_eq!(add(PELT, 8).await, Ok(()));
    assert_eq!(add(PELT, 5).await, Ok(()));
    let pelts = on_map(&realm, move |m| {
        let p = &m.players[&g];
        let mut v: Vec<_> = p
            .items
            .iter()
            .filter(|i| i.entry == PELT)
            .map(|i| (i.slot, i.count()))
            .collect();
        v.sort();
        v
    })
    .await;
    assert_eq!(pelts, vec![(PACK_START + 1, 10), (PACK_START + 2, 3)]);

    // Room: two on the stack of eighteen and one free slot's twenty.
    let mut stack = item(9_400_100);
    stack.stackable = 20;
    let mut limited = stack.clone();
    limited.entry = 9_400_101;
    limited.max_count = 21;
    let mut banked_unique = stack.clone();
    banked_unique.entry = 9_400_102;
    banked_unique.max_count = 3;
    let mut d = data();
    for t in [stack, limited, banked_unique] {
        d.items.insert(t.entry, t);
    }
    let (realm, a, g) = setup_with(d).await;
    give_at(&realm, g, 9_400_100, 18, PACK).await;
    give_at(&realm, g, 9_400_101, 18, PACK2).await;
    for slot in PACK_START + 2..PACK_END - 1 {
        give_at(&realm, g, FILLER, 1, (BAG_PLAYER, slot)).await;
    }
    let room = on_map(&realm, move |m| {
        let d = m.realm.data.clone();
        let p = &m.players[&g];
        (
            inventory::fitting(p, &d, 9_400_100, 25),
            inventory::fitting(p, &d, 9_400_101, 25),
            inventory::room_for(p, &d, 9_400_100, 22),
            inventory::room_for(p, &d, 9_400_100, 23),
        )
    })
    .await;
    assert_eq!(
        room,
        (Ok(22), Ok(3), Ok(()), Err(inventory::Fail::InventoryFull))
    );
    give_at(&realm, g, 9_400_102, 2, BANK1).await;
    let unique = on_map(&realm, move |m| {
        let d = m.realm.data.clone();
        let p = &m.players[&g];
        (
            inventory::fitting(p, &d, 9_400_102, 5),
            inventory::room_for(p, &d, 9_400_102, 2),
        )
    })
    .await;
    assert_eq!(unique, (Ok(1), Err(inventory::Fail::CantCarryMore)));
    // Everything full.
    give_at(&realm, g, FILLER, 1, (BAG_PLAYER, PACK_END - 1)).await;
    let full = on_map(&realm, move |m| inventory::add_item(m, g, CHEST, 1, false)).await;
    assert_eq!(full, Err(inventory::Fail::InventoryFull));
    let _ = a;

    // A soul bag carried and one in the bank: shards fill the carried one
    // and the backpack, anything else the backpack.
    let (realm, _a, g) = setup().await;
    on_map(&realm, move |m| {
        m.players
            .get_mut(&g)
            .unwrap()
            .fields
            .set_byte(PLAYER_BYTES_2, 2, 1)
    })
    .await;
    give_at(&realm, g, SOUL_BAG, 1, BAG1).await;
    give_at(&realm, g, SOUL_BAG, 1, BANK_BAG1).await;
    let room = on_map(&realm, move |m| {
        let d = m.realm.data.clone();
        let p = &m.players[&g];
        (
            inventory::fitting(p, &d, SHARD, 100),
            inventory::fitting(p, &d, FILLER, 100),
        )
    })
    .await;
    assert_eq!(room, (Ok(22), Ok(16)));
    // The overflow into a plain bag.
    let (realm, _a, g) = setup().await;
    give_at(&realm, g, BAG, 1, BAG1).await;
    fill_pack(&realm, g).await;
    let placed = on_map(&realm, move |m| {
        inventory::add_item(m, g, CHEST, 1, false).unwrap();
        let p = &m.players[&g];
        let i = p.items.iter().rposition(|i| i.entry == CHEST).unwrap();
        (p.items[i].bag, p.items[i].slot)
    })
    .await;
    assert_eq!(placed, (BAG_START, 0));

    // Keys: four on a first-level keyring, the fifth in the backpack; a
    // chest never on it.
    let (realm, _a, g) = setup().await;
    let placed = on_map(&realm, move |m| {
        for _ in 0..5 {
            inventory::add_item(m, g, KEY, 1, false).unwrap();
        }
        inventory::add_item(m, g, CHEST, 1, false).unwrap();
        let p = &m.players[&g];
        let mut keys: Vec<u8> = p
            .items
            .iter()
            .filter(|i| i.entry == KEY)
            .map(|i| i.slot)
            .collect();
        keys.sort();
        let chest = p.items.iter().find(|i| i.entry == CHEST).unwrap().slot;
        (keys, chest)
    })
    .await;
    assert_eq!(
        placed,
        (
            vec![
                PACK_START,
                KEYRING_START,
                KEYRING_START + 1,
                KEYRING_START + 2,
                KEYRING_START + 3,
            ],
            PACK_START + 1
        )
    );
}

/// Thistle Tea's `remove_count/4`, its removals carried only: a spell's
/// six reagents take a whole stack and part of the next; with too few
/// carried, the banked ones not counted, it isn't cast.
#[tokio::test]
async fn reagents_come_from_the_stacks_carried() {
    let (realm, mut a, g) = setup().await;
    learn(&realm, g, REAGENT_SPELL).await;
    // Not a GM, who casts without reagents.
    on_map(&realm, move |m| m.players.get_mut(&g).unwrap().gm = false).await;
    let first = give_at(&realm, g, PELT, 4, PACK).await;
    let second = give_at(&realm, g, PELT, 4, PACK2).await;
    let banked = give_at(&realm, g, PELT, 10, BANK1).await;
    cast(&a, REAGENT_SPELL).await;
    a.wait_for("SMSG_SPELL_GO", |m| match m {
        ServerOpcodeMessage::SMSG_SPELL_GO(s) if s.spell == REAGENT_SPELL => Some(()),
        _ => None,
    })
    .await;
    handled(&mut a).await;
    let left = (
        at(&realm, g, first).await,
        at(&realm, g, second).await,
        at(&realm, g, banked).await,
    );
    let one_left = match left {
        (None, Some((_, _, 2)), Some((_, _, 10))) | (Some((_, _, 2)), None, Some((_, _, 10))) => {
            true
        }
        _ => false,
    };
    assert!(one_left, "a stack and two of the next: {left:?}");
    // Two carried, ten banked: refused, nothing taken.
    a.clear();
    cast(&a, REAGENT_SPELL).await;
    let refused = a
        .wait_frame("SMSG_CAST_RESULT", 0x130, |f| {
            f[..4] == REAGENT_SPELL.to_le_bytes()
        })
        .await;
    assert_ne!(refused[4], 0, "failed");
    handled(&mut a).await;
    let carried = on_map(&realm, move |m| m.players[&g].carried_count_of(PELT)).await;
    assert_eq!(carried, 2);
    assert_eq!(
        at(&realm, g, banked).await,
        Some((BAG_PLAYER, BANK_START, 10))
    );
}

/// Thistle Tea's `plan/2`, the change its quest turn-ins and trades are
/// made in: unique limits against earlier additions, a unique taken and
/// given in one change, slots freed by what is taken, additions that only
/// fit apart refused together, later additions stacked onto earlier ones,
/// nothing when a removal can't be met, and removals carried only.
#[tokio::test]
async fn a_change_is_planned_whole() {
    let (realm, _a, g) = setup().await;
    let plan = |removals: Vec<(u32, u32)>, additions: Vec<(u32, u32)>| {
        let realm = realm.clone();
        async move {
            on_map(&realm, move |m| {
                let d = m.realm.data.clone();
                inventory::plan_change(&m.players[&g], &d, &removals, &additions)
            })
            .await
        }
    };
    give_at(&realm, g, CHEST, 1, PACK).await;
    assert_eq!(
        plan(vec![(CHEST, 1)], vec![(UNIQUE, 1), (UNIQUE, 1)]).await,
        Err(inventory::Fail::CantCarryMore)
    );
    assert_eq!(plan(vec![], vec![(UNIQUE, 1)]).await, Ok(()));
    give_at(&realm, g, UNIQUE, 1, PACK2).await;
    assert_eq!(
        plan(vec![], vec![(UNIQUE, 1)]).await,
        Err(inventory::Fail::CantCarryMore)
    );
    assert_eq!(plan(vec![(UNIQUE, 1)], vec![(UNIQUE, 1)]).await, Ok(()));
    // Two free slots left of sixteen, then one.
    for slot in PACK_START + 2..PACK_END - 2 {
        give_at(&realm, g, FILLER, 1, (BAG_PLAYER, slot)).await;
    }
    assert_eq!(plan(vec![], vec![(RING, 1), (SWORD, 1)]).await, Ok(()));
    give_at(&realm, g, FILLER, 1, (BAG_PLAYER, PACK_END - 2)).await;
    assert_eq!(plan(vec![], vec![(RING, 1)]).await, Ok(()));
    assert_eq!(
        plan(vec![], vec![(RING, 1), (SWORD, 1)]).await,
        Err(inventory::Fail::InventoryFull),
        "each fits alone"
    );
    assert_eq!(
        plan(vec![(CHEST, 1)], vec![(RING, 1), (SWORD, 1)]).await,
        Ok(())
    );
    assert_eq!(
        plan(vec![], vec![(PELT, 4), (PELT, 3)]).await,
        Ok(()),
        "one stack"
    );
    assert_eq!(
        plan(vec![], vec![(PELT, 4), (PELT, 7)]).await,
        Err(inventory::Fail::InventoryFull)
    );
    assert_eq!(
        plan(vec![(CHEST, 2)], vec![(RING, 1)]).await,
        Err(inventory::Fail::NotFound)
    );
    give_at(&realm, g, SHIELD, 1, BANK1).await;
    assert_eq!(
        plan(vec![(SHIELD, 1)], vec![]).await,
        Err(inventory::Fail::NotFound),
        "banked"
    );
}

fn giver() -> Guid {
    crate::game::guid::unit(GIVER, 1)
}

async fn turn_in(a: &mut Client, quest_id: u32) -> bool {
    a.clear();
    a.send(CMSG_QUESTGIVER_CHOOSE_REWARD {
        guid: giver(),
        quest_id,
        reward: 0,
    })
    .await;
    handled(a).await;
    a.gets(Duration::from_millis(1), |m| match m {
        ServerOpcodeMessage::SMSG_QUESTGIVER_QUEST_COMPLETE(r) => {
            (r.quest_id == quest_id).then_some(())
        }
        _ => None,
    })
    .await
    .is_some()
}

async fn accept(a: &mut Client, realm: &Arc<Realm>, g: Guid, quest_id: u32) {
    a.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: giver(),
        quest_id,
    })
    .await;
    handled(a).await;
    let has = on_map(realm, move |m| m.players[&g].journal.has(quest_id)).await;
    assert!(has, "quest {quest_id} taken");
}

/// A quest turned in with the bags full: its reward where the item it
/// takes was; two rewards for one free slot refused; an item it wants in
/// the bank not taken (mangos's "quest items in bank").
#[tokio::test]
async fn a_turn_in_rewards_in_the_room_its_items_leave() {
    let (realm, mut a, g) = setup().await;
    a.sees(giver()).await;
    accept(&mut a, &realm, g, PELT_QUEST).await;
    on_map(&realm, move |m| {
        inventory::add_item(m, g, PELT, 1, false).unwrap()
    })
    .await;
    fill_pack(&realm, g).await;
    assert!(turn_in(&mut a, PELT_QUEST).await, "a full pack");
    let chest = on_map(&realm, move |m| {
        let p = &m.players[&g];
        (
            p.count_of(PELT),
            p.item_at(BAG_PLAYER, PACK_START).map(|i| p.items[i].entry),
        )
    })
    .await;
    assert_eq!(chest, (0, Some(CHEST)));

    // One slot free: two rewards don't fit.
    on_map(&realm, move |m| {
        let p = &m.players[&g];
        let i = p.item_at(BAG_PLAYER, PACK_START).unwrap();
        inventory::take_item(m, g, i, 1);
    })
    .await;
    accept(&mut a, &realm, g, TWO_QUEST).await;
    assert!(!turn_in(&mut a, TWO_QUEST).await);
    a.clear();

    // A ring banked isn't taken for the quest.
    let (realm, mut a, g) = setup().await;
    a.sees(giver()).await;
    accept(&mut a, &realm, g, RING_QUEST).await;
    let ring = give_at(&realm, g, RING, 1, BANK1).await;
    assert!(!turn_in(&mut a, RING_QUEST).await, "banked");
    assert_eq!(at(&realm, g, ring).await, Some((BAG_PLAYER, BANK_START, 1)));
    swap(&mut a, BANK1, PACK).await;
    assert!(turn_in(&mut a, RING_QUEST).await, "carried");
    assert_eq!(at(&realm, g, ring).await, None);
}
