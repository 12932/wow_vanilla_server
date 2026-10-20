//! A player between maps as their copy stops: never lost with its queue.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::scripts::MapTemplate;
use crate::game::data::*;
use crate::game::harness::{self, Client};
use crate::game::map::{Arrival, ArrivalKind, MapEvent, MapHandle, MapKey};

const DUNGEON: u32 = 36;

fn realm() -> Arc<Realm> {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = true;
    cfg.instance_unload = Duration::from_secs(600);
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
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
    Realm::new(cfg, store, startup, d, None)
}

fn map_of(realm: &Realm, g: wow_world_messages::Guid) -> MapHandle {
    realm.registry.online(g).unwrap().map()
}

/// A player sent to a copy with a handle taken just before it stopped (as
/// a session asks for the copy, and the copy unloads before the player is
/// sent) isn't lost with it: the send is refused, and they go into the
/// copy started again, where they are saved as they leave. Before, the
/// send was dropped, and them with it: unsaved, online, and in no map.
#[tokio::test]
async fn a_player_sent_with_a_handle_to_a_copy_just_stopped_arrives_in_it_again() {
    let realm = realm();
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let key = MapKey {
        map: DUNGEON,
        instance: 7,
    };
    let stale = realm.maps.get(&realm, key);
    realm.maps.stop(key);
    for _ in 0..500 {
        if stale.is_closed() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(stale.is_closed(), "the copy has stopped");
    let mut player = map_of(&realm, g)
        .query(move |m| m.take_player(g))
        .await
        .flatten()
        .expect("Alice");
    player.movement.position = Vector3d {
        x: -16.0,
        y: -383.0,
        z: 61.0,
    };
    let online = player.online.clone();
    let session = online.session.clone();
    let took = crate::game::map::deliver(
        &realm,
        stale,
        Box::new(Arrival {
            kind: ArrivalKind::Transfer(Box::new(player)),
            online,
        }),
    )
    .expect("taken by a map");
    assert_eq!(took.key, key, "the copy asked for");
    assert!(!took.is_closed(), "started again");
    let mut there = false;
    for _ in 0..500 {
        if took.query(move |m| m.players.contains_key(&g)).await == Some(true) {
            there = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(there, "Alice is in the copy started again");
    assert_eq!(map_of(&realm, g).key, key, "and online there");
    // Her client goes: she is saved where she is.
    took.send(MapEvent::Disconnect(g, session));
    for _ in 0..500 {
        if realm.registry.online(g).is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(realm.registry.online(g).is_none(), "she has left");
    realm.store.flush().await;
    let row = realm
        .store
        .load(crate::game::guid::counter(g))
        .await
        .unwrap();
    assert_eq!(
        (row.map, row.x, row.y),
        (DUNGEON, -16.0, -383.0),
        "saved there"
    );
}
