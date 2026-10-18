//! Phase 5's auras on the real data, Thistle Tea's `DepletingAuraTest`
//! and its DBC test ported: Restless Strength, Brittle Armor and Mercurial
//! Shield start full, lose a stack for what lands, and go with the aura
//! that gave them.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::fields::*;
use crate::game::map::auras::{self, AURA_MOD_SKILL};
use crate::game::map::combat::Outcome;
use crate::game::map::procs::{self, *};
use crate::game::map::{Map, spells};
use crate::game::tests_p3_combat::forced_swing;

const RESTLESS_STRENGTH: u32 = 24_661;
const RESTLESS_STRENGTH_BONUS: u32 = 24_662;
const BRITTLE_ARMOR: u32 = 24_574;
const BRITTLE_ARMOR_BONUS: u32 = 24_575;
const MERCURIAL_SHIELD: u32 = 26_463;
const MERCURIAL_SHIELD_BONUS: u32 = 26_464;
const AUTO_SHOT: u32 = 75;
const ARCANE_SHOT: u32 = 3044;
const FIREBALL: u32 = 133;
const CORRUPTION: u32 = 172;

fn stacks(m: &Map, g: Guid, spell: u32) -> Option<u32> {
    holder(m, g, spell).map(|h| h.stacks)
}

/// The count the client shows for `g`'s `spell`: its slot's byte of
/// `UNIT_FIELD_AURAAPPLICATIONS`.
fn shown(m: &Map, g: Guid, spell: u32) -> Option<u32> {
    let slot = holder(m, g, spell)?.slot?;
    let word = m
        .unit_fields(g)?
        .get_u32(UNIT_FIELD_AURAAPPLICATIONS + (slot / 4) as u16);
    Some(word >> ((slot % 4) * 8) & 0xFF)
}

fn field(m: &Map, g: Guid, index: u16) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(index))
}

fn float(m: &Map, g: Guid, index: u16) -> f32 {
    m.unit_fields(g).map_or(0.0, |f| f.get_f32(index))
}

/// `who` does (or takes, `taken`) what `flags` say with `spell`, to `ex`.
fn event(actor: Guid, target: Guid, flags: (u32, u32), ex: u32, spell: Option<u32>) -> Event {
    Event {
        actor,
        target,
        actor_flags: flags.0,
        target_flags: flags.1,
        ex,
        spell,
        damage: if ex & (EX_NORMAL_HIT | EX_CRITICAL_HIT) != 0 {
            10
        } else {
            0
        },
        school: 1,
        ..Default::default()
    }
}

