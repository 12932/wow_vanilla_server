//! Round 11's movement in creates, against Thistle Tea's
//! `movement_block_test.exs` (a jump and a spline elevation in a living
//! block), `movement_inform_test.exs` (an observer arriving during a move
//! is told its final facing) and `taxi_test.exs` (a flight resumed a hair
//! from its end lands): what one who comes into view of a unit is told of
//! how it moves, as mangos-classic's `MovementInfo::Write` and
//! `PacketBuilder::WriteCreate` put it.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::fields::UNIT_FIELD_MOUNTDISPLAYID;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::creature::{Creature, Walk};
use crate::game::map::{npc, scripts};
use crate::game::tests_p3_entity::on;
use crate::game::tests_pets::{BOAR, data, enter};

/// The living movement block of `of` in a create `m` carries, its flags
/// and where it puts it.
fn living_in(m: &ServerOpcodeMessage, of: Guid) -> Option<(MovementBlock_MovementFlags, Vector3d)> {
    let objects = match m {
        ServerOpcodeMessage::SMSG_UPDATE_OBJECT(u) => &u.objects,
        ServerOpcodeMessage::SMSG_COMPRESSED_UPDATE_OBJECT(u) => &u.objects,
        _ => return None,
    };
    objects.iter().find_map(|o| match &o.update_type {
        Object_UpdateType::CreateObject {
            guid3, movement2, ..
        }
        | Object_UpdateType::CreateObject2 {
            guid3, movement2, ..
        } if *guid3 == of => match movement2.update_flag.get_living()? {
            MovementBlock_UpdateFlag_Living::Living {
                flags,
                living_position,
                ..
            } => Some((flags.clone(), *living_position)),
            MovementBlock_UpdateFlag_Living::HasPosition { .. } => None,
        },
        _ => None,
    })
}

/// Thistle Tea's `to_binary/1` "includes jumping data when flag set" and
/// "includes spline elevation when flag set", as mangos-classic's
/// `MovementInfo::Write` sends them in a create: one who comes into view
/// of Alice mid-jump is told of her jump, its speeds and heading and its
/// time in the air, and of a spline elevation she is raised by. wvs took
/// both out of a create: a newcomer saw a jumper standing in the air.
#[tokio::test]
async fn one_who_comes_into_view_of_a_jumper_or_a_raised_one_is_told_the_jump_and_height() {
    jumper_and_raised().await;
}

