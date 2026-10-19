//! Round 6's player tests of dungeon copies, ported from Thistle Tea's
//! `test/game/world/entity/player/instances_test.exs` (docs/parity/matrix.csv):
//! the minute's grace of one in a copy no longer theirs.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::scripts::MapTemplate;
use crate::game::data::{GameData, MapInfo};
use crate::game::harness::{self, Client};
use crate::game::map::MapKey;
use crate::game::social::groups;

/// Shadowfang Keep's map, a dungeon of ten.
const DUNGEON: u32 = 33;

fn copies() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    d.maps.insert(
        DUNGEON,
        MapInfo {
            id: DUNGEON,
            internal_name: "Map33".into(),
            name: "Map 33".into(),
            instance_type: 1,
            player_limit: 0,
        },
    );
    d.scripts.map_templates.insert(
        DUNGEON,
        MapTemplate {
            map: DUNGEON,
            map_type: 1,
            player_limit: 10,
            ..Default::default()
        },
    );
    d
}

/// Players who are no GMs, and copies that last a while empty.
fn realm() -> Arc<Realm> {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    cfg.instance_unload = Duration::from_secs(600);
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(cfg, store, startup, copies(), None)
}

async fn player(realm: &Arc<Realm>, name: &str) -> Client {
    let mut c = Client::connect(realm, name);
    c.enter(name, Race::Human, Class::Warrior).await;
    c
}

/// Send `c` into `map`'s copy (or the open world), and wait for it to land.
async fn go(realm: &Arc<Realm>, c: &mut Client, map: u32) {
    let g = c.guid;
    let from = realm.registry.online(g).unwrap().map();
    from.call(move |m| m.teleport(g, MapKey::world(map), Vector3d::default(), 0.0));
    c.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    c.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    c.sees(g).await;
}

fn key_of(realm: &Realm, g: Guid) -> MapKey {
    realm.registry.online(g).unwrap().map().key
}

/// The next grace `c` is told, in milliseconds (0: it's over), within
/// `within`.
async fn grace(c: &mut Client, within: Duration) -> Option<u32> {
    c.gets(within, |m| match m {
        ServerOpcodeMessage::SMSG_RAID_GROUP_ONLY(r) => Some(r.homebind_timer),
        _ => None,
    })
    .await
}

/// `g`'s grace as if its minute were up now.
async fn deadline_now(realm: &Arc<Realm>, g: Guid) {
    let map = realm.registry.online(g).unwrap().map();
    crate::game::tests_p3_entity::on(&map, move |m| {
        let now = m.now;
        if let Some(p) = m.players.get_mut(&g) {
            p.eviction = Some(now);
        }
    })
    .await;
}

/// Alice and Bob in a party, in its copy of the dungeon.
async fn party_inside(realm: &Arc<Realm>) -> (Client, Client) {
    let mut a = player(realm, "Alice").await;
    let mut b = player(realm, "Bob").await;
    let (ao, bo) = (
        realm.registry.online(a.guid).unwrap(),
        realm.registry.online(b.guid).unwrap(),
    );
    assert!(groups::join_directly(realm, &ao, &bo));
    go(realm, &mut a, DUNGEON).await;
    go(realm, &mut b, DUNGEON).await;
    assert_eq!(key_of(realm, a.guid), key_of(realm, b.guid));
    (a, b)
}

/// Bob out of Alice's party.
fn part(realm: &Arc<Realm>, b: Guid) {
    groups::leave(realm, &realm.registry.online(b).unwrap());
}

/// Thistle Tea's `Instances.expire` "teleports home once after the
/// deadline and revalidates eligibility": out of the party that owns the
/// copy, Bob has a minute's grace; back in it, he's told it's over, and a
/// deadline come while he belongs again sends him nowhere; out again, at
/// the deadline he goes home, told it's over, and once.
#[tokio::test]
async fn a_copys_grace_sends_home_at_its_deadline_those_who_still_dont_belong() {
    let realm = realm();
    let (a, mut b) = party_inside(&realm).await;
    let (ga, gb) = (a.guid, b.guid);
    let copy = key_of(&realm, gb);
    part(&realm, gb);
    assert_eq!(grace(&mut b, harness::WAIT).await, Some(60_000), "a minute");
    let (ao, bo) = (
        realm.registry.online(ga).unwrap(),
        realm.registry.online(gb).unwrap(),
    );
    assert!(groups::join_directly(&realm, &ao, &bo));
    assert_eq!(grace(&mut b, harness::WAIT).await, Some(0), "back in time");
    deadline_now(&realm, gb).await;
    assert_eq!(grace(&mut b, harness::WAIT).await, Some(0), "belongs at it");
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    assert_eq!(key_of(&realm, gb), copy, "stays");
    part(&realm, gb);
    assert_eq!(grace(&mut b, harness::WAIT).await, Some(60_000));
    b.clear();
    deadline_now(&realm, gb).await;
    assert_eq!(grace(&mut b, harness::WAIT).await, Some(0), "over");
    b.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    b.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    b.sees(gb).await;
    assert_eq!(
        key_of(&realm, gb),
        MapKey::world(0),
        "home, where he started"
    );
    assert_eq!(
        grace(&mut b, Duration::from_millis(1_500)).await,
        None,
        "once"
    );
}