/// Thistle Tea's Restless Strength: twenty stacks shown as such, nothing
/// on empty hands; a stack spent for
/// each melee blow or ranged attack that lands (a crit, a glancing, a
/// crushing blow or a block too, and Auto Shot and Arcane Shot); none for
/// a miss, a dodge or parry, magic or a tick. Spent out, the bonus is gone
/// and the trinket's aura stays; on again (once it is gone), twenty.
#[tokio::test]
#[ignore = "extended"]
async fn restless_strength_starts_full_and_spends_a_stack_a_blow_that_lands() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, w, map) = player(&realm, "P5D1", "Restlessa", Race::Human, Class::Warrior).await;
    let got = map
        .query(move |m| {
            ready(m, w, 60, &[]);
            let kobold = dummy(m, w, 60, 2.0);
            let before = (
                float(m, w, UNIT_FIELD_MINDAMAGE),
                float(m, w, UNIT_FIELD_MINOFFHANDDAMAGE),
                float(m, w, UNIT_FIELD_MINRANGEDDAMAGE),
            );
            spells::cast_triggered(m, w, RESTLESS_STRENGTH, Some(w));
            let full = (
                has(m, w, RESTLESS_STRENGTH),
                stacks(m, w, RESTLESS_STRENGTH_BONUS),
                shown(m, w, RESTLESS_STRENGTH_BONUS),
            );
            let after = (
                float(m, w, UNIT_FIELD_MINDAMAGE),
                float(m, w, UNIT_FIELD_MINOFFHANDDAMAGE),
                float(m, w, UNIT_FIELD_MINRANGEDDAMAGE),
            );
            let mut spent = Vec::new();
            for outcome in [
                Outcome::Hit,
                Outcome::Crit,
                Outcome::Glancing,
                Outcome::Crushing,
                Outcome::Block,
            ] {
                let was = stacks(m, w, RESTLESS_STRENGTH_BONUS);
                forced_swing(m, w, kobold, outcome);
                spent.push((outcome, was.zip(stacks(m, w, RESTLESS_STRENGTH_BONUS))));
            }
            for (flags, spell) in [
                (PROC_DONE_RANGED, Some(AUTO_SHOT)),
                (PROC_DONE_RANGED_SPELL, Some(ARCANE_SHOT)),
            ] {
                let was = stacks(m, w, RESTLESS_STRENGTH_BONUS);
                procs::trigger(m, event(w, kobold, (flags, 0), EX_NORMAL_HIT, spell));
                spent.push((Outcome::Hit, was.zip(stacks(m, w, RESTLESS_STRENGTH_BONUS))));
            }
            let mut kept = Vec::new();
            for outcome in [Outcome::Miss, Outcome::Dodge, Outcome::Parry] {
                forced_swing(m, w, kobold, outcome);
                kept.push(stacks(m, w, RESTLESS_STRENGTH_BONUS));
            }
            for (flags, ex) in [
                (PROC_DONE_NEGATIVE_SPELL, EX_NORMAL_HIT),
                (PROC_DONE_PERIODIC, EX_NORMAL_HIT),
                (PROC_DONE_POSITIVE_SPELL, EX_NORMAL_HIT),
            ] {
                procs::trigger(m, event(w, kobold, (flags, 0), ex, Some(FIREBALL)));
                kept.push(stacks(m, w, RESTLESS_STRENGTH_BONUS));
            }
            let left = stacks(m, w, RESTLESS_STRENGTH_BONUS).unwrap_or(0);
            for _ in 0..left {
                forced_swing(m, w, kobold, Outcome::Hit);
            }
            let out = (
                has(m, w, RESTLESS_STRENGTH_BONUS),
                has(m, w, RESTLESS_STRENGTH),
                float(m, w, UNIT_FIELD_MINDAMAGE),
            );
            forced_swing(m, w, kobold, Outcome::Hit);
            // The trinket's two minutes are longer than its twenty
            // seconds: it is on again only once its aura is gone.
            auras::remove_spell(m, w, RESTLESS_STRENGTH, None);
            fresh(m, w);
            spells::cast_triggered(m, w, RESTLESS_STRENGTH, Some(w));
            let again = stacks(m, w, RESTLESS_STRENGTH_BONUS);
            (before, full, after, spent, kept, out, again)
        })
        .await
        .unwrap();
    let (before, full, after, spent, kept, out, again) = got;
    assert_eq!(full, (true, Some(20), Some(19)), "twenty, shown as such");
    // The bonus goes on each blow, not on the fields (Thistle Tea and
    // mangos put it there): the empty hands' fields stay as they were.
    assert_eq!(
        (after.1, after.2),
        (before.1, before.2),
        "not on empty hands"
    );
    let mut was = 20;
    for (outcome, s) in spent {
        assert_eq!(s, Some((was, was - 1)), "{outcome:?} spends one");
        was -= 1;
    }
    assert!(kept.iter().all(|s| *s == Some(was)), "{kept:?}");
    assert_eq!(out, (false, true, before.0), "spent out, the bonus gone");
    assert_eq!(again, Some(20));
}

