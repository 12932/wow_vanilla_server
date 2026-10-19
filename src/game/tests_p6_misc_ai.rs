//! Round 6's tests of creatures, ported from Thistle Tea's
//! `test/game/core/entity/component/internal/waypoint_route_test.exs` and
//! others (docs/parity/matrix.csv): a path's start, its round and its end.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::Waypoint;
use crate::game::map::Map;
use crate::game::map::event_ai;
use crate::game::tests_p3_combat::{BRUTE, START, setup_with, spawn};
use crate::game::tests_p3_entity::{on, step};

fn point(point: u32, east: f32, wait_ms: u32) -> Waypoint {
    Waypoint {
        point,
        x: START.0 + east,
        y: START.1 + 5.0,
        z: START.2,
        wait_ms,
        ..Default::default()
    }
}

/// Where the creature is bound on its path, and whether it goes round.
fn bound(m: &Map, g: Guid) -> (usize, Option<bool>) {
    let c = m.creature(g).unwrap();
    (
        c.waypoint,
        event_ai::scripted(m, g).and_then(|s| s.route.as_ref().map(|r| r.repeat)),
    )
}

/// Wait until `pick` holds of the map, or fail.
async fn until(
    map: &crate::game::map::MapHandle,
    mut pick: impl FnMut(&Map) -> bool + Send + Copy + 'static,
) {
    let deadline = tokio::time::Instant::now() + crate::game::harness::WAIT;
    while !on(map, move |m| pick(m)).await {
        assert!(tokio::time::Instant::now() < deadline, "never came");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Thistle Tea's `WaypointRoute.start` "selects an explicit start point and
/// repeat mode" and "falls back to the first point for zero or invalid
/// starts", and `increment_waypoint` "wraps to first_point when at end",
/// "wraps when next point does not exist" and "stops a non-repeating route
/// after its final point": a script's path (START_WAYPOINTS) starts at the
/// point it names, once or round as it says; at its first for none or one
/// it hasn't; round, it is bound for its first after its last; once, it
/// stops at its last.
#[tokio::test]
async fn a_path_starts_where_asked_goes_round_and_once_ends() {
    let mut d = crate::game::tests_p3_combat::data();
    // Round: its last point holds it a minute, to be looked at.
    d.waypoints_special.insert(
        BRUTE,
        vec![point(1, 3.0, 0), point(2, 6.0, 0), point(3, 9.0, 60_000)],
    );
    d.waypoints_by_entry.insert(
        BRUTE,
        vec![point(1, 3.0, 0), point(2, 6.0, 0), point(3, 9.0, 0)],
    );
    let (_realm, mut a, ga, map) = setup_with(d, Class::Warrior).await;
    let brute = on(&map, move |m| {
        let g = spawn(m, ga, BRUTE, 20, 3.0);
        let c = m.creature_mut(g).unwrap();
        c.speeds.walk = 60.0;
        c.speeds.run = 60.0;
        g
    })
    .await;
    a.sees(brute).await;
    let run = move |m: &mut Map, datalong: [u32; 4]| {
        crate::game::map::scripts::execute(m, &step(60, datalong), brute, Guid::zero());
        bound(m, brute)
    };
    let starts = on(&map, move |m| {
        [
            run(m, [3, 2, 0, 0]),
            run(m, [3, 0, 0, 1]),
            run(m, [3, 99, 0, 1]),
        ]
    })
    .await;
    assert_eq!(
        starts,
        [(1, Some(false)), (0, Some(true)), (0, Some(true))],
        "point 2 once; the first, round, for none or one it hasn't"
    );
    // Round from its last point: bound for its first.
    on(&map, move |m| run(m, [3, 3, 0, 1])).await;
    until(&map, move |m| {
        m.creature(brute).unwrap().last_waypoint == Some(2)
    })
    .await;
    assert_eq!(on(&map, move |m| bound(m, brute)).await, (0, Some(true)));
    // Once, from point 2: it stops at its last, its path over.
    on(&map, move |m| run(m, [2, 2, 0, 0])).await;
    until(&map, move |m| {
        let c = m.creature(brute).unwrap();
        c.movement_type == 0 && event_ai::scripted(m, brute).is_none_or(|s| s.route.is_none())
    })
    .await;
    let (last, at, still) = on(&map, move |m| {
        let c = m.creature(brute).unwrap();
        (
            c.last_waypoint,
            c.position,
            matches!(
                c.motion,
                crate::game::map::creature::Motion::Idle { until: None }
            ),
        )
    })
    .await;
    assert_eq!(last, Some(2));
    assert!(still, "still");
    assert!(
        (at.x - (START.0 + 9.0)).abs() < 0.5,
        "at its last point: {at:?}"
    );
}

/// What a brute becomes: a level-40 creature of another faction, look,
/// size, swing and fire resistance, with no spell list.
const NEW: u32 = 3_790;
/// The brute's spawn.
const SPAWN: u32 = 77;

fn entry_data() -> crate::game::data::GameData {
    use crate::game::data::{ClassLevelStats, CreatureSpawn};
    let mut d = crate::game::tests_p3_combat::data();
    d.creature_templates.get_mut(&BRUTE).unwrap().spell_list_id = 123;
    let mut t = crate::game::tests_p3_combat::template(NEW, 7);
    t.level_min = 40;
    t.level_max = 40;
    t.faction = 35;
    t.display_ids = [200, 0, 0, 0];
    t.display_scales = [2.0, 0.0, 0.0, 0.0];
    t.base_attack_time = 3_000;
    t.resistances[1] = 50;
    d.creature_templates.insert(NEW, t);
    d.class_level_stats.insert(
        (1, 40),
        ClassLevelStats {
            health: 20_000,
            melee_damage: 30.0,
            ..Default::default()
        },
    );
    d.creature_spawns.entry(0).or_default().push(CreatureSpawn {
        guid: SPAWN,
        entries: vec![BRUTE],
        map: 0,
        x: START.0 + 3.0,
        y: START.1,
        z: START.2,
        respawn_min: 600,
        respawn_max: 600,
        ..Default::default()
    });
    d
}

/// What of a creature an entry change touches.
#[derive(Debug, Clone, PartialEq)]
struct Looks {
    entry: u32,
    field_entry: u32,
    level: u32,
    faction: u32,
    display: u32,
    scale: f32,
    swing: u32,
    fire: u32,
    health: (u32, u32),
}

fn looks(m: &Map, g: Guid) -> Looks {
    use crate::game::fields::*;
    let c = m.creature(g).unwrap();
    let f = &c.fields;
    Looks {
        entry: c.entry,
        field_entry: f.get_u32(OBJECT_FIELD_ENTRY),
        level: f.get_u32(UNIT_FIELD_LEVEL),
        faction: f.get_u32(UNIT_FIELD_FACTIONTEMPLATE),
        display: f.get_u32(UNIT_FIELD_NATIVEDISPLAYID),
        scale: f.get_f32(OBJECT_FIELD_SCALE_X),
        swing: f.get_u32(UNIT_FIELD_BASEATTACKTIME),
        fire: f.get_u32(UNIT_FIELD_RESISTANCES + 2),
        health: (
            f.get_u32(UNIT_FIELD_HEALTH),
            f.get_u32(UNIT_FIELD_MAXHEALTH),
        ),
    }
}

/// Thistle Tea's `CreatureEntry.apply`: "replaces archetype fields without
/// replacing the actor", "does not revive dead creatures or kill a living
/// low-health creature", "ignores missing definitions and repeated entry
/// changes" and "retains original AI and preserves a list when the new
/// template has none"; `Mob.respawn` "restores the original archetype
/// after multiple changes" and "retains a later persistent faction
/// override": an entry change (UPDATE_ENTRY) makes the creature the new
/// template's level, faction, look, size, swing and resistance, its health
/// in proportion, the same creature still in its fight (its victim, threat
/// and tap); its EventAI and its spell list (the new one having none) its
/// old entry's; none to an entry without a template, nothing again to the
/// same; the dead stay dead, one at 1 health alive. A respawn brings the
/// spawn's own back, a faction a script set kept, which cleared is its
/// own template's (mangos's `ClearTemporaryFaction`).
#[tokio::test]
async fn an_entry_change_is_a_new_template_on_the_same_creature_till_it_respawns() {
    use crate::game::fields::*;
    use crate::game::map::scripts::update_entry;
    let (_realm, mut a, ga, map) = setup_with(entry_data(), Class::Warrior).await;
    let brute = crate::game::guid::unit(BRUTE, SPAWN);
    a.sees(brute).await;
    let (before, missing, changed, again, kept) = on(&map, move |m| {
        let f = &mut m.players.get_mut(&ga).unwrap().fields;
        f.set_u32(UNIT_FIELD_MAXHEALTH, 1_000_000);
        f.set_u32(UNIT_FIELD_HEALTH, 1_000_000);
        crate::game::map::ai::attacked(m, brute, ga);
        let c = m.creature_mut(brute).unwrap();
        let max = c.fields.get_u32(UNIT_FIELD_MAXHEALTH);
        c.fields.set_u32(UNIT_FIELD_HEALTH, max / 4);
        let before = looks(m, brute);
        let missing = update_entry(m, brute, 99_999);
        let unchanged = looks(m, brute) == before;
        assert!(update_entry(m, brute, NEW));
        let changed = looks(m, brute);
        assert!(update_entry(m, brute, NEW));
        let again = looks(m, brute) == changed;
        let c = m.creature(brute).unwrap();
        let kept = (
            c.fields.has_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            ),
            c.fields.get_guid(UNIT_FIELD_TARGET) == ga,
            c.threat.iter().any(|(t, _)| *t == ga),
            c.tapped_by == Some(ga),
            event_ai::ai_entry(c) == BRUTE,
            crate::game::map::spells::lists::base_list(m, brute) == 123,
        );
        (before, (missing, unchanged), changed, again, kept)
    })
    .await;
    assert_eq!(missing, (false, true), "no template, no change");
    assert_eq!(
        (
            changed.entry,
            changed.field_entry,
            changed.level,
            changed.faction
        ),
        (NEW, NEW, 40, 35)
    );
    assert_eq!(
        (changed.display, changed.scale, changed.swing, changed.fire),
        (200, 2.0, 3_000, 50)
    );
    assert_eq!(changed.health.0 * 4, changed.health.1, "a quarter still");
    assert!(again, "the same entry again changes nothing");
    assert_eq!(
        kept,
        (true, true, true, true, true, true),
        "the same creature, its fight and AI"
    );
    // The dead stay dead; one at 1 health lives.
    let (dead, low) = on(&map, move |m| {
        let dead = spawn(m, ga, BRUTE, 20, 4.0);
        crate::game::map::ai::die(m, dead, ga);
        update_entry(m, dead, NEW);
        let low = spawn(m, ga, BRUTE, 20, 4.0);
        m.creature_mut(low)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_HEALTH, 1);
        update_entry(m, low, NEW);
        let health = |m: &Map, g: Guid| {
            let c = m.creature(g).unwrap();
            (c.alive(), c.fields.get_u32(UNIT_FIELD_HEALTH))
        };
        (health(m, dead), health(m, low))
    })
    .await;
    assert_eq!(dead, (false, 0));
    assert_eq!(low, (true, 1));
    // A faction set, the creature dies and comes back as its spawn.
    on(&map, move |m| {
        crate::game::map::scripts::execute(m, &step(22, [113, 0, 0, 0]), brute, Guid::zero());
        crate::game::map::ai::die(m, brute, ga);
        let now = m.now;
        if let crate::game::map::creature::Life::Dead { respawn_at, .. } =
            &mut m.creature_mut(brute).unwrap().life
        {
            *respawn_at = now;
        }
    })
    .await;
    until(&map, move |m| m.creature(brute).is_some_and(|c| c.alive())).await;
    let (back, cleared) = on(&map, move |m| {
        let back = looks(m, brute);
        crate::game::map::scripts::execute(m, &step(22, [0; 4]), brute, Guid::zero());
        (back, looks(m, brute).faction)
    })
    .await;
    assert_eq!(
        (back.entry, back.level, back.display, back.swing),
        (BRUTE, before.level, before.display, before.swing),
        "its spawn's own"
    );
    assert_eq!(back.faction, 113, "the script's faction kept");
    assert_eq!(cleared, before.faction, "cleared, its own template's");
}

