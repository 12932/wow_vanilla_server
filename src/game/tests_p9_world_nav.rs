//! Round 9's world tests of the navmesh and of where things are, ported
//! from Thistle Tea's `test/game/world/pathfinding_test.exs`,
//! `pathfinding_unloaded_test.exs`, `position_test.exs`,
//! `world_position_test.exs` and `area_effects_test.exs`: where a
//! creature's habitat lets it go in the sea and the air (asked of
//! `creature_motion::route` as its walks ask), a Blink under water and on
//! a map with no navmesh, a map's only zone, a creature in flight found
//! where it is, and a channel's areas, which go and are cut short
//! together.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::{Class, Vector3d};

use crate::game::Realm;
use crate::game::data::GameData;
use crate::game::data::spells::{ATTR_EX_CHANNELED, SpellEffect};
use crate::game::harness;
use crate::game::map::creature::Motion;
use crate::game::map::creature_motion::{route, water};
use crate::game::map::spells::{self, Targets, dynamic};
use crate::game::map::{Map, MapKey};
use crate::game::tests_client_emu::{spawn, template};
use crate::game::tests_p3_cast_kit::{KOBOLD, LASTS_15S, bare, effect, fast_realm, human, insert};
use crate::game::tests_p3_spells_kit::{self as kit, cast_fresh};

static BAKE: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| harness::real_data("nav"));
static TERRAIN: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| harness::real_data("terrain"));

fn v(x: f32, y: f32, z: f32) -> Vector3d {
    Vector3d { x, y, z }
}

/// A human's start in Northshire, in front of the abbey.
const START: Vector3d = Vector3d {
    x: -8949.95,
    y: -132.49,
    z: 83.53,
};
/// Five yards under the sea where Thistle Tea fishes, and five yards on
/// from there.
const SEA: Vector3d = Vector3d {
    x: -2183.26,
    y: -1867.59,
    z: -5.0,
};
const SEA_ON: Vector3d = Vector3d {
    x: -2178.26,
    y: -1867.59,
    z: -3.0,
};

/// Made-up creatures that walk only, swim only (without the swimming
/// animation), and fly.
const WALKER: u32 = 997_200;
const SWIMMER: u32 = 997_201;
const FLYER: u32 = 997_202;

/// A realm on the real navmesh and terrain, with the made-up creatures in
/// Northshire (`route` asks only their habitats); none without the bake.
fn nav_realm() -> Option<Arc<Realm>> {
    if !BAKE.join("Azeroth.map").exists() || !TERRAIN.exists() {
        return None;
    }
    let mut d = GameData::empty();
    for (entry, inhabit) in [(WALKER, 1), (SWIMMER, 2), (FLYER, 4)] {
        let mut t = template(entry, "Made-up", 7);
        t.inhabit_type = inhabit;
        d.creature_templates.insert(entry, t);
    }
    d.creature_spawns.insert(
        0,
        [WALKER, SWIMMER, FLYER]
            .into_iter()
            .zip(1..)
            .map(|(e, i)| spawn(i, e, (START.x + i as f32, START.y, START.z)))
            .collect(),
    );
    let mut cfg = harness::test_config();
    cfg.nav_dir = Some(BAKE.clone());
    cfg.terrain_dir = Some(TERRAIN.clone());
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Some(Realm::new(cfg, store, startup, d, None))
}

fn of(m: &Map, entry: u32) -> Guid {
    m.creatures
        .iter()
        .find(|(_, c)| c.entry == entry)
        .map(|(_, c)| c.guid)
        .expect("the creature")
}

fn wet(m: &mut Map, p: Vector3d) -> bool {
    water(m, p).is_some()
}

/// Thistle Tea's `find_path/4` "land-only creatures reject submerged
/// targets and water needs enough depth", "land-only creatures walk to
/// the water's edge toward a submerged target" and "rejects a detour that
/// takes a water-only creature above the surface": a creature that only
/// walks has no way from under the sea; the sea there is water, between
/// five and twenty yards deep (wvs's water is any deeper than a yard and a
/// half, `creature_motion::water`, where Thistle Tea's takes a depth), and
/// under its floor there is none; from the shore toward a point under
/// the sea a walker goes as far as the water's edge, every point dry; one
/// that only swims keeps to the water all its way.
#[tokio::test]
#[ignore = "extended"]
async fn walkers_keep_out_of_the_sea_and_swimmers_in_it() {
    let Some(realm) = nav_realm() else {
        return;
    };
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(|m| {
            let (walker, swimmer) = (of(m, WALKER), of(m, SWIMMER));
            let from_sea = route(m, walker, SEA, SEA_ON).is_none();
            let depth = water(m, SEA).map(|(surface, floor)| surface - floor);
            let under = wet(m, v(SEA.x, SEA.y, -20.0));
            let shore = route(
                m,
                walker,
                v(-2183.26, -1907.59, 4.16),
                v(-2183.26, -1857.59, -3.0),
            )
            .map(|(p, reached, _)| {
                let dry = p.iter().all(|q| !wet(m, *q));
                (p.len(), reached, dry, p.last().unwrap().y)
            });
            let swim = route(m, swimmer, SEA, SEA_ON)
                .map(|(p, reached, _)| (reached, p.iter().all(|q| wet(m, *q))));
            (from_sea, depth, under, shore, swim)
        })
        .await
        .unwrap();
    let (from_sea, depth, under, shore, swim) = out;
    assert!(from_sea, "no way for a walker from under the sea");
    let depth = depth.expect("the sea is water");
    assert!(depth > 5.0 && depth < 20.0, "its depth: {depth}");
    assert!(!under, "none under its floor");
    let (len, reached, dry, y) = shore.expect("a way to the water's edge");
    assert!(len >= 2 && !reached, "as far as the edge");
    assert!(dry, "every point dry");
    assert!(y > -1890.0, "to the edge: {y}");
    assert_eq!(swim, Some((true, true)), "the swimmer keeps to the water");
}

