//! Phase 4's bags and items, Thistle Tea's `InventoryDestinationTest`,
//! `ReplacementTest` and item tests ported: a purchase dropped on a slot
//! (that slot, then its bag, then anywhere; refused where it can't go; a
//! two-hander put on putting the off hand away), an item changed into
//! another by its spell (where it lay, if it may lie there; its own
//! property, the old one's enchantments and wear), timed items (ordinary
//! ones wait while their owner is away, real-time ones don't; through a
//! gift and in a bag), an opened container locked, a bag bound elsewhere
//! going with what is in it, and an item used only by whom it may be worn
//! by.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::enchant::{self, Enchant};
use crate::game::map::inventory::{
    BAG_START, BANK_BAG_START, BANK_START, PACK_END, PACK_START, SLOT_MAIN_HAND, SLOT_OFF_HAND,
};
use crate::game::map::{Map, MapKey};
use crate::game::store::BAG_PLAYER;

const FILLER: u32 = 1000;
const PELT: u32 = 750;
const HERB: u32 = 2000;
const UNIQUE: u32 = 6948;
const BAG: u32 = 800;
const SMALL_BAG: u32 = 803;
const HERB_BAG: u32 = 801;
const SOUL_BAG: u32 = 802;
const SWORD: u32 = 200;
const GREATSWORD: u32 = 300;
const SHIELD: u32 = 400;
const RING: u32 = 500;
/// Timed items: one that waits while its owner is away, one that doesn't,
/// a bag of its own time, one of a second, and a real-time bag.
const TIMED: u32 = 2001;
const REAL: u32 = 2002;
const TIMED_BAG: u32 = 2003;
const SHORT: u32 = 2004;
const REAL_BAG: u32 = 2005;
const TIMED_ONE: u32 = 2006;
/// Items changed into others, the spells that do it, and what they
/// change into.
const OLD: u32 = 3001;
const MID: u32 = 3002;
const FINAL: u32 = 3003;
const SHARD: u32 = 3004;
const OTHER_BAG: u32 = 3005;
const PROPERTIED: u32 = 3006;
const NEW_PROPERTY: u32 = 3007;
const TO_MID: u32 = 9_500_001;
const TO_FINAL: u32 = 9_500_002;
/// A container that rolls its loot when opened, and what is in it.
const CRATE: u32 = 4001;
const CRATE_LOOT: u32 = 4002;
/// A bag bound to Alterac Valley.
const AV_BAG: u32 = 4003;
/// Wrapping paper and its gift (as mangos's `HandleWrapItemOpcode` pairs
/// them).
const PAPER: u32 = 5042;
const GIFT: u32 = 5043;
/// An item used, its spell, and a spell with a cast time.
const USE_SPELL: u32 = 9_500_010;
const SLOW_SPELL: u32 = 9_500_011;
/// Cloth, so a player's proficiency comes from their spells.
const CLOTH: u32 = 9078;
const WEAPONS: u32 = 9_400_001;
const ARMOR: u32 = 9_400_002;
const VENDOR: u32 = 2100;
/// Random properties: the one an item has, and the one an item changed
/// into rolls.
const PROPERTY: u32 = 1182;
const ROLLED: u32 = 19;
const POOL: u32 = 77;
const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

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

fn bag(entry: u32, family: u32, slots: u32) -> ItemTemplate {
    ItemTemplate {
        class: 1,
        inventory_type: 18,
        container_slots: slots,
        bag_family: family,
        ..item(entry)
    }
}

fn spell(id: u32, effect: SpellEffect) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        range_index: 1,
        equipped_item_class: -1,
        ..Default::default()
    };
    s.effects[0] = effect;
    s
}

fn on_self(effect: u32) -> SpellEffect {
    SpellEffect {
        effect,
        target_a: 1,
        ..Default::default()
    }
}

fn proficiency(id: u32, class: i32) -> SpellTemplate {
    let mut s = spell(id, on_self(crate::game::map::spells::EFFECT_PROFICIENCY));
    s.equipped_item_class = class;
    s.equipped_item_subclass_mask = -1;
    s
}

/// An item whose use casts `spell`.
fn used(entry: u32, spell: u32) -> ItemTemplate {
    let mut t = item(entry);
    t.spells[0] = crate::game::data::items::ItemSpell {
        id: spell,
        trigger: 0,
        cooldown: 1000,
        ..Default::default()
    };
    t
}

