//! Round 6, against Thistle Tea's `creature_spell_test.exs`
//! (docs/parity/matrix.csv): a creature spell list entry's chance, an
//! entry with no spell, and the window of its first delay.

use std::time::Duration;

use crate::game::map::spells::lists::{chance_of, roll};

/// Thistle Tea's "normalizes out-of-range probability to 100": an entry's
/// probability of none, or past a hundred, is a sure one; another is its
/// own.
#[test]
fn a_probability_of_none_or_past_a_hundred_is_a_hundred() {
    assert_eq!(chance_of(0), 100);
    assert_eq!(chance_of(150), 100);
    assert_eq!(chance_of(100), 100);
    assert_eq!(chance_of(25), 25);
}

/// Thistle Tea's "stays within the configured window" (and "is exact for
/// a fixed window"): an initial delay is rolled within its window, a
/// window of one width that.
#[test]
fn a_delay_is_rolled_within_its_window() {
    for _ in 0..50 {
        let d = roll((2_000, 4_000));
        assert!(
            (Duration::from_millis(2_000)..=Duration::from_millis(4_000)).contains(&d),
            "{d:?}"
        );
    }
    assert_eq!(roll((2_000, 2_000)), Duration::from_millis(2_000));
}

/// Thistle Tea's "returns nil without a spell id": a list's slots of no
/// spell aren't loaded as entries.
#[test]
#[ignore = "extended"]
fn a_lists_slots_of_no_spell_are_no_entries() {
    let Some(d) = crate::game::data::load::real_data() else {
        return;
    };
    let lists = &d.spells.creature_spells;
    assert!(!lists.is_empty());
    assert!(lists.values().flatten().all(|e| e.spell != 0));
    assert!(lists.values().any(|l| l.len() < 8), "some lists have empty slots");
}
