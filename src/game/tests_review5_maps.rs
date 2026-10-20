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

/// Review 5's maps 6: a player let go by a map that won't take them (a copy
/// its raid's reset took, as they were sent its way) is saved and goes
/// offline as at a logout: out of their channels. Before, the online list
/// alone: they stayed in their channels, and so in their battleground
/// queues, online to their friends and group.
#[tokio::test]
async fn a_player_let_go_by_a_copy_that_has_gone_leaves_their_channels() {
    let realm = realm();
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    join_peon(&realm, &b).await;
    let key = MapKey {
        map: DUNGEON,
        instance: 9,
    };
    realm.social.instances.lock().saves.insert(
        key.instance,
        instances::InstanceSave {
            id: key.instance,
            map: DUNGEON,
            expired: true,
            ..Default::default()
        },
    );
    let stale = realm.maps.get(&realm, key);
    realm.maps.stop(key);
    until("the copy stopped", || stale.is_closed()).await;
    let mut bob = on(&map_of(&realm, gb), move |m| m.take_player(gb))
        .await
        .expect("Bob");
    bob.fields.set_u32(PLAYER_FIELD_COINAGE, 4321);
    let online = bob.online.clone();
    let took = crate::game::map::deliver(
        &realm,
        stale,
        Box::new(Arrival {
            kind: ArrivalKind::Transfer(Box::new(bob)),
            online,
        }),
    );
    assert!(took.is_none(), "let go");
    assert!(realm.registry.online(gb).is_none(), "offline");
    assert_eq!(money_saved(&realm, gb).await, 4321, "saved");
    assert!(
        realm.social.channels.lock().of(gb).is_empty(),
        "out of his channels"
    );
}

/// Review 5's maps 2: a panic as one player is taken out of the map at the
/// shutdown, and another's in their save, cost those two their saves (they
/// go as last saved), not the map's: the other two are saved, and all four
/// go offline, so the shutdown doesn't wait its limit. Before, the first
/// panic unwound the map's whole loop: none of the four were saved, and all
/// stayed online.
#[tokio::test]
async fn one_panic_in_a_maps_shutdown_costs_only_that_players_save() {
    let realm = realm();
    let mut clients = Vec::new();
    let mut guids = Vec::new();
    for (i, name) in ["Alice", "Bobby", "Carol", "Daisy"].iter().enumerate() {
        let mut c = Client::connect(&realm, &format!("ACC{i}"));
        let g = c.enter(name, Race::Human, Class::Warrior).await;
        guids.push(g);
        clients.push(c);
    }
    join_peon(&realm, &clients[1]).await;
    let map = map_of(&realm, guids[0]);
    let gs = guids.clone();
    let order: Vec<Guid> = on(&map, move |m| {
        for g in &gs {
            m.players
                .get_mut(g)
                .unwrap()
                .fields
                .set_u32(PLAYER_FIELD_COINAGE, 777);
        }
        m.players.keys().copied().collect()
    })
    .await;
    // The first in the map's order panics as it is taken out, the second
    // in its save, once out.
    let (first, second) = (order[0], order[1]);
    on(&map, move |m| {
        m.faults.arm(format!("take {first}"), 1);
        m.faults.arm(format!("save {second}"), 1);
    })
    .await;
    let started = std::time::Instant::now();
    realm.shutdown_within(Duration::from_secs(5)).await;
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "no one left online to wait for: {:?}",
        started.elapsed()
    );
    for g in &guids {
        let saved = money_saved(&realm, *g).await == 777;
        assert_eq!(saved, *g != first && *g != second, "{g} saved");
        assert!(realm.registry.online(*g).is_none(), "{g} offline");
        assert!(
            realm.social.channels.lock().of(*g).is_empty(),
            "{g} out of peon"
        );
    }
    drop(clients);
}