fn data() -> GameData {
    let mut d = GameData::empty();
    let mut greatsword = worn(GREATSWORD, 2, 8, 17);
    greatsword.bonding = crate::game::map::items::BIND_ON_EQUIP;
    for t in [
        item(FILLER),
        bag(BAG, 0, 6),
        bag(SMALL_BAG, 0, 2),
        bag(HERB_BAG, 0x20, 6),
        bag(SOUL_BAG, 4, 6),
        bag(OTHER_BAG, 0, 4),
        worn(SWORD, 2, 7, 13),
        greatsword,
        worn(SHIELD, 4, 6, 14),
        worn(RING, 4, 0, 11),
        used(OLD, TO_MID),
        used(MID, TO_FINAL),
        item(FINAL),
    ] {
        d.items.insert(t.entry, t);
    }
    for (entry, stack, family) in [(PELT, 10, 0), (HERB, 20, 0x20), (SHARD, 1, 4)] {
        let mut t = item(entry);
        t.stackable = stack;
        t.bag_family = family;
        d.items.insert(entry, t);
    }
    let mut unique = item(UNIQUE);
    unique.max_count = 1;
    d.items.insert(UNIQUE, unique);
    let mut paper = item(PAPER);
    paper.class = 15;
    paper.flags = 0x200;
    paper.stackable = 10;
    d.items.insert(PAPER, paper);
    let mut gift = item(GIFT);
    gift.flags = 0x200;
    d.items.insert(GIFT, gift);
    // Timed: five minutes each, a second for one.
    for (entry, stack, flags, slots, duration) in [
        (TIMED, 20, 0, 0, 300),
        (REAL, 1, 0x10000, 0, 300),
        (TIMED_BAG, 1, 0, 4, 300),
        (SHORT, 1, 0, 0, 1),
        (REAL_BAG, 1, 0x10000, 4, 300),
        (TIMED_ONE, 1, 0, 0, 300),
    ] {
        let mut t = if slots > 0 {
            bag(entry, 0, slots)
        } else {
            item(entry)
        };
        t.stackable = stack;
        t.flags = flags;
        t.duration = duration;
        d.items.insert(entry, t);
    }
    // Changed into others.
    let mut propertied = worn(PROPERTIED, 2, 7, 13);
    propertied.max_durability = 100;
    d.items.insert(PROPERTIED, propertied);
    let mut new_property = worn(NEW_PROPERTY, 2, 7, 13);
    new_property.random_property = POOL;
    d.items.insert(NEW_PROPERTY, new_property);
    d.crafts.random_pools.insert(POOL, vec![(ROLLED, 100.0)]);
    d.crafts
        .random_properties
        .insert(PROPERTY, [72, 69, 0, 0, 0]);
    d.crafts.random_properties.insert(ROLLED, [72, 0, 0, 0, 0]);
    for (id, into) in [(TO_MID, MID), (TO_FINAL, FINAL)] {
        let mut s = spell(id, on_self(34));
        s.effects[0].misc_value = into as i32;
        d.spells.spells.insert(id, s);
    }
    // Wear carried over: maxima of 100, 80, 20 and none.
    for (entry, max) in [(6001, 100), (6002, 80), (6003, 20), (6004, 0)] {
        let mut t = worn(entry, 4, 1, 5);
        t.max_durability = max;
        d.items.insert(entry, t);
    }
    // A crate opened for its loot.
    let mut crate_ = item(CRATE);
    crate_.flags = 0x4;
    crate_.stackable = 20;
    crate_.min_money_loot = 1;
    crate_.max_money_loot = 1;
    d.items.insert(CRATE, crate_);
    d.items.insert(CRATE_LOOT, item(CRATE_LOOT));
    d.loot.item.insert(
        CRATE,
        vec![LootEntry {
            item: CRATE_LOOT,
            chance: 100.0,
            min: 1,
            max: 1,
            ..Default::default()
        }],
    );
    let mut av_bag = bag(AV_BAG, 0, 4);
    av_bag.map_bound = 30;
    d.items.insert(AV_BAG, av_bag);
    // Item use, and a slow cast.
    d.spells
        .spells
        .insert(USE_SPELL, spell(USE_SPELL, on_self(3)));
    let mut slow = spell(SLOW_SPELL, on_self(3));
    slow.casting_time_index = 5;
    d.spells.spells.insert(SLOW_SPELL, slow);
    d.spells.cast_times.insert(5, 3000);
    let mut cloth = proficiency(CLOTH, 4);
    cloth.equipped_item_subclass_mask = 1 << 1;
    d.spells.spells.insert(CLOTH, cloth);
    d.spells.spells.insert(WEAPONS, proficiency(WEAPONS, 2));
    d.spells.spells.insert(ARMOR, proficiency(ARMOR, 4));
    // A vendor of most of it.
    d.creature_templates.insert(
        VENDOR,
        CreatureTemplate {
            entry: VENDOR,
            name: "Vendor".into(),
            display_ids: [903, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            display_scales: [1.0, 0.0, 0.0, 0.0],
            level_min: 2,
            level_max: 2,
            faction: 35,
            unit_class: 1,
            health_max: 50,
            npc_flags: 0x4,
            ..Default::default()
        },
    );
    d.creature_spawns.insert(
        0,
        vec![CreatureSpawn {
            guid: 1,
            entries: vec![VENDOR],
            map: 0,
            x: START.0 + 2.0,
            y: START.1,
            z: START.2,
            ..Default::default()
        }],
    );
    d.vendors.insert(
        VENDOR,
        [PELT, UNIQUE, GREATSWORD, FILLER, CRATE]
            .iter()
            .enumerate()
            .map(|(i, item)| VendorItem {
                slot: i as u32,
                item: *item,
                ..Default::default()
            })
            .collect(),
    );
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

/// A warrior proficient in every weapon and armor, carrying nothing.
async fn setup() -> (Arc<Realm>, Client, Guid) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        p.spells.extend([CLOTH, WEAPONS, ARMOR]);
        p.items.clear();
        let d = m.realm.data.clone();
        m.players.get_mut(&g).unwrap().refresh_inventory(&d);
    })
    .await;
    a.clear();
    (realm, a, g)
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

