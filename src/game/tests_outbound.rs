//! The outbound path through the protocol: a map's corked run of movement
//! packets reaches each observer whole and in order, and what follows it
//! uncorked comes after it; `[visibility] distant_move_yards` thins the
//! heartbeats only of observers past it, and only heartbeats.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::harness::{self, Client};
use crate::game::map::movement;
use crate::game::tests_p3_entity::on;
use crate::game::{Realm, RealmConfig};

fn realm(cfg: RealmConfig) -> Arc<Realm> {
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(
        cfg,
        store,
        startup,
        crate::game::data::GameData::empty(),
        None,
    )
}

fn info(x: f32, y: f32, z: f32, flags: MovementInfo_MovementFlags) -> MovementInfo {
    MovementInfo {
        flags,
        timestamp: 0,
        position: Vector3d { x, y, z },
        orientation: 0.0,
        fall_time: 0,
    }
}

/// Alice, Bob beside her and Carol `carol_yards` east of her, all three
/// seeing each other.
async fn three(realm: &Arc<Realm>, carol_yards: f32) -> ([Client; 3], [Guid; 3]) {
    let mut a = Client::connect(realm, "OUTA");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(realm, "OUTB");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let mut c = Client::connect(realm, "OUTC");
    let gc = c.enter("Carol", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(ga).unwrap().map();
    on(&map, move |m| {
        let at = m.players[&ga].position();
        let to = Vector3d {
            x: at.x + carol_yards,
            ..at
        };
        movement::teleport_within(m, gc, to, 0.0);
    })
    .await;
    c.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: gc,
        movement_counter: c.teleport_order(),
        time: Default::default(),
    })
    .await;
    for (cl, others) in [(&mut a, [gb, gc]), (&mut b, [ga, gc]), (&mut c, [ga, gb])] {
        for o in others {
            cl.sees(o).await;
        }
    }
    for cl in [&mut a, &mut b, &mut c] {
        cl.clear();
    }
    ([a, b, c], [ga, gb, gc])
}

/// What `cl` was told of `mover`'s movement up to and with their stop:
/// each packet's opcode, in order.
async fn moves_until_stop(cl: &mut Client, mover: Guid) -> Vec<u16> {
    cl.wait_for("the stop", |m| match m {
        ServerOpcodeMessage::MSG_MOVE_STOP(s) if s.guid == mover => Some(()),
        _ => None,
    })
    .await;
    let mut out: Vec<u16> = cl
        .backlog
        .iter()
        .filter_map(|m| match m {
            ServerOpcodeMessage::MSG_MOVE_HEARTBEAT(h) if h.guid == mover => Some(0xEE),
            ServerOpcodeMessage::MSG_MOVE_START_FORWARD(h) if h.guid == mover => Some(0xB5),
            ServerOpcodeMessage::MSG_MOVE_JUMP(h) if h.guid == mover => Some(0xBB),
            _ => None,
        })
        .collect();
    out.push(0xB7);
    out
}

/// Alice runs: a start, six heartbeats, a jump among them, then a stop.
/// Each heartbeat handled before the next is sent: one still waiting
/// would be coalesced into its successor (the mailbox's newest-wins).
async fn run(a: &mut Client, at: Vector3d) {
    let none = MovementInfo_MovementFlags::empty();
    a.send(MSG_MOVE_START_FORWARD_Client {
        info: info(at.x, at.y, at.z, none.set_forward()),
    })
    .await;
    for i in 0..6 {
        a.send(MSG_MOVE_HEARTBEAT_Client {
            info: info(at.x, at.y + 0.1 * i as f32, at.z, none.set_forward()),
        })
        .await;
        crate::game::tests_p3_player2_ammo::handled(a).await;
        if i == 2 {
            a.send(MSG_MOVE_JUMP_Client {
                info: info(at.x, at.y + 0.25, at.z, none.set_forward()),
            })
            .await;
        }
    }
    a.send(MSG_MOVE_STOP_Client {
        info: info(at.x, at.y + 0.6, at.z, none),
    })
    .await;
}

const RUN: [u16; 9] = [0xB5, 0xEE, 0xEE, 0xEE, 0xBB, 0xEE, 0xEE, 0xEE, 0xB7];

/// Off by default: an observer far off gets every heartbeat, in order,
/// as one beside the mover does (the run corked, each pushed in one go).
#[tokio::test]
async fn every_observer_gets_every_movement_packet_in_order_by_default() {
    let realm = realm(harness::test_config());
    let ([mut a, mut b, mut c], [ga, ..]) = three(&realm, 60.0).await;
    let map = realm.registry.online(ga).unwrap().map();
    let at = on(&map, move |m| m.players[&ga].position()).await;
    run(&mut a, at).await;
    assert_eq!(moves_until_stop(&mut b, ga).await, RUN);
    assert_eq!(moves_until_stop(&mut c, ga).await, RUN);
    // What follows the corked run, uncorked, comes after it.
    run(&mut a, at).await;
    a.say("after the run").await;
    stop_before_say(&mut b, ga).await;
}

