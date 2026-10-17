//! Group loot, professions and enchantments through the protocol.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::crafts::{ItemSet, LockKey};
use crate::game::data::spells::{Enchantment, SkillAbility, SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::spells::Targets;
use crate::game::map::{Map, MapKey};
use crate::game::store::BAG_PLAYER;

/// Northshire, where humans start.
const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

const LOOT_ITEM: u32 = 2000;
const CRAFTED: u32 = 2001;
const ORE: u32 = 2770;
const FISH: u32 = 6291;
const MINING_PICK: u32 = 2901;
const BRACER: u32 = 2010;
const SET_HELM: u32 = 2020;
const SET_CHEST: u32 = 2021;

const RECIPE: u32 = 900_001;
const MINING: u32 = 2575;
const FISHING: u32 = 7620;
const ENCHANT_STRENGTH: u32 = 900_002;
const STRENGTH_AURA: u32 = 7507;
const POISON: u32 = 900_003;
const POISON_HIT: u32 = 900_004;
const SET_BONUS: u32 = 900_005;

const SKINNING: u32 = 8613;
const LEATHER: u32 = 2318;
const VEIN: u32 = 1731;
const BOBBER: u32 = 35591;

fn item(
    entry: u32,
    name: &str,
    class: u32,
    subclass: u32,
    inventory_type: u32,
    quality: u32,
) -> ItemTemplate {
    ItemTemplate {
        entry,
        class,
        subclass,
        name: name.into(),
        quality,
        inventory_type,
        stackable: 20,
        ..Default::default()
    }
}

fn spell(id: u32, effects: &[(u32, i32, i32, u32, u32)]) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        range_index: 1,
        equipped_item_class: -1,
        ..Default::default()
    };
    for (i, (effect, misc, base, target, aura)) in effects.iter().enumerate() {
        s.effects[i] = SpellEffect {
            effect: *effect,
            misc_value: *misc,
            base_points: *base,
            target_a: *target,
            aura: *aura,
            ..Default::default()
        };
    }
    s
}

