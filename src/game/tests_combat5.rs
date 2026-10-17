//! The parity sweep's fixes (docs/COMBAT5_SWEEP.md), through the map: the
//! attack table's defender auras, sheathed weapons and mechanic resistance;
//! casts restarting swings; sheathing and Pacify-and-Silence; healing taken;
//! kill XP and its shares; NPC kills; loot's cap and grouped quest drops;
//! money split; wear, repairs, sales, buyback and restock; equip cooldowns;
//! set skills; threat; and calls for help.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::crafts::ItemSet;
use crate::game::data::items::ItemSpell;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::auras;
use crate::game::map::combat::{self, Hand, KillXp, Roll};
use crate::game::map::creature_combat;
use crate::game::map::spells::{self, outcome};
use crate::game::map::{Map, MapHandle, MapKey, vendor};
use crate::game::store::BAG_PLAYER;

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const DUMMY: (u32, u32) = (3500, 60);
const WOLF: u32 = 3501;
const WOLF_A: u32 = 61;
const WOLF_B: u32 = 62;
const VENDOR: (u32, u32) = (3502, 63);

/// The wolves' side, which answers calls for help; the humans'.
const PACK: u32 = 50;
const HUMAN: u32 = 1;

const HIT_DOWN: u32 = 94_001;
const RESIST_STUN: u32 = 94_002;
const KIDNEY: u32 = 94_003;
const SHEATH_BREAKS: u32 = 94_004;
const HUSH: u32 = 94_005;
const CUT_HALF: u32 = 94_006;
const CUT_TENTH: u32 = 94_007;
const GAIN_FIFTH: u32 = 94_008;
const LESS_HOLY: u32 = 94_009;
const TRINKET_SPELL: u32 = 94_010;
const SET_BONUS: u32 = 94_011;

const HELM: u32 = 62_001;
const GLOVES: u32 = 62_002;
const ARROWS: u32 = 62_003;
const TRINKET: u32 = 62_004;
const GOGGLES: u32 = 62_005;
const BOOTS: u32 = 62_006;
const SET: u32 = 77;
const ENGINEERING: u32 = 202;

fn effect(effect: u32, base_points: i32, target_a: u32) -> SpellEffect {
    SpellEffect {
        effect,
        base_points,
        base_dice: 1,
        die_sides: 1,
        target_a,
        ..Default::default()
    }
}

fn aura(kind: u32, base_points: i32, misc: i32, target_a: u32) -> SpellEffect {
    SpellEffect {
        aura: kind,
        misc_value: misc,
        ..effect(6, base_points, target_a)
    }
}

fn spell(id: u32, effects: [SpellEffect; 3]) -> SpellTemplate {
    SpellTemplate {
        id,
        name: format!("Spell {id}"),
        range_index: 1,
        equipped_item_class: -1,
        duration_index: 9,
        effects,
        ..Default::default()
    }
}

fn creature(entry: u32, faction: u32) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: format!("Creature {entry}"),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: 10,
        level_max: 10,
        faction,
        unit_class: 1,
        health_multiplier: 1.0,
        damage_multiplier: 1.0,
        base_attack_time: 2000,
        detection_range: 1.0,
        ..Default::default()
    }
}

fn armor(entry: u32, inventory_type: u32, durability: u32) -> ItemTemplate {
    ItemTemplate {
        entry,
        class: 4,
        subclass: 1,
        name: format!("Item {entry}"),
        display_id: 1000,
        inventory_type,
        item_level: 10,
        max_durability: durability,
        sell_price: 100,
        stackable: 1,
        ..Default::default()
    }
}