/// Thistle Tea's `Script.run` "selects target entries from current
/// perception": a script's TERMINATE_SCRIPT goes on while a living creature
/// of the entry it names is near (ended without one), and the next step
/// runs.
#[tokio::test]
async fn a_script_ends_or_goes_on_by_a_living_creature_near() {
    let (_realm, mut a, ga, map) = setup_with(entry_data(), Class::Warrior).await;
    let brute = crate::game::guid::unit(BRUTE, SPAWN);
    a.sees(brute).await;
    let out = on(&map, move |m| {
        let other = spawn(m, ga, NEW, 40, 6.0);
        let go = |m: &mut Map, entry| {
            crate::game::map::scripts::execute(m, &step(31, [entry, 30, 0, 0]), brute, Guid::zero())
        };
        let near = go(m, NEW);
        crate::game::map::ai::die(m, other, ga);
        let dead = go(m, NEW);
        let none = go(m, 4_242);
        (near, dead, none)
    })
    .await;
    assert_eq!(
        out,
        (true, false, false),
        "on with one living near, else ended"
    );
}

/// Thistle Tea's `ChaseWatch`: "notifies chasers only after the target
/// crosses their movement threshold", "updates the watched position after
/// notifying" and `unwatch` "removes a chaser from all target watches": a
/// creature chasing its victim paths again only once the victim has gone
/// past its re-chase distance from where it last aimed, aiming from there
/// on; and leaving the fight, it watches no one.
#[tokio::test]
async fn a_chaser_paths_again_only_past_its_rechase_distance() {
    use crate::game::map::creature::Motion;
    let (_realm, mut a, ga, map) =
        setup_with(crate::game::tests_p3_combat::data(), Class::Warrior).await;
    let brute = on(&map, move |m| {
        let f = &mut m.players.get_mut(&ga).unwrap().fields;
        f.set_u32(crate::game::fields::UNIT_FIELD_MAXHEALTH, 1_000_000);
        f.set_u32(crate::game::fields::UNIT_FIELD_HEALTH, 1_000_000);
        let g = spawn(m, ga, BRUTE, 20, 25.0);
        let c = m.creature_mut(g).unwrap();
        c.speeds.run = 0.5;
        c.speeds.walk = 0.5;
        crate::game::map::ai::attacked(m, g, ga);
        g
    })
    .await;
    a.sees(brute).await;
    let aimed = move |m: &Map| m.creature(brute).and_then(|c| c.chase_target_at);
    until(&map, move |m| {
        aimed(m).is_some() && matches!(m.creature(brute).unwrap().motion, Motion::Moving(_))
    })
    .await;
    let first = on(&map, move |m| aimed(m)).await.unwrap();
    let step = |east: f32| {
        let map = map.clone();
        async move {
            on(&map, move |m| {
                let p = m.players.get_mut(&ga).unwrap();
                p.movement.position.x += east;
            })
            .await;
            crate::game::harness::an_update(&map).await;
            crate::game::harness::an_update(&map).await;
            on(&map, move |m| aimed(m)).await.unwrap()
        }
    };
    assert_eq!(
        step(0.2).await,
        first,
        "a step within its distance: no new path"
    );
    let second = step(10.0).await;
    assert!(
        (second.x - (first.x + 10.2)).abs() < 0.01,
        "past it, a new path, aimed where she is: {second:?}"
    );
    assert_eq!(step(0.2).await, second, "and from there on");
    let after = on(&map, move |m| {
        crate::game::map::ai::enter_evade(m, brute);
        aimed(m)
    })
    .await;
    assert_eq!(after, None, "out of the fight, watching no one");
}
