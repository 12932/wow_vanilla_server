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
    b.say(".maps").await;
    b.system_message("panics 4, quarantined 1, restarts 0")
        .await;
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

/// The watchdog says when a map has been in one event or update too long
/// (an endless loop), and when it is going again; and when a map's
/// mailbox grows past what it can keep up with.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_watchdog_sees_a_map_hang_and_a_mailbox_swell() {
    use crate::game::supervisor::{Alarm, Limits, Watched};
    let mut watched = Watched::default();
    let mut check =
        |realm: &Realm, limits| crate::game::supervisor::check(realm, limits, &mut watched);
    let realm = harness::realm();
    let key = MapKey::world(0);
    let map = realm.maps.get(&realm, key);
    map.query(|_| ()).await.unwrap();
    let limits = Limits {
        hung_after: Duration::from_millis(500),
        mailbox: 500,
    };
    assert_eq!(check(&realm, limits), []);
    // Stuck in one event for a second.
    map.call(|_| std::thread::sleep(Duration::from_millis(1200)));
    tokio::time::sleep(Duration::from_millis(800)).await;
    let alarms = check(&realm, limits);
    assert!(
        matches!(alarms[..], [Alarm::Hung(k, _)] if k == key),
        "{alarms:?}"
    );
    assert_eq!(check(&realm, limits), [], "said once");
    map.query(|_| ()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(check(&realm, limits), [Alarm::Back(key)]);
    // Sent more than it can get through: each event takes a millisecond.
    for _ in 0..2_000 {
        map.call(|_| std::thread::sleep(Duration::from_millis(1)));
    }
    tokio::time::sleep(Duration::from_millis(700)).await;
    let alarms = check(&realm, limits);
    assert!(
        alarms
            .iter()
            .any(|a| matches!(a, Alarm::Swamped(k, n) if *k == key && *n >= 500)),
        "{alarms:?}"
    );
    map.query(|_| ()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert!(check(&realm, limits).contains(&Alarm::Drained(key)));
}

/// The panic hook logs a panic through tracing with where it happened, and
/// a backtrace the first time at that place only.
#[test]
fn the_panic_hook_logs_where_once_with_a_backtrace() {
    use std::io::Write;
    #[derive(Clone, Default)]
    struct Buffer(Arc<parking_lot::Mutex<Vec<u8>>>);
    impl Write for Buffer {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let out = Buffer::default();
    let writer = out.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .finish();
    crate::game::supervisor::install_panic_hook();
    let line = std::sync::atomic::AtomicU32::new(0);
    tracing::subscriber::with_default(subscriber, || {
        for _ in 0..2 {
            let _ = std::panic::catch_unwind(|| {
                line.store(line!() + 1, std::sync::atomic::Ordering::Relaxed);
                panic!("a test's panic");
            });
        }
    });
    let _ = std::panic::take_hook();
    let at = format!(
        "{}:{}:17",
        file!(),
        line.load(std::sync::atomic::Ordering::Relaxed)
    );
    assert_eq!(crate::game::supervisor::seen(&at), 2, "{at}");
    let log = String::from_utf8(out.0.lock().clone()).unwrap();
    let said = format!("at {at}: a test's panic");
    assert_eq!(log.matches(&said).count(), 2, "{log}");
    assert_eq!(log.matches("a test's panic").count(), 2, "{log}");
    assert_eq!(
        log.matches("its backtrace is at the first").count(),
        1,
        "{log}"
    );
    assert!(log.contains("ERROR"), "{log}");
}

/// A panic while the login server's session keys were held no longer
/// locks everyone out: before, the poisoned lock refused every login and
/// world handshake until a restart.
#[test]
fn a_panic_holding_the_session_keys_locks_no_one_out() {
    let users: crate::auth::UserCache = Default::default();
    let held = users.clone();
    let poisoned = std::thread::spawn(move || {
        let _keys = held.lock();
        panic!("a panic while the session keys are held");
    })
    .join();
    assert!(poisoned.is_err() && users.is_poisoned());
    let mut keys = crate::auth::users_lock(&users);
    assert!(keys.get("NOBODY").is_none());
}

/// A supervised task that panics is started again (at once, then after a
/// rest), until it returns.
#[tokio::test]
async fn a_supervised_task_that_panics_is_started_again() {
    let starts = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let counted = starts.clone();
    let supervised = crate::game::supervisor::supervise("a test's task", move || {
        let n = counted.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        async move {
            if n < 3 {
                panic!("start {n} panics");
            }
        }
    });
    tokio::time::timeout(Duration::from_secs(5), supervised)
        .await
        .expect("it ends once the task returns");
    assert_eq!(starts.load(std::sync::atomic::Ordering::Relaxed), 3);
}

/// A shutdown waits only so long for a map that has hung: the other maps'
/// players are still saved and written. Before, it waited on the hung map
/// for good, and nothing it waited to write reached the disk.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_shutdown_does_not_wait_for_a_hung_map() {
    let realm = harness::realm();
    let mut c = Client::connect(&realm, "THRALL");
    let g = c.enter("Thrall", Race::Orc, Class::Warrior).await;
    let kalimdor = realm.registry.online(g).unwrap().map();
    assert_eq!(kalimdor.key, MapKey::world(1));
    let hung = realm.maps.get(&realm, MapKey::world(0));
    hung.query(|_| ()).await.unwrap();
    hung.call(|_| std::thread::sleep(Duration::from_secs(4)));
    let started = std::time::Instant::now();
    realm.shutdown_within(Duration::from_millis(500)).await;
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    let row = realm.store.load(guid::counter(g)).await.unwrap();
    assert!(row.logout_time > 0, "Thrall was saved");
}

/// A client packet's body, as wow_messages reads it: little-endian
/// values and C strings.
#[derive(Default)]
struct Body(Vec<u8>);

impl Body {
    fn u8(mut self, v: u8) -> Self {
        self.0.push(v);
        self
    }
    fn u32(mut self, v: u32) -> Self {
        self.0.extend(v.to_le_bytes());
        self
    }
    fn bytes(mut self, b: &[u8]) -> Self {
        self.0.extend(b);
        self
    }
    fn string(self, s: &str) -> Self {
        self.bytes(s.as_bytes()).u8(0)
    }
}

/// A `/who` whose count of search strings is `strings`.
fn who(strings: u32) -> Vec<u8> {
    Body::default()
        .u32(1)
        .u32(60)
        .string("")
        .string("")
        .u32(0)
        .u32(0)
        .u32(1)
        .u32(12)
        .u32(strings)
        .string("hello")
        .0
}

/// Packets wow_messages would once have reserved gigabytes for, from a
/// count in the packet (a `/who` claiming 4 billion strings asked for
/// 96 GB, and the failed allocation aborted the process), are refused as
/// parse errors now, or read for what they hold: it reserves no more than
/// the body could hold. Those it reads in reason pass.
#[test]
fn packets_that_would_have_wow_messages_reserve_gigabytes_are_refused() {
    use wow_world_messages::errors::ExpectedOpcodeError;
    use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage as C;
    // As the connection's reader hands a packet to wow_messages.
    let read = |opcode: u32, body: &[u8]| {
        let mut plain = ((body.len() + 4) as u16).to_be_bytes().to_vec();
        plain.extend(opcode.to_le_bytes());
        plain.extend(body);
        C::read_unencrypted(&plain[..])
    };
    let refused =
        |opcode: u32, body: &[u8]| matches!(read(opcode, body), Err(ExpectedOpcodeError::Parse(_)));
    assert!(matches!(read(0x062, &who(1)), Ok(C::CMSG_WHO(_))));
    assert!(refused(0x062, &who(u32::MAX)), "a /who's strings");
    let express = |n: u32| Body::default().bytes(&[0; 12]).u32(n).u32(1).u32(2).0;
    assert!(matches!(
        read(0x312, &express(2)),
        Ok(C::CMSG_ACTIVATETAXIEXPRESS(m)) if m.nodes == [1, 2]
    ));
    assert!(
        refused(0x312, &express(u32::MAX)),
        "an express flight's nodes"
    );
    assert!(refused(0x264, &express(1 << 30)), "a bidder's items");
    let bug = |n: u32| {
        Body::default()
            .u32(0)
            .u32(n)
            .string("x")
            .u32(2)
            .string("y")
            .0
    };
    assert!(matches!(read(0x1CA, &bug(2)), Ok(C::CMSG_BUG(m)) if m.content == "x"));
    assert!(refused(0x1CA, &bug(0)), "a bug report's length, less one");
    assert!(refused(0x1CA, &bug(u32::MAX)));
    // An empty zlib stream.
    const EMPTY: [u8; 8] = [0x78, 0x9C, 3, 0, 0, 0, 0, 1];
    let auth = |size: u32, addons: &[u8]| {
        Body::default()
            .u32(5875)
            .u32(0)
            .string("ALICE")
            .u32(7)
            .bytes(&[0; 20])
            .u32(size)
            .bytes(addons)
            .0
    };
    for size in [300, u32::MAX] {
        assert!(
            matches!(
                read(0x1ED, &auth(size, &EMPTY)),
                Ok(C::CMSG_AUTH_SESSION(m)) if m.addon_info.is_empty()
            ),
            "an auth session's addon data, {size} bytes claimed, is what it holds"
        );
    }
    assert!(refused(0x1ED, &auth(u32::MAX, b"not zlib at all")));
    let ticket = |category: u8, size: u32, chat: &[u8]| {
        Body::default()
            .u8(category)
            .u32(0)
            .bytes(&[0; 12])
            .string("help")
            .string("")
            .u32(3)
            .u32(size)
            .bytes(chat)
            .0
    };
    assert!(matches!(
        read(0x205, &ticket(2, 100, &EMPTY)),
        Ok(C::CMSG_GMTICKET_CREATE(_))
    ));
    assert!(
        matches!(
            read(0x205, &ticket(1, u32::MAX, &[])),
            Ok(C::CMSG_GMTICKET_CREATE(_))
        ),
        "only harassment has chat"
    );
    assert!(
        matches!(
            read(0x205, &ticket(2, u32::MAX, &EMPTY)),
            Ok(C::CMSG_GMTICKET_CREATE(_))
        ),
        "a ticket's chat is what it holds"
    );
    assert!(refused(0x205, &ticket(2, u32::MAX, b"not zlib at all")));
    assert!(
        matches!(
            read(0x2E7, &[1, 2]),
            Ok(C::CMSG_WARDEN_DATA(m)) if m.encrypted_data == [1, 2]
        ),
        "warden data shorter than a size is read whole"
    );
    assert!(matches!(read(0x4B, &[]), Ok(C::CMSG_LOGOUT_REQUEST)));
}

/// A connection is handed paired crypto, as an authenticated client's is.
fn paired_crypto() -> (
    wow_srp::vanilla_header::HeaderCrypto,
    wow_srp::vanilla_header::HeaderCrypto,
) {
    use wow_srp::normalized_string::NormalizedString;
    use wow_srp::vanilla_header::ProofSeed;
    let username = NormalizedString::new("test".to_string()).unwrap();
    let (server_seed, client_seed) = (ProofSeed::new(), ProofSeed::new());
    let (server_value, client_value) = (server_seed.seed(), client_seed.seed());
    let (proof, client) = client_seed.into_client_header_crypto(&username, [0; 40], server_value);
    let server = server_seed
        .into_server_header_crypto(&username, [0; 40], proof, client_value)
        .unwrap();
    (server, client)
}

/// A connection's reader drops a packet that would have aborted the
/// process (a `/who` that asks for 96 GB), and one wow_messages panicked
/// on (a ticket's chat that isn't zlib), each a parse error now, and goes
/// on reading. Before, the first aborted the server (`memory allocation of
/// 103079215080 bytes failed`), and the second ended the connection.
#[tokio::test]
async fn a_packet_that_would_abort_the_server_is_dropped() {
    use tokio::io::AsyncWriteExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut client = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (stream, _) = listener.accept().await.unwrap();
    let (server, crypto) = paired_crypto();
    let mut connection = crate::game::conn::start(stream, server);
    let (mut encrypter, _) = crypto.split();
    let mut send = async |opcode: u32, body: &[u8]| {
        let header = encrypter.encrypt_client_header(4 + body.len() as u16, opcode);
        client.write_all(&header).await.unwrap();
        client.write_all(body).await.unwrap();
    };
    send(0x062, &who(u32::MAX)).await;
    let not_zlib = Body::default()
        .u8(2)
        .u32(0)
        .bytes(&[0; 12])
        .string("help")
        .string("")
        .u32(3)
        .u32(100)
        .bytes(b"not zlib at all")
        .0;
    send(0x205, &not_zlib).await;
    send(0x4B, &[]).await;
    let next = tokio::time::timeout(WAIT, connection.inbound.recv()).await;
    assert!(
        matches!(
            next,
            Ok(Some(crate::game::conn::Inbound::Msg(
                wow_world_messages::vanilla::opcodes::ClientOpcodeMessage::CMSG_LOGOUT_REQUEST
            )))
        ),
        "{next:?}"
    );
}

/// The world handshake over a socket: the login server's session key for
/// the account, a client's `CMSG_AUTH_SESSION`, and the encrypted answer.
/// One whose addon data claims gigabytes is read for what it holds, or
/// refused if it isn't zlib, and the server goes on.
#[tokio::test]
async fn the_world_handshake_reads_an_auth_session_and_refuses_an_absurd_one() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use wow_srp::client::SrpClientUser;
    use wow_srp::normalized_string::NormalizedString;
    use wow_srp::server::SrpVerifier;
    use wow_srp::vanilla_header::ProofSeed;
    use wow_srp::{GENERATOR, LARGE_SAFE_PRIME_LITTLE_ENDIAN, PublicKey};

    let name = || NormalizedString::new("ALICE".to_string()).unwrap();
    // The login, as the login server and the client run it.
    let proof = SrpVerifier::from_username_and_password(name(), name()).into_proof();
    let challenge = SrpClientUser::new(name(), name()).into_challenge(
        GENERATOR,
        LARGE_SAFE_PRIME_LITTLE_ENDIAN,
        PublicKey::from_le_bytes(*proof.server_public_key()).unwrap(),
        *proof.salt(),
    );
    let (server, server_proof) = proof
        .into_server(
            PublicKey::from_le_bytes(*challenge.client_public_key()).unwrap(),
            *challenge.client_proof(),
        )
        .unwrap();
    let session_key = challenge
        .verify_server_proof(server_proof)
        .unwrap()
        .session_key();
    let users: crate::auth::UserCache = Default::default();
    crate::auth::users_lock(&users).insert("ALICE".to_string(), server);
    let realm = {
        let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
        Realm::new(
            harness::test_config(),
            store,
            startup,
            crate::game::data::GameData::empty(),
            Some(users),
        )
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let at = listener.local_addr().unwrap();
    let serving = {
        let realm = realm.clone();
        tokio::spawn(async move {
            let pending = Arc::new(tokio::sync::Semaphore::new(8));
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let permit = pending.clone().try_acquire_owned().unwrap();
                tokio::spawn(crate::game::session::serve(stream, realm.clone(), permit));
            }
        })
    };

    // An auth session by hand, its proof right for the server's seed, its
    // addon data claiming 4 GB uncompressed.
    let by_hand = async |stream: &mut tokio::net::TcpStream, addons: &[u8]| {
        let seed = tokio_expect_server_message::<SMSG_AUTH_CHALLENGE, _>(&mut *stream)
            .await
            .unwrap()
            .server_seed;
        let client_seed = ProofSeed::new();
        let client_seed_value = client_seed.seed();
        let (client_proof, crypto) =
            client_seed.into_client_header_crypto(&name(), session_key, seed);
        let mut body = Body::default()
            .u32(5875)
            .u32(0)
            .string("ALICE")
            .u32(client_seed_value)
            .bytes(&client_proof)
            .u32(u32::MAX)
            .bytes(addons)
            .0;
        let mut packet = ((body.len() + 4) as u16).to_be_bytes().to_vec();
        packet.extend(0x1EDu32.to_le_bytes());
        packet.append(&mut body);
        stream.write_all(&packet).await.unwrap();
        crypto
    };

    // Its addon data not zlib: refused, and the connection closes.
    let mut bad = tokio::net::TcpStream::connect(at).await.unwrap();
    by_hand(&mut bad, b"not zlib at all").await;
    let mut rest = Vec::new();
    let closed = tokio::time::timeout(WAIT, bad.read_to_end(&mut rest)).await;
    assert!(matches!(closed, Ok(Ok(0))), "{closed:?}");

    // An empty zlib stream: read for what it holds, nothing like 4 GB
    // reserved, and the world answers.
    let mut empty = tokio::net::TcpStream::connect(at).await.unwrap();
    let mut crypto = by_hand(&mut empty, &[0x78, 0x9C, 3, 0, 0, 0, 0, 1]).await;
    let answer = tokio_expect_server_message_encryption::<SMSG_AUTH_RESPONSE, _>(
        &mut empty,
        crypto.decrypter(),
    )
    .await
    .unwrap();
    assert!(
        matches!(answer.result, SMSG_AUTH_RESPONSE_WorldResult::AuthOk { .. }),
        "{answer:?}"
    );
    drop(empty);

    // A real one: the world answers, encrypted.
    let mut good = tokio::net::TcpStream::connect(at).await.unwrap();
    let seed = tokio_expect_server_message::<SMSG_AUTH_CHALLENGE, _>(&mut good)
        .await
        .unwrap()
        .server_seed;
    let client_seed = ProofSeed::new();
    let client_seed_value = client_seed.seed();
    let (client_proof, mut crypto) =
        client_seed.into_client_header_crypto(&name(), session_key, seed);
    CMSG_AUTH_SESSION {
        build: 5875,
        server_id: 0,
        username: "ALICE".into(),
        client_seed: client_seed_value,
        client_proof,
        addon_info: Vec::new(),
    }
    .tokio_write_unencrypted_client(&mut good)
    .await
    .unwrap();
    let answer = tokio_expect_server_message_encryption::<SMSG_AUTH_RESPONSE, _>(
        &mut good,
        crypto.decrypter(),
    )
    .await
    .unwrap();
    assert!(
        matches!(answer.result, SMSG_AUTH_RESPONSE_WorldResult::AuthOk { .. }),
        "{answer:?}"
    );
    serving.abort();
}

