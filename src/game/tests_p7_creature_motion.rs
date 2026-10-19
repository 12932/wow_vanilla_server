//! Round 7's creature tests, ported from Thistle Tea's
//! `test/game/core/creature` (docs/parity/matrix.csv): where pets and
//! charmed creatures may go, a flyer's chase and way home through the air,
//! its circle and cyclic path under roots and in a fight, a corpse's fall,
//! and a wounded creature made whole as it leaves its fight.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::AURA_MOD_ROOT;
use crate::game::map::creature::Motion;
use crate::game::map::creature_motion::{self, FLIGHT_FLAGS, INHABIT_AIR, INHABIT_WATER};
use crate::game::map::pets::{Kind, PetState};
use crate::game::tests_guards::hardy;
use crate::game::tests_p3_combat::{START, aura_of, data, setup_with, spawn};
use crate::game::tests_p3_entity::on;
use crate::game::tests_p3_entity_objects::until;

const FISH: u32 = 4_760;
const BAT: u32 = 4_761;
const WOLF: u32 = 4_762;
/// Spawns: a circling bat, a bat on a cyclic path, and one rooted.
const CIRCLER: u32 = 4_760;
const CYCLER: u32 = 4_761;
const ROOTED: u32 = 4_762;
const PINNED: u32 = 94_760;

fn world() -> GameData {
    let mut d = data();
    for (entry, inhabit) in [(FISH, INHABIT_WATER), (BAT, INHABIT_AIR), (WOLF, 0)] {
        let mut t = crate::game::tests_p3_combat::template(entry, 1);
        t.inhabit_type = inhabit;
        t.health_max = 1_000;
        t.speed_run = 1.0;
        t.speed_walk = 1.0;
        d.creature_templates.insert(entry, t);
    }
    let high = |dx: f32, dy: f32| (START.0 + dx, START.1 + dy, START.2 + 20.0);
    let spawn = |guid: u32, at: (f32, f32, f32), movement_type: u8| CreatureSpawn {
        guid,
        entries: vec![BAT],
        map: 0,
        x: at.0,
        y: at.1,
        z: at.2,
        movement_type,
        wander_distance: 10.0,
        ..Default::default()
    };
    d.creature_spawns.insert(
        0,
        vec![
            spawn(CIRCLER, high(30.0, 0.0), 1),
            spawn(CYCLER, high(-30.0, 0.0), 3),
            spawn(ROOTED, high(0.0, 40.0), 1),
        ],
    );
    d.waypoints.insert(
        CYCLER,
        (0..4)
            .map(|i| Waypoint {
                x: START.0 - 30.0 + 8.0 * (i as f32).cos(),
                y: START.1 + 8.0 * (i as f32).sin(),
                z: START.2 + 20.0,
                ..Default::default()
            })
            .collect(),
    );
    d
}

fn of(spawn: u32) -> Guid {
    crate::game::guid::unit(BAT, spawn)
}

/// The walk under way: whether it flies, and its points.
fn walk(m: &Map, g: Guid) -> Option<(bool, Vec<Vector3d>)> {
    match &m.creature(g)?.motion {
        Motion::Moving(w) => Some((w.fly, w.points.clone())),
        _ => None,
    }
}

/// Thistle Tea's "combat pets can travel between habitats while charmed
/// creatures retain their restrictions": a fish made a hunter's pet or a
/// summoned demon walks and swims; charmed, it is a fish still.
#[tokio::test]
async fn a_combat_pet_goes_anywhere_and_a_charmed_creature_where_it_could() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let out = on(&map, move |m| {
        let now = m.now;
        [Kind::Hunter, Kind::Demon, Kind::Charmed].map(|kind| {
            let g = spawn(m, ga, FISH, 20, 3.0);
            m.creature_mut(g).unwrap().pet = Some(Box::new(PetState::new(kind, ga, now)));
            let h = creature_motion::habitat(m, g);
            (h.walk, h.swim)
        })
    })
    .await;
    assert_eq!(out, [(true, true), (true, true), (false, true)]);
}

/// Thistle Tea's "requests airborne navigation for chase and home
/// movement": a flyer goes after its victim through the air, and leaving
/// its fight flies home.
#[tokio::test]
async fn a_flyer_chases_and_goes_home_through_the_air() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let g = of(CIRCLER);
    a.sees(g).await;
    on(&map, move |m| {
        hardy(m, ga);
        crate::game::map::ai::attacked(m, g, ga);
    })
    .await;
    until(&map, "the chase", move |m| {
        m.creature(g).unwrap().chase_target_at.is_some()
    })
    .await;
    let chase = on(&map, move |m| walk(m, g)).await;
    assert_eq!(chase.map(|w| w.0), Some(true), "chasing through the air");
    let home = on(&map, move |m| {
        crate::game::map::ai::enter_evade(m, g);
        walk(m, g).map(|w| w.0)
    })
    .await;
    assert_eq!(home, Some(true), "home through the air");
}

