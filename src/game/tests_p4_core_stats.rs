//! Phase 3, round 4, against Thistle Tea's `stats_test.exs`: a player's
//! stats from their base, gear and auras (the percentages, flats and
//! totals, and their order), armor and resistances with the exclusive
//! auras, and attack power into weapon damage. Through a player's fields,
//! with the level's base stats put in the data.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::GameData;
use crate::game::data::items::ItemTemplate;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::auras::*;
use crate::game::map::derive;
use crate::game::map::player::Item;
use crate::game::map::{Map, MapKey};
use crate::game::store::BAG_PLAYER;

/// Thistle Tea's level 60 mage: strength to spirit, health and mana.
pub(crate) const MAGE_STATS: [u32; 5] = [35, 41, 45, 125, 126];
pub(crate) const MAGE_POOLS: (u32, u32) = (1360, 1273);

const INTELLECT_35: u32 = 962_001;
const ARMOR_100: u32 = 962_002;
const GREATSWORD: u32 = 962_003;
const BOW: u32 = 962_004;

/// An aura spell of up to three effects, each `(aura, amount, misc)`.
fn spell(id: u32, effects: &[(u32, i32, i32)]) -> SpellTemplate {
    let mut e = [SpellEffect::default(); 3];
    for (i, (kind, amount, misc)) in effects.iter().enumerate() {
        e[i] = SpellEffect {
            effect: 6,
            aura: *kind,
            base_points: amount - 1,
            base_dice: 1,
            misc_value: *misc,
            target_a: 1,
            ..Default::default()
        };
    }
    SpellTemplate {
        id,
        name: format!("Aura {id}"),
        effects: e,
        equipped_item_class: -1,
        ..Default::default()
    }
}

pub(crate) fn data(spells: &[SpellTemplate]) -> GameData {
    let mut d = GameData::empty();
    for class in [8u8, 1, 3, 11] {
        for race in [1u8, 3, 4] {
            d.player_level_stats.insert((race, class, 1), MAGE_STATS);
        }
        d.player_class_stats.insert((class, 1), MAGE_POOLS);
    }
    d.items.insert(
        INTELLECT_35,
        ItemTemplate {
            entry: INTELLECT_35,
            class: 4,
            subclass: 1,
            name: "Circlet".into(),
            display_id: 1000,
            inventory_type: 1,
            stats: {
                let mut s = [(0, 0); 10];
                s[0] = (5, 35);
                s
            },
            stackable: 1,
            ..Default::default()
        },
    );
    d.items.insert(
        ARMOR_100,
        ItemTemplate {
            entry: ARMOR_100,
            class: 4,
            subclass: 1,
            name: "Robe".into(),
            display_id: 1000,
            inventory_type: 5,
            armor: 100,
            resistances: [0, 10, 0, 0, 0, 0],
            stackable: 1,
            ..Default::default()
        },
    );
    let mut damages = [(0.0, 0.0, 0); 5];
    damages[0] = (100.0, 150.0, 0);
    d.items.insert(
        GREATSWORD,
        ItemTemplate {
            entry: GREATSWORD,
            class: 2,
            subclass: 8,
            name: "Greatsword".into(),
            display_id: 1000,
            inventory_type: 17,
            delay: 3400,
            damages,
            stackable: 1,
            ..Default::default()
        },
    );
    damages[0] = (20.0, 30.0, 0);
    d.items.insert(
        BOW,
        ItemTemplate {
            entry: BOW,
            class: 2,
            subclass: 2,
            name: "Bow".into(),
            display_id: 1000,
            inventory_type: 15,
            delay: 2500,
            ammo_type: 2,
            damages,
            stackable: 1,
            ..Default::default()
        },
    );
    for s in spells {
        d.spells.spells.insert(s.id, s.clone());
    }
    d
}

pub(crate) async fn on_map<T: Send + 'static>(
    realm: &Arc<Realm>,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    let key = realm
        .registry
        .online(guid::player(1))
        .map_or(MapKey::world(0), |o| o.map().key);
    realm
        .maps
        .get(realm, key)
        .query(f)
        .await
        .expect("the map answers")
}

pub(crate) async fn player(d: GameData, race: Race, class: Class) -> (Arc<Realm>, Client, Guid) {
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", race, class).await;
    (realm, a, g)
}

