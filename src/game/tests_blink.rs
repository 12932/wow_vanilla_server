//! Blink's way (decision 109), on the real world: each case at a real place,
//! cast through the protocol (walls, a corner, a door shut and open, a tree,
//! a hill, a cliff's face and its edge, stairs, a balcony over the way, an
//! overhang, water, open ground, a wall at the caster's nose, a jump, a
//! fall, roots, stuns and Polymorph, Frostwolf Keep's walls, what the client
//! makes of the packets); the property, thousands of Blinks at random over
//! seven places held to an oracle that walks each way twice as finely; and
//! the probes that found the places.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::harness::{self, Client};
use crate::game::map::nav::Nav;
use crate::game::map::{Map, MapHandle, MapKey, grid};

const BLINK: u32 = 1953;

fn nav(name: &str) -> Option<Nav> {
    Nav::open(&harness::real_data("nav"), name)
}

fn v(x: f32, y: f32, z: f32) -> Vector3d {
    Vector3d { x, y, z }
}

/// A realm of the real world, its navmesh and terrain, and a mage in it on
/// map `map_id` at `at`, at level 60 with Blink.
async fn mage_at(map_id: u32, at: Vector3d) -> (Arc<Realm>, Client, Guid, MapHandle) {
    let mut cfg = harness::test_config();
    cfg.nav_dir = Some(harness::real_data("nav"));
    cfg.terrain_dir = Some(harness::real_data("terrain"));
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let data = harness::load_real(&harness::real_data("db"));
    let realm = Realm::new(cfg, store, startup, data, None);
    let mut c = Client::connect(&realm, "M");
    let g = c.enter("Mage", Race::Human, Class::Mage).await;
    let from = realm.registry.online(g).unwrap().map();
    let here = from.query(|m| m.key.map).await.unwrap();
    if here != map_id {
        from.call(move |m| m.teleport(g, MapKey::world(map_id), at, 0.0));
        c.wait_for("SMSG_NEW_WORLD", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
        })
        .await;
        c.send(MSG_MOVE_WORLDPORT_ACK {}).await;
        c.sees(g).await;
    }
    let map = realm.registry.online(g).unwrap().map();
    map.query(move |m| {
        put(m, g, at, 0.0);
        super::tests_p3_spells_kit::ready(m, g, 60, &[BLINK]);
    })
    .await
    .unwrap();
    (realm, c, g, map)
}

/// Player `g` at `at`, facing `o`, standing, as the map has it.
fn put(m: &mut Map, g: Guid, at: Vector3d, o: f32) {
    let p = m.players.get_mut(&g).unwrap();
    p.movement.position = at;
    p.movement.orientation = o;
    p.movement.flags = MovementInfo_MovementFlags::empty();
    p.move_check = None;
    let (old, new) = (p.cell, grid::cell_of(at.x, at.y));
    p.cell = new;
    m.grid.relocate(g, old, new);
}

/// Where a Blink of `g` from `at` facing `o` ends, as the map works it out.
fn blink_end(m: &mut Map, g: Guid, at: Vector3d, o: f32) -> Vector3d {
    put(m, g, at, o);
    crate::game::map::spells::leap::leap_end(m, g, at, o, 20.0)
}

/// Blink cast by `g`'s client from `at` facing `o` with the movement flags
/// `flags`, through the protocol: its `CMSG_CAST_SPELL`, and the
/// `MSG_MOVE_TELEPORT_ACK` that tells its client where it lands, which the
/// client answers as 1.12's does.
async fn cast_blink_with(
    c: &mut Client,
    g: Guid,
    map: &MapHandle,
    at: Vector3d,
    o: f32,
    flags: MovementInfo_MovementFlags,
) -> Vector3d {
    map.query(move |m| {
        use crate::game::fields::{UNIT_FIELD_MAXPOWER1, UNIT_FIELD_POWER1};
        put(m, g, at, o);
        let p = m.players.get_mut(&g).unwrap();
        p.movement.flags = flags;
        let max = p.fields.get_u32(UNIT_FIELD_MAXPOWER1);
        p.fields.set_u32(UNIT_FIELD_POWER1, max);
        super::tests_p3_spells_kit::fresh(m, g);
    })
    .await
    .unwrap();
    c.send(CMSG_CAST_SPELL {
        spell: BLINK,
        targets: crate::game::map::spells::wire_targets(&Default::default()),
    })
    .await;
    let result = c
        .wait_frame(
            "SMSG_CAST_RESULT",
            crate::game::map::spells::SMSG_CAST_RESULT,
            |b| b[..4] == BLINK.to_le_bytes(),
        )
        .await;
    assert_eq!(
        result[4], 0,
        "Blink went from {at:?} facing {o}: {result:?}"
    );
    let to = c
        .wait_for("MSG_MOVE_TELEPORT_ACK", |m| match m {
            ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(t) if t.guid == g => Some(t.info.position),
            _ => None,
        })
        .await;
    c.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: g,
        movement_counter: c.teleport_order(),
        time: Default::default(),
    })
    .await;
    to
}

/// [`cast_blink_with`], standing.
async fn cast_blink(c: &mut Client, g: Guid, map: &MapHandle, at: Vector3d, o: f32) -> Vector3d {
    cast_blink_with(c, g, map, at, o, MovementInfo_MovementFlags::empty()).await
}

/// How far `to` is from `from` over the ground, and how much higher.
fn went(from: Vector3d, to: Vector3d) -> (f32, f32) {
    ((to.x - from.x).hypot(to.y - from.y), to.z - from.z)
}

/// The point `d` yards from `at` toward `o`, at `at`'s height.
fn ahead(at: Vector3d, o: f32, d: f32) -> Vector3d {
    v(at.x + d * o.cos(), at.y + d * o.sin(), at.z)
}

/// Whether the bake's models (WMOs, and doodads if `doodads`) stand across
/// the way from `a` to `b` a yard up.
fn model_between(m: &mut Map, a: Vector3d, b: Vector3d, doodads: bool) -> bool {
    let up = |p: Vector3d| Vector3d { z: p.z + 1.0, ..p };
    m.nav.as_mut().unwrap().crossed_by(up(a), up(b), doodads)
}

