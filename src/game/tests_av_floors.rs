//! On their floors in Drek'Thar's keep, on the real valley and its navmesh
//! (decision 106): a pet at its master's side by the keep's walls keeps to
//! its master's floor and their sight, never down through the floor to the
//! terrain under it, nor into a wall (in the corridor south of his room, out
//! of his guards' sight); his guards walk his room's floor, its sunken
//! middle and the ring a yard over it, on the floor; and a charge across
//! the ring's step ends on the floor.

use std::time::Duration;

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapHandle, MapKey, grid};

/// Alterac Valley.
const AV: u32 = 30;
/// The corridor south of Drek'Thar's room, in Frostwolf Keep, out of his
/// guards' sight: its floor about 99, the terrain under the keep at 91.2.
const CORRIDOR: Vector3d = Vector3d {
    x: -1358.0,
    y: -264.0,
    z: 99.2,
};
/// A pet's place about its master: an angle off their facing (`PetState`).
const PET_ANGLE: f32 = std::f32::consts::FRAC_PI_2 * 1.5;

/// Wanda, a warlock, in the valley's world copy in the corridor south of
/// Drek'Thar's room, in a realm of the real data and navmesh.
async fn by_the_door() -> (Client, Guid, MapHandle) {
    let mut cfg = harness::test_config();
    cfg.nav_dir = Some(harness::real_data("nav"));
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let data = harness::load_real(&harness::real_data("db"));
    let realm = crate::game::Realm::new(cfg, store, startup, data, None);
    let mut c = Client::connect(&realm, "W");
    let g = c.enter("Wanda", Race::Human, Class::Warlock).await;
    let from = realm.registry.online(g).unwrap().map();
    from.call(move |m| m.teleport(g, MapKey::world(AV), CORRIDOR, 0.0));
    c.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    c.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    c.sees(g).await;
    let map = realm.registry.online(g).unwrap().map();
    for _ in 0..5 {
        harness::an_update(&map).await;
    }
    (c, g, map)
}

/// Player `g` at `at`, facing `o`, as the map has it, made hardy.
fn put(m: &mut Map, g: Guid, at: Vector3d, o: f32) {
    let p = m.players.get_mut(&g).unwrap();
    p.movement.position = at;
    p.movement.orientation = o;
    p.move_check = None;
    p.fields
        .set_u32(crate::game::fields::UNIT_FIELD_MAXHEALTH, 1_000_000);
    p.fields
        .set_u32(crate::game::fields::UNIT_FIELD_HEALTH, 1_000_000);
    let (old, new) = (p.cell, grid::cell_of(at.x, at.y));
    p.cell = new;
    m.grid.relocate(g, old, new);
}

/// Spots on the corridor's floor, each with a facing, where a pet's place
/// taken as it was (two yards out at its angle, at its master's height,
/// onto the ground nearest that) is off her floor: under it, or on a wall.
fn bad_spots(m: &mut Map) -> Vec<(Vector3d, f32, f32)> {
    let mut out = Vec::new();
    let nav = m.nav.as_mut().expect("the navmesh");
    for i in 0..15 {
        for j in 0..15 {
            let (x, y) = (CORRIDOR.x - 7.0 + i as f32, CORRIDOR.y - 7.0 + j as f32);
            let Some(z) = nav.ground(x, y, CORRIDOR.z) else {
                continue;
            };
            if (z - CORRIDOR.z).abs() > 1.5 {
                continue;
            }
            for k in 0..16 {
                let o = k as f32 * std::f32::consts::TAU / 16.0;
                let a = o + PET_ANGLE;
                let (sx, sy) = (x + 2.0 * a.cos(), y + 2.0 * a.sin());
                if let Some(sz) = nav.ground(sx, sy, z)
                    && (sz - z).abs() > 3.0
                {
                    out.push((Vector3d { x, y, z }, o, sz));
                }
            }
        }
    }
    out
}

/// The map's creatures and pets take a turn of 50 ms.
fn turn(m: &mut Map) {
    m.now += Duration::from_millis(50);
    crate::game::map::ai::update_creatures(m);
    crate::game::map::combat::update_creatures(m);
    crate::game::map::pets::update(m);
}