fn data() -> GameData {
    let mut d = GameData::empty();
    d.player_level_stats.insert((1, 1, 1), [23, 20, 22, 20, 21]);
    d.player_class_stats.insert((1, 1), (60, 0));
    d.class_level_stats.insert(
        (1, 10),
        ClassLevelStats {
            health: 5000,
            melee_damage: 1.0,
            ..Default::default()
        },
    );
    d.faction_templates.insert(
        PACK,
        FactionTemplate {
            id: PACK,
            faction: 50,
            flags: 0x1,
            ours: 0x8,
            friend_mask: 0x8,
            enemy_mask: 0x3,
            ..Default::default()
        },
    );
    d.faction_templates.insert(
        HUMAN,
        FactionTemplate {
            id: HUMAN,
            faction: 1,
            ours: 0x3,
            friend_mask: 0x2,
            enemy_mask: 0xC,
            ..Default::default()
        },
    );
    d.creature_templates.insert(DUMMY.0, creature(DUMMY.0, 8));
    let mut wolf = creature(WOLF, PACK);
    wolf.call_for_help_range = 15.0;
    d.creature_templates.insert(WOLF, wolf);
    let mut merchant = creature(VENDOR.0, 8);
    merchant.npc_flags = vendor::NPC_FLAG_VENDOR | vendor::NPC_FLAG_REPAIR;
    d.creature_templates.insert(VENDOR.0, merchant);
    let spawn = |guid, entry, dx: f32, dy: f32| CreatureSpawn {
        guid,
        entries: vec![entry],
        map: 0,
        x: START.0 + dx,
        y: START.1 + dy,
        z: START.2,
        respawn_min: 100,
        respawn_max: 100,
        ..Default::default()
    };
    d.creature_spawns.insert(
        0,
        vec![
            spawn(DUMMY.1, DUMMY.0, 2.0, 0.0),
            spawn(WOLF_A, WOLF, 30.0, 0.0),
            spawn(WOLF_B, WOLF, 36.0, 0.0),
            spawn(VENDOR.1, VENDOR.0, -2.0, 0.0),
        ],
    );
    d.vendors.insert(
        VENDOR.0,
        vec![VendorItem {
            slot: 1,
            item: ARROWS,
            max_count: 3,
            restock_secs: 10,
            ..Default::default()
        }],
    );
    // Armor of item level 10 costs ten a point to mend.
    let mut costs = vec![0; 29];
    costs[22] = 10;
    d.durability_costs.insert(10, costs);
    d.durability_quality.insert(2, 1.0);
    d.items.insert(HELM, armor(HELM, 1, 10));
    d.items.insert(GLOVES, armor(GLOVES, 10, 50));
    let mut arrows = armor(ARROWS, 0, 0);
    arrows.class = 6;
    arrows.stackable = 20;
    arrows.buy_price = 10;
    d.items.insert(ARROWS, arrows);
    let mut trinket = armor(TRINKET, 12, 0);
    trinket.spells[0] = ItemSpell {
        id: TRINKET_SPELL,
        trigger: 0,
        cooldown: 120_000,
        ..Default::default()
    };
    d.items.insert(TRINKET, trinket);
    for (entry, kind) in [(GOGGLES, 1), (BOOTS, 8)] {
        let mut it = armor(entry, kind, 0);
        it.set_id = SET;
        d.items.insert(entry, it);
    }
    d.crafts.item_sets.insert(
        SET,
        ItemSet {
            id: SET,
            items: vec![GOGGLES, BOOTS],
            spells: vec![(SET_BONUS, 2)],
            required_skill: ENGINEERING,
            required_skill_rank: 100,
        },
    );
    let s = &mut d.spells;
    s.ranges.insert(1, (0.0, 30.0));
    s.durations.insert(
        9,
        crate::game::data::spells::SpellDuration {
            base: 30_000,
            ..Default::default()
        },
    );
    let none = SpellEffect::default();
    let mut all = vec![
        spell(HIT_DOWN, [aura(184, 9, 0, 1), none, none]),
        spell(RESIST_STUN, [aura(117, 99, 12, 1), none, none]),
        spell(LESS_HOLY, [aura(115, -1001, 2, 1), none, none]),
        spell(CUT_HALF, [aura(118, -51, 0, 1), none, none]),
        spell(CUT_TENTH, [aura(118, -11, 0, 1), none, none]),
        spell(GAIN_FIFTH, [aura(118, 19, 0, 1), none, none]),
        spell(TRINKET_SPELL, [effect(3, 0, 1), none, none]),
        spell(SET_BONUS, [aura(4, 0, 0, 1), none, none]),
    ];
    let mut kidney = spell(KIDNEY, [effect(58, 0, 6), none, none]);
    kidney.mechanic = 12;
    kidney.dmg_class = 2;
    all.push(kidney);
    let mut sheath = spell(SHEATH_BREAKS, [aura(4, 0, 0, 1), none, none]);
    sheath.aura_interrupt_flags = auras::INTERRUPT_SHEATHING;
    all.push(sheath);
    all.push(spell(HUSH, [aura(60, 0, 0, 1), none, none]));
    for t in all {
        s.spells.insert(t.id, t);
    }
    d
}

