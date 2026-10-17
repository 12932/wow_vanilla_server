//! Phase 3's player tests, ported from Thistle Tea's
//! `test/game/world/entity/player_test.exs` (docs/parity/matrix.csv): rest
//! earned logged out, a logout's order, and what a teleport ends.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::AreaInfo;
use crate::game::data::spells::SpellEffect;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::Map;
use crate::game::map::rest::{self, AREA_FLAG_CAPITAL, RestType};
use crate::game::map::spells::{self, Targets};
use crate::game::tests_p3_combat::START;
use crate::game::tests_p3_entity::{NONE_WITHIN, SMSG_CAST_RESULT, SMSG_SPELL_FAILURE, on, spell};

const STORMWIND: u32 = 1519;
const ELWYNN: u32 = 12;
/// Three seconds to cast.
const SLOW: u32 = 98_000;

fn world() -> crate::game::data::GameData {
    let mut d = crate::game::tests_p3_combat::data();
    d.areas.insert(
        STORMWIND,
        AreaInfo {
            id: STORMWIND,
            map: 0,
            flags: AREA_FLAG_CAPITAL,
            team: 2,
            name: "Stormwind City".into(),
            ..Default::default()
        },
    );
    d.areas.insert(
        ELWYNN,
        AreaInfo {
            id: ELWYNN,
            map: 0,
            name: "Elwynn Forest".into(),
            ..Default::default()
        },
    );
    d.spells.cast_times.insert(14, 3000);
    let mut slow = spell(
        SLOW,
        &[SpellEffect {
            effect: 3,
            target_a: 1,
            ..Default::default()
        }],
    );
    slow.casting_time_index = 14;
    d.spells.spells.insert(SLOW, slow);
    d
}

fn wall_now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Thistle Tea's "settles rest on disconnect and consumes offline time
/// before login publication" and "connection loss begins wilderness rest
/// at actual world departure": eight hours logged out at rest earn a
/// fortieth of a level, and anywhere else a quarter of that, in the pool
/// and its field; a dropped connection is a logout then, not resting.
#[tokio::test]
async fn rest_logged_out_is_earned_by_where_one_left() {
    let realm = harness::realm_with(world());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(ga).unwrap().map();
    let out = on(&map, move |m| {
        let now = m.now;
        let p = m.players.get_mut(&ga).unwrap();
        let next = p.fields.get_u32(PLAYER_NEXT_LEVEL_XP) as f32;
        let mut earned = Vec::new();
        for kind in [Some(RestType::City), None] {
            p.row.state.rest_bonus = 0.0;
            p.row.state.rest_type = kind;
            p.row.logout_time = wall_now() - 8 * 3600;
            rest::restore(p, now);
            earned.push((
                p.row.state.rest_bonus,
                p.fields.get_u32(PLAYER_REST_STATE_EXPERIENCE),
            ));
        }
        p.row.state.rest_type = None;
        (next, earned)
    })
    .await;
    let (next, earned) = out;
    assert!(
        (earned[0].0 - next / 40.0).abs() < 1.0,
        "{earned:?} of {next}"
    );
    assert_eq!(earned[0].1, (next / 40.0) as u32, "and the field with it");
    assert!(
        (earned[1].0 - next / 160.0).abs() < 1.0,
        "{earned:?} of {next}"
    );
    // The connection drops: it was a logout then, away from rest.
    let before = wall_now();
    a.disconnect();
    let realm2 = realm.clone();
    let mut b = Client::connect(&realm2, "A");
    b.send(CMSG_CHAR_ENUM {}).await;
    b.wait_for("SMSG_CHAR_ENUM", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_CHAR_ENUM(_)).then_some(())
    })
    .await;
    b.login(ga).await;
    let map = realm.registry.online(ga).unwrap().map();
    let (left, resting) = on(&map, move |m| {
        let p = &m.players[&ga];
        (p.row.logout_time, p.row.state.rest_type)
    })
    .await;
    assert!(left >= before, "a logout at the drop: {left} from {before}");
    assert_eq!(resting, None, "not resting");
}

