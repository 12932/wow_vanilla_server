//! Phase 3 against Thistle Tea's stat tests (`test/game/core/stats`): the
//! chances from agility, parrying and blocking, attack periods under haste
//! and slows, armor and resistances, run speed. Through a player's fields
//! where the server derives them, as unit tests where they are formulas.

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
use crate::game::map::derive::{self, SPELL_BLOCK, SPELL_DUAL_WIELD, SPELL_PARRY};
use crate::game::map::player::Item;
use crate::game::map::{Map, MapKey, stats};
use crate::game::store::BAG_PLAYER;

const SWORD: u32 = 961_001;
const SHORT_SWORD: u32 = 961_002;
const BOW: u32 = 961_003;
const WAND: u32 = 961_004;
const SHIELD: u32 = 961_005;

const WARRIOR: u8 = 1;
const ROGUE: u8 = 4;
const MAGE: u8 = 8;

fn weapon(
    entry: u32,
    subclass: u32,
    inventory_type: u32,
    delay: u32,
    ammo_type: u32,
) -> ItemTemplate {
    let mut damages = [(0.0, 0.0, 0); 5];
    damages[0] = (10.0, 20.0, 0);
    ItemTemplate {
        entry,
        class: 2,
        subclass,
        name: format!("Weapon {entry}"),
        display_id: 1000,
        inventory_type,
        delay,
        ammo_type,
        damages,
        stackable: 1,
        ..Default::default()
    }
}

/// An aura spell of one effect on its caster; passive if it says so.
fn aura(id: u32, kind: u32, amount: i32, misc: i32, passive: bool) -> SpellTemplate {
    let mut effects = [SpellEffect::default(); 3];
    effects[0] = SpellEffect {
        effect: 6,
        aura: kind,
        base_points: amount - 1,
        base_dice: 1,
        misc_value: misc,
        target_a: 1,
        ..Default::default()
    };
    let mut s = SpellTemplate {
        id,
        name: format!("Aura {id}"),
        effects,
        equipped_item_class: -1,
        ..Default::default()
    };
    if passive {
        s.attributes[0] = 0x40;
    }
    s
}

fn data() -> GameData {
    let mut d = GameData::empty();
    for t in [
        weapon(SWORD, 7, 13, 2000, 0),
        weapon(SHORT_SWORD, 7, 13, 1500, 0),
        weapon(BOW, 2, 15, 3000, 2),
        weapon(WAND, 19, 26, 3000, 0),
    ] {
        d.items.insert(t.entry, t);
    }
    d.items.insert(
        SHIELD,
        ItemTemplate {
            entry: SHIELD,
            class: 4,
            subclass: 6,
            name: "Shield".into(),
            display_id: 1000,
            inventory_type: 14,
            block: 20,
            stackable: 1,
            ..Default::default()
        },
    );
    d
}

/// Equip a new item of `entry` in `slot`, and derive the stats again.
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

fn unequip(m: &mut Map, g: Guid, slot: u8) {
    let data = m.realm.data.clone();
    let p = m.players.get_mut(&g).expect("the player");
    p.items.retain(|i| !(i.bag == BAG_PLAYER && i.slot == slot));
    p.refresh_inventory(&data);
    derive::unit(m, g);
}

/// Run `f` on the map the first player is on.
async fn on_map<T: Send + 'static>(
    realm: &Arc<Realm>,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    let g = guid::player(1);
    let key = realm
        .registry
        .online(g)
        .map_or(MapKey::world(0), |o| o.map().key);
    realm
        .maps
        .get(realm, key)
        .query(f)
        .await
        .expect("the map answers")
}

/// Alice, of `class`: a human, or a night elf druid.
async fn player(d: GameData, class: Class) -> (Arc<Realm>, Client, Guid) {
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let race = if class == Class::Druid {
        Race::NightElf
    } else {
        Race::Human
    };
    let g = a.enter("Alice", race, class).await;
    assert_eq!(g, guid::player(1));
    (realm, a, g)
}

/// Put an aura spell of the data on `g`, or take it off again.
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

#[test]
fn melee_crit_and_dodge_follow_the_class_rates_and_non_casters_have_no_spell_crit() {
    // A level 60 warrior: twenty agility a percent; a mage adds its base.
    let crit = |class, level, agility: f32| {
        derive::class_base_bonus(class) + agility / derive::agility_rate(class, level, false)
    };
    assert!((crit(WARRIOR, 60, 100.0) - 5.0).abs() < 0.001);
    assert!((crit(MAGE, 60, 20.0) - 4.2).abs() < 0.001);
    // A rogue dodges a percent for 14.5; no base and no agility, none.
    let dodge = |class, level, agility: f32| {
        derive::class_base_bonus(class) + agility / derive::agility_rate(class, level, true)
    };
    assert!((dodge(ROGUE, 60, 145.0) - 10.0).abs() < 0.001);
    assert_eq!(dodge(WARRIOR, 30, 0.0), 0.0);
    assert_eq!(derive::spell_crit_chance(WARRIOR, 60, 100), 0.0);
    assert!((derive::spell_crit_chance(MAGE, 60, 100) - 5.559).abs() < 0.001);
}

#[tokio::test]
async fn melee_crit_takes_the_rate_vmangos_lists_for_the_class_and_level() {
    let mut d = data();
    d.misc.crit_per_agility.insert((1, 1), 4.0);
    d.misc.crit_per_agility.insert((1, 30), 10.395);
    let (realm, _a, g) = player(d, Class::Warrior).await;
    let got = on_map(&realm, move |m| {
        let data = m.realm.data.clone();
        let read = |m: &Map| {
            let p = &m.players[&g];
            (
                p.fields.get_f32(PLAYER_CRIT_PERCENTAGE),
                p.fields.get_u32(UNIT_FIELD_STAT0 + 1) as f32,
                p.derived.weapon_skill as f32,
            )
        };
        let one = read(m);
        // Level 45 takes level 30's, the last listed up to it.
        let p = m.players.get_mut(&g).unwrap();
        stats::set_level(p, &data, 45);
        derive::unit(m, g);
        (one, read(m))
    })
    .await;
    let ((crit1, agility1, _), (crit45, agility45, skill45)) = got;
    assert!((crit1 - agility1 / 4.0).abs() < 0.01, "{crit1} {agility1}");
    // At 45, what the unarmed skill falls short of 225 is off it.
    let skill_off = (skill45 - 225.0) * 0.04;
    assert!(
        (crit45 - (agility45 / 10.395 + skill_off).max(0.0)).abs() < 0.01,
        "{crit45} {agility45}"
    );
}

