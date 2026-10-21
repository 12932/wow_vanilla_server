//! Round 12's stage h, death as mangos-classic's `Player` has it: the
//! spirit released for the dead after six minutes outside a copy, the
//! client's countdown flag (`KillPlayer`, `Player::Update`); the spirit
//! healer's risen put at their corpse's graveyard (`SendSpiritResurrect`);
//! and a fall below the world (`HandleMovementOpcodes`, z under -500).

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::death::{self, Death, PLAYER_FIELD_BYTE_RELEASE_TIMER};
use crate::game::map::{Map, MapHandle, MapKey};

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const HEALER: u32 = 900_260;
const HEALER_COUNTER: u32 = 260;
const DEADMINES: u32 = 36;
/// A graveyard by the start, its spirit healer beside it, and one 300
/// yards east.
const NEAR_GRAVE: (f32, f32, f32) = (START.0 - 3.0, START.1, START.2);
const FAR_GRAVE: (f32, f32, f32) = (START.0 + 300.0, START.1, START.2);

fn data() -> GameData {
    let mut d = GameData::empty();
    d.creature_templates.insert(
        HEALER,
        CreatureTemplate {
            entry: HEALER,
            name: "Spirit Healer".into(),
            display_ids: [903, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            level_min: 30,
            level_max: 30,
            faction: 35,
            npc_flags: 0x20,
            unit_class: 1,
            health_multiplier: 1.0,
            health_max: 500,
            ..Default::default()
        },
    );
    d.creature_spawns.insert(
        0,
        vec![CreatureSpawn {
            guid: HEALER_COUNTER,
            entries: vec![HEALER],
            map: 0,
            x: START.0 - 2.0,
            y: START.1,
            z: START.2,
            respawn_min: 600,
            respawn_max: 600,
            ..Default::default()
        }],
    );
    for (id, (x, y, z)) in [(1, NEAR_GRAVE), (2, FAR_GRAVE)] {
        d.graveyards.push(Graveyard {
            id,
            map: 0,
            x,
            y,
            z,
            orientation: 0.0,
            zone: 0,
            faction: 0,
        });
    }
    d.maps.insert(
        DEADMINES,
        MapInfo {
            id: DEADMINES,
            internal_name: "DeadminesInstance".into(),
            name: "Deadmines".into(),
            instance_type: 1,
            player_limit: 0,
        },
    );
    d
}

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map answers")
}

fn put(m: &mut Map, g: Guid, (x, y, z): (f32, f32, f32)) {
    m.players.get_mut(&g).unwrap().movement.position = Vector3d { x, y, z };
}

/// `c` teleported within its map: the order acknowledged.
async fn teleported(c: &mut Client, g: Guid) {
    c.wait_for("a teleport", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    c.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: g,
        movement_counter: c.teleport_order(),
        time: Default::default(),
    })
    .await;
}

fn release_timer(m: &Map, g: Guid) -> bool {
    m.players[&g].fields.get_byte(PLAYER_FIELD_BYTES, 0) & PLAYER_FIELD_BYTE_RELEASE_TIMER != 0
}

