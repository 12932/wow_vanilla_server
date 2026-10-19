//! Thistle Tea's encoding tests (Phase 3, `game/network` and the root
//! update-object tests) against wvs's own encoders: packed GUIDs, a walk's
//! packed points, update blocks and their batching, and what a player's
//! fields carry.

use std::sync::Arc;
use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapKey};
use crate::game::update::{self, Batch};

fn parse(frame: &[u8]) -> ServerOpcodeMessage {
    ServerOpcodeMessage::read_unencrypted(frame).expect("wow_messages parses the packet")
}

/// Thistle Tea's binary_utils_test: a zero GUID packs to its mask alone,
/// and reads back as zero.
#[test]
fn a_zero_guid_packs_to_its_mask_alone() {
    let mut b = Vec::new();
    update::write_packed_guid(&mut b, Guid::zero());
    assert_eq!(b, vec![0]);
    let mut b = Vec::new();
    update::write_packed_guid(&mut b, Guid::new(0x1234_0000_0000_0000));
    assert_eq!(b, vec![0xC0, 0x34, 0x12]);
    // Read back in a heartbeat.
    let frame = crate::game::conn::encode(&MSG_MOVE_HEARTBEAT_Server {
        guid: Guid::zero(),
        info: MovementInfo::default(),
    })
    .unwrap();
    match parse(&frame) {
        ServerOpcodeMessage::MSG_MOVE_HEARTBEAT(h) => assert!(h.guid.is_zero()),
        other => panic!("{other}"),
    }
}

/// Thistle Tea's binary_utils_test: a walk's points between its ends pack
/// as offsets from its end; one with no offset, or one beyond what eleven
/// bits of quarter yards hold, is left out rather than clamped, and the
/// packet still reads.
#[test]
fn a_walks_points_pack_zero_and_far_offsets() {
    use crate::game::map::creature::Walk;
    let at = |x: f32, y: f32, z: f32| Vector3d { x, y, z };
    for points in [
        vec![at(0.0, 0.0, 0.0), at(0.0, 0.0, 0.0), at(0.0, 0.0, 0.0)],
        vec![
            at(0.0, 0.0, 0.0),
            at(10_000.0, 20_000.0, 30_000.0),
            at(5.0, 0.0, 0.0),
        ],
    ] {
        let w = Walk::new(points.clone(), 2.5, Instant::now(), 1, false);
        let frame = crate::game::conn::encode(&w.packet(guid::unit(1, 1), points[0])).unwrap();
        let ServerOpcodeMessage::SMSG_MONSTER_MOVE(m) = parse(&frame) else {
            panic!("not a monster move");
        };
        let SMSG_MONSTER_MOVE_MonsterMoveType::Normal { splines, .. } = m.move_type else {
            panic!("not a normal move");
        };
        assert_eq!(splines.len(), 1, "the end alone");
    }
}

/// Thistle Tea's update_object_test: two halves set in one field keep
/// both, low then high, and the field after is untouched.
#[test]
fn two_halves_in_a_field_keep_both_and_their_neighbour() {
    let g = guid::unit(299, 7);
    let mut f = Fields::new(TypeId::Unit, g, 299);
    f.set_u32(UNIT_FIELD_BYTES_1 + 1, 0xDEAD_BEEF);
    f.set_shorts(UNIT_FIELD_BYTES_1, 0x1234, 0xABCD);
    assert_eq!(f.get_u32(UNIT_FIELD_BYTES_1), 0xABCD_1234);
    assert_eq!(f.get_u32(UNIT_FIELD_BYTES_1 + 1), 0xDEAD_BEEF);
}

/// A values block's mask words and the values its set bits carry, in
/// order: (words, values).
fn mask_and_values(block: &[u8]) -> (Vec<u32>, Vec<u32>) {
    assert_eq!(block[0], 0, "a values block");
    let mask_len = block[1].count_ones() as usize + 1;
    let at = 1 + mask_len;
    let words = block[at] as usize;
    let mask: Vec<u32> = (0..words)
        .map(|i| u32::from_le_bytes(block[at + 1 + i * 4..at + 5 + i * 4].try_into().unwrap()))
        .collect();
    let values = block[at + 1 + words * 4..]
        .chunks(4)
        .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
        .collect();
    (mask, values)
}

