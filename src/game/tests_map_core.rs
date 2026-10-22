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

/// Call 13: a client sent to another map that never says it has loaded it
/// is disconnected after `[world] worldport_ack_timeout_secs` (60), its
/// player saved where it was sent and taken out of the world; at 59 s it
/// is still loading. One that has acknowledged is never kicked. (It was
/// left in the world for ever, able only to ask and log out.)
#[tokio::test]
async fn a_client_that_never_loads_its_new_map_is_kicked_after_a_minute() {
    use crate::game::map::login::kick_unloaded;
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
    // The map's clock skipped: 59 s on, still loading.
    let skip = move |secs: u64| {
        move |m: &mut crate::game::map::Map| {
            m.now += Duration::from_secs(secs);
            kick_unloaded(m);
        }
    };
    kalimdor.query(skip(59)).await.unwrap();
    harness::an_update(&kalimdor).await;
    assert!(realm.registry.online(g).is_some(), "still loading at 59 s");
    assert!(on(&kalimdor, move |m| m.players.contains_key(&g)).await);
    // 61 s on: kicked, and gone.
    kalimdor.query(skip(61)).await.unwrap();
    harness::offline(&realm, g).await;
    assert!(!on(&kalimdor, move |m| m.players.contains_key(&g)).await);
    // Saved where it was sent.
    let mut b = Client::connect(&realm, "A");
    b.login(g).await;
    let back = realm.registry.online(g).expect("online").map();
    assert_eq!(on(&back, move |m| m.key).await, MapKey::world(1));
    // Sent back and loaded in time, it is never kicked.
    let home = START_AT;
    back.call(move |m| m.teleport(g, MapKey::world(0), home, 0.0));
    b.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    until(&map, "Alice home", move |m| {
        m.players
            .get(&g)
            .filter(|p| p.worldport_sent.is_some())
            .map(|_| ())
    })
    .await;
    b.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    until(&map, "Alice loaded", move |m| {
        m.players
            .get(&g)
            .filter(|p| !p.awaiting_worldport)
            .map(|_| ())
    })
    .await;
    map.query(skip(600)).await.unwrap();
    harness::an_update(&map).await;
    assert!(realm.registry.online(g).is_some(), "loaded, never kicked");
}

/// Northshire, where the tests' players start.
const START_AT: Vector3d = Vector3d {
    x: -8_949.95,
    y: -132.493,
    z: 83.5312,
};

/// A query on `map`.
async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut crate::game::map::Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.unwrap()
}

/// mangos-classic's `UF_FLAG_PUBLIC` has a unit's aura state, the spell
/// that made it, its NPC flags and its emote state: another player sees a
/// dancer dance (Thistle Tea's "retains dancing for observers"), and a
/// target under a fifth of its health lights Execute.
#[tokio::test]
async fn another_player_sees_a_players_emote_and_aura_states() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let start = crate::game::tests_p3_combat::START;
    a.heartbeat(start.0, start.1, start.2).await;
    let (mut b, _) = crate::game::tests_p3_entity::bob(&realm, ga).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    const DANCE: u32 = 10;
    const HEALTHLESS_20: u32 = 1 << 1;
    map.call(move |m| {
        if let Some(p) = m.players.get_mut(&ga) {
            p.fields.set_u32(UNIT_NPC_EMOTESTATE, DANCE);
            p.fields.set_u32(UNIT_FIELD_AURASTATE, HEALTHLESS_20);
        }
    });
    let seen = b
        .wait_for("Alice's states", |m| {
            harness::values(m)
                .into_iter()
                .find_map(|(of, mask)| match mask {
                    UpdateMask::Player(p) if of == ga => {
                        Some((p.unit_npc_emotestate()?, p.unit_aurastate()?))
                    }
                    _ => None,
                })
        })
        .await;
    assert_eq!(seen, (DANCE as i32, HEALTHLESS_20 as i32));
}

/// A creature despawned in the same run as something marked it for a
/// visibility refresh (a script's `update_entry`, a summon gone in the
/// step it came) was taken out of the grid but left marked: the refresh
/// found it, took it whole, and introduced it again to everyone near.
#[tokio::test]
async fn a_creature_despawned_while_marked_is_not_introduced_again() {
    use crate::game::tests_p3_combat::{BRUTE, START, data, spawn};
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let c = map
        .query(move |m| spawn(m, g, BRUTE, 5, 5.0))
        .await
        .unwrap();
    a.sees(c).await;
    map.call(move |m| {
        m.visibility_dirty.insert(c);
        crate::game::map::ai::quiet_death_for(m, c, Duration::from_secs(300));
    });
    a.forgets(c).await;
    assert!(
        !a.sees_within(c, Duration::from_secs(1)).await,
        "introduced again"
    );
}
