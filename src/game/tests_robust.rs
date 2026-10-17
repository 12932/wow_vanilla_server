//! Reliability: what wvs does when a part of it panics, hangs or is
//! overloaded (docs/RELIABILITY.md). Each test makes the failure happen and
//! shows the server get over it.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::guid;
use crate::game::harness::{self, Client, WAIT};
use crate::game::map::{MapHandle, MapKey};
use crate::game::registry::Online;
use crate::game::session::Control;

/// Poll `what` until it holds, up to [`WAIT`].
async fn eventually(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + WAIT;
    while !f() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "never: {what} within {WAIT:?}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Poll the map until `what` holds of it, up to [`WAIT`].
async fn until(
    map: &MapHandle,
    what: impl Fn(&crate::game::map::Map) -> bool + Send + Clone + 'static,
) -> bool {
    let deadline = tokio::time::Instant::now() + WAIT;
    while tokio::time::Instant::now() < deadline {
        let w = what.clone();
        if map.query(move |m| w(m)).await == Some(true) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    false
}

async fn in_world(realm: &Arc<Realm>, account: &str, name: &str) -> (Client, Guid, MapHandle) {
    let mut c = Client::connect(realm, account);
    let g = c.enter(name, Race::Human, Class::Warrior).await;
    let map = realm.registry.online(g).expect("online").map();
    (c, g, map)
}

/// A client logs in to a character it already has, and gets the world or
/// the reason it can't.
async fn try_login(c: &mut Client, g: Guid) -> Result<(), WorldResult> {
    c.send(CMSG_PLAYER_LOGIN { guid: g }).await;
    c.wait_for("the login's answer", |m| match m {
        ServerOpcodeMessage::SMSG_LOGIN_VERIFY_WORLD(_) => Some(Ok(())),
        ServerOpcodeMessage::SMSG_CHARACTER_LOGIN_FAILED(f) => Some(Err(f.result)),
        _ => None,
    })
    .await
}

/// A session's task that panics in the world still takes its player out
/// of the map and saves them; the account can log straight back in.
#[tokio::test]
async fn a_session_that_panics_saves_its_player_and_frees_the_account() {
    let realm = harness::realm();
    let (c, g, map) = in_world(&realm, "ALICE", "Alice").await;
    let start = map.query(move |m| m.players[&g].position()).await.unwrap();
    c.heartbeat(start.x + 5.0, start.y, start.z).await;
    let moved = move |m: &crate::game::map::Map| {
        m.players
            .get(&g)
            .is_some_and(|p| p.position().x > start.x + 4.0)
    };
    assert!(until(&map, moved).await);

    let online = realm.registry.online(g).unwrap();
    online.session.send(Control::Panic).unwrap();
    eventually("Alice goes offline", || realm.registry.online(g).is_none()).await;
    assert!(
        !map.query(move |m| m.players.contains_key(&g))
            .await
            .unwrap(),
        "no ghost of her is left in the map"
    );
    let row = realm.store.load(guid::counter(g)).await.unwrap();
    assert!(row.x > start.x + 4.0, "saved where she was: {}", row.x);

    let mut again = Client::connect(&realm, "ALICE");
    assert_eq!(try_login(&mut again, g).await, Ok(()));
    drop(c);
}

/// An online entry whose session has gone (a ghost) no longer holds the
/// account out for good: the login that finds it lets it go, and the next
/// one gets in.
#[tokio::test]
async fn a_login_that_finds_a_ghost_lets_it_go() {
    let realm = harness::realm();
    let (c, g, map) = in_world(&realm, "BOB", "Bob").await;
    c.disconnect();
    eventually("Bob goes offline", || realm.registry.online(g).is_none()).await;
    // Bob online with a session that has gone, as a lost task would leave
    // him.
    let (dead, rx) = mpsc::unbounded_channel();
    drop(rx);
    let name = realm.registry.name(g).unwrap();
    let ghost = Arc::new(Online::new(
        &name,
        0,
        crate::game::conn::Outbound::detached().0,
        dead,
        map.clone(),
    ));
    realm.registry.go_online(ghost).unwrap();

    let mut b = Client::connect(&realm, "BOB");
    assert_eq!(
        try_login(&mut b, g).await,
        Err(WorldResult::CharLoginDuplicateCharacter)
    );
    eventually("the ghost goes", || realm.registry.online(g).is_none()).await;
    assert_eq!(try_login(&mut b, g).await, Ok(()));
}

/// A player a map hands over after their session has gone is still saved,
/// at the map they were bound for.
#[tokio::test]
async fn a_transfer_to_a_session_that_has_gone_is_saved_there() {
    let realm = harness::realm();
    let (_c, g, map) = in_world(&realm, "CAROL", "Carol").await;
    let to = MapKey::world(1);
    let r = realm.clone();
    map.query(move |m| {
        let p = m.take_player(g).unwrap();
        let (dead, rx) = mpsc::unbounded_channel();
        drop(rx);
        crate::game::session::transfer(&r, &dead, p, to);
    })
    .await
    .unwrap();
    eventually("Carol goes offline", || realm.registry.online(g).is_none()).await;
    realm.store.flush().await;
    let row = realm.store.load(guid::counter(g)).await.unwrap();
    assert_eq!(row.map, 1, "saved where she was going");
}

/// A panic in the store's thread costs that request, not the thread: the
/// saves and loads after it work. Before, the thread died, and with it
/// every save after (silently), and every load.
#[tokio::test]
async fn the_store_outlives_a_panic() {
    let realm = harness::realm();
    let (c, g, _map) = in_world(&realm, "ALICE", "Alice").await;
    realm.store.panic_for_test();
    c.disconnect();
    eventually("Alice goes offline", || realm.registry.online(g).is_none()).await;
    realm.store.flush().await;
    let row = realm.store.load(guid::counter(g)).await;
    assert_eq!(row.map(|r| r.name), Some("Alice".to_owned()));
    let mut again = Client::connect(&realm, "ALICE");
    assert_eq!(try_login(&mut again, g).await, Ok(()));
}

/// A save that fails takes only itself: the others batched with it in one
/// transaction are written each alone. Before, the batch's transaction
/// failed and every save in it was lost.
#[tokio::test]
async fn a_bad_save_loses_only_itself() {
    use crate::game::store::CharacterRow;
    let realm = harness::realm();
    let row = |guid: u32, name: &str| CharacterRow {
        guid,
        account: "ACC".into(),
        name: name.into(),
        race: 1,
        class: 1,
        level: 1,
        ..Default::default()
    };
    realm
        .store
        .create(row(900, "Anna"))
        .await
        .expect("Anna made");
    realm
        .store
        .create(row(901, "Beth"))
        .await
        .expect("Beth made");
    // Held, so what comes next goes in one batch.
    realm.store.hold_for_test(Duration::from_millis(300));
    tokio::time::sleep(Duration::from_millis(50)).await;
    realm.store.save(CharacterRow {
        level: 7,
        ..row(900, "Anna")
    });
    realm.store.bad_save_for_test();
    realm.store.save(CharacterRow {
        level: 9,
        ..row(901, "Beth")
    });
    realm.store.flush().await;
    let levels = (
        realm.store.load(900).await.map(|r| r.level),
        realm.store.load(901).await.map(|r| r.level),
    );
    assert_eq!(levels, (Some(7), Some(9)));
}

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

/// A sweep that panics rests (a second, then twice as long each time it
/// panics again) while the other sweeps go on; then it is tried again.
/// Before, the panic ended the sweeps' task: mail, auctions, instance
/// resets, battlegrounds, events and weather stopped for good.
#[tokio::test]
async fn a_sweep_that_panics_rests_and_the_others_go_on() {
    let realm = harness::realm();
    let faults = &realm.social.faults;
    faults.arm("the mail sweep", 1);
    let mut watch = crate::game::social::Watch::default();
    let t0 = std::time::Instant::now();
    crate::game::social::sweep_watched(&realm, &mut watch, t0);
    assert_eq!(faults.checked("the mail sweep"), 1);
    assert_eq!(faults.checked("the weather sweep"), 1, "the rest went on");
    // Resting at once after.
    crate::game::social::sweep_watched(&realm, &mut watch, t0);
    assert_eq!(faults.checked("the mail sweep"), 1);
    assert_eq!(faults.checked("the weather sweep"), 2);
    // Tried again at the next round, five seconds on.
    crate::game::social::sweep_watched(&realm, &mut watch, t0 + crate::game::social::SWEEP_EVERY);
    assert_eq!(faults.checked("the mail sweep"), 2);
}

/// The transports' clock outlives a panic: the route rests a moment, and
/// the boat sails on between its maps. Before, the clock's task died and
/// every boat and zeppelin stopped where it was.
#[tokio::test]
async fn the_transports_clock_outlives_a_panic() {
    use crate::game::data::npc::TaxiPathNode;
    use crate::game::data::*;
    const BOAT: u32 = 9999;
    let mut d = GameData::empty();
    let mut data = [0u32; 24];
    data[0] = 77;
    data[1] = 1000;
    data[2] = 1000;
    d.gameobject_templates.insert(
        BOAT,
        GameObjectTemplate {
            entry: BOAT,
            kind: 15,
            display_id: 3015,
            name: "The Test Boat".into(),
            size: 1.0,
            data,
            ..Default::default()
        },
    );
    d.transports.push(TransportRoute {
        entry: BOAT,
        name: "Here and There".into(),
        period: 1600,
    });
    let node = |map, x: f32, y: f32| TaxiPathNode {
        map,
        x,
        y,
        z: START.2,
        flags: 0,
        delay: 0,
    };
    d.npc.taxi_path_nodes.insert(
        77,
        vec![
            node(0, START.0, START.1),
            node(0, START.0 + 30.0, START.1),
            node(0, START.0 + 60.0, START.1),
            node(0, START.0 + 100.0, START.1),
            node(1, 990.0, 1000.0),
            node(1, 1000.0, 1000.0),
            node(1, 1010.0, 1000.0),
            node(1, 1020.0, 1000.0),
        ],
    );
    // Armed before the clock starts: its first tick panics.
    let realm = {
        let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
        let social_faults_first = |r: &Arc<Realm>| r.social.faults.arm("transport", 1);
        let r = Realm::new(harness::test_config(), store, startup, d, None);
        social_faults_first(&r);
        r
    };
    let boat = guid::make(guid::HighGuid::MoTransport, 0, BOAT);
    let on = |realm: &Arc<Realm>, map: u32| {
        let h = realm.maps.get(realm, MapKey::world(map));
        async move { h.query(move |m| m.gameobject(boat).is_some()).await == Some(true) }
    };
    eventually("the clock panics", || {
        realm.social.faults.checked("transport") >= 1
    })
    .await;
    let first = if on(&realm, 0).await { 0 } else { 1 };
    let deadline = tokio::time::Instant::now() + WAIT;
    while !on(&realm, 1 - first).await {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the boat never crossed"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(realm.social.faults.checked("transport") > 1);
}

const WOLF: u32 = 3000;

/// Two friendly wolves by the start, spawns 20 and 21.
fn wolves() -> crate::game::data::GameData {
    use crate::game::data::*;
    let mut d = GameData::empty();
    d.creature_templates.insert(
        WOLF,
        CreatureTemplate {
            entry: WOLF,
            name: "Wolf".into(),
            display_ids: [903, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            level_min: 2,
            level_max: 2,
            faction: 35,
            unit_class: 1,
            health_multiplier: 1.0,
            health_max: 500,
            ..Default::default()
        },
    );
    let spawn = |guid, dx: f32| CreatureSpawn {
        guid,
        entries: vec![WOLF],
        map: 0,
        x: START.0 + dx,
        y: START.1,
        z: START.2,
        respawn_min: 600,
        respawn_max: 600,
        ..Default::default()
    };
    d.creature_spawns
        .insert(0, vec![spawn(20, 3.0), spawn(21, 6.0)]);
    d
}

fn wolf(spawn: u32) -> Guid {
    guid::unit(WOLF, spawn)
}

async fn panics(map: &MapHandle) -> u64 {
    map.query(|m| m.stats.panics).await.unwrap()
}

/// A creature whose turn panics costs its turn, not the map's update: the
/// first time it is let be, then restarted from its spawn, and at the
/// fourth quarantined (despawned for good), so it stops panicking. Its
/// neighbour, and the players, go on.
#[tokio::test]
async fn a_creature_that_keeps_panicking_is_restarted_then_quarantined() {
    let realm = harness::realm_with(wolves());
    let (mut a, _g, map) = in_world(&realm, "ALICE", "Alice").await;
    a.sees(wolf(20)).await;
    a.sees(wolf(21)).await;
    let bad = wolf(20);
    map.query(move |m| m.faults.arm(format!("creature {bad}"), u32::MAX))
        .await
        .unwrap();
    // Restarted: gone from view, and back from its spawn.
    a.forgets(bad).await;
    a.sees(bad).await;
    assert!(
        until(&map, |m| m.stats.quarantined == 1).await,
        "quarantined"
    );
    let (alive, panicked) = map
        .query(move |m| (m.creature(bad).is_some_and(|c| c.alive()), m.stats.panics))
        .await
        .unwrap();
    assert!(!alive, "it lies dead for good");
    assert_eq!(panicked, 4);
    // It panics no more, and the map still answers a newcomer.
    let (mut b, _, _) = in_world(&realm, "BOB", "Bob").await;
    b.sees(wolf(21)).await;
    assert_eq!(panics(&map).await, 4);
    assert!(
        !b.sees_within(bad, Duration::from_millis(300)).await,
        "the quarantined wolf is not shown"
    );
}

/// A step of the update that panics every tick costs that step alone: the
/// rest of the update (visibility, and the flushes) still runs, so a
/// player who comes in sees the world.
#[tokio::test]
async fn an_update_step_that_panics_costs_only_itself() {
    let realm = harness::realm_with(wolves());
    let (_a, _, map) = in_world(&realm, "ALICE", "Alice").await;
    map.query(|m| {
        // Not so many that the map is restarted.
        m.health.limits.map_restart = u32::MAX;
        m.faults.arm("spells", u32::MAX);
    })
    .await
    .unwrap();
    let before = panics(&map).await;
    let (mut b, _, _) = in_world(&realm, "BOB", "Bob").await;
    b.sees(wolf(20)).await;
    assert!(panics(&map).await > before, "the step went on panicking");
}

/// A map whose update keeps panicking is in a bad state: it is restarted.
/// Its players are saved and sent to the character screen, the map is
/// built afresh from the data (without the bad state), and they come back
/// into it.
#[tokio::test]
async fn a_map_whose_update_keeps_panicking_is_restarted() {
    let realm = harness::realm_with(wolves());
    let (mut a, g, map) = in_world(&realm, "ALICE", "Alice").await;
    a.sees(wolf(20)).await;
    a.heartbeat(START.0 + 2.0, START.1 + 2.0, START.2).await;
    assert!(
        until(&map, move |m| m
            .players
            .get(&g)
            .is_some_and(|p| p.position().y > START.1 + 1.0))
        .await
    );
    map.query(|m| {
        m.health.limits.map_restart = 20;
        m.faults.arm("spells", u32::MAX);
    })
    .await
    .unwrap();
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| match m {
        ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE => Some(()),
        _ => None,
    })
    .await;
    assert!(realm.registry.online(g).is_none());
    let row = realm.store.load(guid::counter(g)).await.unwrap();
    assert!(row.y > START.1 + 1.0, "saved where she was: {}", row.y);
    // The same handle, a fresh map.
    let (restarts, wolves, faults) = map
        .query(|m| (m.stats.restarts, m.creatures.len(), m.faults.any()))
        .await
        .unwrap();
    assert_eq!((restarts, wolves, faults), (1, 2, false));
    a.clear();
    assert_eq!(try_login(&mut a, g).await, Ok(()));
    a.sees(wolf(20)).await;
    let p = realm.registry.online(g).unwrap().map();
    assert_eq!(p.key, map.key);
}

/// A player whose packets keep panicking is saved and goes back to the
/// character screen (as Thistle Tea's player process that crashes takes
/// its client out); the map, and the others in it, go on.
#[tokio::test]
async fn a_player_whose_packets_keep_panicking_goes_back_to_the_screen() {
    let realm = harness::realm_with(wolves());
    let (mut a, ga, map) = in_world(&realm, "ALICE", "Alice").await;
    let (mut b, _, _) = in_world(&realm, "BOB", "Bob").await;
    map.query(move |m| m.faults.arm(format!("packets of {ga}"), u32::MAX))
        .await
        .unwrap();
    for _ in 0..10 {
        a.say("hello").await;
    }
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| match m {
        ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE => Some(()),
        _ => None,
    })
    .await;
    eventually("Alice goes offline", || realm.registry.online(ga).is_none()).await;
    assert_eq!(panics(&map).await, 10);
    b.say("still here").await;
    b.wait_for("Bob's own words", |m| match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(c) if c.message == "still here" => Some(()),
        _ => None,
    })
    .await;
    assert_eq!(map.query(|m| m.stats.restarts).await, Some(0));
}

/// A map whose actor has gone (its channel closed) is started again by the
/// next `maps.get`, rather than handing out a dead handle: a player logs
/// in to it.
#[tokio::test]
async fn a_map_whose_actor_has_gone_is_started_again() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "ALICE");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| match m {
        ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE => Some(()),
        _ => None,
    })
    .await;
    let key = MapKey::world(0);
    let old = realm.maps.get(&realm, key);
    // The actor ends; its handle stays where `maps.get` finds it.
    old.send(crate::game::map::MapEvent::Stop);
    eventually("the actor ends", || old.is_closed()).await;
    let new = realm.maps.get(&realm, key);
    assert!(!new.is_closed());
    assert_eq!(new.query(|m| m.key).await, Some(key));
    assert_eq!(try_login(&mut a, g).await, Ok(()));
}
