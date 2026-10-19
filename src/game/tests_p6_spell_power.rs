//! Phase 3, round 6, against Thistle Tea's `spell_refunds_test.exs`: what
//! an avoided ability gives back of what it cost.

use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh, has, power, set_power};
use crate::game::data::GameData;
use crate::game::data::spells::{
    ATTR_EX_DISCOUNT_POWER_ON_MISS, ATTR_EX_REQ_COMBO_POINTS, SpellEffect, SpellTemplate,
};
use crate::game::fields::*;
use crate::game::map::auras::{
    AURA_ADD_FLAT_MODIFIER, AURA_ADD_PCT_MODIFIER, AURA_SCHOOL_IMMUNITY,
};
use crate::game::map::spells::{self, mods};

/// A rogue's strike for 45 energy given back on a miss; one that adds a
/// combo point beside; a finisher for 35 that gives nothing back.
const STRIKE: u32 = 990_940;
const POINTED: u32 = 990_941;
const FINISHER: u32 = 990_942;
/// Five less energy for the next strike, then none, each one charge; a
/// shield of the physical school.
const THRIFT: u32 = 990_943;
const FREE: u32 = 990_944;
const SHIELD: u32 = 990_945;

fn rogue(d: &mut GameData) {
    let ability = |id: u32, cost: u32| {
        let mut s = bare(id, 0);
        s.dmg_class = 2;
        s.power_type = 3;
        s.mana_cost = cost;
        s.family_name = 8;
        s.family_flags = 0x1;
        s.attributes[1] |= ATTR_EX_DISCOUNT_POWER_ON_MISS;
        effect(&mut s, 0, damage(10));
        s
    };
    let mut pointed = ability(POINTED, 45);
    effect(
        &mut pointed,
        1,
        SpellEffect {
            effect: spells::EFFECT_ADD_COMBO_POINTS,
            base_points: 0,
            base_dice: 1,
            target_a: 6,
            ..Default::default()
        },
    );
    let mut finisher = ability(FINISHER, 35);
    finisher.attributes[1] = ATTR_EX_REQ_COMBO_POINTS;
    let modifier = |id: u32, kind: u32, amount: i32| {
        let mut s = bare(id, 0);
        s.family_name = 8;
        s.duration_index = LASTS_LONG;
        s.proc_charges = 1;
        effect(
            &mut s,
            0,
            SpellEffect {
                item_type: 0x1,
                ..aura(kind, mods::OP_COST as i32, amount, 1)
            },
        );
        s
    };
    let mut shield = bare(SHIELD, 1);
    shield.duration_index = LASTS_LONG;
    effect(&mut shield, 0, aura(AURA_SCHOOL_IMMUNITY, 1, 0, 1));
    let spells: Vec<SpellTemplate> = vec![
        ability(STRIKE, 45),
        pointed,
        finisher,
        modifier(THRIFT, AURA_ADD_FLAT_MODIFIER, -5),
        modifier(FREE, AURA_ADD_PCT_MODIFIER, -100),
        shield,
    ];
    insert(d, spells);
}

/// An ability kept off by an immunity gives back 82% of what was paid for
/// it: of the cost a charged modifier cut, which is spent; nothing when a
/// modifier made it free, nor for a triggered one, which paid nothing. One
/// with two effects gives back once and adds no combo point. A finisher
/// without the attribute gives nothing back, and keeps its combo points
/// (Thistle Tea's paid cast feedback and `AttackFeedback`; mangos's
/// `SPELL_ATTR_EX_DISCOUNT_POWER_ON_MISS` and its combo points kept on a
/// miss).
#[tokio::test]
async fn an_avoided_ability_gives_back_what_was_paid() {
    let realm = fast_realm(rogue);
    let (_c, a, map) = human(&realm, "P6PA", "Refunded", Class::Rogue).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[STRIKE, POINTED, FINISHER]);
            if let Some(f) = m.unit_fields_mut(a) {
                f.set_u32(UNIT_FIELD_MAXPOWER1 + 3, 100);
            }
            let kobold = kit::dummy(m, a, 60, 2.0);
            kit::apply(m, kobold, kobold, SHIELD);
            let mut energy = Vec::new();
            let mut strike =
                |m: &mut crate::game::map::Map, spell: u32, from: u32, aura: Option<u32>| {
                    set_power(m, a, 3, from);
                    if let Some(x) = aura {
                        kit::apply(m, a, a, x);
                    }
                    cast_fresh(m, a, spell, at(kobold));
                    energy.push(power(m, a, 3));
                    aura.is_some_and(|x| has(m, a, x))
                };
            let thrift_left = strike(m, STRIKE, 100, Some(THRIFT));
            let free_left = strike(m, STRIKE, 50, Some(FREE));
            set_power(m, a, 3, 50);
            spells::cast_triggered(m, a, STRIKE, Some(kobold));
            let triggered = power(m, a, 3);
            strike(m, POINTED, 100, None);
            let points = m.caster(a).map(|c| c.combo_points);
            if let Some(c) = m.caster_mut(a) {
                c.combo_points = 5;
                c.combo_target = kobold;
            }
            strike(m, FINISHER, 100, None);
            let kept = m
                .caster(a)
                .map(|c| (c.combo_points, c.combo_target == kobold));
            (energy, thrift_left, free_left, triggered, points, kept)
        })
        .await
        .unwrap();
    let (energy, thrift_left, free_left, triggered, points, kept) = out;
    assert_eq!(
        energy,
        [93, 50, 92, 65],
        "40 paid, 33 back; free; 45 paid, 37 back once; a finisher's 35 kept"
    );
    assert!(!thrift_left && !free_left, "the charges spent");
    assert_eq!(triggered, 50, "a triggered strike paid nothing");
    assert_eq!(points, Some(0), "no combo point on an immune target");
    assert_eq!(kept, Some((5, true)), "the finisher keeps its points");
}
