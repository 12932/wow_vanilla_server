//! The whole world awake, round two (docs/PERFORMANCE.md): the 500 ms
//! timers each on a phase of its own, and the awake set pruned once an
//! update.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::guid;
use crate::game::harness::Client;
use crate::game::map::{Map, ai, environment, next_beat, visibility};
use crate::game::tests_p3_combat::*;

/// How many of `waits` fall in each fifth of `every`, (0, every].
fn fifths(waits: &[Duration], every: Duration) -> [usize; 5] {
    let mut bins = [0; 5];
    for w in waits {
        assert!(
            !w.is_zero() && *w <= every,
            "{w:?}: due again within the period"
        );
        let i = ((w.as_secs_f64() / every.as_secs_f64()) * 5.0).ceil() as usize;
        bins[i.clamp(1, 5) - 1] += 1;
    }
    bins
}

#[test]
fn a_timers_beats_keep_its_period_each_on_a_phase_of_its_own() {
    let every = Duration::from_millis(500);
    let start = Instant::now();
    let now = start + Duration::from_millis(7_321);
    let guids: Vec<Guid> = (1..=100).map(|i| guid::unit(BRUTE, i)).collect();
    let next: Vec<Instant> = guids
        .iter()
        .map(|&g| next_beat(start, g, every, now))
        .collect();
    let waits: Vec<Duration> = next.iter().map(|n| *n - now).collect();
    let bins = fifths(&waits, every);
    assert!(
        bins.iter().all(|&b| b >= 5),
        "spread over the period: {bins:?}"
    );
    // From a beat, the next is a period on: the period is kept.
    for (&g, &n) in guids.iter().zip(&next) {
        assert_eq!(next_beat(start, g, every, n), n + every);
    }
    // Before its first beat, that is the next.
    let g = guids[0];
    let first = next_beat(start, g, every, start);
    assert!(first > start && first <= start + every);
    assert_eq!(
        next_beat(start, g, every, first - Duration::from_millis(1)),
        first
    );
}

/// A spawn `yards` east of the start, its aggro check due now, seen.
fn seen_brute(m: &mut Map, ga: Guid, yards: f32) -> Guid {
    let g = spawn(m, ga, BRUTE, 20, yards);
    visibility::refresh(m, g);
    let now = m.now;
    m.creature_mut(g).unwrap().aggro_check_at = now;
    g
}

#[tokio::test]
async fn creatures_that_came_into_view_together_look_again_each_on_a_phase_of_its_own() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let (waits, observed) = map
        .query(move |m| {
            let gs: Vec<Guid> = (0..40)
                .map(|i| seen_brute(m, ga, 30.0 + i as f32 * 0.5))
                .collect();
            let now = m.now;
            ai::update_creatures(m);
            let waits: Vec<Duration> = gs
                .iter()
                .map(|&g| m.creature(g).unwrap().aggro_check_at - now)
                .collect();
            let observed = gs
                .iter()
                .all(|&g| !m.creature(g).unwrap().observers.is_empty());
            (waits, observed)
        })
        .await
        .unwrap();
    assert!(observed, "every one is seen, so each looked");
    // They looked together; they look again spread over the next 500 ms,
    // not all in one update 500 ms on.
    let bins = fifths(&waits, Duration::from_millis(500));
    assert!(bins.iter().all(|&b| b <= 20), "spread: {bins:?}");
}

#[tokio::test]
async fn players_environment_checks_come_each_on_a_phase_of_its_own() {
    let (realm, _a, ga, map) = setup(Class::Warrior).await;
    let mut clients = Vec::new();
    let mut guids = vec![ga];
    for i in 0..9 {
        let name = format!("Pl{}", (b'a' + i) as char);
        let mut c = Client::connect(&realm, &name);
        guids.push(c.enter(&name, Race::Human, Class::Warrior).await);
        c.heartbeat(START.0, START.1, START.2).await;
        clients.push(c);
    }
    let waits = map
        .query(move |m| {
            for g in &guids {
                m.players.get_mut(g).unwrap().misc.env_checked_at = None;
            }
            let now = m.now;
            environment::update(m);
            guids
                .iter()
                .map(|g| {
                    let checked = m.players[g].misc.env_checked_at.unwrap();
                    assert!(checked <= now, "checked as of now or before");
                    checked + Duration::from_millis(500) - now
                })
                .collect::<Vec<_>>()
        })
        .await
        .unwrap();
    // Checked together, each is due again on its own phase.
    let bins = fifths(&waits, Duration::from_millis(500));
    assert!(bins.iter().all(|&b| b <= 7), "spread: {bins:?}");
}