/// What is at `(bag, slot)`: its entry and count.
async fn holds(realm: &Arc<Realm>, g: Guid, at: (u8, u8)) -> Option<(u32, u32)> {
    on_map(realm, move |m| {
        let p = &m.players[&g];
        p.item_at(at.0, at.1)
            .map(|i| (p.items[i].entry, p.items[i].count()))
    })
    .await
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

async fn field_at(realm: &Arc<Realm>, g: Guid, at: (u8, u8), f: u16) -> Option<u32> {
    on_map(realm, move |m| {
        let p = &m.players[&g];
        p.item_at(at.0, at.1).map(|i| p.items[i].fields.get_u32(f))
    })
    .await
}

async fn count(realm: &Arc<Realm>, g: Guid, entry: u32) -> u32 {
    on_map(realm, move |m| m.players[&g].count_of(entry)).await
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

fn vendor() -> Guid {
    guid::unit(VENDOR, 1)
}

/// Buy `amount` of `item` dropped on `slot` of `bag` (a bag's guid, or the
/// player's for the backpack); the refusal.
async fn buy_in(a: &mut Client, item: u32, amount: u8, bag: Guid, slot: u8) -> Option<String> {
    a.clear();
    a.send(CMSG_BUY_ITEM_IN_SLOT {
        vendor: vendor(),
        item,
        bag,
        bag_slot: slot,
        amount,
    })
    .await;
    refusal(a).await
}

const PACK: (u8, u8) = (BAG_PLAYER, PACK_START);
const PACK2: (u8, u8) = (BAG_PLAYER, PACK_START + 1);
const BAG1: (u8, u8) = (BAG_PLAYER, BAG_START);
const MAIN: (u8, u8) = (BAG_PLAYER, SLOT_MAIN_HAND);
const OFF: (u8, u8) = (BAG_PLAYER, SLOT_OFF_HAND);

/// Thistle Tea's explicit destinations: a purchase dropped on an empty
/// backpack slot starts a stack there, not on the partial stack carried;
/// one dropped on a stack in a bag fills it, then the rest of that bag,
/// then goes wherever else it fits.
#[tokio::test]
async fn a_purchase_goes_to_the_slot_it_is_dropped_on_then_its_bag() {
    let (realm, mut a, g) = setup().await;
    a.sees(vendor()).await;
    let stack = give_at(&realm, g, PELT, 8, PACK).await;
    assert_eq!(buy_in(&mut a, PELT, 5, g, PACK_START + 7).await, None);
    assert_eq!(
        holds(&realm, g, (BAG_PLAYER, PACK_START + 7)).await,
        Some((PELT, 5)),
        "where it was dropped"
    );
    assert_eq!(
        at(&realm, g, stack).await,
        Some((BAG_PLAYER, PACK_START, 8))
    );

    // Onto a stack in a bag: that stack, then that bag, then anywhere.
    let small = give_at(&realm, g, SMALL_BAG, 1, BAG1).await;
    let in_bag = give_at(&realm, g, PELT, 8, (BAG_START, 1)).await;
    assert_eq!(buy_in(&mut a, PELT, 5, small, 1).await, None);
    assert_eq!(at(&realm, g, in_bag).await, Some((BAG_START, 1, 10)));
    assert_eq!(holds(&realm, g, (BAG_START, 0)).await, Some((PELT, 3)));
    assert_eq!(
        at(&realm, g, stack).await,
        Some((BAG_PLAYER, PACK_START, 8))
    );
    let contained = on_map(&realm, move |m| {
        let p = &m.players[&g];
        p.items[p.item_at(BAG_START, 0).unwrap()]
            .fields
            .get_guid(ITEM_FIELD_CONTAINED)
    })
    .await;
    assert_eq!(contained, small, "in the bag");
    // The bag's room gone, the rest onto the stack carried elsewhere.
    assert_eq!(buy_in(&mut a, PELT, 9, small, 0).await, None);
    assert_eq!(holds(&realm, g, (BAG_START, 0)).await, Some((PELT, 10)));
    assert_eq!(
        at(&realm, g, stack).await,
        Some((BAG_PLAYER, PACK_START, 10))
    );
    assert_eq!(count(&realm, g, PELT).await, 8 + 5 + 5 + 9 + 8);
}

/// Thistle Tea's preferred backpack spilling into a bag, and its refused
/// destinations: an occupied slot or a full stack (it can't stack), a slot
/// past the bag's end, and a bag not made for it; nothing bought then. A
/// unique one dropped on a free slot, one in the bank, is refused too.
#[tokio::test]
async fn a_purchase_spills_from_a_full_backpack_or_is_refused_where_it_cant_go() {
    let (realm, mut a, g) = setup().await;
    a.sees(vendor()).await;
    give_at(&realm, g, BAG, 1, BAG1).await;
    fill_pack(&realm, g).await;
    assert_eq!(buy_in(&mut a, FILLER, 1, g, u8::MAX).await, None);
    assert_eq!(holds(&realm, g, (BAG_START, 0)).await, Some((FILLER, 1)));

    let (realm, mut a, g) = setup().await;
    a.sees(vendor()).await;
    give_at(&realm, g, FILLER, 1, PACK).await;
    give_at(&realm, g, PELT, 10, PACK2).await;
    for slot in [PACK.1, PACK2.1] {
        assert_eq!(
            buy_in(&mut a, PELT, 1, g, slot).await.as_deref(),
            Some("ItemCantStack"),
            "{slot}"
        );
    }
    let herbs = give_at(&realm, g, HERB_BAG, 1, BAG1).await;
    assert_eq!(
        buy_in(&mut a, PELT, 1, herbs, 0).await.as_deref(),
        Some("ItemDoesntGoIntoBag")
    );
    assert_eq!(
        buy_in(&mut a, PELT, 1, herbs, u8::MAX).await.as_deref(),
        Some("ItemDoesntGoIntoBag")
    );
    assert_eq!(
        buy_in(&mut a, PELT, 1, herbs, 6).await.as_deref(),
        Some("ItemDoesntGoToSlot"),
        "past its end"
    );
    assert_eq!(count(&realm, g, PELT).await, 10, "nothing bought");

    // A unique one, one banked: not carried another of.
    give_at(&realm, g, UNIQUE, 1, (BAG_PLAYER, BANK_START)).await;
    assert_eq!(
        buy_in(&mut a, UNIQUE, 1, g, PACK_START + 5)
            .await
            .as_deref(),
        Some("CantCarryMoreOfThis")
    );
    assert_eq!(count(&realm, g, UNIQUE).await, 1);
}

/// Thistle Tea's two-handed purchase: dropped on the main hand it is put
/// on, bound, the shield put away; with nowhere for it, nothing is bought.
#[tokio::test]
async fn a_two_hander_bought_onto_the_main_hand_puts_the_shield_away() {
    let (realm, mut a, g) = setup().await;
    a.sees(vendor()).await;
    let shield = give_at(&realm, g, SHIELD, 1, OFF).await;
    assert_eq!(buy_in(&mut a, GREATSWORD, 1, g, SLOT_MAIN_HAND).await, None);
    assert_eq!(holds(&realm, g, MAIN).await, Some((GREATSWORD, 1)));
    assert_eq!(
        field_at(&realm, g, MAIN, ITEM_FIELD_FLAGS)
            .await
            .map(|f| f & 1),
        Some(1)
    );
    assert_eq!(
        at(&realm, g, shield).await.map(|(b, _, _)| b),
        Some(BAG_PLAYER)
    );
    assert!(
        at(&realm, g, shield)
            .await
            .is_some_and(|(_, s, _)| (PACK_START..PACK_END).contains(&s))
    );
    let (main, off) = on_map(&realm, move |m| {
        let f = &m.players[&g].fields;
        let shown =
            |slot: u8| f.get_u32(PLAYER_VISIBLE_ITEM_1_0 + u16::from(slot) * VISIBLE_ITEM_SIZE);
        (shown(SLOT_MAIN_HAND), shown(SLOT_OFF_HAND))
    })
    .await;
    assert_eq!((main, off), (GREATSWORD, 0));

    let (realm, mut a, g) = setup().await;
    a.sees(vendor()).await;
    let shield = give_at(&realm, g, SHIELD, 1, OFF).await;
    fill_pack(&realm, g).await;
    assert_eq!(
        buy_in(&mut a, GREATSWORD, 1, g, SLOT_MAIN_HAND)
            .await
            .as_deref(),
        Some("InventoryFull")
    );
    assert_eq!(
        at(&realm, g, shield).await,
        Some((BAG_PLAYER, SLOT_OFF_HAND, 1))
    );
    assert_eq!(count(&realm, g, GREATSWORD).await, 0, "not bought");
}

/// Change the item at `at` into `entry`, as its spell's item change
/// effect does; the refusal.
async fn change(
    a: &mut Client,
    realm: &Arc<Realm>,
    g: Guid,
    at: (u8, u8),
    entry: u32,
) -> Option<String> {
    a.clear();
    on_map(realm, move |m| {
        let p = &m.players[&g];
        let item = p.items[p.item_at(at.0, at.1).expect("an item there")].guid;
        crate::game::map::extra_effects::change_item(m, g, item, entry);
    })
    .await;
    refusal(a).await
}

async fn use_item(a: &mut Client, at: (u8, u8)) -> Option<String> {
    a.clear();
    a.send(CMSG_USE_ITEM {
        bag_index: at.0,
        bag_slot: at.1,
        spell_index: 0,
        targets: SpellCastTargets {
            target_flags: SpellCastTargets_SpellCastTargetFlags::empty(),
        },
    })
    .await;
    refusal(a).await
}

/// Thistle Tea's replacements: used twice, an item becomes the second
/// change's, where it lay; in a bag and in a bank bag it stays in it, and
/// a unique one may become itself.
#[tokio::test]
async fn an_item_changed_twice_is_the_last_where_it_lay_in_bags_and_the_bank() {
    let (realm, mut a, g) = setup().await;
    give_at(&realm, g, OLD, 1, PACK2).await;
    assert_eq!(use_item(&mut a, PACK2).await, None);
    assert_eq!(holds(&realm, g, PACK2).await, Some((MID, 1)));
    on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().caster.cooldowns.clear()
    })
    .await;
    assert_eq!(use_item(&mut a, PACK2).await, None);
    assert_eq!(holds(&realm, g, PACK2).await, Some((FINAL, 1)));
    assert_eq!(count(&realm, g, OLD).await + count(&realm, g, MID).await, 0);

    let bag = give_at(&realm, g, BAG, 1, BAG1).await;
    on_map(&realm, move |m| {
        m.players
            .get_mut(&g)
            .unwrap()
            .fields
            .set_byte(PLAYER_BYTES_2, 2, 1)
    })
    .await;
    let bank_bag = give_at(&realm, g, BAG, 1, (BAG_PLAYER, BANK_BAG_START)).await;
    for (bag, at) in [(bag, (BAG_START, 0)), (bank_bag, (BANK_BAG_START, 0))] {
        give_at(&realm, g, OLD, 1, at).await;
        assert_eq!(change(&mut a, &realm, g, at, MID).await, None);
        assert_eq!(holds(&realm, g, at).await, Some((MID, 1)));
        let (slot, contained) = on_map(&realm, move |m| {
            let p = &m.players[&g];
            let it = &p.items[p.item_at(at.0, at.1).unwrap()];
            let b = &p.items[p.item_by_guid(bag).unwrap()];
            (
                b.fields.get_guid(CONTAINER_FIELD_SLOT_1),
                (it.guid, it.fields.get_guid(ITEM_FIELD_CONTAINED)),
            )
        })
        .await;
        assert_eq!(slot, contained.0, "the bag holds it");
        assert_eq!(contained.1, bag);
    }
    give_at(&realm, g, UNIQUE, 1, PACK).await;
    assert_eq!(change(&mut a, &realm, g, PACK, UNIQUE).await, None);
    assert_eq!(holds(&realm, g, PACK).await, Some((UNIQUE, 1)));
}