fn data() -> GameData {
    let mut d = GameData::empty();
    for t in [
        item(LOOT_ITEM, "Green Blade", 2, 7, 13, 2),
        item(CRAFTED, "Copper Chain Belt", 4, 3, 6, 1),
        item(ORE, "Copper Ore", 7, 0, 0, 1),
        item(FISH, "Raw Brilliant Smallfish", 0, 0, 0, 1),
        item(MINING_PICK, "Mining Pick", 2, 14, 0, 1),
        item(BRACER, "Linen Bracers", 4, 1, 9, 1),
        item(LEATHER, "Light Leather", 7, 0, 0, 1),
        item(7005, "Skinning Knife", 2, 14, 0, 1),
    ] {
        d.items.insert(t.entry, t);
    }
    for (entry, slot) in [(SET_HELM, 1), (SET_CHEST, 5)] {
        let mut t = item(entry, "Set Piece", 4, 1, slot, 2);
        t.set_id = 1;
        d.items.insert(entry, t);
    }
    d.crafts.item_sets.insert(
        1,
        ItemSet {
            id: 1,
            items: vec![SET_HELM, SET_CHEST],
            spells: vec![(SET_BONUS, 2)],
            ..Default::default()
        },
    );
    // A creature that drops the green blade, and two of them near.
    d.creature_templates.insert(
        3001,
        CreatureTemplate {
            entry: 3001,
            name: "Kobold".into(),
            display_ids: [903, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            level_min: 1,
            level_max: 1,
            faction: 32,
            unit_class: 1,
            health_max: 200,
            loot_id: 3001,
            gold_min: 30,
            gold_max: 30,
            ..Default::default()
        },
    );
    d.loot.creature.insert(
        3001,
        vec![LootEntry {
            item: LOOT_ITEM,
            chance: 100.0,
            min: 1,
            max: 1,
            ..Default::default()
        }],
    );
    let spawn = |guid, dx: f32| CreatureSpawn {
        guid,
        entries: vec![3001],
        map: 0,
        x: START.0 + dx,
        y: START.1,
        z: START.2,
        respawn_min: 300,
        respawn_max: 300,
        ..Default::default()
    };
    d.creature_templates.insert(
        3002,
        CreatureTemplate {
            entry: 3002,
            name: "Boar".into(),
            display_ids: [903, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            level_min: 1,
            level_max: 1,
            faction: 32,
            unit_class: 1,
            health_max: 50,
            skinning_loot_id: 3002,
            ..Default::default()
        },
    );
    d.loot.skinning.insert(
        3002,
        vec![LootEntry {
            item: LEATHER,
            chance: 100.0,
            min: 1,
            max: 1,
            ..Default::default()
        }],
    );
    let mut boar = spawn(24, -1.5);
    boar.entries = vec![3002];
    d.creature_spawns.insert(
        0,
        vec![spawn(21, 2.0), spawn(22, 2.5), spawn(23, 3.0), boar],
    );
    // A copper vein two yards off.
    let mut vein = GameObjectTemplate {
        entry: VEIN,
        kind: 3,
        display_id: 310,
        name: "Copper Vein".into(),
        size: 1.0,
        ..Default::default()
    };
    vein.data[0] = 38;
    vein.data[1] = 1502;
    d.gameobject_templates.insert(VEIN, vein);
    d.gameobject_spawns.insert(
        0,
        vec![GameObjectSpawn {
            guid: 50,
            entry: VEIN,
            map: 0,
            x: START.0 - 2.0,
            y: START.1,
            z: START.2,
            state: 1,
            respawn_min: 60,
            ..Default::default()
        }],
    );
    d.crafts.locks.insert(
        38,
        vec![LockKey {
            kind: 2,
            index: 3,
            skill: 0,
        }],
    );
    d.loot.gameobject.insert(
        1502,
        vec![LootEntry {
            item: ORE,
            chance: 100.0,
            min: 1,
            max: 1,
            ..Default::default()
        }],
    );
    // The bobber, and the fish anywhere.
    d.gameobject_templates.insert(
        BOBBER,
        GameObjectTemplate {
            entry: BOBBER,
            kind: 17,
            display_id: 668,
            name: "Fishing Bobber".into(),
            size: 1.0,
            ..Default::default()
        },
    );
    d.loot.fishing.insert(
        0,
        vec![LootEntry {
            item: FISH,
            chance: 100.0,
            min: 1,
            max: 1,
            ..Default::default()
        }],
    );
    d.crafts.fishing_skill.insert(0, -100);
    // Spells.
    let mut recipe = spell(RECIPE, &[(24, 0, 0, 1, 0)]);
    recipe.effects[0].item_type = CRAFTED;
    let mut mining = spell(MINING, &[(33, 3, -1, 23, 0)]);
    mining.totems = [MINING_PICK, 0];
    let mut fishing = spell(FISHING, &[(50, BOBBER as i32, -1, 39, 0)]);
    fishing.attributes[1] = 0x4;
    let enchant = spell(ENCHANT_STRENGTH, &[(53, 107, -1, 0, 0)]);
    let strength = spell(STRENGTH_AURA, &[(6, 0, 4, 1, 29)]);
    let poison = spell(POISON, &[(54, 900, 1799, 0, 0)]);
    let poison_hit = spell(POISON_HIT, &[(2, 0, 9, 6, 0)]);
    let set_bonus = spell(SET_BONUS, &[(6, 2, 9, 1, 29)]);
    let mut skinning = spell(SKINNING, &[(95, 0, 0, 25, 0)]);
    skinning.totems = [7005, 0];
    for s in [
        recipe, mining, fishing, enchant, strength, poison, poison_hit, set_bonus, skinning,
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d.spells.skill_abilities.insert(
        RECIPE,
        vec![SkillAbility {
            skill: 164,
            spell: RECIPE,
            trivial_low: 5,
            trivial_high: 10,
            ..Default::default()
        }],
    );
    d.spells.enchantments.insert(
        107,
        Enchantment {
            id: 107,
            effects: [(3, 0, 0, STRENGTH_AURA), (0, 0, 0, 0), (0, 0, 0, 0)],
            name: "+5 Strength".into(),
        },
    );
    d.spells.enchantments.insert(
        900,
        Enchantment {
            id: 900,
            effects: [(1, 100, 100, POISON_HIT), (0, 0, 0, 0), (0, 0, 0, 0)],
            name: "Instant Poison".into(),
        },
    );
    d.crafts.enchant_charges.insert(POISON, 2);
    d
}

/// Alice, Bob and Cid at the start, in Alice's group.
async fn group_of_three(realm: &std::sync::Arc<crate::game::Realm>) -> (Client, Client, Client) {
    let mut a = Client::connect(realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    let mut c = Client::connect(realm, "C");
    c.enter("Cid", Race::Human, Class::Mage).await;
    for (name, who) in [("Bob", &mut b), ("Cid", &mut c)] {
        a.send(CMSG_GROUP_INVITE { name: name.into() }).await;
        who.wait_for("SMSG_GROUP_INVITE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
        })
        .await;
        who.send(CMSG_GROUP_ACCEPT {}).await;
        who.wait_for("SMSG_GROUP_LIST", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_LIST(l) => (!l.members.is_empty()).then_some(()),
            _ => None,
        })
        .await;
    }
    (a, b, c)
}

fn count_of(map: &Map, g: Guid, entry: u32) -> u32 {
    map.players.get(&g).map_or(0, |p| p.count_of(entry))
}

async fn has(map: &crate::game::map::MapHandle, g: Guid, entry: u32) -> u32 {
    map.query(move |m| count_of(m, g, entry)).await.unwrap_or(0)
}

#[tokio::test]
async fn a_group_rolls_need_and_greed_and_the_need_wins() {
    let realm = harness::realm_with(data());
    let (mut a, mut b, mut c) = group_of_three(&realm).await;
    let (ga, gb) = (a.guid, b.guid);
    let kobold = guid::unit(3001, 21);
    let map = realm.maps.get(&realm, MapKey::world(0));
    map.query(move |m| crate::game::map::combat::kill(m, ga, kobold))
        .await;
    for who in [&mut a, &mut b, &mut c] {
        who.wait_for("SMSG_LOOT_START_ROLL", |m| match m {
            ServerOpcodeMessage::SMSG_LOOT_START_ROLL(r) if r.item == LOOT_ITEM => Some(()),
            _ => None,
        })
        .await;
    }
    // Nobody may take it from the window while it is rolled for.
    let held = map
        .query(move |m| {
            crate::game::map::loot::loot_ref(m, kobold)
                .is_some_and(|l| l.items[0].blocked && l.rolls.len() == 1)
        })
        .await;
    assert_eq!(held, Some(true));
    for (who, vote) in [
        (&a, RollVote::Greed),
        (&b, RollVote::Need),
        (&c, RollVote::Pass),
    ] {
        who.send(CMSG_LOOT_ROLL {
            item: kobold,
            item_slot: 0,
            vote,
        })
        .await;
    }
    let winner = a
        .wait_for("SMSG_LOOT_ROLL_WON", |m| match m {
            ServerOpcodeMessage::SMSG_LOOT_ROLL_WON(w) => Some(w.winning_player),
            _ => None,
        })
        .await;
    assert_eq!(winner, gb, "need beats greed");
    b.wait_for("SMSG_ITEM_PUSH_RESULT", |m| match m {
        ServerOpcodeMessage::SMSG_ITEM_PUSH_RESULT(r) if r.item == LOOT_ITEM => Some(()),
        _ => None,
    })
    .await;
    assert_eq!(has(&map, gb, LOOT_ITEM).await, 1);
    assert_eq!(has(&map, ga, LOOT_ITEM).await, 0);
    // The gold is split three ways.
    a.send(CMSG_LOOT { guid: kobold }).await;
    tokio::time::sleep(Duration::from_millis(60)).await;
    a.send(CMSG_LOOT_MONEY {}).await;
    a.wait_for("SMSG_LOOT_CLEAR_MONEY", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOOT_CLEAR_MONEY).then_some(())
    })
    .await;
    b.wait_for("SMSG_LOOT_MONEY_NOTIFY", |m| match m {
        ServerOpcodeMessage::SMSG_LOOT_MONEY_NOTIFY(n) => Some(n.amount),
        _ => None,
    })
    .await;
    let money = map
        .query(move |m| [ga, gb].map(|g| m.players.get(&g).map_or(0, |p| p.money())))
        .await
        .unwrap();
    assert_eq!(money, [10, 10]);
}