async fn setup() -> (std::sync::Arc<crate::game::Realm>, Client, Guid, MapHandle) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.sees(dummy()).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    (realm, a, ga, map)
}

fn dummy() -> Guid {
    guid::unit(DUMMY.0, DUMMY.1)
}

fn template(m: &Map, id: u32) -> SpellTemplate {
    m.realm.data.spells.get(id).unwrap().clone()
}

/// An item of `entry` into `g`'s bags, durability `left`: its guid.
fn give(m: &mut Map, g: Guid, entry: u32, count: u32, left: Option<u32>) -> Guid {
    crate::game::map::inventory::add_item(m, g, entry, count, false).unwrap();
    let p = m.players.get_mut(&g).unwrap();
    let it = p.items.iter_mut().rev().find(|i| i.entry == entry).unwrap();
    if let Some(left) = left {
        it.fields.set_u32(ITEM_FIELD_DURABILITY, left);
    }
    it.guid
}

#[tokio::test]
async fn the_attack_table_counts_the_defenders_auras_and_sheathed_weapons_block_nothing() {
    let (_realm, _a, ga, map) = setup().await;
    let target = dummy();
    let (debuff, sheathed, drawn) = map
        .query(move |m| {
            spells::cast_triggered(m, target, HIT_DOWN, Some(target));
            let debuff = combat::roll_for(m, ga, target, Hand::Main)
                .unwrap()
                .attacker_hit;
            let sheathed = combat::roll_for(m, target, ga, Hand::Main)
                .unwrap()
                .sheathed;
            combat::sheathe(m, ga, 1);
            let drawn = combat::roll_for(m, target, ga, Hand::Main)
                .unwrap()
                .sheathed;
            (debuff, sheathed, drawn)
        })
        .await
        .unwrap();
    assert_eq!(debuff, 10.0);
    assert!(sheathed && !drawn);
    let even = Roll {
        attacker_skill: 50,
        victim_defense: 50,
        attacker_level: 10,
        victim_level: 10,
        victim_is_player: true,
        attacker_is_player: true,
        standing: true,
        block: 10.0,
        ..Default::default()
    };
    let hindered = Roll {
        attacker_hit: 10.0,
        ..even
    };
    assert_eq!(even.miss_bp(), 500);
    assert_eq!(hindered.miss_bp(), 0, "the defender's aura takes it off");
    let away = Roll {
        sheathed: true,
        ..even
    };
    assert_eq!(combat::roll_outcome(&even, 600), combat::Outcome::Block);
    assert_ne!(combat::roll_outcome(&away, 600), combat::Outcome::Block);
}

#[tokio::test]
async fn an_ability_with_a_mechanic_is_resisted_past_the_miss_chance() {
    let (_realm, _a, ga, map) = setup().await;
    let target = dummy();
    let hit = map
        .query(move |m| {
            spells::cast_triggered(m, target, RESIST_STUN, Some(target));
            let p = m.players.get_mut(&ga).unwrap();
            p.derived.hit = 100.0;
            let s = template(m, KIDNEY);
            let mods = spells::mods::for_spell(m, ga, &s);
            outcome::melee_roll(m, ga, target, &s, &mods).0
        })
        .await
        .unwrap();
    assert_eq!(hit, outcome::Hit::Resist);
}