/// Thistle Tea's refused replacements, the item left as it was: one in a
/// soul bag into what doesn't go there, a bag with something in it, a
/// unique one another of which is banked, a ring in the main hand, and a
/// two-hander whose off hand has nowhere to go. A sword with a shield
/// beside it changed into a two-hander puts the shield away.
#[tokio::test]
async fn an_item_changes_only_into_what_may_lie_where_it_does() {
    let (realm, mut a, g) = setup().await;
    give_at(&realm, g, SOUL_BAG, 1, BAG1).await;
    give_at(&realm, g, SHARD, 1, (BAG_START, 0)).await;
    assert_eq!(
        change(&mut a, &realm, g, (BAG_START, 0), MID)
            .await
            .as_deref(),
        Some("ItemDoesntGoIntoBag")
    );
    assert_eq!(holds(&realm, g, (BAG_START, 0)).await, Some((SHARD, 1)));
    assert_eq!(
        change(&mut a, &realm, g, BAG1, OTHER_BAG).await.as_deref(),
        Some("CanOnlyDoWithEmptyBags")
    );
    assert_eq!(holds(&realm, g, BAG1).await, Some((SOUL_BAG, 1)));
    give_at(&realm, g, UNIQUE, 1, (BAG_PLAYER, BANK_START)).await;
    give_at(&realm, g, OLD, 1, PACK).await;
    assert_eq!(
        change(&mut a, &realm, g, PACK, UNIQUE).await.as_deref(),
        Some("CantCarryMoreOfThis")
    );
    assert_eq!(holds(&realm, g, PACK).await, Some((OLD, 1)));

    give_at(&realm, g, SWORD, 1, MAIN).await;
    let shield = give_at(&realm, g, SHIELD, 1, OFF).await;
    assert_eq!(
        change(&mut a, &realm, g, MAIN, RING).await.as_deref(),
        Some("ItemDoesntGoToSlot")
    );
    assert_eq!(change(&mut a, &realm, g, MAIN, GREATSWORD).await, None);
    assert_eq!(holds(&realm, g, MAIN).await, Some((GREATSWORD, 1)));
    assert_eq!(
        field_at(&realm, g, MAIN, ITEM_FIELD_FLAGS)
            .await
            .map(|f| f & 1),
        Some(1)
    );
    assert_eq!(
        at(&realm, g, shield).await,
        Some((BAG_PLAYER, PACK_START + 1, 1))
    );
    let shown = on_map(&realm, move |m| {
        m.players[&g]
            .fields
            .get_u32(PLAYER_VISIBLE_ITEM_1_0 + u16::from(SLOT_MAIN_HAND) * VISIBLE_ITEM_SIZE)
    })
    .await;
    assert_eq!(shown, GREATSWORD);

    // The two-hander back to a sword, the shield on, the bags full.
    let (realm, mut a, g) = setup().await;
    give_at(&realm, g, SWORD, 1, MAIN).await;
    give_at(&realm, g, SHIELD, 1, OFF).await;
    fill_pack(&realm, g).await;
    assert_eq!(
        change(&mut a, &realm, g, MAIN, GREATSWORD).await.as_deref(),
        Some("CantEquipWithTwohanded")
    );
    assert_eq!(holds(&realm, g, MAIN).await, Some((SWORD, 1)));
    assert_eq!(holds(&realm, g, OFF).await, Some((SHIELD, 1)));
}

