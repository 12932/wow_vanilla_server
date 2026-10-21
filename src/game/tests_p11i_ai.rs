//! Round 11's test-only rows on creatures' footwork (Thistle Tea's
//! `formation_environment_test`, `aquatic_movement_test` and
//! `bt/navigation_test`): a formation that waits out its leader's fight
//! and its own stun, a swimmer's partial route keeping its clock until its
//! victim is in reach, and a chase with no route going nowhere, the last
//! two on the real navmesh and terrain.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::harness::{self, Client};
use crate::game::map::creature::Motion;
use crate::game::map::{Map, MapHandle, MapKey, ai, combat, grid};

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map answers")
}

fn moving(m: &Map, g: Guid) -> bool {
    matches!(m.creature(g).map(|c| &c.motion), Some(Motion::Moving(_)))
}

/// Thistle Tea's "reads published leader motion and excludes combat and
/// other worlds": a formation follows its leader's walk
/// (`creature_groups::leader_walked`), but not while the leader fights
/// (it chases, it leads no one), nor a follower that is stunned or rooted
/// (`free_to_follow`); free again, it follows. Another world can't be in
/// a map actor. The path and the timing are
/// `tests_parity_motion::a_formation_follows_the_last_leg_and_arrives_with_its_leader`.
#[tokio::test]
async fn a_formation_waits_out_its_leaders_fight_and_its_own_stun() {
    use crate::game::map::auras::{AURA_MOD_ROOT, AURA_MOD_STUN};
    use crate::game::tests_parity_motion::{at, enter, follower, group_world, leader};
    let (_realm, mut a, _ga, map) = enter(group_world(0)).await;
    let (l, f) = (leader(), follower());
    a.sees(f).await;
    let walk = move |m: &mut Map, to: Vector3d, id: u32| {
        let now = m.now;
        for g in [l, f] {
            m.creature_mut(g).unwrap().motion = Motion::Idle { until: None };
        }
        let c = m.creature_mut(l).unwrap();
        let from = c.position;
        c.walk(vec![from, to], false, now, id);
        crate::game::map::creature_groups::leader_walked(m, l);
        moving(m, f)
    };
    let out = on(&map, move |m| {
        // The leader in a fight.
        m.creature_mut(l).unwrap().combat.attacking = true;
        let fighting = walk(m, at(20.0, -30.0), 1);
        m.creature_mut(l).unwrap().combat.attacking = false;
        // The follower stunned, then rooted.
        crate::game::tests_p3_combat::aura_of(m, f, 95_310, AURA_MOD_STUN, 0, 0);
        let stunned = walk(m, at(20.0, -25.0), 2);
        crate::game::map::auras::remove_spell(m, f, 95_310, None);
        crate::game::tests_p3_combat::aura_of(m, f, 95_311, AURA_MOD_ROOT, 0, 0);
        let rooted = walk(m, at(20.0, -30.0), 3);
        crate::game::map::auras::remove_spell(m, f, 95_311, None);
        // Free: it follows.
        let free = walk(m, at(20.0, -25.0), 4);
        (fighting, stunned, rooted, free)
    })
    .await;
    assert_eq!(
        out,
        (false, false, false, true),
        "(fighting, stunned, rooted, free)"
    );
}

const FISH: u32 = 3_970;

/// The open sea off Stranglethorn's west coast, two yards under its
/// surface, and the dry headland ninety yards east of it.
const SEA: (f32, f32, f32) = (-14_930.0, 64.0, -2.0);
const HEADLAND: (f32, f32) = (-14_840.0, 64.0);

fn v((x, y, z): (f32, f32, f32)) -> Vector3d {
    Vector3d { x, y, z }
}

fn sea_world() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    let mut fish = crate::game::tests_p3_combat::template(FISH, 1);
    fish.inhabit_type = 2;
    fish.static_flags1 = 0x1000_0000;
    d.creature_templates.insert(FISH, fish);
    d
}