/// Every GM command, with arguments out of range, not numbers, NaN and
/// infinities, never panics its map.
#[tokio::test]
async fn gm_commands_with_odd_arguments_never_panic() {
    let realm = {
        let mut config = harness::test_config();
        config.max_packets_per_sec = 1_000_000;
        let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
        Realm::new(config, store, startup, wolves(), None)
    };
    let (mut a, _, _) = in_world(&realm, "ALICE", "Alice").await;
    let (_b, gb, _) = in_world(&realm, "BOB", "Bob").await;
    let args = [
        "",
        "nan",
        "inf",
        "-inf",
        "-1",
        "0",
        "1e30",
        "-1e30",
        "4294967295",
        "99999999999999999999",
        "x",
        "nan nan nan",
        "inf inf inf",
        "1e30 1e30 1e30 1",
        "-1 -1 -1",
        "0 0 0 0",
        "on",
        "off",
        "start",
        "stop",
        "start nan",
        "start 1e30 1e30",
        "list",
        "close -1",
        "respond -1 x",
        "points nan",
        "points -1",
        "week",
        "hp nan",
        "hp -1",
        "money -1",
        "money 1e30",
        "speed nan",
        "speed inf",
        "mana 99999999999999999999",
        "Bob",
        "Bob nan",
        "xyz nan nan nan",
        "xyz 1e30 1e30 1e30 0",
        "alterac",
        "warsong start",
        "reset",
        "all",
    ];
    for command in chaos::GM_COMMANDS {
        if ["kick", "die"].contains(command) {
            continue;
        }
        for arg in args {
            // Bob's the target, where the command takes one.
            a.send(CMSG_SET_SELECTION { target: gb }).await;
            a.say(&format!(".{command} {arg}")).await;
            // As a client does, sent elsewhere.
            a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
        }
        // A round trip: what was said has been handled.
        a.send(CMSG_NAME_QUERY { guid: a.guid }).await;
        let me = a.guid;
        a.wait_for("the name", |m| match m {
            ServerOpcodeMessage::SMSG_NAME_QUERY_RESPONSE(r) if r.guid == me => Some(()),
            _ => None,
        })
        .await;
        a.clear();
        if realm.registry.online(me).is_none() {
            assert_eq!(try_login(&mut a, me).await, Ok(()));
        }
        let mut total = 0;
        for m in realm.maps.all() {
            total += m.query(|m| m.stats.panics).await.unwrap_or(0);
        }
        assert_eq!(total, 0, "after .{command}");
    }
}

