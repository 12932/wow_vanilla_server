//! Phase 3's game events against Thistle Tea's tests: an event whose
//! length covers its recurrence, the Darkmoon Faire's calendar when the
//! month starts on a Monday, the dragons of Nightmare always out, and the
//! holidays' fireworks and toasts.

use ahash::AHashSet;

use crate::game::data::{GameData, GameEvent};
use crate::game::events::{
    self, DARKMOON_ELWYNN, DRAGONS_OF_NIGHTMARE, FIREWORKS, TOASTING_GOBLETS,
};

fn at(s: &str) -> i64 {
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
        .unwrap()
        .and_utc()
        .timestamp()
}

#[test]
fn an_event_longer_than_its_recurrence_is_on_throughout() {
    // Every hour, for two: on all the while.
    let e = GameEvent {
        id: 1,
        name: "Always".into(),
        start: at("2026-01-01 00:00:00"),
        end: at("2026-01-02 00:00:00"),
        occurence: 60,
        length: 120,
        hardcoded: false,
    };
    for t in [
        "2026-01-01 00:00:00",
        "2026-01-01 12:00:00",
        "2026-01-01 23:59:59",
    ] {
        assert!(e.active(at(t)), "{t}");
    }
    assert!(!e.active(at("2026-01-02 00:00:00")));
}

#[test]
fn the_faire_skips_its_building_when_the_month_starts_on_a_monday_and_the_dragons_stay_out() {
    let day = |s: &str| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
    // June 2026 starts on a Monday: no building at the end of May, the
    // Faire open in Elwynn from the 1st.
    assert_eq!(events::darkmoon(day("2026-05-31")), None);
    assert_eq!(events::darkmoon(day("2026-06-01")), Some(DARKMOON_ELWYNN));
    // Nightmare's dragons, a hardcoded event, are out at any time.
    let mut d = GameData::empty();
    d.events.insert(
        DRAGONS_OF_NIGHTMARE,
        GameEvent {
            id: DRAGONS_OF_NIGHTMARE,
            name: "Dragons of Nightmare".into(),
            hardcoded: true,
            ..Default::default()
        },
    );
    for t in ["2026-10-03 12:00:00", "2030-01-01 00:00:00"] {
        assert!(
            events::active(&d, at(t)).contains(&DRAGONS_OF_NIGHTMARE),
            "{t}"
        );
    }
}

#[test]
fn the_holidays_nights_have_fireworks_and_only_the_winter_ones_a_toast() {
    let rules = |t: &str, holidays: &[u32]| {
        let scheduled: AHashSet<u32> = holidays.iter().copied().collect();
        let on = events::rules(at(t), &scheduled);
        (on.contains(&FIREWORKS), on.contains(&TOASTING_GOBLETS))
    };
    // New Year's: fireworks the first ten minutes of a night hour, then the
    // toast to twenty past.
    assert_eq!(rules("2026-12-31 18:05:00", &[34]), (true, false));
    assert_eq!(rules("2026-12-31 18:10:00", &[34]), (false, true));
    assert_eq!(rules("2026-12-31 18:20:59", &[34]), (false, true));
    assert_eq!(rules("2026-12-31 18:21:00", &[34]), (false, false));
    assert_eq!(rules("2027-01-01 06:09:59", &[34]), (true, false));
    assert_eq!(rules("2027-01-01 07:05:00", &[34]), (false, false));
    // The Fourth of July: fireworks, and no toast; no holiday, nothing.
    assert_eq!(rules("2026-07-04 23:05:00", &[41]), (true, false));
    assert_eq!(rules("2026-07-04 23:15:00", &[41]), (false, false));
    assert_eq!(rules("2026-07-05 23:05:00", &[]), (false, false));
}
