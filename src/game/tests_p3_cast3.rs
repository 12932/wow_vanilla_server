//! Phase 3's spell modifiers, as Thistle Tea's `modifiers_test` has them:
//! which spells a modifier touches, by family and all 64 bits of its mask;
//! its stacks; the spells that ignore their caster's modifiers; the
//! operations an aura's amount and interval take; and which casts spend a
//! modifier's charge. On made-up spells and the fast suite.

use wow_world_messages::Guid;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{apply_template, holder};
use crate::game::data::GameData;
use crate::game::data::spells::{ATTR_EX3_IGNORE_CASTER_MODIFIERS, SpellEffect, SpellTemplate};
use crate::game::map::Map;
use crate::game::map::auras::*;
use crate::game::map::spells::{self, mods};

/// A modifier talent of the rogue's family (8): `kind` of operation `op`
/// by `amount`, on the spells of `mask`, with `charges`.
fn modifier(id: u32, kind: u32, op: usize, amount: i32, mask: u64, charges: u32) -> SpellTemplate {
    let mut s = bare(id, 0);
    s.family_name = 8;
    s.duration_index = LASTS_LONG;
    s.proc_charges = charges;
    effect(
        &mut s,
        0,
        SpellEffect {
            item_type: mask,
            ..aura(kind, op as i32, amount, 1)
        },
    );
    s
}

/// A rogue spell of `flags` with effects `e`.
fn rogue(id: u32, flags: u64, e: &[SpellEffect]) -> SpellTemplate {
    let mut s = bare(id, 0);
    s.family_name = 8;
    s.family_flags = flags;
    for (i, x) in e.iter().enumerate() {
        effect(&mut s, i, *x);
    }
    s
}

fn spell(m: &Map, id: u32) -> SpellTemplate {
    m.realm.data.spells.get(id).cloned().unwrap()
}

