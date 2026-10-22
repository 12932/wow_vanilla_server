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
    /// Which way it faces on arriving (`Walk::face`, `FINAL_ANGLE`).
    pub final_angle: Option<f32>,
    /// The unit it faces as it goes and on arriving (`Walk::face_target`,
    /// `FINAL_TARGET`; over an angle, as `SMSG_MONSTER_MOVE` has it).
    pub final_target: Option<Guid>,
}

/// A jump under way, as the jumper's client said it (mangos's
/// `MovementInfo::jump`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Jump {
    pub z_speed: f32,
    pub cos_angle: f32,
    pub sin_angle: f32,
    pub xy_speed: f32,
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
        /// A jump under way: `MOVEFLAG_JUMPING` and its data.
        jump: Option<Jump>,
        /// `MOVEFLAG_SPLINE_ELEVATION` and its height.
        spline_elevation: Option<f32>,
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

/// `SplineFlags::RUN_MODE`: the unit runs along the path, not walks.
const SPLINE_RUN_MODE: u32 = 0x0000_0100;
/// `SplineFlags::FINAL_TARGET` and `FINAL_ANGLE`: what a spline faces at
/// its end; with `FINAL_POINT`, `Mask_Final_Facing`.
const SPLINE_FINAL_TARGET: u32 = 0x0002_0000;
const SPLINE_FINAL_ANGLE: u32 = 0x0004_0000;
const SPLINE_FINAL_FACING: u32 = 0x0001_0000 | SPLINE_FINAL_TARGET | SPLINE_FINAL_ANGLE;