/// Thistle Tea's "stops the owner before the connection completes
/// logout": an inn's logout takes the player out of the world before its
/// client hears the logout is complete, and those who saw them are told.
#[tokio::test]
async fn an_inn_logout_leaves_the_world_before_it_completes() {
    let realm = harness::realm_with(world());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    b.sees(ga).await;
    let map = realm.registry.online(ga).unwrap().map();
    on(&map, move |m| {
        let now = m.now;
        rest::start(m.players.get_mut(&ga).unwrap(), RestType::Tavern(71), now);
    })
    .await;
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    let gone = on(&map, move |m| !m.players.contains_key(&ga)).await;
    assert!(gone, "out of the world by then");
    b.forgets(ga).await;
}

fn falling(z: f32, far: bool) -> MSG_MOVE_HEARTBEAT_Client {
    MSG_MOVE_HEARTBEAT_Client {
        info: MovementInfo {
            flags: if far {
                MovementInfo_MovementFlags::empty().set_fallingfar()
            } else {
                MovementInfo_MovementFlags::empty()
            },
            timestamp: 0,
            position: Vector3d {
                x: START.0,
                y: START.1,
                z,
            },
            orientation: 0.0,
            fall_time: f32::from_bits(5_000),
        },
    }
}

/// Thistle Tea's "near and cross-map teleports discard an unfinished
/// fall", "near and cross-map teleports notify the client when
/// interrupting a cast", "ordinary teleports detach from a transport" and
/// "initializes instance ownership after a cross-map transfer": a
/// teleport within the map or to another ends a fall under way (the
/// landing hurts no one), stops a cast and tells its caster why, takes the
/// player off a transport, and a worldport is followed by the instance
/// ownership.
#[tokio::test]
async fn a_teleport_ends_a_fall_a_cast_and_a_ride() {
    let realm = harness::realm_with(world());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(ga).unwrap().map();
    let health = move |m: &Map| m.players[&ga].fields.get_u32(UNIT_FIELD_HEALTH);
    // Falling from 200 yards, and casting: a teleport in the map.
    a.send(falling(START.2 + 200.0, true)).await;
    on(&map, move |m| {
        spells::learn(m, ga, SLOW);
        spells::cast(m, ga, SLOW, Targets::default(), None);
    })
    .await;
    a.clear();
    let before = on(&map, move |m| health(m)).await;
    a.say(".go -9000 -100 90").await;
    a.wait_for("MSG_MOVE_TELEPORT_ACK", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    let result = a
        .wait_frame("SMSG_CAST_RESULT", SMSG_CAST_RESULT, |f| {
            f[..4] == SLOW.to_le_bytes()
        })
        .await;
    assert_eq!(&result[4..], &[2, 0x23], "interrupted");
    a.wait_frame("SMSG_SPELL_FAILURE", SMSG_SPELL_FAILURE, |_| true)
        .await;
    a.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: ga,
        movement_counter: 0,
        time: Default::default(),
    })
    .await;
    a.send(MSG_MOVE_HEARTBEAT_Client {
        info: MovementInfo {
            position: Vector3d {
                x: -9000.0,
                y: -100.0,
                z: 90.0,
            },
            ..falling(0.0, false).info
        },
    })
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let (after, casting) = on(&map, move |m| {
        (health(m), m.caster(ga).is_some_and(|c| c.casting.is_some()))
    })
    .await;
    assert_eq!(after, before, "the fall was forgotten");
    assert!(!casting, "the cast is over");
    // On a boat's deck: a teleport takes her off it.
    let deck = on(&map, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        p.movement.flags = MovementInfo_MovementFlags::empty().set_on_transport(
            MovementInfo_MovementFlags_OnTransport {
                transport: TransportInfo {
                    guid: Guid::new(0x1FC0_0000_0000_0001),
                    position: Vector3d {
                        x: 1.0,
                        y: 2.0,
                        z: 3.0,
                    },
                    orientation: 0.25,
                    timestamp: 0,
                },
            },
        );
        crate::game::map::movement::teleport_within(
            m,
            ga,
            Vector3d {
                x: START.0,
                y: START.1,
                z: START.2,
            },
            0.5,
        );
        m.players[&ga].movement.flags.get_on_transport().is_some()
    })
    .await;
    assert!(!deck, "off the deck");
    // Falling again, casting again: to Kalimdor.
    a.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: ga,
        movement_counter: 0,
        time: Default::default(),
    })
    .await;
    a.send(falling(START.2 + 200.0, true)).await;
    on(&map, move |m| {
        if let Some(c) = m.caster_mut(ga) {
            c.gcd_until = None;
        }
        spells::cast(m, ga, SLOW, Targets::default(), None);
    })
    .await;
    a.clear();
    a.say(".go xyz 1629 -4373 31 1").await;
    let result = a
        .wait_frame("SMSG_CAST_RESULT", SMSG_CAST_RESULT, |f| {
            f[..4] == SLOW.to_le_bytes()
        })
        .await;
    assert_eq!(&result[4..], &[2, 0x23], "interrupted again");
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    let saved = a
        .wait_for("SMSG_UPDATE_INSTANCE_OWNERSHIP", |m| match m {
            ServerOpcodeMessage::SMSG_UPDATE_INSTANCE_OWNERSHIP(o) => {
                Some(o.player_is_saved_to_a_raid)
            }
            _ => None,
        })
        .await;
    assert!(!saved, "saved to no raid");
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    a.sees(ga).await;
    let kalimdor = realm.registry.online(ga).unwrap().map();
    let before = on(&kalimdor, move |m| health(m)).await;
    a.send(MSG_MOVE_HEARTBEAT_Client {
        info: MovementInfo {
            position: Vector3d {
                x: 1629.0,
                y: -4373.0,
                z: 31.0,
            },
            ..falling(0.0, false).info
        },
    })
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        on(&kalimdor, move |m| health(m)).await,
        before,
        "no fall in Kalimdor"
    );
    assert!(
        a.gets(NONE_WITHIN, |m| {
            matches!(m, ServerOpcodeMessage::SMSG_ENVIRONMENTAL_DAMAGE_LOG(_)).then_some(())
        })
        .await
        .is_none()
    );
}

