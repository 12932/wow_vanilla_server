//! Round 7's world tests on the real navmesh, ported from Thistle Tea's
//! `test/game/world/pathfinding_test.exs` (docs/parity/matrix.csv): its
//! points, asked of wvs's navmesh (`map::nav`) as the maps ask it. Each
//! test passes over where the bake isn't there.

use wow_world_messages::vanilla::{Class, Vector3d};

use crate::game::harness;
use crate::game::map::nav::Nav;

static BAKE: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| harness::real_data("nav"));

/// A human's start in Northshire, in front of the abbey.
const START: Vector3d = Vector3d {
    x: -8949.95,
    y: -132.49,
    z: 83.53,
};

fn v(x: f32, y: f32, z: f32) -> Vector3d {
    Vector3d { x, y, z }
}

fn nav(name: &str) -> Option<Nav> {
    Nav::open(BAKE.as_path(), name)
}

fn distance(a: Vector3d, b: Vector3d) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2) + (a.z - b.z).powi(2)).sqrt()
}

/// Thistle Tea's `line_of_sight?/3`: "open air is visible", "Northshire
/// Abbey blocks the ray", "a ray through the abbey to the far side is
/// blocked" and "an unloaded map fails open". A map with no navmesh hides
/// nothing (`checks::line_of_sight`).
#[tokio::test]
#[ignore = "extended"]
async fn the_abbey_blocks_sight_and_open_air_does_not() {
    let Some(mut n) = nav("Azeroth") else {
        return;
    };
    assert!(n.line_of_sight(START, v(-8955.0, -140.0, 84.0)), "open air");
    assert!(
        !n.line_of_sight(START, v(-8914.0, -164.0, 82.0)),
        "the abbey"
    );
    assert!(
        !n.line_of_sight(START, v(-8880.0, -180.0, 82.0)),
        "through the abbey to its far side"
    );
    // A map without a navmesh: its sight is open.
    let realm = harness::realm();
    let map = realm.maps.get(&realm, crate::game::map::MapKey::world(0));
    let open = map
        .query(|m| {
            m.nav.is_none()
                && crate::game::map::spells::checks::line_of_sight(
                    m,
                    v(0.0, 0.0, 0.0),
                    v(1.0, 1.0, 1.0),
                )
        })
        .await
        .unwrap();
    assert!(open);
}

/// Thistle Tea's `outdoors/2`: "distinguishes Northshire's abbey interior
/// from its exterior", "keeps outdoor Stormwind WMO groups distinct from
/// building interiors", "finds cave floors beneath terrain and inside
/// global WMO maps" (the Fargodeep mine, the Deadmines) and "returns
/// unknown for unavailable maps" (wvs opens no navmesh for a map not in
/// the bake, so a map's `outdoors` is unknown).
#[tokio::test]
#[ignore = "extended"]
async fn indoors_is_under_a_roof_and_outdoors_under_the_sky() {
    let Some(mut n) = nav("Azeroth") else {
        return;
    };
    for (p, out, what) in [
        (START, true, "Northshire"),
        (v(-8927.62, -201.787, 80.6824), true, "beside the abbey"),
        (v(-8914.0, -164.0, 82.0), false, "in the abbey"),
        (
            v(-9068.827, 413.834, 92.9318),
            true,
            "Stormwind's gate, a WMO's outdoor group",
        ),
        (v(-9086.794, 443.588, 92.9407), true, "inside the gate"),
        (v(-9461.5, 16.19, 56.96), false, "Goldshire's inn"),
        (
            v(-8664.242, -123.1947, 91.81388),
            false,
            "a cave under the terrain",
        ),
    ] {
        assert_eq!(n.outdoors(p), Some(out), "{what}");
    }
    let mut deadmines = nav("DeadminesInstance").expect("the Deadmines' bake");
    assert_eq!(
        deadmines.outdoors(v(-131.29083, -591.2431, 18.07719)),
        Some(false),
        "the Deadmines, a global WMO"
    );
    assert!(nav("NoSuchMap").is_none(), "no navmesh to ask");
}

