//! Phase 6's loader round, the items and professions (Thistle Tea's
//! `world/loader/` tests of items, sets, enchantments, recipes and skill
//! lines): what wvs reads of them, and what they do.

use crate::game::data::GameData;

fn real() -> Option<&'static GameData> {
    crate::game::data::load::real_data()
}

/// The primary professions' skill lines.
const PRIMARY: [u32; 9] = [164, 165, 171, 182, 186, 197, 202, 333, 393];

/// Thistle Tea's skill catalog (`profession_dbc_test.exs`): Smelt Copper
/// (2657) is mining's, yellow at 25 and grey at 70; Minor Healing Potion
/// (2330) alchemy's, 55 and 95; Charred Wolf Meat (2538) cooking's, 45
/// and 85; Conjure Water is no profession's. The nine primary professions
/// are SkillLine's category 11, each may be unlearned (its
/// SkillRaceClassInfo flag, mangos's `SKILL_FLAG_CAN_UNLEARN`) and teaches
/// its training spell; swords, defense, cooking, first aid and fishing may
/// not be; mining has its smelting.
#[test]
#[ignore = "extended"]
fn real_professions_have_their_recipes_and_are_unlearned_alone() {
    let Some(d) = real() else {
        return;
    };
    let recipe = |spell: u32| {
        d.spells.skill_abilities.get(&spell).and_then(|v| {
            v.iter()
                .find(|a| a.trivial_high > 0)
                .map(|a| (a.skill, a.trivial_low, a.trivial_high))
        })
    };
    assert_eq!(recipe(2657), Some((186, 25, 70)));
    assert_eq!(recipe(2330), Some((171, 55, 95)));
    assert_eq!(recipe(2538), Some((185, 45, 85)));
    assert_eq!(recipe(5504), None);
    // SkillLine's category 11, straight from the client's data.
    let dbc = rusqlite::Connection::open(harness_db().join("dbc.sqlite")).unwrap();
    let mut primary: Vec<u32> = dbc
        .prepare("SELECT id FROM SkillLine WHERE category = 11")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    primary.sort_unstable();
    assert_eq!(primary, PRIMARY);
    assert_eq!(crate::game::map::npc::PRIMARY_PROFESSIONS, PRIMARY);
    let unlearnable = |skill| crate::game::map::misc::unlearnable(d, skill, 1, 1);
    for (skill, training) in [
        (164, 2018),
        (165, 2108),
        (171, 2259),
        (182, 2366),
        (186, 2575),
        (197, 3908),
        (202, 4036),
        (333, 7411),
        (393, 8613),
    ] {
        assert!(unlearnable(skill), "{skill}");
        assert_eq!(
            d.spells.spell_skills.get(&training),
            Some(&skill),
            "{training}"
        );
    }
    for skill in [43, 95, 185, 129, 356] {
        assert!(!unlearnable(skill), "{skill}");
    }
    for spell in [2580, 2656, 2657, 2658, 2576, 10_248] {
        assert_eq!(d.spells.spell_skills.get(&spell), Some(&186), "{spell}");
    }
}

fn harness_db() -> std::path::PathBuf {
    crate::game::harness::real_data("db")
}