#[tokio::test]
async fn a_cast_starts_the_swing_over_and_some_end_auto_attack() {
    let (_realm, _a, ga, map) = setup().await;
    let target = dummy();
    let (restarted, stopped) = map
        .query(move |m| {
            combat::start_attack(m, ga, target);
            let now = m.now;
            m.combat_mut(ga).unwrap().next_swing = Some(now);
            let interrupting = SpellTemplate {
                interrupt_flags: 0x08,
                ..Default::default()
            };
            combat::spell_launched(m, ga, &interrupting);
            let period = m
                .unit_fields(ga)
                .unwrap()
                .get_u32(UNIT_FIELD_BASEATTACKTIME);
            let restarted = m.combat(ga).unwrap().next_swing
                == Some(now + Duration::from_millis(u64::from(period)));
            let mut gouge = SpellTemplate::default();
            gouge.attributes[0] = crate::game::data::spells::ATTR_CANCELS_AUTO_ATTACK;
            combat::spell_launched(m, ga, &gouge);
            (restarted, !m.combat(ga).unwrap().attacking)
        })
        .await
        .unwrap();
    assert!(restarted);
    assert!(stopped);
}

#[tokio::test]
async fn sheathing_breaks_its_auras_and_pacify_and_silence_pacifies_too() {
    let (_realm, _a, ga, map) = setup().await;
    let (broken, both, cleared) = map
        .query(move |m| {
            spells::cast_triggered(m, ga, SHEATH_BREAKS, Some(ga));
            combat::sheathe(m, ga, 1);
            let broken = !auras::auras(m, ga).unwrap().has_spell(SHEATH_BREAKS);
            spells::cast_triggered(m, ga, HUSH, Some(ga));
            let flags = |m: &Map| m.players[&ga].fields.get_u32(UNIT_FIELD_FLAGS);
            let both = flags(m) & (auras::UNIT_FLAG_PACIFIED | auras::UNIT_FLAG_SILENCED)
                == auras::UNIT_FLAG_PACIFIED | auras::UNIT_FLAG_SILENCED;
            auras::remove_spell(m, ga, HUSH, None);
            let cleared = flags(m) & (auras::UNIT_FLAG_PACIFIED | auras::UNIT_FLAG_SILENCED) == 0;
            (broken, both, cleared)
        })
        .await
        .unwrap();
    assert!(broken);
    assert!(both, "pacified and silenced");
    assert!(cleared);
}

#[tokio::test]
async fn healing_taken_keeps_the_strongest_cut_and_gain_and_loses_at_most_half() {
    let (_realm, _a, ga, map) = setup().await;
    let healed = map
        .query(move |m| {
            for s in [CUT_HALF, CUT_TENTH, GAIN_FIFTH, LESS_HOLY] {
                spells::cast_triggered(m, ga, s, Some(ga));
            }
            outcome::healing_taken(m, ga, 100.0, 2)
        })
        .await
        .unwrap();
    // (100 - 50) * 0.5 * 1.2: the flat cut stops at half, the tenth's cut
    // is outdone by the half's.
    assert!((healed - 30.0).abs() < 0.01, "{healed}");
}

