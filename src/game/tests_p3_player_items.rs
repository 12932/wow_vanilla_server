//! Phase 3 against Thistle Tea's player item tests
//! (`test/game/world/entity/player`): wear and repair, what broken gear
//! gives, the repair vendor's checks, and ammunition.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::items::ItemTemplate;
use crate::game::data::{CreatureSpawn, CreatureTemplate, GameData};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::derive::{SPELL_BLOCK, SPELL_DUAL_WIELD};
use crate::game::map::player::Item;
use crate::game::map::{Map, MapKey, derive};
use crate::game::store::BAG_PLAYER;

/// Northshire, where humans start.
const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

const SHIELD: u32 = 964_001;
const MAIN: u32 = 964_002;
const OFF: u32 = 964_003;
const BOOTS: u32 = 964_004;
const GLOVES: u32 = 964_005;
const ARMORER: u32 = 964_010;
const ARMORER_A: u32 = 96_410;
const PEDDLER: u32 = 964_011;
const PEDDLER_A: u32 = 96_411;

fn data() -> GameData {
    let mut d = GameData::empty();
    let item = |entry, class, subclass, inventory_type| ItemTemplate {
        entry,
        class,
        subclass,
        name: format!("Item {entry}"),
        display_id: 1000,
        inventory_type,
        item_level: 10,
        max_durability: 50,
        stackable: 1,
        ..Default::default()
    };
    let mut shield = item(SHIELD, 4, 6, 14);
    shield.armor = 300;
    shield.block = 15;
    d.items.insert(SHIELD, shield);
    for (entry, delay) in [(MAIN, 2000), (OFF, 1500)] {
        let mut w = item(entry, 2, 7, 13);
        w.delay = delay;
        w.damages[0] = (10.0, 20.0, 0);
        d.items.insert(entry, w);
    }
    d.items.insert(BOOTS, item(BOOTS, 4, 1, 8));
    d.items.insert(GLOVES, item(GLOVES, 4, 1, 10));
    // A point of cloth at level 10 costs ten copper; quality as is.
    let mut costs = vec![0u32; 29];
    costs[22] = 10;
    d.durability_costs.insert(10, costs);
    d.durability_quality.insert(2, 1.0);
    let npc = |entry, flags| CreatureTemplate {
        entry,
        name: format!("Npc {entry}"),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: 30,
        level_max: 30,
        faction: 35,
        npc_flags: flags,
        unit_class: 1,
        health_multiplier: 1.0,
        health_max: 500,
        ..Default::default()
    };
    d.creature_templates.insert(
        ARMORER,
        npc(ARMORER, crate::game::map::vendor::NPC_FLAG_REPAIR | 0x4),
    );
    d.creature_templates.insert(PEDDLER, npc(PEDDLER, 0x4));
    let spawn = |guid, entry, dx: f32| CreatureSpawn {
        guid,
        entries: vec![entry],
        map: 0,
        x: START.0 + dx,
        y: START.1,
        z: START.2,
        respawn_min: 600,
        respawn_max: 600,
        ..Default::default()
    };
    d.creature_spawns.insert(
        0,
        vec![
            spawn(ARMORER_A, ARMORER, 2.0),
            spawn(PEDDLER_A, PEDDLER, -2.0),
        ],
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

/// Wait until the map has handled what `c` sent so far.
async fn synced(c: &mut Client) {
    c.send(CMSG_PLAYED_TIME {}).await;
    c.wait_for("SMSG_PLAYED_TIME", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_PLAYED_TIME(_)).then_some(())
    })
    .await;
}

/// Equip a new item of `entry` in `slot`: its guid.
fn equip(m: &mut Map, g: Guid, entry: u32, slot: u8) -> Guid {
    let data = m.realm.data.clone();
    let counter = m
        .realm
        .next_item
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let item = guid::item(counter);
    let p = m.players.get_mut(&g).expect("the player");
    p.items.retain(|i| !(i.bag == BAG_PLAYER && i.slot == slot));
    p.items
        .push(Item::new(&data, item, g, entry, BAG_PLAYER, slot, 1));
    p.refresh_inventory(&data);
    crate::game::map::inventory::sync_gear(m, g);
    derive::unit(m, g);
    item
}

