//! Review 5's findings on the map actors and their lifecycle (rev5-maps in
//! docs/review/review5.md), each shown through the protocol: a copy stopped
//! with a player on the way in, a panic in a map's shutdown, a restart's
//! sweep of dungeon copies, a teleport in a logout's countdown, and those
//! let go with a map leaving the social world.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::scripts::MapTemplate;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::instances;
use crate::game::map::{Arrival, ArrivalKind, Map, MapHandle, MapKey};

const DUNGEON: u32 = 36;

fn dungeon() -> GameData {
    let mut d = GameData::empty();
    d.maps.insert(
        DUNGEON,
        MapInfo {
            id: DUNGEON,
            internal_name: "Deadmines".into(),
            name: "The Deadmines".into(),
            instance_type: 1,
            player_limit: 0,
        },
    );
    d.scripts.map_templates.insert(
        DUNGEON,
        MapTemplate {
            map: DUNGEON,
            map_type: 1,
            player_limit: 5,
            ..Default::default()
        },
    );
    d
}

fn realm_on(store: crate::game::store::Store, startup: crate::game::store::Startup) -> Arc<Realm> {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    cfg.instance_unload = Duration::from_secs(600);
    Realm::new(cfg, store, startup, dungeon(), None)
}

fn realm() -> Arc<Realm> {
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    realm_on(store, startup)
}

fn map_of(realm: &Realm, g: Guid) -> MapHandle {
    realm.registry.online(g).unwrap().map()
}

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map answers")
}

/// Teleport `c` to `map` (their copy of it) and see them in.
async fn go(realm: &Arc<Realm>, c: &mut Client, map: u32) {
    let g = c.guid;
    map_of(realm, g).call(move |m| m.teleport(g, MapKey::world(map), Vector3d::default(), 0.0));
    c.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    c.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    c.sees(g).await;
}

async fn until(what: &str, mut f: impl FnMut() -> bool) {
    for _ in 0..500 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("never: {what}");
}

async fn join_peon(realm: &Realm, c: &Client) {
    c.send(CMSG_JOIN_CHANNEL {
        channel_name: "peon".into(),
        channel_password: String::new(),
    })
    .await;
    let g = c.guid;
    until("in peon", || !realm.social.channels.lock().of(g).is_empty()).await;
}

async fn money_saved(realm: &Realm, g: Guid) -> u32 {
    realm.store.flush().await;
    realm
        .store
        .load(crate::game::guid::counter(g))
        .await
        .unwrap()
        .money
}

/// Review 5's maps 1 (b): a group's leader resets its copy as a member is
/// on their way into it (their arrival in the copy's queue, their online
/// entry still naming the map they left). The reset waits for them, as
/// mangos's "zoning" answer, and they arrive. Before, the copy looked
/// empty: it was reset and stopped with the arrival in its queue, and the
/// member was dropped with it, unsaved.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_reset_waits_for_a_member_on_the_way_in() {
    let realm = realm();
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let (ao, bo) = (
        realm.registry.online(a.guid).unwrap(),
        realm.registry.online(gb).unwrap(),
    );
    assert!(crate::game::social::groups::join_directly(&realm, &ao, &bo));
    // Alice makes the group's copy and comes out: it is empty.
    go(&realm, &mut a, DUNGEON).await;
    let key = map_of(&realm, a.guid).key;
    assert_ne!(key.instance, 0);
    go(&realm, &mut a, 0).await;
    let copy = realm.maps.get(&realm, key);
    // The copy is busy a moment (a long update, a burst of events), and
    // Bob sets out for it.
    copy.call(|_| std::thread::sleep(Duration::from_millis(400)));
    map_of(&realm, gb)
        .call(move |m| m.teleport(gb, MapKey::world(DUNGEON), Vector3d::default(), 0.0));
    b.wait_for("SMSG_TRANSFER_PENDING", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_TRANSFER_PENDING(_)).then_some(())
    })
    .await;
    a.send(CMSG_RESET_INSTANCES {}).await;
    let answer = a
        .wait_for("the reset's answer", |m| match m {
            ServerOpcodeMessage::SMSG_INSTANCE_RESET(_) => Some(None),
            ServerOpcodeMessage::SMSG_INSTANCE_RESET_FAILED(f) => Some(Some(f.reason)),
            _ => None,
        })
        .await;
    assert_eq!(
        answer,
        Some(InstanceResetFailedReason::Zoning),
        "refused: Bob is on his way in"
    );
    b.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    b.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    b.sees(gb).await;
    let now = map_of(&realm, gb);
    assert_eq!(now.key, key, "Bob is in the copy");
    assert!(!now.is_closed());
    assert!(on(&now, move |m| m.players.contains_key(&gb)).await);
    assert!(
        instances::save_of(&realm, key.instance).is_some(),
        "its save kept"
    );
    // Inside, he still keeps it from a reset.
    a.send(CMSG_RESET_INSTANCES {}).await;
    let answer = a
        .wait_for("the reset's answer", |m| match m {
            ServerOpcodeMessage::SMSG_INSTANCE_RESET(_) => Some(None),
            ServerOpcodeMessage::SMSG_INSTANCE_RESET_FAILED(f) => Some(Some(f.reason)),
            _ => None,
        })
        .await;
    assert_eq!(answer, Some(InstanceResetFailedReason::General));
}

/// Review 5's maps 1 (a): a copy stopped from outside with a
/// player's arrival ahead of the stop in its queue (a reset or a raid's
/// reset that couldn't see them) doesn't take them with it: they are saved
/// and go back to the character screen, as at a map's restart, out of their
/// channels. Before, the actor ended with them inside: what they had since
/// their last save was lost, and they stayed online, in no map, and in
/// their channels.
#[tokio::test]
async fn a_copy_stopped_with_an_arrival_in_its_queue_saves_the_player() {
    let realm = realm();
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    join_peon(&realm, &b).await;
    let key = MapKey {
        map: DUNGEON,
        instance: 7,
    };
    let copy = realm.maps.get(&realm, key);
    // Bob has earned something since his last save, and leaves his map
    // for the copy.
    let mut bob = on(&map_of(&realm, gb), move |m| m.take_player(gb))
        .await
        .expect("Bob");
    bob.fields.set_u32(PLAYER_FIELD_COINAGE, 12345);
    let online = bob.online.clone();
    // His arrival is queued, and the stop behind it.
    on(&copy, move |m| {
        m.handle
            .send(crate::game::map::MapEvent::Arrive(Box::new(Arrival {
                kind: ArrivalKind::Transfer(Box::new(bob)),
                online,
            })));
        m.realm.maps.stop(m.key);
    })
    .await;
    b.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    until("Bob offline", || realm.registry.online(gb).is_none()).await;
    until("the copy stopped", || copy.is_closed()).await;
    assert_eq!(money_saved(&realm, gb).await, 12345, "saved");
    assert!(
        realm.social.channels.lock().of(gb).is_empty(),
        "out of his channels"
    );
    // He comes back from the character screen (home: the copy has no save).
    b.login(gb).await;
    assert_eq!(map_of(&realm, gb).key, MapKey::world(0));
}