/// Thistle Tea's `ItemTransformation.prepare` and `ItemPropertyTest`: an
/// item changed keeps its permanent and live temporary enchantments, not
/// its random property (the new one rolls its own), and its wear in
/// proportion, rounded down, at least a point, the broken broken.
#[tokio::test]
async fn a_changed_item_keeps_enchantments_and_wear_and_rolls_its_own_property() {
    let (realm, mut a, g) = setup().await;
    let old = give_at(&realm, g, PROPERTIED, 1, PACK).await;
    on_map(&realm, move |m| {
        let d = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        let i = p.item_by_guid(old).unwrap();
        enchant::set_random_property(&d, &mut p.items[i], PROPERTY);
        enchant::set(
            &mut p.items[i],
            enchant::PERM_SLOT,
            Enchant {
                id: 41,
                ..Default::default()
            },
        );
        enchant::set(
            &mut p.items[i],
            enchant::TEMP_SLOT,
            Enchant {
                id: 263,
                duration_ms: 60_000,
                charges: 7,
            },
        );
    })
    .await;
    assert_eq!(change(&mut a, &realm, g, PACK, NEW_PROPERTY).await, None);
    let (property, enchants) = on_map(&realm, move |m| {
        let p = &m.players[&g];
        let it = &p.items[p.item_at(PACK.0, PACK.1).unwrap()];
        (
            it.fields.get_u32(ITEM_FIELD_RANDOM_PROPERTIES_ID),
            enchant::active(it)
                .map(|(s, e)| (s, e.id))
                .collect::<Vec<_>>(),
        )
    })
    .await;
    assert_eq!(property, ROLLED);
    assert_eq!(enchants, vec![(0, 41), (1, 263), (3, 72)]);

    // An expired temporary enchantment isn't carried over.
    let (realm, mut a, g) = setup().await;
    let old = give_at(&realm, g, PROPERTIED, 1, PACK).await;
    on_map(&realm, move |m| {
        crate::game::map::enchant::enchant_item(m, g, old, 41, None);
        crate::game::map::enchant::enchant_item(m, g, old, 323, Some((1000, 7)));
        m.now = std::time::Instant::now();
        m.loot_state.enchant_tick_at = Some(m.now - Duration::from_secs(2));
        enchant::update(m);
    })
    .await;
    assert_eq!(change(&mut a, &realm, g, PACK, FINAL).await, None);
    let enchants = on_map(&realm, move |m| {
        let p = &m.players[&g];
        let it = &p.items[p.item_at(PACK.0, PACK.1).unwrap()];
        enchant::active(it)
            .map(|(s, e)| (s, e.id))
            .collect::<Vec<_>>()
    })
    .await;
    assert_eq!(enchants, vec![(0, 41)]);

    // Wear: (old maximum, left, new maximum, left after).
    for (i, (from, left, to, after)) in [
        (6001, 0, 6002, 0),
        (6001, 100, 6002, 80),
        (6001, 99, 6003, 19),
        (6001, 67, 6003, 14),
        (6004, 0, 6003, 20),
        (6001, 50, 6004, 0),
    ]
    .into_iter()
    .enumerate()
    {
        let slot = (BAG_PLAYER, PACK_START + 2 + i as u8);
        let it = give_at(&realm, g, from, 1, slot).await;
        on_map(&realm, move |m| {
            let p = m.players.get_mut(&g).unwrap();
            let i = p.item_by_guid(it).unwrap();
            p.items[i].fields.set_u32(ITEM_FIELD_DURABILITY, left);
        })
        .await;
        assert_eq!(change(&mut a, &realm, g, slot, to).await, None);
        assert_eq!(
            field_at(&realm, g, slot, ITEM_FIELD_DURABILITY).await,
            Some(after),
            "{from} at {left} into {to}"
        );
    }
}