fn equip(m: &mut Map, g: Guid, entry: u32, slot: u8) {
    let data = m.realm.data.clone();
    let counter = m
        .realm
        .next_item
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let p = m.players.get_mut(&g).expect("the player");
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
    derive::unit(m, g);
}

fn put(m: &mut Map, g: Guid, spell: u32) {
    let s = m.realm.data.spells.get(spell).unwrap().clone();
    let effects: Vec<(usize, i32)> = (0..3)
        .filter(|i| s.effects[*i].effect == 6)
        .map(|i| (i, s.effects[i].base_points + 1))
        .collect();
    apply_spell(m, g, g, &s, &effects, Context::default()).expect("the aura takes");
    derive::unit(m, g);
}

fn take(m: &mut Map, g: Guid, spell: u32) {
    remove_spell(m, g, spell, None);
    derive::unit(m, g);
}

fn stacks(m: &mut Map, g: Guid, spell: u32, n: u32) {
    let p = m.players.get_mut(&g).unwrap();
    for h in p.auras.holders.iter_mut().filter(|h| h.spell == spell) {
        h.stacks = n;
    }
    derive::unit(m, g);
}

fn stats(m: &Map, g: Guid) -> [u32; 5] {
    let f = &m.players[&g].fields;
    std::array::from_fn(|i| f.get_u32(UNIT_FIELD_STAT0 + i as u16))
}

fn pools(m: &Map, g: Guid) -> (u32, u32) {
    let f = &m.players[&g].fields;
    (
        f.get_u32(UNIT_FIELD_MAXHEALTH),
        f.get_u32(UNIT_FIELD_MAXPOWER1),
    )
}

const ALL_DOWN_FLAT_TOTAL: u32 = 962_101;
const INT_UP_20: u32 = 962_102;
const INT_UP_50: u32 = 962_103;
const INT_TOTAL_20: u32 = 962_104;
const INT_TOTAL_50: u32 = 962_105;
const CLAMPED: u32 = 962_106;
const ALL_PLUS_5: u32 = 962_107;

#[tokio::test]
async fn base_and_gear_stats_take_their_percentages_before_flat_auras_and_the_totals_after() {
    let spells = [
        spell(
            ALL_DOWN_FLAT_TOTAL,
            &[
                (AURA_MOD_PERCENT_STAT, -75, -1),
                (AURA_MOD_STAT, 31, 3),
                (AURA_MOD_TOTAL_STAT_PERCENTAGE, 10, -1),
            ],
        ),
        spell(INT_UP_20, &[(AURA_MOD_PERCENT_STAT, 20, 3)]),
        spell(INT_UP_50, &[(AURA_MOD_PERCENT_STAT, 50, 3)]),
        spell(INT_TOTAL_20, &[(AURA_MOD_TOTAL_STAT_PERCENTAGE, 20, 3)]),
        spell(INT_TOTAL_50, &[(AURA_MOD_TOTAL_STAT_PERCENTAGE, 50, 3)]),
        // A penalty past a hundred, and selectors naming no stat.
        spell(
            CLAMPED,
            &[
                (AURA_MOD_PERCENT_STAT, -150, 3),
                (AURA_MOD_STAT, 31, 3),
                (AURA_MOD_PERCENT_STAT, 100, -2),
            ],
        ),
        spell(ALL_PLUS_5, &[(AURA_MOD_STAT, 5, -1)]),
    ];
    let (realm, _a, g) = player(data(&spells), Race::Human, Class::Mage).await;
    let got = on_map(&realm, move |m| {
        let mut out = vec![(stats(m, g), pools(m, g))];
        // The circlet's 35 intellect is scaled with the base, before the
        // flat 31; the total's ten percent last.
        equip(m, g, INTELLECT_35, 0);
        put(m, g, ALL_DOWN_FLAT_TOTAL);
        out.push((stats(m, g), pools(m, g)));
        take(m, g, ALL_DOWN_FLAT_TOTAL);
        out.push((stats(m, g), pools(m, g)));
        // Two percentages multiply, in either layer and either order.
        let mut products = Vec::new();
        for (a, b) in [
            (INT_UP_20, INT_UP_50),
            (INT_UP_50, INT_UP_20),
            (INT_TOTAL_20, INT_TOTAL_50),
            (INT_TOTAL_50, INT_TOTAL_20),
        ] {
            put(m, g, a);
            put(m, g, b);
            products.push(stats(m, g));
            take(m, g, a);
            take(m, g, b);
        }
        // A penalty is a factor of nothing, not less; a selector of no
        // stat changes none.
        put(m, g, CLAMPED);
        let clamped = stats(m, g);
        take(m, g, CLAMPED);
        // Misc -1 raises every stat.
        put(m, g, ALL_PLUS_5);
        let all = stats(m, g);
        (out, products, clamped, all)
    })
    .await;
    let (out, products, clamped, all) = got;
    let [s, a, st, i, sp] = MAGE_STATS;
    // Unbuffed: Thistle Tea's 1360 + 20 + 25 * 10 and 1273 + 20 + 105 * 15.
    assert_eq!(
        out[0],
        (MAGE_STATS, (1360 + 20 + 250, 1273 + 20 + 105 * 15))
    );
    // (125 + 35) * 0.25 + 31 = 71, * 1.1 = 78; stamina 45 * 0.25 * 1.1.
    assert_eq!(out[1].0[3], 78);
    assert_eq!(out[1].0[2], 12);
    assert_eq!(out[1].1, (1372, 2163));
    assert_eq!(out[2].0, [s, a, st, i + 35, sp]);
    for p in products {
        // The circlet's on: 160 at 1.2 and 1.5 (Thistle Tea's 125 makes 225).
        assert_eq!(p, [s, a, st, 288, sp]);
    }
    assert_eq!(clamped, [s, a, st, 31, sp]);
    assert_eq!(all, [s + 5, a + 5, st + 5, i + 35 + 5, sp + 5]);
}

