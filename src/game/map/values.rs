//! The update's values blocks that every one of an object's observers gets
//! alike, sent as packets shared among them rather than copied into each
//! observer's batch (`update::Batch`).
//!
//! Before, an object's public values block went into the batch of every
//! observer, and each batch was cut into packets, hashed and framed on its
//! own: with a crowd of N players in sight of each other and most of them
//! changing every update (health, power, auras in a battle), N² copies and
//! hashes an update (1,600 players: 33 ms of the map's 50).
//!
//! Now each such block is kept once ([`Shared::tell`]) with its audience
//! (the object's observers, as the bits of `conn::Clients`), and at the
//! update's end ([`Shared::plan`]) the blocks are put in packets of their
//! own, each built once and broadcast through the map's feed to everyone
//! who is to have every block in it:
//!
//! - **Chunks.** The blocks, near objects first (by cell), are cut into
//!   chunks of at most `update::PACKET_LIMIT` bytes, each with the clients
//!   that are in the audience of all of its blocks (its *core*). A chunk is
//!   cut short where a block would leave its core much smaller.
//! - **Each player but their own block.** A player isn't among their own
//!   observers (their own fields go to them alone, in their own batch), so
//!   a chunk with a player's block in it isn't for them. The chunk's
//!   players in its core are its *members*: a member gets every block of
//!   the chunk but their own, through a tree over the chunk's blocks in
//!   leaves of [`LEAF`]: each node's packet goes to the members under its
//!   sibling, and a member gets the rest of their own leaf in their batch.
//!   An observer of the chunk outside it gets the whole chunk in one packet.
//! - **The rest** (in the audience of some of a chunk's blocks but not all:
//!   a crowd wider than sight, or one whose sight is still being worked
//!   out) are grouped by which of each [`SEGMENT`] of the chunk's blocks
//!   they get: a packet for each share more than one of them gets, the
//!   blocks of the others in their batches, as before; so do the observers
//!   of a block with fewer than [`SHARE_FROM`] of them (a few players keep
//!   the packets they had).
//!
//! So each client gets the same blocks as before, each once, and only the
//! packets they come in differ: they come after the client's own batch
//! (its forgets and creates, and its own fields), in the update's push. No
//! packet has two blocks of one object, or a block of an object its client
//! hasn't been told of: a block's audience is its object's observers after
//! the update's visibility and forgets.

use std::sync::Arc;

use ahash::AHashMap;
use wow_world_messages::Guid;

use crate::game::conn::Clients;
use crate::game::map::grid::CellKey;
use crate::game::map::observers::ClientTable;
use crate::game::update::{self, Batch};

/// A leaf of a chunk's tree: a member gets the others of its own leaf in
/// its batch.
pub const LEAF: usize = 8;
/// A block for fewer observers than this goes into their batches, as
/// before: sharing pays only in a crowd, and a few players' packets keep
/// the shapes they had (a create and its values in one packet).
pub const SHARE_FROM: usize = 8;
/// A chunk is cut before a block that would leave its core less than this
/// share of what it was ...
const CORE_KEPT: f32 = 0.95;
/// ... once it has this many blocks.
const CUT_AFTER: usize = 16;

/// One object's values block, for its audience alike.
#[derive(Debug)]
struct Told {
    key: (CellKey, Guid),
    /// The object's own client, if a player.
    own: Option<usize>,
    block: Vec<u8>,
    to: Arc<Clients>,
}

/// The update's blocks told alike, kept until [`Shared::plan`].
#[derive(Debug, Default)]
pub struct Shared {
    told: Vec<Told>,
    /// Every block into its observers' batches, as before they were
    /// shared: the benchmark's and the tests' way to compare.
    pub one_by_one: bool,
}

/// What [`Shared::plan`] made: each packet and who gets it, in order.
pub type Packets = Vec<(Arc<[u8]>, Arc<Clients>)>;

impl Shared {
    pub fn is_empty(&self) -> bool {
        self.told.is_empty()
    }

    /// Forget what was told: an update whose values step stopped short (a
    /// panic) leaves nothing for the next to send twice.
    pub fn clear(&mut self) {
        self.told.clear();
    }