/// A walk to a place of no finite distance (out of the world, or NaN)
/// takes no time. Before, working out its duration panicked, every turn
/// of the creature that walked it.
#[test]
fn a_walk_to_nowhere_finite_takes_no_time() {
    use crate::game::map::creature::Walk;
    let here = Vector3d {
        x: START.0,
        y: START.1,
        z: START.2,
    };
    for there in [
        Vector3d {
            x: 1e30,
            y: 1e30,
            z: 0.0,
        },
        Vector3d {
            x: f32::NAN,
            y: 0.0,
            z: 0.0,
        },
    ] {
        let walk = Walk::new(vec![here, there], 7.0, std::time::Instant::now(), 1, true);
        assert_eq!(walk.duration, Duration::ZERO);
    }
    let walk = Walk::new(
        vec![
            here,
            Vector3d {
                x: START.0 + 7.0,
                ..here
            },
        ],
        7.0,
        std::time::Instant::now(),
        1,
        true,
    );
    assert_eq!(
        walk.duration,
        Duration::from_secs(1),
        "a real walk is as before"
    );
}

/// A GM's numbers that aren't finite, or a place out of the world, are
/// refused: a creature moved to `nan` panicked each turn, and a rested
/// bonus of `nan` lost the character's state at the next login.
#[tokio::test]
async fn a_gms_numbers_must_be_finite_and_in_the_world() {
    let realm = harness::realm_with(wolves());
    let (mut a, g, map) = in_world(&realm, "ALICE", "Alice").await;
    a.sees(wolf(20)).await;
    a.send(CMSG_SET_SELECTION { target: wolf(20) }).await;
    a.say(".move nan nan nan").await;
    a.system_message("Moved to").await;
    a.say(".go 1e30 1e30 1e30").await;
    a.system_message("Usage: .go").await;
    a.say(".rested nan").await;
    a.system_message("Rested XP").await;
    let bad = wolf(20);
    let (wolf_at, me_at, rested) = map
        .query(move |m| {
            (
                m.creature(bad).unwrap().position,
                m.players[&g].position(),
                m.players[&g].row.state.rest_bonus,
            )
        })
        .await
        .unwrap();
    assert!(
        wolf_at.x.is_finite() && wolf_at.y.is_finite(),
        "{wolf_at:?}"
    );
    assert!(me_at.x.abs() < 20_000.0, "{me_at:?}");
    assert!(rested.is_finite(), "{rested}");
}