/// The percentages tried: 5, 6 and 18 of a hundred are where a float's
/// factor falls short.
const AMOUNTS: [i32; 8] = [4, 5, 6, 8, 12, 16, 18, 20];
const PCT: [u32; 8] = [
    962_111, 962_112, 962_113, 962_114, 962_115, 962_116, 962_117, 962_118,
];
const TOTAL: [u32; 8] = [
    962_121, 962_122, 962_123, 962_124, 962_125, 962_126, 962_127, 962_128,
];

#[tokio::test]
async fn percentages_keep_exact_integer_totals_and_scale_by_their_stacks() {
    let mut spells = Vec::new();
    for (n, amount) in AMOUNTS.into_iter().enumerate() {
        spells.push(spell(PCT[n], &[(AURA_MOD_PERCENT_STAT, amount, 3)]));
        spells.push(spell(
            TOTAL[n],
            &[(AURA_MOD_TOTAL_STAT_PERCENTAGE, amount, 3)],
        ));
    }
    let mut d = data(&spells);
    // A base of exactly a hundred intellect.
    d.player_level_stats
        .insert((1, 8, 1), [35, 41, 45, 100, 126]);
    let (realm, _a, g) = player(d, Race::Human, Class::Mage).await;
    let got = on_map(&realm, move |m| {
        let mut exact = Vec::new();
        for id in PCT.into_iter().chain(TOTAL) {
            put(m, g, id);
            exact.push(stats(m, g)[3]);
            take(m, g, id);
        }
        // Twenty percent, three stacks then two, then gone.
        let mut stacked = Vec::new();
        for id in [PCT[7], TOTAL[7]] {
            put(m, g, id);
            stacks(m, g, id, 3);
            stacked.push(stats(m, g)[3]);
            stacks(m, g, id, 2);
            stacked.push(stats(m, g)[3]);
            take(m, g, id);
            stacked.push(stats(m, g)[3]);
        }
        (exact, stacked)
    })
    .await;
    let (exact, stacked) = got;
    let want: Vec<u32> = AMOUNTS.iter().map(|a| 100 + *a as u32).collect();
    assert_eq!(exact, [want.clone(), want].concat());
    assert_eq!(stacked, vec![160, 140, 100, 160, 140, 100]);
}

#[test]
fn a_scaled_stat_is_worked_out_in_whole_numbers() {
    // 101 intellect at 4% more: 105, whichever layer.
    assert_eq!(derive::scaled_stat(101, &[104], 0, &[]), 105);
    assert_eq!(derive::scaled_stat(101, &[], 0, &[104]), 105);
    assert_eq!(derive::scaled_stat(100, &[105], 0, &[]), 105);
    // Thistle Tea's mage: (160 * 25 + 31 * 100) * 110 / (100 * 100).
    assert_eq!(derive::scaled_stat(160, &[25], 31, &[110]), 78);
    assert_eq!(derive::scaled_stat(125, &[120, 150], 0, &[]), 225);
    assert_eq!(derive::scaled_stat(125, &[0], 31, &[]), 31);
    // A negative total is nothing.
    assert_eq!(derive::scaled_stat(10, &[], -40, &[]), 0);
}

