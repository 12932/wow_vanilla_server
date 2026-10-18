//! Phase 3, round 4, against Thistle Tea's `stats/attack_power_test.exs`:
//! a creature's attack power from its class's seed, flat amounts with
//! their stacks before the percentages, its damage moved by vmangos's
//! ratio and floored at seven tenths, a debuff's going (expiry, removal,
//! death, a respawn) giving it all back, and its weapon spells striking
//! with the damage so moved.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::{Map, ai, auras, derive, spells};
use crate::game::tests_p3_combat::*;

const SPAWN: u32 = 7_101;
const SEEDED: u32 = 3_720;

/// A level 60 warrior creature of 200 attack power striking for 100,
/// spawned two yards east of the start.
fn world() -> GameData {
    let mut d = data();
    d.class_level_stats.insert(
        (1, 60),
        ClassLevelStats {
            health: 1_000,
            melee_damage: 100.0,
            attack_power: 200,
            ..Default::default()
        },
    );
    let mut t = template(SEEDED, 7);
    (t.level_min, t.level_max) = (60, 60);
    t.damage_variance = 0.000_001;
    d.creature_templates.insert(SEEDED, t);
    d.creature_spawns.insert(
        0,
        vec![CreatureSpawn {
            guid: SPAWN,
            entries: vec![SEEDED],
            map: 0,
            x: START.0 + 2.0,
            y: START.1,
            z: START.2,
            respawn_min: 60,
            respawn_max: 60,
            ..Default::default()
        }],
    );
    for (id, kind, amount) in [
        (FLAT, auras::AURA_MOD_ATTACK_POWER, 10),
        (PCT, auras::AURA_MOD_ATTACK_POWER_PCT, 10),
        (OTHER_PCT, auras::AURA_MOD_ATTACK_POWER_PCT, 50),
        (CRUSHING_PCT, auras::AURA_MOD_ATTACK_POWER_PCT, -150),
        (SAPPED, auras::AURA_MOD_ATTACK_POWER, -500),
    ] {
        d.spells.spells.insert(id, aura(id, kind, amount, 1));
    }
    let mut shout = aura(DEMORALIZING_SHOUT, auras::AURA_MOD_ATTACK_POWER, -100, 6);
    shout.duration_index = 9;
    d.spells.spells.insert(DEMORALIZING_SHOUT, shout);
    d.spells.durations.insert(
        9,
        crate::game::data::spells::SpellDuration {
            base: 30_000,
            ..Default::default()
        },
    );
    for (id, effect) in [(STRIKE_WEAPON, 58), (STRIKE_NORMALIZED, 121)] {
        let mut s = SpellTemplate {
            id,
            name: format!("Strike {id}"),
            equipped_item_class: -1,
            range_index: 1,
            dmg_class: 2,
            ..Default::default()
        };
        // No points of its own: the weapon's blow alone.
        s.effects[0] = SpellEffect {
            effect,
            base_points: -1,
            target_a: 6,
            ..Default::default()
        };
        d.spells.spells.insert(id, s);
    }
    d
}

const FLAT: u32 = 98_001;
const PCT: u32 = 98_002;
const OTHER_PCT: u32 = 98_003;
const CRUSHING_PCT: u32 = 98_004;
const SAPPED: u32 = 98_005;
const DEMORALIZING_SHOUT: u32 = 98_006;
const STRIKE_WEAPON: u32 = 98_007;
const STRIKE_NORMALIZED: u32 = 98_008;
const SURE: u32 = 98_009;

fn aura(id: u32, kind: u32, amount: i32, target_a: u32) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Aura {id}"),
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: 6,
        aura: kind,
        base_points: amount - 1,
        base_dice: 1,
        target_a,
        ..Default::default()
    };
    s
}

