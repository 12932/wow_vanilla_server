//! `SMSG_UPDATE_OBJECT` blocks, encoded by hand so that a block is built
//! once and its bytes shared by every observer that gets it.
//!
//! A packet is `[u32 block count][u8 has transport][blocks]`; [`Batch`]
//! gathers one observer's blocks and frames them, compressing big batches
//! into `SMSG_COMPRESSED_UPDATE_OBJECT`.

use std::io::Write;
use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::{MovementFlags, Vector3d};

use crate::game::fields::{Fields, TypeId, Visibility};

pub const SMSG_UPDATE_OBJECT: u16 = 0x0A9;
pub const SMSG_COMPRESSED_UPDATE_OBJECT: u16 = 0x1F6;

const UPDATETYPE_VALUES: u8 = 0;
const UPDATETYPE_MOVEMENT: u8 = 1;
const UPDATETYPE_CREATE_OBJECT: u8 = 2;
const UPDATETYPE_CREATE_OBJECT2: u8 = 3;
const UPDATETYPE_OUT_OF_RANGE_OBJECTS: u8 = 4;

// Movement block update flags.
pub const UPDATEFLAG_SELF: u8 = 0x01;
pub const UPDATEFLAG_TRANSPORT: u8 = 0x02;
pub const UPDATEFLAG_MELEE_ATTACKING: u8 = 0x04;
pub const UPDATEFLAG_HIGHGUID: u8 = 0x08;
pub const UPDATEFLAG_ALL: u8 = 0x10;
pub const UPDATEFLAG_LIVING: u8 = 0x20;
pub const UPDATEFLAG_HAS_POSITION: u8 = 0x40;

// Movement flags that change what a living block carries.
pub const MOVEFLAG_FORWARD: u32 = MovementFlags::FORWARD;
pub const MOVEFLAG_WALK_MODE: u32 = MovementFlags::WALK_MODE;
pub const MOVEFLAG_ON_TRANSPORT: u32 = MovementFlags::ON_TRANSPORT;
pub const MOVEFLAG_JUMPING: u32 = MovementFlags::JUMPING;
pub const MOVEFLAG_SWIMMING: u32 = MovementFlags::SWIMMING;
pub const MOVEFLAG_SPLINE_ENABLED: u32 = MovementFlags::SPLINE_ENABLED;
pub const MOVEFLAG_SPLINE_ELEVATION: u32 = MovementFlags::SPLINE_ELEVATION;

/// Run, walk, swim and turn speeds, in the order a living block carries
/// them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Speeds {
    pub walk: f32,
    pub run: f32,
    pub run_back: f32,
    pub swim: f32,
    pub swim_back: f32,
    pub turn: f32,
}

impl Default for Speeds {
    fn default() -> Self {
        Self {
            walk: 2.5,
            run: 7.0,
            run_back: 4.5,
            swim: 4.722_222,
            swim_back: 2.5,
            turn: std::f32::consts::PI,
        }
    }
}

/// A spline a unit is moving along, for observers that see it start
/// partway through.
#[derive(Debug, Clone, PartialEq)]
pub struct SplineState {
    pub flags: u32,
    pub time_passed: u32,
    pub duration: u32,
    pub id: u32,
    pub nodes: Vec<Vector3d>,
    pub final_node: Vector3d,
}

/// A unit on a transport: the transport, and its place and facing on deck.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aboard {
    pub transport: Guid,
    pub position: Vector3d,
    pub orientation: f32,
}

/// What a create block says about where an object is.
#[derive(Debug, Clone, PartialEq)]
pub enum Placement {
    /// A unit: its movement state and speeds, and the transport it is on.
    Living {
        flags: u32,
        timestamp: u32,
        position: Vector3d,
        orientation: f32,
        pitch: f32,
        fall_time: u32,
        speeds: Speeds,
        spline: Option<SplineState>,
        transport: Option<Aboard>,
    },
    /// A game object, corpse or dynamic object.
    Position {
        position: Vector3d,
        orientation: f32,
    },
    /// A transport (an elevator, a boat): where it is, and the server's
    /// clock its client times its path by.
    Transport {
        position: Vector3d,
        orientation: f32,
        time: u32,
    },
    /// An item: nowhere.
    None,
}

