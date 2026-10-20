//! `test/game/core/movement/safe_position_test.exs`, round 8: the last safe
//! place, on the real navmesh, is ground the player stood on.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::harness::{self, Client};
use crate::game::map::{MapHandle, MapKey};
use crate::game::tests_p3_player2_ammo::handled;
use crate::game::tests_parity_motion::START;

fn nav_realm() -> Option<Arc<Realm>> {
    let nav = harness::real_data("nav");
    if !nav.join("Azeroth.map").exists() {
        eprintln!("no navmesh at {}: skipped", nav.display());
        return None;
    }
    let mut cfg = harness::test_config();
    cfg.nav_dir = Some(nav);
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Some(Realm::new(
        cfg,
        store,
        startup,
        crate::game::data::GameData::empty(),
        None,
    ))
}

async fn step(a: &mut Client, at: Vector3d) {
    a.send(MSG_MOVE_HEARTBEAT_Client {
        info: MovementInfo {
            flags: MovementInfo_MovementFlags::empty(),
            timestamp: 0,
            position: at,
            orientation: 0.0,
            fall_time: 0,
        },
    })
    .await;
    handled(a).await;
    // Not faster than a run (the speed check).
    tokio::time::sleep(Duration::from_millis(100)).await;
}

async fn safe(map: &MapHandle, g: Guid) -> Option<Vector3d> {
    map.query(move |m| m.players[&g].safe_position.map(|(_, at, _)| at))
        .await
        .unwrap()
}

/// The navmesh's ground `dx` yards east of the start, `dz` above it.
async fn ground(map: &MapHandle, dx: f32, dz: f32) -> Vector3d {
    map.query(move |m| {
        let (x, y) = (START.0 + dx, START.1);
        let z = m
            .nav_at(&[Vector3d { x, y, z: START.2 }])
            .and_then(|n| n.ground(x, y, START.2))
            .expect("the navmesh's ground");
        Vector3d { x, y, z: z + dz }
    })
    .await
    .unwrap()
}

/// A place the player moved to (three yards or more from the last) is
/// kept as safe only if the navmesh's ground is there, within three yards
/// of their height: not one twenty yards under it, though no flag says
/// they fall; then the ground there is (Thistle Tea's "samples moved
/// ground positions only when they match terrain height").
#[tokio::test]
#[ignore = "extended"]
async fn a_safe_place_is_one_at_the_grounds_height() {
    let Some(realm) = nav_realm() else {
        return;
    };
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let first = ground(&map, 4.0, 0.0).await;
    step(&mut a, first).await;
    assert_eq!(safe(&map, ga).await, Some(first), "ground");
    let near = ground(&map, 5.0, 0.0).await;
    step(&mut a, near).await;
    assert_eq!(
        safe(&map, ga).await,
        Some(first),
        "a step from it isn't sampled"
    );
    let under = ground(&map, 9.0, -20.0).await;
    step(&mut a, under).await;
    let at = map.query(move |m| m.players[&ga].position()).await.unwrap();
    assert_eq!(at, under, "she is there");
    assert_eq!(safe(&map, ga).await, Some(first), "under the ground");
    let there = ground(&map, 9.0, 0.5).await;
    step(&mut a, there).await;
    assert_eq!(safe(&map, ga).await, Some(there), "on it again");
}
