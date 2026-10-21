//! Round 10's player on the real world: a talent reset's weapon skills,
//! Feline Swiftness's hidden passives, a skill step below the value, an
//! invalid lesson (Thistle Tea's `talents_dbc_test`, `form_passives_dbc_test`
//! and `teaching_dbc_test`).

use std::sync::LazyLock;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapHandle, auras, misc, skills, spells};
use crate::game::tests_p3_entity::on;

static DB: LazyLock<std::path::PathBuf> = LazyLock::new(|| harness::real_data("db"));

fn have_data() -> bool {
    DB.as_path().join("vmangos.sqlite").exists()
}

async fn player(race: Race, class: Class) -> Option<(Client, Guid, MapHandle)> {
    if !have_data() {
        return None;
    }
    let realm = harness::realm_with(harness::load_real(DB.as_path()));
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Learner", race, class).await;
    let map = realm.registry.online(g).unwrap().map();
    Some((a, g, map))
}

fn skill(m: &Map, g: Guid, id: u32) -> Option<(u32, u32)> {
    let p = &m.players[&g];
    Some((skills::value(p, id)?, skills::max_value(p, id)?))
}

fn has(m: &Map, g: Guid, spell: u32) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has_spell(spell))
}

/// Thistle Tea's "forgets shaman two-handed proficiencies while retaining
/// learned skill values for retraining", as mangos's 1.11 rule has it
/// (`UpdateSpellTrainedSkills`, `m_forgottenSkills`): a tauren shaman's
/// talent (16269) teaches Two-Handed Axes (197) and Maces (199) and their
/// skills (172, 160); a reset takes the spells and the skills, Staves (54)
/// staying, and the talent learned again brings the skills back at what
/// they had reached.
#[tokio::test]
#[ignore = "extended"]
async fn a_talent_reset_forgets_its_weapon_skills_and_a_retrain_brings_them_back() {
    let Some((_a, g, map)) = player(Race::Tauren, Class::Shaman).await else {
        return;
    };
    let seen = on(&map, move |m| {
        crate::game::tests_p3_combat::level(m, g, 60);
        for s in [16_269, 197, 199] {
            spells::learn(m, g, s);
        }
        let learned = (skill(m, g, 172), skill(m, g, 160));
        let p = m.players.get_mut(&g).unwrap();
        skills::set_value(p, 172, 245);
        skills::set_value(p, 160, 190);
        let staves = skill(m, g, 54);
        let data = m.realm.data.clone();
        let talents = misc::talent_spells(&m.players[&g], &data);
        misc::forget_spells(m, g, &talents);
        let p = &m.players[&g];
        let reset = (
            [16_269, 197, 199].map(|s| p.spells.contains(&s)),
            skill(m, g, 172),
            skill(m, g, 160),
            skill(m, g, 54),
        );
        for s in [16_269, 197, 199] {
            spells::learn(m, g, s);
        }
        (learned, staves, reset, (skill(m, g, 172), skill(m, g, 160)))
    })
    .await;
    let (learned, staves, reset, back) = seen;
    assert_eq!(learned, (Some((1, 300)), Some((1, 300))), "learned at 1");
    assert_eq!(reset.0, [false; 3], "the talent's spells go");
    assert_eq!((reset.1, reset.2), (None, None), "and their skills");
    assert_eq!(reset.3, staves, "Staves stays");
    assert_eq!(
        back,
        (Some((245, 300)), Some((190, 300))),
        "kept for a retrain"
    );
}

/// Thistle Tea's "hidden dodge survives indoors independently of speed and
/// follows rank and unlearning", as mangos's `addSpell` and `removeSpell`
/// take a talent rank's dependents: Feline Swiftness's first rank (17002)
/// brings its hidden dodge (24867), its second (24866) the next (24864),
/// which takes the first's place, in the spellbook and its auras; a reset
/// takes both.
#[tokio::test]
#[ignore = "extended"]
async fn feline_swiftness_swaps_its_hidden_passive_with_its_rank() {
    let Some((_a, g, map)) = player(Race::NightElf, Class::Druid).await else {
        return;
    };
    let seen = on(&map, move |m| {
        crate::game::tests_p3_combat::level(m, g, 60);
        spells::cast_triggered(m, g, 768, Some(g));
        let state = |m: &Map| {
            let p = &m.players[&g];
            (
                [17_002, 24_867, 24_866, 24_864].map(|s| p.spells.contains(&s)),
                [17_002, 24_867, 24_866, 24_864].map(|s| has(m, g, s)),
            )
        };
        spells::learn(m, g, 17_002);
        let first = state(m);
        spells::learn(m, g, 24_866);
        let second = state(m);
        let data = m.realm.data.clone();
        let talents = misc::talent_spells(&m.players[&g], &data);
        misc::forget_spells(m, g, &talents);
        (first, second, state(m))
    })
    .await;
    assert_eq!(seen.0.0, [true, true, false, false], "the first rank");
    assert!(seen.0.1[1], "its hidden dodge on");
    assert_eq!(
        seen.1.0,
        [false, false, true, true],
        "the second takes its place"
    );
    assert_eq!(&seen.1.1[..2], &[false; 2], "the first's auras gone");
    assert!(seen.1.1[3], "the second's hidden dodge on");
    assert_eq!(seen.2, ([false; 4], [false; 4]), "a reset takes both");
}

/// Thistle Tea's "skill-only effects set exact caps and survive later
/// unrelated learning", as mangos's `SetSkillStep` keeps the value it has:
/// Apprentice Cooking's step (1) on one at 125 sets the cap to 75 and
/// keeps the 125; learning Unending Breath after changes nothing of it.
#[tokio::test]
#[ignore = "extended"]
async fn a_lower_skill_step_keeps_the_value_over_its_cap() {
    let Some((_a, g, map)) = player(Race::Human, Class::Warlock).await else {
        return;
    };
    let seen = on(&map, move |m| {
        crate::game::tests_p3_combat::level(m, g, 35);
        spells::learn(m, g, 2550);
        spells::learn(m, g, 3102);
        skills::set_value(m.players.get_mut(&g).unwrap(), 185, 125);
        let step = |m: &Map| {
            let p = &m.players[&g];
            let slot = (0..128u16)
                .find(|i| {
                    p.fields
                        .get_u32(crate::game::fields::PLAYER_SKILL_INFO_1_1 + i * 3)
                        & 0xFFFF
                        == 185
                })
                .unwrap();
            p.fields
                .get_u32(crate::game::fields::PLAYER_SKILL_INFO_1_1 + slot * 3)
                >> 16
        };
        let before = (skill(m, g, 185), step(m));
        skills::learn_skill(m.players.get_mut(&g).unwrap(), 185, 1);
        let after = (skill(m, g, 185), step(m));
        spells::learn(m, g, 5697);
        (before, after, (skill(m, g, 185), step(m)))
    })
    .await;
    assert_eq!(seen.0, (Some((125, 150)), 2));
    assert_eq!(seen.1, (Some((125, 75)), 1), "the value kept over the cap");
    assert_eq!(seen.2, seen.1, "unrelated learning changes nothing");
}
