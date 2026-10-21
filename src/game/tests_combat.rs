//! Melee, ranged and procs through the protocol: two hands, a dodge's
//! Overpower, next-swing spells, Auto Shot with ammunition, procs, damage
//! shields, and the table on derived stats.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::combat::{self, Hand, Outcome};
use crate::game::map::spells::{Targets, wire_targets};
use crate::game::map::{Map, MapHandle, MapKey};

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
/// A training dummy beside the start: a lot of health, never fights back.
const DUMMY: (u32, u32) = (3000, 20);
/// One at Auto Shot's range.
const FAR_DUMMY: (u32, u32) = (3000, 21);

const SWORD: u32 = 50_001;
const DAGGER: u32 = 50_002;
const BOW: u32 = 50_003;
const ARROW: u32 = 50_004;

const HEROIC_STRIKE: u32 = 78;
const AUTO_SHOT: u32 = 75;
const PROC_AURA: u32 = 90_001;
const PROC_BLOW: u32 = 90_002;
const THORNS: u32 = 90_003;
const CHANCE_AURA: u32 = 90_004;
const RIGHTEOUSNESS: u32 = 20154;
/// An enchantment whose combat spell is the blow, six times a minute.
const ENCHANT: u32 = 9_001;

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

fn weapon(
    entry: u32,
    subclass: u32,
    inventory_type: u32,
    delay: u32,
    damage: (f32, f32),
) -> ItemTemplate {
    ItemTemplate {
        entry,
        class: 2,
        subclass,
        name: format!("Weapon {entry}"),
        inventory_type,
        delay,
        damages: [
            (damage.0, damage.1, 0),
            (0.0, 0.0, 0),
            (0.0, 0.0, 0),
            (0.0, 0.0, 0),
            (0.0, 0.0, 0),
        ],
        stackable: 1,
        ..Default::default()
    }
}

