//! Round 9's player on the real world: spells learned in a form, secondary
//! skills, an expert book (Thistle Tea's `form_passives_dbc_test`,
//! `spells_dbc_test` and `teaching_dbc_test`).

use std::sync::LazyLock;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::fields::PLAYER_CHARACTER_POINTS2;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapHandle, MapKey, auras, skills, spells};
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
    let _ = MapKey::world(0);
    Some((a, g, map))
}

fn skill(m: &Map, g: Guid, id: u32) -> Option<(u32, u32)> {
    let p = &m.players[&g];
    Some((skills::value(p, id)?, skills::max_value(p, id)?))
}

fn has(m: &Map, g: Guid, spell: u32) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has_spell(spell))
}

/// Thistle Tea's "learning and upgrading in form update stats without
/// duplicate ranks": a druid in Cat Form learning Sharpened Claws (16942)
/// has its aura at once; its second rank (16943) takes the first's place,
/// the two never together.
#[tokio::test]
#[ignore = "extended"]
async fn a_talent_rank_learned_in_cat_form_replaces_the_one_below() {
    let Some((_a, g, map)) = player(Race::NightElf, Class::Druid).await else {
        return;
    };
    let seen = on(&map, move |m| {
        crate::game::tests_p3_combat::level(m, g, 60);
        spells::cast_triggered(m, g, 768, Some(g));
        spells::learn(m, g, 16_942);
        let first = has(m, g, 16_942);
        spells::learn(m, g, 16_943);
        (
            first,
            has(m, g, 16_942),
            has(m, g, 16_943),
            m.players[&g].spells.contains(&16_942),
        )
    })
    .await;
    assert_eq!(seen, (true, false, true, false));
}

/// Thistle Tea's "secondary skills and riding use their learned rank
/// values" and "ordinary spell training does not add profession skills":
/// Cooking, First Aid and Fishing come at 1 of 75, leaving both
/// professions free; Unending Breath brings no Enchanting. Riding (33388)
/// is 75 of 75, its next rank (33391) 150 of 150.
#[tokio::test]
#[ignore = "extended"]
async fn secondary_skills_start_at_their_rank_and_take_no_profession() {
    let Some((_a, g, map)) = player(Race::Human, Class::Warlock).await else {
        return;
    };
    let seen = on(&map, move |m| {
        let free = m.players[&g].fields.get_u32(PLAYER_CHARACTER_POINTS2);
        for s in [2550, 3273, 7620, 5697] {
            spells::learn(m, g, s);
        }
        (
            [185, 129, 356].map(|s| skill(m, g, s)),
            skill(m, g, 333),
            free,
            m.players[&g].fields.get_u32(PLAYER_CHARACTER_POINTS2),
            {
                spells::learn(m, g, 33_388);
                let apprentice = skill(m, g, skills::RIDING);
                spells::learn(m, g, 33_391);
                (apprentice, skill(m, g, skills::RIDING))
            },
        )
    })
    .await;
    assert_eq!(seen.0, [Some((1, 75)); 3]);
    assert_eq!(seen.1, None, "no Enchanting");
    assert_eq!(seen.2, seen.3, "the professions free");
    assert_eq!(seen.4, (Some((75, 75)), Some((150, 150))), "riding's ranks");
}

/// Thistle Tea's "expert books grant the new rank while preserving earned
/// skill": the Expert Cookbook's spell (19886) at Journeyman Cooking with
/// 125 teaches Expert Cooking (3413), its cap 225 and the 125 kept.
#[tokio::test]
#[ignore = "extended"]
async fn an_expert_book_raises_the_cap_and_keeps_the_skill() {
    let Some((_a, g, map)) = player(Race::Human, Class::Warrior).await else {
        return;
    };
    let seen = on(&map, move |m| {
        crate::game::tests_p3_combat::level(m, g, 35);
        spells::learn(m, g, 2550);
        spells::learn(m, g, 3102);
        skills::set_value(m.players.get_mut(&g).unwrap(), 185, 125);
        let before = skill(m, g, 185);
        spells::cast_triggered(m, g, 19_886, Some(g));
        (
            before,
            skill(m, g, 185),
            m.players[&g].spells.contains(&3413),
        )
    })
    .await;
    assert_eq!(seen, (Some((125, 150)), Some((125, 225)), true));
}