/// Thistle Tea's property bonuses: a random property's enchantments stay
/// as the enchantments put on change and run out.
#[tokio::test]
async fn a_random_property_stays_as_enchantments_change_and_run_out() {
    let (realm, _a, g) = setup().await;
    let it = give_at(&realm, g, PROPERTIED, 1, PACK).await;
    let (property, enchants) = on_map(&realm, move |m| {
        let d = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        let i = p.item_by_guid(it).unwrap();
        enchant::set_random_property(&d, &mut p.items[i], PROPERTY);
        enchant::enchant_item(m, g, it, 41, None);
        enchant::enchant_item(m, g, it, 263, Some((1000, 0)));
        enchant::enchant_item(m, g, it, 1883, None);
        m.now = std::time::Instant::now();
        m.loot_state.enchant_tick_at = Some(m.now - Duration::from_secs(2));
        enchant::update(m);
        let p = &m.players[&g];
        let item = &p.items[p.item_by_guid(it).unwrap()];
        (
            item.fields.get_u32(ITEM_FIELD_RANDOM_PROPERTIES_ID),
            enchant::active(item)
                .map(|(s, e)| (s, e.id))
                .collect::<Vec<_>>(),
        )
    })
    .await;
    assert_eq!(property, PROPERTY);
    assert_eq!(enchants, vec![(0, 1883), (3, 72), (4, 69)]);
}

/// Thistle Tea's `ItemLifetime`: an ordinary timed item waits while its
/// owner is away, a real-time one doesn't, and one whose time ran out
/// while away is gone at login, a bag with what is in it.
#[tokio::test]
async fn ordinary_timed_items_wait_offline_and_real_time_ones_run_out() {
    let (realm, _a, g) = setup().await;
    let timed = give_at(&realm, g, TIMED, 1, PACK).await;
    let real = give_at(&realm, g, REAL, 1, (BAG_PLAYER, BANK_START)).await;
    give_at(&realm, g, REAL_BAG, 1, BAG1).await;
    let inside = give_at(&realm, g, FILLER, 1, (BAG_START, 0)).await;
    let left = move |m: &Map, it: Guid| {
        let p = &m.players[&g];
        p.item_by_guid(it)
            .map(|i| p.items[i].fields.get_u32(ITEM_FIELD_DURATION))
    };
    let (after_100, after_400) = on_map(&realm, move |m| {
        let d = m.realm.data.clone();
        for it in [timed, real] {
            let p = m.players.get_mut(&g).unwrap();
            let i = p.item_by_guid(it).unwrap();
            p.items[i].fields.set_u32(ITEM_FIELD_DURATION, 250);
        }
        let p = m.players.get_mut(&g).unwrap();
        p.row.logout_time = crate::game::social::mail::now() - 100;
        crate::game::map::items::on_login(p, &d);
        let after_100 = (left(m, timed), left(m, real));
        let p = m.players.get_mut(&g).unwrap();
        p.row.logout_time = crate::game::social::mail::now() - 400;
        crate::game::map::items::on_login(p, &d);
        (after_100, (left(m, timed), left(m, real)))
    })
    .await;
    assert_eq!(after_100, (Some(250), Some(150)));
    assert_eq!(after_400, (Some(250), None), "the real-time one ran out");
    assert_eq!(holds(&realm, g, BAG1).await, None, "the real-time bag too");
    assert_eq!(at(&realm, g, inside).await, None, "with what was in it");
}

/// Thistle Tea's expired child of a surviving bag: a timed item in a timed
/// bag runs out first, and is gone from the bag, which runs on.
#[tokio::test]
async fn a_timed_item_runs_out_of_a_bag_that_runs_on() {
    let (realm, _a, g) = setup().await;
    let bag = give_at(&realm, g, TIMED_BAG, 1, BAG1).await;
    let short = give_at(&realm, g, SHORT, 1, (BAG_START, 0)).await;
    let (gone, slot, bag_left) = on_map(&realm, move |m| {
        m.now = std::time::Instant::now();
        m.loot_state.items_tick_at = Some(m.now - Duration::from_secs(2));
        crate::game::map::items::update(m);
        let p = &m.players[&g];
        let b = &p.items[p.item_by_guid(bag).unwrap()];
        (
            p.item_by_guid(short).is_none(),
            b.fields.get_guid(CONTAINER_FIELD_SLOT_1),
            b.fields.get_u32(ITEM_FIELD_DURATION),
        )
    })
    .await;
    assert!(gone);
    assert_eq!(slot, Guid::zero(), "gone from the bag");
    assert_eq!(bag_left, 298);
}

