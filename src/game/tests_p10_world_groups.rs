//! Round 10's tests of the formations scripts make, ported from Thistle
//! Tea's `creature_groups_test.exs`: a runtime formation whose leader dies
//! on its walk is led on along it by its first member alive, whenever the
//! walk began and whenever they joined, then by the next once that one
//! dies, the leader's corpse gone; and one joining a leader already dead
//! hears of its death and, in formation, leads it on.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::Waypoint;
use crate::game::data::scripts::*;
use crate::game::fields::*;
use crate::game::map::creature::MOVEMENT_WAYPOINT;
use crate::game::map::creature_groups::{self, formation_home, join_with, members_of};
use crate::game::map::event_ai::Route;
use crate::game::map::{Map, combat};
use crate::game::tests_p3_combat::{BRUTE, START, setup_with, spawn, template};
use crate::game::tests_p3_entity::{on, step};

/// A creature that marks itself (emote state 10) when it hears its
/// group's leader has died.
const LISTENER: u32 = 997_500;
const MARK: u32 = 997_501;

fn world() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    d.creature_templates.insert(LISTENER, template(LISTENER, 7));
    d.scripts.ai_events.insert(
        LISTENER,
        vec![AiEvent {
            id: 1,
            creature: LISTENER,
            kind: EVENT_GROUP_MEMBER_DIED,
            chance: 100,
            flags: EF_REPEATABLE,
            params: [0, 1, 0, 0],
            actions: [MARK, 0, 0],
            ..Default::default()
        }],
    );
    d.scripts.ai_scripts.insert(
        MARK,
        vec![step(2, [u32::from(UNIT_NPC_EMOTESTATE), 10, 0, 0])],
    );
    d
}

/// A walk of two points east of the start; `g` bound for the second.
fn walk(m: &mut Map, g: Guid) -> Arc<Vec<Waypoint>> {
    let points: Arc<Vec<Waypoint>> = Arc::new(
        (0..2)
            .map(|i| Waypoint {
                point: i,
                x: START.0 + 2.0 + 10.0 * i as f32,
                y: START.1,
                z: START.2,
                ..Default::default()
            })
            .collect(),
    );
    let c = m.creature_mut(g).unwrap();
    c.scripted.get_or_insert_with(Default::default).route = Some(Route {
        points: points.clone(),
        repeat: true,
    });
    c.movement_type = MOVEMENT_WAYPOINT;
    c.waypoint = 1;
    c.last_waypoint = Some(0);
    points
}

/// What `g` walks: its route's points, the point it is bound for and
/// whether it walks a path.
fn walking(m: &Map, g: Guid) -> (Option<usize>, usize, bool) {
    let c = m.creature(g).unwrap();
    (
        c.scripted
            .as_deref()
            .and_then(|s| s.route.as_ref())
            .map(|r| r.points.len()),
        c.waypoint,
        c.movement_type == MOVEMENT_WAYPOINT,
    )
}

const FORMATION: u32 = GROUP_FORMATION;
const ALL: u32 = GROUP_FORMATION
    | GROUP_AGGRO_TOGETHER
    | GROUP_EVADE_TOGETHER
    | GROUP_INFORM_MEMBERS_ON_ANY_DIED;

/// Thistle Tea's `CreatureGroups.formation/3` "runtime formations inherit
/// routes that began after the leader spawned", "runtime formations
/// inherit a route and cursor that preceded membership" and "promotes
/// again after the temporary commander's owner exits": a formation a
/// script made, its leader given its walk after they joined, has its first
/// member (by guid) lead on along the walk from the point it was bound
/// for once the leader dies, the other keeping its place by it; a member
/// that joined out of formation (only told of deaths) leads no one; the
/// leader's corpse gone, the two are of the first's group; the first dead,
/// the second leads on from where the first was bound. mangos-classic's
/// `FormationData::OnDeath` sets a new master on the master's path.
#[tokio::test]
async fn a_script_formation_is_led_on_along_its_dead_leaders_walk() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let out = on(&map, move |m| {
        // Out of formation: only told of deaths.
        let lone = spawn(m, ga, BRUTE, 20, 30.0);
        walk(m, lone);
        let told = spawn(m, ga, BRUTE, 20, 31.0);
        join_with(m, told, lone, 2.0, 0.0, GROUP_INFORM_MEMBERS_ON_ANY_DIED);
        combat::kill(m, ga, lone);
        let unled = (m.script_acting.get(&lone).copied(), walking(m, told).2);
        // In formation, the walk begun after they joined.
        let leader = spawn(m, ga, BRUTE, 20, 10.0);
        let first = spawn(m, ga, BRUTE, 20, 12.0);
        let second = spawn(m, ga, BRUTE, 20, 14.0);
        assert!(first.guid() < second.guid());
        join_with(m, first, leader, 2.0, 1.0, ALL);
        join_with(m, second, leader, 4.0, 2.0, FORMATION);
        let points = walk(m, leader);
        combat::kill(m, ga, leader);
        let led = (
            m.script_acting.get(&leader).copied(),
            walking(m, first),
            formation_home(m, second).is_some(),
            formation_home(m, first).is_none(),
        );
        // Its corpse gone, the first's group.
        m.remove_creature(leader);
        let mut group = members_of(m, second);
        group.sort();
        let after = (group, creature_groups::group_leader(m, second));
        // The first dead, the second leads on.
        m.creature_mut(first).unwrap().waypoint = 0;
        combat::kill(m, ga, first);
        let next = (m.script_acting.get(&first).copied(), walking(m, second));
        (unled, points.len(), led, (first, second), after, next)
    })
    .await;
    let (unled, len, led, (first, second), after, next) = out;
    assert_eq!(unled, (None, false), "out of formation, no one leads");
    assert_eq!(
        led,
        (Some(first), (Some(len), 1, true), true, true),
        "the first leads on the walk, from the point bound for"
    );
    let mut both = vec![first, second];
    both.sort();
    assert_eq!(after.0, both, "the two, the leader's corpse gone");
    assert_eq!(
        next,
        (Some(second), (Some(len), 0, true)),
        "then the second"
    );
}

/// Thistle Tea's `CreatureGroups.join/6` "joining a dead leader receives
/// its death and inherits its route", with the flags 0x80 and 0x87: one
/// joining a leader that died on its walk hears of the death (the group
/// telling its members of any); in formation, it leads the walk on from
/// the point the leader was bound for; out of formation, it leads none.
#[tokio::test]
async fn one_joining_a_dead_leader_hears_of_it_and_in_formation_leads_its_walk_on() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let out = on(&map, move |m| {
        let marked = |m: &Map, g: Guid| {
            m.unit_fields(g)
                .is_some_and(|f| f.get_u32(UNIT_NPC_EMOTESTATE) == 10)
        };
        let mut out = Vec::new();
        for flags in [GROUP_INFORM_MEMBERS_ON_ANY_DIED, ALL] {
            let leader = spawn(m, ga, BRUTE, 20, 10.0);
            walk(m, leader);
            combat::kill(m, ga, leader);
            let member = spawn(m, ga, LISTENER, 20, 12.0);
            join_with(m, member, leader, 2.0, 0.0, flags);
            out.push((
                marked(m, member),
                m.script_acting.get(&leader) == Some(&member),
                walking(m, member),
            ));
        }
        out
    })
    .await;
    assert_eq!(out[0], (true, false, (None, 0, false)), "told, leads none");
    assert_eq!(out[1], (true, true, (Some(2), 1, true)), "told, leads on");
}
