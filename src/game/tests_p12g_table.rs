//! Round 12's combat table, as mangos-classic has it: crushing blows
//! against a defense counted no higher than its level's cap; a player's
//! pet or charm on the players' side of the table (no crushing blows,
//! glancing ones on a boss, players' factors as a victim, spells missing
//! it by 7% a level); parry by the attacker's whole weapon skill; crowd
//! control taking avoidance away; and a paladin's blows renewing their
//! judgements.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::{ATTR_EX3_ALWAYS_HIT, SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::auras::{self, AURA_MOD_SKILL_TALENT};
use crate::game::map::combat::{self, Hand, Outcome};
use crate::game::map::derive::SKILL_DEFENSE;
use crate::game::map::{Map, pets, skills};
use crate::game::tests_p3_combat::{self as p3, BRUTE, SWORD};

/// The swords skill.
const SWORDS: u32 = 43;
/// A judgement: a paladin's spell that always hits.
const JUDGEMENT: u32 = 96_900;
/// A warrior's, that doesn't.
const REND: u32 = 96_901;

fn data() -> GameData {
    let mut d = p3::data();
    for (id, family, always) in [(JUDGEMENT, 10, true), (REND, 4, false)] {
        let mut s = SpellTemplate {
            id,
            name: format!("Test {id}"),
            family_name: family,
            equipped_item_class: -1,
            ..Default::default()
        };
        if always {
            s.attributes[3] = ATTR_EX3_ALWAYS_HIT;
        }
        s.effects[0] = SpellEffect {
            effect: 6,
            aura: 4,
            base_points: 0,
            base_dice: 1,
            target_a: 6,
            ..Default::default()
        };
        d.spells.spells.insert(id, s);
    }
    d
}

fn restat(m: &mut Map, g: Guid) {
    let data = m.realm.data.clone();
    m.players.get_mut(&g).unwrap().apply_stats(&data);
}

/// Alice at 60 with a sword, her skills as she trained them: swords and
/// defense at `skill`, each with `talent` more from a talent.
fn trained(m: &mut Map, g: Guid, skill: u32, talent: i32) {
    p3::level(m, g, 60);
    p3::equip(m, g, SWORD, 15);
    m.players
        .get_mut(&g)
        .unwrap()
        .fields
        .set_byte(UNIT_FIELD_BYTES_2, 0, 1);
    let p = m.players.get_mut(&g).unwrap();
    p.skills_maxed = false;
    for s in [SKILL_DEFENSE, SWORDS] {
        skills::learn_skill(p, s, 300);
        skills::set_value(p, s, skill);
    }
    if talent > 0 {
        p3::aura_of(
            m,
            g,
            96_910,
            AURA_MOD_SKILL_TALENT,
            SKILL_DEFENSE as i32,
            talent,
        );
        p3::aura_of(m, g, 96_911, AURA_MOD_SKILL_TALENT, SWORDS as i32, talent);
    }
    restat(m, g);
}

/// How many of the table's ten thousand rolls come to `outcome`.
fn share(r: &combat::Roll, outcome: Outcome) -> usize {
    (0..10_000)
        .filter(|roll| combat::roll_outcome(r, *roll) == outcome)
        .count()
}

/// A creature of `level` that Alice charms: a player's.
fn charm(m: &mut Map, g: Guid, level: u8, yards: f32) -> Guid {
    let c = p3::spawn(m, g, BRUTE, level, yards);
    pets::charmed(m, g, c, false);
    assert!(combat::player_controlled(m, c), "charmed");
    c
}

/// mangos's `CalculateEffectiveCrushChance`: a boss of 315 weapon skill
/// crushes a tank of 300 defense 15% of the time, and one of 315 (300 and
/// a talent's 15) as often, her defense past her cap counted no higher;
/// below it, each point short counts twice (290: 35%).
#[tokio::test]
async fn a_bosss_crushing_blows_meet_defense_no_higher_than_the_tanks_cap() {
    let (_realm, _a, g, map) = p3::setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let mut out = Vec::new();
            for (skill, talent) in [(300, 0), (300, 15), (290, 0)] {
                trained(m, g, skill, talent);
                let boss = p3::spawn(m, g, BRUTE, 63, 3.0);
                let r = combat::roll_for(m, boss, g, Hand::Main).unwrap();
                out.push(share(&r, Outcome::Crushing));
                auras::remove_spell(m, g, 96_910, None);
                auras::remove_spell(m, g, 96_911, None);
            }
            out
        })
        .await
        .unwrap();
    assert_eq!(r, [1_500, 1_500, 3_500]);
}

/// mangos's `CanCrush`, `CanGlance` and `CanCrushInCombat`: a player's
/// charm of 60 never crushes a creature of 55, and glances off one of 63
/// as a player would; a creature crushes the charm, a player's, but not
/// another creature.
#[tokio::test]
async fn a_players_charm_glances_and_never_crushes_and_only_the_players_side_is_crushed() {
    let (_realm, _a, g, map) = p3::setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            p3::level(m, g, 60);
            let pet = charm(m, g, 60, 3.0);
            let low = p3::spawn(m, g, BRUTE, 55, 4.0);
            let boss = p3::spawn(m, g, BRUTE, 63, 5.0);
            let plain = p3::spawn(m, g, BRUTE, 60, 6.0);
            let on = |m: &Map, a: Guid, b: Guid| combat::roll_for(m, a, b, Hand::Main).unwrap();
            let low_r = on(m, pet, low);
            let boss_r = on(m, pet, boss);
            let crushed = on(m, boss, pet);
            let creature = on(m, boss, plain);
            (
                share(&low_r, Outcome::Crushing),
                share(&boss_r, Outcome::Glancing),
                share(&boss_r, Outcome::Crushing),
                share(&crushed, Outcome::Crushing),
                share(&creature, Outcome::Crushing),
            )
        })
        .await
        .unwrap();
    // The charm of 300 skill on the boss's 315: glancing 10 + 2 * 15.
    assert_eq!(r, (0, 4_000, 0, 1_500, 0));
}

