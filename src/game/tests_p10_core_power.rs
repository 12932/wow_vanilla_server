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