/// Wear the item in `slot` to `left`.
fn wear_to(m: &mut Map, g: Guid, slot: u8, left: u32) {
    let p = m.players.get_mut(&g).unwrap();
    let i = p.item_at(BAG_PLAYER, slot).unwrap();
    p.items[i].fields.set_u32(ITEM_FIELD_DURABILITY, left);
    crate::game::map::inventory::sync_gear(m, g);
    derive::unit(m, g);
}

fn durability(m: &Map, g: Guid, slot: u8) -> u32 {
    let p = &m.players[&g];
    let i = p.item_at(BAG_PLAYER, slot).unwrap();
    p.items[i].fields.get_u32(ITEM_FIELD_DURABILITY)
}

#[tokio::test]
async fn a_broken_shield_gives_no_armor_or_block_and_a_broken_off_hand_no_second_weapon_till_mended()
 {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let got = on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        p.spells.push(SPELL_BLOCK);
        p.spells.push(SPELL_DUAL_WIELD);
        let shield = |m: &Map| {
            let f = &m.players[&g].fields;
            (
                f.get_u32(UNIT_FIELD_RESISTANCES),
                f.get_f32(PLAYER_BLOCK_PERCENTAGE),
            )
        };
        let bare = shield(m);
        equip(m, g, SHIELD, 16);
        let whole = shield(m);
        wear_to(m, g, 16, 0);
        let broken = shield(m);
        wear_to(m, g, 16, 50);
        let mended = shield(m);
        // A second weapon: its swing and damage, gone broken.
        equip(m, g, MAIN, 15);
        equip(m, g, OFF, 16);
        let off = |m: &Map| {
            let f = &m.players[&g].fields;
            (
                f.get_u32(UNIT_FIELD_BASEATTACKTIME + 1),
                f.get_f32(UNIT_FIELD_MAXOFFHANDDAMAGE) > 0.0,
            )
        };
        let wielding = off(m);
        wear_to(m, g, 16, 0);
        let broken_off = off(m);
        wear_to(m, g, 16, 50);
        let mended_off = off(m);
        (
            (bare, whole, broken, mended),
            (wielding, broken_off, mended_off),
        )
    })
    .await;
    let ((bare, whole, broken, mended), (wielding, broken_off, mended_off)) = got;
    assert_eq!((whole.0 - bare.0, whole.1), (300, 5.0));
    assert_eq!(broken, (bare.0, 0.0));
    assert_eq!(mended, whole);
    assert_eq!(wielding, (1500, true));
    assert_eq!(broken_off, (2000, false));
    assert_eq!(mended_off, wielding);
}

#[tokio::test]
async fn a_repair_mends_the_one_item_asked_from_a_living_armorer_near_and_none_for_the_dead() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let armorer = guid::unit(ARMORER, ARMORER_A);
    let peddler = guid::unit(PEDDLER, PEDDLER_A);
    a.sees(armorer).await;
    a.sees(peddler).await;
    let (boots, _gloves) = on_map(&realm, move |m| {
        let boots = equip(m, g, BOOTS, 7);
        let gloves = equip(m, g, GLOVES, 9);
        wear_to(m, g, 7, 40);
        wear_to(m, g, 9, 40);
        m.players.get_mut(&g).unwrap().set_money(1_000);
        (boots, gloves)
    })
    .await;
    let state = async |realm: &Arc<Realm>| {
        on_map(realm, move |m| {
            (
                durability(m, g, 7),
                durability(m, g, 9),
                m.players[&g].money(),
            )
        })
        .await
    };
    let repair = |npc: Guid, item: Guid| CMSG_REPAIR_ITEM { npc, item };
    // Not a repairer: nothing.
    a.send(repair(peddler, boots)).await;
    synced(&mut a).await;
    assert_eq!(state(&realm).await, (40, 40, 1_000));
    // The boots alone, for their ten points.
    a.send(repair(armorer, boots)).await;
    synced(&mut a).await;
    assert_eq!(state(&realm).await, (50, 40, 900));
    // Too far: nothing.
    a.heartbeat(START.0 + 40.0, START.1, START.2).await;
    a.send(repair(armorer, Guid::zero())).await;
    synced(&mut a).await;
    assert_eq!(state(&realm).await, (50, 40, 900));
    a.heartbeat(START.0, START.1, START.2).await;
    // Back in reach first, as a dead body takes no move; then dead, nothing
    // either.
    synced(&mut a).await;
    on_map(&realm, move |m| crate::game::map::death::kill(m, g)).await;
    a.send(repair(armorer, Guid::zero())).await;
    synced(&mut a).await;
    assert_eq!(state(&realm).await, (50, 40, 900));
    // Alive, everything at once.
    on_map(&realm, move |m| {
        crate::game::map::death::resurrect(m, g, 1.0)
    })
    .await;
    a.send(repair(armorer, Guid::zero())).await;
    synced(&mut a).await;
    assert_eq!(state(&realm).await, (50, 50, 800));
}

