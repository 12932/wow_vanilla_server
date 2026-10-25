//! A chase on a slope keeps to the ground, on the real navmesh: a creature
//! after a victim a few yards down a slope in Tanaris's Waterspring Field
//! walks a spline whose every point is on the ground.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::harness::{self, Client};
use crate::game::map::creature::Motion;
use crate::game::map::{Map, MapHandle, MapKey, ai, combat, grid};

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map answers")
}

fn v((x, y, z): (f32, f32, f32)) -> Vector3d {
    Vector3d { x, y, z }
}

/// Alice on Kalimdor, at `to`, in a realm of the real navmesh and terrain.
async fn in_tanaris(to: Vector3d) -> (Client, Guid, MapHandle) {
    let mut cfg = harness::test_config();
    cfg.terrain_dir = Some(harness::real_data("terrain"));
    cfg.nav_dir = Some(harness::real_data("nav"));
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = crate::game::Realm::new(
        cfg,
        store,
        startup,
        crate::game::tests_p3_combat::data(),
        None,
    );
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let home = realm.maps.get(&realm, MapKey::world(0));
    home.call(move |m| m.teleport(g, MapKey::world(1), to, 0.0));
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    a.sees(g).await;
    let map = realm.maps.get(&realm, MapKey::world(1));
    for _ in 0..10 {
        harness::an_update(&map).await;
    }
    (a, g, map)
}

/// The map's creatures take `n` turns of 50 ms.
fn turns(m: &mut Map, n: u32) {
    for _ in 0..n {
        m.now += Duration::from_millis(50);
        ai::update_creatures(m);
        combat::update_creatures(m);
    }
}

/// Player `g` at `at`, as the map sees it, hardy.
fn put_player(m: &mut Map, g: Guid, at: Vector3d) {
    let p = m.players.get_mut(&g).unwrap();
    p.movement.position = at;
    p.move_check = None;
    p.fields
        .set_u32(crate::game::fields::UNIT_FIELD_MAXHEALTH, 1_000_000);
    p.fields
        .set_u32(crate::game::fields::UNIT_FIELD_HEALTH, 1_000_000);
    let (old, new) = (p.cell, grid::cell_of(at.x, at.y));
    p.cell = new;
    m.grid.relocate(g, old, new);
}

/// Creature `g` at `at`, its grid cell with it.
fn put_creature(m: &mut Map, g: Guid, at: Vector3d) {
    let c = m.creature_mut(g).unwrap();
    let old = c.cell;
    c.position = at;
    c.cell = grid::cell_of(at.x, at.y);
    let new = c.cell;
    m.grid.relocate(g, old, new);
}

/// The ground at `(x, y)` nearest `z`.
fn ground(m: &mut Map, x: f32, y: f32, z: f32) -> f32 {
    m.nav.as_mut().unwrap().ground(x, y, z).expect("ground")
}

/// The chase of a creature at `from` after a victim at `to`, both on the
/// ground: its spline's points.
fn chase(m: &mut Map, g: Guid, from: (f32, f32), to: (f32, f32)) -> Vec<Vector3d> {
    let from = v((from.0, from.1, ground(m, from.0, from.1, 10.0)));
    let to = v((to.0, to.1, ground(m, to.0, to.1, 10.0)));
    put_player(m, g, to);
    let brute =
        crate::game::tests_p3_combat::spawn(m, g, crate::game::tests_p3_combat::BRUTE, 20, 3.0);
    put_creature(m, brute, from);
    ai::attacked(m, brute, g);
    turns(m, 2);
    let points = match &m.creature(brute).unwrap().motion {
        Motion::Moving(w) => w.points.clone(),
        Motion::Idle { .. } => Vec::new(),
    };
    crate::game::map::objects::despawn(m, brute);
    points
}

/// How far under the ground the walk along `points` goes at its
/// deepest, looked at every yard; and how far off the ground its points
/// are at most.
fn off_ground(m: &mut Map, points: &[Vector3d]) -> (f32, f32) {
    let mut under = 0.0f32;
    for w in points.windows(2) {
        let n = (combat::dist(w[0], w[1]).ceil() as usize).max(1);
        for i in 0..=n {
            let f = i as f32 / n as f32;
            let p = Vector3d {
                x: w[0].x + (w[1].x - w[0].x) * f,
                y: w[0].y + (w[1].y - w[0].y) * f,
                z: w[0].z + (w[1].z - w[0].z) * f,
            };
            under = under.max(ground(m, p.x, p.y, p.z) - p.z);
        }
    }
    let off = points
        .iter()
        .map(|p| (p.z - ground(m, p.x, p.y, p.z)).abs())
        .fold(0.0, f32::max);
    (under, off)
}

