//! Phase 3, round 4, against Thistle Tea's `movement_test.exs`: a
//! creature's walk, where it is along it and which way it faces, what is
//! left of it, a run retimed at a new speed (and which walks aren't), and
//! a stop: once, and none for a walk that ended of itself.

use std::time::{Duration, Instant};

use wow_world_messages::vanilla::*;

use crate::game::data::CreatureTemplate;
use crate::game::guid;
use crate::game::map::ai;
use crate::game::map::creature::{Creature, Motion, Walk};
use crate::game::tests_p3_combat::*;

fn v(x: f32, y: f32) -> Vector3d {
    Vector3d { x, y, z: 0.0 }
}

fn near(a: Vector3d, b: Vector3d) -> bool {
    (a.x - b.x).abs() < 0.01 && (a.y - b.y).abs() < 0.01 && (a.z - b.z).abs() < 0.01
}

#[test]
fn a_walk_is_where_its_time_has_brought_it_facing_its_segment_and_clamped_at_its_ends() {
    let start = Instant::now() + Duration::from_secs(1);
    // Twenty yards at two a second: ten seconds.
    let w = Walk::new(
        vec![v(0.0, 0.0), v(10.0, 0.0), v(10.0, 10.0)],
        2.0,
        start,
        1,
        false,
    );
    assert_eq!(w.duration, Duration::from_secs(10));
    let at = |s: f32| w.position_at(start + Duration::from_secs_f32(s));
    // A quarter of the way: on the first leg, facing east, both corners
    // ahead.
    let (p, o) = at(2.5);
    assert!(near(p, v(5.0, 0.0)) && o.abs() < 0.001);
    assert_eq!(w.remaining(start + Duration::from_millis(2_500)).len(), 2);
    // Three quarters: the corner passed, facing north, one point left.
    let (p, o) = at(7.5);
    assert!(near(p, v(10.0, 5.0)));
    assert!((o - std::f32::consts::FRAC_PI_2).abs() < 0.001);
    let left = w.remaining(start + Duration::from_millis(7_500));
    assert_eq!(left.len(), 1);
    assert!(near(left[0], v(10.0, 10.0)));
    // Before it starts, at its start; after it ends, at its end.
    assert!(near(
        w.position_at(start - Duration::from_millis(500)).0,
        v(0.0, 0.0)
    ));
    assert!(near(at(30.0).0, v(10.0, 10.0)));
    assert!(w.done(start + Duration::from_secs(10)));
    assert!(!w.done(start + Duration::from_millis(9_999)));
}

fn creature(run: f32) -> Creature {
    let t = CreatureTemplate {
        entry: 1,
        speed_run: run / 7.0,
        speed_walk: 1.0,
        ..Default::default()
    };
    Creature::new(guid::unit(1, 1), &t, 1, v(0.0, 0.0), 0.0)
}

#[test]
fn a_run_retimed_at_a_new_speed_goes_on_from_where_it_is_with_its_corners_and_facing() {
    let start = Instant::now();
    let mut c = creature(10.0);
    let path = vec![v(0.0, 0.0), v(10.0, 0.0), v(10.0, 20.0)];
    c.walk(path, true, start, 1).expect("a run");
    if let Motion::Moving(w) = &mut c.motion {
        w.face = Some(1.5);
    }
    // At 1.5 s, at five a second.
    let now = start + Duration::from_millis(1_500);
    c.speeds.run = 5.0;
    assert!(c.retime(10.0, now, 2).is_some());
    let Motion::Moving(w) = &c.motion else {
        panic!("still running");
    };
    assert!(near(c.position, v(10.0, 5.0)));
    assert!(near(w.points[0], v(10.0, 5.0)));
    assert_eq!(w.points.len(), 2);
    assert!(near(w.destination(), v(10.0, 20.0)));
    assert_eq!(w.duration, Duration::from_secs(3));
    assert_eq!((w.spline_id, w.face), (2, Some(1.5)));
    // The same speed again is no change.
    assert!(
        c.retime(10.0, now + Duration::from_millis(100), 3)
            .is_none()
    );
    // And back to ten.
    let later = now + Duration::from_secs(1);
    c.speeds.run = 10.0;
    assert!(c.retime(5.0, later, 4).is_some());
    let Motion::Moving(w) = &c.motion else {
        panic!("still running");
    };
    assert!(near(c.position, v(10.0, 10.0)));
    assert_eq!(w.duration, Duration::from_secs(1));
}

#[test]
fn walks_timed_moves_finished_and_stopped_ones_arent_retimed() {
    let start = Instant::now();
    let path = vec![v(0.0, 0.0), v(30.0, 0.0)];
    let speed_up = |c: &mut Creature, at: Duration| {
        let was = c.speeds.run;
        c.speeds.run = 20.0;
        c.retime(was, start + at, 9).is_none()
    };
    let mut walking = creature(10.0);
    walking.walk(path.clone(), false, start, 1);
    assert!(speed_up(&mut walking, Duration::from_millis(500)));
    let mut timed = creature(10.0);
    timed.walk_at(
        path.clone(),
        true,
        false,
        Some(Duration::from_secs(2)),
        start,
        1,
    );
    assert!(speed_up(&mut timed, Duration::from_millis(500)));
    let mut finished = creature(10.0);
    finished.walk(path.clone(), true, start, 1);
    assert!(speed_up(&mut finished, Duration::from_secs(5)));
    let mut stopped = creature(10.0);
    stopped.walk(path, true, start, 1);
    stopped.motion = Motion::Idle { until: None };
    assert!(speed_up(&mut stopped, Duration::from_millis(500)));
}

#[tokio::test]
async fn a_halt_stops_a_walk_where_it_is_once_and_a_walk_ended_of_itself_needs_no_stop() {
    let (_realm, _a, g, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            let brute = spawn(m, g, BRUTE, 20, 2.0);
            let from = m.creature(brute).unwrap().position;
            let to = Vector3d {
                x: from.x + 50.0,
                ..from
            };
            let (now, id) = (m.now, m.next_spline_id);
            m.next_spline_id += 1;
            let c = m.creature_mut(brute).unwrap();
            c.walk(vec![from, to], true, now - Duration::from_secs(1), id);
            let speed = c.speeds.run;
            let ids = m.next_spline_id;
            ai::halt(m, brute);
            let c = m.creature(brute).unwrap();
            let stopped = (
                matches!(c.motion, Motion::Idle { .. }),
                (c.position.x - (from.x + speed)).abs() < 0.2,
                m.next_spline_id == ids + 1,
            );
            // Again: still, no second stop.
            let ids = m.next_spline_id;
            ai::halt(m, brute);
            let again = m.next_spline_id == ids;
            // A walk over by its time ends where it was going, no stop.
            let id = m.next_spline_id;
            m.next_spline_id += 1;
            let c = m.creature_mut(brute).unwrap();
            let here = c.position;
            let there = Vector3d {
                x: here.x + 1.0,
                ..here
            };
            c.walk(vec![here, there], true, now - Duration::from_secs(5), id);
            let ids = m.next_spline_id;
            ai::halt(m, brute);
            let c = m.creature(brute).unwrap();
            let ended = (
                matches!(c.motion, Motion::Idle { .. }),
                (c.position.x - there.x).abs() < 0.01,
                m.next_spline_id == ids,
            );
            (stopped, again, ended)
        })
        .await
        .unwrap();
    assert_eq!(r, ((true, true, true), true, (true, true, true)));
}
