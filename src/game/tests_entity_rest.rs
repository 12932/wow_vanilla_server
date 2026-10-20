//! A creature put at rest on a walk is stopped where the walk has come to,
//! its watchers told (`ai::rest`, as death's stop and mangos's
//! `MoveSplineInit::Stop`): a script's default movement, its idle and its
//! wander, a pet told to stay, and the helper itself. Without the stop, a
//! client that saw the walk set off ran the creature on to its end; with
//! one at the last update's place, it jumped back.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::scripts::ScriptStep;
use crate::game::harness::{self, Client};
use crate::game::map::Map;
use crate::game::map::combat::dist;
use crate::game::map::creature::Motion;
use crate::game::tests_pets::{BOAR, data, enter, place};

/// Where `g` is told it stopped.
fn stop_of(g: Guid) -> impl FnMut(&ServerOpcodeMessage) -> Option<Vector3d> {
    move |m| match m {
        ServerOpcodeMessage::SMSG_MONSTER_MOVE(mv)
            if mv.guid == g && matches!(mv.move_type, SMSG_MONSTER_MOVE_MonsterMoveType::Stop) =>
        {
            Some(mv.spline_point)
        }
        _ => None,
    }
}

/// `g` sets off 30 yards east, and a third of a second on (between two of
/// the map's updates) is where this returns.
fn walking(m: &mut Map, g: Guid) -> Vector3d {
    let from = m.creature(g).unwrap().position;
    let to = Vector3d {
        x: from.x + 30.0,
        ..from
    };
    crate::game::map::scripts::move_to(m, g, to, true);
    m.now += Duration::from_millis(300);
    match &m.creature(g).unwrap().motion {
        Motion::Moving(w) => w.position_at(m.now).0,
        _ => panic!("it walks"),
    }
}

fn step(command: u32, datalong: [u32; 4]) -> ScriptStep {
    ScriptStep {
        command,
        datalong,
        x: 5.0,
        ..Default::default()
    }
}

/// Each way a creature on a walk is put at rest stops it where the walk
/// has come to, for one who watched it set off, and the server has it
/// there too, standing (or about to wander, for a wander).
#[tokio::test]
async fn a_creature_put_at_rest_on_a_walk_stops_where_it_has_come_to() {
    type Rest = fn(&mut Map, Guid);
    let ways: [(&str, Rest); 5] = [
        ("the helper", |m, g| crate::game::map::ai::rest(m, g, None)),
        ("a script's default movement", |m, g| {
            crate::game::map::scripts::execute(m, &step(67, [0, 0, 0, 0]), g, g);
        }),
        ("a script's idle", |m, g| {
            crate::game::map::scripts::execute(m, &step(20, [0, 0, 0, 0]), g, g);
        }),
        ("a script's wander", |m, g| {
            crate::game::map::scripts::execute(m, &step(20, [1, 0, 0, 0]), g, g);
        }),
        ("a pet's stop", |m, g| {
            crate::game::map::pets::ai::stop(m, g)
        }),
    ];
    let realm = harness::realm_with(data());
    let (mut a, ga, map) = enter(&realm, "A", "Ari", Race::Dwarf, Class::Hunter).await;
    for (i, (what, way)) in ways.into_iter().enumerate() {
        let boar = place(&map, ga, BOAR, 0x72_0001 + i as u32, 6.0).await;
        a.sees(boar).await;
        a.clear();
        let (along, at) = map
            .query(move |m| {
                let along = walking(m, boar);
                way(m, boar);
                let c = m.creature(boar).unwrap();
                (along, (c.position, c.motion.clone()))
            })
            .await
            .unwrap();
        let (server, motion) = at;
        let told = told_stop(&mut a, boar).await;
        assert!(
            dist(told, along) < 0.1,
            "{what}: stopped where it had come to, {along:?}, not {told:?}"
        );
        assert!(dist(server, along) < 0.1, "{what}: the server's {server:?}");
        assert!(
            matches!(motion, Motion::Idle { .. }),
            "{what}: at rest, {motion:?}"
        );
    }
}

async fn told_stop(c: &mut Client, g: Guid) -> Vector3d {
    c.wait_for("its stop", stop_of(g)).await
}
