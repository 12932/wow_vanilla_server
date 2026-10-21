//! A client gone leaves its character in the world for a minute, as
//! mangos-classic's `WorldSession::SetOffline` and `ShouldDisconnect` keep
//! them: still fighting and attackable, for a login of the account to take
//! them over where they are (`HandlePlayerReconnect`), then saved and gone.
//! No Alt-F4 out of a fight. A kick (`KickPlayer`) takes them out at once.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage as S;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::fields::{UNIT_FIELD_FLAGS, UNIT_FIELD_MAXHEALTH};
use crate::game::harness::{self, Client, WAIT};
use crate::game::map::death::Death;
use crate::game::map::player::UNIT_FLAG_IN_COMBAT;
use crate::game::map::{Map, MapHandle, combat, login};
use crate::game::session::Control;
use crate::game::{Realm, RealmConfig};

fn realm() -> Arc<Realm> {
    let cfg = RealmConfig {
        disconnect_linger: login::DISCONNECT_LINGER,
        ..harness::test_config()
    };
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(cfg, store, startup, GameData::empty(), None)
}

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map answers")
}

fn map_of(realm: &Arc<Realm>, g: Guid) -> MapHandle {
    realm.registry.online(g).expect("online").map()
}

/// Wait, a short poll up to a limit, until `g`'s client has gone: their
/// session closed, they online still.
async fn gone_but_online(realm: &Arc<Realm>, g: Guid) {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        match realm.registry.online(g) {
            Some(o) if o.session.is_closed() => return,
            Some(_) => {}
            None => panic!("{g:?} went offline at their client's going"),
        }
        assert!(tokio::time::Instant::now() < deadline, "the session stays");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Wait until the map has `g` lingering: the word of their client's going
/// has reached it.
async fn lingering(map: &MapHandle, g: Guid) {
    let deadline = tokio::time::Instant::now() + WAIT;
    while !on(map, move |m| {
        m.players.get(&g).is_some_and(|p| p.linger_until.is_some())
    })
    .await
    {
        assert!(tokio::time::Instant::now() < deadline, "never lingering");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// The minute run out: what the map's clock would come to.
async fn linger_ends(map: &MapHandle, g: Guid) {
    on(map, move |m| {
        let now = m.now;
        m.players.get_mut(&g).expect("in the world").linger_until = Some(now);
    })
    .await;
}

/// Create a character on the character screen, not logging it in.
async fn create(c: &mut Client, name: &str) -> Guid {
    c.send(CMSG_CHAR_CREATE {
        name: name.to_owned(),
        race: Race::Human,
        class: Class::Warrior,
        gender: Gender::Female,
        skin_color: 1,
        face: 1,
        hair_style: 1,
        hair_color: 1,
        facial_hair: 1,
    })
    .await;
    c.wait_for("SMSG_CHAR_CREATE", |m| match m {
        S::SMSG_CHAR_CREATE(r) => Some(r.result),
        _ => None,
    })
    .await;
    c.send(CMSG_CHAR_ENUM {}).await;
    c.wait_for("SMSG_CHAR_ENUM", |m| match m {
        S::SMSG_CHAR_ENUM(e) => e.characters.iter().find(|x| x.name == name).map(|x| x.guid),
        _ => None,
    })
    .await
}

/// A client that goes leaves its character in the world: in their map,
/// online, a fight's flag still up, and attackable: a blow there kills
/// them. At the minute's end they leave, saved, and (dead and not
/// released) come back a ghost. Before, they left the world at once, the
/// fight ended with them.
#[tokio::test]
async fn a_client_gone_leaves_its_character_in_the_world_attackable_for_a_minute() {
    let realm = realm();
    let mut a = Client::connect(&realm, "LINGER");
    let g = a.enter("Lingerer", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "FOE");
    let foe = b.enter("Foe", Race::Human, Class::Warrior).await;
    let map = map_of(&realm, g);
    on(&map, move |m| combat::engage(m, foe, g)).await;
    a.disconnect();
    gone_but_online(&realm, g).await;
    lingering(&map, g).await;
    harness::an_update(&map).await;
    harness::an_update(&map).await;
    let fighting = on(&map, move |m| {
        m.players
            .get(&g)
            .map(|p| p.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT))
    })
    .await;
    assert_eq!(fighting, Some(true), "in the world, in the fight");
    let dead = on(&map, move |m| {
        let max = m.unit_fields(g).unwrap().get_u32(UNIT_FIELD_MAXHEALTH);
        combat::deal_damage(m, foe, g, max + 10);
        m.players[&g].death
    })
    .await;
    assert!(matches!(dead, Death::Dead { .. }), "killed: {dead:?}");
    linger_ends(&map, g).await;
    harness::offline(&realm, g).await;
    assert!(on(&map, move |m| !m.players.contains_key(&g)).await);
    let mut c = Client::connect(&realm, "LINGER");
    c.login(g).await;
    let death = on(&map_of(&realm, g), move |m| m.players[&g].death).await;
    assert!(matches!(death, Death::Ghost { .. }), "a ghost: {death:?}");
}

/// A login of the account within the minute takes the character over
/// where they are, as they are: money not yet saved, its linger over.
/// Before, the character had left, and came back as last saved.
#[tokio::test]
async fn a_reconnect_within_the_minute_takes_the_character_over_where_they_are() {
    let realm = realm();
    let mut a = Client::connect(&realm, "BACK");
    let g = a.enter("Comeback", Race::Human, Class::Warrior).await;
    let map = map_of(&realm, g);
    on(&map, move |m| {
        m.players.get_mut(&g).unwrap().set_money(4321)
    })
    .await;
    a.disconnect();
    gone_but_online(&realm, g).await;
    lingering(&map, g).await;
    let mut b = Client::connect(&realm, "BACK");
    b.login(g).await;
    let kept = on(&map, move |m| {
        let p = &m.players[&g];
        (p.money(), p.linger_until, p.online.session.is_closed())
    })
    .await;
    assert_eq!(kept, (4321, None, false), "the same body, theirs again");
    harness::an_update(&map).await;
    assert!(on(&map, move |m| m.players.contains_key(&g)).await);
}

/// At the minute's end the character leaves, saved: what they had when
/// their client went is there at the next login.
#[tokio::test]
async fn at_the_minutes_end_the_character_leaves_saved() {
    let realm = realm();
    let mut a = Client::connect(&realm, "SAVED");
    let g = a.enter("Savedone", Race::Human, Class::Warrior).await;
    let map = map_of(&realm, g);
    a.disconnect();
    gone_but_online(&realm, g).await;
    lingering(&map, g).await;
    on(&map, move |m| m.players.get_mut(&g).unwrap().set_money(777)).await;
    harness::an_update(&map).await;
    assert!(
        realm.registry.online(g).is_some(),
        "still in the world within the minute"
    );
    linger_ends(&map, g).await;
    harness::offline(&realm, g).await;
    let mut b = Client::connect(&realm, "SAVED");
    b.login(g).await;
    let money = on(&map_of(&realm, g), move |m| m.players[&g].money()).await;
    assert_eq!(money, 777);
}

/// A kick takes the character out of the world at once, as mangos's
/// `KickPlayer` logs them out: no linger.
#[tokio::test]
async fn a_kicked_client_leaves_the_world_at_once() {
    let realm = realm();
    let mut a = Client::connect(&realm, "KICKED");
    let g = a.enter("Kicked", Race::Human, Class::Warrior).await;
    let _ = realm
        .registry
        .online(g)
        .expect("online")
        .session
        .send(Control::Kick);
    harness::offline(&realm, g).await;
}

/// While one character lingers, another of the account is refused
/// (mangos's `CHAR_LOGIN_DUPLICATE_CHARACTER`), and the one lingering
/// stays: logging in another can't take them out of a fight early. The
/// same character is taken over.
#[tokio::test]
async fn another_character_of_the_account_is_refused_while_one_lingers() {
    let realm = realm();
    let mut a = Client::connect(&realm, "TWO");
    let other = create(&mut a, "Secondone").await;
    let g = a.enter("Firstone", Race::Human, Class::Warrior).await;
    let map = map_of(&realm, g);
    a.disconnect();
    gone_but_online(&realm, g).await;
    lingering(&map, g).await;
    let mut b = Client::connect(&realm, "TWO");
    b.send(CMSG_PLAYER_LOGIN { guid: other }).await;
    let result = b
        .wait_for("SMSG_CHARACTER_LOGIN_FAILED", |m| match m {
            S::SMSG_CHARACTER_LOGIN_FAILED(f) => Some(f.result),
            _ => None,
        })
        .await;
    assert_eq!(result, WorldResult::CharLoginDuplicateCharacter);
    harness::an_update(&map).await;
    assert!(
        on(&map, move |m| m.players.contains_key(&g)).await,
        "the first lingers on"
    );
    assert!(realm.registry.online(other).is_none());
    b.login(g).await;
    assert!(on(&map, move |m| m.players[&g].linger_until.is_none()).await);
}

/// One who logs out, rather than closing the client, leaves at once: the
/// linger is for a client gone.
#[tokio::test]
async fn a_logout_leaves_at_once() {
    let realm = realm();
    let mut a = Client::connect(&realm, "LOGOUT");
    let g = a.enter("Loggedout", Race::Human, Class::Warrior).await;
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, S::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    assert!(realm.registry.online(g).is_none());
}

/// A second connection of the account, the first still up, replaces it
/// (one session an account), and the character lingers from then, as
/// mangos's `SetOffline` has them when a new socket takes the session: the
/// new session has the minute to take them over, and they don't stay
/// past it while it sits on the character screen. Before, they stayed in
/// the world with no clock at all until the new session logged in or
/// went.
#[tokio::test]
async fn a_second_connection_starts_the_linger_and_takes_them_over() {
    let realm = realm();
    let mut a = Client::connect(&realm, "SECOND");
    let g = a.enter("Secondconn", Race::Human, Class::Warrior).await;
    let map = map_of(&realm, g);
    let mut b = Client::connect(&realm, "SECOND");
    lingering(&map, g).await;
    b.login(g).await;
    assert!(on(&map, move |m| m.players[&g].linger_until.is_none()).await);
    // Replaced again, and left on the screen: at the minute's end they go.
    let _b2 = Client::connect(&realm, "SECOND");
    lingering(&map, g).await;
    linger_ends(&map, g).await;
    harness::offline(&realm, g).await;
    drop(b);
    drop(a);
}