/// A create block's object: its kind, place and fields.
pub struct Create<'a> {
    pub guid: Guid,
    pub type_id: TypeId,
    pub placement: Placement,
    pub fields: &'a Fields,
    /// The block is for the object's own client.
    pub is_self: bool,
    /// The target the unit is swinging at, if it is.
    pub melee_target: Option<Guid>,
}

pub fn write_packed_guid(out: &mut Vec<u8>, guid: Guid) {
    let value = guid.guid();
    let mask_at = out.len();
    out.push(0);
    let mut mask = 0u8;
    for i in 0..8 {
        let byte = (value >> (i * 8)) as u8;
        if byte != 0 {
            mask |= 1 << i;
            out.push(byte);
        }
    }
    out[mask_at] = mask;
}

/// `SplineFlags::FLYING`: the client draws the path smooth (Catmull-Rom).
const SPLINE_FLYING: u32 = 0x0000_0200;

/// A create's spline points as mangos-classic sends them, its spline's
/// whole array (`MoveSpline::getPath`, `SplineBase::InitLinear` and
/// `InitCatmullRom`): a linear path with its last point again; a smooth one
/// with a point before its first (the first less the step to the second)
/// and its last again. The 1.12 client counts a create's segments as its
/// points less three (`0x454770`, `lea esi, [eax - 3]`), and a create whose
/// spline had fewer points crashed it, copying a count of -1 (`0x6194DD`).
pub fn create_spline_points(flags: u32, path: &[Vector3d]) -> Vec<Vector3d> {
    let (Some(&first), Some(&last)) = (path.first(), path.last()) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(path.len() + 3);
    if flags & SPLINE_FLYING != 0 {
        let second = path.get(1).copied().unwrap_or(first);
        out.push(Vector3d {
            x: first.x * 2.0 - second.x,
            y: first.y * 2.0 - second.y,
            z: first.z * 2.0 - second.z,
        });
    }
    out.extend_from_slice(path);
    if path.len() == 1 {
        // A path of one point is the point twice, as mangos asks for two.
        out.push(last);
    }
    out.push(last);
    out
}

fn write_vector(out: &mut Vec<u8>, v: Vector3d) {
    out.extend_from_slice(&v.x.to_le_bytes());
    out.extend_from_slice(&v.y.to_le_bytes());
    out.extend_from_slice(&v.z.to_le_bytes());
}