/// Thistle Tea's `get_zone_and_area/2`: "resolves Alterac Valley
/// graveyards", "retains the tower area on an unlabelled roof and in the
/// air above it", "resolves Goldshire across the inn's unlabelled floor
/// surface" and, of "rejects unsigned unknown-area sentinels on
/// Programmer Isle", its labelled point: where a point says nothing, the
/// nearest floor's.
#[tokio::test]
#[ignore = "extended"]
async fn zones_and_areas_come_from_the_nearest_labelled_floor() {
    let Some(mut n) = nav("Azeroth") else {
        return;
    };
    for z in [192.51268, 194.6, 220.0] {
        assert_eq!(
            n.zone_and_area(v(1854.0, -3724.0, z)),
            Some((139, 2263)),
            "the tower in the Plaguelands at {z}"
        );
    }
    for z in [56.96, 56.962_56, 57.05, 59.0] {
        assert_eq!(
            n.zone_and_area(v(-9461.5, 16.19, z)),
            Some((12, 87)),
            "Goldshire's inn at {z}"
        );
    }
    let mut av = nav("PVPZone01").expect("Alterac Valley's bake");
    for p in [
        v(73.417_755, -496.433, 48.731_92),
        v(-531.2178, -405.2314, 49.5514),
    ] {
        assert_eq!(av.zone_and_area(p).map(|(z, _)| z), Some(2597), "{p:?}");
    }
    // Thistle Tea's namigator has no area at (16 335.2, 16 298.1), where
    // rustigator's bake has the isle's own: the labelled point only.
    let mut isle = nav("development").expect("Programmer Isle's bake");
    assert_eq!(
        isle.zone_and_area(v(16_303.2, 16_318.1, 69.44)),
        Some((22, 22))
    );
}

/// Thistle Tea's `find_path/4`: "routes from overlapping Deadmines
/// surfaces", "keeps chasing after crossing a Deadmines surface overlap",
/// "preserves routes where the closest overlapping surface is
/// disconnected", "preserves routes through RFC and the Elwynn mines",
/// "routes the Deadmines alarm pirates to the breached door", "routes the
/// gunpowder ambush into the corridor" and "finds a path on
/// player-walkable ground": each path ends within a yard and a half of
/// where it was asked to go.
#[tokio::test]
#[ignore = "extended"]
async fn paths_through_the_deadmines_ragefire_and_elwynns_mines_arrive() {
    let (Some(mut azeroth), Some(mut deadmines), Some(mut rfc)) = (
        nav("Azeroth"),
        nav("DeadminesInstance"),
        nav("OrgrimmarInstance"),
    ) else {
        return;
    };
    let arrives = |n: &mut Nav, from: Vector3d, to: Vector3d, within: f32| {
        let path = n
            .path(from, to)
            .unwrap_or_else(|| panic!("a path {from:?} to {to:?}"));
        let end = *path.last().unwrap();
        assert!(
            distance(end, to) < within,
            "{from:?} to {to:?} ends at {end:?}"
        );
        path
    };
    let overlap = v(-192.036_9, -592.948_2, 38.747_13);
    let below = v(-123.865_13, -607.068_9, 16.866_046);
    for (from, to) in [
        (v(-192.036_9, -592.948_2, 39.148_48), below),
        (
            v(-191.786_9, -592.680_2, 39.258_59),
            v(-126.667_65, -594.773_3, 19.114_894),
        ),
        (
            v(-191.963_13, -592.856, 39.210_44),
            v(-127.230_82, -594.357_9, 19.099_384),
        ),
        (overlap, below),
        // The nearest surface to this one isn't joined to the rest.
        (
            v(-120.521_15, -405.568_7, 59.121_227),
            v(-127.230_82, -594.357_9, 19.099_384),
        ),
        // The gunpowder ambush, into the corridor.
        (
            v(-131.290_83, -591.243_1, 18.077_19),
            v(-115.263_67, -617.396_1, 13.579_387),
        ),
    ] {
        arrives(&mut deadmines, from, to, 1.5);
    }
    // The alarm's pirates, to the breached door.
    let door = v(-99.6611, -671.071_66, 7.42241);
    for from in [
        v(-102.521, -697.942, 8.84454),
        v(-89.7001, -691.332, 8.24514),
    ] {
        arrives(&mut deadmines, from, door, 1.0);
    }
    // A chase across the overlap: a step at a time toward the path's next
    // point, a new path each step, until it arrives.
    let mut at = overlap;
    for _ in 0..600 {
        if distance(at, below) < 1.5 {
            break;
        }
        let path = deadmines.path(at, below).expect("a path on");
        let next = *path.get(1).expect("a next point");
        let d = distance(at, next);
        assert!(d > 0.0001, "the chase moves on from {at:?}");
        let r = (0.3 / d).min(1.0);
        at = v(
            at.x + (next.x - at.x) * r,
            at.y + (next.y - at.y) * r,
            at.z + (next.z - at.z) * r,
        );
    }
    assert!(distance(at, below) < 1.5, "the chase arrives: {at:?}");
    arrives(
        &mut rfc,
        v(-376.811, 209.224, -21.801),
        v(-244.743, 150.085, -18.7494),
        1.5,
    );
    for (from, to) in [
        // Fargodeep, Jasperlode and the Elwynn mines.
        (
            v(-8671.72, -124.325, 92.6409),
            v(-8766.6, -156.686, 82.4446),
        ),
        (v(-9796.72, 131.134, 24.4699), v(-9954.19, 221.055, 26.0012)),
        (
            v(-9326.82, -713.03, 67.5269),
            v(-9088.73, -573.684, 62.5813),
        ),
        (START, v(-8955.0, -140.0, 84.0)),
    ] {
        arrives(&mut azeroth, from, to, 1.5);
    }
}

