//! Round 12e: class spells as mangos-classic runs them: Blind's
//! diminishing returns, Seal of Righteousness's proc, Frenzied
//! Regeneration, Ruthlessness's combo point after its finisher.

use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh};
use wow_world_messages::Guid;

use super::tests_p3_combat::equip;
use crate::game::data::GameData;
use crate::game::data::items::ItemTemplate;
use crate::game::data::spells::{ATTR_EX2_CANT_CRIT, ATTR_EX3_ALWAYS_HIT};
use crate::game::map::Map;
use crate::game::map::auras::{AURA_DUMMY, AURA_MOD_CONFUSE, AURA_MOD_DAMAGE_DONE};
use crate::game::map::procs::{self, EX_NORMAL_HIT, Event};

/// A rogue's Blind, ten seconds (family flags `0x1000000`).
const BLIND: u32 = 997_901;
const RANGE_10: u32 = 990_901;

fn rogue(d: &mut GameData) {
    d.spells.ranges.insert(RANGE_10, (0.0, 10.0));
    let mut blind = bare(BLIND, 1);
    blind.family_name = 8;
    blind.family_flags = 0x100_0000;
    blind.range_index = RANGE_10;
    blind.duration_index = 990_901;
    d.spells.durations.insert(
        990_901,
        crate::game::data::spells::SpellDuration {
            base: 10_000,
            per_level: 0,
            max: 10_000,
        },
    );
    effect(&mut blind, 0, aura(AURA_MOD_CONFUSE, 0, 0, 6));
    insert(d, [blind]);
}

/// A rogue's Blind on a kobold, four times, each after the last is taken
/// off: ten seconds, five, two and a half, then not at all, as mangos's
/// `DIMINISHING_BLIND`, which counts for creatures too. Before, Blind
/// had no group and lasted ten seconds each time.
#[tokio::test]
async fn blind_diminishes_on_a_creature_too() {
    let realm = fast_realm(rogue);
    let (_c, r, map) = human(&realm, "P12EZ", "Blinder", Class::Rogue).await;
    let lasts = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, r, 60, &[BLIND]);
            let kobold = kit::dummy(m, r, 60, 3.0);
            (0..4)
                .map(|_| {
                    cast_fresh(m, r, BLIND, at(kobold));
                    let d = kit::holder(m, kobold, BLIND).and_then(|h| h.duration);
                    crate::game::map::auras::remove_spell(m, kobold, BLIND, None);
                    d
                })
                .collect::<Vec<_>>()
        })
        .await
        .unwrap();
    assert_eq!(
        lasts,
        vec![Some(10_000), Some(5_000), Some(2_500), None],
        "a half each time, then immune"
    );
}

/// Seal of Righteousness's eighth rank and its strike (the real ids the
/// code names, made up here); a hundred holy spell power; a two-hander of
/// 3.6 seconds and 100 to 200, a one-hander of 2 seconds and 50 to 100.
const SEAL: u32 = 20293;
const STRIKE: u32 = 25713;
const HOLY_POWER: u32 = 997_902;
const GREATSWORD: u32 = 991_901;
const SWORD: u32 = 991_902;

fn paladin(d: &mut GameData) {
    let weapon = |entry: u32, inventory_type: u32, delay: u32, min: f32, max: f32| ItemTemplate {
        entry,
        class: 2,
        subclass: 7,
        name: format!("Weapon {entry}"),
        display_id: 1000,
        inventory_type,
        delay,
        damages: [
            (min, max, 0),
            (0.0, 0.0, 0),
            (0.0, 0.0, 0),
            (0.0, 0.0, 0),
            (0.0, 0.0, 0),
        ],
        stackable: 1,
        ..Default::default()
    };
    d.items
        .insert(GREATSWORD, weapon(GREATSWORD, 17, 3600, 100.0, 200.0));
    d.items.insert(SWORD, weapon(SWORD, 13, 2000, 50.0, 100.0));
    let mut seal = bare(SEAL, 1);
    seal.family_name = 10;
    seal.family_flags = 0x800_0000;
    seal.spell_level = 58;
    seal.duration_index = LASTS_LONG;
    seal.proc_flags = procs::PROC_DONE_MELEE;
    seal.proc_chance = 100;
    effect(&mut seal, 0, aura(AURA_DUMMY, 0, 1000, 1));
    let mut strike = bare(STRIKE, 1);
    strike.attributes[2] |= ATTR_EX2_CANT_CRIT;
    strike.attributes[3] |= ATTR_EX3_ALWAYS_HIT;
    effect(&mut strike, 0, damage(1));
    // No spell power of its own: the seal's formula has it.
    strike.effects[0].bonus_coefficient = 0.0;
    let mut power = bare(HOLY_POWER, 1);
    power.duration_index = LASTS_LONG;
    effect(&mut power, 0, aura(AURA_MOD_DAMAGE_DONE, 0x2, 100, 1));
    insert(d, [seal, strike, power]);
}

fn blow(m: &mut Map, g: Guid, target: Guid) {
    procs::trigger(
        m,
        Event {
            actor: g,
            target,
            actor_flags: procs::PROC_DONE_MELEE,
            target_flags: procs::PROC_TAKEN_MELEE,
            ex: EX_NORMAL_HIT,
            school: 1,
            damage: 10,
            ..Default::default()
        },
    );
}