fn write_fields(out: &mut Vec<u8>, mask: &[u32], values: &[u32]) {
    out.push(mask.len() as u8);
    for w in mask {
        out.extend_from_slice(&w.to_le_bytes());
    }
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

fn write_placement(out: &mut Vec<u8>, c: &Create) {
    let mut flags = UPDATEFLAG_ALL;
    if c.is_self {
        flags |= UPDATEFLAG_SELF;
    }
    if c.melee_target.is_some() {
        flags |= UPDATEFLAG_MELEE_ATTACKING;
    }
    match &c.placement {
        Placement::Living { .. } => flags |= UPDATEFLAG_LIVING,
        Placement::Position { .. } => flags |= UPDATEFLAG_HAS_POSITION,
        Placement::Transport { .. } => flags |= UPDATEFLAG_HAS_POSITION | UPDATEFLAG_TRANSPORT,
        Placement::None => {}
    }
    out.push(flags);
    match &c.placement {
        Placement::Living {
            flags,
            timestamp,
            position,
            orientation,
            pitch,
            fall_time,
            speeds,
            spline,
            transport,
        } => {
            // A create carries a stable state: no jump or spline elevation;
            // a spline only if there is one to send; a transport if aboard,
            // its whole GUID and the place and facing on deck (mangos's
            // `MovementInfo::Write`, wow_messages's `MovementBlock`).
            let mut move_flags = *flags
                & !(MOVEFLAG_ON_TRANSPORT
                    | MOVEFLAG_JUMPING
                    | MOVEFLAG_SPLINE_ELEVATION
                    | MOVEFLAG_SPLINE_ENABLED);
            if spline.is_some() {
                move_flags |= MOVEFLAG_SPLINE_ENABLED;
            }
            if transport.is_some() {
                move_flags |= MOVEFLAG_ON_TRANSPORT;
            }
            out.extend_from_slice(&move_flags.to_le_bytes());
            out.extend_from_slice(&timestamp.to_le_bytes());
            write_vector(out, *position);
            out.extend_from_slice(&orientation.to_le_bytes());
            if let Some(t) = transport {
                out.extend_from_slice(&t.transport.guid().to_le_bytes());
                write_vector(out, t.position);
                out.extend_from_slice(&t.orientation.to_le_bytes());
            }
            if move_flags & MOVEFLAG_SWIMMING != 0 {
                out.extend_from_slice(&pitch.to_le_bytes());
            }
            out.extend_from_slice(&fall_time.to_le_bytes());
            for s in [
                speeds.walk,
                speeds.run,
                speeds.run_back,
                speeds.swim,
                speeds.swim_back,
                speeds.turn,
            ] {
                out.extend_from_slice(&s.to_le_bytes());
            }
            if let Some(spline) = spline {
                // No final angle, target or point.
                let spline_flags = spline.flags & !(0x0001_0000 | 0x0002_0000 | 0x0004_0000);
                out.extend_from_slice(&spline_flags.to_le_bytes());
                out.extend_from_slice(&spline.time_passed.to_le_bytes());
                out.extend_from_slice(&spline.duration.to_le_bytes());
                out.extend_from_slice(&spline.id.to_le_bytes());
                let points = create_spline_points(spline.flags, &spline.nodes);
                out.extend_from_slice(&(points.len() as u32).to_le_bytes());
                for n in &points {
                    write_vector(out, *n);
                }
                write_vector(out, spline.final_node);
            }
        }
        Placement::Position {
            position,
            orientation,
        } => {
            write_vector(out, *position);
            out.extend_from_slice(&orientation.to_le_bytes());
        }
        Placement::Transport {
            position,
            orientation,
            ..
        } => {
            write_vector(out, *position);
            out.extend_from_slice(&orientation.to_le_bytes());
        }
        Placement::None => {}
    }
    // UPDATEFLAG_ALL's word.
    out.extend_from_slice(&1u32.to_le_bytes());
    if let Some(target) = c.melee_target {
        write_packed_guid(out, target);
    }
    if let Placement::Transport { time, .. } = &c.placement {
        out.extend_from_slice(&time.to_le_bytes());
    }
}

/// A create block for `c`, with the fields `visibility` may see.
pub fn create_block(c: &Create, visibility: Visibility) -> Vec<u8> {
    let mut out = Vec::with_capacity(256);
    // Units and players use CREATE_OBJECT2 (a spawn the client animates);
    // everything else the plain one.
    let kind = match c.type_id {
        TypeId::Unit | TypeId::Player => UPDATETYPE_CREATE_OBJECT2,
        _ => UPDATETYPE_CREATE_OBJECT,
    };
    out.push(kind);
    write_packed_guid(&mut out, c.guid);
    out.push(c.type_id as u8);
    write_placement(&mut out, c);
    let (mask, values) = c.fields.select(visibility, false);
    write_fields(&mut out, &mask, &values);
    out
}

/// A values block with `fields`' dirty fields that `visibility` may see,
/// or `None` if there are none.
pub fn values_block(guid: Guid, fields: &Fields, visibility: Visibility) -> Option<Vec<u8>> {
    // Nothing changed, so nothing to say (and nothing to allocate for).
    if !fields.is_dirty() {
        return None;
    }
    let (mask, values) = fields.select(visibility, true);
    let only_type = values.len() == 1
        && mask.len() == 1
        && mask[0] == 1 << crate::game::fields::OBJECT_FIELD_TYPE;
    if values.is_empty() || only_type {
        return None;
    }
    let mut out = Vec::with_capacity(16 + values.len() * 4);
    out.push(UPDATETYPE_VALUES);
    write_packed_guid(&mut out, guid);
    write_fields(&mut out, &mask, &values);
    Some(out)
}

/// A block telling the client these objects are out of range: it forgets
/// them.
pub fn out_of_range_block(guids: &[Guid]) -> Vec<u8> {
    let mut out = Vec::with_capacity(5 + guids.len() * 9);
    out.push(UPDATETYPE_OUT_OF_RANGE_OBJECTS);
    out.extend_from_slice(&(guids.len() as u32).to_le_bytes());
    for g in guids {
        write_packed_guid(&mut out, *g);
    }
    out
}

/// A movement block: where a unit now is, for a teleport the observer
/// must see as a jump.
pub fn movement_block(guid: Guid, placement: Placement) -> Vec<u8> {
    let mut out = Vec::with_capacity(64);
    out.push(UPDATETYPE_MOVEMENT);
    write_packed_guid(&mut out, guid);
    let c = Create {
        guid,
        type_id: TypeId::Unit,
        placement,
        fields: empty_fields(),
        is_self: false,
        melee_target: None,
    };
    write_placement(&mut out, &c);
    out
}

fn empty_fields() -> &'static Fields {
    static F: std::sync::OnceLock<Fields> = std::sync::OnceLock::new();
    F.get_or_init(|| Fields::new(TypeId::Item, Guid::zero(), 0))
}