#[tokio::test]
async fn parry_wants_it_learned_and_a_weapon_and_block_value_is_the_shields_and_strength_over_twenty()
 {
    let (realm, _a, g) = player(data(), Class::Warrior).await;
    let got = on_map(&realm, move |m| {
        let parry = |m: &Map| m.players[&g].fields.get_f32(PLAYER_PARRY_PERCENTAGE);
        let mut out = Vec::new();
        // Not learned: none, with a sword or without.
        m.players
            .get_mut(&g)
            .unwrap()
            .spells
            .retain(|s| *s != SPELL_PARRY && *s != SPELL_BLOCK);
        equip(m, g, SWORD, 15);
        out.push(parry(m));
        m.players.get_mut(&g).unwrap().spells.push(SPELL_PARRY);
        derive::unit(m, g);
        out.push(parry(m));
        unequip(m, g, 15);
        out.push(parry(m));
        // An off-hand weapon will do.
        equip(m, g, SHORT_SWORD, 16);
        out.push(parry(m));
        unequip(m, g, 16);
        // Blocking: a shield and the spell.
        equip(m, g, SHIELD, 16);
        let block = |m: &Map| {
            let p = &m.players[&g];
            (
                p.fields.get_f32(PLAYER_BLOCK_PERCENTAGE),
                p.derived.block_value,
                p.fields.get_u32(UNIT_FIELD_STAT0),
            )
        };
        let unlearned = block(m);
        m.players.get_mut(&g).unwrap().spells.push(SPELL_BLOCK);
        derive::unit(m, g);
        (out, unlearned, block(m))
    })
    .await;
    let (parry, unlearned, (chance, value, strength)) = got;
    assert_eq!(parry, vec![0.0, 5.0, 0.0, 5.0]);
    assert_eq!((unlearned.0, unlearned.1), (0.0, 0));
    assert_eq!(chance, 5.0);
    assert_eq!(value, (20.0 + strength as f32 / 20.0 - 1.0) as u32);
    // A class with no parry of its own: none even armed.
    let (realm, _b, g) = player(data(), Class::Mage).await;
    let mage = on_map(&realm, move |m| {
        equip(m, g, SWORD, 15);
        m.players[&g].fields.get_f32(PLAYER_PARRY_PERCENTAGE)
    })
    .await;
    assert_eq!(mage, 0.0);
}

const BARKSKIN: u32 = 961_101;
const HASTE_20: u32 = 961_102;
const HASTE_30: u32 = 961_103;
const SLOW_10: u32 = 961_104;
const SLOW_20: u32 = 961_105;
const ATTACK_SLOW: u32 = 961_106;
const RANGED_SLOW: u32 = 961_107;
const SPEED_50: u32 = 961_108;
const AMMO_15: u32 = 961_109;
const RANGED_25: u32 = 961_110;
const HASTE_100: u32 = 961_111;

/// A slow beside another aura, as Thunder Clap's is beside its damage: not
/// one of the slows that replace each other (`Exclusive::NegativeHaste`),
/// so they stand together and the strongest counts.
fn slow(id: u32, amount: i32) -> SpellTemplate {
    let mut s = aura(id, AURA_MOD_HASTE, amount, 0, false);
    s.effects[1] = SpellEffect {
        aura: AURA_DUMMY,
        ..s.effects[0]
    };
    s
}

