//! Cell geometry — the unit of tick-loop sharding.
//!
//! Each `CellKey` identifies a contiguous square of the world: a
//! `[network] cell_size_in_grid_cells × cell_size_in_grid_cells` block of the
//! creature spatial grid (`CREATURE_GRID_CELL_YD = 250 yd`), giving
//! 1000-yd cells at the default `cell_size_in_grid_cells = 4`.
//!
//! A player's owning cell is the one whose square contains their
//! current position. As the player walks across a cell boundary the
//! per-tick transition pass hands them off to the new cell.
//!
//! AOI broadcasts can straddle cell boundaries — a player anchored
//! within `AOI_RADIUS` of a cell edge needs their broadcast
//! delivered to the neighboring cell(s) too. [`cells_within_aoi`]
//! returns the full set of cells whose interior overlaps the AOI
//! disc around an anchor (including the anchor's own cell).
//!
//! This module also exposes the message types that cross cell
//! boundaries: [`CrossCellMsg`] (fan-out frames from a neighbor) and
//! [`CellInbox`] / [`RoutingTable`] — used by `world::world` when
//! cells actually run on separate tokio tasks.

use arc_swap::ArcSwap;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use wow_world_base::vanilla::Map;
use wow_world_messages::vanilla::Vector3d;
use wow_world_messages::Guid;

/// Width (and height) of one creature spatial-grid cell, in yards.
/// Defined here (mirroring `crate::world::world::CREATURE_GRID_CELL_YD`)
/// so this module is self-contained and importable from anywhere
/// without dragging in `world::world`. Keep in sync.
pub const GRID_CELL_YD: f32 = 250.0;

/// Returns the cell edge length in yards, derived from
/// `config().network.cell_size_in_grid_cells × GRID_CELL_YD`.
#[inline]
pub fn cell_size_yd() -> f32 {
    crate::config::config().network.cell_size_in_grid_cells.max(1) as f32 * GRID_CELL_YD
}

/// Identifier for a cell. Two cells are equal iff their `(map,
/// cx, cy)` triple matches. `Hash` so it works as an `AHashMap` key.
#[derive(Hash, Eq, PartialEq, Copy, Clone, Debug)]
pub struct CellKey {
    pub map: Map,
    pub cx: i32,
    pub cy: i32,
}

impl std::fmt::Display for CellKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Two-letter map prefix keeps the slow-tick log column narrow.
        // `(0,0)` is the "common" cell for default-spawn content.
        write!(f, "{:?}({},{})", self.map, self.cx, self.cy)
    }
}

impl CellKey {
    /// Compute the cell containing position `(x, y)` on `map`.
    ///
    /// Uses `floor`, not `as i32` truncation, so positions on the
    /// negative side of the origin map to the correct cell. (`as i32`
    /// truncates toward zero, so `-0.01 / 1000.0 as i32` would give 0
    /// instead of -1 — same footgun guarded by [`grid_cell_for`] in
    /// `world/mod.rs`.)
    #[inline]
    pub fn from_position(map: Map, x: f32, y: f32) -> Self {
        let size = cell_size_yd();
        Self {
            map,
            cx: (x / size).floor() as i32,
            cy: (y / size).floor() as i32,
        }
    }

    /// Inclusive AABB of this cell in world-yard coords:
    /// returns `(x_min, y_min, x_max, y_max)`.
    #[inline]
    pub fn bounds(&self) -> (f32, f32, f32, f32) {
        let size = cell_size_yd();
        let x_min = self.cx as f32 * size;
        let y_min = self.cy as f32 * size;
        (x_min, y_min, x_min + size, y_min + size)
    }
}