/// In the corridor south of Drek'Thar's room, where a pet's place beside
/// its master taken as it was is under the floor (the terrain beneath the
/// keep, 8 yd down) or up on a wall, her imp goes only to places on her
/// floor: every point of its every walk within a step of her height, each
/// leg of it in sight, and it with her. Before, it walked there: down
/// through the floor and back up, 29 times in 5 attempts of the all-in at
/// Drek'Thar (the watcher's `peon::seen`, the fight record), and straight
/// through the keep's walls where the navmesh had no way.
#[tokio::test]
#[ignore = "extended"]
async fn a_pet_in_drek_thars_keep_keeps_to_its_masters_floor() {
    let (_c, g, map) = by_the_door().await;
    let (spots, walks, faults) = map
        .query(move |m| {
            let spots = bad_spots(m);
            let first = *spots.first().expect("a spot");
            put(m, g, first.0, first.1);
            crate::game::map::pets::testing::summon_demon(
                m,
                g,
                crate::game::tests_pets::IMP,
                crate::game::tests_pets::SUMMON_IMP,
            );
            let imp = crate::game::map::pets::pet_of(m, g).expect("her imp");
            let (mut walks, mut faults) = (0, Vec::new());
            let mut last_spline = None;
            for (at, o, old) in spots.iter().step_by((spots.len() / 8).max(1)).take(8).copied() {
                put(m, g, at, o);
                for _ in 0..40 {
                    turn(m);
                    let Some(c) = m.creature(imp) else {
                        faults.push(format!("her imp gone at {at:?}"));
                        break;
                    };
                    if let crate::game::map::creature::Motion::Moving(w) = &c.motion
                        && last_spline != Some(w.spline_id)
                    {
                        last_spline = Some(w.spline_id);
                        walks += 1;
                        let points = w.points.clone();
                        for p in &points {
                            if (p.z - at.z).abs() > 2.5 {
                                faults.push(format!(
                                    "her at {at:?} facing {o:.2} (the old place's ground {old:.1}): \
                                     a point {p:?} of {points:?}"
                                ));
                            }
                        }
                        for leg in points.windows(2) {
                            if !crate::game::map::spells::checks::line_of_sight(m, leg[0], leg[1]) {
                                faults.push(format!("her at {at:?}: a leg {leg:?} through a wall"));
                            }
                        }
                    }
                    let z = m.position_of(imp).map_or(at.z, |p| p.z);
                    if (z - at.z).abs() > 2.5 {
                        faults.push(format!("her at {at:?}: the imp at {z:.1}"));
                    }
                }
            }
            (spots.len(), walks, faults)
        })
        .await
        .unwrap();
    assert!(
        spots >= 3,
        "{spots} spots in the corridor where the old place is off the floor: the case is gone"
    );
    assert!(walks >= 3, "the imp walked {walks} times");
    assert!(faults.is_empty(), "{faults:#?}");
}

/// Drek'Thar's room: its sunken middle (98.43) and the ring about it a yard
/// higher (99.37), the step between.
const MIDDLE: Vector3d = Vector3d {
    x: -1366.0,
    y: -224.0,
    z: 98.43,
};
const RING: Vector3d = Vector3d {
    x: -1381.0,
    y: -221.0,
    z: 99.37,
};

/// How far each point of `points` is off the floor under it, at most:
/// the floor its navmesh has there, nearest its height.
fn off_floor(m: &mut Map, points: &[Vector3d]) -> f32 {
    let nav = m.nav.as_mut().expect("the navmesh");
    points
        .iter()
        .filter_map(|p| nav.ground(p.x, p.y, p.z).map(|z| (p.z - z).abs()))
        .fold(0.0, f32::max)
}

/// Wanda in Drek'Thar's room, a god (his guards can't kill her): where
/// she is put.
async fn in_his_room(at: Vector3d) -> (Client, Guid, MapHandle) {
    let (c, g, map) = by_the_door().await;
    map.query(move |m| {
        put(m, g, at, 0.0);
        m.players.get_mut(&g).unwrap().misc.god = true;
    })
    .await;
    (c, g, map)
}