/// Alice in a realm of the real navmesh and terrain, at `to`, its tiles
/// read.
async fn ashore(d: GameData, to: Vector3d) -> (Client, Guid, MapHandle) {
    let mut cfg = harness::test_config();
    cfg.terrain_dir = Some(harness::real_data("terrain"));
    cfg.nav_dir = Some(harness::real_data("nav"));
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = crate::game::Realm::new(cfg, store, startup, d, None);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    on(&map, move |m| m.teleport(g, m.key, to, 0.0)).await;
    a.wait_for("MSG_MOVE_TELEPORT_ACK", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: g,
        movement_counter: a.teleport_order(),
        time: Default::default(),
    })
    .await;
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

/// Thistle Tea's "a partial route retains the timeout until the victim
/// enters melee contact": a fish after a player on dry land swims only as
/// far as the water goes (`creature_motion::route`, a partial route), so
/// its clock of a victim out of reach starts
/// (`creature_combat::reach`) and keeps its start as it chases on; the
/// player come into the water within its reach, the clock is gone.
#[tokio::test]
#[ignore = "extended"]
async fn a_swimmers_partial_route_keeps_its_clock_until_its_victim_is_in_reach() {
    let (_a, g, map) = ashore(sea_world(), v((HEADLAND.0, HEADLAND.1, 10.0))).await;
    let out = on(&map, move |m| {
        let gz = m
            .nav
            .as_mut()
            .unwrap()
            .ground(HEADLAND.0, HEADLAND.1, 30.0)
            .unwrap();
        put_player(m, g, v((HEADLAND.0, HEADLAND.1, gz)));
        let fish = crate::game::tests_p3_combat::spawn(m, g, FISH, 20, 3.0);
        put_creature(m, fish, v(SEA));
        let partial = crate::game::map::creature_motion::route(
            m,
            fish,
            v(SEA),
            v((HEADLAND.0, HEADLAND.1, gz)),
        )
        .map(|r| r.1);
        ai::attacked(m, fish, g);
        turns(m, 2);
        let since = |m: &Map| m.creature(fish).unwrap().engagement.unreachable_since;
        let started = since(m);
        turns(m, 20);
        let kept = since(m);
        // Into the water, by the fish.
        let by = m.creature(fish).unwrap().position;
        put_player(
            m,
            g,
            Vector3d {
                x: by.x + 1.0,
                ..by
            },
        );
        turns(m, 4);
        (partial, started, kept, since(m))
    })
    .await;
    let (partial, started, kept, contact) = out;
    assert_eq!(partial, Some(false), "a partial route to her");
    assert!(started.is_some(), "the clock started at the partial route");
    assert_eq!(kept, started, "kept from its start as the chase goes on");
    assert_eq!(contact, None, "gone at contact");
}

/// A meadow by Elwynn's south lake.
const MEADOW: (f32, f32) = (-9_620.0, -1_040.0);

/// Thistle Tea's "drains a request when no path is available", as mangos's
/// `TargetedMovementGenerator` does nothing on `PATHFIND_NOPATH`: a brute
/// after a victim the navmesh has no route to (thirty yards under the
/// meadow) walks nowhere, and its clock of a victim out of reach starts.
#[tokio::test]
#[ignore = "extended"]
async fn a_chase_with_no_route_goes_nowhere() {
    use crate::game::tests_p3_combat::BRUTE;
    let (_a, g, map) = ashore(sea_world(), v((MEADOW.0, MEADOW.1, 44.0))).await;
    let out = on(&map, move |m| {
        // Where the teleport left her, and thirty yards under the meadow
        // ten yards on.
        let start = m.players[&g].position();
        let brute = crate::game::tests_p3_combat::spawn(m, g, BRUTE, 20, 3.0);
        let here = m.creature(brute).unwrap().position;
        put_player(
            m,
            g,
            Vector3d {
                x: start.x + 10.0,
                z: start.z - 30.0,
                ..start
            },
        );
        let to = m.players[&g].position();
        let routed = crate::game::map::creature_motion::route(m, brute, here, to);
        ai::attacked(m, brute, g);
        turns(m, 10);
        let c = m.creature(brute).unwrap();
        (
            routed.is_none(),
            combat::dist(c.position, here),
            moving(m, brute),
            c.engagement.unreachable_since.is_some(),
        )
    })
    .await;
    let (no_route, moved, walking, clock) = out;
    assert!(no_route, "no route to her");
    assert!(moved < 0.01 && !walking, "it goes nowhere: {moved}");
    assert!(clock, "out of reach");
}