fn wrap(paper: (u8, u8), what: (u8, u8)) -> CMSG_WRAP_ITEM {
    CMSG_WRAP_ITEM {
        gift_bag_index: paper.0,
        gift_slot: paper.1,
        item_bag_index: what.0,
        item_slot: what.1,
    }
}

/// Thistle Tea's wrapped deadline and full bags: the last paper wraps an
/// item with the bags full and is gone; the gift keeps the item's time,
/// and runs out; opened, the item has its time still. Not while casting.
#[tokio::test]
async fn a_gift_keeps_its_items_time_and_the_last_paper_goes() {
    let (realm, mut a, g) = setup().await;
    give_at(&realm, g, PAPER, 1, PACK).await;
    let timed = give_at(&realm, g, TIMED_ONE, 1, PACK2).await;
    fill_pack(&realm, g).await;
    a.send(wrap(PACK, PACK2)).await;
    assert_eq!(refusal(&mut a).await, None);
    assert_eq!(holds(&realm, g, PACK).await, None, "the last paper gone");
    assert_eq!(holds(&realm, g, PACK2).await, Some((GIFT, 1)));
    let left = field_at(&realm, g, PACK2, ITEM_FIELD_DURATION)
        .await
        .unwrap();
    assert!((298..=300).contains(&left), "{left}");
    a.send(CMSG_OPEN_ITEM {
        bag_index: PACK2.0,
        slot: PACK2.1,
    })
    .await;
    handled(&mut a).await;
    assert_eq!(holds(&realm, g, PACK2).await, Some((TIMED_ONE, 1)));
    let opened = field_at(&realm, g, PACK2, ITEM_FIELD_DURATION)
        .await
        .unwrap();
    assert!(opened <= left && opened + 2 >= left, "{opened}");

    // Wrapped again and run out: the gift is gone.
    give_at(&realm, g, PAPER, 2, PACK).await;
    a.send(wrap(PACK, PACK2)).await;
    assert_eq!(refusal(&mut a).await, None);
    let gone = on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        let i = p.item_by_guid(timed).unwrap();
        p.items[i].fields.set_u32(ITEM_FIELD_DURATION, 1);
        m.now = std::time::Instant::now();
        m.loot_state.items_tick_at = Some(m.now - Duration::from_secs(2));
        crate::game::map::items::update(m);
        m.players[&g].item_by_guid(timed).is_none()
    })
    .await;
    assert!(gone, "the gift ran out");

    // Not while casting.
    let other = give_at(&realm, g, TIMED_ONE, 1, PACK2).await;
    on_map(&realm, move |m| {
        crate::game::map::spells::learn(m, g, SLOW_SPELL)
    })
    .await;
    a.send(CMSG_CAST_SPELL {
        spell: SLOW_SPELL,
        targets: crate::game::map::spells::wire_targets(&crate::game::map::spells::Targets {
            unit: Some(g),
            ..Default::default()
        }),
    })
    .await;
    handled(&mut a).await;
    let casting = on_map(&realm, move |m| m.players[&g].caster.casting.is_some()).await;
    assert!(casting);
    a.clear();
    a.send(wrap(PACK, PACK2)).await;
    assert_eq!(refusal(&mut a).await.as_deref(), Some("CantDoRightNow"));
    assert_eq!(at(&realm, g, other).await, Some((BAG_PLAYER, PACK2.1, 1)));
    assert_eq!(holds(&realm, g, PACK2).await, Some((TIMED_ONE, 1)));
}

/// Thistle Tea's opening checks: an empty slot, and an item that is no gift
/// nor holds anything, aren't opened, and stay.
#[tokio::test]
async fn what_isnt_a_gift_or_a_container_isnt_opened() {
    let (realm, mut a, g) = setup().await;
    give_at(&realm, g, FILLER, 1, PACK).await;
    for slot in [PACK2, PACK] {
        a.clear();
        a.send(CMSG_OPEN_ITEM {
            bag_index: slot.0,
            slot: slot.1,
        })
        .await;
        let why = refusal(&mut a).await;
        assert!(why.is_some(), "{slot:?}");
    }
    assert_eq!(holds(&realm, g, PACK).await, Some((FILLER, 1)));
}

/// Thistle Tea's rolled container: opened, with its loot left in it, a
/// stack of crates isn't split, traded, or added to: a new crate goes
/// beside it.
#[tokio::test]
async fn an_opened_crate_isnt_split_traded_or_stacked_onto() {
    let (realm, mut a, g) = setup().await;
    a.sees(vendor()).await;
    let crates = give_at(&realm, g, CRATE, 3, PACK).await;
    a.send(CMSG_OPEN_ITEM {
        bag_index: PACK.0,
        slot: PACK.1,
    })
    .await;
    a.wait_frame("SMSG_LOOT_RESPONSE", 0x160, |_| true).await;
    a.send(CMSG_LOOT_RELEASE { guid: crates }).await;
    a.clear();
    a.send(CMSG_SPLIT_ITEM {
        source_bag: PACK.0,
        source_slot: PACK.1,
        destination_bag: PACK2.0,
        destination_slot: PACK2.1,
        amount: 1,
    })
    .await;
    assert_eq!(refusal(&mut a).await.as_deref(), Some("ItemLocked"));
    assert_eq!(
        at(&realm, g, crates).await,
        Some((BAG_PLAYER, PACK_START, 3))
    );
    assert_eq!(buy_in(&mut a, CRATE, 1, g, u8::MAX).await, None);
    assert_eq!(
        at(&realm, g, crates).await,
        Some((BAG_PLAYER, PACK_START, 3))
    );
    assert_eq!(holds(&realm, g, PACK2).await, Some((CRATE, 1)), "beside it");

    // Offered in a trade: not taken; the unopened one is.
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    a.send(CMSG_INITIATE_TRADE { guid: gb }).await;
    b.wait_for("the trade asked", |m| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS(s) => {
            matches!(s.status, SMSG_TRADE_STATUS_TradeStatus::BeginTrade { .. }).then_some(())
        }
        _ => None,
    })
    .await;
    b.send(CMSG_BEGIN_TRADE {}).await;
    handled(&mut b).await;
    for (trade_slot, from) in [(0, PACK), (1, PACK2)] {
        a.send(CMSG_SET_TRADE_ITEM {
            trade_slot,
            bag: from.0,
            slot: from.1,
        })
        .await;
    }
    handled(&mut a).await;
    let offered = on_map(&realm, move |m| {
        m.players[&g]
            .trade
            .as_ref()
            .map(|t| (t.items[0], t.items[1].is_some()))
    })
    .await;
    assert_eq!(offered, Some((None, true)));
}

