//! Round 7, against Thistle Tea's
//! `test/game/core/entity/component/movement_block_test.exs`
//! (docs/parity/matrix.csv): a player's swim, jump and spline elevation as
//! their client sends them and others are told, and what a create block
//! carries of a unit's movement.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::fields::{Fields, TypeId, UNIT_FIELD_DISPLAYID, UNIT_FIELD_HEALTH, Visibility};
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::tests_p3_entity::on;
use crate::game::update::{
    Aboard, Batch, Create, MOVEFLAG_ON_TRANSPORT, MOVEFLAG_SWIMMING, Placement, Speeds,
    create_block,
};

/// Thistle Tea's `from_binary/2` "parses with swimming flag", "parses with
/// jumping flag" and "parses with spline elevation flag": a swimmer's
/// pitch, a jump's angle and speeds, and a spline elevation, as Alice's
/// client sends them, are what Bob is told, and what the server keeps.
#[tokio::test]
async fn a_swim_a_jump_and_a_spline_elevation_reach_others_as_sent() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    b.sees(ga).await;
    let map = realm.registry.online(ga).unwrap().map();
    let at = on(&map, move |m| m.players[&ga].position()).await;
    let info = |flags, dx: f32| MovementInfo {
        flags,
        timestamp: 10,
        position: Vector3d { x: at.x + dx, ..at },
        orientation: 1.0,
        fall_time: 0,
    };
    // A swim, its pitch a quarter.
    let swim = MovementInfo_MovementFlags::empty()
        .set_swimming(MovementInfo_MovementFlags_Swimming { pitch: 0.25 });
    a.send(MSG_MOVE_START_SWIM_Client {
        info: info(swim, 0.5),
    })
    .await;
    let told = b
        .wait_for("Alice's swim", |m| match m {
            ServerOpcodeMessage::MSG_MOVE_START_SWIM(s) if s.guid == ga => Some(s.info),
            _ => None,
        })
        .await;
    assert_eq!(told.flags.get_swimming().map(|s| s.pitch), Some(0.25));
    let kept = on(&map, move |m| {
        m.players[&ga]
            .movement
            .flags
            .get_swimming()
            .map(|s| s.pitch)
    })
    .await;
    assert_eq!(kept, Some(0.25), "kept as sent");
    // Out of the water, a jump.
    let jump =
        MovementInfo_MovementFlags::empty().set_jumping(MovementInfo_MovementFlags_Jumping {
            z_speed: -7.9,
            cos_angle: 0.6,
            sin_angle: 0.8,
            xy_speed: 7.0,
        });
    a.send(MSG_MOVE_JUMP_Client {
        info: info(jump, 1.0),
    })
    .await;
    let told = b
        .wait_for("Alice's jump", |m| match m {
            ServerOpcodeMessage::MSG_MOVE_JUMP(s) if s.guid == ga => Some(s.info),
            _ => None,
        })
        .await;
    let j = told.flags.get_jumping().expect("a jump");
    assert_eq!(
        (j.z_speed, j.cos_angle, j.sin_angle, j.xy_speed),
        (-7.9, 0.6, 0.8, 7.0)
    );
    // Landed, a spline elevation.
    let raised = MovementInfo_MovementFlags::empty().set_spline_elevation(
        MovementInfo_MovementFlags_SplineElevation {
            spline_elevation: 1.5,
        },
    );
    a.send(MSG_MOVE_HEARTBEAT_Client {
        info: info(raised, 1.5),
    })
    .await;
    let told = b
        .wait_for("Alice's heartbeat", |m| match m {
            ServerOpcodeMessage::MSG_MOVE_HEARTBEAT(s) if s.guid == ga => Some(s.info),
            _ => None,
        })
        .await;
    assert_eq!(
        told.flags
            .get_spline_elevation()
            .map(|e| e.spline_elevation),
        Some(1.5)
    );
}

