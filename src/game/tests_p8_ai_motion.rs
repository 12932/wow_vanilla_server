//! Round 8's creature AI tests of formations and followers, ported from
//! Thistle Tea's `test/game/core/ai/bt/formation_test.exs` and
//! `bt/follow_test.exs` (docs/parity/matrix.csv): a follower's catch-up
//! speed and its own leg, a leader that stands or fights, a follower's own
//! route held, a fight's movement kept on joining; a scripted follower
//! settling, waiting, and leaving its fight where it stands.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::scripts::{GROUP_FORMATION, GroupMember};
use crate::game::data::{GameData, Waypoint};
use crate::game::guid;
use crate::game::map::combat::{self, dist};
use crate::game::map::creature::Motion;
use crate::game::map::{Map, ai, creature_groups, event_ai};
use crate::game::tests_parity_motion::{at, enter, spawn, wolf, wolf_world};

const WALKER: u32 = 5_803;

/// A leader (spawn 190) and its follower (191) three yards behind it, in
/// formation; the follower has a path of its own; both neutral.
fn world() -> GameData {
    let mut d = wolf_world();
    let wolf_entry = guid::entry(wolf());
    let mut t = d.creature_templates[&wolf_entry].clone();
    t.entry = WALKER;
    t.faction = 32;
    d.creature_templates.insert(WALKER, t);
    let spawns = d.creature_spawns.get_mut(&0).unwrap();
    spawns.push(spawn(190, WALKER, at(20.0, -40.0)));
    let mut own_way = spawn(191, WALKER, at(17.0, -40.0));
    own_way.movement_type = 2;
    spawns.push(own_way);
    let p = at(17.0, -60.0);
    d.waypoints.insert(
        191,
        vec![Waypoint {
            point: 1,
            x: p.x,
            y: p.y,
            z: p.z,
            ..Default::default()
        }],
    );
    let member = |guid, distance, angle| GroupMember {
        guid,
        distance,
        angle,
        flags: GROUP_FORMATION,
    };
    d.scripts.groups.insert(
        190,
        vec![
            member(190, 0.0, 0.0),
            member(191, 3.0, std::f32::consts::PI),
        ],
    );
    d.scripts.group_of.insert(190, 190);
    d.scripts.group_of.insert(191, 190);
    d
}

fn leader() -> Guid {
    guid::unit(WALKER, 190)
}

fn follower() -> Guid {
    guid::unit(WALKER, 191)
}

/// The map's creatures take `n` turns of 50 ms.
fn turns(m: &mut Map, n: u32) {
    for _ in 0..n {
        m.now += Duration::from_millis(50);
        ai::update_creatures(m);
        combat::update_creatures(m);
    }
}

/// Its walk: its id and speed.
fn walk(m: &Map, g: Guid) -> Option<(u32, f32)> {
    match &m.creature(g)?.motion {
        Motion::Moving(w) => Some((w.spline_id, w.speed)),
        Motion::Idle { .. } => None,
    }
}

/// `g` sets off for `to`, as the map would send it; its walk's id.
fn send_off(m: &mut Map, g: Guid, to: Vector3d) -> u32 {
    let now = m.now;
    let id = m.next_spline_id;
    m.next_spline_id += 1;
    let c = m.creature_mut(g).unwrap();
    let from = c.position;
    let _ = c.walk(vec![from, to], false, now, id);
    id
}

/// Thistle Tea's "caps catch-up speed at 130 percent of run speed" and
/// "finishes its own leg before starting another": a follower far behind
/// its leader's walk runs to its place no faster than 1.3 times its run
/// speed; while its leader is on that walk, its own goes on, not begun
/// again.
#[tokio::test]
async fn a_follower_catches_up_at_most_at_130_percent_and_keeps_its_leg() {
    let (_realm, mut a, _ga, map) = enter(world()).await;
    let (l, f) = (leader(), follower());
    a.sees(f).await;
    let out = map
        .query(move |m| {
            m.creature_mut(f).unwrap().position = at(20.0, -100.0);
            m.creature_mut(l).unwrap().orientation = std::f32::consts::FRAC_PI_2;
            send_off(m, l, at(20.0, -38.0));
            creature_groups::leader_walked(m, l);
            let run = m.creature(f).unwrap().speeds.run;
            let first = walk(m, f);
            turns(m, 10);
            (first, run, walk(m, f))
        })
        .await
        .unwrap();
    let (first, run, later) = out;
    let (id, speed) = first.expect("it follows");
    assert!(
        (speed - run * 1.3).abs() < 0.01,
        "{speed} for a run of {run}"
    );
    assert_eq!(later.map(|w| w.0), Some(id), "its own leg goes on");
}