fn data() -> GameData {
    let mut d = GameData::empty();
    // A human warrior's first level.
    d.player_level_stats.insert((1, 1, 1), [23, 20, 22, 20, 21]);
    d.player_class_stats.insert((1, 1), (60, 0));
    d.class_level_stats.insert(
        (1, 1),
        ClassLevelStats {
            health: 100_000,
            melee_damage: 1.0,
            ..Default::default()
        },
    );
    d.creature_templates.insert(
        DUMMY.0,
        CreatureTemplate {
            entry: DUMMY.0,
            name: "Dummy".into(),
            display_ids: [903, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            level_min: 1,
            level_max: 1,
            faction: 7,
            unit_class: 1,
            health_multiplier: 1.0,
            damage_multiplier: 1.0,
            base_attack_time: 2000,
            ..Default::default()
        },
    );
    let spawn = |guid, dx: f32| CreatureSpawn {
        guid,
        entries: vec![DUMMY.0],
        map: 0,
        x: START.0 + dx,
        y: START.1,
        z: START.2,
        respawn_min: 1,
        respawn_max: 1,
        ..Default::default()
    };
    d.creature_spawns
        .insert(0, vec![spawn(DUMMY.1, 2.0), spawn(FAR_DUMMY.1, 15.0)]);
    d.items
        .insert(SWORD, weapon(SWORD, 7, 13, 2000, (10.0, 20.0)));
    d.items
        .insert(DAGGER, weapon(DAGGER, 15, 13, 1500, (5.0, 8.0)));
    let mut bow = weapon(BOW, 2, 15, 2000, (10.0, 12.0));
    bow.ammo_type = 2;
    d.items.insert(BOW, bow);
    d.items.insert(
        ARROW,
        ItemTemplate {
            entry: ARROW,
            class: 6,
            subclass: 2,
            name: "Arrow".into(),
            inventory_type: 24,
            stackable: 200,
            damages: [
                (2.0, 2.0, 0),
                (0.0, 0.0, 0),
                (0.0, 0.0, 0),
                (0.0, 0.0, 0),
                (0.0, 0.0, 0),
            ],
            ..Default::default()
        },
    );
    let s = &mut d.spells;
    s.ranges.insert(2, (0.0, 5.0));
    s.ranges.insert(114, (5.0, 35.0));
    let spell = |id, name: &str, effects: [SpellEffect; 3]| SpellTemplate {
        id,
        name: name.into(),
        range_index: 2,
        equipped_item_class: -1,
        effects,
        ..Default::default()
    };
    let none = SpellEffect::default();
    let mut hs = spell(
        HEROIC_STRIKE,
        "Heroic Strike",
        [effect(17, 99, 6), none, none],
    );
    hs.attributes[0] = 0x4 | 0x10;
    hs.power_type = 1;
    hs.mana_cost = 150;
    hs.dmg_class = 2;
    let mut shot = spell(AUTO_SHOT, "Auto Shot", [effect(58, -1, 6), none, none]);
    shot.attributes[0] = 0x2 | 0x10;
    shot.attributes[2] = 0x20;
    shot.range_index = 114;
    shot.dmg_class = 3;
    shot.equipped_item_class = 2;
    // A proc on every swing that lands: a blow of 1.
    let mut proc_effect = effect(6, -1, 1);
    proc_effect.aura = 42;
    proc_effect.trigger_spell = PROC_BLOW;
    let mut proc_aura = spell(PROC_AURA, "Every Blow", [proc_effect, none, none]);
    proc_aura.proc_flags = 0x4;
    proc_aura.proc_chance = 100;
    proc_aura.attributes[0] = crate::game::data::spells::ATTR_PASSIVE;
    let blow = spell(PROC_BLOW, "Blow", [effect(2, 0, 6), none, none]);
    // A third of the time.
    let mut chance_effect = effect(6, -1, 1);
    chance_effect.aura = 42;
    chance_effect.trigger_spell = PROC_BLOW;
    let mut chance = spell(CHANCE_AURA, "A Third", [chance_effect, none, none]);
    chance.proc_flags = 0x4;
    chance.proc_chance = 30;
    chance.attributes[0] = crate::game::data::spells::ATTR_PASSIVE;
    let mut thorns_effect = effect(6, 9, 1);
    thorns_effect.aura = 15;
    let mut thorns = spell(THORNS, "Thorns", [thorns_effect, none, none]);
    thorns.school = 3;
    let mut seal_effect = effect(6, 999, 1);
    seal_effect.aura = 4;
    let mut seal = spell(
        RIGHTEOUSNESS,
        "Seal of Righteousness",
        [seal_effect, none, none],
    );
    seal.proc_flags = 0x4;
    seal.proc_chance = 100;
    // Its first rank's holy strike (round 12e: the seal casts it).
    let mut strike = spell(
        25742,
        "Seal of Righteousness",
        [effect(2, 0, 6), none, none],
    );
    strike.school = 1;
    for t in [hs, shot, proc_aura, blow, chance, thorns, seal, strike] {
        s.spells.insert(t.id, t);
    }
    s.enchantments.insert(
        ENCHANT,
        crate::game::data::spells::Enchantment {
            id: ENCHANT,
            effects: [(1, 0, 0, PROC_BLOW), (0, 0, 0, 0), (0, 0, 0, 0)],
            name: "Blows".into(),
        },
    );
    s.enchant_ppm.insert(PROC_BLOW, 6.0);
    d
}

/// Alice, a warrior next to the dummy, facing it.
async fn setup() -> (std::sync::Arc<crate::game::Realm>, Client, Guid, MapHandle) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.sees(guid::unit(DUMMY.0, DUMMY.1)).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    (realm, a, ga, map)
}

fn dummy() -> Guid {
    guid::unit(DUMMY.0, DUMMY.1)
}

