//! Phase 3's combat tests: Shield Block's charges, against the real data
//! (Thistle Tea's `shield_block_test.exs`): spent by a block, full or
//! partial, kept through misses, dodges, parries and plain hits, gone at
//! its end; and a block-value aura's bonus that comes and goes with it.

use wow_world_messages::vanilla::*;

use crate::game::map::auras;
use crate::game::map::combat::{self, Outcome};
use crate::game::tests_p3_combat::forced_swing;
use crate::game::tests_p3_spells_kit::*;

const SHIELD_BLOCK: u32 = 2565;
/// A block value of 235 (Lucky Fishing Hat's kind of buff).
const BLOCK_VALUE: u32 = 28_773;

#[tokio::test]
#[ignore = "extended"]
async fn shield_block_is_spent_by_a_block_kept_through_anything_else_and_gone_at_its_end() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = player(&realm, "P3P1", "Blocker", Race::Human, Class::Warrior).await;
    let out = map
        .query(move |m| {
            ready(m, g, 60, &[]);
            let brute = dummy(m, g, 60, 2.0);
            let charges = |m: &crate::game::map::Map| holder(m, g, SHIELD_BLOCK).map(|h| h.charges);
            let mut out = Vec::new();
            // A full block and a partial one: each spends its one charge.
            for damage in [10.0, 1000.0] {
                set_health(m, g, 100_000);
                apply(m, g, g, SHIELD_BLOCK);
                let before = charges(m);
                let f = m.unit_fields_mut(brute).unwrap();
                f.set_f32(crate::game::fields::UNIT_FIELD_MINDAMAGE, damage);
                f.set_f32(crate::game::fields::UNIT_FIELD_MAXDAMAGE, damage);
                forced_swing(m, brute, g, Outcome::Block);
                out.push((before, charges(m)));
            }
            // A miss, a dodge, a parry and a hit keep it (blows of ten,
            // and her health topped up, so she lives through them).
            let f = m.unit_fields_mut(brute).unwrap();
            f.set_f32(crate::game::fields::UNIT_FIELD_MINDAMAGE, 10.0);
            f.set_f32(crate::game::fields::UNIT_FIELD_MAXDAMAGE, 10.0);
            let max = health(m, g).max(1000);
            set_health(m, g, max);
            apply(m, g, g, SHIELD_BLOCK);
            for o in [Outcome::Miss, Outcome::Dodge, Outcome::Parry, Outcome::Hit] {
                forced_swing(m, brute, g, o);
                out.push((Some(1), charges(m)));
            }
            // Two charges: the first block leaves one.
            if let Some(a) = auras::auras_mut(m, g) {
                a.holders
                    .iter_mut()
                    .filter(|h| h.spell == SHIELD_BLOCK)
                    .for_each(|h| h.charges = 2);
            }
            forced_swing(m, brute, g, Outcome::Block);
            out.push((Some(2), charges(m)));
            forced_swing(m, brute, g, Outcome::Block);
            out.push((Some(1), charges(m)));
            // Its end, and a death, take it off unspent.
            apply(m, g, g, SHIELD_BLOCK);
            pass(m, 6_000);
            out.push((Some(1), charges(m)));
            apply(m, g, g, SHIELD_BLOCK);
            combat::deal_damage(m, brute, g, 1_000_000);
            out.push((Some(1), charges(m)));
            out
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        [
            (Some(1), None),
            (Some(1), None),
            (Some(1), Some(1)),
            (Some(1), Some(1)),
            (Some(1), Some(1)),
            (Some(1), Some(1)),
            (Some(2), Some(1)),
            (Some(1), None),
            (Some(1), None),
            (Some(1), None),
        ]
    );
}

#[tokio::test]
#[ignore = "extended"]
async fn a_block_value_aura_adds_while_it_lasts_and_not_after_its_end_removal_or_a_death() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = player(&realm, "P3P2", "Valued", Race::Human, Class::Warrior).await;
    let out = map
        .query(move |m| {
            ready(m, g, 60, &[]);
            let brute = dummy(m, g, 60, 2.0);
            let base = combat::block_value(m, g);
            apply(m, g, g, BLOCK_VALUE);
            let with = combat::block_value(m, g);
            // Again: no more.
            apply(m, g, g, BLOCK_VALUE);
            let again = combat::block_value(m, g);
            auras::remove_spell(m, g, BLOCK_VALUE, None);
            let removed = combat::block_value(m, g);
            apply(m, g, g, BLOCK_VALUE);
            pass(m, 3_600_000);
            let expired = combat::block_value(m, g);
            apply(m, g, g, BLOCK_VALUE);
            combat::deal_damage(m, brute, g, 1_000_000);
            let dead = combat::block_value(m, g);
            (base, with, again, removed, expired, dead)
        })
        .await
        .unwrap();
    let (base, with, again, removed, expired, dead) = out;
    assert_eq!(with, base + 235);
    assert_eq!(again, with);
    assert_eq!([removed, expired, dead], [base; 3]);
}