/// Seal of Righteousness's eighth rank (its dummy 1000) with a hundred
/// holy spell power: with a two-hander of 3.6 seconds and 100 to 200, its
/// strike is mangos's 1.2 x 1000 x 1.2 x 1.03 x 3.6 / 100 + 1, 3% of the
/// weapon's 150, one more, and 0.108 x 3.6 of the power: 97; with a
/// one-hander of 2 seconds and 50 to 100, 0.85 x ceil(24.72) - 1, 2.25,
/// one more and 0.092 x 2 of the power: 41. Before, both were the
/// tooltip's formula with no spell power (35 and 17).
#[tokio::test]
async fn seal_of_righteousness_strikes_by_mangos_formula() {
    let realm = fast_realm(paladin);
    let (_c, p, map) = human(&realm, "P12ER", "Righteous", Class::Paladin).await;
    let (two, one) = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, p, 60, &[]);
            kit::apply(m, p, p, HOLY_POWER);
            kit::apply(m, p, p, SEAL);
            let kobold = kit::dummy(m, p, 60, 2.0);
            let mut dealt = Vec::new();
            for w in [GREATSWORD, SWORD] {
                equip(m, p, w, 15);
                let before = kit::health(m, kobold);
                blow(m, p, kobold);
                dealt.push(before - kit::health(m, kobold));
            }
            (dealt[0], dealt[1])
        })
        .await
        .unwrap();
    assert_eq!(two, 97, "a two-hander's");
    assert_eq!(one, 41, "a one-hander's");
}

/// Frenzied Regeneration's first rank, its tick naming no spell, and the
/// heal mangos casts for it.
const FRENZIED: u32 = 22842;
const FRENZIED_HEAL: u32 = 22845;

fn druid(d: &mut GameData) {
    let mut frenzied = bare(FRENZIED, 0);
    frenzied.family_name = 7;
    frenzied.duration_index = LASTS_LONG;
    let mut tick = aura(
        crate::game::map::auras::AURA_PERIODIC_TRIGGER_SPELL,
        0,
        10,
        1,
    );
    tick.amplitude = 1_000;
    effect(&mut frenzied, 0, tick);
    let mut heal = bare(FRENZIED_HEAL, 0);
    heal.attributes[2] |= ATTR_EX2_CANT_CRIT;
    effect(
        &mut heal,
        0,
        crate::game::data::spells::SpellEffect {
            effect: 10,
            base_points: 0,
            base_dice: 1,
            target_a: 1,
            ..Default::default()
        },
    );
    heal.effects[0].bonus_coefficient = 0.0;
    insert(d, [frenzied, heal]);
}

/// A druid under Frenzied Regeneration (10 health a rage) with 30 rage:
/// a tick takes 10 rage and heals 100; with 5 rage left, it takes the 5
/// and heals 50. Before, its ticks did nothing.
#[tokio::test]
async fn frenzied_regeneration_turns_rage_into_health() {
    let realm = fast_realm(druid);
    let (_c, d, map) = human(&realm, "P12EG", "Frenzied", Class::Warrior).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, d, 60, &[]);
            kit::set_power(m, d, 1, 300);
            kit::set_health(m, d, 100);
            kit::apply(m, d, d, FRENZIED);
            kit::tick_now(m, d, FRENZIED);
            let first = (kit::power(m, d, 1), kit::health(m, d));
            kit::set_power(m, d, 1, 50);
            kit::tick_now(m, d, FRENZIED);
            (first, (kit::power(m, d, 1), kit::health(m, d)))
        })
        .await
        .unwrap();
    assert_eq!(out.0, (200, 200), "10 rage for 100 health");
    assert_eq!(out.1, (0, 250), "the 5 left for 50");
}

/// On the real data: a rogue with Ruthlessness (its third rank, made sure
/// to proc) and five combo points on a kobold finishes it with
/// Eviscerate: the finisher takes the five, and Ruthlessness's point is
/// on the kobold after it, as mangos-classic's `AddTriggeredSpell` defers
/// it past the finisher's own clearing.
#[tokio::test]
#[ignore = "extended"]
async fn ruthlessness_leaves_its_point_after_the_finisher() {
    let realm = kit::realm_plus(|d| {
        if let Some(s) = d.spells.spells.get_mut(&14161) {
            s.proc_chance = 100;
        }
    });
    let (_c, r, map) = kit::player(&realm, "P12EY", "Ruthless", Race::Human, Class::Rogue).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, r, 60, &[31016, 14161]);
            let kobold = kit::dummy(m, r, 60, 2.0);
            crate::game::map::spells::add_combo_points(m, r, kobold, 5);
            let before = kit::health(m, kobold);
            cast_fresh(m, r, 31016, at(kobold));
            let c = m.caster(r).unwrap();
            (
                before > kit::health(m, kobold),
                c.combo_points,
                c.combo_target == kobold,
            )
        })
        .await
        .unwrap();
    assert!(out.0, "Eviscerate hit");
    assert_eq!(
        (out.1, out.2),
        (1, true),
        "one point on the kobold after it"
    );
}