/// Put an item of `entry` in the equipment slot `slot`, in place of what
/// the character started with there.
fn equip(m: &mut Map, g: Guid, entry: u32, slot: u8) {
    if let Some(p) = m.players.get_mut(&g) {
        p.items
            .retain(|i| !(i.bag == crate::game::store::BAG_PLAYER && i.slot == slot));
    }
    crate::game::map::inventory::add_item(m, g, entry, 1, false).expect("room for it");
    let data = m.realm.data.clone();
    let p = m.players.get_mut(&g).expect("the player");
    let i = p
        .items
        .iter()
        .rposition(|i| i.entry == entry)
        .expect("the item");
    p.items[i].bag = crate::game::store::BAG_PLAYER;
    p.items[i].slot = slot;
    p.refresh_inventory(&data);
}

/// A packed GUID at the start of `b`.
fn packed_guid(b: &[u8]) -> Guid {
    let mask = b[0];
    let mut value = 0u64;
    let mut at = 1;
    for i in 0..8 {
        if mask & (1 << i) != 0 {
            value |= u64::from(b[at]) << (i * 8);
            at += 1;
        }
    }
    Guid::new(value)
}

#[tokio::test]
async fn both_hands_swing_with_two_weapons_and_the_off_hand_says_so() {
    let (_realm, mut a, ga, map) = setup().await;
    let (main, off) = map
        .query(move |m| {
            equip(m, ga, SWORD, 15);
            equip(m, ga, DAGGER, 16);
            if let Some(p) = m.players.get_mut(&ga) {
                p.spells.push(crate::game::map::derive::SPELL_DUAL_WIELD);
            }
            crate::game::map::auras::recalc(m, ga);
            let p = &m.players[&ga];
            (
                p.fields.get_u32(UNIT_FIELD_BASEATTACKTIME),
                p.fields.get_u32(UNIT_FIELD_BASEATTACKTIME + 1),
            )
        })
        .await
        .unwrap();
    assert_eq!((main, off), (2000, 1500));
    a.clear();
    a.send(CMSG_ATTACKSWING { guid: dummy() }).await;
    // The off hand's blow carries the left-swing flag.
    a.wait_frame("an off-hand swing", combat::SMSG_ATTACKERSTATEUPDATE, |b| {
        u32::from_le_bytes([b[0], b[1], b[2], b[3]]) & combat::HITINFO_LEFTSWING != 0
    })
    .await;
    let (dual, roll) = map
        .query(move |m| {
            (
                combat::dual_wields(m, ga),
                combat::roll_for(m, ga, dummy(), Hand::Main).map(|r| r.dual_wield),
            )
        })
        .await
        .unwrap();
    assert!(dual);
    assert_eq!(roll, Some(true), "white swings pay the dual-wield miss");
}

#[tokio::test]
async fn a_dodge_gives_a_warrior_overpower_for_a_while_and_opens_the_defender_window() {
    let (_realm, mut a, ga, map) = setup().await;
    let target = dummy();
    // The dummy dodges everything (and is of a level that dodges in full).
    map.query(move |m| {
        if let Some(c) = m.creature_mut(target) {
            c.fields.set_u32(UNIT_FIELD_LEVEL, 10);
            c.derived.dodge = 100.0;
            c.derived.defense = 5;
        }
        if let Some(p) = m.players.get_mut(&ga) {
            p.derived.hit = 100.0;
        }
    })
    .await;
    a.send(CMSG_ATTACKSWING { guid: target }).await;
    a.wait_frame("a dodge", combat::SMSG_ATTACKERSTATEUPDATE, |b| {
        // The victim state follows the two packed GUIDs and the damage.
        b.len() > 20
    })
    .await;
    let (points, combo_target) = map
        .query(move |m| {
            let p = &m.players[&ga];
            (
                p.caster.combo_points,
                p.fields.get_guid(PLAYER_FIELD_COMBO_TARGET),
            )
        })
        .await
        .unwrap();
    assert_eq!(points, 1, "Overpower's combo point");
    assert_eq!(combo_target, target);
    // The point runs out.
    map.query(move |m| {
        combat::stop_attack(m, ga);
        if let Some(p) = m.players.get_mut(&ga) {
            p.combat.dodge_combo_until = Some(Instant::now());
        }
    })
    .await;
    tokio::time::sleep(Duration::from_millis(120)).await;
    let points = map
        .query(move |m| m.players[&ga].caster.combo_points)
        .await
        .unwrap();
    assert_eq!(points, 0);
    // A player who parries opens their defense window (Revenge).
    let state = map
        .query(move |m| {
            combat::after_outcome(m, target, ga, Outcome::Parry);
            m.players[&ga].fields.get_u32(UNIT_FIELD_AURASTATE)
        })
        .await
        .unwrap();
    assert_ne!(state & combat::AURA_STATE_DEFENSE, 0);
}