fn store_file(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("wvs-rev5-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("characters.sqlite")
}

/// Review 5's maps 3: after a restart, a dungeon copy someone was saved in
/// at the shutdown, its last kill (here its entry) more than the sweep's
/// limit before, is kept for as long from the restart as a copy left empty
/// is: the sweep that runs at start-up leaves it, and she logs back into
/// it. Before, the stored time was all the sweep had: it took the copy at
/// once, and she went home.
#[tokio::test]
async fn a_dungeon_copy_outlasts_a_restart_whatever_its_last_kill() {
    let db = store_file("sweep");
    let (store, startup) = crate::game::store::Store::open(&db).unwrap();
    let realm = realm_on(store, startup);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    go(&realm, &mut a, DUNGEON).await;
    let key = map_of(&realm, g).key;
    let id = key.instance;
    realm.shutdown().await;
    drop(a);
    drop(realm);
    // Inside 11 minutes without a kill: past the sweep's limit,
    // max(instance_unload, 5 min), 10 minutes here.
    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        let data: String = conn
            .query_row(
                "SELECT data FROM records WHERE kind = 'instance' AND id = ?1",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .unwrap();
        let mut save: instances::InstanceSave = serde_json::from_str(&data).unwrap();
        save.touched_at -= 660;
        conn.execute(
            "UPDATE records SET data = ?1 WHERE kind = 'instance' AND id = ?2",
            rusqlite::params![serde_json::to_string(&save).unwrap(), id],
        )
        .unwrap();
    }
    let (store, startup) = crate::game::store::Store::open(&db).unwrap();
    let realm = realm_on(store, startup);
    // The realm's sweep, as at start-up.
    instances::sweep(&realm);
    assert!(instances::save_of(&realm, id).is_some(), "the copy is kept");
    let mut a = Client::connect(&realm, "A");
    a.login(g).await;
    assert_eq!(map_of(&realm, g).key, key, "Alice is back in her copy");
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

type LogoutState = (bool, bool, bool);

/// Stunned, logging out, rooted by the logout.
fn logout_state(g: Guid) -> impl FnOnce(&mut Map) -> LogoutState + Send + 'static {
    move |m| {
        let p = &m.players[&g];
        (
            p.fields.has_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_STUNNED,
            ),
            p.logout_at.is_some(),
            p.misc.logout_rooted,
        )
    }
}

/// Alice, asked to log out (20 s), teleported to map 1 in its countdown.
async fn teleported_logging_out() -> (Arc<Realm>, Client, Guid) {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    cfg.logout_delay = Duration::from_secs(20);
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = Realm::new(cfg, store, startup, Default::default(), None);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.wait_for("SMSG_LOGOUT_RESPONSE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_RESPONSE(_)).then_some(())
    })
    .await;
    assert_eq!(
        on(&map_of(&realm, g), logout_state(g)).await,
        (true, true, true)
    );
    map_of(&realm, g).call(move |m| m.teleport(g, MapKey::world(1), Vector3d::default(), 0.0));
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    a.clear();
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    a.sees(g).await;
    (realm, a, g)
}

/// Review 5's maps 4: a far teleport in a logout's countdown (a boat's
/// crossing, an eviction, a battleground's end) keeps the logout going, as
/// mangos's session timer does: the client is rooted again on the new map,
/// and a cancel there undoes the stun and the root. Before, the teleport
/// dropped the countdown and left its stun and root: the player came in
/// stunned, and a cancel had nothing to cancel.
#[tokio::test]
async fn a_teleport_in_a_logouts_countdown_keeps_it_and_a_cancel_undoes_it() {
    let (realm, mut a, g) = teleported_logging_out().await;
    let map = map_of(&realm, g);
    assert_eq!(map.key, MapKey::world(1));
    assert_eq!(on(&map, logout_state(g)).await, (true, true, true));
    a.wait_for("the root again", |m| match m {
        ServerOpcodeMessage::SMSG_FORCE_MOVE_ROOT(r) if r.guid == g => Some(()),
        _ => None,
    })
    .await;
    a.send(CMSG_LOGOUT_CANCEL {}).await;
    a.wait_for("SMSG_LOGOUT_CANCEL_ACK", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_CANCEL_ACK).then_some(())
    })
    .await;
    a.wait_for("the unroot", |m| match m {
        ServerOpcodeMessage::SMSG_FORCE_MOVE_UNROOT(r) if r.guid == g => Some(()),
        _ => None,
    })
    .await;
    assert_eq!(on(&map, logout_state(g)).await, (false, false, false));
}

/// Review 5's maps 4: the logout a teleport carried on completes on the
/// new map when it is due, saved there.
#[tokio::test]
async fn a_logout_carried_through_a_teleport_completes_on_the_new_map() {
    let (realm, mut a, g) = teleported_logging_out().await;
    let map = map_of(&realm, g);
    // Due now, rather than in 20 s.
    on(&map, move |m| {
        let now = m.now;
        m.players.get_mut(&g).unwrap().logout_at = Some(now);
    })
    .await;
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    until("offline", || realm.registry.online(g).is_none()).await;
    realm.store.flush().await;
    let row = realm
        .store
        .load(crate::game::guid::counter(g))
        .await
        .unwrap();
    assert_eq!(row.map, 1, "saved on the new map");
}