/// Wait up to a second for `g`'s death to be `want`'s kind.
async fn until(map: &MapHandle, g: Guid, ghost: bool) -> Death {
    let mut d = Death::Alive;
    for _ in 0..100 {
        d = on(map, move |m| m.players[&g].death).await;
        if matches!(d, Death::Ghost { .. }) == ghost {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    d
}

/// mangos's `KillPlayer` and `Player::Update`: out of a copy the dead are
/// released to their graveyard after six minutes, the client shown the
/// countdown (`PLAYER_FIELD_BYTE_RELEASE_TIMER`); in a dungeon they lie
/// until they release. wvs never released them, nor set the flag.
#[tokio::test]
async fn the_dead_are_released_after_six_minutes_out_of_a_copy() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(g).unwrap().map();
    let flag = on(&map, move |m| {
        put(m, g, FAR_GRAVE);
        death::kill(m, g);
        release_timer(m, g)
    })
    .await;
    assert!(flag, "the countdown shown");
    // Five minutes in: still lying there.
    on(&map, move |m| {
        m.players.get_mut(&g).unwrap().death = Death::Dead {
            since: m.now - Duration::from_secs(300),
        };
    })
    .await;
    harness::an_update(&map).await;
    assert!(matches!(
        on(&map, move |m| m.players[&g].death).await,
        Death::Dead { .. }
    ));
    on(&map, move |m| {
        m.players.get_mut(&g).unwrap().death = Death::Dead {
            since: m.now - death::AUTO_RELEASE,
        };
    })
    .await;
    teleported(&mut a, g).await;
    assert!(matches!(until(&map, g, true).await, Death::Ghost { .. }));
    // Risen, the flag goes.
    let after = on(&map, move |m| {
        death::resurrect(m, g, 1.0);
        release_timer(m, g)
    })
    .await;
    assert!(!after);
}

#[tokio::test]
async fn in_a_dungeon_the_dead_lie_until_they_release() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let from = realm.registry.online(g).unwrap().map();
    from.call(move |m| {
        m.teleport(
            g,
            MapKey::world(DEADMINES),
            Vector3d {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            0.0,
        )
    });
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    a.sees(g).await;
    let map = realm.registry.online(g).unwrap().map();
    let flag = on(&map, move |m| {
        death::kill(m, g);
        m.players.get_mut(&g).unwrap().death = Death::Dead {
            since: m.now - Duration::from_secs(3600),
        };
        release_timer(m, g)
    })
    .await;
    assert!(!flag, "no countdown in a copy");
    harness::an_update(&map).await;
    harness::an_update(&map).await;
    assert!(matches!(
        on(&map, move |m| m.players[&g].death).await,
        Death::Dead { .. }
    ));
}

fn near(a: Vector3d, (x, y, z): (f32, f32, f32)) -> bool {
    (a.x - x).abs() < 1.0 && (a.y - y).abs() < 1.0 && (a.z - z).abs() < 1.0
}

/// mangos's `SendSpiritResurrect`: a ghost revived by a spirit healer at a
/// graveyard other than the one nearest its corpse is put at that one, so a
/// ghost's run to a far healer is no free journey. wvs left them where
/// they stood.
#[tokio::test]
async fn a_spirit_healer_puts_the_risen_at_their_corpses_graveyard() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(g).unwrap().map();
    // Dead by the far graveyard, released there.
    on(&map, move |m| {
        put(m, g, (FAR_GRAVE.0 - 10.0, FAR_GRAVE.1, FAR_GRAVE.2));
        death::kill(m, g);
    })
    .await;
    a.send(CMSG_REPOP_REQUEST {}).await;
    teleported(&mut a, g).await;
    let at = on(&map, move |m| m.players[&g].position()).await;
    assert!(near(at, FAR_GRAVE), "released by the corpse: {at:?}");
    // The ghost runs to the start's healer, and is revived there.
    on(&map, move |m| {
        put(m, g, NEAR_GRAVE);
        m.players.get_mut(&g).unwrap().death = Death::Ghost {
            since: m.now - Duration::from_secs(60),
        };
    })
    .await;
    a.clear();
    let healer = crate::game::guid::unit(HEALER, HEALER_COUNTER);
    a.send(CMSG_SPIRIT_HEALER_ACTIVATE { guid: healer }).await;
    teleported(&mut a, g).await;
    let (death, at) = on(&map, move |m| {
        (m.players[&g].death, m.players[&g].position())
    })
    .await;
    assert_eq!(death, Death::Alive);
    assert!(near(at, FAR_GRAVE), "at the corpse's graveyard: {at:?}");
}

/// mangos's `HandleMovementOpcodes`: below z -500 the living take their
/// whole health as a fall and are released to their graveyard (a ghost is
/// put there). wvs let them fall for ever.
#[tokio::test]
async fn a_fall_below_the_world_kills_and_releases() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(g).unwrap().map();
    on(&map, move |m| put(m, g, (START.0, START.1, -495.0))).await;
    a.clear();
    a.heartbeat(START.0, START.1, -505.0).await;
    let log = a
        .wait_for("the void's damage", |m| match m {
            ServerOpcodeMessage::SMSG_ENVIRONMENTAL_DAMAGE_LOG(l) if l.guid == g => {
                Some(l.damage_type)
            }
            _ => None,
        })
        .await;
    assert_eq!(log, EnvironmentalDamageType::Fall);
    teleported(&mut a, g).await;
    let (death, at) = on(&map, move |m| {
        (m.players[&g].death, m.players[&g].position())
    })
    .await;
    assert!(matches!(death, Death::Ghost { .. }), "{death:?}");
    assert!(near(at, NEAR_GRAVE), "at the graveyard: {at:?}");
    // A ghost falling through is put back at its graveyard.
    on(&map, move |m| put(m, g, (START.0, START.1, -495.0))).await;
    a.heartbeat(START.0, START.1, -505.0).await;
    teleported(&mut a, g).await;
    let at = on(&map, move |m| m.players[&g].position()).await;
    assert!(near(at, NEAR_GRAVE), "{at:?}");
}