/// A batch above this many bytes is sent compressed.
const COMPRESS_ABOVE: usize = 1024;
/// A batch is cut into packets of at most this many bytes of blocks, so
/// no packet needs a size the 1.12 client can't read.
const PACKET_LIMIT: usize = 24 * 1024;

/// One observer's blocks for one update, framed into packets by
/// [`Batch::finish`].
///
/// What the client is to forget goes first, in one out-of-range block, as
/// mangos writes it: the 1.12 client crashes on a values or create block
/// for an object it was told in the same packet to forget, and on corpses
/// created ahead of their owners' going. A block about an object (a create,
/// or a values block sent during the update) is kept with its object, so
/// forgetting the object drops what the client would no longer have.
///
/// An object gets one values block a packet: the client keeps a copy of
/// an object's fields as they were before each values block is written
/// (its first pass, `0x466590`, copying at `0x4667a0`), and its second pass
/// runs a field's handlers only where the field differs from that copy
/// (`0x465570`). A second values block about the object in the packet
/// copies the first's changes over, and they run nothing: a death neither
/// fell nor showed on the target's frame, a Frost Nova showed no frost
/// (`tap::tell`, which gives each viewer its own block).
#[derive(Debug, Default)]
pub struct Batch {
    forget: Vec<Guid>,
    body: Vec<u8>,
    spans: Vec<Span>,
}

#[derive(Debug, Clone, Copy)]
struct Span {
    object: Option<Guid>,
    start: u32,
    len: u32,
    create: bool,
    /// A create of a boat or zeppelin.
    transport: bool,
    live: bool,
}