#[test]
fn kill_xp_and_its_shares_go_as_thistle_tea_counts_them() {
    let elite = KillXp {
        elite: true,
        ..Default::default()
    };
    let dungeon = KillXp {
        dungeon: true,
        ..elite
    };
    assert_eq!(combat::kill_xp_with(10, 10, 10, &elite), 190);
    // 237.5, rounded half to even.
    assert_eq!(combat::kill_xp_with(10, 10, 10, &dungeon), 238);
    // 52.5 rounds down to the even 52.
    assert_eq!(combat::kill_xp(1, 2, false, 1.0), 52);
    let summoned = KillXp {
        no_xp: true,
        ..Default::default()
    };
    assert_eq!(combat::kill_xp_with(10, 10, 10, &summoned), 0);
    assert_eq!(combat::group_xp_rate(10), 0.5);
    assert_eq!(combat::group_xp_rate(5), 1.4);
    // A level 30 for whom the kill is grey, beside a level 20: the 20's
    // level-weighted share halved, plus one.
    let (shares, top) = combat::group_shares(&[30, 20], 21, &KillXp::default());
    assert_eq!(top, Some(20));
    let base = combat::kill_xp(20, 21, false, 1.0) as f32;
    assert_eq!(shares[1], (base * 20.0 / 50.0 / 2.0) as u32 + 1);
}

#[tokio::test]
async fn a_creature_mostly_killed_by_npcs_drops_nothing_and_gives_its_share_of_xp() {
    let (_realm, _a, ga, map) = setup().await;
    let target = dummy();
    let npc = guid::unit(WOLF, WOLF_A);
    let (refused, none, allowed, half) = map
        .query(move |m| {
            creature_combat::damaged(m, target, npc, 700);
            creature_combat::damaged(m, target, ga, 300);
            let refused = !creature_combat::loot_allowed(m, target);
            let none = creature_combat::xp_share(m, target);
            creature_combat::disengaged(m, target);
            creature_combat::damaged(m, target, npc, 500);
            creature_combat::damaged(m, target, ga, 500);
            (
                refused,
                none,
                creature_combat::loot_allowed(m, target),
                creature_combat::xp_share(m, target),
            )
        })
        .await
        .unwrap();
    assert!(refused);
    assert_eq!(none, 0.0);
    assert!(allowed);
    assert_eq!(half, 0.5);
}

#[test]
fn loot_holds_sixteen_items_and_a_grouped_quest_drop_is_picked_by_its_chance() {
    let data = GameData::empty();
    let always = |item| LootEntry {
        item,
        chance: 100.0,
        min: 1,
        max: 1,
        ..Default::default()
    };
    let many: Vec<LootEntry> = (1..=20).map(always).collect();
    let mut out = Vec::new();
    crate::game::map::loot::roll(&data, &many, &[], &mut rand::rng(), &mut out, 0);
    assert_eq!(out.len(), 16);
    // A group of a certain quest drop and an equally chanced item: the
    // quest drop wins the group, and no one wanting it, nothing drops.
    let group = vec![
        LootEntry {
            item: 30,
            chance: -100.0,
            group: 1,
            min: 1,
            max: 1,
            ..Default::default()
        },
        LootEntry {
            item: 31,
            group: 1,
            min: 1,
            max: 1,
            ..Default::default()
        },
    ];
    let mut out = Vec::new();
    crate::game::map::loot::roll(&data, &group, &[], &mut rand::rng(), &mut out, 0);
    assert!(out.is_empty(), "{out:?}");
    let mut out = Vec::new();
    crate::game::map::loot::roll(&data, &group, &[30], &mut rand::rng(), &mut out, 0);
    assert_eq!(out.iter().map(|s| s.item).collect::<Vec<_>>(), vec![30]);
}

#[tokio::test]
async fn money_is_split_evenly_among_the_group_near_and_each_share_announced() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    a.send(CMSG_GROUP_INVITE { name: "Bob".into() }).await;
    b.wait_for("SMSG_GROUP_INVITE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
    })
    .await;
    b.send(CMSG_GROUP_ACCEPT {}).await;
    a.wait_for("SMSG_GROUP_LIST", |m| match m {
        ServerOpcodeMessage::SMSG_GROUP_LIST(l) if !l.members.is_empty() => Some(()),
        _ => None,
    })
    .await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let target = dummy();
    let money = map
        .query(move |m| {
            combat::kill(m, ga, target);
            if let Some(l) = m.creature_mut(target).and_then(|c| c.loot.as_mut()) {
                l.gold = 101;
                l.owner = None;
            }
            crate::game::map::loot::open(m, ga, target);
            crate::game::map::loot::take_money(m, ga);
            (m.players[&ga].money(), m.players[&gb].money())
        })
        .await
        .unwrap();
    assert_eq!(money, (50, 50), "even shares, the copper over lost");
    for c in [&mut a, &mut b] {
        let body = c
            .wait_frame("SMSG_LOOT_MONEY_NOTIFY", 0x163, |b| b.len() == 4)
            .await;
        assert_eq!(u32::from_le_bytes(body[..4].try_into().unwrap()), 50);
    }
}