#[tokio::test]
async fn heroic_strike_takes_the_next_swing() {
    let (_realm, mut a, ga, map) = setup().await;
    let target = dummy();
    map.query(move |m| {
        equip(m, ga, SWORD, 15);
        if let Some(p) = m.players.get_mut(&ga) {
            p.spells.push(HEROIC_STRIKE);
            p.fields.set_u32(UNIT_FIELD_POWER1 + 1, 1000);
            p.fields.set_u32(UNIT_FIELD_MAXPOWER1 + 1, 1000);
            // Wait for the queue before swinging.
            p.combat.next_swing = Some(Instant::now() + Duration::from_secs(60));
        }
    })
    .await;
    a.send(CMSG_CAST_SPELL {
        spell: HEROIC_STRIKE,
        targets: wire_targets(&Targets {
            unit: Some(target),
            ..Default::default()
        }),
    })
    .await;
    tokio::time::sleep(Duration::from_millis(60)).await;
    let queued = map
        .query(move |m| m.players[&ga].combat.queued)
        .await
        .unwrap();
    assert_eq!(queued, Some(HEROIC_STRIKE));
    a.clear();
    map.query(move |m| {
        combat::start_attack(m, ga, target);
        if let Some(p) = m.players.get_mut(&ga) {
            p.combat.next_swing = Some(Instant::now());
        }
        combat::try_swing(m, ga);
    })
    .await;
    a.wait_for("Heroic Strike going off", |m| match m {
        ServerOpcodeMessage::SMSG_SPELL_GO(s) if s.spell == HEROIC_STRIKE => Some(()),
        _ => None,
    })
    .await;
    let (queued, rage, health) = map
        .query(move |m| {
            (
                m.players[&ga].combat.queued,
                m.players[&ga].fields.get_u32(UNIT_FIELD_POWER1 + 1),
                m.creature(target)
                    .map(|c| c.fields.get_u32(UNIT_FIELD_HEALTH)),
            )
        })
        .await
        .unwrap();
    assert_eq!(queued, None);
    // Its rage paid (the dummy's blows back give a little).
    assert!((850..900).contains(&rage), "{rage}");
    assert!(health.is_some_and(|h| h < 100_000));
    // That swing was the strike's: no white blow of Alice's with it.
    let white = a
        .gets(Duration::from_millis(100), |m| match m {
            ServerOpcodeMessage::SMSG_ATTACKERSTATEUPDATE(s) if s.attacker == ga => Some(()),
            _ => None,
        })
        .await;
    assert!(white.is_none());
    assert!(
        !a.raw
            .iter()
            .filter(|f| u16::from_le_bytes([f[2], f[3]]) == combat::SMSG_ATTACKERSTATEUPDATE)
            .any(|f| packed_guid(&f[8..]) == ga)
    );
}

