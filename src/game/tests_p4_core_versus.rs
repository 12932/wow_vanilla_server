//! Phase 3, round 4, against Thistle Tea's
//! `stats/target_attack_power_test.exs`: attack power against creature
//! types (each bit of the mask, by stacks, melee and ranged apart), a
//! player a humanoid but in a beast's form a beast, a mark's attack power
//! on its target, damage and crits against a type, and no weapon-speed
//! bonus on a flat ability.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::combat::{self, Hand};
use crate::game::map::{Map, auras, spells};
use crate::game::tests_p3_combat::*;

const GHOUL: u32 = 3_730;

fn world() -> GameData {
    let mut d = data();
    d.creature_templates.insert(GHOUL, template(GHOUL, 6));
    d
}

/// `g`'s blows of a hundred, both hands and the bow, every two seconds.
fn arm(m: &mut Map, g: Guid) {
    let f = m.unit_fields_mut(g).unwrap();
    for i in [
        UNIT_FIELD_MINDAMAGE,
        UNIT_FIELD_MAXDAMAGE,
        UNIT_FIELD_MINRANGEDDAMAGE,
        UNIT_FIELD_MAXRANGEDDAMAGE,
    ] {
        f.set_f32(i, 100.0);
    }
    f.set_u32(UNIT_FIELD_BASEATTACKTIME, 2_000);
    f.set_u32(UNIT_FIELD_RANGEDATTACKTIME, 2_000);
}

fn blow(m: &Map, g: Guid, victim: Guid, hand: Hand) -> u32 {
    combat::weapon_damage(m, g, victim, hand, &mut rand::rng()).0
}

fn stacked(m: &mut Map, g: Guid, spell: u32, n: u32) {
    for h in auras::auras_mut(m, g)
        .unwrap()
        .holders
        .iter_mut()
        .filter(|h| h.spell == spell)
    {
        h.stacks = n;
    }
}

#[tokio::test]
async fn attack_power_versus_a_type_counts_each_bit_and_stack_with_melee_and_ranged_apart() {
    let (_realm, _a, g, map) = setup_with(world(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, g, 60);
            let ghoul = spawn(m, g, GHOUL, 60, 2.0);
            let beast = spawn(m, g, BEAST, 60, 3.0);
            // 70 against demons and undead, two stacks; 42 ranged against
            // the undead.
            aura_of(
                m,
                g,
                98_201,
                auras::AURA_MOD_MELEE_ATTACK_POWER_VERSUS,
                36,
                70,
            );
            stacked(m, g, 98_201, 2);
            aura_of(
                m,
                g,
                98_202,
                auras::AURA_MOD_RANGED_ATTACK_POWER_VERSUS,
                32,
                42,
            );
            arm(m, g);
            let shown = m
                .unit_fields(g)
                .unwrap()
                .get_u32(UNIT_FIELD_ATTACK_POWER_MODS);
            let out = [
                blow(m, g, ghoul, Hand::Main),
                blow(m, g, beast, Hand::Main),
                blow(m, g, ghoul, Hand::Ranged),
                blow(m, g, beast, Hand::Ranged),
            ];
            // Gone, and nothing more.
            auras::remove_spell(m, g, 98_201, None);
            arm(m, g);
            (out, shown, blow(m, g, ghoul, Hand::Main))
        })
        .await
        .unwrap();
    let (out, shown, gone) = r;
    // 140 / 14 a second, two seconds: 20; 42: 6.
    assert_eq!(out, [120, 100, 106, 100]);
    assert_eq!(shown, 0, "not the attack power shown");
    assert_eq!(gone, 100);
}

#[tokio::test]
async fn a_player_is_a_humanoid_but_a_beast_in_cat_form_and_a_marks_attack_power_is_on_its_target()
{
    let (realm, _a, g, map) = setup_with(world(), Class::Warrior).await;
    let mut b = crate::game::harness::Client::connect(&realm, "B");
    // Bob, by the start with her; a cat by his form's byte alone.
    let bob = b.enter("Bob", Race::Human, Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, g, 60);
            aura_of(
                m,
                g,
                98_211,
                auras::AURA_MOD_MELEE_ATTACK_POWER_VERSUS,
                64,
                70,
            );
            aura_of(
                m,
                g,
                98_212,
                auras::AURA_MOD_MELEE_ATTACK_POWER_VERSUS,
                1,
                42,
            );
            arm(m, g);
            let humanoid = blow(m, g, bob, Hand::Main);
            let p = m.players.get_mut(&bob).unwrap();
            p.fields.set_byte(UNIT_FIELD_BYTES_1, 2, auras::FORM_CAT);
            let cat = blow(m, g, bob, Hand::Main);
            m.players
                .get_mut(&bob)
                .unwrap()
                .fields
                .set_byte(UNIT_FIELD_BYTES_1, 2, 0);
            auras::remove_spell(m, g, 98_211, None);
            auras::remove_spell(m, g, 98_212, None);
            arm(m, g);
            // Marked: 28 for blows, 42 for shots.
            let ghoul = spawn(m, g, GHOUL, 60, 2.0);
            aura_of(
                m,
                ghoul,
                98_213,
                auras::AURA_MELEE_ATTACK_POWER_ATTACKER_BONUS,
                0,
                28,
            );
            aura_of(
                m,
                ghoul,
                98_214,
                auras::AURA_RANGED_ATTACK_POWER_ATTACKER_BONUS,
                0,
                42,
            );
            let marked = (
                blow(m, g, ghoul, Hand::Main),
                blow(m, g, ghoul, Hand::Ranged),
            );
            (humanoid, cat, marked)
        })
        .await
        .unwrap();
    // 70: 10 more; in cat form the beasts' 42: 6.
    assert_eq!(r.0, 110);
    assert_eq!(r.1, 106);
    assert_eq!(r.2, (104, 106));
}

