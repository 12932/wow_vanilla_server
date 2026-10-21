//! Round 10, against Thistle Tea's `power/power_leech_test.exs`: a direct
//! drain of power takes the caster's spell power and damage done and the
//! target's damage taken, as mangos's `EffectPowerDrain`
//! (`SpellDamageBonusDone`, `SpellDamageBonusTaken`), before it is taken
//! and given back by its multiplier.

use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::{auras, derive, spells};
use crate::game::tests_p3_combat::*;

const DRAIN: u32 = 98_301;
const SHADOW_POWER: u32 = 98_302;
const SHADOW_PCT: u32 = 98_303;
const SHADOW_WARD: u32 = 98_304;
const SHADOW: i32 = 1 << 5;

#[tokio::test]
async fn a_drain_takes_spell_power_damage_done_and_damage_taken() {
    let mut d = data();
    // A shadow drain of 40 mana, half its spell power's share, all back.
    let mut s = SpellTemplate {
        id: DRAIN,
        name: "Drain".into(),
        school: 5,
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: spells::EFFECT_POWER_DRAIN,
        base_points: 39,
        base_dice: 1,
        misc_value: 0,
        multiple_value: 1.0,
        bonus_coefficient: 0.5,
        target_a: 6,
        ..Default::default()
    };
    d.spells.spells.insert(DRAIN, s);
    let (_realm, _a, g, map) = setup_with(d, Class::Mage).await;
    let r = map
        .query(move |m| {
            let c = spawn(m, g, BRUTE, 20, 3.0);
            let mut drained = Vec::new();
            for buffed in [false, true] {
                let f = m.unit_fields_mut(c).unwrap();
                f.set_u32(UNIT_FIELD_MAXPOWER1, 150);
                f.set_u32(UNIT_FIELD_POWER1, 150);
                f.set_byte(UNIT_FIELD_BYTES_0, 3, 0);
                m.unit_fields_mut(g).unwrap().set_u32(UNIT_FIELD_POWER1, 0);
                if buffed {
                    aura_of(m, g, SHADOW_POWER, auras::AURA_MOD_DAMAGE_DONE, SHADOW, 80);
                    aura_of(
                        m,
                        g,
                        SHADOW_PCT,
                        auras::AURA_MOD_DAMAGE_PERCENT_DONE,
                        SHADOW,
                        50,
                    );
                    aura_of(
                        m,
                        c,
                        SHADOW_WARD,
                        auras::AURA_MOD_DAMAGE_PERCENT_TAKEN,
                        SHADOW,
                        -50,
                    );
                    derive::unit(m, g);
                    derive::unit(m, c);
                }
                spells::cast_triggered(m, g, DRAIN, Some(c));
                drained.push((
                    m.unit_fields(c).unwrap().get_u32(UNIT_FIELD_POWER1),
                    m.unit_fields(g).unwrap().get_u32(UNIT_FIELD_POWER1),
                ));
            }
            drained
        })
        .await
        .unwrap();
    // 40 alone; (40 + 80 x 0.5) x 1.5 x 0.5 = 60 with them.
    assert_eq!(r, vec![(110, 40), (90, 60)]);
}

const GEAR: u32 = 98_310;
const BUFF: u32 = 98_311;
const ARMED_BUFF: u32 = 98_312;
const FIRE_BOLT: u32 = 98_313;
const FIRE_HEAL: u32 = 98_314;
const FROST_HEAL: u32 = 98_315;
const FIRE: i32 = 1 << 2;

