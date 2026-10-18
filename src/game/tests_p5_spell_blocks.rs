//! Phase 3, round 5, against Thistle Tea's `melee_proc_test.exs`: what a
//! blow partly or wholly blocked, or absorbed, sets off on its defender:
//! their procs on a blow taken, on a block, their damage shields,
//! Retaliation's charges; and a proc's cooldown holding its charges.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_combat::*;
use crate::game::data::GameData;
use crate::game::data::spells::{ProcEvent, SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::{
    self, AURA_DAMAGE_SHIELD, AURA_DUMMY, AURA_PROC_TRIGGER_SPELL, AURA_SCHOOL_ABSORB,
};
use crate::game::map::combat::Outcome;
use crate::game::map::procs::EX_BLOCK;

/// On a blow taken; on a block alone; their marks; a damage shield of
/// five; an absorb; Retaliation and its strike.
const TAKEN: u32 = 991_200;
const BLOCKED: u32 = 991_201;
const TAKEN_MARK: u32 = 991_202;
const BLOCKED_MARK: u32 = 991_203;
const THORNS: u32 = 991_204;
const ABSORB: u32 = 991_205;
const RETALIATION: u32 = 20230;
const RETALIATION_STRIKE: u32 = 22858;

fn spell(id: u32, e: SpellEffect) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        equipped_item_class: -1,
        range_index: 1,
        duration_index: 21,
        ..Default::default()
    };
    s.effects[0] = e;
    s
}

fn aura_effect(kind: u32, trigger: u32, misc: i32, amount: i32) -> SpellEffect {
    SpellEffect {
        effect: 6,
        aura: kind,
        trigger_spell: trigger,
        misc_value: misc,
        base_points: amount - 1,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    }
}

fn blocks() -> GameData {
    let mut d = data();
    d.spells.durations.insert(
        21,
        crate::game::data::spells::SpellDuration {
            base: 7_200_000,
            per_level: 0,
            max: 7_200_000,
        },
    );
    let mut taken = spell(
        TAKEN,
        aura_effect(AURA_PROC_TRIGGER_SPELL, TAKEN_MARK, 0, 0),
    );
    taken.proc_flags = crate::game::map::procs::PROC_TAKEN_MELEE;
    taken.proc_chance = 100;
    taken.proc_charges = 2;
    let mut blocked = spell(
        BLOCKED,
        aura_effect(AURA_PROC_TRIGGER_SPELL, BLOCKED_MARK, 0, 0),
    );
    blocked.proc_flags = crate::game::map::procs::PROC_TAKEN_MELEE;
    blocked.proc_chance = 100;
    blocked.proc_charges = 2;
    d.spells.proc_events.insert(
        BLOCKED,
        ProcEvent {
            ex: EX_BLOCK,
            ..Default::default()
        },
    );
    let mut retaliation = spell(RETALIATION, aura_effect(AURA_DUMMY, 0, 0, 0));
    retaliation.proc_flags = crate::game::map::procs::PROC_TAKEN_MELEE;
    retaliation.proc_chance = 100;
    retaliation.proc_charges = 30;
    let mut strike = spell(
        RETALIATION_STRIKE,
        SpellEffect {
            effect: 2,
            base_points: 0,
            base_dice: 1,
            target_a: 6,
            ..Default::default()
        },
    );
    strike.attributes[3] |= crate::game::data::spells::ATTR_EX3_ALWAYS_HIT;
    strike.attributes[2] |= crate::game::data::spells::ATTR_EX2_CANT_CRIT;
    for s in [
        taken,
        blocked,
        spell(TAKEN_MARK, aura_effect(AURA_DUMMY, 0, 0, 0)),
        spell(BLOCKED_MARK, aura_effect(AURA_DUMMY, 0, 0, 0)),
        spell(THORNS, aura_effect(AURA_DAMAGE_SHIELD, 0, 0, 5)),
        retaliation,
        strike,
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

fn has(m: &Map, g: Guid, spell: u32) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has_spell(spell))
}

fn charges(m: &Map, g: Guid, spell: u32) -> Option<u32> {
    auras::auras(m, g).and_then(|a| {
        a.holders
            .iter()
            .find(|h| h.spell == spell)
            .map(|h| h.charges)
    })
}

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

fn put(m: &mut Map, g: Guid, id: u32) {
    let s = m.realm.data.spells.get(id).cloned().unwrap();
    let effects = [(0, crate::game::map::spells::points(&s, 0, 60))];
    let _ = auras::apply_spell(m, g, g, &s, &effects, auras::Context::default());
}