/// How far ahead of `at` facing `o`, a yard up and level, the bake's models
/// first stand across the way: the WMOs alone, and with the doodads.
fn first_model(nav: &mut Nav, at: Vector3d, o: f32, yards: f32) -> (Option<f32>, Option<f32>) {
    let a = v(at.x, at.y, at.z + 1.0);
    let b = v(at.x + yards * o.cos(), at.y + yards * o.sin(), at.z + 1.0);
    let mut out = [None, None];
    for (k, doodads) in [false, true].into_iter().enumerate() {
        if !nav.crossed_by(a, b, doodads) {
            continue;
        }
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..20 {
            let mid = (lo + hi) / 2.0;
            let q = v(a.x + (b.x - a.x) * mid, a.y + (b.y - a.y) * mid, a.z);
            if nav.crossed_by(a, q, doodads) {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        out[k] = Some(hi * yards);
    }
    (out[0], out[1])
}

// ---- The cases, each at a real place. -------------------------------------
//
// Facings are WoW's: 0 north (+x), a quarter turn west (+y), half south,
// three quarters east. "The old way" is how wvs worked a Blink out before
// decision 109 (`spells::first_clear`: the ground nearest two yards over
// the caster's height twenty yards ahead, short of what a line of sight
// that only WMOs' faces turned toward it could stop): what it did at each
// place is said, as `probe_blink_cases` showed it.

/// In Ironforge, a wall five yards north of the floor, and one fifteen yards
/// east of another spot: Blink stops half a yard short of each, on its
/// floor, the wall right ahead. The old way went up 12.2 yards onto
/// something over the first (6.6 yards on), and past the second.
#[tokio::test]
#[ignore = "extended"]
async fn blink_stops_short_of_a_wall_five_and_fifteen_yards_ahead() {
    const NEAR: (Vector3d, f32) = (
        Vector3d {
            x: -4906.22,
            y: -976.0,
            z: 501.44,
        },
        0.0,
    );
    const FAR: (Vector3d, f32) = (
        Vector3d {
            x: -4906.0,
            y: -963.83,
            z: 501.46,
        },
        3.0 * std::f32::consts::FRAC_PI_2,
    );
    let (_realm, mut c, g, map) = mage_at(0, NEAR.0).await;
    for (at, o, wall) in [(NEAR.0, NEAR.1, 5.0), (FAR.0, FAR.1, 15.0)] {
        let measured = map
            .query(move |m| first_model(m.nav.as_mut().unwrap(), at, o, 25.0).0)
            .await
            .unwrap()
            .expect("a wall");
        assert!(
            (measured - wall).abs() < 0.1,
            "the wall {measured} yd ahead, not {wall}"
        );
        let to = cast_blink(&mut c, g, &map, at, o).await;
        let (d, dz) = went(at, to);
        assert!(
            (d - (wall - 0.5)).abs() < 0.15 && dz.abs() < 0.15,
            "half a yard short of the wall {wall} yd ahead, on the floor: {d} yd, {dz:+} ({to:?})"
        );
        let (clear, blocked) = map
            .query(move |m| {
                (
                    !model_between(m, at, to, true),
                    model_between(m, to, ahead(to, o, 1.0), false),
                )
            })
            .await
            .unwrap();
        assert!(
            clear && blocked,
            "this side of it: clear behind {clear}, the wall ahead {blocked}"
        );
    }
}

/// Into an inside corner of Drek'Thar's keep, its two walls meeting 8.9
/// yards ahead: Blink stops short of the corner on the floor. The old way
/// went 13.5 yards and 11.1 up, onto the keep.
#[tokio::test]
#[ignore = "extended"]
async fn blink_into_a_corner_stops_short_of_it() {
    let at = v(-1384.39, -226.76, 99.37);
    let o = 3.0 * std::f32::consts::FRAC_PI_4;
    let (_realm, mut c, g, map) = mage_at(30, at).await;
    let walls = map
        .query(move |m| {
            let nav = m.nav.as_mut().unwrap();
            [-20f32, 0.0, 20.0].map(|k| first_model(nav, at, o + k.to_radians(), 25.0).0.unwrap())
        })
        .await
        .unwrap();
    let a = 20f32.to_radians();
    assert!(
        walls[0] * a.cos() < walls[1] - 0.4 && walls[2] * a.cos() < walls[1] - 0.4,
        "a corner ahead: {walls:?}"
    );
    let to = cast_blink(&mut c, g, &map, at, o).await;
    let (d, dz) = went(at, to);
    assert!(
        d < walls[1] - 0.4 && d > walls[1] - 1.5 && dz.abs() < 0.2,
        "{d} yd, {dz:+}"
    );
    let verdict = map.query(move |m| oracle(m, at, o, to)).await.unwrap();
    assert_eq!(verdict, Ok(()));
}

/// The Deadmines' first Heavy Door (17153), shut: Blink from five yards
/// before it stops short of it; open, Blink goes the whole twenty yards
/// through. The old way knew no doors.
#[tokio::test]
#[ignore = "extended"]
async fn blink_goes_through_an_open_door_and_stops_at_a_shut_one() {
    use crate::game::fields::GAMEOBJECT_STATE;
    let at = v(-257.71, -482.36, 49.45);
    let o = std::f32::consts::PI;
    let (_realm, mut c, g, map) = mage_at(36, at).await;
    let door = map
        .query(|m| {
            m.gameobjects
                .iter()
                .find(|(_, o)| o.entry == 17153)
                .map(|(_, o)| (o.guid, o.position, o.fields.get_u32(GAMEOBJECT_STATE)))
        })
        .await
        .unwrap()
        .expect("the Heavy Door");
    assert_eq!(
        door.2,
        crate::game::map::objects::GO_STATE_READY,
        "shut to begin with"
    );
    let shut = cast_blink(&mut c, g, &map, at, o).await;
    let (d, dz) = went(at, shut);
    let to_door = at.x - door.1.x;
    assert!(
        d < to_door - 0.5 && d > 2.0 && dz.abs() < 0.2,
        "short of the door {to_door} yd on: {d} yd ({shut:?})"
    );
    map.query(move |m| {
        if let Some(o) = m.gameobject_mut(door.0) {
            o.fields
                .set_u32(GAMEOBJECT_STATE, crate::game::map::objects::GO_STATE_ACTIVE);
        }
    })
    .await
    .unwrap();
    let open = cast_blink(&mut c, g, &map, at, o).await;
    let (d, dz) = went(at, open);
    assert!(
        (d - 20.0).abs() < 0.05 && dz.abs() < 1.0,
        "through it, twenty yards: {d} yd ({open:?})"
    );
}

/// A tree west of Goldshire (an M2's trunk, no WMO) 4.7 yards east
/// of the spot, and a doodad in Orgrimmar's Valley of Strength 4.5 yards
/// south-east: Blink stops short of each. The old way went through the
/// tree, twenty yards, and through the doodad and 3.7 yards up.
#[tokio::test]
#[ignore = "extended"]
async fn blink_stops_short_of_a_tree_or_a_doodad() {
    for (map_id, at, o) in [
        (
            0,
            v(-9550.0, -41.0, 56.94),
            3.0 * std::f32::consts::FRAC_PI_2,
        ),
        (
            1,
            v(1549.0, -4379.0, 1.35),
            5.0 * std::f32::consts::FRAC_PI_4,
        ),
    ] {
        let (_realm, mut c, g, map) = mage_at(map_id, at).await;
        let (wmo, any) = map
            .query(move |m| first_model(m.nav.as_mut().unwrap(), at, o, 25.0))
            .await
            .unwrap();
        let doodad = any.expect("a doodad ahead");
        assert!(
            wmo.is_none_or(|w| w > doodad + 5.0),
            "a doodad, not a WMO: {wmo:?} {doodad}"
        );
        let to = cast_blink(&mut c, g, &map, at, o).await;
        let (d, dz) = went(at, to);
        assert!(
            d < doodad - 0.4 && d > doodad - 1.2 && dz.abs() < 0.3,
            "short of it {doodad} yd on: {d} yd, {dz:+}"
        );
        let clear = map
            .query(move |m| !model_between(m, at, to, true))
            .await
            .unwrap();
        assert!(clear, "this side of it");
    }
}

/// Up a hill in Thunder Bluff, the terrain rising 9.5 yards over the
/// twenty (25°): Blink goes the whole way up the slope, on it at its end,
/// and never through it (the oracle walks it).
#[tokio::test]
#[ignore = "extended"]
async fn blink_goes_up_a_hill_and_never_through_it() {
    let (at, o) = (v(-1337.0, 9.0, 121.12), 5.8905);
    let (_realm, mut c, g, map) = mage_at(1, at).await;
    let to = cast_blink(&mut c, g, &map, at, o).await;
    let (d, dz) = went(at, to);
    assert!(
        (d - 20.0).abs() < 0.05 && dz > 8.0 && dz < 11.0,
        "up the hill: {d} yd, {dz:+}"
    );
    let (terrain, verdict) = map
        .query(move |m| {
            (
                m.nav.as_mut().unwrap().terrain_height(to.x, to.y),
                oracle(m, at, o, to),
            )
        })
        .await
        .unwrap();
    assert!(
        terrain.is_some_and(|t| (t - to.z).abs() < 0.05),
        "on the slope: {terrain:?} {to:?}"
    );
    assert_eq!(verdict, Ok(()));
}

/// Before a steep cliff face in Thunder Bluff (the terrain rising 38 yards
/// from five yards on): Blink stops at its foot. The old way went to its
/// top, through it, 38.3 yards up.
#[tokio::test]
#[ignore = "extended"]
async fn blink_stops_at_the_foot_of_a_cliff() {
    let (at, o) = (v(-1208.0, 27.0, 127.66), 3.0 * std::f32::consts::FRAC_PI_2);
    let (_realm, mut c, g, map) = mage_at(1, at).await;
    let to = cast_blink(&mut c, g, &map, at, o).await;
    let (d, dz) = went(at, to);
    assert!(
        d > 3.0 && d < 7.0 && dz.abs() < 1.5,
        "at its foot: {d} yd, {dz:+} ({to:?})"
    );
    let (far, verdict) = map
        .query(move |m| {
            let far = ahead(at, o, 20.0);
            let far = m.nav.as_mut().unwrap().terrain_height(far.x, far.y);
            (far, oracle(m, at, o, to))
        })
        .await
        .unwrap();
    assert!(
        far.is_some_and(|f| f > at.z + 20.0),
        "the cliff's top: {far:?}"
    );
    assert_eq!(verdict, Ok(()));
}

/// Toward the edge of Thunder Bluff's mesa, twelve yards on, with thirty
/// yards' drop beyond it: Blink stops at the edge, on the mesa, as
/// mangos-classic's stops at the last look with a floor within reach
/// ("not in water and cannot get correct height"), and vmangos's at the
/// navmesh's edge. The old way went over it, 30.9 yards down.
#[tokio::test]
#[ignore = "extended"]
async fn blink_stops_at_a_cliffs_edge() {
    let (at, o) = (v(-1199.0, 168.0, 131.22), 1.9635);
    let (_realm, mut c, g, map) = mage_at(1, at).await;
    let to = cast_blink(&mut c, g, &map, at, o).await;
    let (d, dz) = went(at, to);
    assert!(
        d > 10.0 && d < 13.0 && dz.abs() < 1.5,
        "at the edge: {d} yd, {dz:+} ({to:?})"
    );
    let (below, verdict) = map
        .query(move |m| {
            let beyond = ahead(to, o, 2.0);
            let floors = m
                .nav
                .as_mut()
                .unwrap()
                .surfaces(beyond.x, beyond.y)
                .map(|s| s.to_vec());
            (floors, oracle(m, at, o, to))
        })
        .await
        .unwrap();
    assert!(
        below
            .as_ref()
            .is_some_and(|f| f.iter().all(|h| *h < to.z - 5.0)),
        "the drop just beyond it: {below:?}"
    );
    assert_eq!(verdict, Ok(()));
}

/// Ironforge's stairs, up 3.5 yards over twenty and down 4.9: Blink goes
/// the whole way along them, on them.
#[tokio::test]
#[ignore = "extended"]
async fn blink_goes_up_and_down_stairs() {
    let up = (
        v(-4861.0, -1036.0, 502.19),
        std::f32::consts::FRAC_PI_2,
        3.52,
    );
    let down = (v(-4891.0, -1090.0, 510.95), 2.7489, -4.85);
    let (_realm, mut c, g, map) = mage_at(0, up.0).await;
    for (at, o, rise) in [up, down] {
        let to = cast_blink(&mut c, g, &map, at, o).await;
        let (d, dz) = went(at, to);
        assert!(
            (d - 20.0).abs() < 0.05 && (dz - rise).abs() < 0.3,
            "along the stairs: {d} yd, {dz:+}"
        );
        let verdict = map.query(move |m| oracle(m, at, o, to)).await.unwrap();
        assert_eq!(verdict, Ok(()));
    }
}

/// Under the Great Forge's balcony, 7.2 yards over the floor ahead: Blink
/// goes on along the floor beneath it, never onto it. The old way ended on
/// the balcony.
#[tokio::test]
#[ignore = "extended"]
async fn blink_never_ends_on_a_balcony_over_its_way() {
    let (at, o) = (v(-4934.77, -985.36, 501.47), 5.4978);
    let (_realm, mut c, g, map) = mage_at(0, at).await;
    let to = cast_blink(&mut c, g, &map, at, o).await;
    let (d, dz) = went(at, to);
    assert!(
        d > 15.0 && dz.abs() < 0.3,
        "on the floor beneath: {d} yd, {dz:+} ({to:?})"
    );
    let (over, verdict) = map
        .query(move |m| {
            let s = m
                .nav
                .as_mut()
                .unwrap()
                .surfaces(to.x, to.y)
                .map(|s| s.to_vec());
            (s, oracle(m, at, o, to))
        })
        .await
        .unwrap();
    assert!(
        over.as_ref()
            .is_some_and(|s| s.iter().any(|h| *h > to.z + 6.0 && *h < to.z + 8.0)),
        "the balcony over it: {over:?}"
    );
    assert_eq!(verdict, Ok(()));
}

/// Standing under an overhang in Stormwind's Trade District, its roof 3.3
/// yards over the caster: Blink out from under it goes the whole way, on
/// the ground.
#[tokio::test]
#[ignore = "extended"]
async fn blink_from_under_an_overhang_goes_its_way() {
    let (at, o) = (v(-8866.0, 676.0, 97.90), 5.1051);
    let (_realm, mut c, g, map) = mage_at(0, at).await;
    let roof = map
        .query(move |m| {
            m.nav.as_mut().unwrap().surfaces(at.x, at.y).and_then(|s| {
                s.iter()
                    .copied()
                    .filter(|h| *h > at.z + 2.0)
                    .fold(None, |b: Option<f32>, h| Some(b.map_or(h, |b| b.min(h))))
            })
        })
        .await
        .unwrap();
    assert!(
        roof.is_some_and(|r| r < at.z + 4.0),
        "a roof over the caster: {roof:?}"
    );
    let to = cast_blink(&mut c, g, &map, at, o).await;
    let (d, dz) = went(at, to);
    assert!(
        (d - 20.0).abs() < 0.05 && dz.abs() < 2.0,
        "out from under it: {d} yd, {dz:+}"
    );
    let verdict = map.query(move |m| oracle(m, at, o, to)).await.unwrap();
    assert_eq!(verdict, Ok(()));
}

/// On open, flat ground in Orgrimmar's Valley of Strength: the whole
/// twenty yards, on the ground.
#[tokio::test]
#[ignore = "extended"]
async fn blink_goes_twenty_yards_over_open_ground() {
    let (at, o) = (v(1540.0, -4490.0, 8.83), std::f32::consts::FRAC_PI_2);
    let (_realm, mut c, g, map) = mage_at(1, at).await;
    let to = cast_blink(&mut c, g, &map, at, o).await;
    let (d, dz) = went(at, to);
    assert!((d - 20.0).abs() < 0.01 && dz.abs() < 0.3, "{d} yd, {dz:+}");
    let ahead_of = ahead(at, o, 20.0);
    assert!(
        (to.x - ahead_of.x).abs() < 0.01 && (to.y - ahead_of.y).abs() < 0.01,
        "straight ahead: {to:?}"
    );
    let verdict = map.query(move |m| oracle(m, at, o, to)).await.unwrap();
    assert_eq!(verdict, Ok(()));
}

/// Facing a wall in Stormwind 0.4 yards ahead: Blink stays where it is,
/// never through the wall. The old way went through it, 5.4 yards.
#[tokio::test]
#[ignore = "extended"]
async fn blink_into_a_wall_half_a_yard_off_stays_put() {
    let (at, o) = (v(-8890.0, 679.0, 100.08), std::f32::consts::PI);
    let (_realm, mut c, g, map) = mage_at(0, at).await;
    let wall = map
        .query(move |m| first_model(m.nav.as_mut().unwrap(), at, o, 5.0).0)
        .await
        .unwrap();
    assert!(
        wall.is_some_and(|w| w < 0.6),
        "the wall just ahead: {wall:?}"
    );
    let to = cast_blink(&mut c, g, &map, at, o).await;
    let (d, dz) = went(at, to);
    assert!(d < 0.1 && dz.abs() < 0.05, "it stays: {d} yd, {dz:+}");
}

/// Into a pond in Thunder Bluff from its bank, 1.6 yards over the water,
/// the water from twelve yards on: Blink goes the whole way, swimming at
/// its end, under the surface by a swimmer's depth (mangos's
/// `IN_OR_UNDER_LIQUID_RANGE`, 0.8).
#[tokio::test]
#[ignore = "extended"]
async fn blink_into_water_swims_on() {
    use crate::game::map::spells::leap::SWIM_DEPTH;
    let (at, o) = (v(-1286.0, 96.0, 130.06), 0.0);
    let (_realm, mut c, g, map) = mage_at(1, at).await;
    let to = cast_blink(&mut c, g, &map, at, o).await;
    let (d, _) = went(at, to);
    let (surface, bed) = map
        .query(move |m| {
            let nav = m.nav.as_mut().unwrap();
            (
                nav.liquid_surface(to),
                nav.surfaces(to.x, to.y).and_then(|s| s.first().copied()),
            )
        })
        .await
        .unwrap();
    let surface = surface.expect("water there");
    assert!((d - 20.0).abs() < 0.05, "the whole way: {d} yd ({to:?})");
    assert!(
        (to.z - (surface - SWIM_DEPTH)).abs() < 0.3,
        "a swimmer's depth under {surface}: {to:?}"
    );
    assert!(
        bed.is_some_and(|b| b < to.z - 0.5),
        "over the pond's bed: {bed:?}"
    );
}

/// Swimming in the pond toward its bank: Blink comes out of the water onto
/// the bank, on the ground; and under water elsewhere it keeps its depth
/// (Thistle Tea's "retains submerged depth", mangos's "under water so next
/// z equal prev z").
#[tokio::test]
#[ignore = "extended"]
async fn blink_from_water_comes_out_onto_the_bank_and_under_it_keeps_its_depth() {
    let swimming = MovementInfo_MovementFlags::empty().set_swimming(Default::default());
    let (at, o) = (v(-1268.0, 96.0, 127.2), std::f32::consts::PI);
    let (_realm, mut c, g, map) = mage_at(1, at).await;
    let to = cast_blink_with(&mut c, g, &map, at, o, swimming).await;
    let (d, _) = went(at, to);
    let (surface, ground) = map
        .query(move |m| {
            let nav = m.nav.as_mut().unwrap();
            (
                nav.liquid_surface(to),
                nav.surfaces(to.x, to.y).map(|s| s.to_vec()),
            )
        })
        .await
        .unwrap();
    assert!(d > 12.0, "out onto the bank: {d} yd ({to:?})");
    assert!(
        ground
            .as_ref()
            .is_some_and(|s| s.iter().any(|h| (h - to.z).abs() < 0.05)),
        "on the ground: {ground:?} {to:?}"
    );
    assert!(
        surface.is_none_or(|s| s < to.z + 1.5),
        "out of the water: {surface:?}"
    );
    drop((_realm, c, map));
    let (at, o) = (v(-2180.0, -1867.59, -5.0), 0.0);
    let (_realm, mut c, g, map) = mage_at(0, at).await;
    let to = cast_blink_with(&mut c, g, &map, at, o, swimming).await;
    let (d, dz) = went(at, to);
    assert!(
        (d - 20.0).abs() < 0.05 && dz.abs() < 0.01,
        "at its depth: {d} yd, {dz:+} ({to:?})"
    );
}

/// Jumping on open ground (a yard and a half up, `MOVEFLAG_JUMPING`):
/// Blink leaps from the ground under the jump, twenty yards on along it
/// (mangos-classic's "fix origin position if player was jumping and near of
/// the ground").
#[tokio::test]
#[ignore = "extended"]
async fn blink_while_jumping_goes_along_the_ground() {
    let (ground, o) = (v(1540.0, -4490.0, 8.83), std::f32::consts::FRAC_PI_2);
    let at = v(ground.x, ground.y, ground.z + 1.5);
    let (_realm, mut c, g, map) = mage_at(1, ground).await;
    let jumping = MovementInfo_MovementFlags::empty().set_jumping(Default::default());
    let to = cast_blink_with(&mut c, g, &map, at, o, jumping).await;
    let (d, _) = went(at, to);
    assert!(
        (d - 20.0).abs() < 0.05 && (to.z - ground.z).abs() < 0.3,
        "on the ground: {d} yd ({to:?})"
    );
}

/// Falling, mangos-classic's falling case (no ground within three yards
/// under it): fifteen yards over open ground, Blink goes twenty yards on and
/// two yards lower, in the air, and the fall goes on from there; six yards
/// over it, onto the ground ahead (within ten yards under that); falling
/// toward one of Ironforge's walls (6.6 yards ahead at that height, its
/// foot a yard high five yards ahead), half a yard short of the wall, lower
/// than it was, never through it. In the air with no fall under way, it
/// stays.
#[tokio::test]
#[ignore = "extended"]
async fn blink_while_falling_falls_on_as_mangos_has_it() {
    let falling = MovementInfo_MovementFlags::empty()
        .set_jumping(Default::default())
        .set_fallingfar();
    let (ground, o) = (v(1540.0, -4490.0, 8.83), std::f32::consts::FRAC_PI_2);
    let (_realm, mut c, g, map) = mage_at(1, ground).await;
    let high = v(ground.x, ground.y, ground.z + 15.0);
    let to = cast_blink_with(&mut c, g, &map, high, o, falling).await;
    let (d, dz) = went(high, to);
    assert!(
        (d - 20.0).abs() < 0.05 && (dz + 2.0).abs() < 0.01,
        "on in the air: {d} yd, {dz:+}"
    );
    let low = v(ground.x, ground.y, ground.z + 6.0);
    let to = cast_blink_with(&mut c, g, &map, low, o, falling).await;
    let (d, _) = went(low, to);
    let floor = map
        .query(move |m| {
            m.nav
                .as_mut()
                .unwrap()
                .surfaces(to.x, to.y)
                .map(|s| s.to_vec())
        })
        .await
        .unwrap();
    assert!(
        (d - 20.0).abs() < 0.05 && floor.is_some_and(|s| s.iter().any(|h| (h - to.z).abs() < 0.05)),
        "onto the ground ahead: {d} yd ({to:?})"
    );
    let still = cast_blink_with(
        &mut c,
        g,
        &map,
        high,
        o,
        MovementInfo_MovementFlags::empty(),
    )
    .await;
    assert_eq!(
        went(high, still),
        (0.0, 0.0),
        "in the air, not falling: it stays"
    );
    drop((_realm, c, map));
    let (wall, o) = (v(-4906.22, -976.0, 501.44), 0.0);
    let (_realm, mut c, g, map) = mage_at(0, wall).await;
    let at = v(wall.x, wall.y, wall.z + 6.0);
    let face = map
        .query(move |m| first_model(m.nav.as_mut().unwrap(), v(at.x, at.y, at.z - 0.5), o, 25.0).1)
        .await
        .unwrap()
        .expect("the wall");
    assert!((face - 6.6).abs() < 0.2, "its face {face} yd ahead");
    let to = cast_blink_with(&mut c, g, &map, at, o, falling).await;
    let (d, dz) = went(at, to);
    assert!(
        d < face - 0.3 && d > face - 1.0 && dz <= 0.0 && dz > -2.0,
        "short of the wall, lower: {d} yd, {dz:+}"
    );
    let clear = map
        .query(move |m| {
            !m.nav
                .as_mut()
                .unwrap()
                .crossed(v(at.x, at.y, at.z + 0.5), to)
        })
        .await
        .unwrap();
    assert!(clear, "this side of it");
}

/// Rooted (Frost Nova), stunned (Hammer of Justice) and Polymorphed, as
/// 1.12 has them (mangos-classic's and vmangos's `CheckCasterAuras`: a
/// spell that grants immunity to a control's mechanic is cast through it,
/// Blink's to stuns and roots): rooted and stunned, Blink goes and takes
/// the root or the stun off; Polymorphed, it is refused, confused.
#[tokio::test]
#[ignore = "extended"]
async fn blink_frees_from_roots_and_stuns_but_not_from_polymorph() {
    use super::tests_p3_spells_kit::{apply, has};
    const FROST_NOVA: u32 = 122;
    const HAMMER_OF_JUSTICE: u32 = 853;
    const POLYMORPH: u32 = 118;
    let (at, o) = (v(1540.0, -4490.0, 8.83), std::f32::consts::FRAC_PI_2);
    let (_realm, mut c, g, map) = mage_at(1, at).await;
    for control in [FROST_NOVA, HAMMER_OF_JUSTICE] {
        // Blink's own immunities from the last, gone.
        map.query(move |m| {
            crate::game::map::auras::remove_spell(m, g, BLINK, None);
            apply(m, g, g, control);
        })
        .await
        .unwrap();
        let held = map.query(move |m| has(m, g, control)).await.unwrap();
        assert!(held, "{control} on");
        let to = cast_blink(&mut c, g, &map, at, o).await;
        let (d, _) = went(at, to);
        assert!((d - 20.0).abs() < 0.05, "{control}: Blink goes, {d} yd");
        let gone = map.query(move |m| !has(m, g, control)).await.unwrap();
        assert!(gone, "{control}: Blink takes it off");
    }
    map.query(move |m| {
        put(m, g, at, o);
        super::tests_p3_spells_kit::fresh(m, g);
        crate::game::map::auras::remove_spell(m, g, BLINK, None);
        apply(m, g, g, POLYMORPH);
    })
    .await
    .unwrap();
    c.clear();
    c.send(CMSG_CAST_SPELL {
        spell: BLINK,
        targets: crate::game::map::spells::wire_targets(&Default::default()),
    })
    .await;
    let b = c
        .wait_frame(
            "SMSG_CAST_RESULT",
            crate::game::map::spells::SMSG_CAST_RESULT,
            |b| b[..4] == BLINK.to_le_bytes(),
        )
        .await;
    assert_eq!(
        (b[4], b[5]),
        (2, crate::game::map::spells::checks::fail::CONFUSED),
        "refused, confused"
    );
    let stayed = map
        .query(move |m| m.players[&g].movement.position)
        .await
        .unwrap();
    assert_eq!(went(at, stayed), (0.0, 0.0), "and it stays");
}

/// In Frostwolf Keep, where a bot mage's Blink in the Drek'Thar all-in put
/// it on top of the keep's wall: from the corridor by his doors (the start
/// of the record, (-1361.41, -245.86, 99.36)), facing every way (every
/// wall), Blink never ends higher than a step over where it started; and
/// from spots across his room (its sunken middle, the ring a yard over it),
/// never on a wall's top or anywhere off the floor it walked: on a floor
/// with room over it, reached over the ground (the oracle), no higher than
/// the stairs out of the room take it. The old way, from the corridor,
/// ended 5.9 yards up facing north, 4.4 up at 22.5°, and 11.8 up on the
/// keep at 202.5°.
#[tokio::test]
#[ignore = "extended"]
async fn blink_in_frostwolf_keep_never_ends_on_its_walls() {
    use crate::game::map::spells::leap::CLIMB;
    // The stairs out of his room climb about five yards over twenty.
    const STAIRS: f32 = 5.0;
    let corridor = v(-1361.41, -245.86, 99.36);
    let (_realm, mut c, g, map) = mage_at(30, corridor).await;
    // Through the protocol from the corridor, each of sixteen ways.
    for k in 0..16 {
        let o = k as f32 * std::f32::consts::TAU / 16.0;
        let to = cast_blink(&mut c, g, &map, corridor, o).await;
        let (_, dz) = went(corridor, to);
        assert!(dz <= CLIMB + 0.05, "facing {o:.3}: {dz:+} yd up ({to:?})");
        let verdict = map
            .query(move |m| oracle(m, corridor, o, to))
            .await
            .unwrap();
        assert_eq!(verdict, Ok(()), "facing {o:.3}, to {to:?}");
    }
    // Across his room, by the map's own working out.
    let bad = map
        .query(move |m| {
            let room = v(-1370.26, -219.70, 98.51);
            let mut bad = Vec::new();
            for i in -2..=2 {
                for j in -2..=2 {
                    let (x, y) = (room.x + 3.0 * i as f32, room.y + 3.0 * j as f32);
                    let Some(z) = m.nav.as_mut().unwrap().ground(x, y, room.z + 0.5) else {
                        continue;
                    };
                    if (z - room.z).abs() > 1.5 {
                        continue;
                    }
                    let at = v(x, y, z);
                    for k in 0..16 {
                        let o = k as f32 * std::f32::consts::TAU / 16.0;
                        let to = blink_end(m, g, at, o);
                        let dz = to.z - at.z;
                        let verdict = oracle(m, at, o, to);
                        if dz > STAIRS || verdict.is_err() {
                            bad.push(format!("from {at:?} facing {o:.3}: {dz:+} up, {verdict:?}"));
                        }
                    }
                }
            }
            bad
        })
        .await
        .unwrap();
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// What the clients see of a Blink, through the 1.12.1 client's own parsing
/// (`tools/client_check`): the caster's client its `MSG_MOVE_TELEPORT_ACK`
/// (its GUID, the counter it answers, its movement at the end, no flags
/// left over from a fall), and an onlooker's its `MSG_MOVE_TELEPORT` (the
/// caster's packed GUID and the same movement), each read exactly by the
/// client's handlers; falling, swimming and on the ground.
#[tokio::test]
#[ignore = "extended"]
async fn a_blink_passes_the_client() {
    let (check_py, wow_exe) = super::tests_client_emu::checker().expect(
        "the client-emulation check needs tools/client_check/WoW.exe and `uv`: see docs/TESTING.md",
    );
    let (at, o) = (v(1540.0, -4490.0, 8.83), std::f32::consts::FRAC_PI_2);
    let (realm, mut c, g, map) = mage_at(1, at).await;
    let mut w = Client::connect(&realm, "W");
    let watcher = w.enter("Watcher", Race::Orc, Class::Warrior).await;
    let here = realm.registry.online(watcher).unwrap().map();
    let next_to = v(at.x + 5.0, at.y + 10.0, at.z);
    here.call(move |m| m.teleport(watcher, MapKey::world(1), next_to, 0.0));
    if here.query(|m| m.key.map).await.unwrap() != 1 {
        w.wait_for("SMSG_NEW_WORLD", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
        })
        .await;
        w.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    }
    w.sees(g).await;
    let falling = MovementInfo_MovementFlags::empty()
        .set_jumping(Default::default())
        .set_fallingfar();
    let swimming = MovementInfo_MovementFlags::empty().set_swimming(Default::default());
    let mut ends = Vec::new();
    for (from, flags) in [
        (at, MovementInfo_MovementFlags::empty()),
        (v(at.x, at.y, at.z + 15.0), falling),
        (v(at.x - 2.0, at.y, at.z), swimming),
    ] {
        ends.push(cast_blink_with(&mut c, g, &map, from, o, flags).await);
        let seen = w
            .wait_for("MSG_MOVE_TELEPORT", |m| match m {
                ServerOpcodeMessage::MSG_MOVE_TELEPORT(t) if t.player == g => Some(t.info),
                _ => None,
            })
            .await;
        let end = ends[ends.len() - 1];
        assert_eq!(
            seen.position, end,
            "the onlooker sees it where its client is"
        );
        assert!(
            seen.flags.is_empty() && seen.fall_time == 0,
            "no fall carried over: {seen:?}"
        );
    }
    let (capture, counts) = super::tests_client_emu::capture(vec![&mut c, &mut w]);
    assert!(
        counts.get(&0x0C7).copied().unwrap_or(0) >= 3
            && counts.get(&0x0C5).copied().unwrap_or(0) >= 3,
        "the teleports captured: {counts:?}"
    );
    super::tests_client_emu::run_checker(&check_py, &wow_exe, &capture);
}

// ---- The property: Blinks at random over real places. --------------------

/// How far apart the oracle looks along a Blink's way: half the way's own
/// look.
const ORACLE_STEP: f32 = crate::game::map::spells::leap::LOOK / 2.0;
/// How much more the oracle lets a look climb, and how much less room it
/// asks over a floor, than the way does: between the way's looks a step's
/// edge or a beam's is met at heights the way's looks don't see, and it
/// passes there by a hair the way's rule is drawn at.
const LENIENCE: f32 = 0.1;

/// Whether a Blink from `start` (on its floor) facing `o` that ended at
/// `end` went as it may, looked at on its own terms, twice as finely as the
/// way is worked out: at most twenty yards ahead, along the facing; on a
/// floor at its end; reached over the ground from the start, the floor at
/// each look a step's climb or drop from the last (so never up a wall, onto
/// a roof or a ledge, or off a cliff), with room to stand over it, and that
/// floor at the end;
/// the way between each two looks clear, at a knee's and a head's height,
/// of the bake's WMOs and doodads (met from either side), the terrain, and
/// the objects in the way; a crack or a dip under three looks wide stepped
/// across. Where the way stays level (within a quarter of a
/// yard of the start's height: not over a step or a crate), the straight
/// segment from start to end is clear too. `Err` says what was wrong.
fn oracle(m: &mut Map, start: Vector3d, o: f32, end: Vector3d) -> Result<(), String> {
    use crate::game::map::spells::leap::{BODY, CLIMB, HEADROOM};
    let (dx, dy) = (end.x - start.x, end.y - start.y);
    let d = dx.hypot(dy);
    if d > 20.0 + 0.01 {
        return Err(format!("{d:.2} yd away"));
    }
    let along = dx * o.cos() + dy * o.sin();
    let across = -dx * o.sin() + dy * o.cos();
    if d > 0.01 && (along < 0.0 || across.abs() > 0.02) {
        return Err(format!("off its way: {along:.2} along, {across:.2} across"));
    }
    let obstacles = crate::game::map::obstacles::Obstacles::near(m, start, 21.0);
    let nav = m.nav.as_mut().expect("a navmesh");
    let floors_at = |nav: &mut Nav, x: f32, y: f32| {
        let mut s: Vec<f32> = nav.surfaces(x, y).map(|s| s.to_vec()).unwrap_or_default();
        obstacles.surfaces(x, y, -5000.0, 5000.0, &mut s);
        s.sort_by(|a, b| b.total_cmp(a));
        s
    };
    if !floors_at(nav, end.x, end.y)
        .iter()
        .any(|h| (h - end.z).abs() < 0.05)
    {
        return Err(format!("not on a floor at its end, {:.2}", end.z));
    }
    let n = (d / ORACLE_STEP).ceil() as usize;
    let mut walked = vec![(0.0f32, start)];
    let mut level = true;
    // The look `i`'s place on from `last`, or why there is none.
    let step = |nav: &mut Nav, last: Vector3d, i: usize| -> Result<Vector3d, String> {
        let f = i as f32 / n as f32;
        let at_d = f * d;
        let (x, y) = (start.x + dx * f, start.y + dy * f);
        let floors = floors_at(nav, x, y);
        let Some(z) = floors
            .iter()
            .copied()
            .find(|h| *h <= last.z + CLIMB + LENIENCE)
            .filter(|z| *z >= last.z - CLIMB - LENIENCE)
        else {
            return Err(format!("no floor to walk on at {at_d:.2} yd"));
        };
        if let Some(over) = floors.iter().rev().find(|h| **h > z + 0.05)
            && *over < z + HEADROOM - 2.0 * LENIENCE
        {
            return Err(format!(
                "no room at {at_d:.2} yd, {:.2} over the floor",
                over - z
            ));
        }
        let at = v(x, y, z);
        for h in BODY {
            let up = |p: Vector3d| Vector3d { z: p.z + h, ..p };
            if nav.crossed(up(last), up(at))
                || nav.into_terrain(up(last), up(at))
                || obstacles.crossed(up(last), up(at))
            {
                return Err(format!("through something {h} yd up at {at_d:.2} yd"));
            }
        }
        Ok(at)
    };
    // Each look on from the last place walked; failing that, from one of
    // the three before it (a crack in the floor, a dip between two steps,
    // stepped across as the way does); failing those, the next look, up to
    // three in a row.
    let mut missed: Option<String> = None;
    let mut misses = 0;
    for i in 1..=n {
        let k = walked.len();
        let tried = (k.saturating_sub(4)..k)
            .rev()
            .find_map(|j| step(nav, walked[j].1, i).ok().map(|at| (j, at)));
        match tried {
            Some((j, at)) => {
                walked.truncate(j + 1);
                level &= (at.z - start.z).abs() <= 0.25;
                walked.push((i as f32 / n as f32 * d, at));
                misses = 0;
            }
            None => {
                let why = step(nav, walked[k - 1].1, i).err().unwrap_or_default();
                misses += 1;
                if misses > 3 || i == n {
                    return Err(missed.unwrap_or(why));
                }
                missed.get_or_insert(why);
                continue;
            }
        }
        missed = None;
    }
    let last = walked[walked.len() - 1].1;
    if (last.z - end.z).abs() > 0.1 {
        return Err(format!(
            "the ground there is at {:.2}, it ended at {:.2}",
            last.z, end.z
        ));
    }
    if level {
        // At a chest's height: a knee's grazes a riser's edge, a head's a
        // shelf's, as the way between two looks may and the line from end
        // to end not.
        let h = BODY[1];
        let up = |p: Vector3d| Vector3d { z: p.z + h, ..p };
        if nav.crossed(up(start), up(end))
            || nav.into_terrain(up(start), up(end))
            || obstacles.crossed(up(start), up(end))
        {
            return Err(format!("the straight way {h} yd up crosses something"));
        }
    }
    Ok(())
}

/// Whether there is water deep enough to swim in within twenty yards of
/// `at` toward `o`, or over it.
fn wet(m: &mut Map, at: Vector3d, o: f32) -> bool {
    let nav = m.nav.as_mut().expect("a navmesh");
    (0..=42).any(|i| {
        let d = i as f32 * 0.5;
        let (x, y) = (at.x + d * o.cos(), at.y + d * o.sin());
        nav.liquid_surface(v(x, y, at.z)).is_some_and(|s| {
            nav.surfaces(x, y)
                .is_some_and(|f| f.iter().any(|h| *h < s - 1.0 && *h > s - 30.0))
        })
    })
}

/// A spot on a floor within `r` yards of `centre` (about its height), with
/// room to stand, chosen by `rng`.
fn spot(
    m: &mut Map,
    rng: &mut impl rand::RngExt,
    centre: Vector3d,
    r: f32,
    zr: f32,
) -> Option<Vector3d> {
    let nav = m.nav.as_mut().expect("a navmesh");
    for _ in 0..50 {
        let (a, d) = (
            rng.random::<f32>() * std::f32::consts::TAU,
            r * rng.random::<f32>().sqrt(),
        );
        let (x, y) = (centre.x + d * a.cos(), centre.y + d * a.sin());
        let Some(s) = nav.surfaces(x, y).map(|s| s.to_vec()) else {
            continue;
        };
        let roomy: Vec<f32> = s
            .iter()
            .enumerate()
            .filter(|(k, h)| (*h - centre.z).abs() <= zr && (*k == 0 || s[k - 1] > *h + 2.2))
            .map(|(_, h)| *h)
            .collect();
        if let Some(&h) = roomy.get(
            rng.random_range(0..roomy.len().max(1))
                .min(roomy.len().saturating_sub(1)),
        ) {
            return Some(v(x, y, h));
        }
    }
    None
}

/// The property: `n` Blinks from spots about `centre` on map `map_id`, each
/// at a facing of its own, all as the oracle has them. Those whose way
/// meets deep water are left to the water's tests.
async fn blinks_hold_about(map_id: u32, centre: Vector3d, r: f32, zr: f32, n: usize, seed: u64) {
    // By hand, more of them, or others: `BLINK_N`, `BLINK_SEED`.
    let n = std::env::var("BLINK_N").map_or(n, |s| s.parse().unwrap());
    let seed = std::env::var("BLINK_SEED").map_or(seed, |s| seed + s.parse::<u64>().unwrap());
    let (_realm, _c, g, map) = mage_at(map_id, centre).await;
    let (bad, tried, moved) = map
        .query(move |m| {
            use rand::{RngExt, SeedableRng};
            let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
            let (mut bad, mut tried, mut moved) = (Vec::new(), 0, 0.0);
            let mut spent = std::time::Duration::ZERO;
            while tried < n {
                let Some(at) = spot(m, &mut rng, centre, r, zr) else {
                    continue;
                };
                let o = rng.random::<f32>() * std::f32::consts::TAU;
                if wet(m, at, o) {
                    continue;
                }
                tried += 1;
                // By hand, the old way (`BLINK_OLD`), to see the oracle find
                // what was wrong with it.
                let end = if std::env::var_os("BLINK_OLD").is_some() {
                    let want = v(at.x + 20.0 * o.cos(), at.y + 20.0 * o.sin(), at.z);
                    crate::game::map::spells::first_clear(m, at, want)
                } else {
                    let t = std::time::Instant::now();
                    let end = blink_end(m, g, at, o);
                    spent += t.elapsed();
                    end
                };
                moved += (end.x - at.x).hypot(end.y - at.y);
                if let Err(e) = oracle(m, at, o, end) {
                    bad.push(format!(
                        "from ({:.2}, {:.2}, {:.2}) facing {o:.4} to ({:.2}, {:.2}, {:.2}): {e}",
                        at.x, at.y, at.z, end.x, end.y, end.z
                    ));
                }
            }
            println!("{tried} Blinks, {:?} each", spent / tried.max(1) as u32);
            (bad, tried, moved)
        })
        .await
        .unwrap();
    assert!(
        bad.is_empty(),
        "{} of {tried} Blinks went wrong:\n{}",
        bad.len(),
        bad.join("\n")
    );
    // And they go somewhere: not every Blink stopped at once.
    assert!(
        moved / tried as f32 > 3.0,
        "{:.1} yd on average",
        moved / tried as f32
    );
}

#[tokio::test]
#[ignore = "extended"]
async fn blinks_about_stormwinds_trade_district_hold() {
    blinks_hold_about(0, v(-8830.0, 640.0, 96.0), 70.0, 10.0, 250, 1).await;
}

#[tokio::test]
#[ignore = "extended"]
async fn blinks_about_the_great_forge_hold() {
    blinks_hold_about(0, v(-4880.0, -1020.0, 503.0), 60.0, 8.0, 250, 2).await;
}

#[tokio::test]
#[ignore = "extended"]
async fn blinks_about_orgrimmars_valley_of_strength_hold() {
    blinks_hold_about(1, v(1600.0, -4420.0, 12.0), 80.0, 14.0, 250, 3).await;
}

#[tokio::test]
#[ignore = "extended"]
async fn blinks_in_elwynns_woods_hold() {
    blinks_hold_about(0, v(-9520.0, 20.0, 58.0), 80.0, 10.0, 250, 4).await;
}

#[tokio::test]
#[ignore = "extended"]
async fn blinks_in_frostwolf_keep_hold() {
    blinks_hold_about(30, v(-1361.41, -245.86, 99.36), 45.0, 6.0, 250, 5).await;
}

#[tokio::test]
#[ignore = "extended"]
async fn blinks_about_thunder_bluff_hold() {
    blinks_hold_about(1, v(-1280.0, 80.0, 130.0), 90.0, 12.0, 250, 6).await;
}

/// The Deadmines, a dungeon that is one WMO: Blinks about its mine and
/// shipyard hold as the oracle has them.
#[tokio::test]
#[ignore = "extended"]
async fn blinks_in_the_deadmines_hold() {
    blinks_hold_about(36, v(-191.0, -457.0, 54.0), 60.0, 12.0, 250, 7).await;
}

// ---- Probes, run by hand: how the spots above were found. ----------------

/// What a straight walk ahead from a spot meets, every half yard, as the
/// scout sees it.
#[derive(Debug, Default, Clone)]
struct Profile {
    /// The floor at each half yard while it goes on (a half yard's climb
    /// or drop at most).
    floors: Vec<f32>,
    /// Where it broke off, if it did: how far, and the floor there nearest
    /// the last (up or down by how much), if any.
    broke: Option<(f32, Option<f32>)>,
    /// The first WMO, and the first doodad, met a yard up, how far.
    wmo: Option<f32>,
    doodad: Option<f32>,
    /// Where the terrain is the floor, by half yard.
    terrain: Vec<bool>,
    /// The water's surface first met, how far, and its height.
    water: Option<(f32, f32)>,
    /// The nearest surface over the floor within eight yards, by half yard.
    over: Vec<Option<f32>>,
}

fn profile(nav: &mut Nav, at: Vector3d, o: f32, yards: f32) -> Profile {
    let mut p = Profile::default();
    let mut last = at;
    let n = (yards / 0.5) as usize;
    for i in 1..=n {
        let d = i as f32 * 0.5;
        let (x, y) = (at.x + d * o.cos(), at.y + d * o.sin());
        let s: Vec<f32> = nav.surfaces(x, y).map(|s| s.to_vec()).unwrap_or_default();
        let floor = s
            .iter()
            .copied()
            .filter(|h| *h <= last.z + 0.6)
            .fold(None, |b: Option<f32>, h| Some(b.map_or(h, |b| b.max(h))));
        let next = match floor.filter(|f| *f >= last.z - 0.6) {
            Some(f) => v(x, y, f),
            None => {
                let near = s
                    .iter()
                    .copied()
                    .min_by(|a, b| (a - last.z).abs().total_cmp(&(b - last.z).abs()));
                p.broke = Some((d, near.map(|h| h - last.z)));
                break;
            }
        };
        let up = |q: Vector3d| Vector3d { z: q.z + 1.0, ..q };
        if p.wmo.is_none() && nav.crossed_by(up(last), up(next), false) {
            p.wmo = Some(d);
        }
        if p.doodad.is_none() && p.wmo.is_none() && nav.crossed_by(up(last), up(next), true) {
            p.doodad = Some(d);
        }
        if p.water.is_none()
            && let Some(w) = nav.liquid_surface(v(x, y, next.z))
            && w > next.z + 0.3
        {
            p.water = Some((d, w));
        }
        p.terrain.push(
            nav.terrain_height(x, y)
                .is_some_and(|t| (t - next.z).abs() < 0.05),
        );
        p.over.push(
            s.iter()
                .copied()
                .filter(|h| *h > next.z + 0.3 && *h < next.z + 8.0)
                .fold(None, |b: Option<f32>, h| Some(b.map_or(h, |b| b.min(h)))),
        );
        p.floors.push(next.z);
        last = next;
    }
    p
}

/// Each facing's Blink from a spot, as it was (the old way: the ground
/// nearest two yards over the caster's height, twenty yards ahead, short of
/// what a front-facing line of sight met) and as it is (run by hand).
#[tokio::test]
#[ignore = "a probe, run by hand"]
async fn probe_blinks_at() {
    let map_id: u32 = std::env::var("PROBE_MAP_ID").map_or(30, |s| s.parse().unwrap());
    let at: Vec<f32> = std::env::var("PROBE_AT")
        .unwrap_or_else(|_| "-1361.41,-245.86,99.36".into())
        .split(',')
        .map(|v| v.trim().parse().unwrap())
        .collect();
    let at = v(at[0], at[1], at[2]);
    let (_realm, _c, g, map) = mage_at(map_id, at).await;
    let lines = map
        .query(move |m| {
            let mut out = Vec::new();
            for k in 0..16 {
                let o = k as f32 * std::f32::consts::TAU / 16.0;
                let want = v(at.x + 20.0 * o.cos(), at.y + 20.0 * o.sin(), at.z);
                let old = crate::game::map::spells::first_clear(m, at, want);
                let new = blink_end(m, g, at, o);
                let d = |p: Vector3d| (p.x - at.x).hypot(p.y - at.y);
                out.push(format!(
                    "{:5.1}deg: old {:5.1} yd dz {:+6.2} ({:.2}, {:.2}, {:.2})   new {:5.1} yd dz {:+6.2} ({:.2}, {:.2}, {:.2})",
                    o.to_degrees(),
                    d(old),
                    old.z - at.z,
                    old.x,
                    old.y,
                    old.z,
                    d(new),
                    new.z - at.z,
                    new.x,
                    new.y,
                    new.z
                ));
            }
            out
        })
        .await
        .unwrap();
    for l in lines {
        println!("{l}");
    }
}

/// The surfaces about a spot, and what a ray each way meets (run by hand).
#[test]
#[ignore = "a probe, run by hand"]
fn probe_blink_spot() {
    let name = std::env::var("PROBE_MAP").unwrap_or_else(|_| "PVPZone01".into());
    let at: Vec<f32> = std::env::var("PROBE_AT")
        .unwrap_or_else(|_| "-1361.41,-245.86,99.36".into())
        .split(',')
        .map(|v| v.trim().parse().unwrap())
        .collect();
    let Some(mut nav) = nav(&name) else {
        return;
    };
    let from = v(at[0], at[1], at[2]);
    println!("at {from:?}: {:?}", nav.surfaces(from.x, from.y));
    for k in 0..16 {
        let o = k as f32 * std::f32::consts::TAU / 16.0;
        let mut line = format!("{:5.1}deg:", o.to_degrees());
        for r in 1..=20 {
            let (x, y) = (from.x + r as f32 * o.cos(), from.y + r as f32 * o.sin());
            let s: Vec<String> = nav
                .surfaces(x, y)
                .unwrap_or(&[])
                .iter()
                .filter(|h| (*h - from.z).abs() < 12.0)
                .map(|h| format!("{:.1}", h - from.z))
                .collect();
            let up = |p: Vector3d, h: f32| Vector3d { z: p.z + h, ..p };
            let to = v(x, y, from.z);
            let hit = nav.crossed(up(from, 1.0), up(to, 1.0));
            let t = nav
                .terrain_height(x, y)
                .map_or("-".into(), |h| format!("{:.1}", h - from.z));
            line += &format!(" {r}{}[{}|t{t}]", if hit { "#" } else { "" }, s.join("/"));
        }
        println!("{line}");
    }
}

/// The spots about a place for each case of Blink's way (run by hand):
/// `PROBE_MAP`, `PROBE_AT` (x, y, the floor's height about), `PROBE_R`.
#[test]
#[ignore = "a probe, run by hand"]
fn probe_blink_scout() {
    let name = std::env::var("PROBE_MAP").unwrap_or_else(|_| "Azeroth".into());
    let at: Vec<f32> = std::env::var("PROBE_AT")
        .unwrap_or_else(|_| "-8830,640,94".into())
        .split(',')
        .map(|v| v.trim().parse().unwrap())
        .collect();
    let r: f32 = std::env::var("PROBE_R").map_or(60.0, |s| s.parse().unwrap());
    let zr: f32 = std::env::var("PROBE_ZR").map_or(15.0, |s| s.parse().unwrap());
    let Some(mut nav) = nav(&name) else {
        return;
    };
    let mut found: std::collections::BTreeMap<&str, Vec<String>> = Default::default();
    let mut add = |k: &'static str, s: String| {
        let e = found.entry(k).or_default();
        if e.len() < 6 {
            e.push(s);
        }
    };
    let step = 3.0;
    let n = (r / step) as i32;
    for i in -n..=n {
        for j in -n..=n {
            let (x, y) = (at[0] + i as f32 * step, at[1] + j as f32 * step);
            let floors: Vec<f32> = nav.surfaces(x, y).map(|s| s.to_vec()).unwrap_or_default();
            for (k, &h) in floors.iter().enumerate() {
                if (h - at[2]).abs() > zr {
                    continue;
                }
                // Room to stand: nothing within two yards over it.
                if k > 0 && floors[k - 1] < h + 2.2 {
                    continue;
                }
                let here = v(x, y, h);
                let ceiling = (k > 0).then(|| floors[k - 1] - h);
                for f in 0..16 {
                    let o = f as f32 * std::f32::consts::TAU / 16.0;
                    let p = profile(&mut nav, here, o, 21.0);
                    let tag = format!(
                        "({x:.2}, {y:.2}, {h:.2}) o {o:.4} ({:.1} deg)",
                        o.to_degrees()
                    );
                    let flat_to = |d: f32| {
                        let k = ((d / 0.5) as usize).min(p.floors.len());
                        k > 0 && p.floors[..k].iter().all(|z| (z - h).abs() < 0.25)
                    };
                    let all_clear = p.wmo.is_none() && p.doodad.is_none() && p.water.is_none();
                    if let Some(w) = p.wmo {
                        if (4.5..=5.5).contains(&w) && flat_to(w - 0.5) && p.doodad.is_none() {
                            add("wall_5", format!("{tag} wall at {w}"));
                        }
                        if (14.5..=15.5).contains(&w) && flat_to(w - 0.5) && p.doodad.is_none() {
                            add("wall_15", format!("{tag} wall at {w}"));
                        }
                        if w <= 1.0 && p.doodad.is_none() {
                            add("wall_close", format!("{tag} wall at {w}"));
                        }
                    }
                    if let Some(dd) = p.doodad
                        && (4.0..=14.0).contains(&dd)
                        && flat_to(dd - 0.5)
                    {
                        add("doodad", format!("{tag} doodad at {dd}"));
                    }
                    if p.floors.len() >= 41
                        && all_clear
                        && flat_to(20.5)
                        && p.over.iter().all(|o| o.is_none())
                    {
                        add("open_flat", tag.clone());
                    }
                    if p.floors.len() >= 41 && all_clear && p.terrain.iter().all(|t| *t) {
                        let rise = p.floors[39] - h;
                        if (5.0..=12.0).contains(&rise) {
                            add("hill_up", format!("{tag} rise {rise:.1}"));
                        }
                        if (-12.0..=-5.0).contains(&rise) {
                            add("hill_down", format!("{tag} rise {rise:.1}"));
                        }
                    }
                    if let Some((d, Some(dz))) = p.broke
                        && (4.0..=14.0).contains(&d)
                        && p.wmo.is_none()
                        && p.doodad.is_none()
                        && p.terrain.iter().all(|t| *t)
                    {
                        if dz > 3.0 {
                            add("cliff_face", format!("{tag} at {d} up {dz:.1}"));
                        }
                        if dz < -4.0 && p.water.is_none() {
                            add("ledge", format!("{tag} at {d} down {dz:.1}"));
                        }
                    }
                    if let Some((d, None)) = p.broke
                        && (4.0..=14.0).contains(&d)
                        && p.wmo.is_none()
                        && p.doodad.is_none()
                        && p.water.is_none()
                    {
                        add("ledge_deep", format!("{tag} at {d} nothing under"));
                    }
                    if p.floors.len() >= 41
                        && p.wmo.is_none()
                        && p.doodad.is_none()
                        && !p.terrain.iter().any(|t| *t)
                    {
                        let rise = p.floors[39] - h;
                        let steps = p
                            .floors
                            .windows(2)
                            .filter(|w| (w[1] - w[0]).abs() > 0.15)
                            .count();
                        if rise > 3.0 && steps >= 4 {
                            add("stairs_up", format!("{tag} rise {rise:.1} steps {steps}"));
                        }
                        if rise < -3.0 && steps >= 4 {
                            add("stairs_down", format!("{tag} rise {rise:.1} steps {steps}"));
                        }
                    }
                    if all_clear && flat_to(10.0) {
                        let under: Vec<usize> = p
                            .over
                            .iter()
                            .enumerate()
                            .filter(|(_, o)| o.is_some_and(|o| o - h > 2.5 && o - h < 6.0))
                            .map(|(i, _)| i)
                            .collect();
                        if let (Some(&a), Some(&b)) = (under.first(), under.last())
                            && a >= 4
                            && b <= 30
                            && b - a >= 3
                        {
                            add(
                                "bridge_over",
                                format!(
                                    "{tag} over from {} to {}",
                                    (a + 1) as f32 * 0.5,
                                    (b + 1) as f32 * 0.5
                                ),
                            );
                        }
                    }
                    if let Some(c) = ceiling
                        && (2.2..6.0).contains(&c)
                        && p.wmo.is_none()
                        && p.doodad.is_none()
                        && p.floors.len() >= 41
                        && p.over[30..].iter().all(|o| o.is_none())
                    {
                        add("overhang", format!("{tag} ceiling {c:.1}"));
                    }
                    if let Some((d, w)) = p.water
                        && (4.0..=12.0).contains(&d)
                        && p.wmo.is_none()
                        && p.doodad.is_none()
                    {
                        add("water_in", format!("{tag} water at {d} surface {w:.2}"));
                    }
                }
            }
        }
    }
    for (k, list) in found {
        println!("{k}:");
        for s in list {
            println!("    {s}");
        }
    }
}

/// A Blink's way, look by look, and the oracle's verdict (run by hand):
/// `PROBE_MAP_ID`, `PROBE_AT` (x, y, z), `PROBE_O`.
#[tokio::test]
#[ignore = "a probe, run by hand"]
async fn probe_blink_trace() {
    let map_id: u32 = std::env::var("PROBE_MAP_ID").map_or(0, |s| s.parse().unwrap());
    let at: Vec<f32> = std::env::var("PROBE_AT")
        .unwrap()
        .split(',')
        .map(|v| v.trim().parse().unwrap())
        .collect();
    let o: f32 = std::env::var("PROBE_O").unwrap().parse().unwrap();
    let at = v(at[0], at[1], at[2]);
    let (_realm, _c, g, map) = mage_at(map_id, at).await;
    let (end, lines, verdict, surfaces) = map
        .query(move |m| {
            put(m, g, at, o);
            let (end, lines) = crate::game::map::spells::leap::trace(m, g, at, o, 20.0);
            let verdict = oracle(m, at, o, end);
            let mut surfaces = Vec::new();
            for i in 0..=((at.x - end.x).hypot(at.y - end.y) / 0.25).ceil() as usize + 4 {
                let d = i as f32 * 0.25;
                let (x, y) = (at.x + d * o.cos(), at.y + d * o.sin());
                let s = m
                    .nav
                    .as_mut()
                    .unwrap()
                    .surfaces(x, y)
                    .map(|s| s.to_vec())
                    .unwrap_or_default();
                let s: Vec<String> = s
                    .iter()
                    .filter(|h| (*h - at.z).abs() < 6.0)
                    .map(|h| format!("{h:.2}"))
                    .collect();
                surfaces.push(format!("  {d:5.2}: {}", s.join(" ")));
            }
            let obstacles = crate::game::map::obstacles::Obstacles::near(m, at, 21.0);
            for h in [1.0f32, 1.6] {
                let (a, b) = (
                    Vector3d { z: at.z + h, ..at },
                    Vector3d {
                        z: end.z + h,
                        ..end
                    },
                );
                if obstacles.crossed(a, b) {
                    let (mut lo, mut hi) = (0.0f32, 1.0f32);
                    for _ in 0..20 {
                        let mid = (lo + hi) / 2.0;
                        let q = v(
                            a.x + (b.x - a.x) * mid,
                            a.y + (b.y - a.y) * mid,
                            a.z + (b.z - a.z) * mid,
                        );
                        if obstacles.crossed(a, q) {
                            hi = mid
                        } else {
                            lo = mid
                        }
                    }
                    let q = v(a.x + (b.x - a.x) * hi, a.y + (b.y - a.y) * hi, a.z);
                    let near: Vec<String> = m
                        .gameobjects
                        .iter()
                        .filter(|(_, o)| (o.position.x - q.x).hypot(o.position.y - q.y) < 15.0)
                        .map(|(_, o)| {
                            format!(
                                "{} d{} ({:.2}, {:.2}, {:.2}) o {:.3} kind {} state {}",
                                o.entry,
                                o.fields.get_u32(crate::game::fields::GAMEOBJECT_DISPLAYID),
                                o.position.x,
                                o.position.y,
                                o.position.z,
                                o.orientation,
                                o.kind,
                                o.fields.get_u32(crate::game::fields::GAMEOBJECT_STATE)
                            )
                        })
                        .collect();
                    surfaces.push(format!(
                        "an object {h} up at {hi:.3} ({:.2}, {:.2}); near: {near:?}",
                        q.x, q.y
                    ));
                    let len = (b.x - a.x).hypot(b.y - a.y);
                    for piece in [0.125f32, 0.25, 0.5, 1.0, 2.0, 4.0] {
                        let t0 = (hi - piece / len / 2.0).max(0.0);
                        let t1 = (hi + piece / len / 2.0).min(1.0);
                        let pa = v(
                            a.x + (b.x - a.x) * t0,
                            a.y + (b.y - a.y) * t0,
                            a.z + (b.z - a.z) * t0,
                        );
                        let pb = v(
                            a.x + (b.x - a.x) * t1,
                            a.y + (b.y - a.y) * t1,
                            a.z + (b.z - a.z) * t1,
                        );
                        surfaces.push(format!(
                            "   a piece of {piece} about it: {}",
                            obstacles.crossed(pa, pb)
                        ));
                    }
                }
            }
            (end, lines, verdict, surfaces)
        })
        .await
        .unwrap();
    for l in lines {
        println!("{l}");
    }
    println!("end {end:?}: {verdict:?}");
    for s in surfaces {
        println!("{s}");
    }
}

/// Where a segment first meets the bake's models, and what they are
/// (run by hand): `PROBE_MAP`, `PROBE_FROM`, `PROBE_TO` (x, y, z).
#[test]
#[ignore = "a probe, run by hand"]
fn probe_segment_hit() {
    let name = std::env::var("PROBE_MAP").unwrap_or_else(|_| "Azeroth".into());
    let p = |k: &str| {
        let a: Vec<f32> = std::env::var(k)
            .unwrap()
            .split(',')
            .map(|v| v.trim().parse().unwrap())
            .collect();
        v(a[0], a[1], a[2])
    };
    let (from, to) = (p("PROBE_FROM"), p("PROBE_TO"));
    let Some(mut nav) = nav(&name) else {
        return;
    };
    let along = |t: f32| {
        v(
            from.x + (to.x - from.x) * t,
            from.y + (to.y - from.y) * t,
            from.z + (to.z - from.z) * t,
        )
    };
    for doodads in [false, true] {
        let hit = nav.crossed_by(from, to, doodads);
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        if hit {
            for _ in 0..20 {
                let mid = (lo + hi) / 2.0;
                if nav.crossed_by(from, along(mid), doodads) {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
        }
        let at = along(hi);
        println!(
            "doodads {doodads}: crossed {hit}, first at {hi:.4} ({:.2}, {:.2}, {:.2}); surfaces there {:?}; terrain {:?}",
            at.x,
            at.y,
            at.z,
            nav.surfaces(at.x, at.y).map(|s| s.to_vec()),
            nav.terrain_height(at.x, at.y)
        );
    }
    println!("into terrain: {}", nav.into_terrain(from, to));
}

/// Each case's spot, as the old way and the new take it (run by hand):
/// `PROBE_CASES` lines of `label map_id x y z o`.
#[tokio::test]
#[ignore = "a probe, run by hand"]
async fn probe_blink_cases() {
    let text = std::fs::read_to_string(std::env::var("PROBE_CASES").unwrap()).unwrap();
    let cases: Vec<(String, u32, Vector3d, f32)> = text
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| {
            let w: Vec<&str> = l.split_whitespace().collect();
            let f = |i: usize| w[i].parse::<f32>().unwrap();
            (
                w[0].to_owned(),
                w[1].parse().unwrap(),
                v(f(2), f(3), f(4)),
                f(5),
            )
        })
        .collect();
    let mut maps = cases.iter().map(|c| c.1).collect::<Vec<_>>();
    maps.dedup();
    for map_id in maps {
        let first = cases.iter().find(|c| c.1 == map_id).unwrap().2;
        let (_realm, _c, g, map) = mage_at(map_id, first).await;
        let mine: Vec<_> = cases.iter().filter(|c| c.1 == map_id).cloned().collect();
        let lines = map
            .query(move |m| {
                let mut out = Vec::new();
                for (label, _, at, o) in mine {
                    let want = v(at.x + 20.0 * o.cos(), at.y + 20.0 * o.sin(), at.z);
                    let old = crate::game::map::spells::first_clear(m, at, want);
                    put(m, g, at, o);
                    let (new, trace) = crate::game::map::spells::leap::trace(m, g, at, o, 20.0);
                    let verdict = oracle(m, at, o, new);
                    let (wmo, any) = first_model(m.nav.as_mut().unwrap(), at, o, 25.0);
                    let d = |p: Vector3d| (p.x - at.x).hypot(p.y - at.y);
                    let tail: Vec<String> = trace.iter().rev().take(4).rev().cloned().collect();
                    out.push(format!(
                        "{label}: wall {wmo:?} model {any:?} | old {:.2} yd dz {:+.2} | new {:.2} yd dz {:+.2} ({:.2}, {:.2}, {:.2}) {verdict:?}\n      {}",
                        d(old),
                        old.z - at.z,
                        d(new),
                        new.z - at.z,
                        new.x,
                        new.y,
                        new.z,
                        tail.join(" / ")
                    ));
                }
                out
            })
            .await
            .unwrap();
        for l in lines {
            println!("{l}");
        }
    }
}

/// Spots about a place where the old way ended over a bridge's deck or a
/// roof and the new under it, and inside corners ahead (run by hand):
/// `PROBE_MAP_ID`, `PROBE_AT`, `PROBE_R`, `PROBE_ZR`.
#[tokio::test]
#[ignore = "a probe, run by hand"]
async fn probe_blink_finds() {
    let map_id: u32 = std::env::var("PROBE_MAP_ID").map_or(0, |s| s.parse().unwrap());
    let at: Vec<f32> = std::env::var("PROBE_AT")
        .unwrap()
        .split(',')
        .map(|v| v.trim().parse().unwrap())
        .collect();
    let r: f32 = std::env::var("PROBE_R").map_or(60.0, |s| s.parse().unwrap());
    let zr: f32 = std::env::var("PROBE_ZR").map_or(10.0, |s| s.parse().unwrap());
    let centre = v(at[0], at[1], at[2]);
    let (_realm, _c, g, map) = mage_at(map_id, centre).await;
    let lines = map
        .query(move |m| {
            use rand::{RngExt, SeedableRng};
            let mut rng = rand::rngs::StdRng::seed_from_u64(7);
            let (mut under, mut corners) = (Vec::new(), Vec::new());
            for _ in 0..3000 {
                let Some(at) = spot(m, &mut rng, centre, r, zr) else {
                    continue;
                };
                let o = (rng.random_range(0..16) as f32) * std::f32::consts::TAU / 16.0;
                let want = v(at.x + 20.0 * o.cos(), at.y + 20.0 * o.sin(), at.z);
                let old = crate::game::map::spells::first_clear(m, at, want);
                let new = blink_end(m, g, at, o);
                let nav = m.nav.as_mut().unwrap();
                let dn = (new.x - at.x).hypot(new.y - at.y);
                let over = nav
                    .surfaces(new.x, new.y)
                    .map(|s| s.iter().copied().filter(|h| *h > new.z + 2.5 && *h < new.z + 9.0).fold(f32::MAX, f32::min))
                    .unwrap_or(f32::MAX);
                if dn > 15.0 && over < f32::MAX && (old.z - over).abs() < 0.3 && (new.z - at.z).abs() < 1.5 && under.len() < 8 {
                    under.push(format!(
                        "under: ({:.2}, {:.2}, {:.2}) o {o:.4}: old on {:.2} ({:+.2}), new {dn:.2} yd on {:.2} under {over:.2}",
                        at.x, at.y, at.z, old.z, old.z - at.z, new.z
                    ));
                }
                let (w, _) = first_model(nav, at, o, 20.0);
                if let Some(w) = w.filter(|w| (4.0..=12.0).contains(w)) {
                    let side = |nav: &mut Nav, k: f32| first_model(nav, at, o + k, 25.0).0;
                    let a = 20f32.to_radians();
                    if let (Some(l), Some(rr)) = (side(nav, -a), side(nav, a))
                        && l * a.cos() < w - 0.4
                        && rr * a.cos() < w - 0.4
                        && (new.z - at.z).abs() < 0.3
                        && corners.len() < 8
                    {
                        corners.push(format!(
                            "corner: ({:.2}, {:.2}, {:.2}) o {o:.4}: ahead {w:.2}, at -20 {l:.2}, at +20 {rr:.2}; new {dn:.2} yd dz {:+.2}, old {:.2} yd dz {:+.2}",
                            at.x, at.y, at.z, new.z - at.z, (old.x - at.x).hypot(old.y - at.y), old.z - at.z
                        ));
                    }
                }
            }
            under.extend(corners);
            under
        })
        .await
        .unwrap();
    for l in lines {
        println!("{l}");
    }
}

/// Spots before each closed door of a map where Blink goes through it open
/// and stops at it closed (run by hand): `PROBE_MAP_ID`, `PROBE_AT`.
#[tokio::test]
#[ignore = "a probe, run by hand"]
async fn probe_blink_doors() {
    let map_id: u32 = std::env::var("PROBE_MAP_ID").map_or(36, |s| s.parse().unwrap());
    let at: Vec<f32> = std::env::var("PROBE_AT")
        .unwrap()
        .split(',')
        .map(|v| v.trim().parse().unwrap())
        .collect();
    let (_realm, _c, g, map) = mage_at(map_id, v(at[0], at[1], at[2])).await;
    let lines = map
        .query(move |m| {
            use crate::game::fields::{GAMEOBJECT_DISPLAYID, GAMEOBJECT_STATE};
            let doors: Vec<(Guid, u32, Vector3d, f32, u32)> = m
                .gameobjects
                .iter()
                .filter(|(_, o)| o.kind == 0)
                .map(|(_, o)| (o.guid, o.entry, o.position, o.orientation, o.fields.get_u32(GAMEOBJECT_DISPLAYID)))
                .collect();
            let models = m.nav.as_ref().map_or(0, |n| n.obstacle_models.len());
            let mut out = vec![format!("{} doors, {models} models", doors.len())];
            for (door, entry, p, facing, display) in doors {
                let has = m.nav.as_ref().is_some_and(|n| n.obstacle_models.contains_key(&display));
                out.push(format!("door {entry} display {display} model {has} at ({:.2}, {:.2}, {:.2}) o {facing:.3}", p.x, p.y, p.z));
                let set = |m: &mut Map, s: u32| {
                    if let Some(o) = m.gameobject_mut(door) {
                        o.fields.set_u32(GAMEOBJECT_STATE, s);
                    }
                };
                let mut found = 0;
                for ring in [5.0f32, 7.0, 9.0] {
                    for k in 0..24 {
                        let a = k as f32 * std::f32::consts::TAU / 24.0;
                        let (x, y) = (p.x + ring * a.cos(), p.y + ring * a.sin());
                        let Some(z) = m.nav.as_mut().unwrap().ground(x, y, p.z) else {
                            continue;
                        };
                        if (z - p.z).abs() > 2.0 {
                            continue;
                        }
                        let from = v(x, y, z);
                        let o = (p.y - y).atan2(p.x - x);
                        set(m, 1);
                        let shut = blink_end(m, g, from, o);
                        set(m, 0);
                        let open = blink_end(m, g, from, o);
                        set(m, 1);
                        let d = |q: Vector3d| (q.x - from.x).hypot(q.y - from.y);
                        if d(open) > ring + 3.0 && d(shut) < ring - 0.4 && (open.z - z).abs() < 2.0 && found < 3 {
                            found += 1;
                            out.push(format!(
                                "    from ({x:.2}, {y:.2}, {z:.2}) o {o:.4}: shut {:.2} yd ({:.2}, {:.2}, {:.2}), open {:.2} yd ({:.2}, {:.2}, {:.2})",
                                d(shut), shut.x, shut.y, shut.z, d(open), open.x, open.y, open.z
                            ));
                        }
                    }
                }
            }
            out
        })
        .await
        .unwrap();
    for l in lines {
        println!("{l}");
    }
}

/// How long the bake's models for a continent's objects take to read, and
/// how many there are (run by hand).
#[test]
#[ignore = "a probe, run by hand"]
fn probe_obstacle_models() {
    let data = harness::load_real(&harness::real_data("db"));
    let dir = harness::real_data("nav");
    for map in [0u32, 1, 30, 36] {
        let displays: ahash::AHashSet<u32> = data
            .gameobject_spawns
            .get(&map)
            .into_iter()
            .flatten()
            .filter_map(|s| data.gameobject_templates.get(&s.entry))
            .map(|t| t.display_id)
            .collect();
        let t = std::time::Instant::now();
        let models = crate::game::map::obstacles::read(&dir, displays.iter().copied());
        println!(
            "map {map}: {} displays, {} models, {:?}",
            displays.len(),
            models.len(),
            t.elapsed()
        );
    }
}

/// What each of the way's looks costs, by its parts, over Elwynn's woods and
/// Stormwind (run by hand; the navmesh's own code is optimized in the
/// tests' build too).
#[test]
#[ignore = "a probe, run by hand"]
fn probe_blink_costs() {
    use rand::{RngExt, SeedableRng};
    let Some(mut nav) = nav("Azeroth") else {
        return;
    };
    for (label, c) in [
        ("elwynn", v(-9520.0, 20.0, 58.0)),
        ("stormwind", v(-8830.0, 640.0, 96.0)),
    ] {
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        let mut pts = Vec::new();
        while pts.len() < 400 {
            let (x, y) = (
                c.x + rng.random_range(-60.0..60.0),
                c.y + rng.random_range(-60.0..60.0),
            );
            if let Some(z) = nav.ground(x, y, c.z) {
                pts.push(v(x, y, z + 1.0));
            }
        }
        let time = |f: &mut dyn FnMut(Vector3d, Vector3d)| {
            let t = std::time::Instant::now();
            for w in pts.windows(2) {
                let b = v(w[0].x + 0.25, w[0].y, w[0].z);
                f(w[0], b);
            }
            t.elapsed() / (pts.len() - 1) as u32
        };
        let s = time(&mut |a, _| {
            let _ = nav.surfaces(a.x, a.y);
        });
        let cr = time(&mut |a, b| {
            let _ = nav.crossed(a, b);
        });
        let w = time(&mut |a, b| {
            let _ = nav.crossed_by(a, b, false);
        });
        let t = time(&mut |a, b| {
            let _ = nav.into_terrain(a, b);
        });
        let l = time(&mut |a, _| {
            let _ = nav.liquid_surface(a);
        });
        println!(
            "{label}: surfaces {s:?}, crossed {cr:?} (WMOs alone {w:?}), into_terrain {t:?}, liquid {l:?}"
        );
    }
}
