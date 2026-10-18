//! Phase 3, round 4, against Thistle Tea's `skills_test.exs`: a skill's
//! slot in the fields (its id and step, its value and cap, its bonuses
//! apart), slots kept where they are, a profession's ranks, the weapon
//! skills a weapon trains, and what a swing never trains.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::map::combat::Outcome;
use crate::game::map::skills;
use crate::game::map::{Map, auras, derive, gm_dev};
use crate::game::tests_p3_combat::*;

/// The three words of `skill`'s slot, and which slot.
fn slot(m: &Map, g: Guid, skill: u32) -> Option<(u16, [u32; 3])> {
    let f = &m.players[&g].fields;
    (0..128u16).find_map(|i| {
        let base = PLAYER_SKILL_INFO_1_1 + i * 3;
        (f.get_u32(base) & 0xFFFF == skill).then(|| {
            (
                i,
                [f.get_u32(base), f.get_u32(base + 1), f.get_u32(base + 2)],
            )
        })
    })
}

fn stacked(m: &mut Map, g: Guid, spell: u32, n: u32) {
    let p = m.players.get_mut(&g).unwrap();
    for h in p.auras.holders.iter_mut().filter(|h| h.spell == spell) {
        h.stacks = n;
    }
    derive::unit(m, g);
}

#[tokio::test]
async fn a_skills_bonuses_go_in_its_slot_apart_from_its_value_signed_and_by_their_stacks() {
    let (_realm, _a, g, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, g, 60);
            let p = m.players.get_mut(&g).unwrap();
            skills::learn_skill(p, 95, 1);
            let plain = slot(m, g, 95).unwrap().1;
            // +3 of ten stacks, -5 of two: temporary; +2 of three: permanent.
            aura_of(m, g, 96_001, auras::AURA_MOD_SKILL, 95, 3);
            stacked(m, g, 96_001, 10);
            aura_of(m, g, 96_002, auras::AURA_MOD_SKILL, 95, -5);
            stacked(m, g, 96_002, 2);
            aura_of(m, g, 96_003, auras::AURA_MOD_SKILL_TALENT, 95, 2);
            stacked(m, g, 96_003, 3);
            let bonused = slot(m, g, 95).unwrap().1;
            let with = skills::value_with_bonuses(&m.players[&g], 95);
            // A penalty alone is a negative half.
            auras::remove_spell(m, g, 96_001, None);
            auras::remove_spell(m, g, 96_003, None);
            derive::unit(m, g);
            let negative = slot(m, g, 95).unwrap().1;
            auras::remove_spell(m, g, 96_002, None);
            derive::unit(m, g);
            (plain, bonused, with, negative, slot(m, g, 95).unwrap().1)
        })
        .await
        .unwrap();
    let (plain, bonused, with, negative, gone) = r;
    assert_eq!(plain[1], 300 | 300 << 16);
    assert_eq!(plain[2], 0);
    // The value and cap are as they were; 20 and 6 beside them.
    assert_eq!(bonused[1], plain[1]);
    assert_eq!(bonused[2], 20 | 6 << 16);
    assert_eq!(with, 326);
    assert_eq!(negative[2], u32::from((-10i16) as u16));
    assert_eq!(gone, plain);
}

#[tokio::test]
async fn a_skills_slot_holds_its_id_and_step_then_value_and_cap_and_slots_stay_put() {
    let (_realm, _a, g, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            let p = m.players.get_mut(&g).unwrap();
            // Apprentice fishing: 1 of 75, step 1.
            skills::learn_skill(p, 356, 1);
            let apprentice = slot(m, g, 356).unwrap();
            let p = m.players.get_mut(&g).unwrap();
            skills::set_value(p, 356, 50);
            // Journeyman keeps the 50 and raises the cap and step.
            skills::learn_skill(p, 356, 2);
            let journeyman = slot(m, g, 356).unwrap();
            // Mining, then Blacksmithing (a lower number) after it.
            let p = m.players.get_mut(&g).unwrap();
            skills::learn_skill(p, 186, 1);
            skills::learn_skill(p, 164, 1);
            let (mining, smithing) = (slot(m, g, 186).unwrap(), slot(m, g, 164).unwrap());
            // Mining forgotten: its slot is empty, smithing's where it was;
            // learned again, mining is back in its own.
            let p = m.players.get_mut(&g).unwrap();
            skills::forget_skill(p, 186);
            let f = &m.players[&g].fields;
            let emptied: Vec<u32> = (0..3)
                .map(|k| f.get_u32(PLAYER_SKILL_INFO_1_1 + mining.0 * 3 + k))
                .collect();
            let smithing_after = slot(m, g, 164).unwrap();
            let p = m.players.get_mut(&g).unwrap();
            skills::learn_skill(p, 186, 1);
            let relearned = slot(m, g, 186).unwrap();
            // The slots after the last skill are empty.
            let f = &m.players[&g].fields;
            let last = (0..128u16)
                .rfind(|i| f.get_u32(PLAYER_SKILL_INFO_1_1 + i * 3) != 0)
                .unwrap();
            let rest_empty = (last + 1..128)
                .all(|i| (0..3).all(|k| f.get_u32(PLAYER_SKILL_INFO_1_1 + i * 3 + k) == 0));
            (
                apprentice,
                journeyman,
                mining,
                smithing,
                emptied,
                smithing_after,
                relearned,
                rest_empty,
            )
        })
        .await
        .unwrap();
    let (apprentice, journeyman, mining, smithing, emptied, smithing_after, relearned, rest_empty) =
        r;
    assert_eq!(apprentice.1, [356 | 1 << 16, 1 | 75 << 16, 0]);
    assert_eq!(
        journeyman,
        (apprentice.0, [356 | 2 << 16, 50 | 150 << 16, 0])
    );
    assert!(smithing.0 > mining.0);
    assert_eq!(emptied, vec![0, 0, 0]);
    assert_eq!(smithing_after, smithing);
    assert_eq!(relearned, mining);
    assert!(rest_empty);
}