const RING: u32 = 964_020;
const STONES: u32 = 964_021;
const TRINKET: u32 = 964_022;

fn with_goods(mut d: GameData) -> GameData {
    let good = |entry, stackable, max_count| ItemTemplate {
        entry,
        class: 15,
        name: format!("Good {entry}"),
        display_id: 1000,
        stackable,
        max_count,
        ..Default::default()
    };
    d.items.insert(RING, good(RING, 1, 0));
    d.items.insert(STONES, good(STONES, 20, 0));
    d.items.insert(TRINKET, good(TRINKET, 1, 1));
    d
}

/// How many of `entry` `g` holds, and in how many pieces.
fn holding(m: &Map, g: Guid, entry: u32) -> (u32, usize) {
    let p = &m.players[&g];
    let pieces: Vec<u32> = p
        .items
        .iter()
        .filter(|i| i.entry == entry)
        .map(|i| i.count())
        .collect();
    (pieces.iter().sum(), pieces.len())
}

#[tokio::test]
async fn a_grant_goes_one_a_piece_or_in_legal_stacks_whole_or_not_at_all_and_a_unique_one_in_the_bank_counts()
 {
    use crate::game::map::inventory::{Fail, add_item};
    let realm = harness::realm_with(with_goods(data()));
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let got = on_map(&realm, move |m| {
        // Three rings, each its own; forty-five stones, in twenties.
        let rings = (add_item(m, g, RING, 3, false), holding(m, g, RING));
        let stones = (add_item(m, g, STONES, 45, false), holding(m, g, STONES));
        // The backpack filled but for a slot: twenty-one stones more want
        // two, and none are given.
        let free = m.players[&g]
            .items
            .iter()
            .filter(|i| i.bag == BAG_PLAYER && (23..39).contains(&i.slot))
            .count();
        let fill = 16 - free - 1;
        if fill > 0 {
            add_item(m, g, RING, fill as u32, false).unwrap();
        }
        let before = holding(m, g, STONES);
        let too_many = add_item(m, g, STONES, 41, false);
        let after = holding(m, g, STONES);
        // A unique trinket in the bank: no second one.
        let data = m.realm.data.clone();
        let counter = m
            .realm
            .next_item
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p = m.players.get_mut(&g).unwrap();
        p.items.push(Item::new(
            &data,
            guid::item(counter),
            g,
            TRINKET,
            BAG_PLAYER,
            crate::game::map::inventory::BANK_START,
            1,
        ));
        let unique = (add_item(m, g, TRINKET, 1, false), holding(m, g, TRINKET));
        (rings, stones, (before, too_many, after), unique)
    })
    .await;
    let (rings, stones, (before, too_many, after), unique) = got;
    assert_eq!(rings, (Ok(()), (3, 3)));
    assert_eq!(stones, (Ok(()), (45, 3)));
    assert!(matches!(too_many, Err(Fail::InventoryFull)), "{too_many:?}");
    assert_eq!(before, after);
    assert!(matches!(unique.0, Err(Fail::CantCarryMore)), "{unique:?}");
    assert_eq!(unique.1, (1, 1));
}