/// Thistle Tea's update_object_test: a values block whose highest field is
/// the last word of the nineteenth visible item's eight (its entry and
/// enchantments) has a mask reaching it, and carries it.
#[test]
fn a_mask_reaches_the_last_visible_items_last_word() {
    let g = guid::player(9);
    let mut f = Fields::new(TypeId::Player, g, 0);
    f.clear_dirty();
    let entry = PLAYER_VISIBLE_ITEM_1_0 + 18 * VISIBLE_ITEM_SIZE;
    let last = entry + 7;
    assert_eq!(last, 483, "0x1DC's eighth word");
    f.set_u32(entry, 77);
    f.set_u32(last, 88);
    let block = update::values_block(g, &f, Visibility::Public).expect("a block");
    let (mask, values) = mask_and_values(&block);
    let word = usize::from(last / 32);
    assert!(
        mask.len() > word,
        "the mask reaches word {word}: {}",
        mask.len()
    );
    assert_ne!(mask[word] & (1 << (last % 32)), 0);
    assert_eq!(&values[values.len() - 2..], &[77, 88]);
}

/// Thistle Tea's update_batcher_test: removals queued between other blocks
/// go out in one block before them all, each once; and what was said of an
/// object before its create, dropped for it, is gone.
#[test]
fn removals_lead_and_what_precedes_a_create_goes() {
    let (a, b, c) = (guid::unit(299, 1), guid::unit(299, 2), guid::unit(299, 3));
    let fields = |g| {
        let mut f = Fields::new(TypeId::Unit, g, 299);
        f.set_u32(UNIT_FIELD_HEALTH, 40);
        f.set_u32(UNIT_FIELD_MAXHEALTH, 40);
        f
    };
    let mut batch = Batch::new();
    batch.push_for(
        c,
        &update::values_block(c, &fields(c), Visibility::Public).unwrap(),
    );
    batch.forget(a);
    batch.push_for(
        c,
        &update::values_block(c, &fields(c), Visibility::Public).unwrap(),
    );
    batch.forget(b);
    batch.forget(a);
    // What was said of `c` goes, but a create of it after.
    batch.drop_for(c, false);
    let f = fields(c);
    let create = update::create_block(
        &update::Create {
            guid: c,
            type_id: TypeId::Unit,
            placement: update::Placement::Living {
                flags: 0,
                timestamp: 0,
                position: Vector3d::default(),
                orientation: 0.0,
                pitch: 0.0,
                fall_time: 0.0,
                speeds: update::Speeds::default(),
                spline: None,
                transport: None,
            },
            fields: &f,
            is_self: false,
            melee_target: None,
        },
        Visibility::Public,
    );
    batch.push_for(c, &create);
    let frames = batch.finish();
    assert_eq!(frames.len(), 1);
    let ServerOpcodeMessage::SMSG_UPDATE_OBJECT(m) = parse(&frames[0]) else {
        panic!("not an update");
    };
    assert_eq!(m.objects.len(), 2, "the removals, then the create");
    let Object_UpdateType::OutOfRangeObjects { guids } = &m.objects[0].update_type else {
        panic!("the removals first");
    };
    assert_eq!(guids, &vec![a, b], "each once");
    assert!(matches!(
        m.objects[1].update_type,
        Object_UpdateType::CreateObject { .. } | Object_UpdateType::CreateObject2 { .. }
    ));
}

async fn on_map<T: Send + 'static>(
    realm: &Arc<Realm>,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    realm
        .maps
        .get(realm, MapKey::world(0))
        .query(f)
        .await
        .expect("the map answers")
}

/// Thistle Tea's update_batcher_test: a field changed twice in one update
/// goes out once, in one values block, with its last value.
#[tokio::test]
async fn a_field_changed_twice_goes_out_once_with_its_last_value() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    b.sees(ga).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    b.clear();
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        p.fields.set_u32(UNIT_FIELD_MAXHEALTH, 900);
        p.fields.set_u32(UNIT_FIELD_HEALTH, 100);
        p.fields.set_u32(UNIT_FIELD_HEALTH, 200);
    })
    .await;
    let healths = b
        .wait_for("Alice's health", |m| {
            let found: Vec<_> = update_values(m)
                .into_iter()
                .filter(|(g, _)| *g == ga)
                .collect();
            (!found.is_empty()).then_some(found)
        })
        .await;
    assert_eq!(healths.len(), 1, "one values block for Alice");
    assert_eq!(healths[0].1, Some(200), "her last health");
    let _ = gb;
}