#[tokio::test]
async fn dying_wears_gear_down_at_least_a_point_but_not_to_a_player() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let target = dummy();
    let (by_creature, by_player) = map
        .query(move |m| {
            let helm = give(m, ga, HELM, 1, None);
            let p = m.players.get_mut(&ga).unwrap();
            p.items
                .retain(|i| !(i.bag == BAG_PLAYER && i.slot == 0) || i.guid == helm);
            let i = p.item_by_guid(helm).unwrap();
            p.items[i].bag = BAG_PLAYER;
            p.items[i].slot = 0;
            let left = |m: &Map| {
                let p = &m.players[&ga];
                p.items[p.item_by_guid(helm).unwrap()]
                    .fields
                    .get_u32(ITEM_FIELD_DURABILITY)
            };
            combat::kill(m, target, ga);
            let by_creature = left(m);
            crate::game::map::death::resurrect(m, ga, 1.0);
            combat::kill(m, gb, ga);
            (by_creature, left(m))
        })
        .await
        .unwrap();
    assert_eq!(by_creature, 9, "a tenth of ten, a point");
    assert_eq!(by_player, 9, "no wear dying to a player");
}

#[tokio::test]
async fn repairing_everything_is_all_or_nothing_and_a_damaged_sale_pays_its_repair() {
    let (_realm, _a, ga, map) = setup().await;
    let npc = guid::unit(VENDOR.0, VENDOR.1);
    let (short, repaired, sale, partial) = map
        .query(move |m| {
            let gloves = give(m, ga, GLOVES, 1, Some(40));
            let helm = give(m, ga, HELM, 1, Some(5));
            // Ten points and five at ten copper each: 150.
            m.players.get_mut(&ga).unwrap().set_money(149);
            vendor::repair(m, ga, npc, Guid::zero());
            let wear = |m: &Map, g: Guid| {
                let p = &m.players[&ga];
                p.items[p.item_by_guid(g).unwrap()]
                    .fields
                    .get_u32(ITEM_FIELD_DURABILITY)
            };
            let short = (wear(m, gloves), wear(m, helm), m.players[&ga].money());
            m.players.get_mut(&ga).unwrap().set_money(150);
            vendor::repair(m, ga, npc, Guid::zero());
            let repaired = (wear(m, gloves), wear(m, helm), m.players[&ga].money());
            // A worn helm sells for its price less its mending.
            let helm2 = give(m, ga, HELM, 1, Some(5));
            m.players.get_mut(&ga).unwrap().set_money(0);
            vendor::sell(m, ga, npc, helm2, 0);
            let sale = m.players[&ga].money();
            // Two of a stack of five, bought back on their own.
            let arrows = give(m, ga, ARROWS, 5, None);
            vendor::sell(m, ga, npc, arrows, 2);
            let p = &m.players[&ga];
            let left = p.items[p.item_by_guid(arrows).unwrap()].count();
            let sold = p
                .items
                .iter()
                .find(|i| i.entry == ARROWS && vendor::in_buyback(i.bag, i.slot))
                .map(|i| (i.guid != arrows, i.count(), i.slot));
            let stamp = sold.map(|(_, _, slot)| {
                p.fields.get_u32(
                    PLAYER_FIELD_BUYBACK_TIMESTAMP_1
                        + u16::from(slot - crate::game::map::inventory::BUYBACK_START),
                )
            });
            (short, repaired, sale, (left, sold, stamp))
        })
        .await
        .unwrap();
    assert_eq!(
        short,
        (40, 5, 149),
        "nothing mended short of the whole price"
    );
    assert_eq!(repaired, (50, 10, 0));
    assert_eq!(sale, 50, "a hundred less fifty to mend");
    let (left, sold, stamp) = partial;
    assert_eq!(left, 3);
    let (own, count, _) = sold.expect("in buyback");
    assert!(own && count == 2, "its own item of two");
    assert!(
        stamp.is_some_and(|s| (108_000..108_100).contains(&s)),
        "{stamp:?}"
    );
}

