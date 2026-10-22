//! Round 12c's speed check: an allowance earned over time, not per packet;
//! slows held to; a knockback's flight at its own speed; and real movement
//! (a run, a jump, lag bunching packets up, a fall) still let through.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::GameData;
use crate::game::harness::Client;
use crate::game::map::MapHandle;
use crate::game::map::auras::{self, AURA_MOD_DECREASE_SPEED};
use crate::game::tests_p3_combat::START;
use crate::game::tests_p3_entity::{aura_effect, on, spell};

pub(crate) const SLOW: u32 = 97_820;

/// A realm whose players aren't GMs (the check passes GMs), with a packet
/// rate no test reaches, on `data`.
pub(crate) fn realm_on(data: GameData) -> Arc<Realm> {
    let mut cfg = crate::game::harness::test_config();
    cfg.everyone_gm = false;
    cfg.max_packets_per_sec = 2000;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(cfg, store, startup, data, None)
}

/// The tests' world, with a slow.
pub(crate) fn data() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    d.spells.spells.insert(
        SLOW,
        spell(SLOW, &[aura_effect(AURA_MOD_DECREASE_SPEED, -50, 0, 1)]),
    );
    d
}

/// A player, Alice, in the world, not a GM, stood at the start.
pub(crate) async fn alice() -> (Arc<Realm>, Client, Guid, MapHandle) {
    let realm = realm_on(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, crate::game::map::MapKey::world(0));
    let gm = on(&map, move |m| m.players[&g].gm).await;
    assert!(!gm, "the check passes GMs");
    (realm, a, g, map)
}

pub(crate) fn at(dx: f32, dy: f32) -> Vector3d {
    Vector3d {
        x: START.0 + dx,
        y: START.1 + dy,
        z: START.2,
    }
}

/// A movement packet of `flags` at `position`.
pub(crate) fn info(flags: MovementInfo_MovementFlags, position: Vector3d) -> MovementInfo {
    MovementInfo {
        flags,
        timestamp: 0,
        position,
        orientation: 0.0,
        fall_time: 0,
    }
}

pub(crate) async fn beat(a: &Client, flags: MovementInfo_MovementFlags, position: Vector3d) {
    a.send(MSG_MOVE_HEARTBEAT_Client {
        info: info(flags, position),
    })
    .await;
}

/// As if `secs` had gone by since the player's last move was taken (and
/// its knockback, if it is in one): the map's clock isn't the test's to
/// wait on.
pub(crate) async fn elapse(map: &MapHandle, g: Guid, secs: f32) {
    on(map, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        crate::game::map::move_guard::backdate_launch(p, Duration::from_secs_f32(secs));
        p.move_check = p.move_check.map(|(pos, at)| {
            (
                pos,
                at.checked_sub(Duration::from_secs_f32(secs)).unwrap_or(at),
            )
        });
    })
    .await;
}

/// Where the server has the player.
pub(crate) async fn server_at(map: &MapHandle, g: Guid) -> Vector3d {
    on(map, move |m| m.players[&g].position()).await
}

/// Whether the client is pulled back (`MSG_MOVE_TELEPORT_ACK`) once what
/// it sent has been handled: where to, acknowledged as a client does.
pub(crate) async fn pulled_back(a: &mut Client) -> Option<Vector3d> {
    crate::game::tests_p3_player2_ammo::handled(a).await;
    let back = a
        .gets(Duration::from_millis(100), |m| match m {
            ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(t) => Some(t.info.position),
            _ => None,
        })
        .await?;
    a.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: a.guid,
        movement_counter: a.teleport_order(),
        time: Default::default(),
    })
    .await;
    crate::game::tests_p3_player2_ammo::handled(a).await;
    Some(back)
}

fn forward() -> MovementInfo_MovementFlags {
    MovementInfo_MovementFlags::new_forward()
}