fn with_hastes(mut d: GameData) -> GameData {
    for s in [
        aura(BARKSKIN, AURA_MOD_HASTE, -25, 0, true),
        aura(HASTE_20, AURA_MOD_HASTE, 20, 0, false),
        aura(HASTE_30, AURA_MOD_HASTE, 30, 0, false),
        slow(SLOW_10, -10),
        slow(SLOW_20, -20),
        aura(ATTACK_SLOW, AURA_MOD_ATTACKSPEED, -20, 0, false),
        aura(RANGED_SLOW, AURA_MOD_RANGED_HASTE, -20, 0, false),
        aura(SPEED_50, AURA_MOD_ATTACKSPEED, 50, 0, false),
        aura(AMMO_15, AURA_MOD_RANGED_AMMO_HASTE, 15, 0, false),
        aura(RANGED_25, AURA_MOD_RANGED_HASTE, 25, 0, false),
        aura(HASTE_100, AURA_MOD_HASTE, 100, 0, false),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

/// The three periods: main hand, off hand, ranged.
fn periods(m: &Map, g: Guid) -> (u32, u32, u32) {
    let f = &m.players[&g].fields;
    (
        f.get_u32(UNIT_FIELD_BASEATTACKTIME),
        f.get_u32(UNIT_FIELD_BASEATTACKTIME + 1),
        f.get_u32(UNIT_FIELD_RANGEDATTACKTIME),
    )
}

#[tokio::test]
async fn hastes_and_slows_multiply_on_the_periods_but_of_the_melee_slows_only_the_strongest() {
    let (realm, _a, g) = player(with_hastes(data()), Class::Warrior).await;
    let got = on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().spells.push(SPELL_DUAL_WIELD);
        equip(m, g, SWORD, 15);
        equip(m, g, SHORT_SWORD, 16);
        equip(m, g, BOW, 17);
        let mut out = vec![periods(m, g)];
        let (min, max) = {
            let f = &m.players[&g].fields;
            (
                f.get_f32(UNIT_FIELD_MINDAMAGE),
                f.get_f32(UNIT_FIELD_MAXDAMAGE),
            )
        };
        // A passive slow (Barkskin's) on both hands, not the bow.
        put(m, g, BARKSKIN);
        out.push(periods(m, g));
        take(m, g, BARKSKIN);
        out.push(periods(m, g));
        // Two hastes multiply.
        put(m, g, HASTE_20);
        put(m, g, HASTE_30);
        out.push(periods(m, g));
        take(m, g, HASTE_20);
        take(m, g, HASTE_30);
        // Of two slows, the stronger, beside the passive one.
        put(m, g, SLOW_10);
        put(m, g, SLOW_20);
        put(m, g, BARKSKIN);
        out.push(periods(m, g));
        take(m, g, SLOW_20);
        out.push(periods(m, g));
        take(m, g, SLOW_10);
        take(m, g, BARKSKIN);
        // Attack speed slows the bow too; a ranged slow only the bow.
        put(m, g, ATTACK_SLOW);
        out.push(periods(m, g));
        take(m, g, ATTACK_SLOW);
        put(m, g, RANGED_SLOW);
        out.push(periods(m, g));
        take(m, g, RANGED_SLOW);
        // Haste leaves the damage as it was.
        put(m, g, SPEED_50);
        let hasted = {
            let f = &m.players[&g].fields;
            (
                f.get_f32(UNIT_FIELD_MINDAMAGE),
                f.get_f32(UNIT_FIELD_MAXDAMAGE),
            )
        };
        take(m, g, SPEED_50);
        // Ammunition haste is the bow's, not a wand's; a wand takes a
        // ranged haste.
        put(m, g, AMMO_15);
        out.push(periods(m, g));
        equip(m, g, WAND, 17);
        out.push(periods(m, g));
        put(m, g, RANGED_25);
        out.push(periods(m, g));
        ((min, max), hasted, out)
    })
    .await;
    let (damage, hasted, periods) = got;
    assert_eq!(damage, hasted);
    assert_eq!(
        periods,
        vec![
            (2000, 1500, 3000),
            (2500, 1875, 3000),
            (2000, 1500, 3000),
            (1282, 961, 3000),
            (3000, 2250, 3000),
            (2750, 2062, 3000),
            (2400, 1800, 3600),
            (2000, 1500, 3600),
            (2000, 1500, 2608),
            (2000, 1500, 3000),
            (2000, 1500, 2400),
        ]
    );
}

#[tokio::test]
async fn a_druids_forms_fight_at_their_own_periods_before_haste() {
    let (realm, _a, g) = player(with_hastes(data()), Class::Druid).await;
    let got = on_map(&realm, move |m| {
        put(m, g, HASTE_100);
        let mut out = Vec::new();
        for form in [FORM_CAT, FORM_BEAR, FORM_DIREBEAR] {
            m.players
                .get_mut(&g)
                .unwrap()
                .fields
                .set_byte(UNIT_FIELD_BYTES_1, 2, form);
            derive::unit(m, g);
            out.push(periods(m, g).0);
        }
        out
    })
    .await;
    assert_eq!(got, vec![500, 1250, 1250]);
}

const SPRINT: u32 = 961_201;
const CRIPPLE: u32 = 961_202;
const SNARE_50: u32 = 961_203;
const SWIM_100: u32 = 961_204;
const SPEED_100: u32 = 961_205;
const NORMAL_SPEED: u32 = 961_206;
const MOUNT_60: u32 = 961_207;
const ALWAYS_10: u32 = 961_208;
const ALWAYS_10_B: u32 = 961_209;
const NOT_STACK_15: u32 = 961_210;

fn with_speeds(mut d: GameData) -> GameData {
    for s in [
        aura(SPRINT, AURA_MOD_INCREASE_SPEED, 70, 0, false),
        aura(CRIPPLE, AURA_MOD_DECREASE_SPEED, -70, 0, false),
        aura(SNARE_50, AURA_MOD_DECREASE_SPEED, -50, 0, false),
        aura(SWIM_100, AURA_MOD_INCREASE_SWIM_SPEED, 100, 0, false),
        aura(SPEED_100, AURA_MOD_INCREASE_SPEED, 100, 0, false),
        aura(NORMAL_SPEED, 191, 7, 0, false),
        aura(MOUNT_60, AURA_MOD_INCREASE_MOUNTED_SPEED, 60, 0, false),
        aura(ALWAYS_10, AURA_MOD_SPEED_ALWAYS, 10, 0, false),
        aura(ALWAYS_10_B, AURA_MOD_SPEED_ALWAYS, 10, 0, false),
        aura(NOT_STACK_15, AURA_MOD_SPEED_NOT_STACK, 15, 0, false),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

#[tokio::test]
async fn run_speed_multiplies_its_best_bonus_by_the_worst_slow_capped_by_a_normal_speed_and_a_rate()
{
    let (realm, _a, g) = player(with_speeds(data()), Class::Rogue).await;
    let got = on_map(&realm, move |m| {
        let speeds = |m: &Map| {
            let s = m.players[&g].speeds;
            (
                (s.run * 100.0).round() / 100.0,
                (s.swim * 100.0).round() / 100.0,
            )
        };
        let mut out = Vec::new();
        // Sprint under Crippling Poison: 7 * 1.7 * 0.3, not 7.
        put(m, g, SPRINT);
        put(m, g, CRIPPLE);
        out.push(speeds(m));
        take(m, g, SPRINT);
        take(m, g, CRIPPLE);
        // A normal speed of 7 caps the bonuses, then the slow halves.
        for s in [NORMAL_SPEED, SPEED_100, SWIM_100, SNARE_50] {
            put(m, g, s);
        }
        out.push(speeds(m));
        for s in [NORMAL_SPEED, SPEED_100, SWIM_100, SNARE_50] {
            take(m, g, s);
        }
        out.push(speeds(m));
        // Stacking bonuses multiply, and the better of them and the
        // non-stacking one counts.
        for s in [ALWAYS_10, ALWAYS_10_B, NOT_STACK_15] {
            put(m, g, s);
        }
        out.push(speeds(m));
        for s in [ALWAYS_10, ALWAYS_10_B, NOT_STACK_15] {
            take(m, g, s);
        }
        // Mounted, a mount's bonus, not Sprint's.
        m.players
            .get_mut(&g)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_MOUNTDISPLAYID, 2402);
        put(m, g, MOUNT_60);
        put(m, g, SPRINT);
        out.push(speeds(m));
        take(m, g, MOUNT_60);
        take(m, g, SPRINT);
        m.players
            .get_mut(&g)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_MOUNTDISPLAYID, 0);
        // A rate of five (`.speed`): the base, and a snare on top.
        m.players.get_mut(&g).unwrap().speed_rate = 5.0;
        derive::unit(m, g);
        out.push(speeds(m));
        put(m, g, SNARE_50);
        out.push(speeds(m));
        take(m, g, SNARE_50);
        out.push(speeds(m));
        out
    })
    .await;
    assert_eq!(
        got,
        vec![
            (3.57, 1.42),
            (3.5, 3.5),
            (7.0, 4.72),
            (8.47, 4.72),
            (11.2, 4.72),
            (35.0, 4.72),
            (17.5, 2.36),
            (35.0, 4.72),
        ]
    );
}