/// The user's Tanaris drill: a creature chasing a player a little below
/// it went through the ground. A chase's goal is beside its victim at the
/// victim's height, so a few yards up a slope from them it is under the
/// ground; the navmesh keeps an end point's height where it is over a
/// polygon, and joins its polygons' corners straight, over a dune as
/// through it (`Nav::path`). Now each point is on the ground and one is
/// put in wherever the line leaves it: in Waterspring Field, and on the
/// dunes' steeper slopes south-west of it (the second was 3.6 yards under
/// at its end, the first 0.7 under the dune it crossed), every point is
/// within a fifth of a yard of the ground and the walk never goes a third
/// of a yard under it. Each chase five times over: after the first, its
/// victim's earlier attackers set its goal's side at random (a chase's
/// spread), and one of those on the steeper dunes' face is off the
/// navmesh's paths (its steep polygons): its walk went straight through the
/// dune to its victim's height, 1.03 yards off the slope, once in a full
/// extended run (decision 107; now a straight walk is on the ground too,
/// `Nav::straight`).
#[tokio::test]
#[ignore = "extended"]
async fn a_chase_down_a_slope_keeps_to_the_ground() {
    let (_a, g, map) = in_tanaris(v((-7920.0, -4460.0, 9.0))).await;
    let out = on(&map, move |m| {
        let mut out = Vec::new();
        for _ in 0..5 {
            for (from, to) in [
                // Waterspring Field, two yards down a dune and over another.
                ((-7955.0, -4455.0), (-7920.0, -4455.0)),
                ((-7945.0, -4445.0), (-7935.0, -4440.0)),
                // Four and eight yards down the steeper dunes.
                ((-8040.0, -4500.0), (-8030.0, -4500.0)),
                ((-8012.0, -4562.0), (-8000.0, -4555.0)),
            ] {
                let drop = ground(m, from.0, from.1, 10.0) - ground(m, to.0, to.1, 10.0);
                let points = chase(m, g, from, to);
                let (under, off) = off_ground(m, &points);
                out.push((from, drop, points, under, off));
            }
        }
        out
    })
    .await;
    for (from, drop, points, under, off) in out {
        assert!(drop > 1.0, "{from:?}: the victim is below it ({drop})");
        assert!(points.len() >= 2, "{from:?}: a chase");
        assert!(
            under < 0.35,
            "{from:?}: {under} yards under the ground: {points:?}"
        );
        assert!(
            off <= 0.2,
            "{from:?}: a point {off} yards off the ground: {points:?}"
        );
    }
}

/// What putting a path on the ground costs (`Nav::path` against the
/// navmesh's own answer): 2,000 chases of 5 to 40 yards about
/// Waterspring Field, and how many points they gain. Run by hand, in
/// release: `cargo nextest run --release --lib --profile extended
/// --run-ignored all --no-capture --ignore-default-filter -E
/// 'test(probe_path_on_the_ground)'` (the extended profile's default
/// filter leaves the probes out).
#[tokio::test]
#[ignore = "extended"]
async fn probe_path_on_the_ground() {
    use rand::{RngExt, SeedableRng};
    let (_a, _g, map) = in_tanaris(v((-7920.0, -4460.0, 9.0))).await;
    on(&map, move |m| {
        let mut rng = rand::rngs::StdRng::seed_from_u64(7);
        let pairs: Vec<(Vector3d, Vector3d)> = (0..2_000)
            .filter_map(|_| {
                let (x, y) = (
                    -7920.0 + rng.random_range(-60.0..60.0),
                    -4460.0 + rng.random_range(-60.0..60.0),
                );
                let (d, a) = (
                    rng.random_range(5.0..40.0f32),
                    rng.random_range(0.0..std::f32::consts::TAU),
                );
                let nav = m.nav.as_mut().unwrap();
                let from = v((x, y, nav.ground(x, y, 10.0)?));
                let (tx, ty) = (x + d * a.cos(), y + d * a.sin());
                Some((from, v((tx, ty, nav.ground(tx, ty, 10.0)? - 1.0))))
            })
            .collect();
        let nav = m.nav.as_mut().unwrap();
        for _ in 0..2 {
            let t = std::time::Instant::now();
            let raw: usize = pairs
                .iter()
                .filter_map(|(a, b)| nav.path_as_found(*a, *b))
                .map(|p| p.len())
                .sum();
            let raw_t = t.elapsed();
            let t = std::time::Instant::now();
            let grounded: usize = pairs
                .iter()
                .filter_map(|(a, b)| nav.path(*a, *b))
                .map(|p| p.len())
                .sum();
            let grounded_t = t.elapsed();
            println!(
                "{} paths: as found {:?} ({:.1} us each, {raw} points), on the ground {:?} \
                 ({:.1} us each, {grounded} points)",
                pairs.len(),
                raw_t,
                raw_t.as_secs_f64() * 1e6 / pairs.len() as f64,
                grounded_t,
                grounded_t.as_secs_f64() * 1e6 / pairs.len() as f64,
            );
        }
    })
    .await;
}