#[tokio::test]
async fn limited_stock_comes_back_a_buy_count_each_restock_interval() {
    let (_realm, _a, ga, map) = setup().await;
    let npc = guid::unit(VENDOR.0, VENDOR.1);
    let (after_buy, later) = map
        .query(move |m| {
            m.players.get_mut(&ga).unwrap().set_money(10_000);
            vendor::buy(m, ga, npc, ARROWS, 3);
            let v = m.realm.data.vendor_items(VENDOR.0)[0];
            let after_buy = vendor::stock_left(m, npc, &v, 1);
            let now = m.now;
            let s = m.vendor_stock.get_mut(&(npc, ARROWS)).unwrap();
            let interval = s.restock;
            s.last_increment_at = now - interval * 2 - Duration::from_millis(1);
            (after_buy, vendor::stock_left(m, npc, &v, 1))
        })
        .await
        .unwrap();
    assert_eq!(after_buy, 0);
    assert_eq!(later, 2, "one back for each interval gone by");
}

#[tokio::test]
async fn putting_a_trinket_on_starts_its_spell_on_a_thirty_second_cooldown() {
    let (_realm, mut a, ga, map) = setup().await;
    let waits = map
        .query(move |m| {
            let trinket = give(m, ga, TRINKET, 1, None);
            let p = &m.players[&ga];
            let at = p.items[p.item_by_guid(trinket).unwrap()].clone();
            crate::game::map::inventory::move_item(m, ga, (at.bag, at.slot), (BAG_PLAYER, 12));
            let now = m.now;
            m.players[&ga]
                .caster
                .cooldowns
                .get(&TRINKET_SPELL)
                .map(|t| t.duration_since(now))
        })
        .await
        .unwrap();
    assert_eq!(waits, Some(crate::game::map::items::EQUIP_COOLDOWN));
    a.wait_for("SMSG_ITEM_COOLDOWN", |m| match m {
        ServerOpcodeMessage::SMSG_ITEM_COOLDOWN(c) if c.id == TRINKET_SPELL => Some(()),
        _ => None,
    })
    .await;
}

#[tokio::test]
async fn a_set_that_asks_a_skill_gives_nothing_without_it() {
    let (_realm, _a, ga, map) = setup().await;
    let (without, with) = map
        .query(move |m| {
            for (entry, slot) in [(GOGGLES, 0), (BOOTS, 7)] {
                let g = give(m, ga, entry, 1, None);
                let p = m.players.get_mut(&ga).unwrap();
                p.items
                    .retain(|i| !(i.bag == BAG_PLAYER && i.slot == slot) || i.guid == g);
                let i = p.item_by_guid(g).unwrap();
                p.items[i].bag = BAG_PLAYER;
                p.items[i].slot = slot;
            }
            let data = m.realm.data.clone();
            let without = crate::game::map::enchant::set_spells(&data, &m.players[&ga]).len();
            let p = m.players.get_mut(&ga).unwrap();
            crate::game::map::skills::learn_skill(p, ENGINEERING, 2);
            crate::game::map::skills::set_value(p, ENGINEERING, 100);
            let with = crate::game::map::enchant::set_spells(&data, &m.players[&ga]).len();
            (without, with)
        })
        .await
        .unwrap();
    assert_eq!((without, with), (0, 1));
}

