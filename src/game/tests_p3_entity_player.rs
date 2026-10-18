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
            rest::restore(p, now, &Default::default());
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

/// Thistle Tea's "marks player in combat when a mob attack lands",
/// "cancels an in-flight cast when the character is dead", "ignores an
/// attack already in flight after death", "after spirit release" and "a
/// harmful spell outcome already in flight after spirit release": a
/// creature's blow puts a player in the fight, where their health doesn't
/// come back; death stops their cast and says so; and nothing hurts the
/// dead, nor a ghost, a blow or a missile on its way.
#[tokio::test]
async fn the_dead_and_their_ghosts_take_no_harm_and_cast_no_more() {
    use crate::game::tests_p3_combat::{BRUTE, spawn};
    let mut d = world();
    let mut bolt = spell(
        98_010,
        &[SpellEffect {
            effect: 2,
            base_points: 9,
            base_dice: 1,
            die_sides: 1,
            target_a: 6,
            ..Default::default()
        }],
    );
    bolt.speed = 20.0;
    d.spells.spells.insert(98_010, bolt);
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(ga).unwrap().map();
    let health = move |m: &Map| m.players[&ga].fields.get_u32(UNIT_FIELD_HEALTH);
    // Struck, hurt: in the fight, and no health back.
    let (brute, fighting, hurt, after) = on(&map, move |m| {
        let brute = spawn(m, ga, BRUTE, 20, 2.0);
        let max = m.players[&ga].fields.get_u32(UNIT_FIELD_MAXHEALTH);
        m.players
            .get_mut(&ga)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_HEALTH, max / 2);
        crate::game::map::combat::engage(m, brute, ga);
        crate::game::map::combat::deal_damage(m, brute, ga, 5);
        let fighting = m.players[&ga].fields.has_flag(
            UNIT_FIELD_FLAGS,
            crate::game::map::player::UNIT_FLAG_IN_COMBAT,
        );
        let hurt = health(m);
        let now = m.now;
        m.players.get_mut(&ga).unwrap().regen_at = now;
        crate::game::map::stats::regen(m);
        (brute, fighting, hurt, health(m))
    })
    .await;
    assert!(fighting, "in the fight");
    assert_eq!(after, hurt, "no health back while fighting");
    // Casting, she dies: the cast stops and she hears it.
    on(&map, move |m| {
        spells::learn(m, ga, SLOW);
        spells::cast(m, ga, SLOW, Targets::default(), None);
    })
    .await;
    a.clear();
    let casting = on(&map, move |m| {
        crate::game::map::death::kill(m, ga);
        m.caster(ga).is_some_and(|c| c.casting.is_some())
    })
    .await;
    assert!(!casting, "the cast is over");
    let failure = a
        .wait_frame("SMSG_SPELL_FAILURE", SMSG_SPELL_FAILURE, |_| true)
        .await;
    assert!(failure.windows(4).any(|w| w == SLOW.to_le_bytes()));
    // Dead: a blow hurts nothing.
    let dead = on(&map, move |m| {
        crate::game::map::combat::deal_damage(m, brute, ga, 10);
        health(m)
    })
    .await;
    assert_eq!(dead, 0, "the dead are hurt no more");
    // A ghost: neither a blow nor a missile on its way.
    a.send(CMSG_REPOP_REQUEST {}).await;
    crate::game::tests_p3_entity_objects::until(&map, "a ghost", move |m| {
        m.players[&ga].death != crate::game::map::death::Death::Alive && health(m) > 0
    })
    .await;
    let ghost = on(&map, move |m| {
        let before = health(m);
        crate::game::map::combat::deal_damage(m, brute, ga, 10);
        m.spells.travel = Some(true);
        crate::game::map::spells::cast_triggered(m, brute, 98_010, Some(ga));
        let flying = m.spells.in_flight.len();
        m.now += Duration::from_secs(3);
        spells::update(m);
        m.spells.travel = None;
        (before, flying, health(m))
    })
    .await;
    assert_eq!(ghost.0, ghost.2, "the ghost unhurt");
    assert!(ghost.1 <= 1, "a missile flew, or none was let fly");
}

const BOAT: u32 = 9_998;

/// A boat that sails 100 yards from the start, crosses to Kalimdor and
/// back, every 1.6 seconds.
fn boat_world() -> crate::game::data::GameData {
    use crate::game::data::npc::TaxiPathNode;
    use crate::game::data::{GameObjectTemplate, TransportRoute};
    let mut d = world();
    let mut data = [0u32; 24];
    data[0] = 78;
    data[1] = 1000;
    data[2] = 1000;
    d.gameobject_templates.insert(
        BOAT,
        GameObjectTemplate {
            entry: BOAT,
            kind: 15,
            display_id: 3015,
            name: "The Test Boat".into(),
            size: 1.0,
            data,
            ..Default::default()
        },
    );
    d.transports.push(TransportRoute {
        entry: BOAT,
        name: "Here and There".into(),
        period: 1600,
    });
    let node = |map, x: f32, y: f32| TaxiPathNode {
        map,
        x,
        y,
        z: START.2,
        flags: 0,
        delay: 0,
    };
    d.npc.taxi_path_nodes.insert(
        78,
        vec![
            node(0, START.0, START.1),
            node(0, START.0 + 30.0, START.1),
            node(0, START.0 + 60.0, START.1),
            node(0, START.0 + 100.0, START.1),
            node(1, 990.0, 1000.0),
            node(1, 1000.0, 1000.0),
            node(1, 1010.0, 1000.0),
            node(1, 1020.0, 1000.0),
        ],
    );
    d
}