#[tokio::test]
async fn the_master_looter_gives_a_kept_item() {
    let realm = harness::realm_with(data());
    let (mut a, mut b, _c) = group_of_three(&realm).await;
    let (ga, gb) = (a.guid, b.guid);
    a.send(CMSG_LOOT_METHOD {
        loot_setting: GroupLootSetting::MasterLoot,
        loot_master: ga,
        loot_threshold: ItemQuality::Uncommon,
    })
    .await;
    a.wait_for("the master loot list", |m| match m {
        ServerOpcodeMessage::SMSG_GROUP_LIST(l) => l
            .group_not_empty
            .as_ref()
            .filter(|g| g.loot_setting == GroupLootSetting::MasterLoot)
            .map(|_| ()),
        _ => None,
    })
    .await;
    let kobold = guid::unit(3001, 22);
    let map = realm.maps.get(&realm, MapKey::world(0));
    map.query(move |m| crate::game::map::combat::kill(m, ga, kobold))
        .await;
    // Bob can't take it from the window: it's the master's to give.
    b.send(CMSG_LOOT { guid: kobold }).await;
    tokio::time::sleep(Duration::from_millis(60)).await;
    b.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(has(&map, gb, LOOT_ITEM).await, 0);
    a.send(CMSG_LOOT { guid: kobold }).await;
    a.wait_for("SMSG_LOOT_MASTER_LIST", |m| match m {
        ServerOpcodeMessage::SMSG_LOOT_MASTER_LIST(l) => (l.guids.len() == 3).then_some(()),
        _ => None,
    })
    .await;
    a.send(CMSG_LOOT_MASTER_GIVE {
        loot: kobold,
        slot_id: 0,
        player: gb,
    })
    .await;
    b.wait_for("SMSG_ITEM_PUSH_RESULT", |m| match m {
        ServerOpcodeMessage::SMSG_ITEM_PUSH_RESULT(r) if r.item == LOOT_ITEM => Some(()),
        _ => None,
    })
    .await;
    assert_eq!(has(&map, gb, LOOT_ITEM).await, 1);
}