/// With a burst's movement kept and worked out at once (`[world]
/// parallel_moves`, `map::moves`), every observer still gets every packet,
/// in order, and what follows uncorked after it.
#[tokio::test]
async fn kept_for_the_bursts_end_every_movement_packet_comes_in_order() {
    let realm = realm(RealmConfig {
        parallel_moves: true,
        ..harness::test_config()
    });
    let ([mut a, mut b, mut c], [ga, ..]) = three(&realm, 60.0).await;
    let map = realm.registry.online(ga).unwrap().map();
    let at = on(&map, move |m| m.players[&ga].position()).await;
    run(&mut a, at).await;
    assert_eq!(moves_until_stop(&mut b, ga).await, RUN);
    assert_eq!(moves_until_stop(&mut c, ga).await, RUN);
    run(&mut a, at).await;
    a.say("after the run").await;
    stop_before_say(&mut b, ga).await;
}

/// On, an observer past the yards gets one in `every` heartbeats, and
/// every start, jump and stop; one within them gets everything.
#[tokio::test]
async fn distant_observers_get_one_in_n_heartbeats_and_every_other_move() {
    let realm = realm(RealmConfig {
        distant_move_yards: 30.0,
        distant_move_every: 3,
        ..harness::test_config()
    });
    let ([mut a, mut b, mut c], [ga, ..]) = three(&realm, 60.0).await;
    let map = realm.registry.online(ga).unwrap().map();
    let at = on(&map, move |m| m.players[&ga].position()).await;
    run(&mut a, at).await;
    assert_eq!(moves_until_stop(&mut b, ga).await, RUN, "beside her");
    // Her third and sixth heartbeats.
    assert_eq!(
        moves_until_stop(&mut c, ga).await,
        [0xB5, 0xEE, 0xBB, 0xEE, 0xB7],
        "far off"
    );
}

/// The kept sets follow who sees whom: one who left gets no more, their
/// index going to the next to come, who is told of the mover's moves only
/// once introduced to them.
#[tokio::test]
async fn a_broadcast_follows_comings_and_goings() {
    let realm = realm(harness::test_config());
    let mut a = Client::connect(&realm, "OUTA");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "OUTB");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    b.sees(ga).await;
    a.sees(gb).await;
    let map = realm.registry.online(ga).unwrap().map();
    let at = on(&map, move |m| m.players[&ga].position()).await;
    run(&mut a, at).await;
    assert_eq!(moves_until_stop(&mut b, ga).await, RUN);
    let index = on(&map, move |m| m.clients.index_of(gb)).await;
    b.disconnect();
    harness::offline(&realm, gb).await;
    run(&mut a, at).await;
    let mut c = Client::connect(&realm, "OUTC");
    let gc = c.enter("Carol", Race::Human, Class::Warrior).await;
    // Told of her, then of her moves.
    let first = c
        .wait_for("Alice, or her moves", |m| match m {
            ServerOpcodeMessage::MSG_MOVE_HEARTBEAT(h) if h.guid == ga => Some(false),
            ServerOpcodeMessage::MSG_MOVE_STOP(h) if h.guid == ga => Some(false),
            ServerOpcodeMessage::SMSG_UPDATE_OBJECT(_)
            | ServerOpcodeMessage::SMSG_COMPRESSED_UPDATE_OBJECT(_) => Some(true),
            _ => None,
        })
        .await;
    assert!(first, "a move before her create");
    c.sees(ga).await;
    c.clear();
    run(&mut a, at).await;
    assert_eq!(moves_until_stop(&mut c, ga).await, RUN);
    let carol = on(&map, move |m| m.clients.index_of(gc)).await;
    assert_eq!(carol, index, "Bob's index, Carol's now");
}

/// Batched (`[network] broadcast_batch_ms`), a run reaches each observer
/// whole and in order, and what follows it comes after it.
#[tokio::test]
async fn a_batched_run_reaches_each_observer_in_order() {
    let realm = realm(RealmConfig {
        broadcast_batch: std::time::Duration::from_millis(20),
        ..harness::test_config()
    });
    let ([mut a, mut b, mut c], [ga, ..]) = three(&realm, 60.0).await;
    let map = realm.registry.online(ga).unwrap().map();
    let at = on(&map, move |m| m.players[&ga].position()).await;
    run(&mut a, at).await;
    a.say("after the run").await;
    assert_eq!(moves_until_stop(&mut c, ga).await, RUN);
    // Bob is near enough to hear it (Carol isn't), after the run.
    stop_before_say(&mut b, ga).await;
}

/// Of `mover`'s stop and a say, the stop came first.
async fn stop_before_say(cl: &mut Client, mover: Guid) {
    let stop_first = cl
        .wait_for("the stop or the say", |m| match m {
            ServerOpcodeMessage::MSG_MOVE_STOP(s) if s.guid == mover => Some(true),
            ServerOpcodeMessage::SMSG_MESSAGECHAT(_) => Some(false),
            _ => None,
        })
        .await;
    assert!(stop_first, "the say overtook the run");
}