#[tokio::test]
async fn threat_passes_to_one_in_melee_past_a_tenth_further_down_the_list() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let mut c = Client::connect(&realm, "C");
    let gc = c.enter("Cid", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let target = dummy();
    let next = map
        .query(move |m| {
            let at = m.creature(target).unwrap().position;
            // Bob at range with 115, Cid beside it with 112, Alice its
            // victim with 100.
            for (g, dx) in [(ga, 1.0), (gb, 20.0), (gc, 1.0)] {
                m.players.get_mut(&g).unwrap().movement.position = Vector3d { x: at.x + dx, ..at };
            }
            let cr = m.creature_mut(target).unwrap();
            cr.threat = vec![(ga, 100.0), (gb, 115.0), (gc, 112.0)];
            cr.combat.victim = ga;
            crate::game::map::ai::overtake(m, target, ga)
        })
        .await
        .unwrap();
    assert_eq!(next, gc);
}

#[tokio::test]
async fn a_creature_calls_its_kind_near_and_they_come_a_moment_later() {
    let (_realm, mut a, ga, map) = setup().await;
    let (wolf_a, wolf_b) = (guid::unit(WOLF, WOLF_A), guid::unit(WOLF, WOLF_B));
    a.sees(wolf_b).await;
    let (at_once, later) = map
        .query(move |m| {
            crate::game::map::ai::attacked(m, wolf_a, ga);
            let at_once = m.creature(wolf_b).unwrap().combat.attacking;
            let now = m.now;
            for x in m.creature_mut(wolf_a).unwrap().engagement.assist.iter_mut() {
                x.due = now;
            }
            creature_combat::update(m);
            let b = m.creature(wolf_b).unwrap();
            (at_once, b.combat.attacking && b.combat.victim == ga)
        })
        .await
        .unwrap();
    assert!(!at_once, "not at once");
    assert!(later, "a moment later");
}

/// A creature swings only from within its melee range, as the client
/// measures it: one a yard beyond doesn't hit from there (it did, the
/// player out of reach of their own swing), but closes in first. Two
/// running units have the leeway.
#[tokio::test]
async fn a_creature_swings_only_from_within_its_reach() {
    let (_realm, _a, ga, map) = setup().await;
    let wolf = guid::unit(WOLF, WOLF_A);
    let (from_beyond, closed, swung, leeway) = map
        .query(move |m| {
            let at = m.creature(wolf).unwrap().position;
            crate::game::map::ai::attacked(m, wolf, ga);
            let reach = combat::melee_range(&m.unit_view(wolf).unwrap(), &m.unit_view(ga).unwrap());
            m.players.get_mut(&ga).unwrap().movement.position = Vector3d {
                x: at.x + reach + 0.6,
                ..at
            };
            let swung = |m: &Map| {
                let now = m.now;
                m.creature(wolf)
                    .unwrap()
                    .combat
                    .next_swing
                    .is_some_and(|t| t > now)
            };
            crate::game::map::ai::update_creatures(m);
            let from_beyond = swung(m);
            for _ in 0..40 {
                m.now += Duration::from_millis(100);
                crate::game::map::ai::update_creatures(m);
                if swung(m) {
                    break;
                }
            }
            let (w, p) = (m.unit_view(wolf).unwrap(), m.unit_view(ga).unwrap());
            let closed = combat::dist(w.position, p.position) <= combat::melee_range(&w, &p);
            let (mut w, mut p) = (w, p);
            p.position.x = w.position.x + combat::melee_range(&w, &p) + 2.0;
            let still = combat::in_melee_range(&w, &p);
            (w.running, p.running) = (true, true);
            (
                from_beyond,
                closed,
                swung(m),
                (still, combat::in_melee_range(&w, &p)),
            )
        })
        .await
        .unwrap();
    assert!(!from_beyond, "no swing from beyond its reach");
    assert!(closed && swung, "it closes in and swings");
    assert_eq!(leeway, (false, true), "the leeway is for two running");
}