#[tokio::test]
async fn round_robin_hands_the_corpses_out_in_turn() {
    let realm = harness::realm_with(data());
    let (a, b, c) = group_of_three(&realm).await;
    let (ga, gb, gc) = (a.guid, b.guid, c.guid);
    a.send(CMSG_LOOT_METHOD {
        loot_setting: GroupLootSetting::RoundRobin,
        loot_master: Guid::zero(),
        loot_threshold: ItemQuality::Uncommon,
    })
    .await;
    tokio::time::sleep(Duration::from_millis(80)).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let looters = map
        .query(move |m| {
            [21u32, 22, 23].map(|n| {
                let k = guid::unit(3001, n);
                crate::game::map::combat::kill(m, ga, k);
                crate::game::map::loot::loot_ref(m, k).and_then(|l| l.assigned)
            })
        })
        .await
        .unwrap();
    // Each kill goes to the next member, round the group.
    assert_eq!(looters, [Some(ga), Some(gb), Some(gc)]);
    // Only the assigned looter may open a corpse.
    let may = map
        .query(move |m| {
            let l = crate::game::map::loot::loot_ref(m, guid::unit(3001, 22)).unwrap();
            (l.may_loot(ga), l.may_loot(gb))
        })
        .await
        .unwrap();
    assert_eq!(may, (false, true));
}

/// One player at the start, and the map.
async fn alone(
    name: &str,
    class: Class,
) -> (
    std::sync::Arc<crate::game::Realm>,
    Client,
    crate::game::map::MapHandle,
) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, name);
    a.enter(name, Race::Human, class).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    (realm, a, map)
}