#[tokio::test]
async fn the_debug_commands_max_the_skills_and_only_the_known_professions() {
    let (_realm, _a, g, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, g, 50);
            let p = m.players.get_mut(&g).unwrap();
            skills::learn_skill(p, 43, 1);
            skills::set_value(p, 43, 1);
            skills::learn_skill(p, 186, 2);
            skills::set_value(p, 186, 75);
            skills::learn_skill(p, 98, 1);
            skills::learn_skill(p, 415, 1);
            let read = |m: &Map| {
                let p = &m.players[&g];
                [43, 186, 98, 415, 182].map(|s| (skills::value(p, s), skills::max_value(p, s)))
            };
            gm_dev::command(m, g, "debug", &["professions"]);
            let professions = read(m);
            gm_dev::command(m, g, "debug", &["skills"]);
            (professions, read(m))
        })
        .await
        .unwrap();
    let (professions, all) = r;
    // Mining at 300 of 300; swords as they were; herbalism not learned.
    assert_eq!(professions[0], (Some(1), Some(250)));
    assert_eq!(professions[1], (Some(300), Some(300)));
    assert_eq!(professions[4], (None, None));
    // Every skill at its cap: swords; the language and cloth as they are.
    assert_eq!(all[0], (Some(250), Some(250)));
    assert_eq!(all[2], (Some(300), Some(300)));
    assert_eq!(all[3], (Some(1), Some(1)));
}

const BOW: u32 = 96_101;
const DAGGER: u32 = 96_102;

#[tokio::test]
async fn each_hand_trains_and_rolls_its_own_weapons_skill() {
    use crate::game::data::items::ItemTemplate;
    let mut d = data();
    for (entry, subclass, inventory_type) in [(BOW, 2, 15), (DAGGER, 15, 13)] {
        d.items.insert(
            entry,
            ItemTemplate {
                entry,
                class: 2,
                subclass,
                name: format!("Weapon {entry}"),
                display_id: 1000,
                inventory_type,
                delay: 2000,
                stackable: 1,
                ..Default::default()
            },
        );
    }
    let (_realm, _a, g, map) = setup_with(d, Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, g, 60);
            let p = m.players.get_mut(&g).unwrap();
            skills::learn_skill(p, 45, 1);
            skills::set_value(p, 45, 120);
            skills::learn_skill(p, 173, 1);
            skills::set_value(p, 173, 80);
            equip(m, g, DAGGER, 15);
            equip(m, g, BOW, 17);
            derive::unit(m, g);
            let d = &m.players[&g].derived;
            (d.weapon_skill, d.ranged_skill)
        })
        .await
        .unwrap();
    assert_eq!(r, (80, 120));
    assert_eq!([7, 10, 15, 2].map(derive::weapon_skill), [43, 136, 173, 45]);
    // A fishing pole (subclass 20) or anything else isn't a weapon's skill:
    // it swings as unarmed.
    assert_eq!(derive::weapon_skill(20), 162);
}

#[tokio::test]
async fn a_swing_trains_no_skill_at_its_cap_nor_one_the_player_hasnt() {
    let (_realm, _a, g, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, g, 60);
            equip(m, g, SWORD, 15);
            let p = m.players.get_mut(&g).unwrap();
            skills::learn_skill(p, 43, 1);
            skills::set_value(p, 43, 300);
            let brute = spawn(m, g, BRUTE, 60, 1.0);
            for _ in 0..5 {
                forced_swing(m, g, brute, Outcome::Hit);
            }
            let capped = skills::value(&m.players[&g], 43);
            // Without the sword's skill, a swing teaches none.
            let p = m.players.get_mut(&g).unwrap();
            skills::forget_skill(p, 43);
            for _ in 0..5 {
                forced_swing(m, g, brute, Outcome::Hit);
            }
            (capped, skills::value(&m.players[&g], 43))
        })
        .await
        .unwrap();
    assert_eq!(r, (Some(300), None));
}