    /// `object`'s block, for each of `to` (the clients of its observers);
    /// `own`, the object's own client if it is a player, `cell` where it is.
    pub fn tell(
        &mut self,
        object: Guid,
        own: Option<usize>,
        cell: CellKey,
        block: Vec<u8>,
        to: Arc<Clients>,
    ) {
        if to.is_empty() {
            return;
        }
        self.told.push(Told {
            key: (cell, object),
            own,
            block,
            to,
        });
    }

    /// The packets for what was told, and the blocks for those who get
    /// only some of a packet's into their batches (`batches`, by the guids
    /// of `table`'s clients). Empties what was told.
    pub fn plan(&mut self, table: &ClientTable, batches: &mut AHashMap<Guid, Batch>) -> Packets {
        let mut told = std::mem::take(&mut self.told);
        if told.is_empty() {
            return Vec::new();
        }
        let _z = crate::zone!("values: shared");
        told.sort_unstable_by_key(|t| t.key);
        #[cfg(debug_assertions)]
        {
            let mut objects: Vec<Guid> = told.iter().map(|t| t.key.1).collect();
            objects.sort_unstable();
            debug_assert!(
                objects.windows(2).all(|w| w[0] != w[1]),
                "an object's values were told twice in one update"
            );
        }
        let guids = table.guids();
        let guid_of = |i: usize| guids.get(i).copied().flatten();
        // Those for a few: into their batches, as before.
        let one_by_one = self.one_by_one;
        told.retain(|t| {
            if !one_by_one && t.to.len() >= SHARE_FROM {
                return true;
            }
            for i in t.to.iter() {
                if let Some(g) = guid_of(i) {
                    batches.entry(g).or_default().push(&t.block);
                }
            }
            false
        });
        let mut out = Vec::new();
        let mut patterns = Patterns::new(guids.len());
        let mut start = 0;
        while start < told.len() {
            let (end, core) = chunk(&told, start);
            let chunk = &told[start..end];
            // Those in some of its audiences but not in its core: theirs by
            // what of it each gets, a packet for each such share that more
            // than one gets, the rest into their batches.
            for seg in chunk.chunks(SEGMENT) {
                for (pattern, who) in patterns.of(seg, &core) {
                    if who.len() >= 2 {
                        let mut to = Clients::default();
                        for &i in &who {
                            to.insert(i);
                        }
                        let blocks = (0..seg.len())
                            .filter(|k| pattern & (1 << k) != 0)
                            .map(|k| seg[k].block.as_slice());
                        out.push((update::frame_blocks(blocks), Arc::new(to)));
                        continue;
                    }
                    let Some(g) = who.first().and_then(|&i| guid_of(i)) else {
                        continue;
                    };
                    let batch = batches.entry(g).or_default();
                    for (k, t) in seg.iter().enumerate() {
                        if pattern & (1 << k) != 0 {
                            batch.push(&t.block);
                        }
                    }
                }
            }
            // Its members: the players of its blocks in its core.
            let members: Vec<Option<usize>> = chunk
                .iter()
                .map(|t| t.own.filter(|&i| core.contains(i)))
                .collect();
            let mut inside = Clients::default();
            for &m in members.iter().flatten() {
                inside.insert(m);
            }
            let mut outside = core.clone();
            outside.take_all(&inside);
            if !outside.is_empty() {
                out.push((frame(chunk), Arc::new(outside)));
            }
            if !inside.is_empty() {
                tree(chunk, &members, 0, chunk.len(), &mut out, &mut |m, t| {
                    if let Some(g) = guid_of(m) {
                        batches.entry(g).or_default().push(&t.block);
                    }
                });
            }
            start = end;
        }
        out
    }
}

/// What of a chunk outside its core is shared by its pattern: this many
/// blocks at a time, a bit each.
const SEGMENT: usize = 64;

/// Each client's pattern over a segment of a chunk (which of its blocks
/// it gets), worked out in a table by client kept between segments.
struct Patterns {
    of: Vec<u64>,
    touched: Vec<usize>,
}

impl Patterns {
    fn new(clients: usize) -> Self {
        Self {
            of: vec![0; clients],
            touched: Vec::new(),
        }
    }

