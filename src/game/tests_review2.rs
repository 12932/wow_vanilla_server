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