fn put(m: &mut Map, caster: Guid, target: Guid, spell: u32) {
    let s = m.realm.data.spells.get(spell).unwrap().clone();
    let amount = s.effects[0].base_points + 1;
    auras::apply_spell(m, caster, target, &s, &[(0, amount)], Default::default())
        .expect("it takes");
    derive::unit(m, target);
}

fn stacks(m: &mut Map, g: Guid, spell: u32, n: u32) {
    for h in auras::auras_mut(m, g)
        .unwrap()
        .holders
        .iter_mut()
        .filter(|h| h.spell == spell)
    {
        h.stacks = n;
    }
    derive::unit(m, g);
}

/// Its attack power and its damage's least.
fn ap(m: &Map, g: Guid) -> (u32, f32) {
    (
        m.creature(g).unwrap().derived.attack_power,
        m.unit_fields(g).unwrap().get_f32(UNIT_FIELD_MINDAMAGE),
    )
}

#[tokio::test]
async fn a_creatures_flat_attack_power_and_its_stacks_come_before_the_percentages_and_its_damage_follows()
 {
    let (_realm, _a, g, map) = setup_with(world(), Class::Warrior).await;
    let brute = guid::unit(SEEDED, SPAWN);
    let r = map
        .query(move |m| {
            let seed = ap(m, brute);
            // 10 of three stacks, 10% of two, and 50%: (200 + 30) by 1.2
            // and 1.5, in whichever order.
            put(m, brute, brute, FLAT);
            stacks(m, brute, FLAT, 3);
            put(m, brute, brute, PCT);
            stacks(m, brute, PCT, 2);
            put(m, brute, brute, OTHER_PCT);
            let raised = ap(m, brute);
            for id in [FLAT, PCT, OTHER_PCT] {
                auras::remove_spell(m, brute, id, None);
            }
            derive::unit(m, brute);
            put(m, brute, brute, OTHER_PCT);
            put(m, brute, brute, FLAT);
            stacks(m, brute, FLAT, 3);
            put(m, brute, brute, PCT);
            stacks(m, brute, PCT, 2);
            let reordered = ap(m, brute).0;
            for id in [FLAT, PCT, OTHER_PCT] {
                auras::remove_spell(m, brute, id, None);
            }
            // A penalty past a hundred percent: nothing, and seven tenths of
            // its damage; so with no attack power at all.
            put(m, brute, brute, CRUSHING_PCT);
            let crushed = ap(m, brute);
            auras::remove_spell(m, brute, CRUSHING_PCT, None);
            put(m, brute, brute, SAPPED);
            let sapped = ap(m, brute);
            auras::remove_spell(m, brute, SAPPED, None);
            derive::unit(m, brute);
            (seed, raised, reordered, crushed, sapped, ap(m, brute))
        })
        .await
        .unwrap();
    let (seed, raised, reordered, crushed, sapped, back) = r;
    let near = |a: f32, b: f32| (a - b).abs() < 0.01;
    assert_eq!(seed.0, 200);
    assert!(near(seed.1, 100.0), "{seed:?}");
    assert_eq!(raised.0, 414);
    // (7 * 200 + 3 * 414) / (10 * 200) of 100.
    assert!(near(raised.1, 132.1), "{raised:?}");
    assert_eq!(reordered, 414);
    assert_eq!(crushed.0, 0);
    assert!(near(crushed.1, 70.0), "{crushed:?}");
    assert_eq!(sapped.0, 0);
    assert!(near(sapped.1, 70.0), "{sapped:?}");
    assert_eq!(back, seed);
    let _ = g;
}