/// Thistle Tea's `Instances.refresh` "clears the timer on world transfer",
/// "clears the timer on logout" and "does not start a timer in open
/// worlds", and `restore` "returns to the home bind when the old instance
/// refuses admission": a worldport out of the copy ends the grace, told
/// so; in the open world, none starts; one logged out in it has none when
/// back, and comes back home, the copy not theirs.
#[tokio::test]
async fn a_copys_grace_ends_with_a_worldport_out_and_a_logout_and_none_starts_outside() {
    let realm = realm();
    let (_a, mut b) = party_inside(&realm).await;
    let gb = b.guid;
    part(&realm, gb);
    assert_eq!(grace(&mut b, harness::WAIT).await, Some(60_000));
    b.clear();
    go(&realm, &mut b, 0).await;
    assert_eq!(
        grace(&mut b, Duration::from_millis(1)).await,
        Some(0),
        "over"
    );
    assert_eq!(
        grace(&mut b, Duration::from_millis(1_500)).await,
        None,
        "no grace in the open world"
    );
    // In again, alone: his own copy (a fresh one); with Alice's party's
    // copy refused to him on a login, he comes back home.
    let mut c = player(&realm, "Cid").await;
    let (ao, co) = (
        realm.registry.online(_a.guid).unwrap(),
        realm.registry.online(c.guid).unwrap(),
    );
    assert!(groups::join_directly(&realm, &ao, &co));
    go(&realm, &mut c, DUNGEON).await;
    let theirs = key_of(&realm, c.guid);
    part(&realm, c.guid);
    assert_eq!(grace(&mut c, harness::WAIT).await, Some(60_000));
    let gc = c.guid;
    c.send(CMSG_LOGOUT_REQUEST {}).await;
    c.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    realm.store.flush().await;
    c.clear();
    c.login(gc).await;
    c.sees(gc).await;
    let back = key_of(&realm, gc);
    assert_ne!(back, theirs, "not into a copy no longer his");
    assert_eq!(back, MapKey::world(0), "home, where he started");
    assert_eq!(
        grace(&mut c, Duration::from_millis(1_500)).await,
        None,
        "no grace carried over"
    );
}

/// Thistle Tea's `Instances.reject` "projects the distinct vanilla
/// admission failures": a sixth fresh copy in an hour is refused with
/// `SMSG_TRANSFER_ABORTED` too many instances, the player left where they
/// were (mangos's `CanPlayerEnter`, `TRANSFER_ABORT_TOO_MANY_INSTANCES`);
/// a full copy is `tests_ai5_instances`'.
#[tokio::test]
async fn a_sixth_fresh_copy_in_an_hour_is_refused_too_many_instances() {
    let realm = realm();
    let mut a = player(&realm, "Alice").await;
    let g = a.guid;
    let mut seen = Vec::new();
    for _ in 0..5 {
        go(&realm, &mut a, DUNGEON).await;
        seen.push(key_of(&realm, g));
        go(&realm, &mut a, 0).await;
        // Reset: the next is a fresh copy.
        a.send(CMSG_RESET_INSTANCES {}).await;
        crate::game::tests_p3_player2_ammo::handled(&mut a).await;
    }
    seen.dedup();
    assert_eq!(seen.len(), 5, "five copies: {seen:?}");
    a.clear();
    let from = realm.registry.online(g).unwrap().map();
    from.call(move |m| m.teleport(g, MapKey::world(DUNGEON), Vector3d::default(), 0.0));
    let reason = a
        .wait_for("SMSG_TRANSFER_ABORTED", |m| match m {
            ServerOpcodeMessage::SMSG_TRANSFER_ABORTED(t) => Some(t.reason),
            ServerOpcodeMessage::SMSG_NEW_WORLD(_) => panic!("let in"),
            _ => None,
        })
        .await;
    assert_eq!(reason, TransferAbortReason::TooManyInstances);
    assert_eq!(key_of(&realm, g), MapKey::world(0), "where she was");
}