/// survey 12c: the allowance came again with every packet (0.5 s of run
/// and 3 yards each), so fifty heartbeats a second went at about 370 yards
/// a second. Now it is a bank time fills: thirty 3-yard steps at once go
/// no further than it holds.
#[tokio::test]
async fn heartbeats_sent_faster_earn_no_more_ground() {
    let (_realm, mut a, g, map) = alice().await;
    for i in 1..=30 {
        beat(&a, forward(), at(3.0 * i as f32, 0.0)).await;
    }
    let back = pulled_back(&mut a).await.expect("pulled back");
    assert!(
        back.x - START.0 < 25.0,
        "no further than the bank: {back:?}"
    );
    let x = server_at(&map, g).await.x - START.0;
    assert!(x < 25.0, "the server has them where they were put: {x}");
}

/// A run at run speed, a heartbeat every half second, goes on as long as
/// it lasts; and jumping on the way changes nothing.
#[tokio::test]
async fn a_run_and_its_jumps_pass() {
    let (_realm, mut a, g, map) = alice().await;
    let jump = MovementInfo_MovementFlags::new_jumping(MovementInfo_MovementFlags_Jumping {
        cos_angle: 1.0,
        sin_angle: 0.0,
        xy_speed: 7.0,
        z_speed: 8.0,
    })
    .set_forward();
    for i in 1..=20 {
        elapse(&map, g, 0.5).await;
        let flags = if i % 4 == 0 { jump } else { forward() };
        beat(&a, flags, at(3.5 * i as f32, 0.0)).await;
    }
    assert_eq!(pulled_back(&mut a).await, None);
    let x = server_at(&map, g).await.x - START.0;
    assert!((x - 70.0).abs() < 0.1, "{x}");
}

/// Lag holds a run's packets back and lets them through at once: two and a
/// half seconds of them still pass, as the bank holds two seconds of run
/// and three yards over the tolerance.
#[tokio::test]
async fn a_runs_packets_bunched_up_by_lag_pass() {
    let (_realm, mut a, g, map) = alice().await;
    elapse(&map, g, 2.5).await;
    for i in 1..=5 {
        beat(&a, forward(), at(3.5 * i as f32, 0.0)).await;
    }
    assert_eq!(pulled_back(&mut a).await, None);
    let x = server_at(&map, g).await.x - START.0;
    assert!((x - 17.5).abs() < 0.1, "{x}");
}

/// A long fall goes straight down as fast as it likes: height isn't
/// checked (review 2's M1, accepted).
#[tokio::test]
async fn a_fall_passes() {
    let (_realm, mut a, g, map) = alice().await;
    let falling = MovementInfo_MovementFlags::new_fallingfar();
    for i in 1..=10 {
        let mut p = at(0.5 * i as f32, 0.0);
        p.z -= 20.0 * i as f32;
        beat(&a, falling, p).await;
    }
    assert_eq!(pulled_back(&mut a).await, None);
    let z = server_at(&map, g).await.z;
    assert!((z - (START.2 - 200.0)).abs() < 0.1, "{z}");
}