#[tokio::test]
async fn new_base_stats_and_gear_are_scaled_by_the_auras_on_and_pools_clamp_to_new_maxima() {
    let spells = [
        spell(INT_UP_20, &[(AURA_MOD_PERCENT_STAT, 20, 3)]),
        spell(ALL_PLUS_5, &[(AURA_MOD_STAT, 31, 3)]),
    ];
    let mut d = data(&spells);
    d.player_level_stats
        .insert((1, 8, 2), [35, 41, 51, 150, 126]);
    d.player_class_stats.insert((8, 2), (1397, 1420));
    let (realm, _a, g) = player(d, Race::Human, Class::Mage).await;
    let got = on_map(&realm, move |m| {
        let data = m.realm.data.clone();
        put(m, g, INT_UP_20);
        let before = stats(m, g)[3];
        // A level's new base, and gear, under the percentage.
        let p = m.players.get_mut(&g).unwrap();
        crate::game::map::stats::set_level(p, &data, 2);
        derive::unit(m, g);
        equip(m, g, INTELLECT_35, 0);
        let changed = stats(m, g)[3];
        take(m, g, INT_UP_20);
        let without = stats(m, g)[3];
        // The flat 31 through repeated recomputes.
        put(m, g, ALL_PLUS_5);
        derive::unit(m, g);
        derive::unit(m, g);
        let flat = (stats(m, g), pools(m, g));
        // Health and mana above their maxima come down to them.
        let p = m.players.get_mut(&g).unwrap();
        p.fields.set_u32(UNIT_FIELD_HEALTH, 99_999);
        p.fields.set_u32(UNIT_FIELD_POWER1, 99_999);
        derive::unit(m, g);
        let f = &m.players[&g].fields;
        let current = (f.get_u32(UNIT_FIELD_HEALTH), f.get_u32(UNIT_FIELD_POWER1));
        (before, changed, without, flat, current)
    })
    .await;
    let (before, changed, without, (flat_stats, flat_pools), current) = got;
    assert_eq!(before, 150);
    // (150 + 35) * 1.2 = 222; Thistle Tea's 150 + 50 at 20% is 240.
    assert_eq!(changed, 222);
    assert_eq!(without, 185);
    assert_eq!(flat_stats[3], 185 + 31);
    assert_eq!(
        flat_pools,
        (1397 + 20 + (51 - 20) * 10, 1420 + 20 + (185 + 31 - 20) * 15)
    );
    assert_eq!(current, flat_pools);
}

const RES_MIX: u32 = 962_201;
const FIRE_WEAK: u32 = 962_202;
const FIRE_STRONG: u32 = 962_203;
const FIRE_STRONG_2: u32 = 962_204;
const ARMOR_STACKED: u32 = 962_205;
const ARMOR_OTHER: u32 = 962_206;
const ARMOR_PCT: u32 = 962_207;
const ARCANE_RESILIENCE: u32 = 962_208;

/// The resistances, armor first.
fn resistances(m: &Map, g: Guid) -> [u32; 7] {
    let f = &m.players[&g].fields;
    std::array::from_fn(|i| f.get_u32(UNIT_FIELD_RESISTANCES + i as u16))
}