/// Thistle Tea's Brittle Armor and Mercurial Shield: ten stacks, 200 armor
/// and three defense each, ten of each magic school's resistance each; a
/// blow that lands takes one off the armor, without moving its end, and a
/// harmful spell one off the shield; avoided blows, a spell resisted, a
/// tick and a blow on the shield spend none. Spent out, the bonus goes and
/// the trinket's aura stays.
#[tokio::test]
#[ignore = "extended"]
async fn brittle_armor_and_mercurial_shield_lose_a_stack_for_what_lands() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, w, map) = player(&realm, "P5D2", "Brittleb", Race::Human, Class::Warrior).await;
    let (_d, mage, _) = player(&realm, "P5D3", "Mercuryb", Race::Human, Class::Mage).await;
    let got = map
        .query(move |m| {
            ready(m, w, 60, &[]);
            let armor = field(m, w, UNIT_FIELD_RESISTANCES);
            let fire = field(m, w, UNIT_FIELD_RESISTANCES + 2);
            spells::cast_triggered(m, w, BRITTLE_ARMOR, Some(w));
            spells::cast_triggered(m, w, MERCURIAL_SHIELD, Some(w));
            let bonus = (
                field(m, w, UNIT_FIELD_RESISTANCES) - armor,
                auras::auras(m, w).map_or(0, |a| a.sum_misc(AURA_MOD_SKILL, 95)),
                (1..=6)
                    .map(|i| field(m, w, UNIT_FIELD_RESISTANCES + i))
                    .collect::<Vec<_>>(),
            );
            let full = (
                stacks(m, w, BRITTLE_ARMOR_BONUS),
                stacks(m, w, MERCURIAL_SHIELD_BONUS),
            );
            let ends = holder(m, w, BRITTLE_ARMOR_BONUS).and_then(|h| h.expires);
            let blow = |ex| {
                event(
                    mage,
                    w,
                    (PROC_DONE_MELEE, PROC_TAKEN_MELEE | PROC_TAKEN_DAMAGE),
                    ex,
                    None,
                )
            };
            let spell = |ex, taken, s| event(mage, w, (0, taken), ex, Some(s));
            let mut kept = Vec::new();
            for ex in [EX_MISS, EX_DODGE, EX_PARRY, EX_BLOCK] {
                procs::trigger(m, blow(ex));
                kept.push(stacks(m, w, BRITTLE_ARMOR_BONUS));
            }
            for e in [
                spell(EX_RESIST, PROC_TAKEN_NEGATIVE_SPELL, FIREBALL),
                spell(
                    EX_NORMAL_HIT,
                    PROC_TAKEN_PERIODIC | PROC_TAKEN_DAMAGE,
                    CORRUPTION,
                ),
            ] {
                procs::trigger(m, e);
                kept.push(stacks(m, w, MERCURIAL_SHIELD_BONUS));
            }
            m.now += Duration::from_secs(1);
            let mut armor_left = Vec::new();
            for _ in 0..9 {
                procs::trigger(m, blow(EX_NORMAL_HIT));
                armor_left.push((
                    stacks(m, w, BRITTLE_ARMOR_BONUS),
                    field(m, w, UNIT_FIELD_RESISTANCES) - armor,
                ));
            }
            let same_end = holder(m, w, BRITTLE_ARMOR_BONUS).and_then(|h| h.expires) == ends;
            let shield_after_blows = stacks(m, w, MERCURIAL_SHIELD_BONUS);
            procs::trigger(m, blow(EX_NORMAL_HIT));
            let armor_out = (
                has(m, w, BRITTLE_ARMOR_BONUS),
                has(m, w, BRITTLE_ARMOR),
                field(m, w, UNIT_FIELD_RESISTANCES) - armor,
            );
            let mut shield_left = Vec::new();
            for _ in 0..10 {
                procs::trigger(
                    m,
                    spell(
                        EX_NORMAL_HIT,
                        PROC_TAKEN_NEGATIVE_SPELL | PROC_TAKEN_DAMAGE,
                        FIREBALL,
                    ),
                );
                shield_left.push(field(m, w, UNIT_FIELD_RESISTANCES + 2) - fire);
            }
            let shield_out = (
                has(m, w, MERCURIAL_SHIELD_BONUS),
                has(m, w, MERCURIAL_SHIELD),
            );
            (
                bonus,
                full,
                kept,
                armor_left,
                same_end,
                shield_after_blows,
                armor_out,
                shield_left,
                shield_out,
            )
        })
        .await
        .unwrap();
    let (
        bonus,
        full,
        kept,
        armor_left,
        same_end,
        shield_after_blows,
        armor_out,
        shield_left,
        shield_out,
    ) = got;
    assert_eq!(
        bonus,
        (2_000, 30, vec![0, 100, 100, 100, 100, 100]),
        "holy none"
    );
    assert_eq!(full, (Some(10), Some(10)));
    assert_eq!(kept, vec![Some(10); 6], "avoided, resisted, ticked: kept");
    for (i, (s, a)) in armor_left.into_iter().enumerate() {
        assert_eq!((s, a), (Some(9 - i as u32), 200 * (9 - i as u32)));
    }
    assert!(same_end, "the end doesn't move");
    assert_eq!(shield_after_blows, Some(10), "blows don't touch the shield");
    assert_eq!(armor_out, (false, true, 0));
    assert_eq!(
        shield_left,
        (0..10).rev().map(|n| n * 10).collect::<Vec<_>>()
    );
    assert_eq!(shield_out, (false, true));
}

