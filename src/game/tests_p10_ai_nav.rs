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

/// Thistle Tea's loaded random-point movement "Horde defenders and
/// Mindless Undead request the authored region without fixed facing": the
/// six steps of vmangos's scripts 945704, 945804 (Horde Defenders and Axe
/// Throwers, the Barrens) and 1103003, 1103004 (Stratholme's Mindless
/// Undead) are moves to a random point within five yards of their place
/// (script command 3, its type 3), at a run, on the navmesh, and take no
/// facing there.
#[tokio::test]
#[ignore = "extended"]
async fn the_horde_defenders_and_mindless_undead_run_to_a_point_of_their_region() {
    let db = harness::real_data("db");
    if !db.join("vmangos.sqlite").exists() {
        return;
    }
    let Some(realm) = nav_realm(harness::load_real(&db)) else {
        return;
    };
    let steps: Vec<(u32, crate::game::data::scripts::ScriptStep)> = [
        (1, 945_704u32),
        (1, 945_804),
        (0, 1_103_003),
        (0, 1_103_004),
    ]
    .into_iter()
    .flat_map(|(map, id)| {
        realm.data.scripts.ai_scripts[&id]
            .iter()
            .cloned()
            .map(move |s| (map, s))
    })
    .collect();
    assert_eq!(steps.len(), 6);
    let mut c = harness::Client::connect(&realm, "Region");
    c.enter("Region", Race::Human, Class::Warrior).await;
    let mut walked = 0;
    let mut here: Option<(u32, crate::game::map::MapHandle)> = None;
    for (map_id, step) in steps {
        assert_eq!((step.command, step.datalong[0], step.o), (3, 3, 5.0));
        let at = Vector3d {
            x: step.x,
            y: step.y,
            z: step.z,
        };
        let map = match here.take() {
            Some((id, map)) if id == map_id => {
                super::tests_p3_scripts::relocate(&map, &mut c, at).await;
                map
            }
            _ => super::tests_p3_scripts::go(&realm, &mut c, map_id, at).await,
        };
        here = Some((map_id, map.clone()));
        let out = map
            .query(move |m| {
                let g = super::tests_p3_scripts::summon(m, HORDE_DEFENDER, at, 0.0);
                let step = crate::game::data::scripts::ScriptStep {
                    condition: 0,
                    ..step
                };
                crate::game::map::scripts::execute(m, &step, g, g);
                let c = m.creature(g).unwrap();
                let walk = match &c.motion {
                    Motion::Moving(w) => Some((w.destination(), w.run, w.face)),
                    _ => None,
                };
                let face = event_ai::scripted(m, g)
                    .and_then(|s| s.scripted_move)
                    .and_then(|s| s.face);
                crate::game::map::scripts::despawn(m, g);
                (walk, face)
            })
            .await
            .unwrap();
        let (walk, face) = out;
        // Within five yards of its place, or (already there) at rest.
        if let Some((to, run, arrival_face)) = walk {
            walked += 1;
            let d = ((to.x - at.x).powi(2) + (to.y - at.y).powi(2)).sqrt();
            assert!(d <= 5.0 + 0.1, "{}: within five yards: {d}", step.id);
            assert!(run, "{}: at a run", step.id);
            assert_eq!((arrival_face, face), (None, None), "{}: no facing", step.id);
        }
    }
    assert!(walked > 0, "some walk");
}

/// A creature to send on the steps.
const HORDE_DEFENDER: u32 = 9_457;