#[tokio::test]
async fn auto_shot_repeats_and_spends_arrows() {
    let (_realm, mut a, ga, map) = setup().await;
    let target = guid::unit(FAR_DUMMY.0, FAR_DUMMY.1);
    a.sees(target).await;
    map.query(move |m| {
        equip(m, ga, BOW, 17);
        crate::game::map::inventory::add_item(m, ga, ARROW, 10, false).expect("room");
        crate::game::map::ranged::set_ammo(m, ga, ARROW);
        if let Some(p) = m.players.get_mut(&ga) {
            p.spells.push(AUTO_SHOT);
            p.derived.ranged_hit = 100.0;
        }
        // It stands where it is: closing to melee, it would be too near to
        // shoot.
        if let Some(s) = crate::game::map::event_ai::scripted_mut(m, target) {
            s.no_combat_movement = true;
        }
    })
    .await;
    let speed = map
        .query(move |m| m.players[&ga].fields.get_u32(UNIT_FIELD_RANGEDATTACKTIME))
        .await
        .unwrap();
    assert_eq!(speed, 2000);
    let started = Instant::now();
    a.send(CMSG_CAST_SPELL {
        spell: AUTO_SHOT,
        targets: wire_targets(&Targets {
            unit: Some(target),
            ..Default::default()
        }),
    })
    .await;
    for _ in 0..2 {
        a.wait_for("a shot", |m| match m {
            ServerOpcodeMessage::SMSG_SPELL_GO(s) if s.spell == AUTO_SHOT => Some(()),
            _ => None,
        })
        .await;
    }
    // Half a second, then the bow's two seconds.
    assert!(started.elapsed() >= Duration::from_millis(2400));
    let arrows = map
        .query(move |m| {
            m.players[&ga]
                .items
                .iter()
                .filter(|i| i.entry == ARROW)
                .map(|i| i.count())
                .sum::<u32>()
        })
        .await
        .unwrap();
    assert_eq!(arrows, 8);
    a.send(CMSG_CANCEL_AUTO_REPEAT_SPELL {}).await;
    tokio::time::sleep(Duration::from_millis(60)).await;
    let repeating = map
        .query(move |m| m.players[&ga].combat.auto_repeat.is_some())
        .await
        .unwrap();
    assert!(!repeating);
}

#[tokio::test]
async fn a_proc_aura_goes_off_at_its_chance() {
    let (_realm, _a, ga, map) = setup().await;
    let target = dummy();
    let (always, third) = map
        .query(move |m| {
            let data = m.realm.data.clone();
            let health = |m: &Map| {
                m.creature(target)
                    .map_or(0, |c| c.fields.get_u32(UNIT_FIELD_HEALTH))
            };
            let landed = crate::game::map::procs::Event {
                actor: ga,
                target,
                actor_flags: crate::game::map::procs::PROC_DONE_MELEE,
                target_flags: crate::game::map::procs::PROC_TAKEN_MELEE,
                ex: crate::game::map::procs::EX_NORMAL_HIT,
                school: 1,
                ..Default::default()
            };
            crate::game::map::auras::apply(m, ga, ga, data.spells.get(PROC_AURA).unwrap(), 0, 0);
            let before = health(m);
            for _ in 0..100 {
                crate::game::map::procs::trigger(m, landed);
            }
            let always = before - health(m);
            crate::game::map::auras::remove_spell(m, ga, PROC_AURA, None);
            crate::game::map::auras::apply(m, ga, ga, data.spells.get(CHANCE_AURA).unwrap(), 0, 0);
            let before = health(m);
            for _ in 0..1000 {
                crate::game::map::procs::trigger(m, landed);
            }
            // A dodge doesn't proc it.
            let mut dodged = landed;
            dodged.ex = crate::game::map::procs::EX_DODGE;
            let after = health(m);
            for _ in 0..100 {
                crate::game::map::procs::trigger(m, dodged);
            }
            assert_eq!(after, health(m));
            (always, before - after)
        })
        .await
        .unwrap();
    assert_eq!(always, 100, "a blow of one on every hit");
    assert!((220..=380).contains(&third), "about 300 of 1000: {third}");
}