#[tokio::test]
async fn exclusive_resistances_take_the_best_bonus_and_worst_penalty_of_each_school() {
    let spells = [
        spell(
            RES_MIX,
            &[
                (AURA_MOD_RESISTANCE_EXCLUSIVE, 20, 126),
                (AURA_MOD_RESISTANCE_EXCLUSIVE, 60, 4),
                (AURA_MOD_RESISTANCE_EXCLUSIVE, 40, 20),
            ],
        ),
        spell(
            RES_MIX + 50,
            &[
                (AURA_MOD_RESISTANCE_EXCLUSIVE, -10, 126),
                (AURA_MOD_RESISTANCE_EXCLUSIVE, -25, 16),
                (AURA_MOD_RESISTANCE, 8, 126),
            ],
        ),
        spell(RES_MIX + 51, &[(AURA_MOD_RESISTANCE, 7, 126)]),
        spell(FIRE_WEAK, &[(AURA_MOD_RESISTANCE_EXCLUSIVE, 20, 4)]),
        spell(FIRE_STRONG, &[(AURA_MOD_RESISTANCE_EXCLUSIVE, 60, 4)]),
        spell(FIRE_STRONG_2, &[(AURA_MOD_RESISTANCE_EXCLUSIVE, 60, 4)]),
        spell(ARMOR_STACKED, &[(AURA_MOD_RESISTANCE_EXCLUSIVE, 20, 1)]),
        spell(ARMOR_OTHER, &[(AURA_MOD_RESISTANCE_EXCLUSIVE, 50, 1)]),
        spell(ARMOR_PCT, &[(AURA_MOD_RESISTANCE_PCT, 50, 1)]),
        // Arcane Resilience: half the intellect as armor.
        spell(ARCANE_RESILIENCE, &[(182, 50, 1)]),
    ];
    let (realm, _a, g) = player(data(&spells), Race::Human, Class::Mage).await;
    let got = on_map(&realm, move |m| {
        // The robe: 100 armor, 10 fire.
        equip(m, g, ARMOR_100, 4);
        let base = resistances(m, g);
        put(m, g, RES_MIX);
        put(m, g, RES_MIX + 50);
        put(m, g, RES_MIX + 51);
        let mixed = resistances(m, g);
        take(m, g, RES_MIX);
        take(m, g, RES_MIX + 50);
        take(m, g, RES_MIX + 51);
        // The stronger goes, the weaker counts again, in any order.
        let mut fallbacks = Vec::new();
        for order in [
            vec![FIRE_WEAK, FIRE_STRONG],
            vec![FIRE_STRONG, FIRE_WEAK],
            vec![FIRE_WEAK, FIRE_STRONG, FIRE_STRONG_2],
        ] {
            for id in &order {
                put(m, g, *id);
            }
            let active = resistances(m, g)[2];
            for id in order.iter().filter(|id| **id != FIRE_WEAK) {
                take(m, g, *id);
            }
            let weak = resistances(m, g)[2];
            take(m, g, FIRE_WEAK);
            fallbacks.push((active, weak, resistances(m, g)[2]));
        }
        // Three stacks of 20 beat a single 50, before the multiplier.
        put(m, g, ARMOR_STACKED);
        stacks(m, g, ARMOR_STACKED, 3);
        put(m, g, ARMOR_OTHER);
        put(m, g, ARMOR_PCT);
        let stacked = resistances(m, g)[0];
        take(m, g, ARMOR_STACKED);
        take(m, g, ARMOR_OTHER);
        take(m, g, ARMOR_PCT);
        put(m, g, ARCANE_RESILIENCE);
        let resilient = resistances(m, g)[0];
        (base, mixed, fallbacks, stacked, resilient)
    })
    .await;
    let (base, mixed, fallbacks, stacked, resilient) = got;
    // A mage's armor: the robe's and twice the agility.
    let agility = MAGE_STATS[1];
    assert_eq!(base[0], 100 + agility * 2);
    assert_eq!(base[2], 10);
    // Fire: 10 + 60 + 15 flats - 10; frost 40 - 25 + 15; the others 20 -
    // 10 + 15; armor isn't in mask 126, so only its own.
    assert_eq!(mixed[2], 10 + 60 - 10 + 15);
    assert_eq!(mixed[4], 40 - 25 + 15);
    for school in [1, 3, 5, 6] {
        assert_eq!(mixed[school], 20 - 10 + 15, "school {school}");
    }
    assert_eq!(mixed[0], base[0]);
    for f in fallbacks {
        assert_eq!(f, (70, 30, 10));
    }
    assert_eq!(stacked, ((base[0] + 60) as f64 * 1.5) as u32);
    assert_eq!(resilient, base[0] + MAGE_STATS[3] * 50 / 100);
}

const BATTLE_SHOUT: u32 = 962_301;
const STRENGTH_30: u32 = 962_302;
const AGILITY_20: u32 = 962_303;
const PREDATORY_STRIKES: u32 = 16_975;