/// Return every cell whose interior intersects the AOI disc of
/// radius `aoi_r` around `anchor`. Always includes the anchor's own
/// cell; appends up to 8 neighbors (4 edges + 4 corners). In
/// practice an anchor deep in its cell returns just self; anchors
/// near an edge return 2; anchors near a corner return up to 4.
///
/// Output is unordered. Capped at 9 entries — bigger AOI radii than
/// cell size are nonsensical and caught by the caller as a config
/// error (would mean every broadcast hits every cell, defeating
/// the purpose).
///
/// Allocation: a small `Vec` is fine. At hundreds of broadcasts per
/// tick × ~30 % cross-cell rate, that's a few hundred small allocs
/// per tick — well within mimalloc's pool sweet spot. If Tracy ever
/// flags this we can switch to a stack array or smallvec.
pub fn cells_within_aoi(anchor: Vector3d, anchor_map: Map, aoi_r: f32) -> Vec<CellKey> {
    let self_cell = CellKey::from_position(anchor_map, anchor.x, anchor.y);
    let (x_min, y_min, x_max, y_max) = self_cell.bounds();

    // Distance from anchor to each of the four edges of its cell.
    // Negative would mean "outside the cell" which shouldn't happen
    // by construction (self_cell was computed FROM anchor).
    let to_west = anchor.x - x_min;
    let to_east = x_max - anchor.x;
    let to_south = anchor.y - y_min;
    let to_north = y_max - anchor.y;

    let touches_west = to_west < aoi_r;
    let touches_east = to_east < aoi_r;
    let touches_south = to_south < aoi_r;
    let touches_north = to_north < aoi_r;

    let mut out = Vec::with_capacity(4);
    out.push(self_cell);

    if touches_west {
        out.push(CellKey { map: anchor_map, cx: self_cell.cx - 1, cy: self_cell.cy });
    }
    if touches_east {
        out.push(CellKey { map: anchor_map, cx: self_cell.cx + 1, cy: self_cell.cy });
    }
    if touches_south {
        out.push(CellKey { map: anchor_map, cx: self_cell.cx, cy: self_cell.cy - 1 });
    }
    if touches_north {
        out.push(CellKey { map: anchor_map, cx: self_cell.cx, cy: self_cell.cy + 1 });
    }
    // Corner: a corner-neighbor only matters if the AOI disc actually
    // reaches the corner (Euclidean distance), not just both edges.
    // Without this check we'd over-deliver to a cell 1.4× the AOI
    // radius away on the diagonal — wasted work but not incorrect.
    let aoi_sq = aoi_r * aoi_r;
    let corner = |dx_yd: f32, dy_yd: f32| dx_yd * dx_yd + dy_yd * dy_yd < aoi_sq;
    if touches_west && touches_south && corner(to_west, to_south) {
        out.push(CellKey { map: anchor_map, cx: self_cell.cx - 1, cy: self_cell.cy - 1 });
    }
    if touches_east && touches_south && corner(to_east, to_south) {
        out.push(CellKey { map: anchor_map, cx: self_cell.cx + 1, cy: self_cell.cy - 1 });
    }
    if touches_west && touches_north && corner(to_west, to_north) {
        out.push(CellKey { map: anchor_map, cx: self_cell.cx - 1, cy: self_cell.cy + 1 });
    }
    if touches_east && touches_north && corner(to_east, to_north) {
        out.push(CellKey { map: anchor_map, cx: self_cell.cx + 1, cy: self_cell.cy + 1 });
    }
    out
}

/// Bucket a slab of creatures into per-cell groups by
/// [`CellKey::from_position`]. Used by `World::with_creatures` (and
/// `for_test`) to partition the worlddb's full creature slab into
/// per-`CellState` slabs at startup.
///
/// Pure function: drains the input slab into `Vec`s keyed by cell.
/// `creature.map` + `creature.info.position` determine the bucket. The
/// caller is responsible for re-indexing each bucket into per-cell
/// `Slab<Creature>` + `creature_by_guid` + `creature_grid_cells` data
/// structures.
pub fn partition_creatures(
    slab: slab::Slab<crate::world::world_opcode_handler::creature::Creature>,
) -> ahash::AHashMap<
    CellKey,
    Vec<crate::world::world_opcode_handler::creature::Creature>,