/// Thistle Tea's "approaches the first node through navigation after an
/// interruption": a flyer on a cyclic path, far from it, makes its way to
/// the path's first point before it goes round.
#[tokio::test]
async fn a_cyclic_flyer_far_from_its_path_goes_to_its_first_point() {
    let (_realm, mut a, _ga, map) = setup_with(world(), Class::Warrior).await;
    let g = of(CYCLER);
    a.sees(g).await;
    let first = on(&map, move |m| {
        let now = m.now;
        let c = m.creature_mut(g).unwrap();
        c.position.y += 30.0;
        c.motion = Motion::Idle { until: Some(now) };
        let w = m.realm.data.waypoints[&CYCLER][0];
        Vector3d {
            x: w.x,
            y: w.y,
            z: w.z,
        }
    })
    .await;
    until(&map, "a flight", move |m| walk(m, g).is_some()).await;
    let (fly, points) = on(&map, move |m| walk(m, g)).await.unwrap();
    assert!(fly);
    let end = *points.last().unwrap();
    assert!(
        crate::game::map::combat::dist(end, first) < 0.01,
        "to its first point: {end:?} for {first:?}"
    );
}

/// Thistle Tea's "roots block both cyclic routes and idle flight circles":
/// a flyer rooted from the start doesn't go round.
#[tokio::test]
async fn a_rooted_flyer_doesnt_circle() {
    let (_realm, mut a, _ga, map) = setup_with(world(), Class::Warrior).await;
    let (circler, rooted) = (of(CIRCLER), of(ROOTED));
    a.sees(rooted).await;
    on(&map, move |m| {
        aura_of(m, rooted, PINNED, AURA_MOD_ROOT, 0, 0);
    })
    .await;
    until(&map, "the free one circling", move |m| {
        walk(m, circler).is_some()
    })
    .await;
    let from = on(&map, |m| m.stats.updates).await;
    until(&map, "ten updates more", move |m| {
        m.stats.updates >= from + 10
    })
    .await;
    let held = on(&map, move |m| walk(m, rooted)).await;
    assert!(held.is_none(), "rooted, it stays");
}

/// Thistle Tea's "combat interrupts circles at their current position": a
/// circling flyer drawn into a fight leaves its lap from where it is along
/// it, still in the air.
#[tokio::test]
async fn a_fight_takes_a_flyer_from_its_circle_where_it_is() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let g = of(CIRCLER);
    a.sees(g).await;
    until(&map, "the circle", move |m| walk(m, g).is_some()).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let lap_end = on(&map, move |m| {
        hardy(m, ga);
        let end = *walk(m, g).unwrap().1.last().unwrap();
        crate::game::map::ai::attacked(m, g, ga);
        end
    })
    .await;
    until(&map, "the chase", move |m| {
        m.creature(g).unwrap().chase_target_at.is_some()
    })
    .await;
    let (fly, points, flags) = on(&map, move |m| {
        let (fly, points) = walk(m, g).unwrap();
        (fly, points, m.creature(g).unwrap().move_flags)
    })
    .await;
    assert!(fly && flags & FLIGHT_FLAGS != 0, "in the air");
    assert!(
        crate::game::map::combat::dist(points[0], lap_end) > 0.5,
        "from where it was, not its lap's end"
    );
}

/// Thistle Tea's `land_corpse` with nothing under it: a flyer killed where
/// no ground is known stays where it died, not falling.
#[tokio::test]
async fn a_flyer_killed_over_no_known_ground_stays_where_it_died() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let g = of(ROOTED);
    a.sees(g).await;
    let out = on(&map, move |m| {
        let at = m.position_of(g).unwrap();
        crate::game::map::combat::deal_damage(m, ga, g, 1_000_000);
        let c = m.creature(g).unwrap();
        (
            c.alive(),
            crate::game::map::combat::dist(c.position, at),
            matches!(c.motion, Motion::Moving(_)),
        )
    })
    .await;
    assert_eq!(out, (false, 0.0, false));
}

/// Thistle Tea's "limits long falls to terminal velocity": a long fall's
/// last stretch goes at terminal velocity, 60.148 yards a second.
#[test]
fn a_long_fall_goes_no_faster_than_terminal_velocity() {
    let (a, b) = (
        creature_motion::fall_time(400.0),
        creature_motion::fall_time(460.148),
    );
    assert!((b - a - 1.0).abs() < 0.002, "{a} then {b}");
    // A short one under gravity: 19.29 yards a second squared.
    let t = creature_motion::fall_time(20.0);
    assert!(
        (t - (2.0f32 * 20.0 / 19.291_105).sqrt()).abs() < 0.002,
        "{t}"
    );
}

/// Thistle Tea's "restores health and speed before returning home" (and
/// mangos's evade): a wounded creature leaving its fight is whole at once,
/// its run its own and its wounds' aura state gone, as it starts home.
#[tokio::test]
async fn a_wounded_creature_is_whole_as_it_leaves_its_fight() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let out = on(&map, move |m| {
        hardy(m, ga);
        let g = spawn(m, ga, WOLF, 20, 3.0);
        let whole = m.creature(g).unwrap().speeds.run;
        crate::game::map::ai::attacked(m, g, ga);
        crate::game::map::combat::deal_damage(m, ga, g, 99_000);
        let hurt = m.creature(g).unwrap().speeds.run;
        if let Some(c) = m.creature_mut(g) {
            c.position.x += 10.0;
        }
        crate::game::map::ai::enter_evade(m, g);
        let c = m.creature(g).unwrap();
        (
            hurt < whole,
            c.fields.get_u32(UNIT_FIELD_HEALTH) == c.fields.get_u32(UNIT_FIELD_MAXHEALTH),
            (c.speeds.run - whole).abs() < 0.01,
            c.fields.get_u32(UNIT_FIELD_AURASTATE) & 0x700,
            c.evading,
        )
    })
    .await;
    assert_eq!(out, (true, true, true, 0, true));
}