/// Against Thistle Tea's `stats/temporary_spell_power_test.exs`: a buff's
/// spell power stacks on gear's, by its stacks, in its own school; one
/// that asks for a weapon gives none to spells; a healing bonus of one
/// school isn't another's heal's (mangos's `SpellBaseHealingBonusDone` by
/// the heal's school mask).
#[tokio::test]
async fn a_buffs_spell_power_stacks_on_gears_in_its_school_and_not_asking_a_weapon() {
    let mut d = data();
    let aura = |aura: u32, misc: i32, points: i32| SpellEffect {
        effect: 6,
        aura,
        misc_value: misc,
        base_points: points - 1,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    for (id, stacks, item_class, effects) in [
        (
            GEAR,
            0,
            -1,
            [
                aura(auras::AURA_MOD_DAMAGE_DONE, FIRE, 10),
                aura(auras::AURA_MOD_HEALING_DONE, 127, 20),
            ],
        ),
        (
            BUFF,
            5,
            -1,
            [
                aura(auras::AURA_MOD_DAMAGE_DONE, FIRE, 50),
                aura(auras::AURA_MOD_HEALING_DONE, FIRE, 100),
            ],
        ),
        (
            ARMED_BUFF,
            5,
            2,
            [
                aura(auras::AURA_MOD_DAMAGE_DONE, FIRE, 50),
                aura(auras::AURA_MOD_HEALING_DONE, FIRE, 100),
            ],
        ),
    ] {
        let mut s = SpellTemplate {
            id,
            name: format!("Buff {id}"),
            equipped_item_class: item_class,
            stack_amount: stacks,
            ..Default::default()
        };
        s.effects[0] = effects[0];
        s.effects[1] = effects[1];
        d.spells.spells.insert(id, s);
    }
    for (id, school, effect) in [(FIRE_BOLT, 2, 2), (FIRE_HEAL, 2, 10), (FROST_HEAL, 4, 10)] {
        let mut s = SpellTemplate {
            id,
            name: format!("Spell {id}"),
            school,
            equipped_item_class: -1,
            dmg_class: 1,
            ..Default::default()
        };
        s.effects[0] = SpellEffect {
            effect,
            base_points: 99,
            base_dice: 1,
            target_a: 6,
            ..Default::default()
        };
        d.spells.spells.insert(id, s);
    }
    let (_realm, _a, g, map) = setup_with(d, Class::Mage).await;
    let r = map
        .query(move |m| {
            let put = |m: &mut crate::game::map::Map, id: u32| {
                let s = m.realm.data.spells.get(id).unwrap().clone();
                let effects: Vec<(usize, i32)> =
                    (0..2).map(|i| (i, s.effects[i].base_points + 1)).collect();
                auras::apply_spell(m, g, g, &s, &effects, Default::default()).unwrap();
                derive::unit(m, g);
            };
            let read = |m: &crate::game::map::Map| {
                let s = |id: u32| m.realm.data.spells.get(id).unwrap().clone();
                let power = |id: u32, heal: bool| {
                    crate::game::map::spells::outcome::power(m, g, g, &s(id), heal) as i32
                };
                let f = &m.players[&g].fields;
                (
                    power(FIRE_BOLT, false),
                    f.get_u32(PLAYER_FIELD_MOD_DAMAGE_DONE_POS + 2),
                    f.get_u32(PLAYER_FIELD_MOD_DAMAGE_DONE_POS + 4),
                    power(FIRE_HEAL, true),
                    power(FROST_HEAL, true),
                )
            };
            put(m, GEAR);
            put(m, BUFF);
            put(m, BUFF);
            let stacks = auras::auras(m, g)
                .unwrap()
                .holders
                .iter()
                .find(|h| h.spell == BUFF)
                .map(|h| h.stacks);
            let buffed = read(m);
            auras::remove_spell(m, g, BUFF, None);
            put(m, ARMED_BUFF);
            let armed = read(m);
            (stacks, buffed, armed)
        })
        .await
        .unwrap();
    let (stacks, buffed, armed) = r;
    assert_eq!(stacks, Some(2));
    // Fire: 10 and two of 50; frost none; a fire heal 20 and two of 100, a
    // frost heal the gear's 20 alone.
    assert_eq!(buffed, (110, 110, 0, 220, 20));
    // Asking for a weapon she hasn't: the gear's alone to her fire spells.
    assert_eq!((armed.0, armed.1), (10, 10));
}
