//! The way from a client to its map (`Session::heartbeat`, `Session::room`):
//! a mover's heartbeats that wait for a busy map are coalesced, the newest
//! taking the place of the last, while every other packet keeps its place
//! and no heartbeat goes ahead of one; and a client's share of the map's
//! mailbox is bounded (`[network] max_queued_packets`), the session reading
//! no more of it while the share is full, and losing nothing.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::harness::{self, Client};
use crate::game::map::MapHandle;
use crate::game::stats::{COALESCED, HELD_BACK};

/// A realm whose clients may each have `queued` packets waiting for their
/// map.
fn realm(queued: usize) -> Arc<Realm> {
    let mut config = harness::test_config();
    config.max_queued_packets = queued;
    let (store, startup) = crate::game::store::Store::open(":memory:").expect("a store");
    Realm::new(
        config,
        store,
        startup,
        crate::game::data::GameData::empty(),
        None,
    )
}

/// Alice and Bob, side by side, Bob seeing Alice: their clients, Alice's
/// GUID, her map, and where she stands.
async fn pair(realm: &Arc<Realm>) -> (Client, Client, Guid, MapHandle, Vector3d) {
    let mut a = Client::connect(realm, "ALICE");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(realm, "BOB");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    b.sees(ga).await;
    let map = realm.registry.online(ga).expect("online").map();
    let at = map
        .query(move |m| m.players[&ga].position())
        .await
        .expect("the map answers");
    a.clear();
    b.clear();
    (a, b, ga, map, at)
}

/// Hold the map's actor until the returned sender is dropped or sent to
/// (at most 20 s): what is sent to it meanwhile waits in its mailbox.
async fn hold(map: &MapHandle) -> std::sync::mpsc::Sender<()> {
    let (release, wait) = std::sync::mpsc::channel::<()>();
    let (held, is_held) = tokio::sync::oneshot::channel();
    map.call(move |_| {
        let _ = held.send(());
        let _ = wait.recv_timeout(Duration::from_secs(20));
    });
    is_held.await.expect("the map holds");
    release
}

