//! Round 8, against Thistle Tea's `spell_amount_modifiers_test.exs`: the
//! order a spell's amount takes its caster's and its target's modifiers in,
//! direct and over time (mangos-classic's `SpellDamageBonusDone`,
//! `SpellDamageBonusTaken`, their healing kin and `SPELLMOD_DOT`).
//!
//! The caster has 40 fire damage and 40 healing, 50% more fire damage and
//! healing done, and modifiers to its family's spell: 20 and 50% to all
//! effects, 30 and 50% to damage, 10 and 20% to damage over time. The
//! target takes 20 and 50% more fire damage and healing. A spell of 100 at
//! a coefficient of 1 comes to 180 of base, (180 + 40) × 1.5 = 330 done,
//! (330 + 30) × 1.5 = 540 with its damage modifiers, (540 + 20) × 1.5 = 840
//! taken.

use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh, pass};
use crate::game::data::GameData;
use crate::game::data::spells::{CUSTOM_FIXED_DAMAGE, SpellDuration, SpellEffect, SpellTemplate};
use crate::game::map::Map;
use crate::game::map::auras::*;
use crate::game::map::spells::{self, mods};

const BOLT: u32 = 998_001;
const HEAL: u32 = 998_002;
const GAIN: u32 = 998_003;
const DOT: u32 = 998_004;
const HOT: u32 = 998_005;
const FIXED_BOLT: u32 = 998_006;
const FIXED_HEAL: u32 = 998_007;
const FIXED_DOT: u32 = 998_008;
const FIXED_HOT: u32 = 998_009;
/// Ignite's burn, cast for itself.
const IGNITE: u32 = 12_654;
/// The caster's power, its percentages and its modifiers; a lone flat 50
/// to damage; what the target takes.
const POWER: u32 = 998_010;
const DONE_PCT: u32 = 998_011;
const MODS_A: u32 = 998_012;
const MODS_B: u32 = 998_013;
const DAMAGE_50: u32 = 998_014;
const TAKEN: u32 = 998_015;
const LASTS_3S: u32 = 990;

const FIRE: u32 = 2;
const FIRE_MASK: i32 = 1 << FIRE;

fn mage_spell(id: u32, e: SpellEffect) -> SpellTemplate {
    let mut s = bare(id, FIRE);
    s.family_name = 3;
    s.family_flags = 1;
    s.duration_index = LASTS_3S;
    effect(
        &mut s,
        0,
        SpellEffect {
            bonus_coefficient: 1.0,
            ..e
        },
    );
    s
}

fn holder(id: u32, family: u32, effects: &[SpellEffect]) -> SpellTemplate {
    let mut s = bare(id, 0);
    s.family_name = family;
    s.duration_index = LASTS_LONG;
    for (i, e) in effects.iter().enumerate() {
        effect(&mut s, i, *e);
    }
    s
}

fn modifier(kind: u32, op: usize, amount: i32) -> SpellEffect {
    SpellEffect {
        item_type: 1,
        ..aura(kind, op as i32, amount, 1)
    }
}