impl Batch {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.forget.is_empty() && !self.spans.iter().any(|s| s.live)
    }

    /// A block about nothing the update may take away.
    pub fn push(&mut self, block: &[u8]) {
        self.add(None, block);
    }

    /// A block about `object`: a create, or its values.
    pub fn push_for(&mut self, object: Guid, block: &[u8]) {
        self.add(Some(object), block);
    }

    fn add(&mut self, object: Option<Guid>, block: &[u8]) {
        let create = matches!(
            block.first(),
            Some(&UPDATETYPE_CREATE_OBJECT | &UPDATETYPE_CREATE_OBJECT2)
        );
        self.spans.push(Span {
            object,
            start: self.body.len() as u32,
            len: block.len() as u32,
            create,
            transport: create && creates_transport(block),
            live: true,
        });
        self.body.extend_from_slice(block);
    }

    /// The creates for `object` in the batch.
    pub fn creates_of(&self, object: Guid) -> usize {
        self.spans
            .iter()
            .filter(|s| s.live && s.create && s.object == Some(object))
            .count()
    }

    /// The client forgets `object` first of all.
    pub fn forget(&mut self, object: Guid) {
        if !self.forget.contains(&object) {
            self.forget.push(object);
        }
    }

    /// What the batch says about `object`, all of it (`keep_last_create`:
    /// but its last create and what follows it).
    pub fn drop_for(&mut self, object: Guid, keep_last_create: bool) {
        let last = keep_last_create
            .then(|| {
                self.spans
                    .iter()
                    .rposition(|s| s.live && s.create && s.object == Some(object))
            })
            .flatten();
        for (i, s) in self.spans.iter_mut().enumerate() {
            if s.object == Some(object) && last.is_none_or(|l| i < l) {
                s.live = false;
            }
        }
    }

    /// The batch's packets, ready to queue: the out-of-range block first,
    /// then the blocks in order, cut so no packet needs a size the 1.12
    /// client can't read. A packet that makes a boat or zeppelin says it
    /// has a transport, as mangos's `SendInitTransports` and Thistle Tea's
    /// header do.
    pub fn finish(self) -> Vec<Arc<[u8]>> {
        let mut packets = Vec::new();
        let mut count = 0u32;
        let mut transport = false;
        let mut body = Vec::new();
        let mut cut = |count: &mut u32, transport: &mut bool, body: &mut Vec<u8>| {
            if *count == 0 {
                return;
            }
            let mut payload = Vec::with_capacity(5 + body.len());
            payload.extend_from_slice(&count.to_le_bytes());
            payload.push(u8::from(*transport));
            payload.extend_from_slice(body);
            packets.push(frame_update(&payload));
            body.clear();
            *count = 0;
            *transport = false;
        };
        // At most 9 bytes a guid: a block that keeps under the limit.
        for guids in self.forget.chunks(PACKET_LIMIT / 9) {
            body.extend_from_slice(&out_of_range_block(guids));
            count += 1;
        }
        for s in self.spans.iter().filter(|s| s.live) {
            let block = &self.body[s.start as usize..(s.start + s.len) as usize];
            if !body.is_empty() && body.len() + block.len() > PACKET_LIMIT {
                cut(&mut count, &mut transport, &mut body);
            }
            body.extend_from_slice(block);
            count += 1;
            transport |= s.transport;
        }
        cut(&mut count, &mut transport, &mut body);
        packets
    }
}

/// Whether a create block makes a boat or zeppelin: its packed GUID is a
/// moving transport's.
fn creates_transport(block: &[u8]) -> bool {
    let Some((&mask, rest)) = block.get(1..).and_then(|b| b.split_first()) else {
        return false;
    };
    let mut guid = 0u64;
    let mut bytes = rest.iter();
    for i in 0..8 {
        if mask & (1 << i) != 0 {
            let Some(&b) = bytes.next() else {
                return false;
            };
            guid |= u64::from(b) << (i * 8);
        }
    }
    crate::game::guid::high(Guid::new(guid)) == Some(crate::game::guid::HighGuid::MoTransport)
}

/// `[size BE][opcode LE][body]`, the shape the writer task encrypts.
pub fn frame(opcode: u16, body: &[u8]) -> Arc<[u8]> {
    let size = u16::try_from(body.len() + 2).unwrap_or(u16::MAX);
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&size.to_be_bytes());
    out.extend_from_slice(&opcode.to_le_bytes());
    out.extend_from_slice(body);
    Arc::from(out)
}

