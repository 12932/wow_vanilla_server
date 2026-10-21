//! Round 12c's roots and stuns: a client's move under one is refused, and
//! a client that went anywhere is put back (mangos's `VerifyMovementInfo`);
//! a root's order on its way, a turn in place and a fall are let through.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::harness::Client;
use crate::game::map::MapHandle;
use crate::game::map::auras::{self, AURA_MOD_ROOT, AURA_MOD_STUN};
use crate::game::tests_p3_combat::START;
use crate::game::tests_p3_entity::{aura_effect, on, spell};
use crate::game::tests_p12c_speed::{at, beat, info, pulled_back, realm_on, server_at};

const ROOT: u32 = 97_821;
const STUN: u32 = 97_822;

fn data() -> GameData {
    let mut d = crate::game::tests_p12c_speed::data();
    for (id, aura) in [(ROOT, AURA_MOD_ROOT), (STUN, AURA_MOD_STUN)] {
        d.spells
            .spells
            .insert(id, spell(id, &[aura_effect(aura, 1, 0, 1)]));
    }
    d
}

/// Alice, not a GM, at the start, and Bob beside her, watching.
async fn two() -> (
    Client,
    Guid,
    Client,
    MapHandle,
    std::sync::Arc<crate::game::Realm>,
) {
    let realm = realm_on(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let (b, _) = crate::game::tests_p3_entity::bob(&realm, g).await;
    let map = realm.maps.get(&realm, crate::game::map::MapKey::world(0));
    (a, g, b, map, realm)
}

/// `spell` on `g`, and the order it sends: its counter.
async fn held_by(a: &mut Client, map: &MapHandle, g: Guid, spell: u32) -> u32 {
    on(map, move |m| {
        let s = m.realm.data.spells.get(spell).cloned().unwrap();
        auras::apply(m, g, g, &s, 0, 1);
    })
    .await;
    a.wait_for("SMSG_FORCE_MOVE_ROOT", |m| match m {
        ServerOpcodeMessage::SMSG_FORCE_MOVE_ROOT(r) if r.guid == g => Some(r.counter),
        _ => None,
    })
    .await
}

/// The root acknowledged where the client stands.
async fn acknowledged(a: &Client, g: Guid, counter: u32) {
    a.send(CMSG_FORCE_MOVE_ROOT_ACK {
        guid: g,
        movement_counter: counter,
        info: info(MovementInfo_MovementFlags::new_root(), at(0.0, 0.0)),
    })
    .await;
}

fn forward() -> MovementInfo_MovementFlags {
    MovementInfo_MovementFlags::new_forward()
}

/// Whether `b` is told of Alice's heartbeat at `x` yards along.
async fn told_at(b: &mut Client, x: f32) -> bool {
    b.gets(Duration::from_millis(150), |m| match m {
        ServerOpcodeMessage::MSG_MOVE_HEARTBEAT(h)
            if (h.info.position.x - (START.0 + x)).abs() < 0.01 =>
        {
            Some(())
        }
        _ => None,
    })
    .await
    .is_some()
}

/// survey 12c: a root or stun sent its order and waited for the ack, but
/// moves that came meanwhile or instead were taken and told on. Frost
/// Nova's root, acknowledged, holds: a client that runs on is refused,
/// those who watch aren't told, and it is put back.
#[tokio::test]
async fn a_rooted_clients_run_is_refused_and_put_back() {
    let (mut a, g, mut b, map, _realm) = two().await;
    let counter = held_by(&mut a, &map, g, ROOT).await;
    acknowledged(&a, g, counter).await;
    beat(&a, forward(), at(5.0, 0.0)).await;
    let back = pulled_back(&mut a).await.expect("put back");
    assert!((back.x - START.0).abs() < 0.01, "{back:?}");
    assert!(!told_at(&mut b, 5.0).await, "the run isn't told on");
    let x = server_at(&map, g).await.x - START.0;
    assert!(x.abs() < 0.01, "{x}");
    // A step under the allowance is refused, and not put back.
    beat(&a, forward(), at(2.0, 0.0)).await;
    assert_eq!(pulled_back(&mut a).await, None);
    let x = server_at(&map, g).await.x - START.0;
    assert!(x.abs() < 0.01, "{x}");
}

/// A stun holds as a root does (Hammer of Justice, Kidney Shot).
#[tokio::test]
async fn a_stunned_clients_run_is_refused() {
    let (mut a, g, _b, map, _realm) = two().await;
    let counter = held_by(&mut a, &map, g, STUN).await;
    acknowledged(&a, g, counter).await;
    beat(&a, forward(), at(5.0, 0.0)).await;
    assert!(pulled_back(&mut a).await.is_some());
    let x = server_at(&map, g).await.x - START.0;
    assert!(x.abs() < 0.01, "{x}");
}

/// A client that never acknowledges its root is held once the order has
/// had its time to arrive; until then, it hasn't heard of it.
#[tokio::test]
async fn a_root_never_acknowledged_holds_after_its_grace() {
    let (mut a, g, mut b, map, _realm) = two().await;
    held_by(&mut a, &map, g, ROOT).await;
    beat(&a, forward(), at(1.0, 0.0)).await;
    assert_eq!(pulled_back(&mut a).await, None);
    assert!(told_at(&mut b, 1.0).await, "the order is on its way");
    on(&map, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        crate::game::map::move_guard::backdate_held(p, Duration::from_secs(2));
    })
    .await;
    beat(&a, forward(), at(6.0, 0.0)).await;
    let back = pulled_back(&mut a).await.expect("put back");
    assert!((back.x - (START.0 + 1.0)).abs() < 0.01, "{back:?}");
}

/// Rooted, a player still turns where they stand and falls: only a step
/// across is refused. Freed, they run again.
#[tokio::test]
async fn a_rooted_player_turns_and_falls_and_runs_once_freed() {
    let (mut a, g, mut b, map, _realm) = two().await;
    let counter = held_by(&mut a, &map, g, ROOT).await;
    acknowledged(&a, g, counter).await;
    a.send(MSG_MOVE_HEARTBEAT_Client {
        info: MovementInfo {
            orientation: 2.0,
            ..info(
                MovementInfo_MovementFlags::new_root().set_turn_left(),
                at(0.0, 0.0),
            )
        },
    })
    .await;
    assert_eq!(pulled_back(&mut a).await, None);
    let o = on(&map, move |m| m.players[&g].movement.orientation).await;
    assert!((o - 2.0).abs() < 0.01, "turned: {o}");
    let mut down = at(0.0, 0.0);
    down.z -= 4.0;
    beat(&a, MovementInfo_MovementFlags::new_fallingfar(), down).await;
    assert_eq!(pulled_back(&mut a).await, None);
    let z = server_at(&map, g).await.z;
    assert!((z - (START.2 - 4.0)).abs() < 0.01, "fell: {z}");
    on(&map, move |m| auras::remove_spell(m, g, ROOT, None)).await;
    beat(&a, forward(), at(3.0, 0.0)).await;
    assert_eq!(pulled_back(&mut a).await, None);
    assert!(told_at(&mut b, 3.0).await, "freed, the run is told on");
}