/// Thistle Tea's `find_path/4` "does not fly straight through an
/// obstructing wall": a flyer from beside Northshire Abbey to a point
/// through its wall doesn't fly straight there, and any way it takes has
/// every leg in the clear (wvs's flyer has none here: the navmesh's way
/// round goes through the doorway, a leg its line of sight clips, so it
/// stays). Of `world_position_test.exs`'s `grounded_target_position/2`,
/// "leaves a grounded unit's position alone": a walker's way to a point on
/// the ground ends at that point.
#[tokio::test]
#[ignore = "extended"]
async fn a_flyer_doesnt_fly_through_the_abbeys_wall() {
    let Some(realm) = nav_realm() else {
        return;
    };
    let map = realm.maps.get(&realm, MapKey::world(0));
    let (from, to) = (v(-8930.0, -150.0, 84.0), v(-8910.0, -150.0, 84.0));
    let ground = v(START.x - 6.0, START.y, START.z);
    let (flight, walk) = map
        .query(move |m| {
            let (flyer, walker) = (of(m, FLYER), of(m, WALKER));
            let flight = route(m, flyer, from, to).map(|(p, _, fly)| {
                let n = m.nav.as_mut().unwrap();
                let clear = p.windows(2).all(|w| n.line_of_sight(w[0], w[1]));
                (p, clear, fly)
            });
            let walk = route(m, walker, START, ground).map(|(p, reached, _)| (p, reached));
            (flight, walk)
        })
        .await
        .unwrap();
    if let Some((p, clear, fly)) = flight {
        assert!(p.len() > 2, "not straight through the wall: {p:?}");
        assert!(clear && fly, "every leg flown in the clear: {p:?}");
    }
    let (p, reached) = walk.expect("a walk on the ground");
    assert!(reached);
    assert_eq!(p.last(), Some(&ground), "its place as it is");
}

const BLINK: u32 = 997_210;
const R20: u32 = 997;

fn blink(d: &mut GameData) {
    d.spells.radii.insert(R20, 20.0);
    let mut s = bare(BLINK, 0);
    effect(
        &mut s,
        0,
        SpellEffect {
            effect: spells::EFFECT_LEAP,
            target_a: 1,
            target_b: 55,
            radius_index: R20,
            ..Default::default()
        },
    );
    insert(d, [s]);
}

/// `world_position_test.exs`'s `grounded_target_position/2` "keeps an
/// airborne unit's position when the map has no navmesh": with no
/// navmesh, a creature's way to a point thirty yards up goes straight
/// there, and a Blink there goes its twenty yards at its height.
#[tokio::test]
async fn with_no_navmesh_a_place_in_the_air_keeps_its_height() {
    let realm = fast_realm(blink);
    let (_c, a, map) = human(&realm, "P9WN2", "Floater", Class::Mage).await;
    let high = v(START.x, START.y, START.z + 30.0);
    let (way, at) = map
        .query(move |m| {
            let g = kit::dummy(m, a, 60, 5.0);
            let from = m.creature(g).unwrap().position;
            let way = route(m, g, from, high).map(|(p, _, _)| p);
            kit::ready(m, a, 60, &[BLINK]);
            m.near_teleport(a, high, std::f32::consts::PI);
            assert!(cast_fresh(m, a, BLINK, Targets::default()));
            (
                way.map(|p| (p.len(), p.last().copied())),
                m.players[&a].position(),
            )
        })
        .await
        .unwrap();
    assert_eq!(way, Some((2, Some(high))), "straight up to it");
    assert!(
        (at.x - (high.x - 20.0)).abs() < 0.05 && (at.y - high.y).abs() < 0.05,
        "twenty yards on: {at:?}"
    );
    assert_eq!(at.z, high.z, "at its height");
}

