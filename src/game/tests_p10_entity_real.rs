//! Round 10's entity tests on the real navmesh (extended), ported from
//! Thistle Tea's `effect_resolver/leap_test.exs` (docs/parity/matrix.csv):
//! a leap while falling and one under water.

use wow_world_messages::vanilla::*;

use crate::game::harness;

/// A realm whose maps have the real navmesh, and a player in Northshire.
async fn northshire() -> Option<(
    harness::Client,
    wow_world_messages::Guid,
    crate::game::map::MapHandle,
)> {
    let bake = harness::real_data("nav");
    if !bake.join("Azeroth.map").exists() {
        return None;
    }
    let mut cfg = harness::test_config();
    cfg.nav_dir = Some(bake);
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = crate::game::Realm::new(cfg, store, startup, crate::game::tests_pets::data(), None);
    let mut c = harness::Client::connect(&realm, "A");
    let g = c.enter("Mage", Race::Human, Class::Mage).await;
    let map = realm.registry.online(g).expect("online").map();
    Some((c, g, map))
}

/// A leap (a Blink) by one falling, thirty yards over Northshire's ground,
/// falls on two yards lower, not to the ground, which is beyond ten yards
/// (mangos's `TARGET_LOCATION_CASTER_FRONT_LEAP`, its falling case: the
/// ground taken within ten yards). Thistle Tea grounds it within forty
/// (its "grounds falling leaps within forty yards": 83.3), so a Blink
/// while falling lands its mage and spares the fall; wvs did as Thistle
/// Tea. One standing on the ground leaps along it. One swimming keeps
/// its depth (Thistle Tea's "retains submerged depth instead of snapping
/// to the liquid mesh"; mangos's "under water so next z equal prev z"),
/// where wvs put it on the bed.
#[tokio::test]
#[ignore = "extended"]
async fn a_leap_while_falling_falls_on_and_one_under_water_keeps_its_depth() {
    let Some((_c, g, map)) = northshire().await else {
        return;
    };
    let out = map
        .query(move |m| {
            let v = |x: f32, y: f32, z: f32| Vector3d { x, y, z };
            let set = |m: &mut crate::game::map::Map, flags: MovementInfo_MovementFlags| {
                m.players.get_mut(&g).unwrap().movement.flags = flags;
            };
            set(m, MovementInfo_MovementFlags::empty().set_fallingfar());
            let fell = crate::game::map::spells::leap_end(
                m,
                g,
                v(-8949.95, -132.49, 113.53),
                v(-8969.95, -132.49, 113.53),
            );
            set(m, MovementInfo_MovementFlags::empty());
            let walked = crate::game::map::spells::leap_end(
                m,
                g,
                v(-8949.95, -132.49, 83.53),
                v(-8969.95, -132.49, 83.53),
            );
            set(
                m,
                MovementInfo_MovementFlags::empty().set_swimming(Default::default()),
            );
            let swam = crate::game::map::spells::leap_end(
                m,
                g,
                v(-2180.0, -1867.59, -5.0),
                v(-2160.0, -1867.59, -5.0),
            );
            (m.nav.is_some(), fell, walked, swam)
        })
        .await
        .unwrap();
    let (navigated, fell, walked, swam) = out;
    assert!(navigated, "the map has its navmesh");
    assert!(
        (fell.z - 111.53).abs() < 0.1 && (fell.x + 8969.95).abs() < 0.5,
        "falls on two yards lower: {fell:?}"
    );
    assert!(
        (walked.z - 83.3).abs() < 2.0 && (walked.x + 8969.95).abs() < 1.0,
        "along the ground: {walked:?}"
    );
    assert!(
        (swam.z + 5.0).abs() < 0.01 && (swam.x + 2160.0).abs() < 0.01,
        "at its depth: {swam:?}"
    );
}