> {
    let mut out: ahash::AHashMap<
        CellKey,
        Vec<crate::world::world_opcode_handler::creature::Creature>,
    > = ahash::AHashMap::new();
    for (_, creature) in slab {
        let key = CellKey::from_position(
            creature.map,
            creature.info.position.x,
            creature.info.position.y,
        );
        out.entry(key).or_default().push(creature);
    }
    out
}

/// A pre-serialized broadcast frame headed to a neighbor cell.
///
/// Built by the originating cell during its broadcast phase: the
/// `Arc<[u8]>` is the same one produced by the local serialize step,
/// so the cross-cell delivery is just an `Arc::clone` + an mpsc send.
/// The receiving cell's tick drains its inbox at the top of its
/// broadcast phase and fans out to local clients within AOI of
/// `anchor`.
#[derive(Debug, Clone)]
pub struct CrossCellFrame {
    /// Anchor for the AOI filter on the receiving side.
    pub anchor: Vector3d,
    pub anchor_map: Map,
    /// Guid to exclude from delivery (typically the source player so
    /// they don't receive their own movement opcode back).
    pub exclude_guid: Option<Guid>,
    /// Pre-serialized wire frame (size-prefixed opcode body).
    pub frame: Arc<[u8]>,
    /// Length of `frame` — cached so the writer doesn't reach into
    /// the `Arc` for `.len()` on every recipient.
    pub frame_bytes: usize,
}

/// A cross-cell state-change request. Generated when a handler in
/// cell A applies an `Effect` to a guid that lives in cell B; the
/// effect is dispatched through the same routing table as broadcast
/// frames and applied during B's next tick.
///
/// Lag: target sees the effect on B's next tick (~33 ms at 30 Hz).
/// Acceptable for visible mechanics (root, damage); for tighter
/// timing a synchronous cross-cell lock path would be needed.
#[derive(Debug, Clone)]
pub struct CrossCellEffect {
    pub target_guid: Guid,
    pub effect: crate::world::command::UnitEffect,
}

/// Inbound message to a cell. `Frame` is broadcast fan-out (movement,
/// spell visuals, etc.). `Effect` is a state-change request the
/// receiving cell applies to one of its own entities.
#[derive(Debug, Clone)]
pub enum CrossCellMsg {
    Frame(CrossCellFrame),
    Effect(CrossCellEffect),
}

/// Send half of a cell's inbox. Cloned into the routing table and
/// kept on every neighbor's outbound path.
#[derive(Clone, Debug)]
pub struct CellInbox {
    pub cross_cell_tx: kanal::AsyncSender<CrossCellMsg>,
}

/// Lookup table from `CellKey` to its inbox. Wrapped in `ArcSwap`
/// at the call site so broadcast hot paths get lock-free reads and
/// the rare cell-spinup case is a copy-on-write swap.
#[derive(Default, Debug)]
pub struct RoutingTable {
    pub inboxes: ahash::AHashMap<CellKey, CellInbox>,
}

