//! Round 7's world tests, ported from Thistle Tea's
//! `test/game/world/proximity_test.exs` and its visibility tests
//! (docs/parity/matrix.csv): who a creature notices, by its standing with
//! them and where they are as it looks.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::misc::FactionDef;
use crate::game::data::{FactionTemplate, GameData};
use crate::game::harness;
use crate::game::map::{Map, MapHandle};
use crate::game::tests_p3_combat::{START, data, level, setup_with, spawn, template};
use crate::game::tests_p3_entity::on;

/// A reputation faction humans start hated by, and one that likes them;
/// a creature of each, neutral to every side by its template.
const HATING: u32 = 960;
const LIKING: u32 = 961;
const HATER: u32 = 3_960;
const LIKER: u32 = 3_961;

pub(crate) fn standing_data() -> GameData {
    let mut d = data();
    for (id, index, base) in [(HATING, 40, -6_000), (LIKING, 41, 3_100)] {
        let f = FactionDef {
            id,
            index,
            parent: 0,
            name: format!("Faction {id}"),
            variants: [(1, 0, base, 0), (0, 0, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0)],
        };
        d.misc.faction_by_index.insert(index, id);
        d.misc.factions.insert(id, f);
        d.faction_templates.insert(
            id,
            FactionTemplate {
                id,
                faction: id,
                ours: 0x8,
                friend_mask: 0x8,
                ..Default::default()
            },
        );
    }
    for (entry, faction) in [(HATER, HATING), (LIKER, LIKING)] {
        let mut t = template(entry, 7);
        t.faction = faction;
        t.detection_range = 20.0;
        d.creature_templates.insert(entry, t);
    }
    d
}