/// Thistle Tea's `snap_to_ground/2` "drops a mid-jump position back onto
/// the terrain", `query_liquid_surface/2` "returns the liquid surface at
/// a fishing-hole coordinate" and `find_random_point_around_circle/3`
/// "bounds small-radius samples inside large navigation polygons": the
/// ground under a jump is the start's; the sea at Thistle Tea's fishing
/// hole has its surface at 0.268; a point within two yards of Programmer
/// Isle's anchor is within two yards, on its floor, and in its sight.
#[tokio::test]
#[ignore = "extended"]
async fn the_ground_under_a_jump_the_seas_surface_and_points_about_one() {
    let (Some(mut n), Some(mut isle)) = (nav("Azeroth"), nav("development")) else {
        return;
    };
    let ground = n.ground(START.x, START.y, START.z + 4.0).expect("ground");
    assert!((ground - START.z).abs() < 0.5, "{ground}");
    // Namigator's surface there is 0.268; the bake's polygons, rasterised,
    // lie 0.19.
    let surface = n
        .liquid_surface(v(-2183.26, -1867.59, 0.0))
        .expect("the sea");
    assert!((surface - 0.268).abs() < 0.1, "{surface}");
    let anchor = v(16_479.2, 16_468.1, 69.432_78);
    // The navmesh's own pick is anywhere in a polygon the circle touches,
    // eleven yards out on the isle's big ones; a wanderer's, a script's
    // scatter and a summon's are pulled back within the radius.
    let mut rng = rand::rng();
    let points: Vec<Vector3d> = (0..30)
        .filter_map(|_| crate::game::map::world::wander_point(&mut isle, anchor, 2.0, &mut rng))
        .collect();
    assert!(points.len() >= 20, "{} of 30", points.len());
    for p in points {
        let flat = ((p.x - anchor.x).powi(2) + (p.y - anchor.y).powi(2)).sqrt();
        assert!(flat <= 2.01, "{p:?} is {flat} out");
        assert!((p.z - anchor.z).abs() < 1.0, "{p:?}");
        assert!(isle.line_of_sight(anchor, p), "{p:?} in sight");
    }
}