/// A made-up kobold to hold the modifiers.
fn holder_unit(m: &mut Map) -> Guid {
    let t = m.realm.data.creature_template(KOBOLD).cloned().unwrap();
    let counter = m.next_creature_counter;
    m.next_creature_counter += 1;
    let c = crate::game::map::creature::Creature::new(
        crate::game::guid::unit(KOBOLD, counter | 0x80_0000),
        &t,
        60,
        wow_world_messages::vanilla::Vector3d {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        0.0,
    )
    .with_base();
    m.add_creature(c)
}

fn put(m: &mut Map, g: Guid, id: u32) {
    let s = spell(m, id);
    apply_template(m, g, g, &s);
}

const STACKED: u32 = 990_300;
const NO_MASK: u32 = 990_301;
const HIGH: u32 = 990_302;
const CHEAPER: u32 = 990_303;
const SLOWER: u32 = 990_304;

fn modifiers(d: &mut GameData) {
    insert(
        d,
        [
            modifier(
                STACKED,
                AURA_ADD_FLAT_MODIFIER,
                mods::OP_CRITICAL_CHANCE,
                10,
                1,
                0,
            ),
            modifier(NO_MASK, AURA_ADD_PCT_MODIFIER, mods::OP_COST, -100, 0, 0),
            modifier(HIGH, AURA_ADD_PCT_MODIFIER, mods::OP_COST, -100, 1 << 32, 0),
            modifier(CHEAPER, AURA_ADD_PCT_MODIFIER, mods::OP_COST, -50, 1, 0),
            modifier(
                SLOWER,
                AURA_ADD_FLAT_MODIFIER,
                mods::OP_SPEED,
                -20,
                0x40_0000,
                0,
            ),
        ],
    );
}

/// A modifier touches its family's spells whose flags meet its mask, the
/// upper 32 bits too, and none with an empty mask; its stacks multiply it;
/// a spell that ignores its caster's modifiers takes none, and costs what
/// it costs; a cost modifier takes from the cost; a speed modifier from a
/// slow's amount (Thistle Tea's `Modifiers.value` and `power_cost`).
#[tokio::test]
async fn a_modifier_touches_its_familys_spells_by_all_of_its_mask() {
    let out = on_map(modifiers, |m| {
        let g = holder_unit(m);
        let crit = rogue(990_310, 1, &[damage(10)]);
        let mut costly = rogue(990_311, 0x8000_0000, &[damage(10)]);
        costly.mana_cost = 450;
        let mut high = costly.clone();
        high.family_flags = 1 << 32;
        let mut other_high = costly.clone();
        other_high.family_flags = 2 << 32;
        let mut low = costly.clone();
        low.family_flags = 1;
        low.mana_cost = 200;
        let mut ignoring = low.clone();
        ignoring.attributes[3] |= ATTR_EX3_IGNORE_CASTER_MODIFIERS;
        let cost = |m: &Map, s: &SpellTemplate| {
            mods::for_spell(m, g, s).apply_ms(mods::OP_COST, spells::power_cost(m, g, s))
        };
        put(m, g, STACKED);
        if let Some(a) = auras_mut(m, g) {
            for h in a.holders.iter_mut().filter(|h| h.spell == STACKED) {
                h.stacks = 10;
            }
        }
        let stacked = mods::for_spell(m, g, &crit).flat[mods::OP_CRITICAL_CHANCE];
        put(m, g, NO_MASK);
        let no_mask = cost(m, &costly);
        remove_spell(m, g, NO_MASK, None);
        put(m, g, HIGH);
        let high_bits = (cost(m, &high), cost(m, &other_high));
        remove_spell(m, g, HIGH, None);
        put(m, g, CHEAPER);
        let cheaper = (cost(m, &low), cost(m, &ignoring));
        let ignored = mods::for_spell(m, g, &ignoring);
        put(m, g, SLOWER);
        let mut slow = rogue(
            990_312,
            0x40_0000,
            &[aura(AURA_MOD_DECREASE_SPEED, 0, -10, 6)],
        );
        slow.duration_index = LASTS_15S;
        let slowed = mods::for_spell(m, g, &slow).points(&slow, 0, -10);
        (
            stacked,
            no_mask,
            high_bits,
            cheaper,
            ignored
                .flat
                .iter()
                .chain(ignored.pct.iter())
                .all(|v| *v == 0),
            slowed,
        )
    })
    .await;
    let (stacked, no_mask, high_bits, cheaper, ignored, slowed) = out;
    assert_eq!(stacked, 100, "ten stacks of ten");
    assert_eq!(no_mask, 450, "an empty mask touches nothing");
    assert_eq!(high_bits, (0, 450), "the mask's upper half");
    assert_eq!(
        cheaper,
        (100, 200),
        "half the cost, and none off an ignoring spell"
    );
    assert!(ignored, "an ignoring spell takes no modifier");
    assert_eq!(slowed, -30, "a speed modifier on a slow's amount");
}

/// A periodic aura's interval takes its activation time modifiers, flat
/// then percent, and keeps at least a millisecond; what doesn't tick
/// keeps its own, and an aura with no interval has none (Thistle Tea's
/// `Modifiers.periodic_interval`).
#[test]
fn a_ticks_interval_takes_its_modifiers_and_keeps_a_millisecond() {
    use std::time::Duration;
    let trigger = SpellEffect {
        amplitude: 4_000,
        ..aura(AURA_PERIODIC_TRIGGER_SPELL, 0, 0, 1)
    };
    let ms = |e: &SpellEffect, a: (i32, i32)| interval(e, a).map(|d| d.as_millis() as u64);
    assert_eq!(ms(&trigger, (-1_001, -50)), Some(1_499));
    assert_eq!(ms(&trigger, (-4_001, 0)), Some(1));
    assert_eq!(ms(&trigger, (0, -100)), Some(1));
    let none = aura(AURA_PERIODIC_DAMAGE, 0, 0, 1);
    assert_eq!(interval(&none, (-500, 0)), None, "no interval, none made");
    let mana = aura(AURA_OBS_MOD_MANA, 0, 0, 1);
    assert_eq!(
        ms(&mana, (-500, 0)),
        Some(500),
        "a mana tick's second, less"
    );
    let stat = SpellEffect {
        amplitude: 4_000,
        ..aura(AURA_MOD_STAT, 0, 0, 1)
    };
    assert_eq!(
        interval(&stat, (-500, 0)),
        Some(Duration::from_millis(4_000)),
        "what doesn't tick keeps its own"
    );
}

/// Which of a caster's charged modifiers a cast spends: all effects' by
/// any, damage's by a direct amount (a heal, a drain or a burn, not a gain
/// of power or a tick), a tick's by a tick; attack power's and haste's by
/// an aura of theirs; crit by a strike; none by a spell that ignores its
/// caster's modifiers or another family's (Thistle Tea's
/// `consumable_holder_ids`); and a charge spent is the holder gone.
#[tokio::test]
async fn a_charged_modifier_is_spent_by_a_cast_that_uses_it() {
    use mods::*;
    let ops = [
        (990_320, OP_ALL_EFFECTS),
        (990_321, OP_DAMAGE),
        (990_322, OP_DOT),
        (990_323, OP_ATTACK_POWER),
        (990_324, OP_HASTE),
        (990_325, OP_CRITICAL_CHANCE),
        (990_326, OP_CASTING_TIME),
    ];
    let out = on_map(
        move |d| {
            insert(
                d,
                ops.map(|(id, op)| modifier(id, AURA_ADD_FLAT_MODIFIER, op, 10, 1, 1)),
            );
        },
        move |m| {
            let g = holder_unit(m);
            for (id, _) in ops {
                put(m, g, id);
            }
            let spends = |m: &Map, s: &SpellTemplate| {
                let mut v: Vec<usize> = mods::for_spell(m, g, s)
                    .charged
                    .iter()
                    .map(|c| c.2)
                    .collect();
                v.sort();
                v
            };
            let periodic = |a: u32| SpellEffect {
                amplitude: 3_000,
                ..aura(a, 0, 10, 6)
            };
            let kinds = [
                SpellEffect {
                    effect: 10,
                    ..damage(10)
                },
                SpellEffect {
                    effect: 8,
                    ..damage(10)
                },
                SpellEffect {
                    effect: 30,
                    ..damage(10)
                },
                aura(AURA_MOD_STAT, 0, 10, 6),
                periodic(AURA_PERIODIC_HEAL),
                periodic(AURA_PERIODIC_DAMAGE),
                periodic(AURA_PERIODIC_MANA_LEECH),
                aura(AURA_MOD_ATTACK_POWER, 0, 10, 6),
                aura(AURA_MOD_RANGED_ATTACK_POWER_PCT, 0, 10, 6),
                aura(AURA_MOD_ATTACKSPEED, 0, 10, 6),
                aura(AURA_MOD_CASTING_SPEED, 0, 10, 6),
            ];
            let mut seen: Vec<Vec<usize>> = kinds
                .iter()
                .map(|e| spends(m, &rogue(990_330, 1, &[*e])))
                .collect();
            // A strike crits, a spell with nothing to crit doesn't.
            let mut strike = rogue(
                990_331,
                1,
                &[SpellEffect {
                    effect: 121,
                    ..damage(10)
                }],
            );
            strike.dmg_class = 2;
            seen.push(spends(m, &strike));
            seen.push(spends(m, &rogue(990_332, 1, &[])));
            // Another mask, another family, or ignoring the caster's.
            seen.push(spends(m, &rogue(990_333, 2, &[damage(10)])));
            let mut other = rogue(990_334, 1, &[damage(10)]);
            other.family_name = 3;
            seen.push(spends(m, &other));
            let mut ignoring = rogue(990_335, 1, &[damage(10)]);
            ignoring.attributes[3] |= ATTR_EX3_IGNORE_CASTER_MODIFIERS;
            seen.push(spends(m, &ignoring));
            // Spent by a strike: crit's and all effects' go.
            let used = mods::for_spell(m, g, &strike);
            mods::spend(m, g, &used);
            let gone = (
                holder(m, g, 990_325).is_none(),
                holder(m, g, 990_320).is_none(),
                holder(m, g, 990_322).is_some(),
            );
            (seen, gone)
        },
    )
    .await;
    let (seen, gone) = out;
    let (all, dmg, dot, ap, haste, crit) = (
        mods::OP_ALL_EFFECTS,
        mods::OP_DAMAGE,
        mods::OP_DOT,
        mods::OP_ATTACK_POWER,
        mods::OP_HASTE,
        mods::OP_CRITICAL_CHANCE,
    );
    let sorted = |mut v: Vec<usize>| {
        v.sort();
        v
    };
    assert_eq!(seen[0], sorted(vec![all, dmg, crit]), "a heal");
    assert_eq!(seen[1], sorted(vec![all, dmg]), "a drain");
    assert_eq!(seen[2], vec![all], "a gain of power");
    assert_eq!(seen[3], vec![all], "a stat");
    assert_eq!(seen[4], sorted(vec![all, dot]), "a heal over time");
    assert_eq!(seen[5], sorted(vec![all, dot]), "a damage over time");
    assert_eq!(seen[6], vec![all], "a mana leech's tick");
    assert_eq!(seen[7], sorted(vec![all, ap]), "attack power");
    assert_eq!(
        seen[8],
        sorted(vec![all, ap]),
        "ranged attack power's percent"
    );
    assert_eq!(seen[9], sorted(vec![all, haste]), "attack speed");
    assert_eq!(seen[10], sorted(vec![all, haste]), "casting speed");
    assert_eq!(seen[11], sorted(vec![all, dmg, crit]), "a strike");
    assert_eq!(seen[12], vec![all], "nothing to crit");
    for (i, s) in seen[13..].iter().enumerate() {
        assert!(s.is_empty(), "case {i}: another mask, family, or ignoring");
    }
    assert_eq!(gone, (true, true, true), "the strike spent its own");
}
