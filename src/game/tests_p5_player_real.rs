//! Round 5's player tests on the real world, ported from Thistle Tea's
//! `test/game/world/entity/player/spells_dbc_test.exs` (docs/parity/matrix.csv):
//! what learning a profession, a language or an active spell gives.

use std::sync::LazyLock;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::harness::{self, Client};
use crate::game::map::{Map, skills, spells};
use crate::game::tests_p3_entity::on;

static DB: LazyLock<std::path::PathBuf> = LazyLock::new(|| harness::real_data("db"));

fn have_data() -> bool {
    DB.as_path().join("vmangos.sqlite").exists()
}

fn skill(m: &Map, g: Guid, s: u32) -> Option<(u32, u32)> {
    let p = &m.players[&g];
    skills::value(p, s).zip(skills::max_value(p, s))
}

/// Thistle Tea's `Spells.learn` "does not automatically cast an active
/// spell", "direct profession learning grants its rank, tools and starting
/// recipes" and "publishes language skill changes on learning": Unending
/// Breath learned is no aura; Apprentice Mining is Mining at 1 of 75 with
/// Find Minerals, Smelting and Smelt Copper, not Smelt Tin; Journeyman
/// Mining keeps the skill earned and raises the cap to 150; a language
/// learned is its skill at 300.
#[tokio::test]
#[ignore = "extended"]
async fn a_profession_learned_brings_its_rank_and_recipes_and_an_active_spell_nothing() {
    const UNENDING_BREATH: u32 = 5697;
    const MINING: u32 = 186;
    const APPRENTICE_MINING: u32 = 2575;
    const JOURNEYMAN_MINING: u32 = 2576;
    const DWARVEN: u32 = 672;
    if !have_data() {
        return;
    }
    let realm = harness::realm_with(harness::load_real(DB.as_path()));
    let mut c = Client::connect(&realm, "A");
    let g = c.enter("Miner", Race::Human, Class::Mage).await;
    let map = realm.maps.get(&realm, crate::game::map::MapKey::world(0));
    let out = on(&map, move |m| {
        spells::learn(m, g, UNENDING_BREATH);
        let breath = m.players[&g].auras.has_spell(UNENDING_BREATH);
        spells::learn(m, g, APPRENTICE_MINING);
        let apprentice = skill(m, g, MINING);
        let known = |m: &Map, s: u32| m.players[&g].spells.contains(&s);
        let recipes = [2580, 2656, 2657].map(|s| known(m, s));
        let tin = known(m, 2658);
        let p = m.players.get_mut(&g).unwrap();
        skills::set_value(p, MINING, 50);
        spells::learn(m, g, JOURNEYMAN_MINING);
        let journeyman = skill(m, g, MINING);
        spells::learn(m, g, DWARVEN);
        let language = skill(m, g, 111);
        (breath, apprentice, recipes, tin, journeyman, language)
    })
    .await;
    let (breath, apprentice, recipes, tin, journeyman, language) = out;
    assert!(!breath, "an active spell isn't cast");
    assert_eq!(apprentice, Some((1, 75)), "Apprentice Mining");
    assert_eq!(recipes, [true; 3], "Find Minerals, Smelting, Smelt Copper");
    assert!(!tin, "not Smelt Tin");
    assert_eq!(
        journeyman,
        Some((50, 150)),
        "the skill kept, the cap raised"
    );
    assert_eq!(language, Some((300, 300)), "Dwarven");
    // Mining given up: its spells go with it, Cooking stays; learned again,
    // it starts over.
    on(&map, move |m| spells::learn(m, g, 2550)).await;
    c.send(CMSG_UNLEARN_SKILL {
        skill: Skill::Mining,
    })
    .await;
    crate::game::tests_p3_player2_ammo::handled(&mut c).await;
    let (gone, recipes, cooking) = on(&map, move |m| {
        let known = |s: u32| m.players[&g].spells.contains(&s);
        (
            skill(m, g, MINING),
            [APPRENTICE_MINING, JOURNEYMAN_MINING, 2580, 2656, 2657].map(known),
            skill(m, g, 185),
        )
    })
    .await;
    assert_eq!(gone, None, "Mining unlearned");
    assert_eq!(recipes, [false; 5], "and its spells");
    assert_eq!(cooking, Some((1, 75)), "Cooking kept");
    let again = on(&map, move |m| {
        spells::learn(m, g, APPRENTICE_MINING);
        (skill(m, g, MINING), m.players[&g].spells.contains(&2657))
    })
    .await;
    assert_eq!(again, (Some((1, 75)), true), "learned again from the start");
}

/// Thistle Tea's `Spells.apply_passives` "replaces lower talent ranks
/// instead of stacking their modifiers": Improved Fireball's fifth rank
/// learned over its first leaves the fifth's aura alone, and the first's
/// spell gone.
#[tokio::test]
#[ignore = "extended"]
async fn a_talents_rank_replaces_the_one_below() {
    const RANK_ONE: u32 = 11_070;
    const RANK_FIVE: u32 = 16_766;
    if !have_data() {
        return;
    }
    let realm = harness::realm_with(harness::load_real(DB.as_path()));
    let mut c = Client::connect(&realm, "A");
    let g = c.enter("Pyro", Race::Human, Class::Mage).await;
    let map = realm.maps.get(&realm, crate::game::map::MapKey::world(0));
    let out = on(&map, move |m| {
        let data = m.realm.data.clone();
        let (talent, ranks) = data
            .spells
            .talents
            .iter()
            .find(|(_, t)| t.ranks.contains(&RANK_ONE))
            .map(|(id, t)| (*id, t.ranks))
            .expect("Improved Fireball");
        let p = m.players.get_mut(&g).unwrap();
        crate::game::map::stats::set_level(p, &data, 60);
        spells::learn_talent(m, g, talent, 0);
        let first = m.players[&g].auras.has_spell(RANK_ONE);
        let five = ranks.iter().position(|r| *r == RANK_FIVE).unwrap() as u32;
        spells::learn_talent(m, g, talent, five);
        let p = &m.players[&g];
        let on: Vec<u32> = p
            .auras
            .holders
            .iter()
            .map(|h| h.spell)
            .filter(|s| ranks.contains(s))
            .collect();
        (
            first,
            on,
            p.spells.contains(&RANK_ONE),
            p.spells.contains(&RANK_FIVE),
        )
    })
    .await;
    assert_eq!(out, (true, vec![RANK_FIVE], false, true));
}
