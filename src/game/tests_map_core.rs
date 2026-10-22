//! The bug hunt's fixes in the map's core: what the client claims, what
//! the map shows and keeps, through the protocol.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;

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

/// mangos-classic handles an in-world opcode only for a player in the
/// world: one sent to another map, whose client holds its
/// `MSG_MOVE_WORLDPORT_ACK` back (no one sees them, no creature minds
/// them), does nothing there but ask and acknowledge.
#[tokio::test]
async fn a_client_still_loading_a_map_acts_on_nothing_there() {
    use crate::game::map::player::PLAYER_FLAGS_HIDE_HELM;
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let to = Vector3d {
        x: 1_629.0,
        y: -4_373.0,
        z: 31.0,
    };
    map.call(move |m| m.teleport(g, MapKey::world(1), to, 0.0));
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    let kalimdor = realm.maps.get(&realm, MapKey::world(1));
    until(&kalimdor, "Alice on Kalimdor", move |m| {
        m.players.contains_key(&g).then_some(())
    })
    .await;
    let helm = move |m: &mut crate::game::map::Map| {
        m.players
            .get(&g)
            .map(|p| p.fields.has_flag(PLAYER_FLAGS, PLAYER_FLAGS_HIDE_HELM))
    };
    let before = kalimdor.query(helm).await.flatten();
    a.send(CMSG_TOGGLE_HELM {}).await;
    a.send(CMSG_QUERY_TIME {}).await;
    a.wait_for("SMSG_QUERY_TIME_RESPONSE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_QUERY_TIME_RESPONSE(_)).then_some(())
    })
    .await;
    assert_eq!(kalimdor.query(helm).await.flatten(), before, "dropped");
    // Arrived, it is theirs again.
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    a.send(CMSG_TOGGLE_HELM {}).await;
    let after = until(&kalimdor, "the helm toggled", move |m| {
        helm(m).filter(|h| Some(*h) != before)
    })
    .await;
    assert_eq!(Some(!after), before);
}