fn data(d: &mut GameData) {
    d.spells.durations.insert(
        LASTS_3S,
        SpellDuration {
            base: 3_000,
            per_level: 0,
            max: 3_000,
        },
    );
    let direct = |effect: u32| SpellEffect {
        effect,
        base_points: 99,
        base_dice: 1,
        target_a: 6,
        ..Default::default()
    };
    let over_time = |kind: u32| SpellEffect {
        amplitude: 1_000,
        ..aura(kind, 0, 100, 6)
    };
    let bolt = mage_spell(BOLT, direct(spells::EFFECT_SCHOOL_DAMAGE));
    let heal = mage_spell(HEAL, direct(spells::EFFECT_HEAL));
    let gain = mage_spell(
        GAIN,
        SpellEffect {
            misc_value: 0,
            target_a: 1,
            ..direct(spells::EFFECT_ENERGIZE)
        },
    );
    let dot = mage_spell(DOT, over_time(AURA_PERIODIC_DAMAGE));
    let hot = mage_spell(HOT, over_time(AURA_PERIODIC_HEAL));
    let fixed = |s: &SpellTemplate, id: u32| {
        let mut f = s.clone();
        f.id = id;
        f.custom_flags |= CUSTOM_FIXED_DAMAGE;
        f
    };
    let (fixed_bolt, fixed_heal) = (fixed(&bolt, FIXED_BOLT), fixed(&heal, FIXED_HEAL));
    let (mut fixed_dot, mut fixed_hot) = (fixed(&dot, FIXED_DOT), fixed(&hot, FIXED_HOT));
    let mut ignite = dot.clone();
    ignite.id = IGNITE;
    for s in [&mut fixed_dot, &mut fixed_hot, &mut ignite] {
        s.effects[0].bonus_coefficient = 0.0;
    }
    let power = holder(
        POWER,
        0,
        &[
            aura(AURA_MOD_DAMAGE_DONE, FIRE_MASK, 40, 1),
            aura(AURA_MOD_HEALING_DONE, 127, 40, 1),
        ],
    );
    let done_pct = holder(
        DONE_PCT,
        0,
        &[
            aura(AURA_MOD_DAMAGE_PERCENT_DONE, FIRE_MASK, 50, 1),
            aura(AURA_MOD_HEALING_DONE_PERCENT, 0, 50, 1),
        ],
    );
    let mods_a = holder(
        MODS_A,
        3,
        &[
            modifier(AURA_ADD_FLAT_MODIFIER, mods::OP_ALL_EFFECTS, 20),
            modifier(AURA_ADD_PCT_MODIFIER, mods::OP_ALL_EFFECTS, 50),
            modifier(AURA_ADD_FLAT_MODIFIER, mods::OP_DAMAGE, 30),
        ],
    );
    let mods_b = holder(
        MODS_B,
        3,
        &[
            modifier(AURA_ADD_PCT_MODIFIER, mods::OP_DAMAGE, 50),
            modifier(AURA_ADD_FLAT_MODIFIER, mods::OP_DOT, 10),
            modifier(AURA_ADD_PCT_MODIFIER, mods::OP_DOT, 20),
        ],
    );
    let damage_50 = holder(
        DAMAGE_50,
        3,
        &[modifier(AURA_ADD_FLAT_MODIFIER, mods::OP_DAMAGE, 50)],
    );
    let taken = holder(
        TAKEN,
        0,
        &[
            aura(AURA_MOD_DAMAGE_TAKEN, FIRE_MASK, 20, 1),
            aura(AURA_MOD_DAMAGE_PERCENT_TAKEN, FIRE_MASK, 50, 1),
            aura(AURA_MOD_HEALING, FIRE_MASK, 20, 1),
        ],
    );
    let mut heal_pct = holder(TAKEN + 100, 0, &[aura(AURA_MOD_HEALING_PCT, 0, 50, 1)]);
    heal_pct.id = TAKEN + 100;
    insert(
        d,
        [
            bolt, heal, gain, dot, hot, fixed_bolt, fixed_heal, fixed_dot, fixed_hot, ignite,
            power, done_pct, mods_a, mods_b, damage_50, taken, heal_pct,
        ],
    );
}

const ALL: &[u32] = &[
    BOLT, HEAL, GAIN, DOT, HOT, FIXED_BOLT, FIXED_HEAL, FIXED_DOT, FIXED_HOT, IGNITE,
];

/// The caster with every buff, a target half hurt with what it takes.
fn set_up(m: &mut Map, a: wow_world_messages::Guid) -> wow_world_messages::Guid {
    m.spells.rolls = Some(false);
    kit::ready(m, a, 60, ALL);
    for s in [POWER, DONE_PCT, MODS_A, MODS_B] {
        kit::apply(m, a, a, s);
    }
    let kobold = kit::dummy(m, a, 60, 5.0);
    kit::apply(m, kobold, kobold, TAKEN);
    kit::apply(m, kobold, kobold, TAKEN + 100);
    kit::set_health(m, kobold, 500_000);
    kobold
}

/// What a cast of `spell` at `kobold` changes its health by.
fn change(
    m: &mut Map,
    a: wow_world_messages::Guid,
    kobold: wow_world_messages::Guid,
    spell: u32,
) -> i64 {
    kit::set_health(m, kobold, 500_000);
    cast_fresh(m, a, spell, at(kobold));
    kit::health(m, kobold) as i64 - 500_000
}

/// The amount `spell`'s aura on `g` holds.
fn held(m: &Map, g: wow_world_messages::Guid, spell: u32) -> Option<i32> {
    kit::holder(m, g, spell).and_then(|h| h.effects.first().map(|x| x.amount))
}