const CHEST: u32 = 961_301;
const ROBE: u32 = 961_302;
const CLOAK: u32 = 961_303;
const BARK: u32 = 961_311;
const BASE_FIRE_FROST: u32 = 961_312;
const BASE_FIRE_20: u32 = 961_313;
const BASE_FIRE_50: u32 = 961_314;
const FIRE_40: u32 = 961_315;
const FIRE_LESS_20: u32 = 961_316;
const FIRE_LESS_25: u32 = 961_317;
const ARMOR_10: u32 = 961_318;
const ARMOR_OF_INTELLECT: u32 = 961_319;
const ARMOR_30: u32 = 961_320;
const ARMOR_50: u32 = 961_321;
const FIRE_20_STACKING: u32 = 961_322;
const FIRE_MINUS_200: u32 = 961_323;
const FIRE_MINUS_150: u32 = 961_324;

const ARMOR: i32 = 1;
const FIRE: i32 = 4;
const FROST: i32 = 16;

fn with_resistances(mut d: GameData) -> GameData {
    let piece = |entry, armor: i32, resistances: [i32; 6]| ItemTemplate {
        entry,
        class: 4,
        subclass: 1,
        name: format!("Armor {entry}"),
        display_id: 1000,
        inventory_type: 5,
        armor,
        resistances,
        stackable: 1,
        ..Default::default()
    };
    d.items.insert(CHEST, piece(CHEST, 100, [0; 6]));
    d.items.insert(ROBE, piece(ROBE, 0, [0, 150, 0, 25, 0, 0]));
    d.items
        .insert(CLOAK, piece(CLOAK, 101, [0, 100, 0, 0, 0, 0]));
    for s in [
        aura(BARK, AURA_MOD_BASE_RESISTANCE_PCT, 180, ARMOR, false),
        aura(
            BASE_FIRE_FROST,
            AURA_MOD_BASE_RESISTANCE,
            50,
            FIRE | FROST,
            false,
        ),
        aura(BASE_FIRE_20, AURA_MOD_BASE_RESISTANCE_PCT, 20, FIRE, false),
        aura(BASE_FIRE_50, AURA_MOD_BASE_RESISTANCE_PCT, 50, FIRE, false),
        aura(FIRE_40, AURA_MOD_RESISTANCE, 40, FIRE, false),
        aura(FIRE_LESS_20, AURA_MOD_RESISTANCE_PCT, -20, FIRE, false),
        aura(FIRE_LESS_25, AURA_MOD_RESISTANCE_PCT, -25, FIRE, false),
        aura(ARMOR_10, AURA_MOD_BASE_RESISTANCE_PCT, 10, ARMOR, false),
        aura(ARMOR_OF_INTELLECT, 182, 50, ARMOR, false),
        aura(ARMOR_30, AURA_MOD_RESISTANCE, 30, ARMOR, false),
        aura(ARMOR_50, AURA_MOD_RESISTANCE_PCT, 50, ARMOR, false),
        aura(FIRE_20_STACKING, AURA_MOD_RESISTANCE_PCT, 20, FIRE, false),
        aura(FIRE_MINUS_200, AURA_MOD_RESISTANCE, -200, FIRE, false),
        aura(FIRE_MINUS_150, AURA_MOD_RESISTANCE_PCT, -150, FIRE, false),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

/// Armor and the schools, as the fields show them.
fn resistance(m: &Map, g: Guid, school: u16) -> u32 {
    m.players[&g]
        .fields
        .get_u32(UNIT_FIELD_RESISTANCES + school)
}

#[tokio::test]
async fn armor_and_resistances_scale_base_flats_before_bonuses_multiply_percentages_and_go_no_lower_than_none()
 {
    let (realm, _a, g) = player(with_resistances(data()), Class::Warrior).await;
    let got = on_map(&realm, move |m| {
        let f = |m: &Map, i: u16| m.players[&g].fields.get_u32(UNIT_FIELD_STAT0 + i) as f32;
        let mut out = Vec::new();
        // Agility's armor is outside the base percentage (bear form's).
        equip(m, g, CHEST, 4);
        let agility = f(m, 1);
        put(m, g, BARK);
        out.push((resistance(m, g, 0) as f32, 100.0 * 2.8 + agility * 2.0));
        take(m, g, BARK);
        out.push((resistance(m, g, 0) as f32, 100.0 + agility * 2.0));
        unequip(m, g, 4);
        // The gear's and the base flats scaled, then the flat bonus, then
        // the percentages each on its own.
        equip(m, g, ROBE, 4);
        for s in [
            BASE_FIRE_FROST,
            BASE_FIRE_20,
            BASE_FIRE_50,
            FIRE_40,
            FIRE_LESS_20,
            FIRE_LESS_25,
        ] {
            put(m, g, s);
        }
        out.push((resistance(m, g, 2) as f32, 240.0));
        out.push((resistance(m, g, 4) as f32, 75.0));
        out.push((resistance(m, g, 5) as f32, 0.0));
        for s in [
            BASE_FIRE_FROST,
            BASE_FIRE_20,
            BASE_FIRE_50,
            FIRE_40,
            FIRE_LESS_20,
            FIRE_LESS_25,
        ] {
            take(m, g, s);
        }
        out.push((resistance(m, g, 2) as f32, 150.0));
        unequip(m, g, 4);
        // Fractions are kept until the end: 101 armor, a tenth more, the
        // agility's, half the intellect, thirty, and half again.
        equip(m, g, CLOAK, 4);
        for s in [ARMOR_10, ARMOR_OF_INTELLECT, ARMOR_30, ARMOR_50] {
            put(m, g, s);
        }
        let (agility, intellect) = (f(m, 1), f(m, 3));
        let (agility, intellect) = (f64::from(agility), f64::from(intellect));
        let expected =
            ((101.0 * 1.1 + agility * 2.0 + intellect * 0.5 + 30.0) * 1.5f64).trunc() as f32;
        out.push((resistance(m, g, 0) as f32, expected));
        // Three stacks of a fifth beside a half: 100 * 1.6 * 1.5; two.
        for s in [ARMOR_10, ARMOR_OF_INTELLECT, ARMOR_30, ARMOR_50] {
            take(m, g, s);
        }
        put(m, g, FIRE_20_STACKING);
        let separate = aura(961_330, AURA_MOD_RESISTANCE_PCT, 50, FIRE, false);
        let mut stacked = Vec::new();
        for stacks in [3, 2] {
            m.players
                .get_mut(&g)
                .unwrap()
                .auras
                .holders
                .iter_mut()
                .filter(|h| h.spell == FIRE_20_STACKING)
                .for_each(|h| h.stacks = stacks);
            apply_spell(m, g, g, &separate, &[(0, 50)], Context::default()).unwrap();
            derive::unit(m, g);
            stacked.push(resistance(m, g, 2));
        }
        take(m, g, FIRE_20_STACKING);
        take(m, g, 961_330);
        // A debuff takes it to none, never below.
        put(m, g, FIRE_MINUS_200);
        let floored = resistance(m, g, 2);
        take(m, g, FIRE_MINUS_200);
        put(m, g, FIRE_MINUS_150);
        let suppressed = resistance(m, g, 2);
        (out, stacked, floored, suppressed)
    })
    .await;
    let (pairs, stacked, floored, suppressed) = got;
    for (got, want) in &pairs {
        assert_eq!(got, want, "{pairs:?}");
    }
    assert_eq!(stacked, vec![240, 210]);
    assert_eq!((floored, suppressed), (0, 0));
}

const RANGED_ROBE: u32 = 961_401;
const PIERCING_ROBE: u32 = 961_402;
const BLOCK_TRINKET: u32 = 961_403;
const QUIVER: u32 = 961_404;
const HEALTH_TRINKET: u32 = 961_405;
const CASTER_ROBE: u32 = 961_406;
const STAMINA_CHEST: u32 = 961_407;
const SLAYER_TRINKET: u32 = 961_408;
const SLAYER_ON_USE: u32 = 961_409;
const RAP_40: u32 = 961_501;
const PIERCE_10: u32 = 961_502;
const BLOCK_3: u32 = 961_503;
const QUIVER_15: u32 = 961_504;
const SPELL_DAMAGE: u32 = 961_505;
const VERSUS_UNDEAD: u32 = 961_506;

fn with_gear_spells(mut d: GameData) -> GameData {
    use crate::game::data::items::ItemSpell;
    use crate::game::map::inventory::ITEM_SPELL_ON_EQUIP;
    let item = |entry, class, subclass, inventory_type, spell: u32, trigger: u32| {
        let mut spells = [ItemSpell::default(); 5];
        spells[0] = ItemSpell {
            id: spell,
            trigger,
            ..Default::default()
        };
        ItemTemplate {
            entry,
            class,
            subclass,
            name: format!("Item {entry}"),
            display_id: 1000,
            inventory_type,
            max_durability: 50,
            spells,
            stackable: 1,
            ..Default::default()
        }
    };
    d.items.insert(
        RANGED_ROBE,
        item(RANGED_ROBE, 4, 1, 5, RAP_40, ITEM_SPELL_ON_EQUIP),
    );
    d.items.insert(
        PIERCING_ROBE,
        item(PIERCING_ROBE, 4, 1, 5, PIERCE_10, ITEM_SPELL_ON_EQUIP),
    );
    d.items.insert(
        BLOCK_TRINKET,
        item(BLOCK_TRINKET, 4, 0, 12, BLOCK_3, ITEM_SPELL_ON_EQUIP),
    );
    d.items.insert(
        QUIVER,
        item(QUIVER, 11, 2, 18, QUIVER_15, ITEM_SPELL_ON_EQUIP),
    );
    d.items.insert(
        CASTER_ROBE,
        item(CASTER_ROBE, 4, 1, 20, SPELL_DAMAGE, ITEM_SPELL_ON_EQUIP),
    );
    d.items.insert(
        SLAYER_TRINKET,
        item(SLAYER_TRINKET, 4, 0, 12, VERSUS_UNDEAD, ITEM_SPELL_ON_EQUIP),
    );
    d.items.insert(
        SLAYER_ON_USE,
        item(SLAYER_ON_USE, 4, 0, 12, VERSUS_UNDEAD, 0),
    );
    let mut health = item(HEALTH_TRINKET, 4, 0, 12, 0, 0);
    health.stats[0] = (1, 50);
    health.stats[1] = (0, 30);
    d.items.insert(HEALTH_TRINKET, health);
    let mut stamina = item(STAMINA_CHEST, 4, 1, 5, 0, 0);
    stamina.stats[0] = (7, 5);
    stamina.armor = 120;
    d.items.insert(STAMINA_CHEST, stamina);
    let mut damage = aura(SPELL_DAMAGE, AURA_MOD_DAMAGE_DONE, 20, 126, false);
    damage.effects[1] = SpellEffect {
        aura: AURA_MOD_HEALING_DONE,
        base_points: 31,
        misc_value: 0,
        ..damage.effects[0]
    };
    damage.effects[2] = SpellEffect {
        aura: AURA_MOD_ATTACK_POWER,
        base_points: 8,
        misc_value: 0,
        ..damage.effects[0]
    };
    for s in [
        aura(RAP_40, AURA_MOD_RANGED_ATTACK_POWER, 40, 0, false),
        aura(PIERCE_10, AURA_MOD_TARGET_RESISTANCE, -10, 124, false),
        aura(BLOCK_3, AURA_MOD_BLOCK_PERCENT, 3, 0, false),
        aura(QUIVER_15, AURA_MOD_RANGED_AMMO_HASTE, 15, 0, false),
        aura(VERSUS_UNDEAD, 180, 48, 32, false),
        damage,
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

/// Equip, then bring the gear's spells in line, as a change does.
fn wear(m: &mut Map, g: Guid, entry: u32, slot: u8) {
    equip(m, g, entry, slot);
    crate::game::map::inventory::sync_gear(m, g);
    derive::unit(m, g);
}

fn strip(m: &mut Map, g: Guid, slot: u8) {
    unequip(m, g, slot);
    crate::game::map::inventory::sync_gear(m, g);
    derive::unit(m, g);
}

/// Wear the item in `slot` down to `left`.
fn wear_down(m: &mut Map, g: Guid, slot: u8, left: u32) {
    let p = m.players.get_mut(&g).unwrap();
    let i = p.item_at(BAG_PLAYER, slot).unwrap();
    p.items[i].fields.set_u32(ITEM_FIELD_DURABILITY, left);
    crate::game::map::inventory::sync_gear(m, g);
    derive::unit(m, g);
}

fn holds(m: &Map, g: Guid, spell: u32) -> bool {
    m.players[&g].auras.holders.iter().any(|h| h.spell == spell)
}

#[tokio::test]
async fn gear_gives_its_equip_spells_while_whole_and_worn_and_its_stats_to_whatever_base() {
    let d = with_gear_spells(with_hastes(data()));
    let (realm, _a, g) = player(d, Class::Warrior).await;
    let got = on_map(&realm, move |m| {
        let rap = |m: &Map| m.players[&g].derived.ranged_attack_power;
        let mut out = Vec::new();
        // Ranged attack power from an equip spell: gone broken, back
        // repaired, gone taken off.
        let bare = rap(m);
        wear(m, g, RANGED_ROBE, 4);
        out.push(rap(m) - bare);
        wear_down(m, g, 4, 0);
        out.push(rap(m) - bare);
        wear_down(m, g, 4, 50);
        out.push(rap(m) - bare);
        strip(m, g, 4);
        out.push(rap(m) - bare);
        // Penetration likewise.
        let mut pierce = Vec::new();
        wear(m, g, PIERCING_ROBE, 4);
        pierce.push(holds(m, g, PIERCE_10));
        wear_down(m, g, 4, 0);
        pierce.push(holds(m, g, PIERCE_10));
        wear_down(m, g, 4, 50);
        pierce.push(holds(m, g, PIERCE_10));
        strip(m, g, 4);
        pierce.push(holds(m, g, PIERCE_10));
        // A trinket's block chance, beside a shield's.
        m.players.get_mut(&g).unwrap().spells.push(SPELL_BLOCK);
        wear(m, g, SHIELD, 16);
        let shield = m.players[&g].fields.get_f32(PLAYER_BLOCK_PERCENTAGE);
        wear(m, g, BLOCK_TRINKET, 12);
        let trinket = m.players[&g].fields.get_f32(PLAYER_BLOCK_PERCENTAGE);
        strip(m, g, 12);
        strip(m, g, 16);
        // A quiver's haste on the bow's shots.
        wear(m, g, BOW, 17);
        wear(m, g, QUIVER, 19);
        let shot = periods(m, g).2;
        // Conditional spell power is held, not shown; a use trinket's is
        // not held.
        wear(m, g, SLAYER_TRINKET, 12);
        let versus = (
            holds(m, g, VERSUS_UNDEAD),
            m.players[&g]
                .fields
                .get_u32(PLAYER_FIELD_MOD_DAMAGE_DONE_POS + 5),
        );
        strip(m, g, 12);
        wear(m, g, SLAYER_ON_USE, 12);
        let on_use = holds(m, g, VERSUS_UNDEAD);
        strip(m, g, 12);
        (out, pierce, (shield, trinket), shot, versus, on_use)
    })
    .await;
    let (rap, pierce, block, shot, versus, on_use) = got;
    assert_eq!(rap, vec![40, 0, 40, 0]);
    assert_eq!(pierce, vec![true, false, true, false]);
    assert_eq!(block, (5.0, 8.0));
    assert_eq!(shot, 2608);
    assert_eq!(versus, (true, 0));
    assert!(!on_use);
    // A caster's robe: spell power by school shown, healing and attack
    // power held; a trinket's health and mana; stamina kept over a level.
    let (realm, _b, g) = player(with_gear_spells(data()), Class::Mage).await;
    let got = on_map(&realm, move |m| {
        let f = |m: &Map, field| m.players[&g].fields.get_u32(field);
        let ap = m.players[&g].derived.attack_power;
        wear(m, g, CASTER_ROBE, 4);
        let shown: Vec<u32> = (0..7)
            .map(|i| f(m, PLAYER_FIELD_MOD_DAMAGE_DONE_POS + i))
            .collect();
        let p = &m.players[&g];
        let powers = (
            p.derived.spell_power[2],
            p.derived.healing_power,
            p.derived.attack_power - ap,
        );
        strip(m, g, 4);
        let (health, mana) = (f(m, UNIT_FIELD_MAXHEALTH), f(m, UNIT_FIELD_MAXPOWER1));
        wear(m, g, HEALTH_TRINKET, 12);
        let more = (
            f(m, UNIT_FIELD_MAXHEALTH) - health,
            f(m, UNIT_FIELD_MAXPOWER1) - mana,
        );
        strip(m, g, 12);
        // Stamina's health on, at full; off, the health comes down with
        // the most.
        let stamina = f(m, UNIT_FIELD_STAT0 + 2);
        wear(m, g, STAMINA_CHEST, 4);
        let worn = f(m, UNIT_FIELD_STAT0 + 2) - stamina;
        let full = f(m, UNIT_FIELD_MAXHEALTH);
        m.players
            .get_mut(&g)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_HEALTH, full);
        // A level on: the new base, and the chest's still.
        let data = m.realm.data.clone();
        stats::set_level(m.players.get_mut(&g).unwrap(), &data, 2);
        let base2 = data.level_stats(1, 8, 2).unwrap().stats[2];
        let levelled = f(m, UNIT_FIELD_STAT0 + 2) - base2;
        let armor = f(m, UNIT_FIELD_RESISTANCES);
        strip(m, g, 4);
        let after = (f(m, UNIT_FIELD_MAXHEALTH), f(m, UNIT_FIELD_HEALTH));
        (shown, powers, more, worn, levelled, armor, after)
    })
    .await;
    let (shown, powers, more, worn, levelled, armor, (max, health)) = got;
    assert_eq!(shown, vec![0, 20, 20, 20, 20, 20, 20]);
    assert_eq!(powers, (20, 32, 9));
    assert_eq!(more, (50, 30));
    assert_eq!((worn, levelled), (5, 5));
    assert!(armor >= 120, "{armor}");
    assert_eq!(health, max);
}

#[test]
fn attack_power_and_stamina_health_follow_each_classs_formula_and_never_go_negative() {
    use derive::{melee_attack_power as melee, ranged_attack_power as ranged, stamina_health};
    let (hunter, shaman, priest, druid) = (3, 7, 5, 11);
    assert_eq!(melee(WARRIOR, 10, 30, 20), 10 * 3 + 30 * 2 - 20);
    assert_eq!(melee(ROGUE, 10, 20, 25), 10 * 2 + 20 + 25 - 20);
    assert_eq!(melee(shaman, 10, 20, 15), 10 * 2 + 20 * 2 - 20);
    assert_eq!(melee(druid, 10, 20, 15), 20 * 2 - 20);
    assert_eq!((melee(MAGE, 10, 15, 15), melee(priest, 10, 15, 15)), (5, 5));
    assert_eq!(melee(MAGE, 1, 3, 3), 0);
    assert_eq!(ranged(hunter, 10, 25), 10 * 2 + 25 * 2 - 10);
    assert_eq!((ranged(WARRIOR, 10, 25), ranged(ROGUE, 10, 25)), (25, 25));
    assert_eq!(ranged(MAGE, 10, 25), 15);
    assert_eq!((stamina_health(15), stamina_health(25)), (15, 70));
}

#[tokio::test]
async fn a_level_raises_the_weapon_skills_caps_not_the_armors_and_the_same_level_keeps_its_time() {
    use crate::game::map::skills;
    let (realm, _a, g) = player(data(), Class::Warrior).await;
    let got = on_map(&realm, move |m| {
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        skills::learn_skill(p, 43, 1);
        skills::set_value(p, 43, 1);
        skills::learn_skill(p, 415, 1);
        let before = (skills::max_value(p, 43), skills::max_value(p, 415));
        stats::set_level(p, &data, 6);
        let after = (
            skills::value(p, 43),
            skills::max_value(p, 43),
            skills::value(p, 415),
            skills::max_value(p, 415),
        );
        // The same level again keeps the level's time played.
        p.row.played_level = 500;
        p.played_at = m.now;
        let p = m.players.get_mut(&g).unwrap();
        stats::set_level(p, &data, 6);
        let kept = p.row.played_level;
        stats::set_level(p, &data, 7);
        (before, after, kept, p.row.played_level)
    })
    .await;
    let (before, after, kept, next) = got;
    assert_eq!(before, (Some(5), Some(1)));
    assert_eq!(after, (Some(1), Some(30), Some(1), Some(1)));
    assert_eq!((kept, next), (500, 0));
}

const QUICK_20: u32 = 961_601;
const QUICK_25: u32 = 961_602;
const SLOW_CAST_50: u32 = 961_603;
const SLOW_CAST_20: u32 = 961_604;
const BOLT: u32 = 961_605;
const BOLT_FASTER: u32 = 961_606;
const TRADE: u32 = 961_607;

fn with_casts(mut d: GameData) -> GameData {
    use crate::game::data::spells::SpellEffect;
    for s in [
        aura(QUICK_20, AURA_MOD_CASTING_SPEED, 20, 0, false),
        aura(QUICK_25, AURA_MOD_CASTING_SPEED, 25, 0, false),
        aura(SLOW_CAST_50, AURA_MOD_CASTING_SPEED, -50, 0, false),
        aura(SLOW_CAST_20, AURA_MOD_CASTING_SPEED, -20, 0, false),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    // A bolt of 2.5 seconds at its target, of family 3 and flag 1; a
    // talent taking half a second off it; a craft of 2.5 seconds.
    d.spells.cast_times.insert(9_901, 2_500);
    let mut bolt = SpellTemplate {
        id: BOLT,
        name: "Bolt".into(),
        casting_time_index: 9_901,
        family_name: 3,
        family_flags: 1,
        range_index: 1,
        equipped_item_class: -1,
        ..Default::default()
    };
    bolt.effects[0] = SpellEffect {
        effect: 2,
        base_points: 9,
        base_dice: 1,
        target_a: 6,
        ..Default::default()
    };
    d.spells.spells.insert(BOLT, bolt);
    let mut faster = aura(BOLT_FASTER, AURA_ADD_FLAT_MODIFIER, -500, 10, true);
    faster.family_name = 3;
    faster.effects[0].item_type = 1;
    d.spells.spells.insert(BOLT_FASTER, faster);
    let mut trade = d.spells.spells[&BOLT].clone();
    trade.id = TRADE;
    trade.family_name = 0;
    trade.attributes[0] = crate::game::data::spells::ATTR_TRADESKILL;
    trade.effects[0] = SpellEffect::default();
    d.spells.spells.insert(TRADE, trade);
    d
}

#[tokio::test]
async fn cast_speed_multiplies_its_hastes_and_slows_by_the_stack_and_lengthens_a_spells_cast_after_its_modifiers()
 {
    let (realm, _a, g) = player(with_casts(data()), Class::Mage).await;
    let got = on_map(&realm, move |m| {
        let speed = |m: &Map| m.players[&g].fields.get_f32(UNIT_MOD_CAST_SPEED);
        for s in [QUICK_20, QUICK_25, SLOW_CAST_50] {
            put(m, g, s);
        }
        let balanced = speed(m);
        for s in [QUICK_20, QUICK_25, SLOW_CAST_50] {
            take(m, g, s);
        }
        let plain = speed(m);
        put(m, g, SLOW_CAST_20);
        m.players
            .get_mut(&g)
            .unwrap()
            .auras
            .holders
            .iter_mut()
            .filter(|h| h.spell == SLOW_CAST_20)
            .for_each(|h| h.stacks = 3);
        derive::unit(m, g);
        let stacked = speed(m);
        take(m, g, SLOW_CAST_20);
        // Half slower, with half a second off by the talent: 3 seconds.
        put(m, g, SLOW_CAST_50);
        put(m, g, BOLT_FASTER);
        let p = m.players.get_mut(&g).unwrap();
        p.spells.push(BOLT);
        p.spells.push(TRADE);
        for i in 0..5 {
            p.fields.set_u32(UNIT_FIELD_POWER1 + i, 100_000);
        }
        let target = {
            let data = m.realm.data.clone();
            let t = crate::game::data::CreatureTemplate {
                entry: 961_700,
                name: "Dummy".into(),
                display_ids: [903, 0, 0, 0],
                display_probabilities: [1, 0, 0, 0],
                level_min: 10,
                level_max: 10,
                faction: 14,
                unit_class: 1,
                health_multiplier: 1.0,
                health_max: 500,
                ..Default::default()
            };
            let _ = data;
            let p = &m.players[&g];
            let (pos, o) = (p.position(), p.movement.orientation);
            let at = Vector3d {
                x: pos.x + 3.0 * o.cos(),
                y: pos.y + 3.0 * o.sin(),
                z: pos.z,
            };
            let c = crate::game::map::creature::Creature::new(
                guid::unit(961_700, 0x7F_0000),
                &t,
                10,
                at,
                o + std::f32::consts::PI,
            );
            m.add_creature(c.with_base())
        };
        let now = m.now;
        let cast_ms = |m: &mut Map, spell: u32| {
            m.players.get_mut(&g).unwrap().caster.casting = None;
            m.players.get_mut(&g).unwrap().caster.gcd_until = None;
            crate::game::map::spells::cast(
                m,
                g,
                spell,
                crate::game::map::spells::Targets {
                    unit: Some(target),
                    ..Default::default()
                },
                None,
            );
            m.players[&g]
                .caster
                .casting
                .as_ref()
                .map(|c| (c.ends - now).as_millis())
        };
        let bolt = cast_ms(m, BOLT);
        // A craft isn't slowed.
        let craft = cast_ms(m, TRADE);
        (balanced, plain, stacked, bolt, craft)
    })
    .await;
    let (balanced, plain, stacked, bolt, craft) = got;
    assert!((balanced - 1.0).abs() < 1e-5, "{balanced}");
    assert_eq!(plain, 1.0);
    assert!((stacked - 1.6).abs() < 1e-5, "{stacked}");
    assert_eq!(bolt, Some(3_000));
    assert_eq!(craft, Some(2_500));
}

#[tokio::test]
async fn a_hunters_ranged_attack_power_takes_gear_and_auras_flat_then_percent_into_the_shot() {
    const RAP_PLUS_50: u32 = 961_801;
    const RAP_HALF_MORE: u32 = 961_802;
    const LONGBOW: u32 = 961_803;
    let mut d = with_gear_spells(data());
    for s in [
        aura(RAP_PLUS_50, AURA_MOD_RANGED_ATTACK_POWER, 50, 0, false),
        aura(
            RAP_HALF_MORE,
            AURA_MOD_RANGED_ATTACK_POWER_PCT,
            50,
            0,
            false,
        ),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    let mut bow = weapon(LONGBOW, 2, 15, 2800, 0);
    bow.damages[0] = (10.0, 20.0, 0);
    d.items.insert(LONGBOW, bow);
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Dwarf, Class::Hunter).await;
    let got = on_map(&realm, move |m| {
        let data = m.realm.data.clone();
        stats::set_level(m.players.get_mut(&g).unwrap(), &data, 60);
        derive::unit(m, g);
        wear(m, g, LONGBOW, 17);
        wear(m, g, RANGED_ROBE, 4);
        put(m, g, RAP_PLUS_50);
        put(m, g, RAP_HALF_MORE);
        let p = &m.players[&g];
        let f = &p.fields;
        (
            f.get_u32(UNIT_FIELD_STAT0) as f32,
            f.get_u32(UNIT_FIELD_STAT0 + 1) as f32,
            p.derived.ranged_attack_power as f32,
            p.derived.attack_power as f32,
            f.get_f32(UNIT_FIELD_MINRANGEDDAMAGE),
            f.get_f32(UNIT_FIELD_MAXRANGEDDAMAGE),
        )
    })
    .await;
    let (strength, agility, rap, ap, min, max) = got;
    let want_rap = ((60.0 * 2.0 + agility * 2.0 - 10.0) + 40.0 + 50.0) * 1.5;
    assert!((rap - want_rap.trunc()).abs() <= 1.0, "{rap} {want_rap}");
    assert!(
        (ap - (60.0 * 2.0 + strength + agility - 20.0)).abs() <= 1.0,
        "{ap}"
    );
    let bonus = rap / 14.0 * 2.8;
    assert!((min - (10.0 + bonus)).abs() < 1.0, "{min}");
    assert!((max - (20.0 + bonus)).abs() < 1.0, "{max}");
}