/// Whether `creature` comes to fight `g` within `within`, the map's
/// updates running meanwhile.
async fn fights(map: &MapHandle, creature: Guid, g: Guid, within: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        let on_her = on(map, move |m| {
            m.creature(creature)
                .is_some_and(|c| c.combat.victim == g && c.threat.iter().any(|(t, _)| *t == g))
        })
        .await;
        if on_her {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// `n` of the map's updates.
async fn updates(map: &MapHandle, n: u32) {
    for _ in 0..n {
        harness::an_update(map).await;
    }
}

/// Thistle Tea's `Proximity.hear/3` "a reputation faction's creatures
/// that hate the player are asked", and `due/5` "rechecks the
/// authoritative position before noticing" and "asks the creature to
/// notice a listener it has reached": a creature whose faction hates
/// Alice (its template no one's enemy) leaves her be forty yards off and
/// comes for her once she is eight yards from it; one whose faction likes
/// her never does, beside her; another that hates her, sent running from
/// thirty yards to beside her where she stands, notices her as she comes
/// into its reach.
#[tokio::test]
async fn a_creature_whose_faction_hates_her_comes_for_her_once_she_is_near() {
    let (_realm, a, ga, map) = setup_with(standing_data(), Class::Warrior).await;
    let hater = on(&map, move |m| {
        level(m, ga, 20);
        spawn(m, ga, HATER, 20, 40.0)
    })
    .await;
    // Four of its looks, half a second apart: she is beyond its reach.
    assert!(
        !fights(&map, hater, ga, Duration::from_secs(2)).await,
        "40 yards"
    );
    let near = on(&map, move |m: &mut Map| {
        let c = m.creature(hater).unwrap();
        let (at, o) = (c.position, c.orientation);
        Vector3d {
            x: at.x + 8.0 * o.cos(),
            y: at.y + 8.0 * o.sin(),
            z: at.z,
        }
    })
    .await;
    a.heartbeat(near.x, near.y, near.z).await;
    assert!(fights(&map, hater, ga, harness::WAIT).await, "8 yards");

    a.heartbeat(START.0, START.1, START.2).await;
    let liker = on(&map, move |m| spawn(m, ga, LIKER, 20, 3.0)).await;
    updates(&map, 30).await;
    assert!(
        !fights(&map, liker, ga, Duration::ZERO).await,
        "a liking faction's"
    );

    let walker = on(&map, move |m| {
        let w = spawn(m, ga, HATER, 20, 30.0);
        let to = {
            let p = m.players[&ga].position();
            let o = m.players[&ga].movement.orientation;
            Vector3d {
                x: p.x + 3.0 * o.cos(),
                y: p.y + 3.0 * o.sin(),
                z: p.z,
            }
        };
        crate::game::map::scripts::move_to(m, w, to, true);
        w
    })
    .await;
    assert!(
        fights(&map, walker, ga, harness::WAIT).await,
        "walked into her reach"
    );
}

/// The units within `radius` of `at`, by the grid and then exactly, as
/// the map looks for those near.
fn near(m: &Map, at: Vector3d, radius: f32) -> Vec<Guid> {
    m.grid
        .around(at.x, at.y, radius)
        .filter(|g| {
            m.position_of(*g)
                .is_some_and(|p| crate::game::map::combat::dist(p, at) <= radius)
        })
        .collect()
}

/// Thistle Tea's `World.position/2` "returns the destination once the
/// published movement has expired", `nearby_units_exact/5` "includes
/// moving units by their interpolated position", "includes moving units
/// whose stale position drifted within a spatial cell" and "excludes
/// units whose interpolated position left the radius", and `moving?/2`
/// "true while a published spline is still running, false once it
/// expires" and "false for a unit that has neither a spline nor a recent
/// position update", `distance_between/3` "uses projected positions for
/// both guid operands", and `Position.snapshot/2` "captures spline motion
/// through arrival and removal" and `put/2` "reconciles a mob's
/// stationary and projected positions": a brute run 120 yards east is, a
/// second on, seven yards along, found there and not where it set out,
/// ten yards from another run beside it; done, it is at its end, two cells
/// on, found there; it moves while it runs, not after nor before. The
/// other, stopped halfway, stays where it stopped. The map's clock is
/// moved on by hand, in one turn of the map.
#[tokio::test]
async fn a_running_creature_is_where_its_run_has_it_and_is_found_there() {
    use crate::game::map::creature::Motion;
    use crate::game::tests_p3_combat::BRUTE;
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let out = on(&map, move |m| {
        let brute = spawn(m, ga, BRUTE, 20, 30.0);
        let moving = |m: &Map| {
            matches!(
                m.creature(brute).map(|c| &c.motion),
                Some(Motion::Moving(_))
            )
        };
        let from = m.creature(brute).unwrap().position;
        let to = Vector3d {
            x: from.x + 120.0,
            ..from
        };
        let before = moving(m);
        // Another ten yards north of it, run along beside it, stopped
        // halfway.
        let beside = spawn(m, ga, BRUTE, 20, 30.0);
        let alongside = Vector3d {
            y: from.y + 10.0,
            ..from
        };
        crate::game::map::scripts::place(m, beside, alongside, 0.0);
        crate::game::map::scripts::move_to(m, brute, to, true);
        crate::game::map::scripts::move_to(
            m,
            beside,
            Vector3d {
                y: to.y + 10.0,
                ..to
            },
            true,
        );
        let speed = crate::game::map::combat::dist(m.creature(brute).unwrap().position, from);
        m.now += Duration::from_secs(1);
        crate::game::map::ai::advance_motion(m, brute);
        crate::game::map::ai::halt(m, beside);
        let at = m.position_of(brute).unwrap();
        let along = at.x - from.x;
        let held = m.position_of(beside).unwrap();
        let mid = (
            moving(m),
            near(m, at, 1.0).contains(&brute),
            near(m, from, 5.0).contains(&brute),
            (crate::game::map::combat::dist(at, held) - 10.0).abs() < 0.01,
        );
        let stopped = m
            .creature(beside)
            .map(|c| matches!(c.motion, Motion::Idle { .. }));
        m.now += Duration::from_secs(99);
        crate::game::map::ai::advance_motion(m, brute);
        let end = m.position_of(brute).unwrap();
        crate::game::map::ai::advance_motion(m, beside);
        let kept = m.position_of(beside) == Some(held) && stopped == Some(true);
        let done = (
            crate::game::map::combat::dist(end, to) < 0.01,
            moving(m),
            near(m, to, 10.0).contains(&brute),
            near(m, from, 10.0).contains(&brute),
        );
        (before, speed, along, mid, done, kept)
    })
    .await;
    let (before, speed, along, mid, done, kept) = out;
    assert!(!before, "standing, not moving");
    assert!(speed < 0.01, "set out from where it stood");
    assert!((6.0..8.0).contains(&along), "a run's second: {along}");
    assert_eq!(
        mid,
        (true, true, false, true),
        "moving; found where it is, not where it was; ten yards from the other where each is"
    );
    assert!(kept, "the one stopped halfway stays where it stopped");
    assert_eq!(
        done,
        (true, false, true, false),
        "at its end, still, found there"
    );
}

/// Thistle Tea's `SpatialHash.distance/2` "calculates euclidean
/// distance", "handles negative coordinates" and "calculates 3d
/// distance": the map's distance (`combat::dist`) is the straight line.
#[test]
fn a_distance_is_the_straight_line_between_two_points() {
    use crate::game::map::combat::dist;
    let v = |x: f32, y: f32, z: f32| Vector3d { x, y, z };
    assert_eq!(dist(v(0.0, 0.0, 0.0), v(3.0, 4.0, 0.0)), 5.0);
    assert_eq!(dist(v(-1.0, -1.0, 0.0), v(2.0, 3.0, 0.0)), 5.0);
    assert_eq!(dist(v(0.0, 0.0, 0.0), v(1.0, 2.0, 2.0)), 3.0);
}

/// Thistle Tea's `SpatialHash.query/6` "uses 2d cells and keeps 3d
/// distance filtering": Alice is near where she stands; a thousand yards
/// above it, in the same cell of the map's grid, she isn't.
#[tokio::test]
async fn those_near_are_found_by_their_cell_and_kept_by_the_straight_line() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let (here, above) = on(&map, move |m| {
        let at = m.players[&ga].position();
        let up = Vector3d {
            z: at.z + 1_000.0,
            ..at
        };
        (
            m.players_near(at, 10.0).contains(&ga),
            m.players_near(up, 10.0).contains(&ga),
        )
    })
    .await;
    assert_eq!((here, above), (true, false));
}
