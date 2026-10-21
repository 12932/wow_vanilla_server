//! Round 11's entity AI rows (docs/parity/matrix.csv): what an aggressive
//! pet notices, and how possessed and formation units move.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::fields::UNIT_FIELD_LEVEL;
use crate::game::map::Map;
use crate::game::map::pets::{self, Kind};
use crate::game::tests_p5_ai_pets::tamed_with;
use crate::game::tests_pets::{WOLF, place_in};

/// A guardian that notices at eighteen yards, as the real data's do.
const WATCHER: u32 = 98_500;

fn watcher_data() -> GameData {
    let mut d = crate::game::tests_pets::data();
    let mut t = d.creature_templates[&WOLF].clone();
    t.entry = WATCHER;
    t.detection_range = 18.0;
    d.creature_templates.insert(WATCHER, t);
    d
}

/// A few of the pets' updates, a second apart.
fn think(m: &mut Map, times: u32) {
    for _ in 0..times {
        m.now += Duration::from_secs(1);
        pets::update(m);
    }
}

fn victim(m: &Map, g: Guid) -> Option<Guid> {
    m.combat(g).filter(|c| c.attacking).map(|c| c.victim)
}

/// Thistle Tea's "aggressive pets observe and check sight beyond twenty
/// yards" (ai_environment_test 174), as mangos-classic's
/// `PetAI::MoveInLineOfSight` looks: an aggressive guardian of 60 that
/// notices at 18 yards takes no wolf of its level 25 yards off, takes one
/// of 50 there (its radius a yard more a level below it, 28), but not
/// while it stands five yards above, nor while it is stunned (where every
/// pet looked twenty yards round, whatever the levels, at any height).
#[tokio::test]
async fn an_aggressive_pet_notices_as_far_as_the_levels_say() {
    let (_realm, _h, gh, map, _pet, wolf) = tamed_with(watcher_data()).await;
    let out = map
        .query(move |m| {
            m.remove_creature(wolf);
            let p = m.players[&gh].position();
            let g = pets::testing::spawn(m, gh, WATCHER, Kind::Guardian, 60, p, 0.0);
            pets::state_mut(m, g).unwrap().react = pets::REACT_AGGRESSIVE;
            let far = place_in(m, gh, WOLF, 0x70_0040, 25.0);
            let level = |m: &mut Map, l: u32| {
                m.creature_mut(far)
                    .unwrap()
                    .fields
                    .set_u32(UNIT_FIELD_LEVEL, l)
            };
            level(m, 60);
            think(m, 2);
            let even = victim(m, g);
            level(m, 50);
            // Up on a ledge, then stunned, then free on the ground.
            m.creature_mut(far).unwrap().position.z += 5.0;
            think(m, 2);
            let above = victim(m, g);
            m.creature_mut(far).unwrap().position.z -= 5.0;
            let stunned = crate::game::map::player::UNIT_FLAG_STUNNED;
            let flags = |m: &mut Map, on: bool| {
                m.creature_mut(far).unwrap().fields.set_flag(
                    crate::game::fields::UNIT_FIELD_FLAGS,
                    stunned,
                    on,
                )
            };
            flags(m, true);
            think(m, 2);
            let held = victim(m, g);
            flags(m, false);
            think(m, 2);
            (even, above, held, victim(m, g), far)
        })
        .await
        .unwrap();
    let (even, above, held, lower, far) = out;
    assert_eq!(even, None, "of its level, 25 yards is beyond its 18");
    assert_eq!(above, None, "five yards above, out of its reach");
    assert_eq!(held, None, "a stunned one is let be");
    assert_eq!(lower, Some(far), "ten levels below it, within its 28");
}