impl RoutingTable {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Process-wide routing table. Hot-path readers (every broadcast hits
/// this) get lock-free `Arc<RoutingTable>` reads via `ArcSwap::load`.
/// Writers (cell spin-up / spin-down) build a fresh table and swap.
///
/// Returns a singleton initialized with an empty routing table on first
/// access — until Stage 3 partition lands there are no inboxes, so the
/// cross-cell post-fanout step in `aoi::broadcast_opcode_within_aoi`
/// finds zero neighbors and is a no-op.
pub fn routing() -> &'static ArcSwap<RoutingTable> {
    static ROUTING: OnceLock<ArcSwap<RoutingTable>> = OnceLock::new();
    ROUTING.get_or_init(|| ArcSwap::from_pointee(RoutingTable::new()))
}

/// Replace the global routing table. Old table is dropped once all
/// in-flight broadcast loads release their `Arc<RoutingTable>` —
/// `arc-swap` handles the grace period.
pub fn install_routing(table: RoutingTable) {
    routing().store(Arc::new(table));
}

// ── Cross-cell broadcast metrics ──
//
// Sampled and zeroed once per global tick. Emitted as Tracy plots so
// the dashboard shows how much of the broadcast traffic is paying the
// cross-cell channel cost. Until partition lands these stay at 0
// because the routing table holds no neighbor inboxes.

/// Bumped each time `cells_within_aoi` returns a non-self neighbor
/// that we successfully `try_send` a frame to. Counts SENDS, not
/// recipients on the neighbor side.
pub static CROSS_CELL_EMITTED: AtomicU64 = AtomicU64::new(0);

/// Bumped when a `try_send` to a neighbor's inbox fails — typically
/// because the receiver is overloaded (full unbounded channel is
/// impossible, but a bounded version would track this; useful
/// scaffold).
pub static CROSS_CELL_DROPPED: AtomicU64 = AtomicU64::new(0);

/// Bumped per `CrossCellMsg` the receiving cell drains from its
/// inbox during its broadcast phase. Populated by the receiver, not
/// the sender — pairs with `CROSS_CELL_EMITTED` to show transport
/// throughput and any backlog.
pub static CROSS_CELL_DRAINED: AtomicU64 = AtomicU64::new(0);

/// Per-cell pacer state snapshot. Updated by each per-cell tokio
/// task at the end of its tick; read by the `.cells` GM command.
/// Kept here (rather than reaching into `World::cells` from the GM
/// handler) so the handler signature doesn't need to grow another
/// parameter and so reads are O(1) under a brief Mutex lock.
#[derive(Debug, Clone)]
pub struct PacerSnapshot {
    pub current_interval_ms: u64,
    pub slow_ema: f32,
    pub healthy_streak: u32,
    pub last_tick_ms: u64,
}

/// Process-wide map of per-cell pacer state. The per-cell task
/// writes its key at the end of every tick; the `.cells` GM command
/// reads it. Locked briefly only — no contention with the hot tick
/// path.
pub static PACER_STATES: std::sync::LazyLock<
    std::sync::Mutex<ahash::AHashMap<CellKey, PacerSnapshot>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(ahash::AHashMap::new()));

/// Convenience: publish a fresh snapshot for `key`. Called by the
/// per-cell task; cheap (allocates only on first call per cell).
pub fn publish_pacer_state(key: CellKey, snap: PacerSnapshot) {
    if let Ok(mut guard) = PACER_STATES.lock() {
        guard.insert(key, snap);
    }
}

/// Process-wide registry of in-world player positions. Maintained by
/// every `CellState::insert_client` / `remove_client` so the GM
/// command parser can answer ".go PlayerName" cross-cell without
/// having to lock every cell's mutex from the GM's cell.
///
/// Entries are best-effort: a player mid-transition (left cell A
/// but not yet admitted to B) may briefly be absent. The `.go`
/// command surfaces this as "Unable to find player 'X'", which is the
/// natural error today as well.
#[derive(Debug, Clone)]
pub struct PlayerRegistryEntry {
    pub guid: Guid,
    pub name: String,
    pub map: Map,
    pub position: Vector3d,
    pub orientation: f32,
}

pub static PLAYER_REGISTRY: std::sync::LazyLock<
    std::sync::Mutex<ahash::AHashMap<Guid, PlayerRegistryEntry>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(ahash::AHashMap::new()));

/// Lowercase-name → guid index. Updated in lockstep with
/// [`PLAYER_REGISTRY`] so `.go PlayerName` (by name) is O(1).
pub static PLAYER_NAME_INDEX: std::sync::LazyLock<
    std::sync::Mutex<ahash::AHashMap<String, Guid>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(ahash::AHashMap::new()));

/// Register or refresh a player's entry in both indexes. Called by
/// `CellState::insert_client` after the slab insert succeeds.
pub fn register_player(entry: PlayerRegistryEntry) {
    let name_lc = entry.name.to_lowercase();
    let guid = entry.guid;
    if let Ok(mut reg) = PLAYER_REGISTRY.lock() {
        reg.insert(guid, entry);
    }
    if let Ok(mut idx) = PLAYER_NAME_INDEX.lock() {
        idx.insert(name_lc, guid);
    }
}

