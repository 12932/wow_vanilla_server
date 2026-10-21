//! Round 10's creature AI tests on the real navmesh (extended), ported
//! from Thistle Tea's `test/game/core/ai/bt/mob_test.exs`
//! (docs/parity/matrix.csv): a special path keeps its points' heights.

use std::sync::Arc;

use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::{GameData, Waypoint};
use crate::game::harness;
use crate::game::map::creature::Motion;
use crate::game::map::{Map, event_ai};
use crate::game::tests_pets::{WOLF, enter, place_in};

/// A realm on `data` with the real navmesh; none without it.
fn nav_realm(data: GameData) -> Option<Arc<Realm>> {
    let nav = harness::real_data("nav");
    if !nav.join("Azeroth.map").exists() {
        eprintln!("no navmesh at {}: skipped", nav.display());
        return None;
    }
    let mut cfg = harness::test_config();
    cfg.nav_dir = Some(nav);
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Some(Realm::new(cfg, store, startup, data, None))
}

/// Where creature `g` is bound for, if it walks.
fn bound_for(m: &Map, g: wow_world_messages::Guid) -> Option<Vector3d> {
    match &m.creature(g)?.motion {
        Motion::Moving(w) => Some(w.destination()),
        _ => None,
    }
}

/// Thistle Tea's scripted waypoint routes "special paths retain authored
/// heights without ground pathfinding" (its `pathfind?: false` for
/// `creature_movement_special`, vmangos's): a creature sent on a special
/// path (script command 60, source 3) to a point forty yards over the
/// ground goes straight there at its height; on an ordinary path, to the
/// same point, it walks the navmesh to the ground beneath.
#[tokio::test]
#[ignore = "extended"]
async fn a_special_path_keeps_its_points_heights_and_an_ordinary_one_the_ground() {
    let Some(realm) = nav_realm(crate::game::tests_pets::data()) else {
        return;
    };
    let (_c, me, map) = enter(&realm, "Paths", "Paths", Race::Human, Class::Warrior).await;
    let out = map
        .query(move |m| {
            let mut out = Vec::new();
            for (i, special) in [true, false].into_iter().enumerate() {
                let g = place_in(m, me, WOLF, 0x70_0060 + i as u32, 3.0);
                let at = m.position_of(g).unwrap();
                let (x, y) = (at.x + 10.0, at.y);
                let ground = m
                    .nav_at(&[at, Vector3d { x, y, z: at.z }])
                    .and_then(|n| n.ground(x, y, at.z))
                    .expect("the ground");
                let to = Vector3d {
                    x,
                    y,
                    z: ground + 40.0,
                };
                let now = m.now;
                event_ai::scripted_mut(m, g).unwrap().route = Some(event_ai::Route {
                    points: Arc::new(vec![Waypoint {
                        point: 1,
                        x: to.x,
                        y: to.y,
                        z: to.z,
                        ..Default::default()
                    }]),
                    repeat: false,
                    pathfind: !special,
                });
                let c = m.creature_mut(g).unwrap();
                c.movement_type = crate::game::map::creature::MOVEMENT_WAYPOINT;
                c.waypoint = 0;
                c.motion = Motion::Idle { until: Some(now) };
                let key = *m.creature_by_guid.get(&g).unwrap();
                crate::game::map::world::step_creature_key(m, key);
                out.push((special, bound_for(m, g), to, ground));
            }
            out
        })
        .await
        .unwrap();
    for (special, dest, to, ground) in out {
        let dest = dest.expect("it walks");
        if special {
            assert!(
                (dest.z - to.z).abs() < 0.1,
                "at its height: {dest:?} {to:?}"
            );
        } else {
            assert!(
                (dest.z - ground).abs() < 2.0,
                "on the ground: {dest:?} {ground}"
            );
        }
    }
}