/// A field of a character's state that won't parse goes back to its
/// default, alone. A NaN is saved as `null`, which an `f32` won't take;
/// before, that one field cost the whole state.
#[tokio::test]
async fn a_saved_field_that_wont_parse_costs_only_itself() {
    use crate::game::store::{CharacterRow, CharacterState};
    let realm = harness::realm();
    let row = CharacterRow {
        guid: 77,
        account: "ACC".into(),
        name: "Nan".into(),
        race: 1,
        class: 1,
        level: 1,
        state: CharacterState {
            rest_bonus: f32::NAN,
            explored_zones: vec![12, 40],
            talent_wipes: 3,
            ..Default::default()
        },
        ..Default::default()
    };
    realm.store.create(row).await.unwrap();
    let back = realm.store.load(77).await.unwrap().state;
    assert_eq!(back.explored_zones, [12, 40]);
    assert_eq!(back.talent_wipes, 3);
    assert_eq!(back.rest_bonus, 0.0);
}

/// Chaos: random, well-typed client messages in random orders, from
/// players among the real world's creatures, NPCs and objects.
mod chaos {
    use std::collections::HashMap;

    use rand::rngs::StdRng;
    use rand::{RngExt, SeedableRng};
    use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;

    use super::*;
    use crate::game::supervisor::catch;