/// What one blow of `outcome` for `damage` from `brute` at `g` set off:
/// (taken mark, blocked mark, the brute's harm from the shield, the
/// defender's loss).
fn blow(
    m: &mut Map,
    brute: Guid,
    g: Guid,
    outcome: Outcome,
    damage: f32,
    absorb: i32,
) -> (bool, bool, u32, u32) {
    for s in [TAKEN, BLOCKED, TAKEN_MARK, BLOCKED_MARK, THORNS, ABSORB] {
        auras::remove_spell(m, g, s, None);
    }
    put(m, g, TAKEN);
    put(m, g, BLOCKED);
    put(m, g, THORNS);
    if absorb > 0 {
        aura_of(m, g, ABSORB, AURA_SCHOOL_ABSORB, 1, absorb);
    }
    if let Some(f) = m.unit_fields_mut(brute) {
        f.set_f32(UNIT_FIELD_MINDAMAGE, damage);
        f.set_f32(UNIT_FIELD_MAXDAMAGE, damage);
        f.set_u32(UNIT_FIELD_HEALTH, 100_000);
    }
    if let Some(f) = m.unit_fields_mut(g) {
        f.set_u32(UNIT_FIELD_HEALTH, 500);
    }
    let before = health(m, g);
    forced_swing(m, brute, g, outcome);
    (
        has(m, g, TAKEN_MARK),
        has(m, g, BLOCKED_MARK),
        100_000 - health(m, brute),
        before - health(m, g),
    )
}

/// A blow partly blocked sets off the defender's procs on a blow taken,
/// on a block, and their damage shield; one wholly blocked, only those on
/// a block and the shield; one wholly absorbed, the procs on a blow taken
/// but not the shield; one partly absorbed, both (Thistle Tea's
/// `receive_attack/4` tests of blocks and absorbs).
#[tokio::test]
async fn blocks_and_absorbs_set_off_what_landing_or_blocking_sets_off() {
    let (_realm, _a, g, map) = setup_with(blocks(), Class::Warrior).await;
    let out = map
        .query(move |m| {
            level(m, g, 60);
            equip(m, g, SHIELD, OFF_HAND as u8);
            let brute = spawn(m, g, BRUTE, 20, 2.0);
            let partly = blow(m, brute, g, Outcome::Block, 200.0, 0);
            let wholly = blow(m, brute, g, Outcome::Block, 1.0, 0);
            let absorbed = blow(m, brute, g, Outcome::Hit, 50.0, 10_000);
            let partly_absorbed = blow(m, brute, g, Outcome::Hit, 200.0, 20);
            (partly, wholly, absorbed, partly_absorbed)
        })
        .await
        .unwrap();
    let (partly, wholly, absorbed, partly_absorbed) = out;
    assert!(
        partly.0 && partly.1 && partly.2 == 5 && partly.3 > 0,
        "partly blocked: {partly:?}"
    );
    assert_eq!(wholly, (false, true, 5, 0), "wholly blocked");
    assert_eq!(absorbed, (true, false, 0, 0), "wholly absorbed");
    assert!(
        partly_absorbed.0 && partly_absorbed.2 == 5,
        "partly absorbed: {partly_absorbed:?}"
    );
}

/// Retaliation strikes back at a blow partly blocked, spending a charge,
/// and not at one wholly blocked (Thistle Tea's Retaliation test).
#[tokio::test]
async fn retaliation_spends_a_charge_on_a_partial_block_not_a_whole_one() {
    let mut d = blocks();
    d.spells.proc_events.remove(&RETALIATION);
    let (_realm, _a, g, map) = setup_with(d, Class::Warrior).await;
    let out = map
        .query(move |m| {
            level(m, g, 60);
            equip(m, g, SHIELD, OFF_HAND as u8);
            let brute = spawn(m, g, BRUTE, 20, 2.0);
            put(m, g, RETALIATION);
            let start = charges(m, g, RETALIATION);
            let mut seen = Vec::new();
            for damage in [200.0, 1.0] {
                if let Some(f) = m.unit_fields_mut(brute) {
                    f.set_f32(UNIT_FIELD_MINDAMAGE, damage);
                    f.set_f32(UNIT_FIELD_MAXDAMAGE, damage);
                    f.set_u32(UNIT_FIELD_HEALTH, 100_000);
                }
                forced_swing(m, brute, g, Outcome::Block);
                seen.push((charges(m, g, RETALIATION), 100_000 - health(m, brute)));
            }
            (start, seen)
        })
        .await
        .unwrap();
    let (start, seen) = out;
    assert_eq!(start, Some(30));
    assert_eq!(seen[0].0, Some(29), "a charge for a partial block");
    assert!(seen[0].1 > 0, "and its strike");
    assert_eq!(seen[1], (Some(29), 0), "none for a whole block");
}

/// A proc's cooldown keeps its charges until it is over (Thistle Tea's
/// "incoming proc cooldowns hold charges").
#[tokio::test]
async fn a_procs_cooldown_holds_its_charges() {
    let mut d = blocks();
    d.spells.proc_events.insert(
        TAKEN,
        ProcEvent {
            cooldown: 1_000,
            ..Default::default()
        },
    );
    let (_realm, _a, g, map) = setup_with(d, Class::Warrior).await;
    let out = map
        .query(move |m| {
            level(m, g, 60);
            let brute = spawn(m, g, BRUTE, 20, 2.0);
            put(m, g, TAKEN);
            let mut seen = Vec::new();
            for wait in [0, 999, 1] {
                m.now += Duration::from_millis(wait);
                auras::remove_spell(m, g, TAKEN_MARK, None);
                forced_swing(m, brute, g, Outcome::Hit);
                seen.push((has(m, g, TAKEN_MARK), charges(m, g, TAKEN)));
            }
            seen
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        [(true, Some(1)), (false, Some(1)), (true, None)],
        "a charge, then held through the cooldown, then the last"
    );
}