/// Thistle Tea's "resurrects a dead passenger before a transport map
/// change": one lying dead on deck, not released, rises whole as the ship
/// takes them to another map (mangos's `Transport::TeleportTransport`).
#[tokio::test]
async fn a_dead_passenger_rises_as_the_ship_crosses() {
    let realm = harness::realm_with(boat_world());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let boat = crate::game::guid::make(crate::game::guid::HighGuid::MoTransport, 0, BOAT);
    assert!(
        a.sees_within(boat, Duration::from_secs(3)).await,
        "the boat"
    );
    a.send(MSG_MOVE_HEARTBEAT_Client {
        info: MovementInfo {
            flags: MovementInfo_MovementFlags::empty().set_on_transport(
                MovementInfo_MovementFlags_OnTransport {
                    transport: TransportInfo {
                        guid: boat,
                        position: Vector3d {
                            x: 1.0,
                            y: 0.0,
                            z: 0.0,
                        },
                        orientation: 0.0,
                        timestamp: 0,
                    },
                },
            ),
            timestamp: 0,
            position: Vector3d {
                x: START.0 + 1.0,
                y: START.1,
                z: START.2,
            },
            orientation: 0.0,
            fall_time: 0.0,
        },
    })
    .await;
    let map = realm.registry.online(ga).unwrap().map();
    crate::game::tests_p3_entity_objects::until(&map, "Alice aboard", move |m| {
        m.players
            .get(&ga)
            .is_some_and(|p| p.movement.flags.get_on_transport().is_some())
    })
    .await;
    on(&map, move |m| crate::game::map::death::kill(m, ga)).await;
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    a.sees(ga).await;
    let kalimdor = realm.registry.online(ga).unwrap().map();
    let (alive, whole) = on(&kalimdor, move |m| {
        let p = &m.players[&ga];
        (
            p.death == crate::game::map::death::Death::Alive,
            p.fields.get_u32(UNIT_FIELD_HEALTH) == p.fields.get_u32(UNIT_FIELD_MAXHEALTH),
        )
    })
    .await;
    assert!(alive, "she rose with the crossing");
    assert!(whole, "whole");
}

/// Thistle Tea's "moves global ship visibility between maps" and
/// "validates boarding, derives world coordinates, and tracks departure":
/// everyone on a ship's map knows it, and as it crosses those it leaves
/// forget it and those it comes to are told of it; a passenger who stepped
/// off before isn't carried.
#[tokio::test]
async fn a_ship_is_known_on_its_map_and_carries_only_who_is_aboard() {
    let realm = harness::realm_with(boat_world());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let boat = crate::game::guid::make(crate::game::guid::HighGuid::MoTransport, 0, BOAT);
    b.say(".go xyz 1629 -4373 31 1").await;
    b.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    b.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    b.sees(gb).await;
    // Alice steps aboard, then off again.
    let deck = |on: bool| MSG_MOVE_HEARTBEAT_Client {
        info: MovementInfo {
            flags: if on {
                MovementInfo_MovementFlags::empty().set_on_transport(
                    MovementInfo_MovementFlags_OnTransport {
                        transport: TransportInfo {
                            guid: boat,
                            position: Vector3d {
                                x: 1.0,
                                y: 0.0,
                                z: 0.0,
                            },
                            orientation: 0.0,
                            timestamp: 0,
                        },
                    },
                )
            } else {
                MovementInfo_MovementFlags::empty()
            },
            timestamp: 0,
            position: Vector3d {
                x: START.0 + 1.0,
                y: START.1,
                z: START.2,
            },
            orientation: 0.0,
            fall_time: 0.0,
        },
    };
    a.sees(boat).await;
    a.send(deck(true)).await;
    a.send(deck(false)).await;
    a.send(CMSG_QUERY_TIME {}).await;
    a.wait_for("SMSG_QUERY_TIME_RESPONSE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_QUERY_TIME_RESPONSE(_)).then_some(())
    })
    .await;
    // The boat goes to Kalimdor: Alice forgets it, Bob is told of it.
    b.sees(boat).await;
    a.forgets(boat).await;
    assert!(
        a.gets(NONE_WITHIN, |m| {
            matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
        })
        .await
        .is_none(),
        "off the deck, she stays behind"
    );
    assert_eq!(realm.registry.online(ga).unwrap().map().key.map, 0);
}