/// Thistle Tea's "acknowledged possession launches reach observers once
/// and never relaunch the controller" (event_sink/movement_test 371): a
/// warlock moving her Eye of Kilrogg moves it, a watcher told once, her
/// own client not told back what it said (mangos-classic's
/// `SendMessageToAllWhoSeeMeMove` sparing the mover's client, where wvs
/// echoed it to her); dead, the eye moves no more.
#[tokio::test]
async fn a_possessed_units_moves_reach_its_watchers_not_its_mover_and_not_once_dead() {
    use crate::game::harness::Client;
    use crate::game::tests_p3_entity_objects::until;
    use crate::game::tests_pets::{cast, enter, learn};
    use crate::game::tests_pets2::{EYE, EYE_OF_KILROGG, more};
    use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
    let realm = crate::game::harness::realm_with(more());
    let (mut w, gw, map) = enter(&realm, "W", "Warlock", Race::Human, Class::Warlock).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    learn(&map, gw, &[EYE_OF_KILROGG]);
    cast(&w, EYE_OF_KILROGG, None).await;
    let eye = w
        .wait_for("control of the eye", |m| match m {
            ServerOpcodeMessage::SMSG_CLIENT_CONTROL_UPDATE(u)
                if u.guid != gw && u.allow_movement =>
            {
                Some(u.guid)
            }
            _ => None,
        })
        .await;
    assert_eq!(crate::game::guid::entry(eye), EYE);
    w.send(CMSG_SET_ACTIVE_MOVER { guid: eye }).await;
    b.sees(eye).await;
    let at = map
        .query(move |m| m.position_of(eye).unwrap())
        .await
        .unwrap();
    w.clear();
    b.clear();
    w.heartbeat(at.x + 3.0, at.y, at.z).await;
    until(&map, "the eye moved", move |m| {
        m.position_of(eye)
            .is_some_and(|p| (p.x - (at.x + 3.0)).abs() < 0.01)
    })
    .await;
    let heartbeat = |m: &ServerOpcodeMessage| match m {
        ServerOpcodeMessage::MSG_MOVE_HEARTBEAT(h) if h.guid == eye => Some(()),
        _ => None,
    };
    assert!(
        b.gets(Duration::from_millis(500), heartbeat)
            .await
            .is_some(),
        "the watcher sees it move"
    );
    assert!(
        b.gets(Duration::from_millis(200), heartbeat)
            .await
            .is_none(),
        "once"
    );
    assert!(
        w.gets(Duration::from_millis(200), heartbeat)
            .await
            .is_none(),
        "its mover isn't told back"
    );
    // Dead, it moves no more.
    map.query(move |m| {
        let now = m.now;
        if let Some(c) = m.creature_mut(eye) {
            c.fields.set_u32(crate::game::fields::UNIT_FIELD_HEALTH, 0);
            c.life = crate::game::map::creature::Life::Dead {
                corpse_until: now,
                respawn_at: now,
            };
        }
    })
    .await
    .unwrap();
    w.heartbeat(at.x + 9.0, at.y, at.z).await;
    // Nothing to wait for: what came of the move, a while after it.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let moved = map
        .query(move |m| m.position_of(eye).map(|p| p.x))
        .await
        .unwrap();
    assert!(
        moved.is_none_or(|x| (x - (at.x + 3.0)).abs() < 0.01),
        "dead, where it fell: {moved:?}"
    );
}

/// Part of Thistle Tea's "an observer arriving during the move retains its
/// final facing" (mob/movement_inform_test.exs:27): a creature a script
/// walks to a point facing 1.5 is seen to walk there by its watcher with
/// that facing at the walk's end (`SMSG_MONSTER_MOVE`'s facing angle,
/// vmangos's `MovePoint` with its orientation), where it turned on the
/// server alone and its watchers saw it stand facing its way.
#[tokio::test]
async fn a_scripted_walks_facing_is_told_with_the_walk() {
    use crate::game::map::scripts;
    use crate::game::tests_p3_combat::{BRUTE, data, setup_with, spawn};
    use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
    let (_realm, mut a, ga, map) = setup_with(data(), Class::Warrior).await;
    let brute = map
        .query(move |m| spawn(m, ga, BRUTE, 20, 8.0))
        .await
        .unwrap();
    a.sees(brute).await;
    a.clear();
    map.query(move |m| {
        let at = m.creature(brute).unwrap().position;
        let to = Vector3d {
            x: at.x + 6.0,
            ..at
        };
        scripts::move_with(
            m,
            brute,
            to,
            scripts::Move {
                face: Some(1.5),
                ..Default::default()
            },
        )
    })
    .await
    .unwrap();
    let facing = a
        .wait_for("the walk", |m| match m {
            ServerOpcodeMessage::SMSG_MONSTER_MOVE(mv) if mv.guid == brute => {
                Some(mv.move_type.clone())
            }
            _ => None,
        })
        .await;
    match facing {
        SMSG_MONSTER_MOVE_MonsterMoveType::FacingAngle { angle, .. } => {
            assert!((angle - 1.5).abs() < 0.001, "facing {angle}")
        }
        other => panic!("a walk with no facing: {other:?}"),
    }
}