/// The awake creatures a look finds, as `Map::awake_where` answers (which
/// debug builds check against a look through every creature).
fn awake_now(m: &mut Map) -> Vec<Guid> {
    m.awake_where(crate::game::map::awake, |_, c| c.guid)
}

#[tokio::test]
async fn a_creature_woken_between_an_updates_looks_is_found_by_the_later_ones() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    map.query(move |m| {
        // Out of sight, and asleep.
        let far = spawn(m, ga, BRUTE, 20, 400.0);
        let seen = seen_brute(m, ga, 30.0);
        m.stats.updates += 1;
        // The update's first look prunes the set.
        assert_eq!(awake_now(m), vec![seen]);
        assert!(!m.awake.contains(&far));
        // Another's turn draws it into a fight, unseen: attacked, not yet
        // fighting back.
        m.combat_mut(far).unwrap().attackers.insert(ga);
        let fighting = m.awake_where(
            |c| c.alive() && (c.combat.attacking || !c.combat.attackers.is_empty()),
            |_, c| c.guid,
        );
        assert_eq!(fighting, vec![far]);
        assert_eq!(awake_now(m).len(), 2);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn a_creature_gone_and_its_key_taken_or_back_under_another_key_is_found_as_it_now_is() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    map.query(move |m| {
        let gone = seen_brute(m, ga, 30.0);
        m.stats.updates += 1;
        assert_eq!(awake_now(m), vec![gone]);
        let key = m.creature_by_guid[&gone];
        let mut back = m.remove_creature(gone).unwrap();
        // Its key is another's now, one asleep: not awake, it isn't found.
        let other = spawn(m, ga, BRUTE, 20, 400.0);
        assert_eq!(
            m.creature_by_guid[&other], key,
            "the slab gave its key again"
        );
        assert!(awake_now(m).is_empty());
        // Woken, it is.
        m.note_fighting(other);
        m.creature_mut(other).unwrap().combat.attacking = true;
        assert_eq!(awake_now(m), vec![other]);
        // The first comes back, fighting, under a key of its own.
        back.combat.attacking = true;
        m.add_creature(back);
        assert_ne!(m.creature_by_guid[&gone], key);
        let mut both = vec![other, gone];
        both.sort_by_key(|g| m.creature_by_guid[g]);
        assert_eq!(awake_now(m), both);
        // A creature that was a member, back asleep, is found once woken.
        let mut again = m.remove_creature(gone).unwrap();
        again.combat.attacking = false;
        m.add_creature(again);
        assert_eq!(awake_now(m), vec![other]);
        m.note_fighting(gone);
        m.creature_mut(gone).unwrap().combat.attacking = true;
        assert_eq!(awake_now(m).len(), 2);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn the_awake_set_lets_go_of_those_no_longer_awake_at_the_next_updates_first_look() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    map.query(move |m| {
        let g = spawn(m, ga, BRUTE, 20, 400.0);
        m.note_fighting(g);
        m.creature_mut(g).unwrap().combat.attacking = true;
        m.stats.updates += 1;
        assert_eq!(awake_now(m), vec![g]);
        m.creature_mut(g).unwrap().combat.attacking = false;
        assert!(awake_now(m).is_empty(), "asleep, it isn't found");
        m.stats.updates += 1;
        assert!(awake_now(m).is_empty());
        assert!(!m.awake.contains(&g), "and is let go");
    })
    .await
    .unwrap();
}