    /// The clients outside `core` of each pattern over `seg` (at most
    /// [`SEGMENT`] blocks), patterns in order.
    fn of(&mut self, seg: &[Told], core: &Clients) -> Vec<(u64, Vec<usize>)> {
        debug_assert!(seg.len() <= SEGMENT);
        for (k, t) in seg.iter().enumerate() {
            for i in t.to.iter_without(core) {
                if i >= self.of.len() {
                    self.of.resize(i + 1, 0);
                }
                if self.of[i] == 0 {
                    self.touched.push(i);
                }
                self.of[i] |= 1 << k;
            }
        }
        let mut by: AHashMap<u64, Vec<usize>> = AHashMap::new();
        for i in self.touched.drain(..) {
            by.entry(std::mem::take(&mut self.of[i]))
                .or_default()
                .push(i);
        }
        let mut out: Vec<(u64, Vec<usize>)> = by.into_iter().collect();
        out.sort_unstable_by_key(|(p, _)| *p);
        out
    }
}

/// The chunk from `start`: where it ends, and its core (those in the
/// audience of every block of it, or the block's own player).
fn chunk(told: &[Told], start: usize) -> (usize, Clients) {
    let first = &told[start];
    let mut core = (*first.to).clone();
    if let Some(i) = first.own {
        core.insert(i);
    }
    let mut bytes = first.block.len();
    let mut end = start + 1;
    while end < told.len() {
        let t = &told[end];
        if bytes + t.block.len() > update::PACKET_LIMIT {
            break;
        }
        if end - start >= CUT_AFTER {
            let before = core.len();
            let after = core.common(&t.to, t.own);
            if (after as f32) < CORE_KEPT * before as f32 {
                break;
            }
        }
        core.keep_common(&t.to, t.own);
        bytes += t.block.len();
        end += 1;
    }
    (end, core)
}

fn frame(told: &[Told]) -> Arc<[u8]> {
    update::frame_blocks(told.iter().map(|t| t.block.as_slice()))
}

