//! The bug hunt's fixes in the map's core: what the client claims, what
//! the map shows and keeps, through the protocol.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::harness::{self, Client};
use crate::game::map::{MapHandle, MapKey};

/// Inside Ironforge, where `cities::indoors` answers without a navmesh.
const IRONFORGE_COMMONS: Vector3d = Vector3d {
    x: -4_900.0,
    y: -1_000.0,
    z: 501.0,
};

/// Wait up to two seconds for `f` on the map to answer `Some`.
async fn until<T: Send + 'static>(
    map: &MapHandle,
    what: &str,
    f: impl Fn(&mut crate::game::map::Map) -> Option<T> + Send + Sync + Clone + 'static,
) -> T {
    let end = Instant::now() + Duration::from_secs(2);
    loop {
        let g = f.clone();
        if let Some(Some(t)) = map.query(move |m| g(m)).await {
            return t;
        }
        assert!(Instant::now() < end, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn zone_of(m: &crate::game::map::Map, g: Guid) -> u32 {
    m.players.get(&g).map_or(0, |p| p.zone)
}

/// mangos's `HandleZoneUpdateOpcode` only asks for an update, which reads
/// the zone from where the player stands: a client in Ironforge naming
/// Westfall is still in Ironforge (its graveyard, rest and channels).
#[tokio::test]
async fn a_client_does_not_pick_its_own_zone_where_the_server_knows_it() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let before = map.query(move |m| zone_of(m, g)).await.unwrap();
    map.call(move |m| {
        if let Some(p) = m.players.get_mut(&g) {
            p.movement.position = IRONFORGE_COMMONS;
        }
    });
    a.send(CMSG_ZONEUPDATE {
        area: Area::Westfall0,
    })
    .await;
    let zone = until(&map, "the zone update", move |m| {
        let z = zone_of(m, g);
        (z != before).then_some(z)
    })
    .await;
    assert_eq!(zone, crate::game::cities::IRONFORGE, "the server's zone");
}