/// Drop a player from both indexes. Called by
/// `CellState::remove_client`. No-op if the player isn't registered
/// (e.g. if the caller forgot to register them on insert — defensive,
/// not load-bearing).
pub fn unregister_player(guid: Guid) {
    let name_lc = {
        if let Ok(mut reg) = PLAYER_REGISTRY.lock() {
            reg.remove(&guid).map(|e| e.name.to_lowercase())
        } else {
            None
        }
    };
    if let Some(name_lc) = name_lc
        && let Ok(mut idx) = PLAYER_NAME_INDEX.lock()
    {
        idx.remove(&name_lc);
    }
}

/// Look up a player's position by guid. Used by `.go` (no args, with
/// selected target) when the target isn't in the GM's local cell.
pub fn lookup_player_position(guid: Guid) -> Option<(Map, Vector3d, f32)> {
    let reg = PLAYER_REGISTRY.lock().ok()?;
    reg.get(&guid).map(|e| (e.map, e.position, e.orientation))
}

/// Look up a player's position by case-insensitive name. Used by
/// `.go PlayerName`.
pub fn lookup_player_position_by_name(name: &str) -> Option<(Map, Vector3d, f32)> {
    let name_lc = name.to_lowercase();
    let guid = {
        let idx = PLAYER_NAME_INDEX.lock().ok()?;
        idx.get(&name_lc).copied()?
    };
    lookup_player_position(guid)
}