#[tokio::test]
async fn attack_power_from_strength_and_auras_goes_into_the_weapons_damage() {
    let spells = [
        spell(BATTLE_SHOUT, &[(AURA_MOD_ATTACK_POWER, 60, 0)]),
        spell(STRENGTH_30, &[(AURA_MOD_STAT, 30, 0)]),
        spell(AGILITY_20, &[(AURA_MOD_STAT, 20, 1)]),
    ];
    let mut d = data(&spells);
    // A warrior of 145 strength.
    d.player_level_stats
        .insert((1, 1, 1), [145, 41, 45, 125, 126]);
    let (realm, _a, g) = player(d, Race::Human, Class::Warrior).await;
    let warrior = on_map(&realm, move |m| {
        equip(m, g, GREATSWORD, 15);
        let read = |m: &Map| {
            let f = &m.players[&g].fields;
            (
                m.players[&g].derived.attack_power,
                f.get_f32(UNIT_FIELD_MINDAMAGE),
                f.get_f32(UNIT_FIELD_MAXDAMAGE),
            )
        };
        let plain = read(m);
        put(m, g, BATTLE_SHOUT);
        let shout = read(m);
        take(m, g, BATTLE_SHOUT);
        put(m, g, STRENGTH_30);
        (plain, shout, read(m))
    })
    .await;
    let (plain, shout, strong) = warrior;
    // Level 1: 1 * 3 + 145 * 2 - 20.
    assert_eq!(plain.0, 3 + 290 - 20);
    assert_eq!(shout.0, plain.0 + 60);
    let near = |a: f32, b: f32| (a - b).abs() < 0.01;
    assert!(
        near(shout.1, 100.0 + shout.0 as f32 / 14.0 * 3.4),
        "{shout:?}"
    );
    assert!(
        near(shout.2, 150.0 + shout.0 as f32 / 14.0 * 3.4),
        "{shout:?}"
    );
    assert_eq!(strong.0, plain.0 + 60);
    assert!(strong.1 > plain.1 && strong.2 > plain.2);

    // A hunter's agility: two ranged attack power a point, into the bow.
    let (realm, _b, g) = player(data(&spells), Race::Dwarf, Class::Hunter).await;
    let hunter = on_map(&realm, move |m| {
        equip(m, g, BOW, 17);
        let read = |m: &Map| {
            let f = &m.players[&g].fields;
            (
                m.players[&g].derived.ranged_attack_power,
                f.get_f32(UNIT_FIELD_MINRANGEDDAMAGE),
                f.get_f32(UNIT_FIELD_MAXRANGEDDAMAGE),
            )
        };
        let plain = read(m);
        put(m, g, AGILITY_20);
        (plain, read(m))
    })
    .await;
    let (plain, agile) = hunter;
    assert_eq!(agile.0, plain.0 + 40);
    assert!(
        near(agile.1 - plain.1, 40.0 / 14.0 * 2.5),
        "{plain:?} {agile:?}"
    );
    assert!(
        near(agile.2 - plain.2, 40.0 / 14.0 * 2.5),
        "{plain:?} {agile:?}"
    );
}

#[tokio::test]
async fn predatory_strikes_adds_its_share_of_the_level_in_cat_form_only() {
    let mut talent = spell(PREDATORY_STRIKES, &[(AURA_DUMMY, 150, 0)]);
    talent.attributes[0] = 0x40;
    let mut d = data(&[talent]);
    d.player_level_stats.insert((4, 11, 60), MAGE_STATS);
    d.player_class_stats.insert((11, 60), MAGE_POOLS);
    let (realm, _a, g) = player(d, Race::NightElf, Class::Druid).await;
    let got = on_map(&realm, move |m| {
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        crate::game::map::stats::set_level(p, &data, 60);
        let form = |m: &mut Map, f: u8| {
            let p = m.players.get_mut(&g).unwrap();
            p.fields.set_byte(UNIT_FIELD_BYTES_1, 2, f);
            derive::unit(m, g);
            m.players[&g].derived.attack_power
        };
        let caster_plain = form(m, 0);
        let cat_plain = form(m, FORM_CAT);
        put(m, g, PREDATORY_STRIKES);
        let cat = form(m, FORM_CAT);
        let caster = form(m, 0);
        (caster_plain, cat_plain, cat, caster)
    })
    .await;
    let (caster_plain, cat_plain, cat, caster) = got;
    assert_eq!(cat, cat_plain + 60 * 150 / 100);
    assert_eq!(caster, caster_plain);
}