/// The jump and elevation test's world: Alice, and Bob and Cid, who came
/// into view of her jumping and raised.
async fn jumper_and_raised() -> [Client; 3] {
    let realm = harness::realm_with(data());
    let (a, ga, map) = enter(&realm, "A", "Alice", Race::Human, Class::Warrior).await;
    let at = on(&map, move |m| m.players[&ga].position()).await;
    let info = |flags, fall_time| MovementInfo {
        flags,
        timestamp: 10,
        position: Vector3d {
            x: at.x + 1.0,
            ..at
        },
        orientation: 1.0,
        fall_time,
    };
    let jump =
        MovementInfo_MovementFlags::empty().set_jumping(MovementInfo_MovementFlags_Jumping {
            z_speed: -7.9,
            cos_angle: 0.6,
            sin_angle: 0.8,
            xy_speed: 7.0,
        });
    a.send(MSG_MOVE_JUMP_Client {
        info: info(jump, 120),
    })
    .await;
    let kept = move |m: &mut crate::game::map::Map| m.players[&ga].movement.fall_time;
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    while on(&map, kept).await != 120 {
        assert!(tokio::time::Instant::now() < deadline, "the jump taken");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    harness::an_update(&map).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    let (flags, _) = b.wait_for("Alice's create", |m| living_in(m, ga)).await;
    let j = flags.get_jumping().expect("her jump in her create");
    assert_eq!(
        (j.z_speed, j.cos_angle, j.sin_angle, j.xy_speed),
        (-7.9, 0.6, 0.8, 7.0)
    );
    assert!(flags.get_spline_elevation().is_none(), "no elevation");
    // Landed, raised by a spline elevation.
    let raised = MovementInfo_MovementFlags::empty().set_spline_elevation(
        MovementInfo_MovementFlags_SplineElevation {
            spline_elevation: 1.5,
        },
    );
    a.send(MSG_MOVE_HEARTBEAT_Client {
        info: info(raised, 0),
    })
    .await;
    let raised_now = move |m: &mut crate::game::map::Map| {
        m.players[&ga]
            .movement
            .flags
            .get_spline_elevation()
            .is_some()
    };
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    while !on(&map, raised_now).await {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the heartbeat taken"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    // A create is built once an update: the next one's has the heartbeat.
    harness::an_update(&map).await;
    let mut c = Client::connect(&realm, "C");
    c.enter("Cid", Race::Human, Class::Warrior).await;
    let (flags, _) = c.wait_for("Alice's create", |m| living_in(m, ga)).await;
    assert_eq!(
        flags.get_spline_elevation().map(|e| e.spline_elevation),
        Some(1.5),
        "her elevation in her create"
    );
    assert!(flags.get_jumping().is_none(), "her jump over");
    [a, b, c]
}

/// Thistle Tea's "an observer arriving during the move retains its final
/// facing", as mangos-classic has it: a script's move to a point with a
/// facing carries it (`PointMovementGenerator`'s `SetFacing`), in its
/// `SMSG_MONSTER_MOVE` (wvs turned the creature only on arriving) and in
/// the spline of a create for one who comes upon it on the way
/// (`PacketBuilder::WriteCreate`'s `FINAL_ANGLE`; wvs sent none); a chase
/// its victim (`FINAL_TARGET`).
#[tokio::test]
async fn one_who_comes_upon_a_creature_on_its_way_is_told_its_final_facing() {
    facing_on_the_way().await;
}

/// The final facing test's world: Alice, who saw the boar and the hound
/// set off, and Bob, who came upon them on their way.
async fn facing_on_the_way() -> [Client; 2] {
    let realm = harness::realm_with(data());
    let (mut a, ga, map) = enter(&realm, "A", "Alice", Race::Human, Class::Warrior).await;
    let at = on(&map, move |m| m.players[&ga].position()).await;
    let (boar, hound) = on(&map, move |m| {
        let t = m.realm.data.creature_template(BOAR).cloned().unwrap();
        let mut put = |n: u32, dy: f32| {
            let c = Creature::new(
                guid::unit(BOAR, 0x71_1f00 + n),
                &t,
                1,
                Vector3d { y: at.y + dy, ..at },
                0.0,
            );
            m.add_creature(c.with_base())
        };
        (put(1, 5.0), put(2, -5.0))
    })
    .await;
    a.sees(boar).await;
    a.sees(hound).await;
    a.clear();
    on(&map, move |m| {
        scripts::move_with(
            m,
            boar,
            Vector3d {
                x: at.x + 30.0,
                y: at.y + 5.0,
                ..at
            },
            scripts::Move {
                time: Some(Duration::from_secs(5)),
                face: Some(1.5),
                ..Default::default()
            },
        );
        let now = m.now;
        let spline_id = m.next_spline_id;
        m.next_spline_id += 1;
        let c = m.creature_mut(hound).unwrap();
        let from = c.position;
        let to = Vector3d {
            x: at.x + 30.0,
            ..from
        };
        c.speeds.run = 2.0;
        c.walk_facing(vec![from, to], true, false, ga, now, spline_id);
    })
    .await;
    let (angle, duration) = a
        .wait_for("the boar's move", |m| match m {
            ServerOpcodeMessage::SMSG_MONSTER_MOVE(mv) if mv.guid == boar => match &mv.move_type {
                SMSG_MONSTER_MOVE_MonsterMoveType::FacingAngle {
                    angle, duration, ..
                } => Some((*angle, *duration)),
                _ => Some((f32::NAN, 0)),
            },
            _ => None,
        })
        .await;
    assert_eq!(angle, 1.5, "its facing there in its move");
    assert!(duration > 0 && duration <= 5_000, "{duration}");
    // Bob comes along on its way, at the next update (a create is built
    // once an update, and this one's may be from before the move).
    harness::an_update(&map).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    let spline_of = |m: &ServerOpcodeMessage, of: Guid| {
        living_in(m, of).and_then(|(f, _)| f.get_spline_enabled().cloned())
    };
    // Both creates, in one update object or two.
    let (mut seen_boar, mut seen_hound) = (None, None);
    while seen_boar.is_none() || seen_hound.is_none() {
        let (x, y) = b
            .wait_for("their creates", |m| {
                let (x, y) = (spline_of(m, boar), spline_of(m, hound));
                (x.is_some() || y.is_some()).then_some((x, y))
            })
            .await;
        seen_boar = seen_boar.or(x);
        seen_hound = seen_hound.or(y);
    }
    let s = seen_boar.unwrap();
    match s.spline_flags.get_final_angle() {
        Some(MovementBlock_SplineFlag_FinalAngle::FinalAngle { angle }) => {
            assert_eq!(*angle, 1.5, "its final facing")
        }
        other => panic!("no final angle: {other:?}"),
    }
    assert!(s.time_passed < s.duration, "on its way");
    let s = seen_hound.unwrap();
    match s.spline_flags.get_final_angle() {
        Some(MovementBlock_SplineFlag_FinalAngle::FinalTarget { target }) => {
            assert_eq!(*target, ga.guid(), "its victim faced")
        }
        other => panic!("no final target: {other:?}"),
    }
    [a, b]
}

/// The creates above, a jumper's, a raised one's, and a creature's on its
/// way with a final angle or target, and the moves with them, pass the
/// 1.12 client's own parsers.
#[tokio::test]
#[ignore = "extended"]
async fn the_creates_with_a_jump_an_elevation_or_a_final_facing_pass_the_client() {
    use crate::game::tests_client_emu::{capture, checker, run_checker};
    let (check_py, wow_exe) = checker()
        .expect("the client-emulation check needs tools/client_check/WoW.exe and `uv`: see docs/TESTING.md");
    let [mut a, mut b, mut c] = jumper_and_raised().await;
    let (frames, counts) = capture(vec![&mut a, &mut b, &mut c]);
    assert!(
        counts.get(&0x0A9).copied().unwrap_or(0) > 0,
        "creates captured"
    );
    run_checker(&check_py, &wow_exe, &frames);
    let [mut a, mut b] = facing_on_the_way().await;
    let (frames, counts) = capture(vec![&mut a, &mut b]);
    assert!(
        counts.get(&0x0DD).copied().unwrap_or(0) > 0,
        "moves captured"
    );
    run_checker(&check_py, &wow_exe, &frames);
}

/// Thistle Tea's `resume/3` "finishes a checkpoint within movement
/// tolerance without an empty spline": a flight saved at logout a hair
/// from its end (within a tenth of a yard on each axis) is over at the
/// next login: no flight, no mount, no `SMSG_MONSTER_MOVE` of no length.
/// wvs flew it, a spline of a few hundredths of a yard, then landed it
/// with a teleport.
#[tokio::test]
async fn a_flight_saved_a_hair_from_its_end_lands_at_the_next_login() {
    let realm = harness::realm_with(data());
    let (a, ga, map) = enter(&realm, "A", "Alice", Race::Human, Class::Warrior).await;
    on(&map, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        let from = p.position();
        let end = Vector3d {
            x: from.x + 0.05,
            ..from
        };
        // Its clock not yet started: it is saved at its start, its end
        // still ahead, whenever the logout comes.
        let walk = Walk::new(
            vec![from, end],
            32.0,
            m.now + Duration::from_secs(3_600),
            9_001,
            true,
        );
        let p = m.players.get_mut(&ga).unwrap();
        p.fields.set_u32(UNIT_FIELD_MOUNTDISPLAYID, 1149);
        p.flight = Some(npc::Flight {
            walk,
            destination: 2,
            mount: 1149,
        });
    })
    .await;
    a.disconnect();
    harness::offline(&realm, ga).await;
    let mut a = Client::connect(&realm, "A");
    a.login(ga).await;
    let map = realm.registry.online(ga).unwrap().map();
    let state = on(&map, move |m| {
        let p = &m.players[&ga];
        (
            p.flight.is_some(),
            p.row.state.flight.is_some(),
            p.fields.get_u32(UNIT_FIELD_MOUNTDISPLAYID),
        )
    })
    .await;
    assert_eq!(state, (false, false, 0), "landed, off the mount");
    harness::an_update(&map).await;
    harness::an_update(&map).await;
    let moves = a
        .captured()
        .iter()
        .filter_map(|f| ServerOpcodeMessage::read_unencrypted(&f[..]).ok())
        .filter(|m| matches!(m, ServerOpcodeMessage::SMSG_MONSTER_MOVE(mv) if mv.guid == ga))
        .count();
    assert_eq!(moves, 0, "no spline of no length");
}