/// Slowed (Hamstring, Crippling Poison, a Frostbolt), the check goes by
/// the slowed speed. The floor of run speed 7 it had let a client ignore
/// the slow.
#[tokio::test]
async fn a_slowed_player_is_held_to_the_slow() {
    let (_realm, mut a, g, map) = alice().await;
    // Unslowed, fifteen yards in a second pass.
    elapse(&map, g, 1.0).await;
    beat(&a, forward(), at(15.0, 0.0)).await;
    assert_eq!(pulled_back(&mut a).await, None);
    on(&map, move |m| {
        let slow = m.realm.data.spells.get(SLOW).cloned().unwrap();
        auras::apply(m, g, g, &slow, 0, -50);
    })
    .await;
    let mut run = 7.0;
    for _ in 0..200 {
        run = on(&map, move |m| m.players[&g].speeds.run).await;
        if run < 4.0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!((run - 3.5).abs() < 0.01, "slowed: {run}");
    // Slowed by half, the same fifteen yards in a second are too far.
    elapse(&map, g, 1.0).await;
    beat(&a, forward(), at(30.0, 0.0)).await;
    let back = pulled_back(&mut a).await.expect("pulled back");
    assert!((back.x - (START.0 + 15.0)).abs() < 0.01, "{back:?}");
    // And at the slow, it runs on.
    for i in 1..=6 {
        elapse(&map, g, 0.5).await;
        beat(&a, forward(), at(15.0 + 1.75 * i as f32, 0.0)).await;
    }
    assert_eq!(pulled_back(&mut a).await, None);
}

/// Launches `g` at `xy_speed`, acknowledged as the client acknowledges it.
pub(crate) async fn knocked_back(a: &mut Client, map: &MapHandle, g: Guid, xy_speed: f32) {
    knocked_up(a, map, g, xy_speed, -10.0).await;
}

/// Launches `g` at `xy_speed` east and `z_speed` (as sent: negative is
/// up), acknowledged as the client acknowledges it.
pub(crate) async fn knocked_up(
    a: &mut Client,
    map: &MapHandle,
    g: Guid,
    xy_speed: f32,
    z_speed: f32,
) {
    let impulse = [1.0, 0.0, xy_speed, z_speed];
    let counter = on(map, move |m| {
        m.players[&g]
            .outbound
            .order(crate::game::conn::Order::KnockBack { guid: g, impulse })
    })
    .await;
    a.send(CMSG_MOVE_KNOCK_BACK_ACK {
        guid: g,
        counter,
        info: info(jump(xy_speed, z_speed), at(0.0, 0.0)),
    })
    .await;
    // Taken before the test backdates it (`elapse` goes to the map at once,
    // the packet through the session).
    crate::game::tests_p3_player2_ammo::handled(a).await;
}

/// [`beat`], and handled.
async fn beat_handled(a: &mut Client, flags: MovementInfo_MovementFlags, position: Vector3d) {
    beat(a, flags, position).await;
    crate::game::tests_p3_player2_ammo::handled(a).await;
}

fn launch(xy_speed: f32) -> MovementInfo_MovementFlags {
    jump(xy_speed, -10.0)
}

fn jump(xy_speed: f32, z_speed: f32) -> MovementInfo_MovementFlags {
    MovementInfo_MovementFlags::new_jumping(MovementInfo_MovementFlags_Jumping {
        cos_angle: 1.0,
        sin_angle: 0.0,
        xy_speed,
        z_speed,
    })
}

/// `dx` east of the start and `dz` above it.
fn up(dx: f32, dz: f32) -> Vector3d {
    Vector3d {
        z: START.2 + dz,
        ..at(dx, 0.0)
    }
}

/// The client's gravity.
const G: f32 = 19.291_105;

/// A launch of 20 yards a second east and 10 up: its arc is back at its
/// height in 1.04 s; the guard's flight, 2.54 s.
const XY: f32 = 20.0;
const VZ: f32 = 10.0;

/// Where a launch of `XY` and `VZ` has its client `t` seconds in.
fn arc(t: f32) -> Vector3d {
    up(XY * t, VZ * t - G * t * t / 2.0)
}

/// Call 12: a knockback's flight ends at its ballistic time. A legit one,
/// its packets along its arc every quarter second to its landing, is let
/// through, and lands where it says.
#[tokio::test]
async fn a_knockback_along_its_arc_is_let_through() {
    let (_realm, mut a, g, map) = alice().await;
    knocked_up(&mut a, &map, g, XY, -VZ).await;
    let land = 2.0 * VZ / G;
    let mut t = 0.0;
    while t + 0.25 < land {
        elapse(&map, g, 0.25).await;
        t += 0.25;
        beat_handled(&mut a, jump(XY, -VZ), arc(t)).await;
    }
    elapse(&map, g, land - t).await;
    a.send(MSG_MOVE_FALL_LAND_Client {
        info: info(MovementInfo_MovementFlags::empty(), up(XY * land, 0.0)),
    })
    .await;
    assert_eq!(pulled_back(&mut a).await, None);
    let x = server_at(&map, g).await.x - START.0;
    assert!((x - XY * land).abs() < 0.1, "{x}");
}

/// Knocked off a ledge, the client falls on past its arc's time, as fast
/// as its launch flung it: a fall as the arc would have it is still its
/// flight, three and a half seconds of it (the arc alone, 2.54), and
/// it isn't pulled back.
#[tokio::test]
async fn a_knockback_off_a_ledge_falls_on_with_no_correction() {
    let (_realm, mut a, g, map) = alice().await;
    knocked_up(&mut a, &map, g, XY, -VZ).await;
    for i in 1..=7 {
        elapse(&map, g, 0.5).await;
        beat_handled(&mut a, jump(XY, -VZ), arc(0.5 * i as f32)).await;
    }
    a.send(MSG_MOVE_FALL_LAND_Client {
        info: info(MovementInfo_MovementFlags::empty(), arc(3.5)),
    })
    .await;
    assert_eq!(pulled_back(&mut a).await, None);
    let x = server_at(&map, g).await.x - START.0;
    assert!((x - XY * 3.5).abs() < 0.1, "{x}");
}

/// The beats, every half second, of a client going on east at the
/// launch's speed with `flags` at `dz` above the start, for eight seconds,
/// and when the first was pulled back (none: `None`). Once pulled back,
/// the client's moves wait for its acknowledgement, so where it was put
/// tells which.
async fn goes_on(
    a: &mut Client,
    map: &MapHandle,
    g: Guid,
    flags: MovementInfo_MovementFlags,
    dz: f32,
) -> Option<f32> {
    for i in 1..=16 {
        elapse(map, g, 0.5).await;
        beat_handled(a, flags, up(XY * 0.5 * i as f32, dz)).await;
    }
    let back = pulled_back(a).await?;
    Some((back.x - START.0) / XY + 0.5)
}

/// A client whose arc has landed, saying it still falls at the launch's
/// height and going on at its speed, is held back once it is past the arc
/// and a fall's lag (2 s) late: the flight lasted until it said it landed,
/// up to ten seconds.
#[tokio::test]
async fn going_on_at_the_launchs_speed_past_its_landing_is_held_back() {
    let (_realm, mut a, g, map) = alice().await;
    knocked_up(&mut a, &map, g, XY, -VZ).await;
    let t = goes_on(
        &mut a,
        &map,
        g,
        MovementInfo_MovementFlags::new_fallingfar(),
        0.0,
    )
    .await;
    let t = t.expect("held back");
    assert!((t - 3.5).abs() < 0.01, "at {t} s");
}

/// A client that says its launch flies on for eight seconds, five yards
/// over where it began, is refused at its first packet past the arc.
#[tokio::test]
async fn a_launch_claimed_for_eight_seconds_is_refused_past_its_arc() {
    let (_realm, mut a, g, map) = alice().await;
    knocked_up(&mut a, &map, g, XY, -VZ).await;
    let t = goes_on(&mut a, &map, g, jump(XY, -VZ), 5.0).await;
    let t = t.expect("refused");
    assert!((t - 3.0).abs() < 0.01, "at {t} s: the arc ends at 2.54");
}

/// review 5's #3: a knockback flung its player faster than they run, and
/// the check pulled them back mid-flight. In flight they go at the
/// launch's speed (mangos's `IsLaunched`), until they land; then at their
/// own again.
#[tokio::test]
async fn a_knockbacks_flight_passes_until_it_lands() {
    let (_realm, mut a, g, map) = alice().await;
    knocked_back(&mut a, &map, g, 30.0).await;
    // Fifteen yards in half a second, then nine and the landing.
    elapse(&map, g, 0.5).await;
    beat_handled(&mut a, launch(30.0), at(15.0, 0.0)).await;
    elapse(&map, g, 0.3).await;
    a.send(MSG_MOVE_FALL_LAND_Client {
        info: info(MovementInfo_MovementFlags::empty(), at(24.0, 0.0)),
    })
    .await;
    assert_eq!(pulled_back(&mut a).await, None);
    let x = server_at(&map, g).await.x - START.0;
    assert!((x - 24.0).abs() < 0.1, "{x}");
    // Landed, thirty yards in half a second is too far.
    elapse(&map, g, 0.5).await;
    beat(&a, forward(), at(54.0, 0.0)).await;
    let back = pulled_back(&mut a).await.expect("pulled back");
    assert!((back.x - (START.0 + 24.0)).abs() < 0.01, "{back:?}");
}