/// `position_test.exs`'s `distance_between/3` "exact queries do not
/// discard projected vertical movement by its stale distance" and
/// `position_snapshot/3` "uses recent movement from the supplied
/// observation": a creature flying straight down is found, halfway, by a
/// search about the point halfway down, not about where it set out; it is
/// on the move until its flight is over, and not after (wvs keeps one
/// place for each creature, moved on by its walk at each update, and a
/// player's moving is their client's movement flags; neither has a
/// window of recent movement).
#[tokio::test]
async fn a_creature_flying_down_is_where_its_flight_has_it() {
    use crate::game::map::scripts::creatures_near;
    let realm = fast_realm(|_| {});
    let (_c, a, map) = human(&realm, "P9WN3", "Watcher", Class::Warrior).await;
    let out = map
        .query(move |m| {
            let g = kit::dummy(m, a, 60, 5.0);
            let ground = m.creature(g).unwrap().position;
            let up = |z: f32| Vector3d {
                z: ground.z + z,
                ..ground
            };
            let (now, id) = (m.now, m.next_spline_id);
            let c = m.creature_mut(g).unwrap();
            c.position = up(20.0);
            c.walk_at(
                vec![up(20.0), ground],
                false,
                true,
                Some(Duration::from_secs(1)),
                now,
                id,
            );
            m.now += Duration::from_millis(500);
            crate::game::map::ai::advance_motion(m, g);
            let halfway = creatures_near(m, up(10.0), KOBOLD, 1.0, true) == vec![g];
            let set_out = creatures_near(m, up(20.0), KOBOLD, 1.0, true).is_empty();
            let moving = matches!(m.creature(g).unwrap().motion, Motion::Moving(_));
            m.now += Duration::from_millis(500);
            crate::game::map::ai::advance_motion(m, g);
            let landed = creatures_near(m, ground, KOBOLD, 1.0, true) == vec![g];
            let still = matches!(m.creature(g).unwrap().motion, Motion::Idle { .. });
            (halfway, set_out, moving, landed, still)
        })
        .await
        .unwrap();
    assert_eq!(out, (true, true, true, true, true));
}

const AREAS: u32 = 997_211;

/// Thistle Tea's `AreaEffects.pids/2` "returns every process registered
/// under the same key", the areas of a caster's spell that its channel
/// ends or cuts short together: a channel with two areas on the ground,
/// pushed back, cuts both short by as much; ended, it takes both away. An
/// area of its spell another caster put down, and one of another spell of
/// its caster's, stay as they were (mangos's `DynamicObject::Delay` and
/// its channel's `RemoveDynObject`; `effects::delay_areas`,
/// `dynamic::remove_spell`).
#[tokio::test]
async fn a_channels_areas_go_and_are_cut_short_together() {
    let realm = fast_realm(|d| {
        let mut s = bare(AREAS, 4);
        s.attributes[1] |= ATTR_EX_CHANNELED;
        s.channel_interrupt_flags = 0x4000;
        s.duration_index = LASTS_15S;
        let area = SpellEffect {
            effect: spells::EFFECT_PERSISTENT_AREA_AURA,
            aura: crate::game::map::auras::AURA_PERIODIC_DAMAGE,
            base_points: 9,
            base_dice: 1,
            amplitude: 1_000,
            target_a: 1,
            ..Default::default()
        };
        effect(&mut s, 0, area);
        effect(&mut s, 1, area);
        insert(d, [s]);
    });
    let (_c, a, map) = human(&realm, "P9WN4", "Channeler", Class::Mage).await;
    let out = map
        .query(move |m| {
            let kobold = kit::dummy(m, a, 60, 5.0);
            kit::ready(m, a, 60, &[AREAS]);
            assert!(cast_fresh(m, a, AREAS, Targets::default()));
            assert_eq!(kit::casting(m, a), Some(AREAS), "it channels");
            let at = m.players[&a].position();
            let theirs = dynamic::spawn(m, kobold, AREAS, dynamic::KIND_AREA_SPELL, at, 5.0, None);
            let other = dynamic::spawn(m, a, AREAS + 1, dynamic::KIND_AREA_SPELL, at, 5.0, None);
            let ends = |m: &Map| {
                let mut e: Vec<_> = m
                    .spells
                    .dynamic
                    .values()
                    .filter(|d| d.caster == a && d.spell == AREAS)
                    .map(|d| d.ends)
                    .collect();
                e.sort();
                e
            };
            let before = ends(m);
            spells::pushback_by(m, a, kobold);
            let after = ends(m);
            let cuts: Vec<_> = before
                .iter()
                .zip(&after)
                .map(|(b, a)| b.unwrap() - a.unwrap())
                .collect();
            spells::interrupt(m, a);
            let left = ends(m).len();
            let kept = [theirs, other].map(|g| m.spells.dynamic.get(&g).map(|d| d.ends));
            (before.len(), cuts, left, kept)
        })
        .await
        .unwrap();
    let (two, cuts, left, kept) = out;
    assert_eq!(two, 2, "two areas of the channel");
    assert_eq!(
        cuts,
        vec![Duration::from_secs(1); 2],
        "both a second sooner"
    );
    assert_eq!(left, 0, "both gone with the channel");
    assert_eq!(kept, [Some(None), Some(None)], "the others as they were");
}