/// Thistle Tea's "clears city rest when teleporting to a map without
/// loaded navigation data": a capital's rest ends once the player is in a
/// zone that isn't one, as their client tells the zone it put them in.
#[tokio::test]
async fn a_capitals_rest_ends_outside_it() {
    let realm = harness::realm_with(world());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(ga).unwrap().map();
    a.send(CMSG_ZONEUPDATE {
        area: Area::StormwindCity,
    })
    .await;
    crate::game::tests_p3_entity_objects::until(&map, "resting in Stormwind", move |m| {
        m.players[&ga].row.state.rest_type == Some(RestType::City)
    })
    .await;
    let flagged = on(&map, move |m| {
        m.players[&ga]
            .fields
            .has_flag(PLAYER_FLAGS, crate::game::map::player::PLAYER_FLAGS_RESTING)
    })
    .await;
    assert!(flagged, "resting, and shown so");
    a.send(CMSG_ZONEUPDATE {
        area: Area::ElwynnForest,
    })
    .await;
    crate::game::tests_p3_entity_objects::until(&map, "the rest over in Elwynn", move |m| {
        m.players[&ga].row.state.rest_type.is_none()
            && !m.players[&ga]
                .fields
                .has_flag(PLAYER_FLAGS, crate::game::map::player::PLAYER_FLAGS_RESTING)
    })
    .await;
}