#[tokio::test]
async fn a_shouts_debuff_holds_once_through_a_refresh_and_its_going_any_way_gives_the_damage_back()
{
    let (_realm, _a, g, map) = setup_with(world(), Class::Warrior).await;
    let brute = guid::unit(SEEDED, SPAWN);
    let r = map
        .query(move |m| {
            level(m, g, 60);
            let seed = ap(m, brute);
            let mut out = Vec::new();
            // Shouted at twice: one aura, a hundred less, 85 a blow.
            put(m, g, brute, DEMORALIZING_SHOUT);
            put(m, g, brute, DEMORALIZING_SHOUT);
            let count = auras::auras(m, brute)
                .unwrap()
                .holders
                .iter()
                .filter(|h| h.spell == DEMORALIZING_SHOUT)
                .count();
            out.push(ap(m, brute));
            // Run out.
            let now = m.now;
            for h in auras::auras_mut(m, brute).unwrap().holders.iter_mut() {
                h.expires = Some(now - Duration::from_millis(1));
            }
            auras::update(m);
            derive::unit(m, brute);
            out.push(ap(m, brute));
            // Taken off.
            put(m, g, brute, DEMORALIZING_SHOUT);
            auras::remove_spell(m, brute, DEMORALIZING_SHOUT, None);
            derive::unit(m, brute);
            out.push(ap(m, brute));
            // Dead, and back.
            put(m, g, brute, DEMORALIZING_SHOUT);
            crate::game::map::combat::deal_damage(m, g, brute, 10_000_000);
            let dead_clear = !auras::auras(m, brute)
                .unwrap()
                .has_spell(DEMORALIZING_SHOUT);
            ai::respawn_now(m, brute);
            out.push(ap(m, brute));
            (seed, count, out, dead_clear)
        })
        .await
        .unwrap();
    let (seed, count, out, dead_clear) = r;
    let near = |a: f32, b: f32| (a - b).abs() < 0.01;
    assert_eq!(count, 1);
    assert_eq!(out[0].0, 100);
    assert!(near(out[0].1, 85.0), "{:?}", out[0]);
    for (n, back) in out[1..].iter().enumerate() {
        assert_eq!(back.0, seed.0, "{n}");
        assert!(near(back.1, seed.1), "{n}: {back:?}");
    }
    assert!(dead_clear);
}

#[tokio::test]
async fn a_creatures_weapon_spells_strike_with_its_damage_as_its_attack_power_moved_it() {
    let mut d = world();
    // Sure to hit and never to crit.
    let mut sure = aura(SURE, auras::AURA_MOD_HIT_CHANCE, 100, 1);
    sure.effects[1] = SpellEffect {
        aura: auras::AURA_MOD_CRIT_PERCENT,
        base_points: -101,
        ..sure.effects[0]
    };
    d.spells.spells.insert(SURE, sure);
    let (_realm, _a, g, map) = setup_with(d, Class::Warrior).await;
    let brute = guid::unit(SEEDED, SPAWN);
    let r = map
        .query(move |m| {
            level(m, g, 60);
            let s = m.realm.data.spells.get(SURE).unwrap().clone();
            auras::apply_spell(
                m,
                brute,
                brute,
                &s,
                &[(0, 100), (1, -100)],
                Default::default(),
            )
            .unwrap();
            put(m, g, brute, DEMORALIZING_SHOUT);
            let mut out = Vec::new();
            for spell in [STRIKE_WEAPON, STRIKE_NORMALIZED] {
                let f = m.unit_fields_mut(g).unwrap();
                f.set_u32(UNIT_FIELD_RESISTANCES, 0);
                let max = f.get_u32(UNIT_FIELD_MAXHEALTH);
                f.set_u32(UNIT_FIELD_HEALTH, max);
                // Stunned: she neither dodges nor parries.
                let f = m.unit_fields_mut(g).unwrap();
                let flags = f.get_u32(UNIT_FIELD_FLAGS);
                f.set_u32(UNIT_FIELD_FLAGS, flags | 0x0004_0000);
                spells::cast_triggered(m, brute, spell, Some(g));
                out.push(max - m.unit_fields(g).unwrap().get_u32(UNIT_FIELD_HEALTH));
            }
            out
        })
        .await
        .unwrap();
    assert_eq!(r, vec![85, 85]);
}