/// His guards and he, set on Wanda in his room's sunken middle by the
/// steps up to the ring a yard over it, come at her, each walk on the floor
/// under it: its points a fifth of a yard off it at most, the legs between
/// them (as a client runs them, straight) no more than a step's half yard
/// (one up a step rises across it). Before, a point by the
/// steps took the navmesh polygon nearest it, the ring's (a yard up, and
/// nearer than the middle's below), and a leg was left half a yard off: in
/// from the ring they walked the middle at the ring's height, up to a yard in
/// the air (the fight record: 134 of 505 points over a third of a yard).
#[tokio::test]
#[ignore = "extended"]
async fn drek_thars_guards_walk_his_room_on_its_floor() {
    let (_c, g, map) = in_his_room(MIDDLE).await;
    let (walks, worst_point, worst_leg) = map
        .query(move |m| {
            let guards: Vec<Guid> = m
                .creatures
                .iter()
                .filter(|(_, c)| crate::game::map::combat::dist(c.position, MIDDLE) <= 30.0)
                .map(|(_, c)| c.guid)
                .collect();
            let mut seen = ahash::AHashSet::new();
            let mut walks = 0;
            let (mut worst_point, mut worst_leg) = ((0.0f32, Vec::new()), (0.0f32, Vec::new()));
            // By the steps, on each side of the middle.
            for spot in [
                Vector3d { x: -1361.5, y: -221.5, z: 98.43 },
                Vector3d { x: -1375.5, y: -221.5, z: 98.43 },
                Vector3d { x: -1368.0, y: -213.5, z: 98.43 },
                Vector3d { x: -1368.0, y: -229.5, z: 98.43 },
            ] {
                let z = m.nav.as_mut().unwrap().ground(spot.x, spot.y, spot.z).unwrap();
                let spot = Vector3d { z, ..spot };
                for c in &guards {
                    crate::game::map::ai::attacked(m, *c, g);
                }
                for _ in 0..80 {
                    put(m, g, spot, 0.0);
                    turn(m);
                    let moving: Vec<(u32, Vec<Vector3d>)> = m
                        .creatures
                        .iter()
                        .filter(|(_, c)| crate::game::map::combat::dist(c.position, MIDDLE) <= 40.0)
                        .filter_map(|(_, c)| match &c.motion {
                            crate::game::map::creature::Motion::Moving(w) => {
                                Some((w.spline_id, w.points.clone()))
                            }
                            _ => None,
                        })
                        .collect();
                    for (id, points) in moving {
                        if !seen.insert(id) || points.len() < 2 {
                            continue;
                        }
                        walks += 1;
                        let off = off_floor(m, &points[1..]);
                        if off > worst_point.0 {
                            worst_point = (off, points.clone());
                        }
                        // The legs, a half yard at a time.
                        let mut along = Vec::new();
                        for w in points.windows(2) {
                            let n = (crate::game::map::combat::dist(w[0], w[1]) / 0.5).ceil().max(1.0) as usize;
                            for i in 1..n {
                                let f = i as f32 / n as f32;
                                along.push(Vector3d {
                                    x: w[0].x + (w[1].x - w[0].x) * f,
                                    y: w[0].y + (w[1].y - w[0].y) * f,
                                    z: w[0].z + (w[1].z - w[0].z) * f,
                                });
                            }
                        }
                        let leg = off_floor(m, &along);
                        if leg > worst_leg.0 {
                            worst_leg = (leg, points);
                        }
                    }
                }
            }
            (walks, worst_point, worst_leg)
        })
        .await
        .unwrap();
    assert!(walks >= 8, "{walks} walks");
    assert!(
        worst_point.0 <= 0.2,
        "a point {:.2} yd off the floor: {:?}",
        worst_point.0,
        worst_point.1
    );
    assert!(
        worst_leg.0 <= 0.5,
        "a leg {:.2} yd off the floor: {:?}",
        worst_leg.0,
        worst_leg.1
    );
}

/// A charge from the ring at one in the sunken middle a yard below ends on
/// the middle's floor. Before, its last leg (cut short of its target) had
/// the height between its ends, the ring's and its target's, and the
/// charger stood there a third of a yard in the air.
#[tokio::test]
#[ignore = "extended"]
async fn a_charge_down_into_drek_thars_room_ends_on_its_floor() {
    let (_c, g, map) = in_his_room(RING).await;
    let end = map
        .query(move |m| {
            let drek = m
                .creatures
                .iter()
                .find(|(_, c)| c.entry == 11946)
                .map(|(_, c)| c.guid)
                .expect("Drek'Thar");
            if let Some(c) = m.creature_mut(drek) {
                c.position = MIDDLE;
            }
            crate::game::map::charge::start(m, g, drek, false);
            let walk = m.players[&g]
                .charge
                .as_ref()
                .and_then(|c| c.walk.as_ref())
                .expect("a charge");
            *walk.points.last().unwrap()
        })
        .await
        .unwrap();
    let off = map.query(move |m| off_floor(m, &[end])).await.unwrap();
    assert!(off <= 0.05, "the charge ends {off:.2} yd off the floor at {end:?}");
}
