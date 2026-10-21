//! Round 10, against Thistle Tea's `skills/spell_skills_test.exs`
//! (`remove/3`): a profession's rank forgotten, its skill falls to the
//! rank below, its value kept as far as that cap; its first rank
//! forgotten, the skill goes (mangos-classic's `UpdateSpellTrainedSkills`,
//! `SetSkillStep(skill, step - 1)`).

use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::map::{misc, skills, spells};
use crate::game::tests_p3_combat::{data, setup_with};

const MINING: u32 = 186;
const APPRENTICE: u32 = 98_401;
const JOURNEYMAN: u32 = 98_402;

#[tokio::test]
async fn a_rank_forgotten_takes_its_skill_to_the_rank_below_and_the_first_takes_it_all() {
    let mut d = data();
    for (id, step) in [(APPRENTICE, 1), (JOURNEYMAN, 2)] {
        let mut s = SpellTemplate {
            id,
            name: format!("Mining {step}"),
            equipped_item_class: -1,
            ..Default::default()
        };
        s.effects[0] = SpellEffect {
            effect: spells::EFFECT_SKILL,
            misc_value: MINING as i32,
            base_points: step - 1,
            base_dice: 1,
            target_a: 1,
            ..Default::default()
        };
        d.spells.spells.insert(id, s);
    }
    let (_realm, _a, g, map) = setup_with(d, Class::Warrior).await;
    let r = map
        .query(move |m| {
            let data = m.realm.data.clone();
            let p = m.players.get_mut(&g).unwrap();
            for s in [APPRENTICE, JOURNEYMAN] {
                p.spells.push(s);
                skills::learn_from_spell(p, &data, s);
            }
            skills::set_value(p, MINING, 100);
            let read = |m: &crate::game::map::Map| {
                let p = &m.players[&g];
                (skills::value(p, MINING), skills::max_value(p, MINING))
            };
            let learned = read(m);
            misc::forget_spells(m, g, &[JOURNEYMAN]);
            let lower = read(m);
            // Again: it isn't known now, and changes nothing.
            misc::forget_spells(m, g, &[JOURNEYMAN]);
            let again = read(m);
            misc::forget_spells(m, g, &[APPRENTICE]);
            (learned, lower, again, read(m))
        })
        .await
        .unwrap();
    let (learned, lower, again, gone) = r;
    assert_eq!(learned, (Some(100), Some(150)));
    assert_eq!(lower, (Some(75), Some(75)));
    assert_eq!(again, lower);
    assert_eq!(gone, (None, None));
}

const RIDING: u32 = 762;
const RIDING_SPELL: u32 = 98_403;
const NO_SKILL: u32 = 98_404;

/// Against `grants/1`: a riding rank is had whole, 75 a step (150 of 150
/// at the second), as mangos's `SetSkillStep` from Riding's tiers; a skill
/// effect of no skill teaches nothing.
#[tokio::test]
async fn a_riding_rank_is_had_whole_and_a_skill_effect_of_no_skill_teaches_nothing() {
    let mut d = data();
    for (id, skill) in [(RIDING_SPELL, RIDING), (NO_SKILL, 0)] {
        let mut s = SpellTemplate {
            id,
            name: format!("Skill {id}"),
            equipped_item_class: -1,
            ..Default::default()
        };
        s.effects[0] = SpellEffect {
            effect: spells::EFFECT_SKILL,
            misc_value: skill as i32,
            base_points: 1,
            base_dice: 1,
            target_a: 1,
            ..Default::default()
        };
        d.spells.spells.insert(id, s);
    }
    let (_realm, _a, g, map) = setup_with(d, Class::Warrior).await;
    let r = map
        .query(move |m| {
            let data = m.realm.data.clone();
            let p = m.players.get_mut(&g).unwrap();
            let before = skills::known(p).len();
            skills::learn_from_spell(p, &data, RIDING_SPELL);
            skills::learn_from_spell(p, &data, NO_SKILL);
            (
                skills::value(p, RIDING),
                skills::max_value(p, RIDING),
                skills::known(p).len() - before,
            )
        })
        .await
        .unwrap();
    assert_eq!(r, (Some(150), Some(150), 1));
}