/// Thistle Tea's "does not start movement while the leader is stopped or
/// fighting": a leader standing, or chasing in a fight, leads its
/// follower nowhere.
#[tokio::test]
async fn a_leader_standing_or_fighting_leads_its_follower_nowhere() {
    let (_realm, mut a, ga, map) = enter(world()).await;
    let (l, f) = (leader(), follower());
    a.sees(f).await;
    let out = map
        .query(move |m| {
            creature_groups::leader_walked(m, l);
            let standing = walk(m, f);
            m.creature_mut(l).unwrap().combat.attacking = true;
            m.creature_mut(l).unwrap().combat.victim = ga;
            send_off(m, l, at(20.0, -30.0));
            creature_groups::leader_walked(m, l);
            (standing, walk(m, f))
        })
        .await
        .unwrap();
    assert_eq!(out, (None, None));
}

/// Thistle Tea's "suppresses the follower's original waypoint route" and
/// "membership changes preserve combat movement": a follower with a path
/// of its own doesn't walk it while in formation, its path still there;
/// a creature chasing in a fight that a script puts in a formation chases
/// on, its walk as it was.
#[tokio::test]
async fn a_followers_own_path_waits_and_a_fight_chases_on_through_a_join() {
    let (_realm, mut a, ga, map) = enter(world()).await;
    let (l, f) = (leader(), follower());
    a.sees(f).await;
    let out = map
        .query(move |m| {
            turns(m, 100);
            let still = (
                dist(m.creature(f).unwrap().position, at(17.0, -40.0)) < 0.1,
                m.realm.data.waypoints.contains_key(&191),
            );
            // The wolf chasing her joins the leader.
            let w = wolf();
            ai::attacked(m, w, ga);
            let id = send_off(m, w, at(5.0, 0.0));
            creature_groups::join_with(m, w, l, 2.0, 0.0, GROUP_FORMATION);
            (still, walk(m, w).map(|x| x.0) == Some(id))
        })
        .await
        .unwrap();
    assert_eq!(out.0, (true, true), "its own path waits");
    assert!(out.1, "its chase goes on");
}

/// Thistle Tea's "settles beside a leader who has stopped", "waits in
/// place while its leader is out of sight" and "a follower drops combat
/// where it stands instead of going home": a creature a script set
/// following a leader who stands near it stays where it is; one whose
/// leader isn't on the map waits there, still following; out of a fight,
/// it leaves it where it stands, following still, not off to its spawn.
#[tokio::test]
async fn a_scripted_follower_settles_by_a_still_leader_waits_for_a_gone_one_and_evades_in_place() {
    let (_realm, mut a, ga, map) = enter(world()).await;
    let l = leader();
    let w = wolf();
    a.sees(w).await;
    let out = map
        .query(move |m| {
            let follow = |m: &mut Map, leader: Guid| {
                m.creature_mut(w)
                    .unwrap()
                    .scripted
                    .get_or_insert_with(Default::default)
                    .follow = Some((leader, 3.0, std::f32::consts::PI));
            };
            // Beside its leader, who stands.
            let by = m.creature(l).unwrap().position;
            m.creature_mut(w).unwrap().position = Vector3d {
                x: by.x - 2.5,
                ..by
            };
            follow(m, l);
            event_ai::follow_step(m, w);
            let settled = walk(m, w);
            // A leader not here.
            m.creature_mut(w).unwrap().position = at(10.0, 0.0);
            let gone = guid::unit(WALKER, 999);
            follow(m, gone);
            event_ai::follow_step(m, w);
            let waits = (
                walk(m, w),
                event_ai::scripted(m, w).and_then(|s| s.follow).map(|f| f.0),
            );
            // Following its leader, 20 yards from its spawn, it fights
            // her and gives up.
            follow(m, l);
            let there = at(10.0, -20.0);
            m.creature_mut(w).unwrap().position = there;
            ai::attacked(m, w, ga);
            ai::enter_evade(m, w);
            let c = m.creature(w).unwrap();
            let evaded = (
                c.evading,
                dist(c.position, there) < 0.1,
                walk(m, w).is_none(),
                c.combat.attacking,
                event_ai::scripted(m, w).and_then(|s| s.follow).map(|f| f.0),
            );
            (settled, waits, gone, evaded)
        })
        .await
        .unwrap();
    let (settled, waits, gone, evaded) = out;
    assert_eq!(settled, None, "settled by its leader");
    assert_eq!(waits, (None, Some(gone)), "waiting, following still");
    assert_eq!(
        evaded,
        (false, true, true, false, Some(l)),
        "out of its fight where it stands, following"
    );
}