fn learn(m: &mut Map, g: Guid, spell: u32, skill: u32) {
    if let Some(p) = m.players.get_mut(&g) {
        p.spells.push(spell);
        if skill != 0 {
            crate::game::map::skills::learn_skill(p, skill, 1);
        }
    }
}

fn skill_of(m: &Map, g: Guid, skill: u32) -> u32 {
    m.players
        .get(&g)
        .and_then(|p| crate::game::map::skills::value(p, skill))
        .unwrap_or(0)
}

#[tokio::test]
async fn crafting_raises_the_skill_up_to_the_trainers_cap() {
    let (_realm, a, map) = alone("Alice", Class::Warrior).await;
    let g = a.guid;
    let (before, after) = map
        .query(move |m| {
            learn(m, g, RECIPE, 164);
            let before = skill_of(m, g, 164);
            crate::game::map::spells::cast(m, g, RECIPE, Targets::default(), None);
            (before, skill_of(m, g, 164))
        })
        .await
        .unwrap();
    assert_eq!((before, after), (1, 2), "an orange recipe always raises");
    assert_eq!(has(&map, g, CRAFTED).await, 1);
    // At the apprentice's cap it rises no more.
    let capped = map
        .query(move |m| {
            if let Some(p) = m.players.get_mut(&g) {
                crate::game::map::skills::learn_skill(p, 164, 1);
                while crate::game::map::skills::raise(p, 164, 1) {}
            }
            let at = skill_of(m, g, 164);
            m.now = Instant::now();
            if let Some(p) = m.players.get_mut(&g) {
                p.caster.gcd_until = None;
            }
            crate::game::map::spells::cast(m, g, RECIPE, Targets::default(), None);
            (at, skill_of(m, g, 164))
        })
        .await
        .unwrap();
    assert_eq!(capped, (75, 75));
}

#[tokio::test]
async fn a_miner_gathers_a_vein_and_learns_from_it() {
    let (_realm, mut a, map) = alone("Alice", Class::Warrior).await;
    let g = a.guid;
    let vein = guid::game_object(VEIN, 50);
    a.sees(vein).await;
    map.query(move |m| {
        learn(m, g, MINING, 186);
        let _ = crate::game::map::inventory::add_item(m, g, MINING_PICK, 1, false);
    })
    .await;
    // A low skill sometimes fails; it gets there.
    let mut opened = false;
    for _ in 0..40 {
        opened = map
            .query(move |m| {
                m.now = Instant::now();
                // Each try is a fresh cast: no cooldown or cast left over.
                if let Some(p) = m.players.get_mut(&g) {
                    p.caster.gcd_until = None;
                    p.caster.casting = None;
                }
                crate::game::map::spells::cast(
                    m,
                    g,
                    MINING,
                    Targets {
                        object: Some(vein),
                        ..Default::default()
                    },
                    None,
                );
                m.players.get(&g).is_some_and(|p| p.looting == Some(vein))
            })
            .await
            .unwrap_or(false);
        if opened {
            break;
        }
    }
    assert!(opened, "the vein opens");
    assert_eq!(
        map.query(move |m| skill_of(m, g, 186)).await,
        Some(2),
        "a first vein always teaches"
    );
    a.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
    a.wait_for("SMSG_ITEM_PUSH_RESULT", |m| match m {
        ServerOpcodeMessage::SMSG_ITEM_PUSH_RESULT(r) if r.item == ORE => Some(()),
        _ => None,
    })
    .await;
    a.send(CMSG_LOOT_RELEASE { guid: vein }).await;
    // Mined out, the vein goes until it respawns.
    a.forgets(vein).await;
    let waiting = map.query(|m| m.loot_state.respawns.len()).await.unwrap();
    assert_eq!(waiting, 1);
}