fn update_values(m: &ServerOpcodeMessage) -> Vec<(Guid, Option<i32>)> {
    harness::values(m)
        .into_iter()
        .map(|(g, mask)| {
            let health = match mask {
                UpdateMask::Player(p) => p.unit_health(),
                UpdateMask::Unit(u) => u.unit_health(),
                _ => None,
            };
            (g, health)
        })
        .collect()
}

/// Thistle Tea's update_object_test: combo points gone are 0 against the
/// target they were on, as mangos's ClearComboPoints sends them.
#[tokio::test]
async fn combo_points_gone_are_none_on_the_last_target() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Rogue).await;
    let target = guid::unit(299, 77);
    let (field, points) = on_map(&realm, move |m| {
        if let Some(c) = m.caster_mut(g) {
            c.combo_points = 3;
            c.combo_target = target;
        }
        let p = m.players.get_mut(&g).unwrap();
        p.fields.set_guid(PLAYER_FIELD_COMBO_TARGET, target);
        p.fields.set_byte(PLAYER_FIELD_BYTES, 1, 3);
        crate::game::map::spells::clear_combo_points(m, g);
        let p = &m.players[&g];
        (
            p.fields.get_guid(PLAYER_FIELD_COMBO_TARGET),
            p.fields.get_byte(PLAYER_FIELD_BYTES, 1),
        )
    })
    .await;
    assert_eq!((field, points), (target, 0));
}

/// Thistle Tea's smsg_monster_move_test: a walk's final facing is its own
/// move type (3, a target; 4, an angle), the target preferred, never a
/// spline flag; the flags are run mode's alone; and a flyer's spline is
/// written as whole points after the start, its flags flying and run mode.
#[test]
fn a_monster_moves_facing_and_flying_splines_are_laid_out_as_1_12_has_them() {
    use crate::game::map::creature::Walk;
    let at = |x: f32, y: f32, z: f32| Vector3d { x, y, z };
    let g = guid::unit(1, 1);
    let mut w = Walk::new(
        vec![at(0.0, 0.0, 0.0), at(10.0, 0.0, 0.0)],
        2.5,
        Instant::now(),
        5,
        false,
    );
    let flags = |m: &SMSG_MONSTER_MOVE_MonsterMoveType| match m {
        SMSG_MONSTER_MOVE_MonsterMoveType::Normal { spline_flags, .. }
        | SMSG_MONSTER_MOVE_MonsterMoveType::FacingTarget { spline_flags, .. }
        | SMSG_MONSTER_MOVE_MonsterMoveType::FacingAngle { spline_flags, .. } => {
            spline_flags.as_int()
        }
        _ => panic!("{m:?}"),
    };
    let target = guid::player(2);
    w.face_target = Some(target);
    w.face = Some(1.5);
    let m = w.packet(g, at(0.0, 0.0, 0.0)).move_type;
    assert!(
        matches!(m, SMSG_MONSTER_MOVE_MonsterMoveType::FacingTarget { target: t, .. } if t == target),
        "the target preferred"
    );
    assert_eq!(flags(&m), 0x100, "run mode alone, no facing flag");
    w.face_target = None;
    let m = w.packet(g, at(0.0, 0.0, 0.0)).move_type;
    assert!(
        matches!(m, SMSG_MONSTER_MOVE_MonsterMoveType::FacingAngle { angle, .. } if angle == 1.5)
    );
    // A flyer: whole points.
    let mut fly = Walk::new(
        vec![at(0.0, 0.0, 0.0), at(1.0, 2.0, 3.0), at(4.0, 5.0, 6.0)],
        2.5,
        Instant::now(),
        5,
        false,
    );
    fly.fly = true;
    let frame = fly.frame(g, at(0.0, 0.0, 0.0)).unwrap();
    let body = &frame[4..];
    let mut want = Vec::new();
    update::write_packed_guid(&mut want, g);
    for f in [0.0f32, 0.0, 0.0] {
        want.extend_from_slice(&f.to_le_bytes());
    }
    want.extend_from_slice(&5u32.to_le_bytes());
    want.push(0);
    want.extend_from_slice(&0x300u32.to_le_bytes());
    want.extend_from_slice(&(fly.duration.as_millis() as u32).to_le_bytes());
    want.extend_from_slice(&2u32.to_le_bytes());
    for f in [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0] {
        want.extend_from_slice(&f.to_le_bytes());
    }
    assert_eq!(body, &want[..]);
}