/// Thistle Tea's cleanup: each trinket's bonus goes when its aura is taken
/// off, runs out, or its wearer dies; a bonus cast with no trinket's aura
/// doesn't go on.
#[tokio::test]
#[ignore = "extended"]
async fn a_trinkets_bonus_goes_with_its_aura_and_never_comes_alone() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, w, map) = player(&realm, "P5D4", "Cleanc", Race::Human, Class::Warrior).await;
    let pairs = [
        (RESTLESS_STRENGTH, RESTLESS_STRENGTH_BONUS),
        (BRITTLE_ARMOR, BRITTLE_ARMOR_BONUS),
        (MERCURIAL_SHIELD, MERCURIAL_SHIELD_BONUS),
    ];
    let got = map
        .query(move |m| {
            ready(m, w, 60, &[]);
            let armor = field(m, w, UNIT_FIELD_RESISTANCES);
            let mut out = Vec::new();
            for (parent, bonus) in pairs {
                // Alone.
                spells::cast_triggered(m, w, bonus, Some(w));
                let alone = has(m, w, bonus);
                // Taken off.
                spells::cast_triggered(m, w, parent, Some(w));
                auras::remove_stack(m, w, bonus);
                auras::remove_spell(m, w, parent, None);
                let removed = has(m, w, bonus);
                // Run out.
                fresh(m, w);
                spells::cast_triggered(m, w, parent, Some(w));
                let lasts = holder(m, w, parent).and_then(|h| h.duration).unwrap_or(0);
                pass(m, u64::from(lasts));
                let expired = has(m, w, bonus) || has(m, w, parent);
                out.push((parent, alone, removed, expired));
            }
            let back = field(m, w, UNIT_FIELD_RESISTANCES) == armor;
            // Dead.
            for (parent, _) in pairs {
                fresh(m, w);
                spells::cast_triggered(m, w, parent, Some(w));
            }
            let kobold = dummy(m, w, 60, 2.0);
            crate::game::map::combat::deal_damage(m, kobold, w, 10_000_000);
            let dead = pairs.iter().any(|(p, b)| has(m, w, *p) || has(m, w, *b));
            (out, back, dead)
        })
        .await
        .unwrap();
    let (out, back, dead) = got;
    for (parent, alone, removed, expired) in out {
        assert!(!alone, "{parent}'s bonus alone");
        assert!(!removed, "{parent}'s bonus outlived it");
        assert!(!expired, "{parent} ran out with its bonus");
    }
    assert!(back, "the armor back");
    assert!(!dead, "death takes them all");
}