#[tokio::test]
async fn a_fisher_lands_a_fish() {
    let (_realm, mut a, map) = alone("Alice", Class::Warrior).await;
    let g = a.guid;
    let bobber = map
        .query(move |m| {
            learn(m, g, FISHING, 356);
            crate::game::map::spells::cast(m, g, FISHING, Targets::default(), None);
            m.loot_state.bobbers.first().map(|b| b.guid)
        })
        .await
        .flatten()
        .expect("a bobber in the water");
    // Too soon: nothing's on the line.
    a.send(CMSG_GAMEOBJ_USE { guid: bobber }).await;
    a.wait_for("SMSG_FISH_NOT_HOOKED", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_FISH_NOT_HOOKED).then_some(())
    })
    .await;
    let bobber = map
        .query(move |m| {
            m.now = Instant::now();
            crate::game::map::spells::cast(m, g, FISHING, Targets::default(), None);
            for b in m.loot_state.bobbers.iter_mut() {
                b.bite_at = Instant::now();
            }
            m.loot_state.bobbers.first().map(|b| b.guid)
        })
        .await
        .flatten()
        .unwrap();
    a.wait_for("the bite", |m| match m {
        ServerOpcodeMessage::SMSG_GAMEOBJECT_CUSTOM_ANIM(c) if c.guid == bobber => Some(()),
        _ => None,
    })
    .await;
    a.send(CMSG_GAMEOBJ_USE { guid: bobber }).await;
    tokio::time::sleep(Duration::from_millis(60)).await;
    a.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
    a.wait_for("SMSG_ITEM_PUSH_RESULT", |m| match m {
        ServerOpcodeMessage::SMSG_ITEM_PUSH_RESULT(r) if r.item == FISH => Some(()),
        _ => None,
    })
    .await;
    assert_eq!(map.query(move |m| skill_of(m, g, 356)).await, Some(2));
}

/// Equip a new item of `entry` in `slot`; its guid.
fn equip(m: &mut Map, g: Guid, entry: u32, slot: u8) -> Guid {
    let data = m.realm.data.clone();
    let counter = m
        .realm
        .next_item
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let guid = guid::item(counter);
    let p = m.players.get_mut(&g).expect("the player");
    p.items.retain(|i| !(i.bag == BAG_PLAYER && i.slot == slot));
    p.items.push(crate::game::map::player::Item::new(
        &data, guid, g, entry, BAG_PLAYER, slot, 1,
    ));
    p.refresh_inventory(&data);
    guid
}

fn stat(m: &Map, g: Guid, i: u16) -> u32 {
    m.players
        .get(&g)
        .map_or(0, |p| p.fields.get_u32(UNIT_FIELD_STAT0 + i))
}