/// A create's spline points as mangos-classic sends them, its spline's
/// whole array (`MoveSpline::getPath`, `SplineBase::InitCatmullRom`, which
/// mangos uses for a linear path too, "client's internal structure
/// limitation"): a point before the first (the first less the step to the
/// second), the path, and its last again. The 1.12 client has one spline,
/// a smooth one (vtable `0x7FFD84`), whose first and last points are only
/// there to shape it: it counts the segments as the points less three
/// (`0x454770`, `lea esi, [eax - 3]`), and lays out a move it reads the
/// same way (`0x601B72`: a point behind the start, the path, its last
/// again). Without the point before, the client took the path's first
/// point for it and dropped its first segment (a walk of two points had
/// none); with fewer than three points, it crashed, copying a count of -1
/// (`0x6194DD`).
pub fn create_spline_points(path: &[Vector3d]) -> Vec<Vector3d> {
    let (Some(&first), Some(&last)) = (path.first(), path.last()) else {
        return Vec::new();
    };
    let second = path.get(1).copied().unwrap_or(first);
    let mut out = Vec::with_capacity(path.len() + 2);
    out.push(Vector3d {
        x: first.x * 2.0 - second.x,
        y: first.y * 2.0 - second.y,
        z: first.z * 2.0 - second.z,
    });
    out.extend_from_slice(path);
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
            jump,
            spline_elevation,
            speeds,
            spline,
            transport,
        } => {
            // A jump and a spline elevation if there are any, a spline only
            // if there is one to send, a transport if aboard: its whole
            // GUID and the place and facing on deck (mangos's
            // `MovementInfo::Write`, wow_messages's `MovementBlock`).
            let mut move_flags = *flags
                & !(MOVEFLAG_ON_TRANSPORT
                    | MOVEFLAG_JUMPING
                    | MOVEFLAG_SPLINE_ELEVATION
                    | MOVEFLAG_SPLINE_ENABLED);
            // On a spline, going forward along it, walking unless it runs,
            // as mangos's `MoveSplineInit::Launch` sets a mover's flags
            // (and the client's own move, `0x7C6AE0`, its own). The client
            // moves a unit it is told of only if the create has it going
            // somewhere (`0x618940`: forward, back, sideways, turning,
            // pitching or jumping, 0x20FF); with no more than the spline,
            // it left the unit where the create put it: a gryphon frozen
            // in the air, a creature standing mid-walk.
            if let Some(s) = spline {
                move_flags |= MOVEFLAG_SPLINE_ENABLED | MOVEFLAG_FORWARD;
                if s.flags & SPLINE_RUN_MODE != 0 {
                    move_flags &= !MOVEFLAG_WALK_MODE;
                } else {
                    move_flags |= MOVEFLAG_WALK_MODE;
                }
            }
            if transport.is_some() {
                move_flags |= MOVEFLAG_ON_TRANSPORT;
            }
            if jump.is_some() {
                move_flags |= MOVEFLAG_JUMPING;
            }
            if spline_elevation.is_some() {
                move_flags |= MOVEFLAG_SPLINE_ELEVATION;
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
            if let Some(j) = jump {
                for v in [j.z_speed, j.cos_angle, j.sin_angle, j.xy_speed] {
                    out.extend_from_slice(&v.to_le_bytes());
                }
            }
            if let Some(e) = spline_elevation {
                out.extend_from_slice(&e.to_le_bytes());
            }
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
                // Its final facing, a target over an angle, as its
                // `SMSG_MONSTER_MOVE` has it; never a final point
                // (mangos's `PacketBuilder::WriteCreate`).
                let mut spline_flags = spline.flags & !SPLINE_FINAL_FACING;
                if spline.final_target.is_some() {
                    spline_flags |= SPLINE_FINAL_TARGET;
                } else if spline.final_angle.is_some() {
                    spline_flags |= SPLINE_FINAL_ANGLE;
                }
                out.extend_from_slice(&spline_flags.to_le_bytes());
                if let Some(target) = spline.final_target {
                    out.extend_from_slice(&target.guid().to_le_bytes());
                } else if let Some(angle) = spline.final_angle {
                    out.extend_from_slice(&angle.to_le_bytes());
                }
                out.extend_from_slice(&spline.time_passed.to_le_bytes());
                out.extend_from_slice(&spline.duration.to_le_bytes());
                out.extend_from_slice(&spline.id.to_le_bytes());
                let points = create_spline_points(&spline.nodes);
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

    /// The forgets of an update settled at once, for each object in
    /// `forgets` (each with how many times it was forgotten, in the order
    /// first forgotten): forgotten first of all if the client had it at the
    /// update's start (`known`, now, plus its forgets, less its creates, is
    /// above nothing), and what the batch says of it dropped, all but its
    /// last create if the client is to have it at the end ([`Batch::forget`]
    /// and [`Batch::drop_for`] for each, in one pass over the blocks rather
    /// than three an object: a hearth within a continent forgets and
    /// creates a thousand or more).
    pub fn settle_forgets(&mut self, forgets: &[(Guid, u32)], known: impl Fn(Guid) -> bool) {
        let wanted: ahash::AHashSet<Guid> = forgets.iter().map(|(g, _)| *g).collect();
        let mut spans: ahash::AHashMap<Guid, smallvec::SmallVec<[usize; 2]>> =
            ahash::AHashMap::with_capacity(forgets.len());
        for (i, s) in self.spans.iter().enumerate() {
            if let Some(o) = s.object
                && wanted.contains(&o)
            {
                spans.entry(o).or_default().push(i);
            }
        }
        let mut queued: ahash::AHashSet<Guid> = self.forget.iter().copied().collect();
        for &(g, n) in forgets {
            let of = spans.get(&g).map_or(&[][..], |v| v.as_slice());
            let is_create = |i: usize| self.spans[i].live && self.spans[i].create;
            let creates = of.iter().filter(|&&i| is_create(i)).count() as i64;
            let now = i64::from(known(g));
            if now + i64::from(n) - creates > 0 && queued.insert(g) {
                self.forget.push(g);
            }
            let last = if now == 1 {
                of.iter().rev().copied().find(|&i| is_create(i))
            } else {
                None
            };
            for &i in of {
                if last.is_none_or(|l| i < l) {
                    self.spans[i].live = false;
                }
            }
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
        self.packets()
            .into_iter()
            .map(|p| frame_update(&p.payload))
            .collect()
    }

    /// [`Batch::finish`], each packet framed (and compressed) once for
    /// every observer whose packet is the same this update: `shared` has
    /// the frames built so far, by their packets' content. A crowd told of
    /// a newcomer gets one frame; newcomers to a crowd share the packets
    /// of it they get alike.
    pub fn finish_shared(self, shared: &mut SharedFrames) -> Vec<Arc<[u8]>> {
        self.packets()
            .into_iter()
            .map(|p| shared.frame(p))
            .collect()
    }

    /// The packets' payloads. Past [`CUT_FROM`] bytes a packet may end
    /// after a block whose hash says so (one in [`CUT_ONE_IN`]), and does
    /// at [`PACKET_LIMIT`]: where a packet ends depends on its own blocks,
    /// so two batches with a run of blocks alike cut it alike, and share
    /// those packets, wherever the run starts.
    fn packets(self) -> Vec<Packet> {
        let mut packets = Vec::new();
        let mut open = Packet::default();
        let mut body: Vec<u8> = Vec::new();
        let cut = |open: &mut Packet, body: &mut Vec<u8>, packets: &mut Vec<Packet>| {
            if open.count == 0 {
                return;
            }
            let mut payload = Vec::with_capacity(5 + body.len());
            payload.extend_from_slice(&open.count.to_le_bytes());
            payload.push(u8::from(open.transport));
            payload.extend_from_slice(body);
            body.clear();
            let mut done = std::mem::take(open);
            done.payload = payload;
            packets.push(done);
        };
        // At most 9 bytes a guid: a block that keeps under the limit.
        for guids in self.forget.chunks(PACKET_LIMIT / 9) {
            let block = out_of_range_block(guids);
            open.add(&block, false);
            body.extend_from_slice(&block);
        }
        for s in self.spans.iter().filter(|s| s.live) {
            let block = &self.body[s.start as usize..(s.start + s.len) as usize];
            if !body.is_empty() && body.len() + block.len() > PACKET_LIMIT {
                cut(&mut open, &mut body, &mut packets);
            }
            let h = open.add(block, s.transport);
            body.extend_from_slice(block);
            // Not between a boat and what follows it (the player aboard).
            if body.len() >= CUT_FROM && h % CUT_ONE_IN == 0 && !s.transport {
                cut(&mut open, &mut body, &mut packets);
            }
        }
        cut(&mut open, &mut body, &mut packets);
        packets
    }
}

/// Past this many bytes of blocks, a packet may end after any block
/// ([`Batch::packets`]).
/// Small next to a packet, so that two batches cut alike soon after
/// where they differ.
const CUT_FROM: usize = 2 * 1024;
/// One block in this many ends a packet past [`CUT_FROM`]: about 10 kB
/// of players' creates a packet, few reaching [`PACKET_LIMIT`].
const CUT_ONE_IN: u64 = 16;

/// The hasher of blocks and packets: the same for every map and run.
fn hasher() -> ahash::AHasher {
    use std::hash::BuildHasher;
    ahash::RandomState::with_seeds(
        0x243f_6a88_85a3_08d3,
        0x1319_8a2e_0370_7344,
        0xa409_3822_299f_31d0,
        0x082e_fa98_ec4e_6c89,
    )
    .build_hasher()
}

/// One packet of a batch: its payload, once cut, and the hash of what it
/// carries.
#[derive(Debug, Default)]
struct Packet {
    count: u32,
    transport: bool,
    key: u64,
    payload: Vec<u8>,
}

impl Packet {
    /// A block into the packet: the block's own hash.
    fn add(&mut self, block: &[u8], transport: bool) -> u64 {
        use std::hash::Hasher;
        let mut h = hasher();
        h.write(block);
        let block_hash = h.finish();
        self.key = self.key.rotate_left(5).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ block_hash;
        self.count += 1;
        self.transport |= transport;
        block_hash
    }
}

/// The update's frames, by their packets' content ([`Batch::finish_shared`]):
/// a packet sent to many observers is framed and compressed once.
#[derive(Debug, Default)]
pub struct SharedFrames {
    by_key: ahash::AHashMap<u64, Vec<Shared>>,
    bytes: usize,
    /// Frames shared, and built, since the last [`SharedFrames::clear`].
    pub hits: u64,
    pub built: u64,
}

/// A packet's payload, and its frame.
type Shared = (Vec<u8>, Arc<[u8]>);

/// What [`SharedFrames`] keeps at most before it starts afresh.
const SHARED_LIMIT: usize = 64 * 1024 * 1024;

impl SharedFrames {
    /// Forget every frame: the update's end.
    pub fn clear(&mut self) {
        self.by_key.clear();
        self.bytes = 0;
    }

    fn frame(&mut self, p: Packet) -> Arc<[u8]> {
        if let Some(same) = self
            .by_key
            .get(&p.key)
            .and_then(|v| v.iter().find(|(payload, _)| *payload == p.payload))
        {
            self.hits += 1;
            return same.1.clone();
        }
        self.built += 1;
        let frame = frame_update(&p.payload);
        if self.bytes + p.payload.len() > SHARED_LIMIT {
            self.clear();
        }
        self.bytes += p.payload.len();
        self.by_key
            .entry(p.key)
            .or_default()
            .push((p.payload, frame.clone()));
        frame
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

/// The largest body a 1.12 frame carries: its size field is two bytes and
/// counts the opcode's two as well. wow_messages adds the size field's own
/// two in a `u16` too, so the bound is the same for what it encodes.
pub const MAX_FRAME_BODY: usize = u16::MAX as usize - 4;

/// `[size BE][opcode LE][body]`, the shape the writer task encrypts. A body
/// too big for the size field gives an empty frame, which the writer skips:
/// sent with its size cut short, the client would read the rest of the
/// body as the packets after it, and be out of step from then on.
pub fn frame(opcode: u16, body: &[u8]) -> Arc<[u8]> {
    if body.len() > MAX_FRAME_BODY {
        tracing::warn!(
            "not sending opcode {opcode:#x}: its {} bytes don't fit a frame",
            body.len()
        );
        return Arc::from([]);
    }
    let size = (body.len() + 2) as u16;
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
    use wow_world_messages::vanilla::{
        MovementBlock_MovementFlags, MovementBlock_UpdateFlag_Living, Object_UpdateType, UpdateMask,
    };

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
    /// mangos's whole array, a walk's as a flight's: a point before the
    /// first, the path, its last again, so never fewer than three, and each
    /// of the path's segments the client's (a walk's first was dropped,
    /// the client taking its first point for the one before).
    #[test]
    fn a_create_spline_is_laid_out_as_the_client_reads_it() {
        let (a, b, c) = (v(1.0, 2.0, 3.0), v(4.0, 6.0, 3.0), v(9.0, 9.0, 9.0));
        let before = v(-2.0, -2.0, 3.0);
        assert_eq!(create_spline_points(&[a, b]), vec![before, a, b, b]);
        assert_eq!(create_spline_points(&[a, b, c]), vec![before, a, b, c, c]);
        assert_eq!(create_spline_points(&[a]), vec![a, a, a]);
        assert!(create_spline_points(&[]).is_empty());
        for n in 1..8 {
            let path: Vec<Vector3d> = (0..n).map(|i| v(i as f32, 0.0, 0.0)).collect();
            let points = create_spline_points(&path);
            assert!(points.len() >= 3, "{n}");
            // The client's segments, the points less three: the path's.
            assert_eq!(points.len() - 3, n.max(1) - 1, "{n}");
            assert_eq!(points[1..=n], path[..], "{n}");
        }
        // As the block carries it: the gorilla's four points.
        let g = guid::unit(1557, 2521);
        let fields = unit_fields(g);
        let spline = SplineState {
            flags: 0,
            time_passed: 1675,
            duration: 1768,
            id: 446,
            nodes: vec![v(-13719.4, 249.08, 28.52), v(-13718.58, 253.25, 29.73)],
            final_node: v(-13718.58, 253.25, 29.73),
            final_angle: None,
            final_target: None,
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
                    jump: None,
                    spline_elevation: None,
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
        assert_eq!(count, 4);
    }

    /// A unit's create, with its movement flags `flags` and a spline of
    /// `spline` flags if given, as wow_messages reads its movement flags.
    fn movement_flags_of(flags: u32, spline: Option<u32>) -> MovementBlock_MovementFlags {
        let g = guid::unit(299, 7);
        let fields = unit_fields(g);
        let (from, to) = (v(1.0, 2.0, 3.0), v(31.0, 2.0, 3.0));
        let block = create_block(
            &Create {
                guid: g,
                type_id: TypeId::Unit,
                placement: Placement::Living {
                    flags,
                    timestamp: 0,
                    position: from,
                    orientation: 0.0,
                    pitch: 0.0,
                    fall_time: 0,
                    jump: None,
                    spline_elevation: None,
                    speeds: Speeds::default(),
                    spline: spline.map(|flags| SplineState {
                        flags,
                        time_passed: 100,
                        duration: 4000,
                        id: 9,
                        nodes: vec![from, to],
                        final_node: to,
                        final_angle: None,
                        final_target: None,
                    }),
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
        let ServerOpcodeMessage::SMSG_UPDATE_OBJECT(m) = parse(&batch.finish()[0]) else {
            panic!("not an update object");
        };
        let Object_UpdateType::CreateObject2 { movement2, .. } = &m.objects[0].update_type else {
            panic!("not a create");
        };
        match movement2.update_flag.get_living() {
            Some(MovementBlock_UpdateFlag_Living::Living { flags, .. }) => flags.clone(),
            _ => panic!("not a living block"),
        }
    }

    /// The frozen gryphons of 2026-10-20: a create of a unit on a spline
    /// has it going forward along it, walking unless the spline runs, as
    /// mangos's `MoveSplineInit::Launch` sets a mover's flags; the 1.12
    /// client moves a unit it is told of only if it is going somewhere
    /// (`0x618940`), and left one with the spline alone where the create
    /// put it. Its own flags are kept (a flyer's); one on no spline is as
    /// its flags say.
    #[test]
    fn a_create_on_a_spline_goes_forward_along_it() {
        let flight = movement_flags_of(0, Some(0x300));
        assert!(flight.get_spline_enabled().is_some(), "on the spline");
        assert!(flight.get_forward(), "going forward");
        assert!(!flight.get_walk_mode(), "running");
        let walk = movement_flags_of(0x0180_0000, Some(0));
        assert!(walk.get_spline_enabled().is_some() && walk.get_forward());
        assert!(walk.get_walk_mode(), "walking");
        assert!(
            walk.get_can_fly() && walk.get_flying(),
            "a flyer's own flags kept"
        );
        let ran = movement_flags_of(MOVEFLAG_WALK_MODE, Some(0x100));
        assert!(!ran.get_walk_mode(), "a run is no walk");
        let standing = movement_flags_of(0, None);
        assert!(standing.get_spline_enabled().is_none() && !standing.get_forward());
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
                    jump: None,
                    spline_elevation: None,
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

    /// Every object a batch's packets carry, in order, and the packets'
    /// payload sizes.
    fn objects_of(packets: &[Arc<[u8]>]) -> Vec<Guid> {
        let mut out = Vec::new();
        for p in packets {
            let objects = match parse(p) {
                ServerOpcodeMessage::SMSG_COMPRESSED_UPDATE_OBJECT(m) => m.objects,
                ServerOpcodeMessage::SMSG_UPDATE_OBJECT(m) => m.objects,
                other => panic!("unexpected {other}"),
            };
            for o in objects {
                match o.update_type {
                    Object_UpdateType::CreateObject { guid3, .. }
                    | Object_UpdateType::CreateObject2 { guid3, .. } => out.push(guid3),
                    other => panic!("unexpected {other:?}"),
                }
            }
        }
        out
    }

    /// A crowd's creates, as a batch: units `from..to`.
    fn crowd(from: u32, to: u32) -> (Batch, Vec<Guid>) {
        let mut batch = Batch::new();
        let mut guids = Vec::new();
        for i in from..to {
            let g = guid::unit(299, i);
            guids.push(g);
            batch.push_for(g, &unit_create(g, &unit_fields(g)));
        }
        (batch, guids)
    }

    /// The same packet for many observers is one frame, framed and
    /// compressed once, byte for byte what each would have had alone.
    #[test]
    fn a_packet_many_observers_get_alike_is_one_frame() {
        let mut shared = SharedFrames::default();
        let alone = crowd(0, 40).0.finish();
        let first = crowd(0, 40).0.finish_shared(&mut shared);
        let second = crowd(0, 40).0.finish_shared(&mut shared);
        assert_eq!(first.len(), alone.len());
        for ((a, b), c) in first.iter().zip(&second).zip(&alone) {
            assert!(Arc::ptr_eq(a, b), "the second observer's is the first's");
            assert_eq!(a[..], c[..], "as it would be alone");
        }
        assert_eq!(shared.built, first.len() as u64);
        assert_eq!(shared.hits, first.len() as u64);
        // Another batch is another frame.
        let other = crowd(1, 40).0.finish_shared(&mut shared);
        assert!(!Arc::ptr_eq(&other[0], &first[0]));
        assert_eq!(objects_of(&other).len(), 39);
    }

    /// A newcomer's creates for a crowd: compressed packets, none over the
    /// limit, every create in order; and a second newcomer, whose creates
    /// are the same but for a few of its own ahead of them, shares most
    /// of the first's packets (where a packet ends depends on its blocks,
    /// not on where the batch began).
    #[test]
    fn newcomers_to_a_crowd_share_its_packets() {
        let (one, guids) = crowd(0, 1000);
        let mut shared = SharedFrames::default();
        let first = one.finish_shared(&mut shared);
        assert!(first.len() > 4, "{} packets", first.len());
        assert!(first.iter().all(|p| p.len() <= MAX_FRAME_BODY));
        // Each at least `CUT_FROM`, so compressed, but for the last.
        assert!(
            first[..first.len() - 1]
                .iter()
                .all(|p| u16::from_le_bytes([p[2], p[3]]) == SMSG_COMPRESSED_UPDATE_OBJECT)
        );
        assert_eq!(objects_of(&first), guids);
        let built = shared.built;
        // Three of its own first: another newcomer, and the crowd.
        let (mut two, mut theirs) = crowd(5000, 5003);
        for g in &guids {
            two.push_for(*g, &unit_create(*g, &unit_fields(*g)));
        }
        theirs.extend(&guids);
        let second = two.finish_shared(&mut shared);
        assert_eq!(objects_of(&second), theirs);
        let new = shared.built - built;
        assert!(
            new * 3 <= second.len() as u64,
            "{new} of the second's {} packets built anew",
            second.len()
        );
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
                    jump: None,
                    spline_elevation: None,
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

    /// An update's forgets settled at once, as each was one by one: one
    /// known and lost is forgotten; one lost and made anew, forgotten, then
    /// made once (its last create); one learned of and lost, nothing.
    #[test]
    fn forgets_settle_at_once_as_they_did_one_by_one() {
        let [lost, remade, passing, other] = [1, 2, 3, 4].map(|i| guid::unit(299, i));
        let mut batch = Batch::new();
        for g in [remade, passing, remade, other] {
            batch.push_for(g, &unit_create(g, &unit_fields(g)));
        }
        let forgets = [(lost, 1), (remade, 2), (passing, 1)];
        // Known now: what the client has at the update's end.
        batch.settle_forgets(&forgets, |g| g == remade || g == other);
        assert_eq!(
            kinds(&batch.finish()),
            [
                ("forget", vec![lost, remade]),
                ("create", vec![remade]),
                ("create", vec![other]),
            ]
        );
    }
}