#[tokio::test]
async fn damage_and_crits_versus_a_type_count_against_that_type_alone() {
    let (_realm, _a, g, map) = setup_with(world(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, g, 60);
            let ghoul = spawn(m, g, GHOUL, 60, 2.0);
            let beast = spawn(m, g, BEAST, 60, 3.0);
            aura_of(m, g, 98_221, auras::AURA_MOD_DAMAGE_DONE_VERSUS, 32, 10);
            aura_of(m, g, 98_222, auras::AURA_MOD_CRIT_PERCENT_VERSUS, 32, 100);
            arm(m, g);
            [ghoul, beast].map(|v| (blow(m, g, v, Hand::Main), combat::crit_multiplier(m, g, v)))
        })
        .await
        .unwrap();
    // 110 against the undead, crit three times: 330; the beast's 100, 200.
    assert_eq!(r[0].0, 110);
    assert!((r[0].1 - 3.0).abs() < 0.001);
    assert_eq!(r[1].0, 100);
    assert!((r[1].1 - 2.0).abs() < 0.001);
}

const FLAT_STRIKE: u32 = 98_230;
const SURE: u32 = 98_231;

#[tokio::test]
async fn a_flat_ability_takes_no_weapon_speed_bonus_from_attack_power_versus_its_target() {
    let mut d = world();
    let mut strike = SpellTemplate {
        id: FLAT_STRIKE,
        name: "Flat Strike".into(),
        equipped_item_class: -1,
        range_index: 1,
        dmg_class: 2,
        ..Default::default()
    };
    strike.effects[0] = SpellEffect {
        effect: 2,
        base_points: 99,
        base_dice: 1,
        target_a: 6,
        ..Default::default()
    };
    d.spells.spells.insert(FLAT_STRIKE, strike);
    let mut sure = SpellTemplate {
        id: SURE,
        name: "Sure".into(),
        equipped_item_class: -1,
        ..Default::default()
    };
    for (i, (kind, amount)) in [
        (auras::AURA_MOD_HIT_CHANCE, 100),
        (auras::AURA_MOD_CRIT_PERCENT, -100),
    ]
    .into_iter()
    .enumerate()
    {
        sure.effects[i] = SpellEffect {
            effect: 6,
            aura: kind,
            base_points: amount - 1,
            base_dice: 1,
            target_a: 1,
            ..Default::default()
        };
    }
    d.spells.spells.insert(SURE, sure);
    let t = d.creature_templates.get_mut(&GHOUL).unwrap();
    t.flags_extra |= combat::FLAG_EXTRA_NO_BLOCK | combat::FLAG_EXTRA_NO_PARRY;
    let (_realm, _a, g, map) = setup_with(d, Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, g, 60);
            let ghoul = spawn(m, g, GHOUL, 60, 2.0);
            let f = m.unit_fields_mut(ghoul).unwrap();
            let flags = f.get_u32(UNIT_FIELD_FLAGS);
            f.set_u32(UNIT_FIELD_FLAGS, flags | 0x0004_0000);
            let s = m.realm.data.spells.get(SURE).unwrap().clone();
            auras::apply_spell(m, g, g, &s, &[(0, 100), (1, -100)], Default::default()).unwrap();
            aura_of(
                m,
                g,
                98_232,
                auras::AURA_MOD_MELEE_ATTACK_POWER_VERSUS,
                32,
                1_400,
            );
            let before = m.unit_fields(ghoul).unwrap().get_u32(UNIT_FIELD_HEALTH);
            spells::cast_triggered(m, g, FLAT_STRIKE, Some(ghoul));
            before - m.unit_fields(ghoul).unwrap().get_u32(UNIT_FIELD_HEALTH)
        })
        .await
        .unwrap();
    assert_eq!(r, 100);
}