/// Thistle Tea's restricted bag: a bag bound to another map goes, with
/// what is in it, which was bound nowhere.
#[tokio::test]
async fn a_bag_bound_elsewhere_goes_with_what_is_in_it() {
    let (realm, _a, g) = setup().await;
    let bag = give_at(&realm, g, AV_BAG, 1, BAG1).await;
    let inside = give_at(&realm, g, FILLER, 1, (BAG_START, 0)).await;
    on_map(&realm, move |m| {
        crate::game::map::item_rules::limit_location(m, g)
    })
    .await;
    assert_eq!(at(&realm, g, bag).await, None);
    assert_eq!(at(&realm, g, inside).await, None);
    assert_eq!(holds(&realm, g, BAG1).await, None);
}

/// The items of [`an_item_is_used_only_by_whom_it_may_be_worn_by`], from
/// entry 7000, each with its refusal.
fn eligibility_cases() -> Vec<(ItemTemplate, Option<&'static str>)> {
    let base = used(0, USE_SPELL);
    let cases = vec![
        (base.clone(), None),
        (
            ItemTemplate {
                allowable_class: 2,
                ..base.clone()
            },
            Some("YouCanNeverUseThatItem"),
        ),
        (
            ItemTemplate {
                allowable_race: 2,
                ..base.clone()
            },
            Some("YouCanNeverUseThatItem"),
        ),
        (
            ItemTemplate {
                required_level: 21,
                ..base.clone()
            },
            Some("CantEquipLevelI"),
        ),
        (
            ItemTemplate {
                required_honor_rank: 6,
                ..base.clone()
            },
            None,
        ),
        (
            ItemTemplate {
                required_honor_rank: 7,
                ..base.clone()
            },
            Some("CantEquipRank"),
        ),
        (
            ItemTemplate {
                required_skill: 164,
                required_skill_rank: 100,
                ..base.clone()
            },
            None,
        ),
        (
            ItemTemplate {
                required_skill: 164,
                required_skill_rank: 101,
                ..base.clone()
            },
            Some("CantEquipSkill"),
        ),
        (
            ItemTemplate {
                required_skill: 165,
                ..base.clone()
            },
            Some("NoRequiredProficiency"),
        ),
        (
            ItemTemplate {
                required_spell: 9788,
                ..base.clone()
            },
            None,
        ),
        (
            ItemTemplate {
                required_spell: 9787,
                ..base.clone()
            },
            Some("NoRequiredProficiency"),
        ),
        (
            ItemTemplate {
                class: 2,
                subclass: 8,
                ..base.clone()
            },
            Some("NoRequiredProficiency"),
        ),
        (
            ItemTemplate {
                class: 4,
                subclass: 4,
                ..base.clone()
            },
            Some("NoRequiredProficiency"),
        ),
    ];
    cases
        .into_iter()
        .enumerate()
        .map(|(i, (t, why))| {
            (
                ItemTemplate {
                    entry: 7000 + i as u32,
                    ..t
                },
                why,
            )
        })
        .collect()
}

/// Thistle Tea's `ItemEligibility`: an item is used only by whom it may be
/// put on by (mangos's `CanUseItem`): its class and race, level, honor
/// rank (the highest held), skill and its rank, the spell it asks for, and
/// its weapon or armor proficiency.
#[tokio::test]
async fn an_item_is_used_only_by_whom_it_may_be_worn_by() {
    let mut d = data();
    let cases = eligibility_cases();
    for (t, _) in &cases {
        d.items.insert(t.entry, t.clone());
    }
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        p.items.clear();
        // Cloth alone, Armorsmithing's spell, Blacksmithing at 100, level
        // 20 and rank 6.
        p.spells.extend([CLOTH, 9788]);
        crate::game::map::skills::learn_skill(p, 164, 2);
        crate::game::map::skills::set_value(p, 164, 100);
        p.fields.set_u32(UNIT_FIELD_LEVEL, 20);
        p.fields.set_byte(PLAYER_FIELD_BYTES, 3, 6);
        let d = m.realm.data.clone();
        m.players.get_mut(&g).unwrap().refresh_inventory(&d);
    })
    .await;
    let skill = on_map(&realm, move |m| {
        crate::game::map::skills::value(&m.players[&g], 164)
    })
    .await;
    assert_eq!(skill, Some(100));
    for (t, want) in cases {
        let entry = t.entry;
        give_at(&realm, g, entry, 1, PACK).await;
        let why = use_item(&mut a, PACK).await;
        assert_eq!(why.as_deref(), want, "{entry}");
        // Used, its cooldown started.
        let cooling = on_map(&realm, move |m| {
            let p = m.players.get_mut(&g).unwrap();
            let cooling = p.caster.cooldowns.remove(&USE_SPELL).is_some();
            let i = p.item_at(PACK.0, PACK.1).unwrap();
            p.items.remove(i);
            cooling
        })
        .await;
        assert_eq!(cooling, want.is_none(), "{entry} used");
    }
}
