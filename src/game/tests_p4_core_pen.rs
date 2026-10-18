//! Phase 3, round 4, against Thistle Tea's
//! `stats/resistance_penetration_test.exs`: a caster's penetration
//! (`SPELL_AURA_MOD_TARGET_RESISTANCE`) by school, stacked, never below
//! nothing and leaving a creature's level resistance; armor penetrated
//! before a swing's damage, its crit and its off hand's, and a weapon
//! ability's.

use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::combat::{self, Hand, Outcome};
use crate::game::map::spells::outcome;
use crate::game::map::{Map, auras, spells};
use crate::game::tests_p3_combat::*;

const PEN: u32 = auras::AURA_MOD_TARGET_RESISTANCE;

#[tokio::test]
async fn penetration_takes_off_each_schools_own_stacked_and_never_below_nothing() {
    let (_realm, _a, g, map) = setup(Class::Mage).await;
    let r = map
        .query(move |m| {
            level(m, g, 60);
            let brute = spawn(m, g, BRUTE, 63, 3.0);
            let f = m.unit_fields_mut(brute).unwrap();
            for school in 1..7u16 {
                f.set_u32(UNIT_FIELD_RESISTANCES + school, 100);
            }
            // -10 of all magic, -30 of fire.
            aura_of(m, g, 98_101, PEN, 126, -10);
            aura_of(m, g, 98_102, PEN, 4, -30);
            let each: Vec<i32> = (1..7)
                .map(|s| outcome::resistance(m, g, brute, s))
                .collect();
            // Over-penetrated: nothing left of it, but its level still
            // resists for it.
            aura_of(m, g, 98_103, PEN, 4, -1_000);
            let fire = outcome::resistance(m, g, brute, 2);
            (each, fire)
        })
        .await
        .unwrap();
    let (each, fire) = r;
    assert_eq!(each, vec![90, 60, 90, 90, 90, 90]);
    assert_eq!(fire, 0);
    assert!(outcome::resist_chance(fire, 60, true, 3) > 0.0);
    assert_eq!(outcome::resist_chance(fire, 60, false, 3), 0.0);
}

const STRIKE: u32 = 98_110;
const SURE: u32 = 98_111;

#[tokio::test]
async fn armor_is_penetrated_before_a_swings_damage_its_crit_and_its_off_hands_and_a_weapon_abilitys()
 {
    let mut d = data();
    let t = d.creature_templates.get_mut(&BRUTE).unwrap();
    t.flags_extra |= combat::FLAG_EXTRA_NO_BLOCK | combat::FLAG_EXTRA_NO_PARRY;
    let mut strike = SpellTemplate {
        id: STRIKE,
        name: "Weapon Strike".into(),
        equipped_item_class: -1,
        range_index: 1,
        dmg_class: 2,
        ..Default::default()
    };
    strike.effects[0] = SpellEffect {
        effect: 58,
        base_points: -1,
        target_a: 6,
        ..Default::default()
    };
    d.spells.spells.insert(STRIKE, strike);
    let mut sure = SpellTemplate {
        id: SURE,
        name: "Sure".into(),
        equipped_item_class: -1,
        ..Default::default()
    };
    for (i, kind) in [auras::AURA_MOD_HIT_CHANCE, auras::AURA_MOD_SPELL_HIT_CHANCE]
        .into_iter()
        .enumerate()
    {
        sure.effects[i] = SpellEffect {
            effect: 6,
            aura: kind,
            base_points: 99,
            base_dice: 1,
            target_a: 1,
            ..Default::default()
        };
    }
    sure.effects[2] = SpellEffect {
        effect: 6,
        aura: auras::AURA_MOD_CRIT_PERCENT,
        base_points: -101,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(SURE, sure);
    let (_realm, _a, g, map) = setup_with(d, Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, g, 60);
            let brute = spawn(m, g, BRUTE, 60, 2.0);
            // 700 armor off, three stacks: 3000 is 900.
            aura_of(m, g, 98_112, PEN, 1, -700);
            for h in auras::auras_mut(m, g)
                .unwrap()
                .holders
                .iter_mut()
                .filter(|h| h.spell == 98_112)
            {
                h.stacks = 3;
            }
            let s = m.realm.data.spells.get(SURE).unwrap().clone();
            auras::apply_spell(
                m,
                g,
                g,
                &s,
                &[(0, 100), (1, 100), (2, -100)],
                Default::default(),
            )
            .unwrap();
            let set_weapon = |m: &mut Map| {
                let f = m.unit_fields_mut(g).unwrap();
                for i in [
                    UNIT_FIELD_MINDAMAGE,
                    UNIT_FIELD_MAXDAMAGE,
                    UNIT_FIELD_MINOFFHANDDAMAGE,
                    UNIT_FIELD_MAXOFFHANDDAMAGE,
                ] {
                    f.set_f32(i, 1_000.0);
                }
            };
            set_weapon(m);
            let f = m.unit_fields_mut(brute).unwrap();
            f.set_u32(UNIT_FIELD_RESISTANCES, 3_000);
            let flags = f.get_u32(UNIT_FIELD_FLAGS);
            f.set_u32(UNIT_FIELD_FLAGS, flags | 0x0004_0000);
            let expected = combat::armor_mitigate(m, g, brute, 1_000);
            let health = |m: &Map| m.unit_fields(brute).unwrap().get_u32(UNIT_FIELD_HEALTH);
            let mut out = Vec::new();
            for outcome in [Outcome::Hit, Outcome::Crit] {
                let before = health(m);
                forced_swing(m, g, brute, outcome);
                out.push(before - health(m));
            }
            // The off hand's blow takes the same armor.
            let off = combat::weapon_damage(m, g, brute, Hand::Off, &mut rand::rng()).0;
            let off = combat::armor_mitigate(m, g, brute, off);
            // A weapon ability, after the white blows.
            set_weapon(m);
            let before = health(m);
            spells::cast_triggered(m, g, STRIKE, Some(brute));
            out.push(before - health(m));
            (expected, out, off)
        })
        .await
        .unwrap();
    let (expected, out, off) = r;
    let cut = combat::armor_reduction(900, 60);
    assert_eq!(expected, 1_000 - (1_000.0 * cut) as u32);
    // Unpenetrated, 3000 would leave much less.
    assert!(expected > 1_000 - (1_000.0 * combat::armor_reduction(3_000, 60)) as u32);
    assert_eq!(out, vec![expected, expected * 2, expected]);
    assert_eq!(off, expected);
}