    /// The kinds of value a body is built of.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    enum Kind {
        U8,
        U16,
        U32,
        Guid,
        F32,
        Str,
    }

    const KINDS: [Kind; 6] = [
        Kind::U8,
        Kind::U16,
        Kind::U32,
        Kind::Guid,
        Kind::F32,
        Kind::Str,
    ];

    /// What the players might name: the objects around them, their
    /// items, and the ids of the world's spells, items, quests and the
    /// rest.
    struct Vocabulary {
        guids: Vec<u64>,
        ids: Vec<u32>,
        floats: Vec<f32>,
        words: Vec<&'static str>,
    }

    impl Vocabulary {
        fn fill(&self, rng: &mut StdRng, kind: Kind, out: &mut Vec<u8>) {
            match kind {
                Kind::U8 => {
                    let v: u8 = match rng.random_range(0..4) {
                        0 => rng.random_range(0..4),
                        1 => rng.random_range(0..40),
                        2 => 0xFF,
                        _ => rng.random(),
                    };
                    out.push(v);
                }
                Kind::U16 => {
                    let v: u16 = match rng.random_range(0..3) {
                        0 => rng.random_range(0..64),
                        1 => self.ids[rng.random_range(0..self.ids.len())] as u16,
                        _ => rng.random(),
                    };
                    out.extend(v.to_le_bytes());
                }
                Kind::U32 => {
                    let v: u32 = match rng.random_range(0..6) {
                        0 => rng.random_range(0..8),
                        1 => rng.random_range(0..64),
                        2 | 3 => self.ids[rng.random_range(0..self.ids.len())],
                        4 => u32::MAX,
                        _ => rng.random(),
                    };
                    out.extend(v.to_le_bytes());
                }
                Kind::Guid => {
                    let v: u64 = match rng.random_range(0..8) {
                        0 => 0,
                        1 => rng.random(),
                        _ => self.guids[rng.random_range(0..self.guids.len())],
                    };
                    out.extend(v.to_le_bytes());
                }
                Kind::F32 => {
                    let v = self.floats[rng.random_range(0..self.floats.len())]
                        + if rng.random_range(0..2) == 0 {
                            rng.random_range(-20.0..20.0)
                        } else {
                            0.0
                        };
                    out.extend(v.to_le_bytes());
                }
                Kind::Str => {
                    let w = self.words[rng.random_range(0..self.words.len())];
                    out.extend(w.as_bytes());
                    out.push(0);
                }
            }
        }
    }

    fn parse(opcode: u32, body: &[u8]) -> Result<Option<ClientOpcodeMessage>, String> {
        let mut plain = Vec::with_capacity(6 + body.len());
        plain.extend(((body.len() + 4) as u16).to_be_bytes());
        plain.extend(opcode.to_le_bytes());
        plain.extend(body);
        catch(|| ClientOpcodeMessage::read_unencrypted(&plain[..]).ok())
    }

    /// The opcodes wow_messages reads from a client.
    fn client_opcodes() -> Vec<u32> {
        use wow_world_messages::errors::ExpectedOpcodeError;
        (0..0x400u32)
            .filter(|op| {
                let plain = [0, 4, *op as u8, (*op >> 8) as u8, 0, 0];
                let read = catch(|| {
                    !matches!(
                        ClientOpcodeMessage::read_unencrypted(&plain[..]),
                        Err(ExpectedOpcodeError::Opcode { .. })
                    )
                });
                read.unwrap_or(true)
            })
            .collect()
    }

    /// A random message of `opcode`: a shape it parsed with before, filled
    /// afresh, or (now and then, or while it has none) a new shape tried.
    struct Generator {
        vocabulary: Vocabulary,
        shapes: HashMap<u32, Vec<Vec<Kind>>>,
        /// Bodies that made wow_messages panic, by opcode.
        parse_panics: HashMap<u32, (u32, String)>,
    }

    impl Generator {
        fn body(&self, rng: &mut StdRng, shape: &[Kind]) -> Vec<u8> {
            let mut b = Vec::new();
            for k in shape {
                self.vocabulary.fill(rng, *k, &mut b);
            }
            b
        }

        fn message(
            &mut self,
            rng: &mut StdRng,
            opcode: u32,
        ) -> Option<(Vec<u8>, ClientOpcodeMessage)> {
            let known = self.shapes.get(&opcode).filter(|s| !s.is_empty()).cloned();
            for attempt in 0..48 {
                let shape: Vec<Kind> = match &known {
                    Some(shapes) if attempt < 40 && rng.random_range(0..10) < 8 => {
                        shapes[rng.random_range(0..shapes.len())].clone()
                    }
                    _ => (0..rng.random_range(0..10))
                        .map(|_| KINDS[rng.random_range(0..KINDS.len())])
                        .collect(),
                };
                let body = self.body(rng, &shape);
                match parse(opcode, &body) {
                    Ok(Some(m)) => {
                        let shapes = self.shapes.entry(opcode).or_default();
                        if !shapes.contains(&shape) && shapes.len() < 8 {
                            shapes.push(shape);
                        }
                        return Some((body, m));
                    }
                    Ok(None) => {}
                    Err(e) => {
                        self.parse_panics.entry(opcode).or_insert((0, e)).0 += 1;
                    }
                }
            }
            None
        }
    }

    /// Players, at the start of the real world, sending random messages:
    /// thousands, in random orders. No map ever panics, and every player's
    /// map still answers them.
    #[tokio::test]
    #[ignore = "extended"]
    async fn random_client_messages_never_panic_a_map() {
        chaos(false).await;
    }

    /// The same, as GMs, a quarter of it GM commands with random
    /// arguments.
    #[tokio::test]
    #[ignore = "extended"]
    async fn random_gm_commands_never_panic_a_map() {
        chaos(true).await;
    }

    pub(super) const GM_COMMANDS: &[&str] = &[
        "go",
        "tele",
        "gps",
        "die",
        "revive",
        "levelup",
        "gm",
        "speed",
        "online",
        "maps",
        "save",
        "announce",
        "kick",
        "spawn",
        "ticket",
        "honor",
        "arena",
        "help",
        "additem",
        "addquest",
        "learn",
        "unlearn",
        "character",
        "damage",
        "kill",
        "respawn",
        "move",
        "threat",
        "guid",
        "pos",
        "modify",
        "rested",
        "start",
        "talents",
        "tgm",
        "weather",
        "instance",
        "bg",
        "mail",
        "debug",
        "invasion",
        "event",
    ];

    const GM_WORDS: &[&str] = &[
        "on",
        "off",
        "all",
        "start",
        "stop",
        "status",
        "history",
        "list",
        "close",
        "respond",
        "week",
        "points",
        "hp",
        "mana",
        "rage",
        "energy",
        "money",
        "speed",
        "xyz",
        "reset",
        "alterac",
        "warsong",
        "arathi",
        "Chaosa",
        "Chaosb",
        "Chaosc",
        "Stormwind",
        "goldshire",
        "-1",
        "0",
        "1",
        "2",
        "60",
        "255",
        "65535",
        "4294967295",
        "99999999999999999999",
        "1.5",
        "nan",
        "inf",
        "-0",
        "1e30",
        "x",
        "",
    ];

    /// A GM command with random arguments.
    fn gm_line(rng: &mut StdRng, v: &Vocabulary) -> String {
        let mut line = format!(".{}", GM_COMMANDS[rng.random_range(0..GM_COMMANDS.len())]);
        for _ in 0..rng.random_range(0..4) {
            let arg = match rng.random_range(0..4) {
                0 => v.ids[rng.random_range(0..v.ids.len())].to_string(),
                1 => format!("{:.1}", v.floats[rng.random_range(0..v.floats.len())]),
                _ => GM_WORDS[rng.random_range(0..GM_WORDS.len())].to_owned(),
            };
            line.push(' ');
            line.push_str(&arg);
        }
        line
    }

    /// A player out of the world (sent to the character screen, kicked,
    /// or their session gone) logs back in.
    async fn back_in(realm: &Arc<Realm>, c: &mut Client, account: &str, g: Guid) {
        for _ in 0..10 {
            if realm.registry.online(g).is_some() {
                return;
            }
            if !c.try_send(CMSG_PLAYER_LOGIN { guid: g }).await {
                *c = Client::connect(realm, account);
                continue;
            }
            let entered = c
                .gets(WAIT, |m| match m {
                    ServerOpcodeMessage::SMSG_LOGIN_VERIFY_WORLD(_) => Some(true),
                    ServerOpcodeMessage::SMSG_CHARACTER_LOGIN_FAILED(_) => Some(false),
                    _ => None,
                })
                .await;
            if entered == Some(true) {
                c.guid = g;
                let _ = c.try_send(CMSG_SET_ACTIVE_MOVER { guid: g }).await;
                return;
            }
            // Still going from where it was: a moment, and again.
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("{g} can't get back into the world");
    }

    async fn chaos(gm: bool) {
        let dir = harness::real_data("db");
        let db = dir.join("vmangos.sqlite");
        if !db.exists() || crate::game::data::load::lfs_pointer(&db) {
            return;
        }
        let data = crate::game::harness::load_real(&dir);
        let mut config = harness::test_config();
        // Players or GMs; and a flood is what this is.
        config.everyone_gm = gm;
        config.max_packets_per_sec = 1_000_000;
        let realm = {
            let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
            Realm::new(config, store, startup, data, None)
        };
        let seed: u64 = std::env::var("WVS_CHAOS_SEED")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);
        let rounds: usize = std::env::var("WVS_CHAOS_ROUNDS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(40);
        let mut rng = StdRng::seed_from_u64(seed + u64::from(gm));
        let classes = [Class::Warrior, Class::Mage, Class::Priest];
        let mut players = Vec::new();
        for (i, class) in classes.into_iter().enumerate() {
            let account = format!("CHAOS{i}");
            let mut c = Client::connect(&realm, &account);
            let g = c
                .enter(&format!("Chaos{}", ["a", "b", "c"][i]), Race::Human, class)
                .await;
            c.send(CMSG_COMPLETE_CINEMATIC {}).await;
            players.push((account, c, g));
        }
        let map = realm.maps.get(&realm, MapKey::world(0));
        let mine: Vec<Guid> = players.iter().map(|p| p.2).collect();
        let (near, items, spells, entries) = map
            .query(move |m| {
                let near: Vec<u64> = m
                    .grid
                    .around(START.0, START.1, 80.0)
                    .map(|g| g.guid())
                    .collect();
                let mut items = Vec::new();
                let mut spells = Vec::new();
                for g in &mine {
                    let p = &m.players[g];
                    items.extend(p.items.iter().map(|i| i.guid.guid()));
                    spells.extend(p.spells.iter().copied());
                }
                let mut entries: Vec<u32> = m.creatures.iter().map(|(_, c)| c.entry).collect();
                entries.extend(m.gameobjects.iter().map(|(_, o)| o.entry));
                (near, items, spells, entries)
            })
            .await
            .unwrap();
        let d = &realm.data;
        let mut ids: Vec<u32> = spells;
        ids.extend(entries);
        ids.extend(d.spells.spells.keys().step_by(97).copied());
        ids.extend(d.items.keys().step_by(53).copied());
        ids.extend(d.npc.quests.keys().step_by(17).copied());
        ids.extend(d.area_triggers.keys().copied());
        ids.extend(d.maps.keys().copied());
        ids.extend(0..40);
        let mut guids: Vec<u64> = near;
        guids.extend(items);
        guids.extend(players.iter().map(|p| p.2.guid()));
        let vocabulary = Vocabulary {
            guids,
            ids,
            floats: vec![
                START.0,
                START.1,
                START.2,
                0.0,
                1.0,
                std::f32::consts::PI,
                -1.0,
                1e9,
                f32::NAN,
                f32::INFINITY,
            ],
            words: vec![
                "",
                "hello",
                "Chaosa",
                "Chaosb",
                "General",
                "x",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "Stormwind",
                "%s%n",
            ],
        };
        let mut generator = Generator {
            vocabulary,
            shapes: HashMap::new(),
            parse_panics: HashMap::new(),
        };
        // Logging out ends the chaos for that player; the rest are fair.
        let skip = [0x4A, 0x4B];
        let opcodes: Vec<u32> = client_opcodes()
            .into_iter()
            .filter(|o| !skip.contains(o) && !crate::game::conn::RAW_OPCODES.contains(o))
            .collect();
        let raw: Vec<u16> = crate::game::conn::RAW_OPCODES
            .iter()
            .map(|o| *o as u16)
            .collect();
        let total_panics = |realm: &Arc<Realm>| {
            let maps = realm.maps.all();
            async move {
                let mut n = 0;
                for m in maps {
                    n += m.query(|m| m.stats.panics).await.unwrap_or(0);
                }
                n
            }
        };
        let mut failures = Vec::new();
        let mut sent_total = 0;
        // What was sent and heard, and where the players were, to see what
        // the chaos reached.
        let mut sent_by: HashMap<u32, u32> = HashMap::new();
        let mut heard: HashMap<String, u32> = HashMap::new();
        let mut whereabouts: HashMap<String, u32> = HashMap::new();
        let started = std::time::Instant::now();
        for round in 0..rounds {
            let mut sent: Vec<String> = Vec::new();
            for _ in 0..250 {
                let who = rng.random_range(0..players.len());
                if rng.random_range(0..50) == 0 {
                    let op = raw[rng.random_range(0..raw.len())];
                    let n = rng.random_range(0..6);
                    let shape: Vec<Kind> = (0..n)
                        .map(|_| KINDS[rng.random_range(0..KINDS.len())])
                        .collect();
                    let body = generator.body(&mut rng, &shape);
                    sent.push(format!("{who}: raw 0x{op:X} {body:?}"));
                    if !players[who].1.try_send_raw(op, &body).await {
                        let account = players[who].0.clone();
                        players[who].1 = Client::connect(&realm, &account);
                    }
                    continue;
                }
                if gm && rng.random_range(0..4) == 0 {
                    let line = gm_line(&mut rng, &generator.vocabulary);
                    sent.push(format!("{who}: {line}"));
                    let said = CMSG_MESSAGECHAT {
                        chat_type: CMSG_MESSAGECHAT_ChatType::Say,
                        language: Language::Universal,
                        message: line,
                    };
                    if !players[who].1.try_send(said).await {
                        let account = players[who].0.clone();
                        players[who].1 = Client::connect(&realm, &account);
                    }
                    continue;
                }
                let op = opcodes[rng.random_range(0..opcodes.len())];
                let Some((body, msg)) = generator.message(&mut rng, op) else {
                    continue;
                };
                sent.push(format!("{who}: 0x{op:03X} {body:?} {msg:?}"));
                *sent_by.entry(op).or_default() += 1;
                if !players[who].1.try_send(msg).await {
                    // Its session went: a new connection.
                    let account = players[who].0.clone();
                    players[who].1 = Client::connect(&realm, &account);
                }
            }
            sent_total += sent.len();
            // Every player in the world again, and answered by their map:
            // what each sent before has been handled.
            for (account, c, g) in players.iter_mut() {
                back_in(&realm, c, account, *g).await;
                c.gets(Duration::ZERO, |_| None::<()>).await;
                for m in c.backlog.drain(..) {
                    let name = m.to_string();
                    let name = name.split([' ', '{', '(']).next().unwrap_or("").to_owned();
                    *heard.entry(name).or_default() += 1;
                }
                c.clear();
                if let Some(o) = realm.registry.online(*g) {
                    let me = *g;
                    let state =
                        o.map()
                            .query(move |m| {
                                // Up again, so the next round can do more than
                                // a ghost may.
                                if m.players.get(&me).is_some_and(|p| {
                                    p.death != crate::game::map::death::Death::Alive
                                }) {
                                    crate::game::map::death::resurrect(m, me, 1.0);
                                }
                                m.players.get(&me).map(|p| {
                                    format!(
                                        "{} {:?}{}",
                                        m.key,
                                        p.death,
                                        if p.awaiting_worldport {
                                            " (in transit)"
                                        } else {
                                            ""
                                        }
                                    )
                                })
                            })
                            .await
                            .flatten()
                            .unwrap_or_else(|| "between maps".to_owned());
                    *whereabouts.entry(state).or_default() += 1;
                }
                let me = *g;
                let mut answered = false;
                for _ in 0..3 {
                    back_in(&realm, c, account, me).await;
                    // Sent to another map, it arrives, as a client does.
                    let _ = c.try_send(MSG_MOVE_WORLDPORT_ACK {}).await;
                    if !c.try_send(CMSG_NAME_QUERY { guid: me }).await {
                        continue;
                    }
                    let heard = c
                        .gets(WAIT, |m| match m {
                            ServerOpcodeMessage::SMSG_NAME_QUERY_RESPONSE(r) if r.guid == me => {
                                Some(())
                            }
                            _ => None,
                        })
                        .await;
                    if heard.is_some() {
                        answered = true;
                        break;
                    }
                }
                assert!(answered, "round {round}: {g}'s map doesn't answer");
                c.clear();
            }
            let panics = total_panics(&realm).await;
            if panics > failures.len() as u64 {
                failures.push(format!(
                    "round {round}: {panics} panics after\n{}",
                    sent.join("\n")
                ));
                // One report a panic: the count is what is asserted.
                while (failures.len() as u64) < panics {
                    failures.push(String::new());
                }
            }
        }
        eprintln!(
            "chaos: {sent_total} messages over {rounds} rounds in {:.1} s; {} opcodes with a shape that parses; wow_messages panicked reading {:?}",
            started.elapsed().as_secs_f32(),
            generator.shapes.len(),
            generator.parse_panics
        );
        let mut heard: Vec<_> = heard.into_iter().collect();
        heard.sort_by_key(|h| std::cmp::Reverse(h.1));
        eprintln!("chaos: {} kinds of answer heard: {heard:?}", heard.len());
        let mut sent_by: Vec<_> = sent_by.into_iter().collect();
        sent_by.sort();
        eprintln!("chaos: sent by opcode: {sent_by:X?}");
        eprintln!("chaos: where the players were, each round: {whereabouts:?}");
        let reports: Vec<&String> = failures.iter().filter(|f| !f.is_empty()).collect();
        assert!(
            reports.is_empty(),
            "{} panics:\n{}",
            failures.len(),
            reports
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join("\n\n")
        );
    }
}
