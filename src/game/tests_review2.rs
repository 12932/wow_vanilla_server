//! The map core's review (review 2): each finding fixed, shown through the
//! protocol where a client can show it.

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
use crate::game::map::{Map, MapHandle, MapKey};

const DUNGEON: u32 = 36;

fn realm(data: GameData, everyone_gm: bool, unload: Duration) -> Arc<Realm> {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = everyone_gm;
    cfg.instance_unload = unload;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(cfg, store, startup, data, None)
}

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

fn map_of(realm: &Realm, g: Guid) -> MapHandle {
    realm.registry.online(g).unwrap().map()
}

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map answers")
}

/// Send `c` to `map` (its copy, or the open world) and wait for it to land.
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

/// A dungeon copy its group leaves (a wipe and the run back, a trip out to
/// repair) is kept while it lasts empty, however long ago it last changed:
/// the realm's sweep counts its empty time from when it emptied, as the
/// copy itself does, rather than from its last kill.
#[tokio::test]
async fn a_dungeon_copy_left_is_kept_however_long_since_it_changed() {
    let realm = realm(dungeon(), false, Duration::from_secs(600));
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    go(&realm, &mut a, DUNGEON).await;
    let copy = map_of(&realm, a.guid).key;
    assert_ne!(copy.instance, 0);
    let handle = map_of(&realm, a.guid);
    // An hour inside without a kill.
    {
        let mut inst = realm.social.instances.lock();
        inst.saves.get_mut(&copy.instance).unwrap().touched_at -= 3600;
    }
    go(&realm, &mut a, 0).await;
    let mut emptied = false;
    for _ in 0..200 {
        if on(&handle, |m| m.players.is_empty() && m.empty_since.is_some()).await {
            emptied = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(emptied, "the copy's own clock runs");
    instances::sweep(&realm);
    assert!(
        instances::save_of(&realm, copy.instance).is_some(),
        "the save is kept while the copy lasts"
    );
    go(&realm, &mut a, DUNGEON).await;
    assert_eq!(map_of(&realm, a.guid).key, copy, "back into the same copy");
}

/// Log `c` out by going away, and back in on a new connection.
async fn relog(realm: &Arc<Realm>, c: Client) -> Client {
    let (g, account) = (c.guid, c.account.clone());
    c.disconnect();
    for _ in 0..500 {
        if realm.registry.online(g).is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(realm.registry.online(g).is_none(), "logged out");
    realm.store.flush().await;
    let mut c = Client::connect(realm, &account);
    c.login(g).await;
    c.sees(g).await;
    c
}

/// The bank's bag slots bought are kept: they are saved, and come back at
/// the next login (mangos keeps them in `playerBytes2`).
#[tokio::test]
async fn bank_bag_slots_bought_survive_a_relog() {
    let realm = realm(GameData::empty(), true, Duration::from_secs(600));
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    on(&map_of(&realm, g), move |m| {
        m.players
            .get_mut(&g)
            .unwrap()
            .fields
            .set_byte(PLAYER_BYTES_2, 2, 3)
    })
    .await;
    let a = relog(&realm, a).await;
    let slots = on(&map_of(&realm, g), move |m| {
        crate::game::map::inventory::bank_bag_slots(&m.players[&g])
    })
    .await;
    assert_eq!(slots, 3, "the three slots bought");
    drop(a);
}

fn aboard(position: Vector3d, offset: Vector3d) -> MSG_MOVE_HEARTBEAT_Client {
    MSG_MOVE_HEARTBEAT_Client {
        info: MovementInfo {
            flags: MovementInfo_MovementFlags::empty().set_on_transport(
                MovementInfo_MovementFlags_OnTransport {
                    transport: TransportInfo {
                        guid: crate::game::guid::make(
                            crate::game::guid::HighGuid::MoTransport,
                            0,
                            20808,
                        ),
                        position: offset,
                        orientation: 0.0,
                    },
                },
            ),
            timestamp: 0,
            position,
            orientation: 0.0,
            fall_time: 0,
        },
    }
}

/// A place on a transport's deck that no deck has (NaN, or a mile off) is
/// dropped, as mangos's `VerifyMovementInfo` drops it: not taken, relayed
/// to those watching, or saved for the next login.
#[tokio::test]
async fn a_place_on_deck_off_any_deck_is_dropped() {
    let realm = realm(GameData::empty(), true, Duration::from_secs(600));
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    b.sees(g).await;
    let map = map_of(&realm, g);
    let at = on(&map, move |m| m.players[&g].position()).await;
    let nan = Vector3d {
        x: f32::NAN,
        y: 0.0,
        z: 0.0,
    };
    let far = Vector3d {
        x: 5000.0,
        y: 0.0,
        z: 0.0,
    };
    for offset in [nan, far] {
        a.send(aboard(at, offset)).await;
    }
    // A good one after: taken, so the bad ones were looked at first.
    let deck = Vector3d {
        x: 2.0,
        y: -1.0,
        z: 3.0,
    };
    a.send(aboard(at, deck)).await;
    // What Bob is told of Alice's moves, in order, up to the good one.
    let mut relayed = Vec::new();
    loop {
        let offset = b
            .wait_for("Alice's heartbeat", |m| match m {
                ServerOpcodeMessage::MSG_MOVE_HEARTBEAT(h) if h.guid == g => h
                    .info
                    .flags
                    .get_on_transport()
                    .map(|t| t.transport.position),
                _ => None,
            })
            .await;
        relayed.push(offset.x);
        if offset.x == deck.x {
            break;
        }
    }
    assert_eq!(relayed, vec![deck.x], "only the deck's place was relayed");
    let kept = on(&map, move |m| {
        m.players[&g]
            .movement
            .flags
            .get_on_transport()
            .map(|t| t.transport.position.x)
    })
    .await;
    assert_eq!(kept, Some(deck.x));
}

/// The first move after logging in is checked as every other is: one a
/// long way from where the player came into the world is pulled back.
#[tokio::test]
async fn the_first_move_after_logging_in_is_checked() {
    let realm = realm(GameData::empty(), false, Duration::from_secs(600));
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = map_of(&realm, g);
    let at = on(&map, move |m| m.players[&g].position()).await;
    a.clear();
    a.heartbeat(at.x + 500.0, at.y, at.z).await;
    let back = a
        .gets(Duration::from_secs(2), |m| match m {
            ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(t) => Some(t.info.position),
            _ => None,
        })
        .await;
    let back = back.expect("pulled back");
    assert!((back.x - at.x).abs() < 0.01, "to where she was: {back:?}");
    let now = on(&map, move |m| m.players[&g].position()).await;
    assert!((now.x - at.x).abs() < 0.01, "and there: {now:?}");
}

/// A passenger is carried by the ship, not by their legs: a move on deck
/// isn't checked for speed, however far the world place goes between two
/// heartbeats (ashore, the same move is pulled back, as above).
#[tokio::test]
async fn a_move_on_deck_is_not_pulled_back_for_speed() {
    let realm = realm(GameData::empty(), false, Duration::from_secs(600));
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = map_of(&realm, g);
    let at = on(&map, move |m| m.players[&g].position()).await;
    let deck = Vector3d {
        x: 2.0,
        y: -1.0,
        z: 3.0,
    };
    a.send(aboard(at, deck)).await;
    let far = Vector3d {
        x: at.x + 500.0,
        ..at
    };
    a.clear();
    a.send(aboard(far, deck)).await;
    let back = a
        .gets(Duration::from_secs(1), |m| match m {
            ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(t) => Some(t.info.position),
            _ => None,
        })
        .await;
    assert_eq!(back, None, "not pulled back on deck");
    let now = on(&map, move |m| m.players[&g].position()).await;
    assert!((now.x - far.x).abs() < 0.01, "carried: {now:?}");
}

/// A ban too long to count to is for good, where its end wrapped into the
/// past and `.ban` said "banned" of an account free to log in.
#[tokio::test]
async fn a_ban_too_long_to_count_is_for_good() {
    crate::accounts::install(crate::accounts::Accounts::in_memory(
        crate::accounts::Creation::Auto,
        false,
    ));
    let accounts = crate::accounts::get().unwrap();
    accounts.set_password("FOREVER", "PW").unwrap();
    let realm = realm(GameData::empty(), true, Duration::from_secs(600));
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    a.say(".ban forever 9223372036854775807 testing").await;
    let reply = a
        .wait_for("the GM's answer", |m| match m {
            ServerOpcodeMessage::SMSG_MESSAGECHAT(c) => Some(c.message.clone()),
            _ => None,
        })
        .await;
    assert_eq!(reply, "FOREVER is banned.");
    assert!(accounts.banned("FOREVER"), "and banned");
}

/// `.levelup` by more levels than there are is the last level, where the
/// sum overflowed (a panic in a debug build, level 1 in a release one).
#[tokio::test]
async fn levelup_by_too_many_is_the_last_level() {
    let realm = realm(GameData::empty(), true, Duration::from_secs(600));
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.say(".levelup 2147483647").await;
    let mut level = 1;
    for _ in 0..100 {
        level = on(&map_of(&realm, g), move |m| m.players[&g].level()).await;
        if level != 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(level, 60);
}

/// A copy that stops (unloaded, or reset) with a player already sent its
/// way doesn't take them with it: they go on into the copy started again,
/// where before they were dropped, unsaved, still online and in no map.
#[tokio::test]
async fn a_player_on_the_way_into_a_copy_that_stops_arrives() {
    let realm = realm(dungeon(), true, Duration::from_secs(600));
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let key = MapKey {
        map: DUNGEON,
        instance: 7,
    };
    let copy = realm.maps.get(&realm, key);
    // Alice leaves her map for the copy; the copy stops before it has
    // heard of her.
    let player = on(&map_of(&realm, g), move |m| m.take_player(g))
        .await
        .expect("Alice");
    let online = player.online.clone();
    on(&copy, move |m| {
        m.realm.maps.stop(m.key);
        m.handle.send(crate::game::map::MapEvent::Arrive(Box::new(
            crate::game::map::Arrival {
                kind: crate::game::map::ArrivalKind::Transfer(Box::new(player)),
                online,
            },
        )));
    })
    .await;
    let mut there = false;
    for _ in 0..200 {
        let now = map_of(&realm, g);
        if now.key == key
            && !now.is_closed()
            && now.query(move |m| m.players.contains_key(&g)).await == Some(true)
        {
            there = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(there, "Alice is in the copy started again");
}

/// The Stockade, for one: a copy with a single place.
const ONE_PLACE: u32 = 34;

/// Alice and Bob, in a party.
async fn party(realm: &Arc<Realm>) -> (Client, Client) {
    let mut a = Client::connect(realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    let (ao, bo) = (
        realm.registry.online(a.guid).unwrap(),
        realm.registry.online(b.guid).unwrap(),
    );
    assert!(crate::game::social::groups::join_directly(realm, &ao, &bo));
    (a, b)
}

type Given = Result<MapKey, instances::Refused>;

/// `key_for` for `a` and `b` at once, each on a thread of its own, `n`
/// times, each time into a copy of `map` the group has yet to make: what
/// each was given.
fn at_once(realm: &Arc<Realm>, a: Guid, b: Guid, map: u32, n: usize) -> Vec<(Given, Given)> {
    tokio::task::block_in_place(|| {
        (0..n)
            .map(|_| {
                realm.social.instances.lock().forget_admissions();
                let start = std::sync::Barrier::new(2);
                std::thread::scope(|s| {
                    let one = s.spawn(|| {
                        start.wait();
                        instances::key_for(realm, a, false, map)
                    });
                    let other = s.spawn(|| {
                        start.wait();
                        instances::key_for(realm, b, false, map)
                    });
                    (one.join().unwrap(), other.join().unwrap())
                })
            })
            .collect()
    })
}

/// Review 2's M2: two of a group going into a dungeon at once, each from
/// their own map's thread, go into the same copy, the one the first of
/// them made. Before, `key_for` let go of the copies between choosing a
/// new copy and storing it, so each could make one: the second went into
/// a copy their group didn't own, and was put out a minute later.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_of_a_group_going_in_at_once_go_into_one_copy() {
    let realm = realm(dungeon(), false, Duration::from_secs(600));
    let (a, b) = party(&realm).await;
    for (i, given) in at_once(&realm, a.guid, b.guid, DUNGEON, 300)
        .into_iter()
        .enumerate()
    {
        match given {
            (Ok(one), Ok(other)) => assert_eq!(one, other, "time {i}: one copy"),
            other => panic!("time {i}: both let in, not {other:?}"),
        }
    }
}

/// Review 2's M2, its admission: a copy with one place left lets in one of
/// two going in at once, and turns the other away as full. Before, the
/// count was of those already inside, and neither was yet, so both were
/// let in.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_copys_last_place_goes_to_one_of_two_at_once() {
    let mut data = dungeon();
    data.maps.insert(
        ONE_PLACE,
        MapInfo {
            id: ONE_PLACE,
            internal_name: "StormwindJail".into(),
            name: "The Stockade".into(),
            instance_type: 1,
            player_limit: 0,
        },
    );
    data.scripts.map_templates.insert(
        ONE_PLACE,
        MapTemplate {
            map: ONE_PLACE,
            map_type: 1,
            player_limit: 1,
            ..Default::default()
        },
    );
    let realm = realm(data, false, Duration::from_secs(600));
    let (a, b) = party(&realm).await;
    let full = Err(instances::Refused::Abort(TransferAbortReason::IsFull));
    for (i, given) in at_once(&realm, a.guid, b.guid, ONE_PLACE, 300)
        .into_iter()
        .enumerate()
    {
        let (one, other) = &given;
        assert!(
            (one.is_ok() && *other == full) || (*one == full && other.is_ok()),
            "time {i}: one let in, one full, not {given:?}"
        );
    }
}

const WIDGET: u32 = 990_201;

/// Review 2's M3: a player between maps as the shutdown begins (taken
/// from one, and handed by their session to a map not yet started) is
/// saved where they were going: their place, level and items on the
/// record when the shutdown is done. Before, the map was started after
/// the shutdown had told the maps to save everyone, never heard of it,
/// and kept them, unsaved.
#[tokio::test]
async fn a_player_between_maps_as_the_shutdown_begins_is_saved() {
    let mut data = GameData::empty();
    data.items.insert(
        WIDGET,
        ItemTemplate {
            entry: WIDGET,
            name: "Widget".into(),
            display_id: 1000,
            stackable: 1,
            allowable_class: -1,
            allowable_race: -1,
            ..Default::default()
        },
    );
    let realm = realm(data, false, Duration::from_secs(600));
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let from = map_of(&realm, g);
    let kalimdor = MapKey::world(1);
    assert!(
        realm.maps.all().iter().all(|h| h.key != kalimdor),
        "Kalimdor not started"
    );
    // Level 5 and a widget since her last save, and she leaves the
    // Eastern Kingdoms for Orgrimmar, as a teleport takes her.
    let mut player = on(&from, move |m| {
        let data = m.realm.data.clone();
        crate::game::map::stats::set_level(m.players.get_mut(&g).unwrap(), &data, 5);
        crate::game::map::inventory::add_item(m, g, WIDGET, 1, false).expect("room");
        m.take_player(g)
    })
    .await
    .expect("Alice");
    player.movement.position = Vector3d {
        x: 1629.0,
        y: -4373.0,
        z: 31.0,
    };
    let session = player.online.session.clone();
    // The shutdown begins before her session hands her on.
    let shutdown = tokio::spawn({
        let realm = realm.clone();
        async move { realm.shutdown_within(Duration::from_secs(5)).await }
    });
    while !realm.maps.is_closed() {
        tokio::task::yield_now().await;
    }
    crate::game::session::transfer(&realm, &session, player, kalimdor);
    shutdown.await.unwrap();
    let row = realm
        .store
        .load(crate::game::guid::counter(g))
        .await
        .unwrap();
    assert_eq!((row.map, row.x, row.y), (1, 1629.0, -4373.0), "her place");
    assert_eq!(row.level, 5, "her level");
    assert!(
        row.items.iter().any(|i| i.entry == WIDGET),
        "her widget: {:?}",
        row.items
    );
    assert!(realm.registry.online(g).is_none(), "and she is offline");
}
