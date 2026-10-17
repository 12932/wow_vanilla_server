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
