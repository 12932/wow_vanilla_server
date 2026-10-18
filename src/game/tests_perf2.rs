//! The whole world awake, round two (docs/PERFORMANCE.md): the 500 ms
//! timers each on a phase of its own.

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
    assert!(bins.iter().all(|&b| b >= 5), "spread over the period: {bins:?}");
    // From a beat, the next is a period on: the period is kept.
    for (&g, &n) in guids.iter().zip(&next) {
        assert_eq!(next_beat(start, g, every, n), n + every);
    }
    // Before its first beat, that is the next.
    let g = guids[0];
    let first = next_beat(start, g, every, start);
    assert!(first > start && first <= start + every);
    assert_eq!(next_beat(start, g, every, first - Duration::from_millis(1)), first);
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
