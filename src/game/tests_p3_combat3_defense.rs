//! Phase 3's combat tests: a player's defenses (Thistle Tea's
//! `defensive_capabilities_test.exs`): defense skill moving dodge, parry
//! and block alike, its bonuses stacking, the floor at none, parry wanting
//! a weapon in hand, and what is unlearned not had.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::map::auras::{AURA_MOD_SKILL, AURA_MOD_SKILL_TALENT};
use crate::game::map::derive::{SKILL_DEFENSE, SPELL_BLOCK, SPELL_PARRY};
use crate::game::map::{Map, MapHandle, skills, spells};
use crate::game::tests_p3_combat::{OFF_HAND, SHIELD, SWORD, aura_of, equip, level};

async fn defender() -> (crate::game::harness::Client, Guid, MapHandle) {
    let realm = crate::game::harness::realm_with(crate::game::tests_p3_combat::data());
    let mut a = crate::game::harness::Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(g).unwrap().map();
    map.query(move |m| {
        level(m, g, 60);
        for s in [SPELL_PARRY, SPELL_BLOCK] {
            spells::learn(m, g, s);
        }
        equip(m, g, SWORD, 15);
        equip(m, g, SHIELD, OFF_HAND);
        let p = m.players.get_mut(&g).unwrap();
        skills::learn_skill(p, SKILL_DEFENSE, 300);
        skills::set_value(p, SKILL_DEFENSE, 300);
        restat(m, g);
    })
    .await
    .unwrap();
    (a, g, map)
}

fn restat(m: &mut Map, g: Guid) {
    let data = m.realm.data.clone();
    m.players.get_mut(&g).unwrap().apply_stats(&data);
}

/// Dodge, parry and block in hundredths of a percent.
fn chances(m: &Map, g: Guid) -> [i32; 3] {
    let f = &m.players[&g].fields;
    [
        PLAYER_DODGE_PERCENTAGE,
        PLAYER_PARRY_PERCENTAGE,
        PLAYER_BLOCK_PERCENTAGE,
    ]
    .map(|i| (f.get_f32(i) * 100.0).round() as i32)
}

fn defense(m: &mut Map, g: Guid, to: u32) {
    let p = m.players.get_mut(&g).unwrap();
    skills::set_value(p, SKILL_DEFENSE, to);
    restat(m, g);
}

#[tokio::test]
async fn defense_skill_moves_every_defense_learned_and_its_bonuses_stack() {
    let (_a, g, map) = defender().await;
    let got = map
        .query(move |m| {
            let at = chances(m, g);
            defense(m, g, 295);
            let below = chances(m, g);
            defense(m, g, 300);
            // A five and a ten: fifteen more.
            aura_of(m, g, 95_800, AURA_MOD_SKILL, SKILL_DEFENSE as i32, 5);
            aura_of(
                m,
                g,
                95_801,
                AURA_MOD_SKILL_TALENT,
                SKILL_DEFENSE as i32,
                10,
            );
            restat(m, g);
            let above = chances(m, g);
            (at, below, above)
        })
        .await
        .unwrap();
    let (at, below, above) = got;
    // A warrior without agility dodges none at 300, so only up.
    for i in 0..3 {
        if i > 0 {
            assert_eq!(at[i] - below[i], 20, "five points down: 0.2% each");
        }
        assert_eq!(above[i] - at[i], 60, "fifteen up: 0.6% each");
    }
    assert_eq!([at[1], at[2]], [500, 500], "parry and block at 5%");
}

#[tokio::test]
async fn a_defense_too_low_floors_at_none() {
    let (_a, g, map) = defender().await;
    let got = map
        .query(move |m| {
            defense(m, g, 1);
            chances(m, g)
        })
        .await
        .unwrap();
    assert_eq!(got, [0, 0, 0]);
}

#[tokio::test]
async fn parry_wants_a_weapon_in_hand_whole_and_no_feral_form_and_neither_is_had_unlearned() {
    let (_a, g, map) = defender().await;
    let got = map
        .query(move |m| {
            let mut out = Vec::new();
            // Broken: no parry.
            let sword = m.players[&g]
                .items
                .iter()
                .position(|i| i.entry == SWORD)
                .unwrap();
            let p = m.players.get_mut(&g).unwrap();
            p.items[sword].fields.set_u32(ITEM_FIELD_MAXDURABILITY, 50);
            p.items[sword].fields.set_u32(ITEM_FIELD_DURABILITY, 0);
            restat(m, g);
            out.push(chances(m, g)[1]);
            let p = m.players.get_mut(&g).unwrap();
            p.items[sword].fields.set_u32(ITEM_FIELD_DURABILITY, 50);
            restat(m, g);
            out.push(chances(m, g)[1]);
            // In a feral form (a warrior's fields set as a bear's): none.
            m.players.get_mut(&g).unwrap().fields.set_byte(
                UNIT_FIELD_BYTES_1,
                2,
                crate::game::map::auras::FORM_BEAR,
            );
            restat(m, g);
            out.push(chances(m, g)[1]);
            m.players
                .get_mut(&g)
                .unwrap()
                .fields
                .set_byte(UNIT_FIELD_BYTES_1, 2, 0);
            // Unarmed: none.
            m.players
                .get_mut(&g)
                .unwrap()
                .items
                .retain(|i| i.entry != SWORD);
            restat(m, g);
            out.push(chances(m, g)[1]);
            // Parry and Block unlearned: neither; dodge stays.
            equip(m, g, SWORD, 15);
            let before = chances(m, g);
            crate::game::map::misc::forget_spells(m, g, &[SPELL_PARRY, SPELL_BLOCK]);
            restat(m, g);
            let after = chances(m, g);
            // Learned again: both back.
            for s in [SPELL_PARRY, SPELL_BLOCK] {
                spells::learn(m, g, s);
            }
            restat(m, g);
            (out, before, after, chances(m, g))
        })
        .await
        .unwrap();
    let (out, before, after, again) = got;
    assert_eq!(out, [0, 500, 0, 0]);
    assert_eq!([after[1], after[2]], [0, 0]);
    assert_eq!(after[0], before[0], "dodge stays");
    assert_eq!(again, before);
}