/// mangos's `CalculateSpellMissChance`: a pet three levels over the caster
/// is missed 7% a level past the second, as a player is, not a creature's
/// 11%.
#[tokio::test]
async fn a_spell_misses_a_players_pet_as_it_would_a_player() {
    let (_realm, _a, g, map) = p3::setup_with(data(), Class::Mage).await;
    let r = map
        .query(move |m| {
            p3::level(m, g, 60);
            let pet = charm(m, g, 63, 3.0);
            let wild = p3::spawn(m, g, BRUTE, 63, 4.0);
            let s = SpellTemplate {
                id: 96_920,
                dmg_class: 1,
                school: 2,
                equipped_item_class: -1,
                ..Default::default()
            };
            let mods = crate::game::map::spells::mods::Mods::default();
            let hit = |t| crate::game::map::spells::outcome::spell_hit_chance(m, g, t, &s, &mods);
            (hit(pet), hit(wild))
        })
        .await
        .unwrap();
    assert_eq!(r, (8_700, 8_300));
}

/// mangos's `CalculateEffectiveParryChance`: a boss three levels over
/// parries 14% of a weapon skill of 300, but only 6% of 305 (a difference
/// of ten, at 0.1% a point), the skill past the cap counting.
#[tokio::test]
async fn weapon_skill_past_the_cap_takes_a_bosss_parry_down() {
    let (_realm, _a, g, map) = p3::setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let mut out = Vec::new();
            for talent in [0, 5] {
                trained(m, g, 300, talent);
                let boss = p3::spawn(m, g, BRUTE, 63, 3.0);
                let r = combat::roll_for(m, g, boss, Hand::Main).unwrap();
                out.push(share(&r, Outcome::Parry));
            }
            out
        })
        .await
        .unwrap();
    assert_eq!(r, [1_400, 600]);
}

/// mangos's `CanReactInCombat`: a creature fleeing or confused, as one
/// stunned, neither dodges, parries nor blocks.
#[tokio::test]
async fn a_feared_or_confused_creature_neither_dodges_parries_nor_blocks() {
    let (_realm, _a, g, map) = p3::setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            trained(m, g, 300, 0);
            let mut out = Vec::new();
            for flag in [0, auras::UNIT_FLAG_FLEEING, auras::UNIT_FLAG_CONFUSED] {
                let c = p3::spawn(m, g, BRUTE, 60, 3.0);
                m.unit_fields_mut(c)
                    .unwrap()
                    .set_flag(UNIT_FIELD_FLAGS, flag, true);
                let r = combat::roll_for(m, g, c, Hand::Main).unwrap();
                out.push(
                    [Outcome::Dodge, Outcome::Parry, Outcome::Block]
                        .map(|o| share(&r, o))
                        .iter()
                        .sum::<usize>(),
                );
            }
            out
        })
        .await
        .unwrap();
    assert!(r[0] > 0, "free, it avoids: {r:?}");
    assert_eq!(&r[1..], &[0, 0]);
}

/// mangos's `DealMeleeDamage`: Alice's blow that hurts gives her
/// judgement on the creature its full ten seconds again; her other
/// auras, and a miss, renew nothing.
#[tokio::test]
async fn a_paladins_blow_renews_their_judgement() {
    let (_realm, _a, g, map) = p3::setup_with(data(), Class::Paladin).await;
    let r = map
        .query(move |m| {
            trained(m, g, 300, 0);
            let c = p3::spawn(m, g, BRUTE, 60, 3.0);
            let data = m.realm.data.clone();
            for id in [JUDGEMENT, REND] {
                let s = data.spells.get(id).unwrap().clone();
                let ctx = auras::Context {
                    duration: Some(Some(10_000)),
                    ..Default::default()
                };
                auras::apply_spell(m, g, c, &s, &[(0, 1)], ctx).unwrap();
            }
            let left = |m: &Map, id: u32| {
                auras::auras(m, c)
                    .and_then(|a| a.holders.iter().find(|h| h.spell == id))
                    .and_then(|h| h.expires)
                    .map(|t| t.saturating_duration_since(m.now))
            };
            m.now += Duration::from_secs(4);
            p3::forced_swing(m, g, c, Outcome::Miss);
            let missed = left(m, JUDGEMENT);
            m.now += Duration::from_secs(2);
            p3::forced_swing(m, g, c, Outcome::Hit);
            (missed, left(m, JUDGEMENT), left(m, REND))
        })
        .await
        .unwrap();
    assert_eq!(r.0, Some(Duration::from_secs(6)), "a miss renews nothing");
    assert_eq!(r.1, Some(Duration::from_secs(10)), "the judgement renewed");
    assert_eq!(r.2, Some(Duration::from_secs(4)), "the rest not");
}