#[tokio::test]
async fn a_weapon_enchantment_procs_a_minute_and_a_seal_strikes_by_weapon_speed() {
    let (_realm, _a, ga, map) = setup().await;
    let target = dummy();
    let (enchanted, seal) = map
        .query(move |m| {
            let data = m.realm.data.clone();
            equip(m, ga, SWORD, 15);
            let health = |m: &Map| {
                m.creature(target)
                    .map_or(0, |c| c.fields.get_u32(UNIT_FIELD_HEALTH))
            };
            let landed = crate::game::map::procs::Event {
                actor: ga,
                target,
                actor_flags: crate::game::map::procs::PROC_DONE_MELEE,
                target_flags: crate::game::map::procs::PROC_TAKEN_MELEE,
                ex: crate::game::map::procs::EX_NORMAL_HIT,
                school: 1,
                ..Default::default()
            };
            if let Some(p) = m.players.get_mut(&ga)
                && let Some(i) = p.items.iter().position(|i| i.entry == SWORD)
            {
                p.items[i].fields.set_u32(ITEM_FIELD_ENCHANTMENT, ENCHANT);
            }
            let before = health(m);
            for _ in 0..1000 {
                crate::game::map::procs::trigger(m, landed);
            }
            let enchanted = before - health(m);
            if let Some(p) = m.players.get_mut(&ga)
                && let Some(i) = p.items.iter().position(|i| i.entry == SWORD)
            {
                p.items[i].fields.set_u32(ITEM_FIELD_ENCHANTMENT, 0);
            }
            crate::game::map::auras::apply(
                m,
                ga,
                ga,
                data.spells.get(RIGHTEOUSNESS).unwrap(),
                0,
                1000,
            );
            let before = health(m);
            crate::game::map::procs::trigger(m, landed);
            (enchanted, before - health(m))
        })
        .await
        .unwrap();
    // Six a minute on a two-second sword: 20%.
    assert!(
        (140..=260).contains(&enchanted),
        "about 200 of 1000: {enchanted}"
    );
    // mangos-classic's one-hander: 0.85 x ceil(1000 x 1.2 x 1.03 x 2.0 /
    // 100) - 1, and 3% of the sword's 15, and one (round 12e).
    assert_eq!(seal, 21);
}

#[tokio::test]
async fn thorns_hit_back_at_a_blow_that_lands() {
    let (_realm, mut a, ga, map) = setup().await;
    let target = dummy();
    map.query(move |m| {
        let data = m.realm.data.clone();
        crate::game::map::auras::apply(m, target, target, data.spells.get(THORNS).unwrap(), 0, 10);
        if let Some(p) = m.players.get_mut(&ga) {
            p.derived.hit = 100.0;
            p.fields.set_u32(UNIT_FIELD_MAXHEALTH, 1000);
            p.fields.set_u32(UNIT_FIELD_HEALTH, 1000);
        }
        if let Some(c) = m.creature_mut(target) {
            c.derived.dodge = 0.0;
            c.derived.parry = 0.0;
            c.derived.block = 0.0;
        }
    })
    .await;
    a.send(CMSG_ATTACKSWING { guid: target }).await;
    let (victim, damage) = a
        .wait_for("SMSG_SPELLDAMAGESHIELD", |m| match m {
            ServerOpcodeMessage::SMSG_SPELLDAMAGESHIELD(s) => Some((s.victim, s.damage)),
            _ => None,
        })
        .await;
    assert_eq!((victim, damage), (ga, 10));
    let health = map
        .query(move |m| m.players[&ga].fields.get_u32(UNIT_FIELD_HEALTH))
        .await
        .unwrap();
    assert!(health <= 990, "{health}");
}

#[tokio::test]
async fn the_table_reads_the_derived_chances() {
    let (_realm, _a, ga, map) = setup().await;
    let target = dummy();
    let (crit_field, roll) = map
        .query(move |m| {
            equip(m, ga, SWORD, 15);
            let crit = m.players[&ga].fields.get_f32(PLAYER_CRIT_PERCENTAGE);
            if let Some(c) = m.creature_mut(target) {
                c.derived.parry = 12.0;
                c.derived.block = 3.0;
            }
            (crit, combat::roll_for(m, ga, target, Hand::Main))
        })
        .await
        .unwrap();
    let roll = roll.expect("a roll");
    // A level-one warrior: agility over the class's level-one rate.
    assert!((crit_field - 20.0 / 3.9).abs() < 0.1, "{crit_field}");
    assert!((roll.crit - crit_field).abs() < 0.01);
    assert_eq!(roll.parry, 12.0);
    assert_eq!(roll.block, 3.0);
    assert_eq!(roll.attacker_skill, 5);
    assert_eq!(roll.victim_defense, 5);
}