const BLINK: u32 = 997_100;
const R20: u32 = 997;

/// Thistle Tea's `walk_hit_position/3` "stops at the abbey instead of
/// routing around it", "retains the forward line where a detour would
/// reverse direction" and "covers the full distance on open ground", and
/// `first_collision_position/3` "clips a forward destination at the abbey
/// wall" and "retains unobstructed horizontal coordinates": a mage's
/// Blink east at Northshire Abbey's wall stops short of it, on her line,
/// where she sees from where she was; west over open ground it goes its
/// twenty yards, on the ground (mangos's `MovePositionToFirstCollision`).
#[tokio::test]
#[ignore = "extended"]
async fn a_blink_at_the_abbey_stops_at_its_wall_and_goes_its_way_in_the_open() {
    use crate::game::data::spells::SpellEffect;
    use crate::game::map::spells::Targets;
    use crate::game::tests_p3_cast_kit::{bare, data_with, effect, insert};
    use crate::game::tests_p3_spells_kit::{self as kit, cast_fresh};
    if !BAKE.join("Azeroth.map").exists() {
        return;
    }
    let data = data_with(|d| {
        d.spells.radii.insert(R20, 20.0);
        let mut s = bare(BLINK, 0);
        effect(
            &mut s,
            0,
            SpellEffect {
                effect: crate::game::map::spells::EFFECT_LEAP,
                target_a: 1,
                target_b: 47,
                radius_index: R20,
                ..Default::default()
            },
        );
        insert(d, [s]);
    });
    let mut cfg = harness::test_config();
    cfg.nav_dir = Some(BAKE.clone());
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = crate::game::Realm::new(cfg, store, startup, data, None);
    let (_c, a, map) =
        crate::game::tests_p3_cast_kit::human(&realm, "P7WN", "Blinker", Class::Mage).await;
    let (wall, open) = map
        .query(move |m| {
            kit::ready(m, a, 60, &[BLINK]);
            let from = v(-8930.0, -150.0, 82.0);
            m.near_teleport(a, from, 0.0);
            assert!(cast_fresh(m, a, BLINK, Targets::default()));
            let wall = m.players[&a].position();
            let sees = crate::game::map::spells::checks::line_of_sight(m, from, wall);
            m.near_teleport(a, START, std::f32::consts::PI);
            assert!(cast_fresh(m, a, BLINK, Targets::default()));
            let open = m.players[&a].position();
            // South-east along the abbey, where a way round would turn
            // back: on her line, ahead of her.
            let angle = 13.0 * std::f32::consts::PI / 8.0;
            m.near_teleport(a, from, angle);
            assert!(cast_fresh(m, a, BLINK, Targets::default()));
            let p = m.players[&a].position();
            let (dx, dy) = (p.x - from.x, p.y - from.y);
            let (ahead, aside) = (
                dx * angle.cos() + dy * angle.sin(),
                (dx * angle.sin() - dy * angle.cos()).abs(),
            );
            assert!(
                ahead > 0.0 && aside < 0.05,
                "{p:?}: {ahead} ahead, {aside} aside"
            );
            ((wall, sees), open)
        })
        .await
        .unwrap();
    let ((at, sees), from) = (wall, v(-8930.0, -150.0, 82.0));
    assert!(
        at.x > from.x + 1.0 && at.x < -8911.0,
        "short of the wall: {at:?}"
    );
    assert!((at.y - from.y).abs() < 0.05, "on her line: {at:?}");
    assert!(sees, "where she sees from where she was");
    assert!(
        (open.x - (START.x - 20.0)).abs() < 0.05 && (open.y - START.y).abs() < 0.05,
        "twenty yards west: {open:?}"
    );
    assert!((open.z - START.z).abs() < 2.0, "on the ground: {open:?}");
}