/// The members of `chunk[lo..hi]`, each sent what of it isn't theirs: at a
/// leaf, the others of the leaf (`leaf`); above, each half's packet to the
/// other half's members. Returns the members under it.
fn tree(
    chunk: &[Told],
    members: &[Option<usize>],
    lo: usize,
    hi: usize,
    out: &mut Packets,
    leaf: &mut impl FnMut(usize, &Told),
) -> Clients {
    let mut under = Clients::default();
    if hi - lo <= LEAF {
        for (k, m) in members[lo..hi].iter().enumerate() {
            let Some(m) = *m else { continue };
            under.insert(m);
            for (j, t) in chunk[lo..hi].iter().enumerate() {
                if j != k {
                    leaf(m, t);
                }
            }
        }
        return under;
    }
    // Halves of whole leaves.
    let leaves = (hi - lo).div_ceil(LEAF);
    let mid = lo + leaves.div_ceil(2) * LEAF;
    let left = tree(chunk, members, lo, mid, out, leaf);
    let right = tree(chunk, members, mid, hi, out, leaf);
    if !right.is_empty() {
        out.push((frame(&chunk[lo..mid]), Arc::new(right.clone())));
    }
    if !left.is_empty() {
        out.push((frame(&chunk[mid..hi]), Arc::new(left.clone())));
    }
    under = left;
    under.add_all(&right);
    under
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::guid;

    /// What each client gets, by guid: the objects whose blocks reach it,
    /// through the packets (by the object each block names) and its batch.
    fn received(
        told: &[(u32, Option<usize>, Vec<usize>)],
        clients: usize,
    ) -> (Vec<Vec<u32>>, Vec<Vec<u32>>, usize) {
        let table = ClientTable::default();
        for i in 0..clients {
            assert_eq!(table.index_of(guid::player(1000 + i as u32)), i);
        }
        let mut shared = Shared::default();
        for (object, own, to) in told {
            let mut c = Clients::default();
            for &i in to {
                c.insert(i);
            }
            // A block that names its object: a values block's first byte,
            // then the id.
            let mut block = vec![0u8];
            block.extend_from_slice(&object.to_le_bytes());
            shared.tell(
                guid::player(*object),
                *own,
                (0, (*object % 3) as i32),
                block,
                Arc::new(c),
            );
        }
        let mut batches = AHashMap::new();
        let packets = shared.plan(&table, &mut batches);
        let mut got = vec![Vec::new(); clients];
        let mut in_packets = vec![Vec::new(); clients];
        for (frame, to) in &packets {
            let body = payload(frame);
            let count = u32::from_le_bytes(body[..4].try_into().unwrap()) as usize;
            let objects: Vec<u32> = body[5..]
                .chunks(5)
                .map(|b| u32::from_le_bytes(b[1..5].try_into().unwrap()))
                .collect();
            assert_eq!(objects.len(), count);
            for i in to.iter() {
                got[i].extend(&objects);
                in_packets[i].push(objects.clone());
            }
        }
        for (g, b) in batches {
            let i = (g.guid() as u32 - 1000) as usize;
            for frame in b.finish() {
                let body = payload(&frame);
                for chunk in body[5..].chunks(5) {
                    got[i].push(u32::from_le_bytes(chunk[1..5].try_into().unwrap()));
                }
            }
        }
        for g in &mut got {
            g.sort_unstable();
        }
        // No packet carries one object twice.
        for packets in &in_packets {
            for p in packets {
                let mut s = p.clone();
                s.sort_unstable();
                s.dedup();
                assert_eq!(s.len(), p.len());
            }
        }
        let frames = in_packets.iter().map(Vec::len).max().unwrap_or(0);
        (
            got,
            in_packets.into_iter().map(|p| p.concat()).collect(),
            frames,
        )
    }

    /// An update packet's payload, inflated if it came compressed.
    fn payload(frame: &[u8]) -> Vec<u8> {
        use std::io::Read;
        let opcode = u16::from_le_bytes([frame[2], frame[3]]);
        if opcode == 0x0A9 {
            return frame[4..].to_vec();
        }
        assert_eq!(opcode, 0x1F6);
        let mut out = Vec::new();
        flate2::read::ZlibDecoder::new(&frame[8..])
            .read_to_end(&mut out)
            .expect("inflates");
        out
    }

    fn expected(told: &[(u32, Option<usize>, Vec<usize>)], clients: usize) -> Vec<Vec<u32>> {
        let mut want = vec![Vec::new(); clients];
        for (object, _, to) in told {
            for &i in to {
                want[i].push(*object);
            }
        }
        for w in &mut want {
            w.sort_unstable();
        }
        want
    }

    /// A crowd of players who all see each other (each but themselves):
    /// every client gets every other's block once, and none its own; the
    /// blocks go in a few packets each, not one each, and little goes
    /// into batches.
    #[test]
    fn a_crowd_gets_every_block_but_its_own_once() {
        for n in [1usize, 2, 7, 8, 9, 40, 200, 1000] {
            let told: Vec<_> = (0..n)
                .map(|i| {
                    let to = (0..n).filter(|&j| j != i).collect();
                    (i as u32, Some(i), to)
                })
                .collect();
            let (got, _, frames) = received(&told, n);
            assert_eq!(got, expected(&told, n), "{n}");
            // About the tree's depth, plus the leaf's rest in the batch.
            assert!(
                frames <= 2 + (n.div_ceil(LEAF).max(1) as f32).log2().ceil() as usize,
                "{n}: {frames}"
            );
        }
    }

    /// Audiences that differ (a crowd at the edge of sight, some not seen
    /// by some, creatures with no client of their own): each client gets
    /// exactly the blocks of the objects it observes, each once.
    #[test]
    fn audiences_that_differ_get_their_own_blocks_each_once() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for round in 0..40 {
            let clients = 1 + (next() % 300) as usize;
            let objects = (next() % 400) as usize;
            let told: Vec<_> = (0..objects)
                .map(|k| {
                    // Players (with a client of their own) and others.
                    let own = (k < clients && next() % 4 != 0).then_some(k);
                    let miss = next() % 5;
                    let to = (0..clients)
                        .filter(|&j| Some(j) != own && next() % 16 >= miss)
                        .collect();
                    (k as u32, own, to)
                })
                .collect();
            let (got, _, _) = received(&told, clients);
            assert_eq!(got, expected(&told, clients), "round {round}");
        }
    }
}