/// The final angle of `g`'s spline in a create `m` carries of it: `None`
/// for no create of it, `Some(None)` for one with no final angle.
fn created_final_angle(
    m: &wow_world_messages::vanilla::opcodes::ServerOpcodeMessage,
    g: Guid,
) -> Option<Option<f32>> {
    use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
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
        } if *guid3 == g => match movement2.update_flag.get_living()? {
            MovementBlock_UpdateFlag_Living::Living { flags, .. } => {
                Some(flags.get_spline_enabled().and_then(
                    |s| match s.spline_flags.get_final_angle() {
                        Some(MovementBlock_SplineFlag_FinalAngle::FinalAngle { angle }) => {
                            Some(*angle)
                        }
                        _ => None,
                    },
                ))
            }
            _ => Some(None),
        },
        _ => None,
    })
}

/// The rest of Thistle Tea's "an observer arriving during the move
/// retains its final facing" (mob/movement_inform_test.exs:27): one who
/// comes to see a creature on a scripted walk to a facing of 1.5 is shown
/// its walk with that facing at its end, in the create's spline
/// (mangos-classic's `PacketBuilder::WriteCreate`, the final angle after
/// the flags), where the create dropped it.
#[tokio::test]
async fn a_late_watcher_is_shown_a_scripted_walks_facing() {
    use crate::game::harness::Client;
    use crate::game::map::scripts;
    use crate::game::tests_p3_combat::{BRUTE, data, setup_with, spawn};
    let (realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let brute = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 8.0);
            let at = m.creature(brute).unwrap().position;
            let to = Vector3d {
                x: at.x + 20.0,
                ..at
            };
            scripts::move_with(
                m,
                brute,
                to,
                scripts::Move {
                    face: Some(1.5),
                    ..Default::default()
                },
            );
            brute
        })
        .await
        .unwrap();
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    let angle = b
        .wait_for("the brute's create", |m| created_final_angle(m, brute))
        .await;
    let walking = map
        .query(move |m| {
            matches!(
                m.creature(brute).unwrap().motion,
                crate::game::map::creature::Motion::Moving(_)
            )
        })
        .await
        .unwrap();
    assert!(walking, "still on its way");
    assert!(
        angle.is_some_and(|a| (a - 1.5).abs() < 0.001),
        "its facing at the end: {angle:?}"
    );
}

/// The client check of a create whose spline ends on a facing: Bob, come
/// to see a brute on a scripted walk to a facing, is sent its create with
/// the final angle, which `WoW.exe`'s own parser reads.
#[tokio::test]
#[ignore = "extended"]
async fn a_create_with_a_walks_final_facing_passes_the_client() {
    use crate::game::harness::Client;
    use crate::game::map::scripts;
    use crate::game::tests_client_emu::{capture, checker, run_checker};
    use crate::game::tests_p3_combat::{BRUTE, data, setup_with, spawn};
    let (check_py, wow_exe) = checker()
        .expect("the client-emulation check needs tools/client_check/WoW.exe and `uv`: see docs/TESTING.md");
    let (realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let brute = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 8.0);
            let at = m.creature(brute).unwrap().position;
            let to = Vector3d {
                x: at.x + 20.0,
                ..at
            };
            scripts::move_with(
                m,
                brute,
                to,
                scripts::Move {
                    face: Some(1.5),
                    ..Default::default()
                },
            );
            brute
        })
        .await
        .unwrap();
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    let angle = b
        .wait_for("the brute's create", |m| created_final_angle(m, brute))
        .await;
    assert!(angle.is_some(), "a create with its facing");
    for _ in 0..3 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        b.captured();
    }
    let (capture, counts) = capture(vec![&mut b]);
    assert!(
        counts.get(&0x0A9).copied().unwrap_or(0) + counts.get(&0x1F6).copied().unwrap_or(0) > 0,
        "its create was captured"
    );
    run_checker(&check_py, &wow_exe, &capture);
}