/// A unit's fields, enough for a create to parse.
fn unit(g: Guid) -> Fields {
    let mut f = Fields::new(TypeId::Unit, g, guid::entry(g));
    f.set_u32(UNIT_FIELD_HEALTH, 40);
    f.set_u32(UNIT_FIELD_DISPLAYID, 903);
    f
}

/// A living unit's create of `flags`, aboard `transport` and swinging at
/// `melee`, and its bytes from the movement flags on.
fn living(
    g: Guid,
    flags: u32,
    transport: Option<Aboard>,
    melee: Option<Guid>,
) -> (Vec<u8>, Vec<u8>) {
    let fields = unit(g);
    let block = create_block(
        &Create {
            guid: g,
            type_id: TypeId::Unit,
            placement: Placement::Living {
                flags,
                timestamp: 7,
                position: Vector3d {
                    x: 1.0,
                    y: 2.0,
                    z: 3.0,
                },
                orientation: 0.5,
                pitch: 0.25,
                fall_time: 9,
                jump: None,
                spline_elevation: None,
                speeds: Speeds::default(),
                spline: None,
                transport,
            },
            fields: &fields,
            is_self: false,
            melee_target: melee,
        },
        Visibility::Public,
    );
    // The update type, the packed GUID, the object type and the update
    // flags come before the movement flags.
    let mut packed = Vec::new();
    crate::game::update::write_packed_guid(&mut packed, g);
    let at = 1 + packed.len() + 2;
    let tail = block[at..].to_vec();
    (block, tail)
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn f32_at(b: &[u8], at: usize) -> f32 {
    f32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

/// Whether a create block parses, as the client would read it.
fn parses(block: &[u8]) -> bool {
    let mut batch = Batch::new();
    batch.push(block);
    batch
        .finish()
        .iter()
        .all(|p| ServerOpcodeMessage::read_unencrypted(&p[..]).is_ok())
}

/// Thistle Tea's `to_binary/1` "includes swimming data when flag set",
/// "serializes transport guid and local position", "serializes melee
/// target as a packed guid" and "strips stale spline movement flags
/// without active spline data": a swimmer's create carries its pitch after
/// its facing; a passenger's the transport's whole GUID and its place and
/// facing on deck before it; a unit swinging, its target's packed GUID
/// after the block; and a spline flag with no spline to send is dropped.
#[test]
fn a_create_carries_a_swimmers_pitch_a_passengers_deck_and_a_swingers_target() {
    let g = guid::unit(299, 7);
    // Swimming: flags, time, place, facing, then the pitch, then the fall.
    let (block, b) = living(g, MOVEFLAG_SWIMMING, None, None);
    assert!(parses(&block));
    assert_eq!(u32_at(&b, 0), MOVEFLAG_SWIMMING);
    assert_eq!(u32_at(&b, 4), 7, "its time");
    assert_eq!(f32_at(&b, 20), 0.5, "its facing");
    assert_eq!(f32_at(&b, 24), 0.25, "its pitch");
    assert_eq!(u32_at(&b, 28), 9, "its fall");
    // Not swimming: no pitch.
    let (_, b) = living(g, 0, None, None);
    assert_eq!(u32_at(&b, 24), 9, "the fall straight after the facing");
    // Aboard a boat: the flag, the boat's whole GUID, the deck's place and
    // facing.
    let boat = guid::game_object(20_808, 1);
    let deck = Aboard {
        transport: boat,
        position: Vector3d {
            x: 4.0,
            y: -2.0,
            z: 6.0,
        },
        orientation: 3.0,
    };
    let (block, b) = living(g, 0, Some(deck), None);
    assert!(parses(&block));
    assert_eq!(u32_at(&b, 0) & MOVEFLAG_ON_TRANSPORT, MOVEFLAG_ON_TRANSPORT);
    assert_eq!(&b[24..32], &boat.guid().to_le_bytes(), "the boat's GUID");
    assert_eq!(
        [
            f32_at(&b, 32),
            f32_at(&b, 36),
            f32_at(&b, 40),
            f32_at(&b, 44)
        ],
        [4.0, -2.0, 6.0, 3.0],
        "its place and facing on deck"
    );
    // Stale transport and spline flags with nothing to send go.
    let (_, b) = living(g, MOVEFLAG_ON_TRANSPORT | 0x0040_0000, None, None);
    assert_eq!(u32_at(&b, 0), 0, "no transport, no spline");
    // Swinging at Bob: his packed GUID ends the block.
    let bob = Guid::new(5);
    let (block, _) = living(g, 0, None, Some(bob));
    assert!(parses(&block));
    let mut packed = Vec::new();
    crate::game::update::write_packed_guid(&mut packed, bob);
    assert!(
        block
            .windows(packed.len() + 4)
            .any(|w| w[..4] == 1u32.to_le_bytes() && w[4..] == packed[..]),
        "the update-all word, then Bob's packed GUID"
    );
}

/// Thistle Tea's `to_binary/1` "serializes a transport from its stationary
/// position and current progress": a transport's create carries its place
/// and facing, and its path's time last.
#[test]
fn a_transports_create_carries_its_place_and_its_paths_time() {
    let g = guid::game_object(20_808, 1);
    let fields = Fields::new(TypeId::GameObject, g, 20_808);
    let block = create_block(
        &Create {
            guid: g,
            type_id: TypeId::GameObject,
            placement: Placement::Transport {
                position: Vector3d {
                    x: 10.0,
                    y: 20.0,
                    z: 30.0,
                },
                orientation: 1.25,
                time: 123_456,
            },
            fields: &fields,
            is_self: false,
            melee_target: None,
        },
        Visibility::Public,
    );
    assert!(parses(&block));
    let mut packed = Vec::new();
    crate::game::update::write_packed_guid(&mut packed, g);
    let b = &block[1 + packed.len() + 2..];
    assert_eq!(
        [f32_at(b, 0), f32_at(b, 4), f32_at(b, 8), f32_at(b, 12)],
        [10.0, 20.0, 30.0, 1.25]
    );
    assert_eq!(u32_at(b, 16), 1, "the update-all word");
    assert_eq!(u32_at(b, 20), 123_456, "its path's time");
}

/// Thistle Tea's `UpdateObject` "transport creates set the packet transport
/// header": a packet with a ship's create (a `MoTransport` GUID) says so in
/// the byte after its count; one with a unit's, or an elevator's (a game
/// object), doesn't.
#[test]
fn a_ships_create_sets_the_packets_transport_byte_and_an_elevators_doesnt() {
    use crate::game::guid::{HighGuid, make};
    let header = |g: Guid, type_id: TypeId| {
        let fields = Fields::new(type_id, g, guid::entry(g));
        let block = create_block(
            &Create {
                guid: g,
                type_id,
                placement: Placement::Transport {
                    position: Vector3d {
                        x: 1.0,
                        y: 2.0,
                        z: 3.0,
                    },
                    orientation: 0.0,
                    time: 5,
                },
                fields: &fields,
                is_self: false,
                melee_target: None,
            },
            Visibility::Public,
        );
        let mut batch = Batch::new();
        batch.push_for(g, &block);
        let packets = batch.finish();
        // `[size BE][opcode LE][u32 count][u8 has transport]`.
        packets[0][8]
    };
    assert_eq!(
        header(make(HighGuid::MoTransport, 164_871, 1), TypeId::GameObject),
        1,
        "a ship"
    );
    assert_eq!(
        header(guid::game_object(176_080, 1), TypeId::GameObject),
        0,
        "an elevator"
    );
    let g = guid::unit(299, 7);
    let (block, _) = living(g, 0, None, None);
    let mut batch = Batch::new();
    batch.push_for(g, &block);
    assert_eq!(batch.finish()[0][8], 0, "a unit");
}