#[tokio::test]
async fn an_enchantment_raises_strength_and_survives_the_store() {
    let (_realm, _a, map) = alone("Alice", Class::Warrior).await;
    let g = _a.guid;
    let bracer = map.query(move |m| equip(m, g, BRACER, 8)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;
    let before = map.query(move |m| stat(m, g, 0)).await.unwrap();
    map.query(move |m| {
        learn(m, g, ENCHANT_STRENGTH, 0);
        crate::game::map::spells::cast(
            m,
            g,
            ENCHANT_STRENGTH,
            Targets {
                item: Some(bracer),
                ..Default::default()
            },
            None,
        );
    })
    .await;
    tokio::time::sleep(Duration::from_millis(80)).await;
    let (after, data) = map
        .query(move |m| {
            let p = &m.players[&g];
            let i = p.items.iter().find(|i| i.guid == bracer).unwrap();
            (stat(m, g, 0), i.to_row().data)
        })
        .await
        .unwrap();
    assert_eq!(after, before + 5);
    assert_eq!(data, "e0:107/0/0");
}

#[tokio::test]
async fn a_poison_procs_spends_its_charges_and_runs_out() {
    let (_realm, a, map) = alone("Alice", Class::Rogue).await;
    let g = a.guid;
    let kobold = guid::unit(3001, 21);
    let health = map
        .query(move |m| {
            let blade = equip(m, g, LOOT_ITEM, 15);
            learn(m, g, POISON, 0);
            crate::game::map::spells::cast(
                m,
                g,
                POISON,
                Targets {
                    item: Some(blade),
                    ..Default::default()
                },
                None,
            );
            let hit = crate::game::map::procs::Event {
                actor: g,
                target: kobold,
                actor_flags: crate::game::map::procs::PROC_DONE_MELEE,
                ex: crate::game::map::procs::EX_NORMAL_HIT,
                ..Default::default()
            };
            let hp = |m: &Map| {
                m.creature(kobold)
                    .map_or(0, |c| c.fields.get_u32(UNIT_FIELD_HEALTH))
            };
            let charges = |m: &Map| {
                let p = &m.players[&g];
                let i = p.items.iter().find(|i| i.guid == blade).unwrap();
                crate::game::map::enchant::get(i, crate::game::map::enchant::TEMP_SLOT)
            };
            let mut seen = vec![(hp(m), charges(m))];
            for _ in 0..3 {
                crate::game::map::procs::trigger(m, hit);
                seen.push((hp(m), charges(m)));
            }
            // A fresh dose, whose time runs out.
            crate::game::map::spells::cast(
                m,
                g,
                POISON,
                Targets {
                    item: Some(blade),
                    ..Default::default()
                },
                None,
            );
            let p = m.players.get_mut(&g).unwrap();
            let i = p.items.iter_mut().find(|i| i.guid == blade).unwrap();
            let mut e = crate::game::map::enchant::get(i, crate::game::map::enchant::TEMP_SLOT);
            e.duration_ms = 500;
            crate::game::map::enchant::set(i, crate::game::map::enchant::TEMP_SLOT, e);
            m.now = Instant::now();
            m.loot_state.enchant_tick_at = Some(m.now - Duration::from_secs(2));
            crate::game::map::enchant::update(m);
            seen.push((hp(m), charges(m)));
            seen
        })
        .await
        .unwrap();
    let (hp0, e0) = health[0];
    assert_eq!(e0.id, 900);
    assert_eq!(e0.charges, 2);
    assert_eq!(e0.duration_ms, 1_800_000);
    assert_eq!(health[1].0, hp0 - 10);
    assert_eq!(health[1].1.charges, 1);
    assert_eq!(health[2].0, hp0 - 20);
    assert_eq!(
        health[2].1.id, 0,
        "the last charge spent, the poison is gone"
    );
    assert_eq!(health[3].0, hp0 - 20, "no poison, no proc");
    assert_eq!(health[4].1.id, 0, "its time run out, the poison is gone");
}

#[tokio::test]
async fn a_set_bonus_comes_with_its_second_piece() {
    let (_realm, a, map) = alone("Alice", Class::Warrior).await;
    let g = a.guid;
    map.query(move |m| equip(m, g, SET_HELM, 0)).await;
    tokio::time::sleep(Duration::from_millis(80)).await;
    let one = map.query(move |m| stat(m, g, 2)).await.unwrap();
    map.query(move |m| equip(m, g, SET_CHEST, 4)).await;
    tokio::time::sleep(Duration::from_millis(80)).await;
    let two = map.query(move |m| stat(m, g, 2)).await.unwrap();
    assert_eq!(two, one + 10);
    // Off again, the bonus goes.
    map.query(move |m| {
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        p.items.retain(|i| i.entry != SET_CHEST);
        p.refresh_inventory(&data);
    })
    .await;
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(map.query(move |m| stat(m, g, 2)).await, Some(one));
}

#[test]
fn a_random_property_rolls_and_is_kept() {
    let mut d = data();
    d.items.get_mut(&LOOT_ITEM).unwrap().random_property = 5;
    d.crafts.random_pools.insert(5, vec![(77, 100.0)]);
    d.crafts.random_properties.insert(77, [107, 0, 0, 0, 0]);
    let rolled = crate::game::map::enchant::roll_random_property(&d, LOOT_ITEM, &mut rand::rng());
    assert_eq!(rolled, 77);
    let mut i =
        crate::game::map::player::Item::new(&d, guid::item(1), Guid::zero(), LOOT_ITEM, 0, 0, 1);
    crate::game::map::enchant::set_random_property(&d, &mut i, rolled);
    let row = i.to_row();
    assert_eq!(row.data, "rp:77");
    let back = crate::game::map::player::Item::from_row(&d, &row, Guid::zero());
    assert_eq!(back.fields.get_u32(ITEM_FIELD_RANDOM_PROPERTIES_ID), 77);
    assert_eq!(
        crate::game::map::enchant::get(&back, crate::game::map::enchant::PROP_SLOT_0).id,
        107,
        "a property's enchantments come back with it"
    );
}

/// The real data's locks, sets, properties and fishing skills load, and
/// a copper vein needs mining.
#[test]
fn the_real_crafting_data_loads() {
    let Some(d) = crate::game::data::load::real_data() else {
        return;
    };
    let vein = d.gameobject_templates.get(&1731).expect("Copper Vein");
    let keys = d.crafts.locks.get(&vein.data[0]).expect("its lock");
    assert!(keys.iter().any(|k| k.kind == 2 && k.index == 3), "{keys:?}");
    assert!(
        d.crafts.item_sets.len() > 100,
        "{}",
        d.crafts.item_sets.len()
    );
    assert!(d.crafts.random_pools.len() > 100);
    assert!(d.crafts.random_properties.len() > 1000);
    assert!(d.crafts.enchant_charges.get(&2823).is_some_and(|c| *c > 0));
    assert!(!d.crafts.fishing_skill.is_empty());
    assert_eq!(
        d.spells.enchantments.get(&107).map(|e| e.effects[0].0),
        Some(3)
    );
    // Herb Gathering and Skinning are profession effects.
    let herb = d.spells.get(2366).expect("Herb Gathering");
    assert!(crate::game::map::professions::handles(d, herb, 0));
    let fishing = d.spells.get(7620).expect("Fishing");
    assert!(crate::game::map::professions::handles(d, fishing, 0));
}

#[tokio::test]
async fn a_skinner_skins_a_looted_corpse() {
    let (_realm, mut a, map) = alone("Alice", Class::Warrior).await;
    let g = a.guid;
    let boar = guid::unit(3002, 24);
    a.sees(boar).await;
    let (before, flagged) = map
        .query(move |m| {
            learn(m, g, SKINNING, 393);
            let _ = crate::game::map::inventory::add_item(m, g, 7005, 1, false);
            crate::game::map::combat::kill(m, g, boar);
            let flagged = m.creature(boar).is_some_and(|c| {
                c.fields.has_flag(
                    UNIT_FIELD_FLAGS,
                    crate::game::map::professions::UNIT_FLAG_SKINNABLE,
                )
            });
            (skill_of(m, g, 393), flagged)
        })
        .await
        .unwrap();
    assert!(flagged, "a corpse with nothing on it can be skinned");
    // An attempt may fail (the skill rolled against the corpse's need):
    // try until it takes.
    for _ in 0..40 {
        let skinned = map
            .query(move |m| {
                if let Some(c) = m.caster_mut(g) {
                    c.gcd_until = None;
                    c.cooldowns.clear();
                }
                crate::game::map::spells::cast(
                    m,
                    g,
                    SKINNING,
                    Targets {
                        unit: Some(boar),
                        ..Default::default()
                    },
                    None,
                );
                m.creature(boar).is_some_and(|c| {
                    !c.fields.has_flag(
                        UNIT_FIELD_FLAGS,
                        crate::game::map::professions::UNIT_FLAG_SKINNABLE,
                    )
                })
            })
            .await
            .unwrap();
        if skinned {
            break;
        }
    }
    tokio::time::sleep(Duration::from_millis(60)).await;
    a.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
    a.wait_for("SMSG_ITEM_PUSH_RESULT", |m| match m {
        ServerOpcodeMessage::SMSG_ITEM_PUSH_RESULT(r) if r.item == LEATHER => Some(()),
        _ => None,
    })
    .await;
    assert_eq!(
        map.query(move |m| skill_of(m, g, 393)).await,
        Some(before + 1)
    );
    // Skinned once, it can't be again.
    let again = map
        .query(move |m| {
            m.creature(boar).is_some_and(|c| {
                c.fields.has_flag(
                    UNIT_FIELD_FLAGS,
                    crate::game::map::professions::UNIT_FLAG_SKINNABLE,
                )
            })
        })
        .await;
    assert_eq!(again, Some(false));
}