fn frame_update(payload: &[u8]) -> Arc<[u8]> {
    if payload.len() <= COMPRESS_ABOVE {
        return frame(SMSG_UPDATE_OBJECT, payload);
    }
    let mut body = Vec::with_capacity(payload.len() / 2 + 8);
    body.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    let mut z = flate2::write::ZlibEncoder::new(body, flate2::Compression::fast());
    if z.write_all(payload).is_err() {
        return frame(SMSG_UPDATE_OBJECT, payload);
    }
    match z.finish() {
        Ok(body) => frame(SMSG_COMPRESSED_UPDATE_OBJECT, &body),
        Err(_) => frame(SMSG_UPDATE_OBJECT, payload),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::fields::*;
    use crate::game::guid;
    use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
    use wow_world_messages::vanilla::{Object_UpdateType, UpdateMask};

    fn parse(frame: &[u8]) -> ServerOpcodeMessage {
        ServerOpcodeMessage::read_unencrypted(frame).expect("wow_messages parses the packet")
    }

    fn unit_fields(g: Guid) -> Fields {
        let mut f = Fields::new(TypeId::Unit, g, 299);
        f.set_u32(UNIT_FIELD_HEALTH, 40);
        f.set_u32(UNIT_FIELD_MAXHEALTH, 40);
        f.set_u32(UNIT_FIELD_LEVEL, 3);
        f.set_u32(UNIT_FIELD_DISPLAYID, 903);
        f
    }

    fn v(x: f32, y: f32, z: f32) -> Vector3d {
        Vector3d { x, y, z }
    }

    /// The crash of 2026-10-18: a gorilla walking a two-point path came into
    /// view, and the 1.12 client, counting the create's segments as its
    /// points less three, copied a count of -1. A create's spline carries
    /// mangos's whole array: a linear path with its last point again, a
    /// smooth one with a point before its first too, so never fewer than
    /// three.
    #[test]
    fn a_create_spline_never_has_fewer_points_than_the_client_counts_from() {
        let (a, b, c) = (v(1.0, 2.0, 3.0), v(4.0, 6.0, 3.0), v(9.0, 9.0, 9.0));
        assert_eq!(create_spline_points(0, &[a, b]), vec![a, b, b]);
        assert_eq!(
            create_spline_points(SPLINE_FLYING, &[a, b]),
            vec![v(-2.0, -2.0, 3.0), a, b, b]
        );
        assert_eq!(create_spline_points(0, &[a, b, c]), vec![a, b, c, c]);
        assert_eq!(create_spline_points(0, &[a]), vec![a, a, a]);
        assert!(create_spline_points(0, &[]).is_empty());
        for flags in [0, SPLINE_FLYING, 0x100, 0x100 | SPLINE_FLYING] {
            for n in 1..8 {
                let path: Vec<Vector3d> = (0..n).map(|i| v(i as f32, 0.0, 0.0)).collect();
                assert!(
                    create_spline_points(flags, &path).len() >= 3,
                    "{flags:#x} {n}"
                );
            }
        }
        // As the block carries it: the gorilla's three points.
        let g = guid::unit(1557, 2521);
        let fields = unit_fields(g);
        let spline = SplineState {
            flags: 0,
            time_passed: 1675,
            duration: 1768,
            id: 446,
            nodes: vec![v(-13719.4, 249.08, 28.52), v(-13718.58, 253.25, 29.73)],
            final_node: v(-13718.58, 253.25, 29.73),
        };
        let block = create_block(
            &Create {
                guid: g,
                type_id: TypeId::Unit,
                placement: Placement::Living {
                    flags: 0x0040_0000,
                    timestamp: 0,
                    position: v(-13718.63, 253.03, 29.67),
                    orientation: 1.38,
                    pitch: 0.0,
                    fall_time: 0,
                    speeds: Speeds::default(),
                    spline: Some(spline),
                    transport: None,
                },
                fields: &fields,
                is_self: false,
                melee_target: None,
            },
            Visibility::Public,
        );
        let header: Vec<u8> = [1675u32, 1768, 446]
            .iter()
            .flat_map(|n| n.to_le_bytes())
            .collect();
        let at = block
            .windows(header.len())
            .position(|w| w == header)
            .expect("the spline's times and id")
            + header.len();
        let count = u32::from_le_bytes(block[at..at + 4].try_into().unwrap());
        assert_eq!(count, 3);
    }

    #[test]
    fn a_unit_create_parses_with_its_fields_and_place() {
        let g = guid::unit(299, 7);
        let fields = unit_fields(g);
        let block = create_block(
            &Create {
                guid: g,
                type_id: TypeId::Unit,
                placement: Placement::Living {
                    flags: MOVEFLAG_FORWARD,
                    timestamp: 0,
                    position: Vector3d {
                        x: 1.0,
                        y: 2.0,
                        z: 3.0,
                    },
                    orientation: 0.5,
                    pitch: 0.0,
                    fall_time: 0,
                    speeds: Speeds::default(),
                    spline: None,
                    transport: None,
                },
                fields: &fields,
                is_self: false,
                melee_target: None,
            },
            Visibility::Public,
        );
        let mut batch = Batch::new();
        batch.push(&block);
        let packets = batch.finish();
        assert_eq!(packets.len(), 1);
        let ServerOpcodeMessage::SMSG_UPDATE_OBJECT(m) = parse(&packets[0]) else {
            panic!("not an update object");
        };
        assert_eq!(m.objects.len(), 1);
        let Object_UpdateType::CreateObject2 { guid3, mask2, .. } = &m.objects[0].update_type
        else {
            panic!("not a create");
        };
        assert_eq!(*guid3, g);
        let UpdateMask::Unit(u) = mask2 else {
            panic!("not a unit mask");
        };
        assert_eq!(u.unit_health(), Some(40));
        assert_eq!(u.unit_displayid(), Some(903));
        assert_eq!(u.object_entry(), Some(299));
    }

    #[test]
    fn a_values_block_carries_only_the_changed_fields() {
        let g = guid::unit(299, 7);
        let mut fields = unit_fields(g);
        fields.clear_dirty();
        assert!(values_block(g, &fields, Visibility::Public).is_none());
        fields.set_u32(UNIT_FIELD_HEALTH, 12);
        let block = values_block(g, &fields, Visibility::Public).unwrap();
        let mut batch = Batch::new();
        batch.push(&block);
        let ServerOpcodeMessage::SMSG_UPDATE_OBJECT(m) = parse(&batch.finish()[0]) else {
            panic!("not an update object");
        };
        let Object_UpdateType::Values { guid1, mask1 } = &m.objects[0].update_type else {
            panic!("not values");
        };
        assert_eq!(*guid1, g);
        let UpdateMask::Unit(u) = mask1 else {
            panic!("not a unit mask");
        };
        assert_eq!(u.unit_health(), Some(12));
        assert_eq!(u.unit_level(), None);
    }

    #[test]
    fn a_big_batch_is_compressed_and_still_parses() {
        let mut batch = Batch::new();
        let mut guids = Vec::new();
        for i in 0..100 {
            let g = guid::unit(299, i);
            guids.push(g);
            let fields = unit_fields(g);
            batch.push(&create_block(
                &Create {
                    guid: g,
                    type_id: TypeId::Unit,
                    placement: Placement::Position {
                        position: Vector3d {
                            x: 0.0,
                            y: 0.0,
                            z: 0.0,
                        },
                        orientation: 0.0,
                    },
                    fields: &fields,
                    is_self: false,
                    melee_target: None,
                },
                Visibility::Public,
            ));
        }
        batch.push(&out_of_range_block(&guids[..3]));
        let packets = batch.finish();
        let mut objects = 0;
        for p in &packets {
            match parse(p) {
                ServerOpcodeMessage::SMSG_COMPRESSED_UPDATE_OBJECT(m) => objects += m.objects.len(),
                ServerOpcodeMessage::SMSG_UPDATE_OBJECT(m) => objects += m.objects.len(),
                other => panic!("unexpected {other}"),
            }
        }
        assert_eq!(objects, 101);
        assert!(packets.iter().all(|p| p.len() < 0x7FFF));
    }

    #[test]
    fn a_player_create_for_its_owner_parses() {
        let g = guid::player(9);
        let mut fields = Fields::new(TypeId::Player, g, 0);
        fields.set_u32(UNIT_FIELD_HEALTH, 60);
        fields.set_u32(PLAYER_FIELD_COINAGE, 5);
        let block = create_block(
            &Create {
                guid: g,
                type_id: TypeId::Player,
                placement: Placement::Living {
                    flags: MOVEFLAG_SWIMMING,
                    timestamp: 1,
                    position: Vector3d {
                        x: 1.0,
                        y: 2.0,
                        z: 3.0,
                    },
                    orientation: 0.0,
                    pitch: 0.25,
                    fall_time: 0,
                    speeds: Speeds::default(),
                    spline: None,
                    transport: None,
                },
                fields: &fields,
                is_self: true,
                melee_target: Some(guid::unit(1, 1)),
            },
            Visibility::Owner,
        );
        let mut batch = Batch::new();
        batch.push(&block);
        let ServerOpcodeMessage::SMSG_UPDATE_OBJECT(m) = parse(&batch.finish()[0]) else {
            panic!("not an update object");
        };
        let Object_UpdateType::CreateObject2 { mask2, .. } = &m.objects[0].update_type else {
            panic!("not a create");
        };
        let UpdateMask::Player(p) = mask2 else {
            panic!("not a player mask");
        };
        assert_eq!(p.player_field_coinage(), Some(5));
    }

    fn unit_create(g: Guid, fields: &Fields) -> Vec<u8> {
        create_block(
            &Create {
                guid: g,
                type_id: TypeId::Unit,
                placement: Placement::Position {
                    position: Vector3d {
                        x: 1.0,
                        y: 2.0,
                        z: 3.0,
                    },
                    orientation: 0.0,
                },
                fields,
                is_self: false,
                melee_target: None,
            },
            Visibility::Public,
        )
    }

    fn kinds(packets: &[Arc<[u8]>]) -> Vec<(&'static str, Vec<Guid>)> {
        let mut out = Vec::new();
        for p in packets {
            let ServerOpcodeMessage::SMSG_UPDATE_OBJECT(m) = parse(p) else {
                panic!("not an update object");
            };
            for o in &m.objects {
                out.push(match &o.update_type {
                    Object_UpdateType::OutOfRangeObjects { guids } => ("forget", guids.clone()),
                    Object_UpdateType::CreateObject { guid3, .. }
                    | Object_UpdateType::CreateObject2 { guid3, .. } => ("create", vec![*guid3]),
                    Object_UpdateType::Values { guid1, .. } => ("values", vec![*guid1]),
                    other => panic!("unexpected {other:?}"),
                });
            }
        }
        out
    }

    /// What the 1.12 client crashed on: corpses created, then their owners
    /// forgotten; a creature made anew, forgotten, then its values. What is
    /// forgotten goes first.
    #[test]
    fn what_is_forgotten_goes_first() {
        let (owner, x) = (guid::player(0x20), guid::unit(299, 7));
        let fields = unit_fields(x);
        let mut batch = Batch::new();
        batch.push_for(x, &unit_create(x, &fields));
        batch.forget(owner);
        batch.push(&values_block(x, &fields, Visibility::Public).unwrap());
        assert_eq!(
            kinds(&batch.finish()),
            [
                ("forget", vec![owner]),
                ("create", vec![x]),
                ("values", vec![x]),
            ]
        );
    }

    /// Learned of and lost in one update: nothing of it goes. Lost and
    /// learned of again: it is forgotten, then made anew, once.
    #[test]
    fn what_the_client_wont_have_is_dropped() {
        let x = guid::unit(299, 7);
        let fields = unit_fields(x);
        let mut batch = Batch::new();
        batch.push_for(x, &unit_create(x, &fields));
        assert_eq!(batch.creates_of(x), 1);
        batch.drop_for(x, false);
        assert!(batch.is_empty());
        assert!(batch.finish().is_empty());
        let mut batch = Batch::new();
        batch.push_for(x, &unit_create(x, &fields));
        batch.push_for(x, &unit_create(x, &fields));
        batch.forget(x);
        batch.drop_for(x, true);
        assert_eq!(
            kinds(&batch.finish()),
            [("forget", vec![x]), ("create", vec![x])]
        );
    }
}
