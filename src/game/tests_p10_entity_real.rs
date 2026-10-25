//! Round 10's entity tests on the real navmesh (extended), ported from
//! Thistle Tea's `effect_resolver/leap_test.exs` (docs/parity/matrix.csv):
//! a leap while falling and one under water.

use wow_world_messages::vanilla::*;

use crate::game::harness;

/// Goblin Jumper Cables and their XL, on the real data, and each's
/// failure: its damage to the user, and its stun.
const CABLES: [(u32, u32, u32); 2] = [(8342, 8338, 50), (22_999, 23_055, 100)];

/// A Goblin Jumper Cables that fails hurts and stuns its user alone, by
/// its failure spell (50 and 8338, 100 and 23055 for the XL), and offers
/// the dead no resurrection; one that works offers it (Thistle Tea's
/// "cable failures damage and stun only the caster without creating an
/// offer"; vmangos's `spell_goblin_jumper_cables`).
#[tokio::test]
#[ignore = "extended"]
async fn jumper_cables_that_fail_hurt_and_stun_their_user_and_offer_nothing() {
    use crate::game::fields::*;
    let dir = harness::real_data("db");
    if !dir.join("vmangos.sqlite").exists() {
        return;
    }
    let realm = harness::realm_with(harness::load_real(&dir));
    let mut a = harness::Client::connect(&realm, "A");
    let ga = a.enter("Tinker", Race::Human, Class::Warrior).await;
    let mut b = harness::Client::connect(&realm, "B");
    let gb = b.enter("Fallen", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(ga).unwrap().map();
    let out = map
        .query(move |m| {
            let data = m.realm.data.clone();
            for g in [ga, gb] {
                let p = m.players.get_mut(&g).unwrap();
                crate::game::map::stats::set_level(p, &data, 60);
            }
            crate::game::map::combat::kill(m, ga, gb);
            let mut seen = Vec::new();
            for (cables, failure, damage) in CABLES {
                let (mut failed, mut offered) = (None, false);
                for _ in 0..200 {
                    let max = m.players[&ga].fields.get_u32(UNIT_FIELD_MAXHEALTH);
                    m.players
                        .get_mut(&ga)
                        .unwrap()
                        .fields
                        .set_u32(UNIT_FIELD_HEALTH, max);
                    m.players.get_mut(&gb).unwrap().resurrect_offer = None;
                    crate::game::map::spells::cast_triggered(m, ga, cables, Some(gb));
                    let lost = max - m.players[&ga].fields.get_u32(UNIT_FIELD_HEALTH);
                    let stunned =
                        crate::game::map::auras::auras(m, ga).is_some_and(|x| x.has_spell(failure));
                    let offer = m.players[&gb].resurrect_offer.is_some();
                    if stunned {
                        failed = Some((lost, offer));
                    } else if offer {
                        offered = true;
                    }
                    crate::game::map::auras::remove_spell(m, ga, failure, None);
                    if failed.is_some() && offered {
                        break;
                    }
                }
                seen.push((cables, damage, failed, offered));
            }
            seen
        })
        .await
        .unwrap();
    for (cables, damage, failed, offered) in out {
        assert_eq!(
            failed,
            Some((damage, false)),
            "{cables}: hurt and stunned, no offer"
        );
        assert!(offered, "{cables}: one that works offers it");
    }
}

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
            let leap = crate::game::map::spells::leap::leap_end;
            let (west, east) = (std::f32::consts::PI, 0.0);
            let fell = leap(m, g, v(-8949.95, -132.49, 113.53), west, 20.0);
            set(m, MovementInfo_MovementFlags::empty());
            let walked = leap(m, g, v(-8949.95, -132.49, 83.53), west, 20.0);
            set(
                m,
                MovementInfo_MovementFlags::empty().set_swimming(Default::default()),
            );
            let swam = leap(m, g, v(-2180.0, -1867.59, -5.0), east, 20.0);
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