/// Atomic load-and-zero — pattern used by the global tick to publish
/// the per-tick rate to Tracy. Single relaxed read+store; no fences
/// are needed because the counter is a coarse-grained sample.
#[inline]
pub fn drain_counter(c: &AtomicU64) -> u64 {
    c.swap(0, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: f32, y: f32) -> Vector3d {
        Vector3d { x, y, z: 0.0 }
    }

    // Tests rely on default `cell_size_in_grid_cells = 4` → 1000-yd cell
    // and `aoi_radius_yards = 200`. If those defaults shift, update
    // these expectations.
    const CELL: f32 = 1000.0;
    const AOI: f32 = 200.0;

    #[test]
    fn cell_of_origin_is_zero() {
        let r = CellKey::from_position(Map::EasternKingdoms, 0.0, 0.0);
        assert_eq!((r.cx, r.cy), (0, 0));
    }

    #[test]
    fn cell_of_positive_inside_first() {
        let r = CellKey::from_position(Map::EasternKingdoms, CELL - 0.01, CELL - 0.01);
        assert_eq!((r.cx, r.cy), (0, 0));
    }

    #[test]
    fn cell_of_boundary_promotes_to_next() {
        // Exactly on the boundary lands in the higher cell —
        // floor(1000/1000) = 1.
        let r = CellKey::from_position(Map::EasternKingdoms, CELL, 0.0);
        assert_eq!(r.cx, 1);
    }

    #[test]
    fn cell_of_small_negative_lands_minus_one() {
        // Regression guard against the floor-vs-truncate trap that
        // `as i32` alone would fall into.
        let r = CellKey::from_position(Map::EasternKingdoms, -0.01, -0.01);
        assert_eq!((r.cx, r.cy), (-1, -1));
    }

    #[test]
    fn cell_of_preserves_map() {
        let r = CellKey::from_position(Map::Kalimdor, 100.0, 200.0);
        assert_eq!(r.map, Map::Kalimdor);
    }

    #[test]
    fn bounds_round_trip() {
        let r = CellKey { map: Map::EasternKingdoms, cx: 3, cy: -2 };
        let (x_min, y_min, x_max, y_max) = r.bounds();
        assert_eq!((x_min, y_min, x_max, y_max), (3000.0, -2000.0, 4000.0, -1000.0));
    }

    #[test]
    fn aoi_anchor_deep_in_cell_returns_only_self() {
        // Anchor at the center of cell (0,0). AOI=200, distance to
        // nearest edge is 500 — well outside, no neighbors triggered.
        let out = cells_within_aoi(v(500.0, 500.0), Map::EasternKingdoms, AOI);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], CellKey { map: Map::EasternKingdoms, cx: 0, cy: 0 });
    }

    #[test]
    fn aoi_anchor_near_west_edge_picks_up_west_neighbor() {
        // 100 yd from west edge of cell (0,0). AOI=200 reaches into
        // (-1,0). No corner neighbors.
        let out = cells_within_aoi(v(100.0, 500.0), Map::EasternKingdoms, AOI);
        assert_eq!(out.len(), 2);
        assert!(out.contains(&CellKey { map: Map::EasternKingdoms, cx: 0, cy: 0 }));
        assert!(out.contains(&CellKey { map: Map::EasternKingdoms, cx: -1, cy: 0 }));
    }

    #[test]
    fn aoi_anchor_near_two_edges_no_corner_when_disc_misses() {
        // 150 yd from south edge AND 150 yd from west edge.
        // Manhattan distance to the SW corner is 300 yd; Euclidean
        // ~212 yd > 200 → corner neighbor is NOT included.
        let out = cells_within_aoi(v(150.0, 150.0), Map::EasternKingdoms, AOI);
        assert_eq!(out.len(), 3);
        assert!(out.contains(&CellKey { map: Map::EasternKingdoms, cx: 0, cy: 0 }));
        assert!(out.contains(&CellKey { map: Map::EasternKingdoms, cx: -1, cy: 0 }));
        assert!(out.contains(&CellKey { map: Map::EasternKingdoms, cx: 0, cy: -1 }));
        assert!(!out.contains(&CellKey { map: Map::EasternKingdoms, cx: -1, cy: -1 }));
    }

    #[test]
    fn aoi_anchor_in_corner_picks_up_diagonal_neighbor() {
        // 50 yd from south edge AND 50 yd from west edge.
        // Corner is ~70.7 yd Euclidean → inside AOI → diagonal
        // neighbor is included.
        let out = cells_within_aoi(v(50.0, 50.0), Map::EasternKingdoms, AOI);
        assert_eq!(out.len(), 4);
        assert!(out.contains(&CellKey { map: Map::EasternKingdoms, cx: 0, cy: 0 }));
        assert!(out.contains(&CellKey { map: Map::EasternKingdoms, cx: -1, cy: 0 }));
        assert!(out.contains(&CellKey { map: Map::EasternKingdoms, cx: 0, cy: -1 }));
        assert!(out.contains(&CellKey { map: Map::EasternKingdoms, cx: -1, cy: -1 }));
    }

    #[test]
    fn aoi_at_negative_coords() {
        // Anchor at (-50, -50) lives in cell (-1, -1). Anchor is
        // 50 yd from EAST and NORTH edges of that cell (since the
        // cell spans -1000..0). NE corner of (-1,-1) is at (0,0);
        // distance ~70.7 < 200 → NE diagonal included.
        let out = cells_within_aoi(v(-50.0, -50.0), Map::EasternKingdoms, AOI);
        assert!(out.contains(&CellKey { map: Map::EasternKingdoms, cx: -1, cy: -1 }));
        assert!(out.contains(&CellKey { map: Map::EasternKingdoms, cx: 0, cy: -1 }));
        assert!(out.contains(&CellKey { map: Map::EasternKingdoms, cx: -1, cy: 0 }));
        assert!(out.contains(&CellKey { map: Map::EasternKingdoms, cx: 0, cy: 0 }));
        assert_eq!(out.len(), 4);
    }

    // ── Tests for partition_creatures (Stage 5 partition TDD) ──

    use crate::world::world_opcode_handler::creature::Creature;
    use slab::Slab;
    use wow_world_messages::Guid;

    /// Build a `Creature` at the given map + (x, y) position with a
    /// unique guid. Used by the partition tests below.
    fn make_creature_at(guid_int: u64, map: Map, x: f32, y: f32) -> Creature {
        let mut c = Creature::new(format!("test_{guid_int}"), Guid::new(guid_int));
        c.map = map;
        c.info.position = Vector3d { x, y, z: 0.0 };
        c
    }

    #[test]
    fn partition_creatures_buckets_by_position() {
        // Three creatures in three distinct CELL (1000-yd) buckets on
        // EasternKingdoms: (0,0), (1,0), (0,1). After partition each
        // bucket should hold exactly its one creature.
        let mut slab: Slab<Creature> = Slab::new();
        slab.insert(make_creature_at(1, Map::EasternKingdoms, 100.0, 100.0));
        slab.insert(make_creature_at(2, Map::EasternKingdoms, 1500.0, 100.0));
        slab.insert(make_creature_at(3, Map::EasternKingdoms, 100.0, 1500.0));

        let buckets = partition_creatures(slab);

        assert_eq!(buckets.len(), 3);
        let r00 = CellKey { map: Map::EasternKingdoms, cx: 0, cy: 0 };
        let r10 = CellKey { map: Map::EasternKingdoms, cx: 1, cy: 0 };
        let r01 = CellKey { map: Map::EasternKingdoms, cx: 0, cy: 1 };
        assert_eq!(buckets.get(&r00).map(Vec::len), Some(1));
        assert_eq!(buckets.get(&r10).map(Vec::len), Some(1));
        assert_eq!(buckets.get(&r01).map(Vec::len), Some(1));
    }

    #[test]
    fn partition_creatures_handles_negative_coords() {
        // Gurubashi Arena position lands in (-14, 0), not the (-13, 0)
        // that truncate-toward-zero would yield. Guards the `floor()`
        // path in `CellKey::from_position`.
        let mut slab: Slab<Creature> = Slab::new();
        slab.insert(make_creature_at(7, Map::EasternKingdoms, -13206.0, 272.0));

        let buckets = partition_creatures(slab);

        let gurubashi = CellKey { map: Map::EasternKingdoms, cx: -14, cy: 0 };
        assert_eq!(buckets.len(), 1);
        assert_eq!(buckets.get(&gurubashi).map(Vec::len), Some(1));
        let not_truncated = CellKey { map: Map::EasternKingdoms, cx: -13, cy: 0 };
        assert!(
            !buckets.contains_key(&not_truncated),
            "creature at -13206 should not land in (-13, 0)"
        );
    }

    #[test]
    fn partition_creatures_preserves_empty_input() {
        let slab: Slab<Creature> = Slab::new();
        let buckets = partition_creatures(slab);
        assert!(buckets.is_empty());
    }

    #[test]
    fn partition_creatures_groups_multiple_in_same_cell() {
        // Two creatures in the same 1000-yd cell should land in one
        // bucket of size 2; confirms `entry(...).or_default().push(...)`.
        let mut slab: Slab<Creature> = Slab::new();
        slab.insert(make_creature_at(1, Map::EasternKingdoms, 100.0, 100.0));
        slab.insert(make_creature_at(2, Map::EasternKingdoms, 900.0, 900.0));

        let buckets = partition_creatures(slab);

        let r00 = CellKey { map: Map::EasternKingdoms, cx: 0, cy: 0 };
        assert_eq!(buckets.len(), 1);
        assert_eq!(buckets.get(&r00).map(Vec::len), Some(2));
    }

    #[test]
    fn partition_creatures_separates_by_map() {
        // Same (x, y) on different maps must produce different
        // CellKeys.
        let mut slab: Slab<Creature> = Slab::new();
        slab.insert(make_creature_at(1, Map::EasternKingdoms, 100.0, 100.0));
        slab.insert(make_creature_at(2, Map::Kalimdor, 100.0, 100.0));

        let buckets = partition_creatures(slab);

        assert_eq!(buckets.len(), 2);
        assert!(buckets.contains_key(&CellKey {
            map: Map::EasternKingdoms, cx: 0, cy: 0,
        }));
        assert!(buckets.contains_key(&CellKey {
            map: Map::Kalimdor, cx: 0, cy: 0,
        }));
    }
}