/// Wait until `counter` has reached `n`.
async fn reaches(counter: &std::sync::atomic::AtomicU64, n: u64, what: &str) {
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    while counter.load(Ordering::Relaxed) < n {
        assert!(tokio::time::Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

fn info(at: Vector3d, dx: f32, forward: bool) -> MovementInfo {
    MovementInfo {
        flags: if forward {
            MovementInfo_MovementFlags::new_forward()
        } else {
            MovementInfo_MovementFlags::empty()
        },
        timestamp: 0,
        position: Vector3d { x: at.x + dx, ..at },
        orientation: 0.0,
        fall_time: 0,
    }
}

/// Alice's movement as Bob is told it: the opcode's name and how far along
/// she is, in order, up to and with the one `last` takes.
async fn told(
    b: &mut Client,
    ga: Guid,
    at: Vector3d,
    last: impl Fn(&str, f32) -> bool,
) -> Vec<(&'static str, f32)> {
    let mut out = Vec::new();
    loop {
        let (name, dx) = b
            .wait_for("Alice's movement", |m| {
                let (name, guid, info) = match m {
                    ServerOpcodeMessage::MSG_MOVE_HEARTBEAT(s) => ("heartbeat", s.guid, s.info),
                    ServerOpcodeMessage::MSG_MOVE_START_FORWARD(s) => ("start", s.guid, s.info),
                    ServerOpcodeMessage::MSG_MOVE_STOP(s) => ("stop", s.guid, s.info),
                    ServerOpcodeMessage::MSG_MOVE_JUMP(s) => ("jump", s.guid, s.info),
                    _ => return None,
                };
                (guid == ga).then(|| (name, ((info.position.x - at.x) * 100.0).round() / 100.0))
            })
            .await;
        out.push((name, dx));
        if last(name, dx) {
            return out;
        }
    }
}

/// Three heartbeats sent while the map is busy: the map takes the last,
/// once, and those who see the mover are told it alone. The speed check
/// judges it by where the mover was last taken and when: not too fast.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn heartbeats_waiting_for_a_busy_map_are_coalesced_to_the_newest() {
    let realm = realm(64);
    let (a, mut b, ga, map, at) = pair(&realm).await;
    let release = hold(&map).await;
    for dx in [1.0, 2.0, 3.0] {
        a.send(MSG_MOVE_HEARTBEAT_Client {
            info: info(at, dx, true),
        })
        .await;
    }
    reaches(&COALESCED, 2, "the second and third heartbeats coalesced").await;
    drop(release);
    let seen = told(&mut b, ga, at, |_, dx| dx == 3.0).await;
    assert_eq!(seen, [("heartbeat", 3.0)], "the newest alone");
    let there = map
        .query(move |m| m.players[&ga].position().x)
        .await
        .expect("the map answers");
    assert!(
        (there - (at.x + 3.0)).abs() < 0.01,
        "taken where the newest has her, not pulled back: {there} (from {})",
        at.x
    );
    // The map took it: the next goes as before, in its own turn.
    a.send(MSG_MOVE_HEARTBEAT_Client {
        info: info(at, 4.0, true),
    })
    .await;
    let seen = told(&mut b, ga, at, |_, dx| dx == 4.0).await;
    assert_eq!(seen, [("heartbeat", 4.0)]);
    assert_eq!(COALESCED.load(Ordering::Relaxed), 2);
}

/// A heartbeat that waits with another packet behind it is never replaced:
/// one sent after keeps its place behind that packet, and every other
/// movement packet keeps its place. Here nothing is coalesced, and Bob is
/// told all in the order Alice sent it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_heartbeat_never_goes_ahead_of_another_packet() {
    // Room for four: the fifth waits, so the four are known to be queued.
    let realm = realm(4);
    let (a, mut b, ga, map, at) = pair(&realm).await;
    let release = hold(&map).await;
    a.send(MSG_MOVE_HEARTBEAT_Client {
        info: info(at, 1.0, true),
    })
    .await;
    a.send(MSG_MOVE_JUMP_Client {
        info: info(at, 2.0, true),
    })
    .await;
    a.send(MSG_MOVE_HEARTBEAT_Client {
        info: info(at, 3.0, true),
    })
    .await;
    a.send(MSG_MOVE_HEARTBEAT_Client {
        info: info(at, 4.0, true),
    })
    .await;
    a.send(MSG_MOVE_STOP_Client {
        info: info(at, 5.0, false),
    })
    .await;
    reaches(&HELD_BACK, 1, "the first four queued, the stop held back").await;
    drop(release);
    let seen = told(&mut b, ga, at, |name, _| name == "stop").await;
    assert_eq!(
        seen,
        [
            ("heartbeat", 1.0),
            ("jump", 2.0),
            ("heartbeat", 3.0),
            ("heartbeat", 4.0),
            ("stop", 5.0),
        ],
        "in the order sent, nothing coalesced past the jump"
    );
    assert_eq!(COALESCED.load(Ordering::Relaxed), 0);
}

/// A client whose share of the map's mailbox is full is read no further
/// until the map takes its packets: the session waits (counted), and what
/// it sends after is neither lost nor reordered.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_full_share_of_the_mailbox_holds_the_client_back_and_loses_nothing() {
    let realm = realm(8);
    let (a, mut b, ga, map, at) = pair(&realm).await;
    let release = hold(&map).await;
    // Twenty movement packets, none a heartbeat (start and stop by turns):
    // eight wait in the mailbox, the ninth waits in the session.
    let sender = tokio::spawn(async move {
        for i in 1..=20 {
            let dx = i as f32 * 0.25;
            if i % 2 == 1 {
                a.send(MSG_MOVE_START_FORWARD_Client {
                    info: info(at, dx, true),
                })
                .await;
            } else {
                a.send(MSG_MOVE_STOP_Client {
                    info: info(at, dx, false),
                })
                .await;
            }
        }
        a
    });
    // (The client's own sends go on into the session's inbound queue, as a
    // socket's reader's would until it is full: the session takes no more
    // from it.)
    reaches(&HELD_BACK, 1, "the session held back").await;
    drop(release);
    let _a = sender.await.expect("the client sends");
    let seen = told(&mut b, ga, at, |_, dx| dx == 5.0).await;
    let expect: Vec<(&str, f32)> = (1..=20)
        .map(|i| (if i % 2 == 1 { "start" } else { "stop" }, i as f32 * 0.25))
        .collect();
    assert_eq!(seen, expect, "every packet, in order");
}

/// A heartbeat that says otherwise of the mover's state (its flags: here
/// the run ends) doesn't take the place of the one waiting: the map sees
/// both, as a fall's last heartbeat and its landing must be seen.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_heartbeat_with_other_flags_is_not_coalesced() {
    let realm = realm(64);
    let (a, mut b, ga, map, at) = pair(&realm).await;
    let release = hold(&map).await;
    a.send(MSG_MOVE_HEARTBEAT_Client {
        info: info(at, 1.0, true),
    })
    .await;
    a.send(MSG_MOVE_HEARTBEAT_Client {
        info: info(at, 2.0, true),
    })
    .await;
    a.send(MSG_MOVE_HEARTBEAT_Client {
        info: info(at, 3.0, false),
    })
    .await;
    // The third goes behind it, whenever the map is let go: as a packet
    // after the marker, or as the next marker once the map has taken it.
    reaches(&COALESCED, 1, "the second coalesced").await;
    drop(release);
    let seen = told(&mut b, ga, at, |_, dx| dx == 3.0).await;
    assert_eq!(seen, [("heartbeat", 2.0), ("heartbeat", 3.0)]);
    assert_eq!(COALESCED.load(Ordering::Relaxed), 1);
}