/// A direct spell's damage and healing take each modifier in turn, 840 of
/// 100; a fixed one only its base modifiers, 180; a gain of power its base
/// modifiers alone, 180; a chain's later share comes off what the caster
/// does before damage's modifiers (Thistle Tea's "direct damage and healing
/// apply each modifier to its actual amount", "fixed damage and healing
/// keep base modifiers", "resource gains use all-effects bonuses" and
/// "flat damage modifiers apply after chain attenuation").
#[tokio::test]
async fn a_direct_spell_takes_each_modifier_in_its_turn() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P8AM1", "Modded", Class::Mage).await;
    let out = map
        .query(move |m| {
            let kobold = set_up(m, a);
            let direct = (change(m, a, kobold, BOLT), change(m, a, kobold, HEAL));
            let fixed = (
                change(m, a, kobold, FIXED_BOLT),
                change(m, a, kobold, FIXED_HEAL),
            );
            kit::set_power(m, a, 0, 100);
            cast_fresh(m, a, GAIN, Default::default());
            let gained = kit::power(m, a, 0);
            // A chain's second target, at half.
            let s = m.realm.data.spells.get(BOLT).unwrap().clone();
            let mods = mods::for_spell(m, a, &s);
            let l = spells::outcome::Landing {
                chain: 0.5,
                mods,
                ..Default::default()
            };
            let chained = spells::outcome::damage(m, a, kobold, &s, 0, 180.0, &l).amount;
            (direct, fixed, gained, chained)
        })
        .await
        .unwrap();
    let (direct, fixed, gained, chained) = out;
    assert_eq!(direct, (-840, 840), "direct");
    assert_eq!(fixed, (-180, 180), "fixed");
    assert_eq!(gained, 280, "a gain of power");
    assert!(matches!(chained, 468 | 469), "a chain's half: {chained}");
}

/// A damage or heal over time holds 408 a tick of 100: its base
/// modifiers, the caster's power and percentages, then damage over time's
/// modifiers (not damage's); each tick takes the target's, 642; a cast
/// again by the caster unbuffed holds what it does now, 140; a fixed one
/// and Ignite's burn hold their base, 180; damage's modifiers alone leave
/// a tick at 140 (Thistle Tea's "periodic amounts use dot modifiers after
/// caster bonuses and keep their snapshot", "fixed periodic effects and
/// Ignite bypass outgoing amount modifiers" and "direct damage modifiers
/// do not amplify a periodic component").
#[tokio::test]
async fn an_effect_over_time_holds_its_casters_amount_and_ticks_the_targets() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P8AM2", "Ticked", Class::Mage).await;
    let out = map
        .query(move |m| {
            let kobold = set_up(m, a);
            let mut ticks = Vec::new();
            for spell in [DOT, HOT] {
                kit::set_health(m, kobold, 500_000);
                cast_fresh(m, a, spell, at(kobold));
                let snapshot = held(m, kobold, spell);
                pass(m, 1_000);
                let ticked = kit::health(m, kobold) as i64 - 500_000;
                ticks.push((snapshot, ticked));
                crate::game::map::auras::remove_where(m, kobold, |h| h.spell == spell);
            }
            let fixed: Vec<Option<i32>> = [FIXED_DOT, FIXED_HOT, IGNITE]
                .into_iter()
                .map(|spell| {
                    cast_fresh(m, a, spell, at(kobold));
                    held(m, kobold, spell)
                })
                .collect();
            // Unbuffed, but for the power its gear would give.
            crate::game::map::auras::remove_where(m, a, |h| {
                matches!(h.spell, DONE_PCT | MODS_A | MODS_B)
            });
            cast_fresh(m, a, DOT, at(kobold));
            let refreshed = held(m, kobold, DOT);
            kit::apply(m, a, a, DAMAGE_50);
            cast_fresh(m, a, DOT, at(kobold));
            let damage_only = held(m, kobold, DOT);
            (ticks, fixed, refreshed, damage_only)
        })
        .await
        .unwrap();
    let (ticks, fixed, refreshed, damage_only) = out;
    assert_eq!(ticks, vec![(Some(408), -642), (Some(408), 642)], "ticks");
    assert_eq!(fixed, vec![Some(180); 3], "fixed and Ignite");
    assert_eq!(refreshed, Some(140), "a cast again, unbuffed");
    assert_eq!(damage_only, Some(140), "damage's modifiers alone");
}
